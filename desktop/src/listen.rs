//! Listens to the default microphone outside recordings: hold-to-talk dictation into a note, and short
//! spoken phrases for the wake word and commands. Nothing is saved; audio is transcribed on this device
//! and dropped.
//!
//! Once someone has trained "Hey Superteem", the keyword spotter hears it about a tenth of a second after
//! it is said, and only the command that follows goes to speech to text. Until then every phrase is
//! transcribed and searched for the wake word, which is slower and mishears more.

use crate::keyphrases::{self, Heard};
use crate::process::{FRAME_SAMPLES, RATE};
use crate::transcribe::{self, Transcriber};
use crate::voices::VoicePrint;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use stv_core::attribute::window_db;
use stv_core::kws::Spotter;

/// Biases whisper towards the app's name, which it otherwise hears as "super team".
const PROMPT: &str = "Hey Superteem.";

static TRANSCRIBER: Mutex<Option<Transcriber>> = Mutex::new(None);
static ON_HEARD: OnceLock<Box<dyn Fn(String) + Send + Sync>> = OnceLock::new();
static ON_PHRASE: OnceLock<Box<dyn Fn(Heard) + Send + Sync>> = OnceLock::new();
/// Bumped when key phrases are trained, so the listener picks them up.
static PHRASES_VERSION: AtomicU32 = AtomicU32::new(0);

/// `on_heard` gets the words of each phrase transcribed; `on_phrase` each trained phrase as it is heard.
pub fn init(on_heard: impl Fn(String) + Send + Sync + 'static, on_phrase: impl Fn(Heard) + Send + Sync + 'static) {
    let _ = ON_HEARD.set(Box::new(on_heard));
    let _ = ON_PHRASE.set(Box::new(on_phrase));
}

pub fn phrases_changed() {
    PHRASES_VERSION.fetch_add(1, Ordering::Relaxed);
}

/// Loads the model once and keeps it, so each phrase is quick.
pub fn transcribe_all(pcm: &[i16]) -> Result<String, String> {
    if !transcribe::model_ready() {
        return Err("Download the speech model first, in Settings.".into());
    }
    let mut slot = TRANSCRIBER.lock().unwrap();
    if slot.is_none() {
        *slot = Some(Transcriber::load()?);
    }
    slot.as_ref().unwrap().transcribe_with_prompt(pcm, PROMPT)
}

const FRAME_MS: usize = 1000 * FRAME_SAMPLES / RATE as usize;
/// Speech kept from just before the level rose, so the first word is not clipped.
const PREROLL_FRAMES: usize = 300 / FRAME_MS;
/// A pause this long ends a phrase.
const END_SILENCE_FRAMES: usize = 700 / FRAME_MS;
/// After the wake word a command is short and expected, so a shorter pause ends it.
const COMMAND_END_SILENCE_FRAMES: usize = 450 / FRAME_MS;
/// How long a command may take to start after the wake word.
const COMMAND_WAIT_FRAMES: usize = 5_000 / FRAME_MS;
const MIN_PHRASE_FRAMES: usize = 350 / FRAME_MS;
const MAX_PHRASE_FRAMES: usize = 8_000 / FRAME_MS;
/// Speech is this far above the room's background level.
const SPEECH_ABOVE_DB: f32 = 12.0;
const SPEECH_MIN_DB: f32 = -52.0;
/// Audio kept so the words straight after the wake word can be taken from where it ended.
const RECENT_SAMPLES: usize = RATE as usize;

pub enum Event {
    /// A trained phrase, heard by the spotter.
    Phrase(Heard),
    /// Speech for whisper: a command after the wake word, or any phrase while listening closely.
    Speech(Vec<i16>),
}

struct Trained {
    spotter: Spotter,
    prints: Vec<VoicePrint>,
    verify: bool,
}

/// Turns microphone frames into trained phrases and phrases to transcribe. Without trained phrases every
/// phrase is transcribed, as the wake word can only be found in the words.
pub struct Hearing {
    trained: Option<Trained>,
    floor_db: f32,
    preroll: VecDeque<Vec<i16>>,
    phrase: Vec<i16>,
    speech_frames: usize,
    silent_frames: usize,
    /// Frames left to start a command after the wake word, while one is awaited.
    command: Option<usize>,
    recent: VecDeque<i16>,
    /// Samples pushed so far, which the spotter counts in too.
    samples: u64,
}

