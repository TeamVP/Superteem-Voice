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
use crate::vocabulary;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use stv_core::attribute::window_db;
use stv_core::kws::{Spotter, Template};

/// Biases dictation towards the app's name, which whisper otherwise hears as "super team". Phrases the
/// listener hears are never given it: on a tap or a cough whisper repeats the prompt back, which read as
/// the wake word every time.
const PROMPT: &str = "Hey Superteem.";

static TRANSCRIBER: Mutex<Option<Transcriber>> = Mutex::new(None);
static ON_HEARD: OnceLock<Box<dyn Fn(String) + Send + Sync>> = OnceLock::new();
static ON_PHRASE: OnceLock<Box<dyn Fn(Heard) + Send + Sync>> = OnceLock::new();
static ON_PRACTICE: OnceLock<Box<dyn Fn(Practice) + Send + Sync>> = OnceLock::new();
/// Good takes of the wake word said so far while practising it, each as said and with noise added.
static PRACTICE: Mutex<Vec<Vec<Template>>> = Mutex::new(Vec::new());

/// One go at saying "Hey Superteem" while practising it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Practice {
    /// It counted.
    pub ok: bool,
    /// Good takes so far.
    pub good: usize,
    /// What speech to text heard, when the model is there.
    pub heard: String,
    /// Why it didn't count.
    pub reason: String,
    /// Something was said and is being checked; the result follows in another event.
    pub checking: bool,
}

pub fn on_practice(f: impl Fn(Practice) + Send + Sync + 'static) {
    let _ = ON_PRACTICE.set(Box::new(f));
}

/// Checks one thing said while practising, and keeps it if it was the wake word.
fn practise(pcm: &[i16]) {
    let spec = keyphrases::spec(keyphrases::WAKE).unwrap();
    // Checking takes a moment, as speech to text runs on it, so the page shows that it heard something.
    if let Some(f) = ON_PRACTICE.get() {
        let good = PRACTICE.lock().unwrap().len();
        f(Practice { ok: false, good, heard: String::new(), reason: String::new(), checking: true });
    }
    let result = keyphrases::take_from_utterance(spec, pcm);
    let mut takes = PRACTICE.lock().unwrap();
    let report = match result {
        Ok((t, heard)) => {
            takes.push(t);
            Practice { ok: true, good: takes.len(), heard, reason: String::new(), checking: false }
        }
        Err(reason) => Practice { ok: false, good: takes.len(), heard: String::new(), reason, checking: false },
    };
    drop(takes);
    if let Some(f) = ON_PRACTICE.get() {
        f(report);
    }
}

/// The good takes from practice, which are cleared.
pub fn practice_takes() -> Vec<Template> {
    std::mem::take(&mut *PRACTICE.lock().unwrap()).into_iter().flatten().collect()
}
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

/// Transcribes dictation, steered towards the app's name and the person's vocabulary, then corrected by it.
pub fn transcribe_all(pcm: &[i16]) -> Result<String, String> {
    let prompt = vocabulary::prompt(PROMPT, &vocabulary::load()).unwrap_or_else(|| PROMPT.to_string());
    let words = with_transcriber(|t| t.transcribe_with_prompt(pcm, &prompt))?;
    Ok(vocabulary::correct_text(&words))
}

/// Returns the words spoken since `dictate_start`, and the audio they came from.
pub fn dictate_stop() -> Result<(String, Vec<i16>), String> {
    let pcm = dictate_stop_audio()?;
    if pcm.len() < RATE as usize / 4 {
        return Ok((String::new(), pcm));
    }
    Ok((transcribe_all(&pcm)?, pcm))
}

/// What whisper writes for the words spoken since `dictate_start` with nothing to steer or correct it, so
/// the person can hear how it gets one of their words wrong.
pub fn dictate_stop_plain() -> Result<String, String> {
    let pcm = dictate_stop_audio()?;
    if pcm.len() < RATE as usize / 4 {
        return Ok(String::new());
    }
    with_transcriber(|t| t.transcribe_plain(&pcm))
}

/// Transcribes a short phrase heard by the listener, with no prompt for whisper to repeat.
pub fn transcribe_phrase(pcm: &[i16]) -> Result<String, String> {
    with_transcriber(|t| t.transcribe_plain(pcm))
}

