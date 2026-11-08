//! The wake word and key phrases. A person trains each phrase by saying it a few times; the takes are kept
//! with their voice print. The listener hears them about a tenth of a second after they are said, apart
//! from speech to text, and meetings are scanned for them when processed.

use crate::library;
use crate::voices::{self, VoicePrint};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use stv_core::embed;
use stv_core::kws::{self, Calibration, PhraseModel, Template, RATE};

pub struct Spec {
    pub id: &'static str,
    pub text: &'static str,
    /// What hearing it does.
    pub does: &'static str,
    /// Only acted on straight after this phrase.
    pub requires: Option<&'static str>,
    /// One line per take, so the takes cover the ways the person really says it.
    pub how: &'static [&'static str],
}

pub const WAKE: &str = "wake";

/// "Hey Superteem" is four syllables with a stressed, unusual middle ("SOO-per-teem"), long enough not to
/// turn up by chance. "Hey" is what tells it apart from "a super team" said in passing, so it is part of
/// the phrase. Commands that change anything need it first; markers that only note the moment do not.
pub const PHRASES: &[Spec] = &[
    Spec {
        id: WAKE,
        text: "Hey Superteem",
        does: "Gets the app's attention. Commands that change anything need it first.",
        requires: None,
        how: &[
            "Normally, as if catching someone's attention across a table",
            "Normally again",
            "Quickly, running the words together",
            "Slowly and clearly",
            "Quietly, or turned a little away from the microphone",
            "Mid-sentence, as in \"…so, hey Superteem\"",
            "As \"Hi Superteem\" or \"Okay Superteem\", if you might say that",
        ],
    },
    Spec {
        id: "bookmark",
        text: "Bookmark that",
        does: "Marks the moment in the transcript.",
        requires: None,
        how: &["Normally", "Quickly", "Slowly and clearly", "Quietly"],
    },
    Spec {
        id: "action",
        text: "Action item",
        does: "Marks an action item. It's heard whenever it's said, as that is when it matters.",
        requires: None,
        how: &["Normally", "Quickly, as in \"action item for Raj\"", "Slowly and clearly", "Quietly"],
    },
    Spec {
        id: "stop",
        text: "Stop recording",
        does: "Stops the recording, but only straight after \"Hey Superteem\".",
        requires: Some(WAKE),
        how: &["Normally", "Quickly", "Slowly and clearly", "Quietly"],
    },
];

/// Up to this many takes of one phrase are kept.
pub const MAX_TAKES: usize = 12;
/// A hit counts as the print's owner speaking at this voice similarity, when checking is on. A phrase is
/// under a second of speech, so this is far below what naming a speaker needs.
const VERIFY_MIN: f32 = 0.25;
/// The same phrase heard on two microphones this close together is one hit.
const SAME_HIT_S: f64 = 1.0;
/// Hits from the scan of a session are kept in its folder.
pub const SESSION_FILE: &str = "keyphrases.json";

pub fn spec(id: &str) -> Option<&'static Spec> {
    PHRASES.iter().find(|s| s.id == id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phrase {
    /// Each take as said, then noisy copies of them.
    pub takes: Vec<Template>,
    pub calibration: Option<Calibration>,
    pub updated: u64,
}

impl Phrase {
    pub fn said(&self) -> usize {
        self.takes.iter().filter(|t| !t.noisy).count()
    }
}

/// One person's trained phrases, kept beside their voice print.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trained {
    pub phrases: BTreeMap<String, Phrase>,
}

fn path(print: &str) -> PathBuf {
    library::data_dir().join("keyphrases").join(format!("{print}.json"))
}

pub fn load(print: &str) -> Trained {
    library::read_json(&path(print)).unwrap_or_default()
}

fn save(print: &str, trained: &Trained) -> Result<(), String> {
    let file = path(print);
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    library::write_json(&file, trained)
}

pub fn delete(print: &str) {
    let _ = std::fs::remove_file(path(print));
}

/// Every phrase is calibrated against the person's other phrases, so all change when one does.
fn recalibrate(trained: &mut Trained) {
    let all: Vec<(String, Vec<Template>)> = trained.phrases.iter().map(|(id, p)| (id.clone(), p.takes.clone())).collect();
    for (id, phrase) in trained.phrases.iter_mut() {
        let others: Vec<Template> = all.iter().filter(|(other, _)| other != id).flat_map(|(_, t)| t.iter().cloned()).collect();
        phrase.calibration = kws::calibrate(&phrase.takes, &others);
    }
}

#[derive(Debug, Serialize)]
pub struct TrainResult {
    /// Takes heard in this recording.
    pub heard: usize,
    /// Takes kept for the phrase now.
    pub takes: usize,
    pub separation: Option<f32>,
}

/// Adds the takes in a recording of someone saying `phrase` several times, or replaces the old ones.
pub fn train(print: &str, phrase: &str, pcm: &[i16], replace: bool) -> Result<TrainResult, String> {
    spec(phrase).ok_or("That isn't a key phrase.")?;
    let takes = kws::takes_from(pcm);
    let heard = takes.iter().filter(|t| !t.noisy).count();
    if heard == 0 {
        return Err("No clear takes were heard. Say the phrase with a pause of about a second between takes.".into());
    }
    let mut trained = load(print);
    let entry = trained.phrases.entry(phrase.to_string()).or_insert(Phrase { takes: Vec::new(), calibration: None, updated: 0 });
    if replace {
        entry.takes.clear();
    }
    entry.takes.extend(takes);
    if entry.said() > MAX_TAKES {
        return Err(format!("That makes more than {MAX_TAKES} takes. Record it again from scratch instead."));
    }
    entry.updated = library::now();
    recalibrate(&mut trained);
    save(print, &trained)?;
    let phrase = &trained.phrases[phrase];
    Ok(TrainResult { heard, takes: phrase.said(), separation: phrase.calibration.map(|c| c.separation) })
}