impl Default for Hearing {
    fn default() -> Self {
        Self::new()
    }
}

impl Hearing {
    pub fn new() -> Self {
        let (models, prints) = keyphrases::models();
        let trained = (!models.is_empty()).then(|| Trained { spotter: Spotter::new(models), prints, verify: crate::prefs::load().kws_verify });
        Self {
            trained,
            floor_db: -60.0,
            preroll: VecDeque::new(),
            phrase: Vec::new(),
            speech_frames: 0,
            silent_frames: 0,
            command: None,
            recent: VecDeque::new(),
            samples: 0,
        }
    }

    /// Whether the wake word is heard by the spotter rather than in transcribed words.
    pub fn trained(&self) -> bool {
        self.trained.is_some()
    }

    /// Forgets the phrase in progress, as when the app starts speaking or dictation takes over.
    pub fn interrupt(&mut self) {
        self.phrase.clear();
        self.speech_frames = 0;
        self.command = None;
    }

    /// `attentive` transcribes every phrase, as when the voice panel is open and waiting for an answer.
    pub fn push(&mut self, frame: &[i16], attentive: bool, out: &mut Vec<Event>) {
        self.samples += frame.len() as u64;
        self.recent.extend(frame);
        let excess = self.recent.len().saturating_sub(RECENT_SAMPLES);
        self.recent.drain(..excess);

        let hits = match self.trained.as_mut() {
            Some(t) => t.spotter.push(frame).into_iter().map(|hit| (keyphrases::describe(&hit, "listen", &t.prints, t.verify), hit)).collect(),
            None => Vec::new(),
        };
        {
            for (heard, hit) in hits {
                if !heard.rejected {
                    if heard.phrase == keyphrases::WAKE {
                        // The command starts where the wake word ended, even if it was said without a pause.
                        let after = (self.samples.saturating_sub(hit.end) as usize).min(self.recent.len());
                        self.phrase = self.recent.range(self.recent.len() - after..).copied().collect();
                        self.speech_frames = 0;
                        self.silent_frames = 0;
                        self.command = Some(COMMAND_WAIT_FRAMES);
                    } else {
                        // The spotter has handled it, so it isn't transcribed and acted on again.
                        self.interrupt();
                    }
                }
                out.push(Event::Phrase(heard));
            }
        }

        let db = window_db(frame, frame.len())[0];
        let speech = db > SPEECH_MIN_DB && db > self.floor_db + SPEECH_ABOVE_DB;
        // The background level follows quiet frames quickly and loud ones slowly.
        self.floor_db += if db < self.floor_db { (db - self.floor_db) * 0.2 } else { (db - self.floor_db) * 0.002 };

        if self.trained.is_some() && !attentive && self.command.is_none() {
            self.phrase.clear();
            self.remember_preroll(frame);
            return;
        }
        if let Some(left) = self.command.as_mut() {
            if self.speech_frames == 0 {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    self.command = None;
                    self.phrase.clear();
                    return;
                }
            }
        }
        if self.phrase.is_empty() {
            if speech {
                self.phrase.extend(self.preroll.drain(..).flatten());
                self.phrase.extend_from_slice(frame);
                self.speech_frames = 1;
                self.silent_frames = 0;
            } else {
                self.remember_preroll(frame);
            }
            return;
        }
        self.phrase.extend_from_slice(frame);
        if speech {
            self.speech_frames += 1;
            self.silent_frames = 0;
        } else {
            self.silent_frames += 1;
        }
        let end_silence = if self.command.is_some() { COMMAND_END_SILENCE_FRAMES } else { END_SILENCE_FRAMES };
        let frames = self.phrase.len() / FRAME_SAMPLES;
        if (self.speech_frames > 0 && self.silent_frames >= end_silence) || frames >= MAX_PHRASE_FRAMES {
            let phrase = std::mem::take(&mut self.phrase);
            if self.speech_frames >= MIN_PHRASE_FRAMES {
                out.push(Event::Speech(phrase));
                self.command = None;
            }
            self.speech_frames = 0;
        } else if self.speech_frames == 0 && frames > PREROLL_FRAMES {
            // Words carried over from the wake word that turned out to be silence.
            self.phrase.clear();
        }
    }

    fn remember_preroll(&mut self, frame: &[i16]) {
        self.preroll.push_back(frame.to_vec());
        if self.preroll.len() > PREROLL_FRAMES {
            self.preroll.pop_front();
        }
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;
    use crate::capture::{downmix, Resampler};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    /// Dictation is capped at the two-minute lock, with a little spare for the last word.
    const MAX_DICTATION: usize = 125 * RATE as usize;

    #[derive(Default)]
    struct Ear {
        wake: bool,
        /// Set while the app speaks, so its own voice isn't taken as a command.
        muted: bool,
        /// Set while the voice panel waits for an answer, so every phrase is transcribed.
        attentive: bool,
        /// Set while a voice print or key phrase is being recorded, so reading it aloud isn't taken as commands.
        paused: bool,
        dictating: bool,
        dictation: Vec<i16>,
        level_db: f32,
    }

    struct Running {
        stop: Arc<AtomicBool>,
        workers: Vec<JoinHandle<()>>,
    }

    static EAR: Mutex<Ear> =
        Mutex::new(Ear { wake: false, muted: false, attentive: false, paused: false, dictating: false, dictation: Vec::new(), level_db: -90.0 });
    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

    /// Opens the microphone when something needs it and closes it when nothing does.
    fn sync_microphone() -> Result<(), String> {
        let needed = {
            let ear = EAR.lock().unwrap();
            ear.wake || ear.dictating
        };
        let mut running = RUNNING.lock().unwrap();
        if needed && running.is_none() {
            *running = Some(open()?);
        } else if !needed {
            if let Some(r) = running.take() {
                r.stop.store(true, Ordering::Relaxed);
                for w in r.workers {
                    let _ = w.join();
                }
            }
        }
        Ok(())
    }

    fn open() -> Result<Running, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (phrases_tx, phrases_rx) = mpsc::channel::<Vec<i16>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let mic = {
            let stop = stop.clone();
            thread::spawn(move || {
                if let Err(e) = run_microphone(&stop, phrases_tx, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
        };
        // Phrases are transcribed on their own thread so listening never stalls behind whisper.
        let words = thread::spawn(move || {
            for phrase in phrases_rx {
                if let Ok(text) = transcribe_all(&phrase) {
                    if !text.is_empty() {
                        if let Some(on_heard) = ON_HEARD.get() {
                            on_heard(text);
                        }
                    }
                }
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Running { stop, workers: vec![mic, words] }),
            Ok(Err(e)) => Err(format!("The microphone could not be opened: {e}")),
            Err(_) => Err("The microphone did not respond.".into()),
        }
    }

    fn run_microphone(stop: &AtomicBool, phrases: mpsc::Sender<Vec<i16>>, ready: &mpsc::Sender<Result<(), String>>) -> Result<(), String> {
        // The microphone chosen when the person's voice print was made, while it is still connected.
        let host = cpal::default_host();
        let chosen = crate::prefs::load().enrol_input;
        let device = chosen
            .and_then(|name| host.input_devices().ok()?.find(|d| d.name().is_ok_and(|n| n == name)))
            .or_else(|| host.default_input_device())
            .ok_or("no microphone found")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let channels = config.channels() as usize;
        let mut resampler = Resampler::new(config.sample_rate().0);
        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let stream_config = config.config();
        let on_error = |_| {};
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |d: &[f32], _: &_| drop(tx.send(downmix(d, channels, |s| s))),
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |d: &[i16], _: &_| drop(tx.send(downmix(d, channels, |s| s as f32 / 32768.0))),
                on_error,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        let _ = ready.send(Ok(()));

        let mut pending: Vec<i16> = Vec::new();
        let mut hearing = Hearing::new();
        let mut version = PHRASES_VERSION.load(Ordering::Relaxed);
        let mut events = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(block) => resampler.push(&block, &mut pending),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if PHRASES_VERSION.load(Ordering::Relaxed) != version {
                version = PHRASES_VERSION.load(Ordering::Relaxed);
                hearing = Hearing::new();
            }
            while pending.len() >= FRAME_SAMPLES {
                let frame: Vec<i16> = pending.drain(..FRAME_SAMPLES).collect();
                let db = window_db(&frame, FRAME_SAMPLES)[0];
                let (listening, attentive) = {
                    let mut ear = EAR.lock().unwrap();
                    ear.level_db = db;
                    if ear.dictating && ear.dictation.len() < MAX_DICTATION {
                        ear.dictation.extend_from_slice(&frame);
                    }
                    (ear.wake && !ear.muted && !ear.paused && !ear.dictating, ear.attentive)
                };
                // While dictating the words go into the note, and while the app speaks it would hear itself.
                if !listening {
                    hearing.interrupt();
                    continue;
                }
                hearing.push(&frame, attentive, &mut events);
                for event in events.drain(..) {
                    match event {
                        Event::Phrase(heard) => {
                            if let Some(on_phrase) = ON_PHRASE.get() {
                                on_phrase(heard);
                            }
                        }
                        Event::Speech(pcm) => {
                            let _ = phrases.send(pcm);
                        }
                    }
                }
            }
        }
        drop(stream);
        Ok(())
    }

    pub fn set_wake(on: bool) -> Result<(), String> {
        EAR.lock().unwrap().wake = on;
        sync_microphone()
    }

    pub fn set_muted(muted: bool) {
        EAR.lock().unwrap().muted = muted;
    }

    pub fn set_attentive(attentive: bool) {
        EAR.lock().unwrap().attentive = attentive;
    }

    pub fn set_paused(paused: bool) {
        EAR.lock().unwrap().paused = paused;
    }

    pub fn dictate_start() -> Result<(), String> {
        if !transcribe::model_ready() {
            return Err("Download the speech model first, in Settings.".into());
        }
        {
            let mut ear = EAR.lock().unwrap();
            ear.dictation.clear();
            ear.dictating = true;
        }
        sync_microphone().inspect_err(|_| EAR.lock().unwrap().dictating = false)
    }

    /// Drops what was said since `dictate_start`, without writing it down.
    pub fn dictate_cancel() -> Result<(), String> {
        {
            let mut ear = EAR.lock().unwrap();
            ear.dictating = false;
            ear.dictation = Vec::new();
        }
        sync_microphone()
    }

    /// Returns the words spoken since `dictate_start`, and the audio they came from.
    pub fn dictate_stop() -> Result<(String, Vec<i16>), String> {
        // A short tail, so the last word isn't cut off when the key is released mid-word.
        thread::sleep(Duration::from_millis(250));
        let pcm = {
            let mut ear = EAR.lock().unwrap();
            ear.dictating = false;
            std::mem::take(&mut ear.dictation)
        };
        sync_microphone()?;
        if pcm.len() < RATE as usize / 4 {
            return Ok((String::new(), pcm));
        }
        Ok((transcribe_all(&pcm)?, pcm))
    }

    /// The microphone level from 0 to 1, for the dictation meter.
    pub fn level() -> f32 {
        ((EAR.lock().unwrap().level_db + 70.0) / 70.0).clamp(0.0, 1.0)
    }
}

#[cfg(target_os = "android")]
mod imp {
    const NOT_YET: &str = "Voice commands and dictation are not available on Android yet.";

    pub fn set_wake(on: bool) -> Result<(), String> {
        if on {
            Err(NOT_YET.into())
        } else {
            Ok(())
        }
    }

    pub fn set_muted(_muted: bool) {}

    pub fn set_attentive(_attentive: bool) {}

    pub fn set_paused(_paused: bool) {}

    pub fn dictate_start() -> Result<(), String> {
        Err(NOT_YET.into())
    }

    pub fn dictate_cancel() -> Result<(), String> {
        Ok(())
    }

    pub fn dictate_stop() -> Result<(String, Vec<i16>), String> {
        Err(NOT_YET.into())
    }

    pub fn level() -> f32 {
        0.0
    }
}

pub use imp::*;
