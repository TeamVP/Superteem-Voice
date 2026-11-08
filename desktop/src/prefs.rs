//! What this device remembers: the person's choices and which microphones have recorded successfully.

use crate::library;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

const FILE: &str = "preferences.json";
/// The name the pins are remembered under, as they are chosen together rather than one by one.
pub const PINS: &str = "Superteem pins";
/// A stream counts as working with at least a second of audio and little loss.
const MIN_FRAMES: u64 = 50;
const MAX_LOSS: f64 = 0.05;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MicRecord {
    pub ok: u32,
    pub failed: u32,
    pub last_ok: u64,
    pub last_failed: u64,
    /// Why the last failure happened, as the stream reported it.
    pub last_problem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: String,
    /// Inputs ticked for the last recording.
    pub inputs: Vec<String>,
    pub pins: bool,
    /// The microphone checked when the voice print was made: the default for dictation, commands and meetings.
    pub enrol_input: Option<String>,
    pub mics: BTreeMap<String, MicRecord>,
    /// Scripts the person wrote for recording voice prints.
    pub scripts: Vec<Script>,
    /// Listen for "Hey Superteem" while the app is open.
    pub wake: bool,
    /// Don't speak replies aloud.
    pub quiet: bool,
    /// The first-run setup was finished or skipped.
    pub onboarded: bool,
    /// The setup step reached, so closing the app part way through returns there.
    pub setup_step: u32,
    /// When the terms of use were agreed to, as a Unix time; 0 if never.
    pub terms_accepted: u64,
    /// Microphones heard working at least once; only these are offered as meeting tracks.
    pub working_mics: Vec<String>,
    /// Microphones the person hid from Devices and Tracks.
    pub hidden_mics: Vec<String>,
    /// Voice Pins were added in Devices, so meetings offer them as tracks.
    pub pins_added: bool,
    /// Scales how close live audio must be to a trained phrase: above 1 hears more, below 1 is stricter.
    pub kws_sensitivity: Option<f32>,
    /// Only act on a phrase when it also sounds like the voice print it was trained by.
    pub kws_verify: bool,
    /// Keep each dictation's audio beside its note for a week.
    pub keep_audio: bool,
}

impl Prefs {
    pub fn kws_sensitivity(&self) -> f32 {
        self.kws_sensitivity.unwrap_or(1.0).clamp(0.7, 1.4)
    }
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn load() -> Prefs {
    library::read_json(&library::data_dir().join(FILE)).unwrap_or_default()
}

/// Reads, changes and writes the file as one step.
pub fn update(change: impl FnOnce(&mut Prefs)) -> Result<Prefs, String> {
    let _guard = LOCK.lock().unwrap();
    let mut prefs = load();
    change(&mut prefs);
    std::fs::create_dir_all(library::data_dir()).map_err(|e| e.to_string())?;
    library::write_json(&library::data_dir().join(FILE), &prefs)?;
    Ok(prefs)
}

/// One stream's result at the end of a recording.
pub struct Outcome {
    pub device: String,
    pub frames: u64,
    pub lost: u64,
    pub problem: String,
}

impl Outcome {
    fn worked(&self) -> bool {
        self.frames >= MIN_FRAMES && (self.lost as f64) < MAX_LOSS * (self.frames + self.lost) as f64
    }
}

/// Records which inputs worked. Several streams from one device (pins) count as one result, which is a
/// success when any of them worked. `chosen` inputs with no stream at all failed to start.
pub fn record_outcomes(chosen: &[String], outcomes: &[Outcome]) -> Result<(), String> {
    let now = library::now();
    update(|prefs| {
        for device in chosen {
            let mine: Vec<&Outcome> = outcomes.iter().filter(|o| &o.device == device).collect();
            let record = prefs.mics.entry(device.clone()).or_default();
            if mine.iter().any(|o| o.worked()) {
                record.ok += 1;
                record.last_ok = now;
            } else {
                record.failed += 1;
                record.last_failed = now;
                record.last_problem = mine
                    .iter()
                    .map(|o| o.problem.as_str())
                    .find(|p| !p.is_empty())
                    .unwrap_or(if mine.is_empty() { "It did not start" } else { "No usable audio" })
                    .to_string();
            }
        }
    })
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_needs_audio_and_little_loss_to_count_as_working() {
        let o = |frames, lost| Outcome { device: "x".into(), frames, lost, problem: String::new() };
        assert!(o(500, 2).worked());
        assert!(!o(10, 0).worked());
        assert!(!o(500, 100).worked());
    }
}