pub fn clear(print: &str, phrase: &str) -> Result<(), String> {
    let mut trained = load(print);
    trained.phrases.remove(phrase);
    recalibrate(&mut trained);
    save(print, &trained)
}

/// How many phrases a print has trained well enough to listen for.
pub fn trained_count(print: &str) -> usize {
    load(print).phrases.values().filter(|p| p.calibration.is_some()).count()
}

/// The phrases to listen for, from every voice print, with the person's sensitivity applied.
pub fn models() -> (Vec<PhraseModel>, Vec<VoicePrint>) {
    let sensitivity = crate::prefs::load().kws_sensitivity();
    let prints = voices::list();
    let mut models = Vec::new();
    for print in &prints {
        for (id, phrase) in load(&print.id).phrases {
            let (Some(spec), Some(calibration)) = (spec(&id), phrase.calibration) else { continue };
            models.push(PhraseModel {
                id,
                owner: print.id.clone(),
                takes: phrase.takes,
                threshold: calibration.threshold * sensitivity,
                requires: spec.requires.map(Into::into),
            });
        }
    }
    (models, prints)
}

/// A phrase that was heard, for the page and the session folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heard {
    pub phrase: String,
    pub text: String,
    /// The voice print whose takes it matched, and their name.
    pub print: String,
    pub who: String,
    pub stream: String,
    pub start_s: f64,
    pub end_s: f64,
    /// Below 1; lower is a closer match.
    pub score: f32,
    /// How long after the phrase ended the spotter was sure.
    pub delay_ms: f32,
    /// How much the phrase sounded like the print's owner, when it was long enough to tell.
    pub speaker: Option<f32>,
    /// Checking the speaker is on and this didn't sound like them, so nothing was done.
    pub rejected: bool,
}

pub fn describe(hit: &kws::Hit, stream: &str, prints: &[VoicePrint], verify: bool) -> Heard {
    let print = prints.iter().find(|p| p.id == hit.owner);
    let speaker = print.and_then(|p| embed::voice(&hit.audio, RATE).map(|v| embed::similarity(&v, &p.voice)));
    Heard {
        phrase: hit.phrase.clone(),
        text: spec(&hit.phrase).map_or(hit.phrase.as_str(), |s| s.text).to_string(),
        print: hit.owner.clone(),
        who: print.map_or(String::new(), |p| p.name.clone()),
        stream: stream.to_string(),
        start_s: hit.start as f64 / RATE as f64,
        end_s: hit.end as f64 / RATE as f64,
        score: hit.score,
        delay_ms: hit.delay_ms(),
        speaker,
        rejected: verify && speaker.is_some_and(|s| s < VERIFY_MIN),
    }
}

/// Whether `heard` is a phrase already heard on another microphone.
fn already_heard(earlier: &[Heard], heard: &Heard) -> bool {
    earlier.iter().any(|h| h.phrase == heard.phrase && h.stream != heard.stream && (h.start_s - heard.start_s).abs() < SAME_HIT_S)
}

/// Every phrase heard in a processed session's streams, in time order.
pub fn scan_session(streams: &[(String, &[i16])]) -> Vec<Heard> {
    let (models, prints) = models();
    if models.is_empty() {
        return Vec::new();
    }
    let verify = crate::prefs::load().kws_verify;
    let mut all: Vec<Heard> =
        streams.iter().flat_map(|(name, pcm)| kws::scan(pcm, models.clone()).into_iter().map(|h| describe(&h, name, &prints, verify)).collect::<Vec<_>>()).collect();
    // Of a phrase heard on several microphones, the closest match is kept.
    all.sort_by(|a, b| a.score.total_cmp(&b.score));
    let mut kept: Vec<Heard> = Vec::new();
    for heard in all {
        if !already_heard(&kept, &heard) {
            kept.push(heard);
        }
    }
    kept.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    kept
}

pub fn load_session(dir: &std::path::Path) -> Vec<Heard> {
    library::read_json(&dir.join(SESSION_FILE)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(phrase: &str, stream: &str, start_s: f64) -> Heard {
        Heard {
            phrase: phrase.into(),
            text: String::new(),
            print: String::new(),
            who: String::new(),
            stream: stream.into(),
            start_s,
            end_s: start_s + 1.0,
            score: 0.5,
            delay_ms: 120.0,
            speaker: None,
            rejected: false,
        }
    }

    #[test]
    fn one_phrase_on_two_microphones_is_one_hit() {
        let earlier = [heard("wake", "input1", 10.0)];
        assert!(already_heard(&earlier, &heard("wake", "pin1", 10.3)));
        assert!(!already_heard(&earlier, &heard("wake", "input1", 10.3)), "the same microphone hears it again");
        assert!(!already_heard(&earlier, &heard("wake", "pin1", 12.0)));
        assert!(!already_heard(&earlier, &heard("bookmark", "pin1", 10.3)));
    }

    #[test]
    fn every_phrase_has_takes_described_and_known_requirements() {
        for s in PHRASES {
            assert!(s.how.len() >= 4, "{}", s.id);
            assert!(s.requires.map_or(true, |r| spec(r).is_some()), "{}", s.id);
        }
    }
}