/// Loads the model once and keeps it, so each phrase is quick.
fn with_transcriber(run: impl FnOnce(&Transcriber) -> Result<String, String>) -> Result<String, String> {
    if !transcribe::model_ready() {
        return Err("Download the speech model first, in Settings.".into());
    }
    let mut slot = TRANSCRIBER.lock().unwrap();
    if slot.is_none() {
        *slot = Some(Transcriber::load()?);
    }
    run(slot.as_ref().unwrap())
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
/// Also the audio around a spotted phrase, to check its words: long enough for the longest phrase.
const RECENT_SAMPLES: usize = 3 * RATE as usize;

pub enum Event {
    /// A trained phrase, heard by the spotter, with the audio around it to check its words.
    Phrase(Heard, Vec<i16>),
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
        Self { trained, ..Self::untrained() }
    }

    /// Hears where phrases start and end, without listening for trained ones.
    pub fn untrained() -> Self {
        Self {
            trained: None,
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

        // The audio from a little before a hit's start to now, for checking its words.
        let around = |recent: &VecDeque<i16>, samples: u64, start: u64| -> Vec<i16> {
            let pad = (keyphrases::CONFIRM_PAD_S * RATE as f64) as u64;
            let back = (samples.saturating_sub(start.saturating_sub(pad)) as usize).min(recent.len());
            recent.range(recent.len() - back..).copied().collect()
        };
        let hits = match self.trained.as_mut() {
            Some(t) => t.spotter.push(frame).into_iter().map(|hit| (keyphrases::describe(&hit, "listen", &t.prints, t.verify), hit)).collect(),
            None => Vec::new(),
        };
        {
            for (heard, hit) in hits {
                if !heard.rejected {
                    if keyphrases::is_answer(&heard.phrase) {
                        // Only the page knows whether a question waits, so the words still go to speech to text.
                    } else if heard.phrase == keyphrases::WAKE {
                        // The command starts where the wake word ended, even if it was said without a pause.
                        let after = (self.samples.saturating_sub(hit.end) as usize).min(self.recent.len());
                        self.phrase = self.recent.range(self.recent.len() - after..).copied().collect();
                        self.speech_frames = 0;
                        self.silent_frames = 0;
                        self.command = Some(COMMAND_WAIT_FRAMES);
                    } else if self.command.is_some() {
                        // The command after the wake word: the spotter has handled it, so it isn't transcribed too.
                        self.interrupt();
                    } else {
                        // The spotter has handled the phrase, so its words aren't transcribed and acted on again,
                        // but what was said before it in the same breath still is.
                        let said = (self.samples.saturating_sub(hit.start) as usize).min(self.phrase.len());
                        self.phrase.truncate(self.phrase.len() - said);
                        self.speech_frames = self.speech_frames.min(self.phrase.len() / FRAME_SAMPLES);
                        self.preroll.clear();
                    }
                }
                let clip = around(&self.recent, self.samples, hit.start);
                out.push(Event::Phrase(heard, clip));
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

    /// Dictation is written down as it goes (see `dictation`), which keeps this buffer short; the cap only
    /// stops it growing without end if that falls behind.
    const MAX_DICTATION: usize = 600 * RATE as usize;

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
        /// Set while setup has the person practise "Hey Superteem": each thing said is checked and kept.
        practising: bool,
    }

    struct Running {
        stop: Arc<AtomicBool>,
        workers: Vec<JoinHandle<()>>,
    }

    static EAR: Mutex<Ear> =
        Mutex::new(Ear {
        wake: false,
        muted: false,
        attentive: false,
        paused: false,
        dictating: false,
        dictation: Vec::new(),
        level_db: -90.0,
        practising: false,
    });
    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

    /// Opens the microphone when something needs it and closes it when nothing does.
    fn sync_microphone() -> Result<(), String> {
        let needed = {
            let ear = EAR.lock().unwrap();
            ear.wake || ear.dictating || ear.practising
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
        let (phrases_tx, phrases_rx) = mpsc::channel::<(Vec<i16>, bool)>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (check_tx, check_rx) = mpsc::channel::<(Heard, Vec<i16>)>();
        let mic = {
            let stop = stop.clone();
            thread::spawn(move || {
                if let Err(e) = run_microphone(&stop, phrases_tx, check_tx, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
        };
        // Spotted phrases are checked against their words on their own thread, ahead of other speech, so
        // listening never stalls behind whisper and a phrase isn't kept waiting behind a long sentence.
        let checker = thread::spawn(move || {
            for (mut heard, clip) in check_rx {
                keyphrases::confirm(&mut heard, &clip);
                if let Some(on_phrase) = ON_PHRASE.get() {
                    on_phrase(heard);
                }
            }
        });
        // Phrases are transcribed on their own thread so listening never stalls behind whisper.
        let words = thread::spawn(move || {
            for (phrase, practice) in phrases_rx {
                if practice {
                    practise(&phrase);
                    continue;
                }
                if let Ok(text) = transcribe_phrase(&phrase) {
                    if !text.is_empty() {
                        if let Some(on_heard) = ON_HEARD.get() {
                            on_heard(text);
                        }
                    }
                }
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Running { stop, workers: vec![mic, words, checker] }),
            Ok(Err(e)) => Err(format!("The microphone could not be opened: {e}")),
            Err(_) => Err("The microphone did not respond.".into()),
        }
    }

    fn run_microphone(
        stop: &AtomicBool,
        phrases: mpsc::Sender<(Vec<i16>, bool)>,
        checks: mpsc::Sender<(Heard, Vec<i16>)>,
        ready: &mpsc::Sender<Result<(), String>>,
    ) -> Result<(), String> {
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
        let mut practice = Hearing::untrained();
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
                let (listening, attentive, practising) = {
                    let mut ear = EAR.lock().unwrap();
                    ear.level_db = db;
                    if ear.dictating && ear.dictation.len() < MAX_DICTATION {
                        ear.dictation.extend_from_slice(&frame);
                    }
                    // While dictating, only trained phrases are listened for, so "Hey Superteem, stop recording"
                    // can end it without every word being transcribed twice.
                    let dictating = ear.dictating && !hearing.trained();
                    (ear.wake && !ear.muted && !ear.paused && !dictating, ear.attentive && !ear.dictating, ear.practising && !ear.muted)
                };
                // Practising the wake word: every phrase said is checked, and nothing is acted on.
                if practising {
                    hearing.interrupt();
                    practice.push(&frame, true, &mut events);
                    for event in events.drain(..) {
                        if let Event::Speech(pcm) = event {
                            let _ = phrases.send((pcm, true));
                        }
                    }
                    continue;
                }
                practice.interrupt();
                // While the app speaks it would hear itself.
                if !listening {
                    hearing.interrupt();
                    continue;
                }
                hearing.push(&frame, attentive, &mut events);
                for event in events.drain(..) {
                    match event {
                        // Rejected already, as taps or another voice: said straight away, for the Try it page.
                        Event::Phrase(heard, _) if heard.rejected => {
                            if let Some(on_phrase) = ON_PHRASE.get() {
                                on_phrase(heard);
                            }
                        }
                        Event::Phrase(heard, clip) => {
                            let _ = checks.send((heard, clip));
                        }
                        Event::Speech(pcm) => {
                            let _ = phrases.send((pcm, false));
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

    /// Starts or ends practising the wake word. Starting forgets earlier takes.
    pub fn set_practising(on: bool) -> Result<(), String> {
        if on {
            PRACTICE.lock().unwrap().clear();
        }
        EAR.lock().unwrap().practising = on;
        sync_microphone().inspect_err(|_| EAR.lock().unwrap().practising = false)
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

    /// Returns the audio recorded since `dictate_start`.
    pub fn dictate_stop_audio() -> Result<Vec<i16>, String> {
        // A short tail, so the last word isn't cut off when the key is released mid-word.
        thread::sleep(Duration::from_millis(250));
        let pcm = {
            let mut ear = EAR.lock().unwrap();
            ear.dictating = false;
            std::mem::take(&mut ear.dictation)
        };
        sync_microphone()?;
        Ok(pcm)
    }

    /// The audio dictated and not yet taken, while it carries on.
    pub fn dictate_pending() -> Vec<i16> {
        EAR.lock().unwrap().dictation.clone()
    }

    /// Lets go of the first `n` samples, once they're written down.
    pub fn dictate_consume(n: usize) {
        let mut ear = EAR.lock().unwrap();
        let n = n.min(ear.dictation.len());
        ear.dictation.drain(..n);
    }

    /// The microphone level from 0 to 1, for the dictation meter.
    pub fn level() -> f32 {
        ((EAR.lock().unwrap().level_db + 70.0) / 70.0).clamp(0.0, 1.0)
    }

    /// Windows doesn't silence an app's microphone without saying so.
    pub fn problem() -> Option<String> {
        None
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

    pub fn set_practising(on: bool) -> Result<(), String> {
        if on {
            Err(NOT_YET.into())
        } else {
            Ok(())
        }
    }

    // Dictation records through the capture service, like a meeting with one microphone, into a folder of
    // its own that is deleted once the words are written down.
    use super::{transcribe, RATE};
    use crate::recorder::{self, Recorder};
    use std::path::Path;
    use std::sync::Mutex;

    static DICTATION: Mutex<Option<Recorder>> = Mutex::new(None);
    /// Following the recording as it's written: where the next read starts, and the audio read but not taken.
    static FOLLOW: Mutex<(u64, Vec<i16>)> = Mutex::new((0, Vec::new()));
    const PHONE: &str = "Phone microphone";

    pub fn dictate_start() -> Result<(), String> {
        if !transcribe::model_ready() {
            return Err("Download the speech model first, in Settings.".into());
        }
        let mut slot = DICTATION.lock().unwrap();
        if slot.is_some() {
            return Err("Already dictating.".into());
        }
        let dir = crate::library::data_dir().join(format!("dictation-{}", crate::library::now()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // The microphone chosen when the voice print was made, as on Windows.
        let device = crate::prefs::load().enrol_input.unwrap_or_else(|| PHONE.into());
        match recorder::start(dir.clone(), &[device], false) {
            Ok(r) => {
                *slot = Some(r);
                *FOLLOW.lock().unwrap() = (0, Vec::new());
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    /// Stops recording and returns the folder it wrote to.
    fn stop() -> Result<std::path::PathBuf, String> {
        let r = DICTATION.lock().unwrap().take().ok_or("Not dictating.")?;
        let dir = r.dir().to_path_buf();
        r.stop()?;
        Ok(dir)
    }

    fn recorded(dir: &Path) -> Result<Vec<i16>, String> {
        let journal = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "stvj"))
            .ok_or("Nothing was recorded. Check Voice may use the microphone, in Settings.")?;
        Ok(stv_core::pipeline::load_source(&journal, RATE).map_err(|e| e.to_string())?.pcm)
    }

    pub fn dictate_cancel() -> Result<(), String> {
        if let Ok(dir) = stop() {
            let _ = std::fs::remove_dir_all(dir);
        }
        *FOLLOW.lock().unwrap() = (0, Vec::new());
        Ok(())
    }

    /// Reads what the capture service wrote since last time onto the audio waiting to be taken.
    fn follow(dir: &Path) {
        let Some(journal) = std::fs::read_dir(dir).ok().and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "stvj"))) else { return };
        let mut follow = FOLLOW.lock().unwrap();
        let Ok((records, next)) = stv_core::journal::read_since(&journal, follow.0) else { return };
        follow.0 = next;
        for record in records {
            let Ok(frame) = stv_core::protocol::Frame::decode(&record.frame) else { continue };
            match frame.codec {
                stv_core::protocol::Codec::ImaAdpcm => follow.1.extend(stv_core::adpcm::decode(&frame.payload).unwrap_or_default()),
                stv_core::protocol::Codec::Opus => {}
                _ => follow.1.extend(frame.payload.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]))),
            }
        }
    }

    pub fn dictate_pending() -> Vec<i16> {
        let dir = DICTATION.lock().unwrap().as_ref().map(|r| r.dir().to_path_buf());
        if let Some(dir) = dir {
            follow(&dir);
        }
        FOLLOW.lock().unwrap().1.clone()
    }

    pub fn dictate_consume(n: usize) {
        let mut follow = FOLLOW.lock().unwrap();
        let n = n.min(follow.1.len());
        follow.1.drain(..n);
    }

    /// Everything not yet taken. A dictation followed from the start returns only what's left; one that
    /// wasn't (the vocabulary test) returns the whole recording.
    pub fn dictate_stop_audio() -> Result<Vec<i16>, String> {
        let dir = stop()?;
        let followed = FOLLOW.lock().unwrap().0 > 0;
        let pcm = if followed {
            follow(&dir);
            Ok(std::mem::take(&mut FOLLOW.lock().unwrap().1))
        } else {
            recorded(&dir)
        };
        *FOLLOW.lock().unwrap() = (0, Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
        pcm
    }

    pub fn level() -> f32 {
        let slot = DICTATION.lock().unwrap();
        slot.as_ref().and_then(|r| r.status().ok()).and_then(|s| s.streams.first().map(|s| s.level)).unwrap_or(0.0)
    }

    /// Why a dictation hears nothing, when Android is the reason, for the page to say.
    pub fn problem() -> Option<String> {
        let slot = DICTATION.lock().unwrap();
        let status = slot.as_ref()?.status().ok()?;
        status.streams.into_iter().find(|s| s.state == "silenced" || s.state == "failed" || s.state == "no headset").map(|s| s.detail)
    }
}

pub use imp::*;
