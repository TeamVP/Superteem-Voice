//! Dictation that runs until it's stopped. The microphone keeps recording; at each real pause the audio so
//! far is clipped (in the middle of the pause, so no word is lost) and that clip is transcribed while the
//! next one records. The page is told when a clip is cut, so it can hold the clip's place in the note, and
//! again when its words are ready. When a clip turns out to have nothing in it and the silence carries on,
//! the dictation ends by itself.

use crate::{listen, notes, process};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use stv_core::attribute::window_db;
use stv_core::chunk;

/// Silence this long after an empty piece ends the dictation.
const END_AFTER_SILENCE_S: f32 = 2.5;
/// Before anything is said, the person gets this long to start.
const GRACE_S: u64 = 10;

/// A clip was cut and is being written down.
#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    pub session: u64,
    pub clip: u64,
}

/// A clip's words, empty when nothing was said in it.
#[derive(Debug, Clone, Serialize)]
pub struct Words {
    pub session: u64,
    pub clip: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ended {
    pub session: u64,
    /// The last words, not yet sent.
    pub text: String,
    pub reason: &'static str,
}

struct Session {
    id: u64,
    started: Instant,
    /// The last piece written down had no words.
    last_empty: bool,
    heard: bool,
    noise_db: Option<f32>,
    /// The note whose dictation audio is kept, and the audio so far.
    keep_for: Option<String>,
    kept: Vec<i16>,
    clips: u64,
    /// The languages the note is in, main one first.
    languages: Vec<String>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// Held while a piece is taken and written down, so stopping waits for it and the words stay in order.
static WORK: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The words in a piece, or nothing for silence or Whisper's markers for it.
fn words(pcm: &[i16], languages: &[String]) -> Result<String, String> {
    if pcm.len() < process::RATE as usize / 4 {
        return Ok(String::new());
    }
    let text = listen::transcribe_all(pcm, languages)?;
    Ok(if process::is_silence(&text) { String::new() } else { text.trim().to_string() })
}

/// The background level, learnt as the dictation goes: it falls at once to anything quieter and rises
/// slowly, so a long stretch of talking isn't taken for the room's noise.
fn learn_noise(session: &mut Session, pcm: &[i16]) -> Option<f32> {
    if pcm.len() < chunk::FRAME * 25 {
        return session.noise_db;
    }
    let now = chunk::noise_level(&window_db(pcm, chunk::FRAME));
    session.noise_db = Some(match session.noise_db {
        None => now,
        Some(was) if now < was => now,
        Some(was) => was + (now - was) * 0.05,
    });
    session.noise_db
}

/// Starts a dictation, returning its number. `on_clip` is told as each clip with speech in it is cut,
/// `on_words` gets its words once written down, and `on_end` is told when it ends by itself.
pub fn start(
    keep_for: Option<String>,
    languages: Vec<String>,
    on_clip: impl Fn(Clip) + Send + 'static,
    on_words: impl Fn(Words) + Send + 'static,
    on_end: impl Fn(Ended) + Send + 'static,
) -> Result<u64, String> {
    if SESSION.lock().unwrap().is_some() {
        return Err("Already dictating.".into());
    }
    listen::dictate_start()?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    *SESSION.lock().unwrap() = Some(Session { id, started: Instant::now(), last_empty: false, heard: false, noise_db: None, keep_for, kept: Vec::new(), clips: 0, languages });
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(300));
            let _work = WORK.lock().unwrap();
            let mut guard = SESSION.lock().unwrap();
            let Some(session) = guard.as_mut().filter(|s| s.id == id) else { return };
            let pending = listen::dictate_pending();
            let noise = learn_noise(session, &pending);
            if let Some(cut) = chunk::find_cut(&pending, noise) {
                let piece = pending[..cut.at].to_vec();
                listen::dictate_consume(cut.at);
                if session.keep_for.is_some() {
                    session.kept.extend_from_slice(&piece);
                }
                if !cut.speech {
                    session.last_empty = true;
                    continue;
                }
                session.clips += 1;
                let clip = session.clips;
                let languages = session.languages.clone();
                on_clip(Clip { session: id, clip });
                // Written down without holding the session, so the level meter and stop stay quick; the
                // microphone keeps recording the next clip meanwhile.
                drop(guard);
                let text = words(&piece, &languages).unwrap_or_default();
                let mut guard = SESSION.lock().unwrap();
                let Some(session) = guard.as_mut().filter(|s| s.id == id) else {
                    // Stopped while this clip was written down: its words still go in.
                    on_words(Words { session: id, clip, text });
                    return;
                };
                session.last_empty = text.is_empty();
                session.heard |= !text.is_empty();
                on_words(Words { session: id, clip, text });
                continue;
            }
            let quiet = chunk::trailing_silence_s(&pending, noise);
            let patient = session.heard || session.started.elapsed() > Duration::from_secs(GRACE_S);
            if session.last_empty && patient && quiet >= END_AFTER_SILENCE_S {
                drop(guard);
                let text = finish(id).unwrap_or_default();
                on_end(Ended { session: id, text, reason: "silence" });
                return;
            }
        }
    });
    Ok(id)
}

/// Ends the dictation: stops recording, writes down what's left and keeps the audio when the note asks.
/// Called with the work lock held.
fn finish(id: u64) -> Result<String, String> {
    let session = {
        let mut guard = SESSION.lock().unwrap();
        if guard.as_ref().is_none_or(|s| s.id != id) {
            return Err("Not dictating.".into());
        }
        guard.take().expect("checked")
    };
    let rest = listen::dictate_stop_audio()?;
    let text = words(&rest, &session.languages)?;
    if let Some(note) = &session.keep_for {
        let mut audio = session.kept;
        audio.extend_from_slice(&rest);
        if session.heard || !text.is_empty() {
            notes::keep_dictation(note, &audio, process::RATE)?;
        }
    }
    Ok(text)
}

/// Stops dictation `id` and returns the words not yet sent.
pub fn stop(id: u64) -> Result<String, String> {
    let _work = WORK.lock().unwrap();
    finish(id)
}

/// Stops dictation `id`, dropping what isn't written down yet.
pub fn cancel(id: u64) -> Result<(), String> {
    let _work = WORK.lock().unwrap();
    let mut guard = SESSION.lock().unwrap();
    if guard.as_ref().is_some_and(|s| s.id == id) {
        *guard = None;
        drop(guard);
        listen::dictate_cancel()?;
    }
    Ok(())
}
