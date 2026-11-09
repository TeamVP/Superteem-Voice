//! Voice prints and working out who spoke each line.
//!
//! Every line long enough gets a voice. Lines are grouped into speakers by how alike they sound; each
//! microphone's wearer is the group that mostly came from their microphone, and the other groups are
//! guests. Groups are then compared with the saved voice prints to put names to them. The person can move
//! a line, and the app offers to move the lines that sound like it too.

use crate::library::{self, Segment, UNKNOWN};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use stv_core::embed::{self, Voice};

const PRINTS: &str = "voiceprints.json";
/// Per-line voices in a session folder, by line id, for moving similar lines and making prints.
pub const LINE_VOICES: &str = "voices.json";

/// Lines shorter than this sound too unlike themselves to start a group; they join the nearest one.
const CLUSTER_MIN_S: f32 = 1.5;
/// Groups this alike are the same person. Distinct voices in 3 s clips rarely reach it.
const SAME_PERSON: f32 = 0.55;
/// A short line joins a group at this similarity, or keeps the microphone's answer.
const JOIN: f32 = 0.45;
/// A group with less speech than this is not called a new guest.
const GUEST_MIN_S: f32 = 3.0;
/// A voice this much louder than the most-heard one on a microphone is its wearer.
const LOUDER_DB: f32 = 4.0;
/// A wearer's group must hold at least this share of speech from their own microphone.
const WEARER_SHARE: f32 = 0.4;
/// Recognised as a voice print at this similarity.
pub const RECOGNISE: f32 = 0.6;
/// "Sounds like this line" when offering to move other lines.
pub const SIMILAR_LINE: f32 = 0.6;
/// Enough voiced speech to start a voice print.
pub const PRINT_MIN_S: f32 = 8.0;
/// Enough to add to an existing voice print.
pub const ADD_MIN_S: f32 = 2.0;
/// Overlap shorter than this is a handover, not talking over each other.
const OVERLAP_S: f64 = 0.2;
/// Only the longest lines are grouped one by one, which keeps long sessions quick.
const MAX_CLUSTERED: usize = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    /// "script" for a read script, or the session it came from.
    pub source: String,
    pub seconds: f32,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePrint {
    pub id: String,
    pub name: String,
    pub created: u64,
    /// When the person agreed to have their voice print kept on this device.
    pub consent_at: u64,
    pub voice: Voice,
    pub samples: Vec<Sample>,
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn list() -> Vec<VoicePrint> {
    library::read_json(&library::data_dir().join(PRINTS)).unwrap_or_default()
}

pub fn edit<T>(change: impl FnOnce(&mut Vec<VoicePrint>) -> Result<T, String>) -> Result<T, String> {
    let _guard = LOCK.lock().unwrap();
    let mut prints = list();
    let out = change(&mut prints)?;
    std::fs::create_dir_all(library::data_dir()).map_err(|e| e.to_string())?;
    library::write_json(&library::data_dir().join(PRINTS), &prints)?;
    Ok(out)
}

/// Adds speech to a voice print, or starts a new one when `id` is None. Returns its id.
pub fn add_sample(id: Option<&str>, name: &str, voice: Voice, source: &str) -> Result<String, String> {
    let needed = if id.is_some() { ADD_MIN_S } else { PRINT_MIN_S };
    if voice.seconds() < needed {
        return Err(format!(
            "Only {:.0} seconds of clear speech; at least {:.0} are needed. Try reading more of the script.",
            voice.seconds(),
            needed
        ));
    }
    let sample = Sample { source: source.into(), seconds: voice.seconds(), at: library::now() };
    edit(|prints| match id {
        Some(id) => {
            let print = prints.iter_mut().find(|p| p.id == id).ok_or("That voice print no longer exists.")?;
            print.voice = embed::merge(&print.voice, &voice);
            print.samples.push(sample);
            Ok(print.id.clone())
        }
        None => {
            let name = name.trim();
            if name.is_empty() {
                return Err("Give the voice print a name.".into());
            }
            let now = library::now();
            let id = (0..).map(|n| format!("vp-{now}-{n}")).find(|id| prints.iter().all(|p| &p.id != id)).unwrap();
            prints.push(VoicePrint { id: id.clone(), name: name.into(), created: now, consent_at: now, voice, samples: vec![sample] });
            Ok(id)
        }
    })
}

pub fn load_line_voices(dir: &Path) -> BTreeMap<usize, Voice> {
    library::read_json(&dir.join(LINE_VOICES)).unwrap_or_default()
}

/// The voice of all of one speaker's lines together.
pub fn speaker_voice(segments: &[Segment], voices: &BTreeMap<usize, Voice>, speaker: &str) -> Option<Voice> {
    segments.iter().filter(|s| s.speaker == speaker).filter_map(|s| voices.get(&s.id)).cloned().reduce(|a, b| embed::merge(&a, &b))
}

/// What processing found for one line before voices are considered.
pub struct Line {
    /// The microphone's answer: a stream for its wearer, or UNKNOWN.
    pub speaker: String,
    pub voice: Option<Voice>,
    /// Loudness in dBFS, as the wearer is the loudest voice on their own microphone.
    pub level_db: f32,
}

struct Group {
    voice: Voice,
    members: Vec<usize>,
    /// Loudness summed over seconds, for the speech-weighted average.
    level_seconds: f32,
    /// Speech seconds per microphone answer.
    seconds: BTreeMap<String, f32>,
}

impl Group {
    fn total(&self) -> f32 {
        self.seconds.values().sum()
    }

    fn level(&self) -> f32 {
        self.level_seconds / self.total().max(1e-3)
    }

    /// The wearer most of this group's speech came from, if any.
    fn main_wearer(&self) -> Option<(&str, f32)> {
        self.seconds.iter().filter(|(k, _)| *k != UNKNOWN).max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, v)| (k.as_str(), *v))
    }
}

/// Picks each line's speaker. `streams` is how many microphones recorded: with one, the microphone
/// says nothing about who spoke, so only voices decide; with several, a group that came almost entirely
/// from one microphone belongs to its wearer even when it sounds a little different.
pub fn assign(lines: &[Line], streams: usize) -> Vec<String> {
    let mut out: Vec<String> = lines.iter().map(|l| l.speaker.clone()).collect();
    let mut long: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].voice.as_ref().is_some_and(|v| v.seconds() >= CLUSTER_MIN_S)).collect();
    long.sort_by(|&a, &b| lines[b].voice.as_ref().unwrap().frames.cmp(&lines[a].voice.as_ref().unwrap().frames));
    long.truncate(MAX_CLUSTERED);
    long.sort();
    if long.is_empty() {
        return out;
    }

    let mut groups: Vec<Group> = long
        .iter()
        .map(|&i| {
            let voice = lines[i].voice.clone().unwrap();
            let seconds = voice.seconds();
            let level_seconds = lines[i].level_db * seconds;
            Group { seconds: BTreeMap::from([(lines[i].speaker.clone(), seconds)]), level_seconds, voice, members: vec![i] }
        })
        .collect();
    // Two groups from different wearers' microphones stay apart: the microphones already told them apart.
    let compatible = |a: &Group, b: &Group| match (a.main_wearer(), b.main_wearer()) {
        (Some((x, _)), Some((y, _))) if streams > 1 => x == y,
        _ => true,
    };
    let mut sim: Vec<Vec<f32>> = groups.iter().map(|a| groups.iter().map(|b| embed::similarity(&a.voice, &b.voice)).collect()).collect();
    let mut alive = vec![true; groups.len()];
    loop {
        let mut best = (SAME_PERSON, usize::MAX, usize::MAX);
        for a in 0..groups.len() {
            if !alive[a] {
                continue;
            }
            for b in a + 1..groups.len() {
                if alive[b] && sim[a][b] >= best.0 && compatible(&groups[a], &groups[b]) {
                    best = (sim[a][b], a, b);
                }
            }
        }
        let (_, a, b) = best;
        if a == usize::MAX {
            break;
        }
        let taken = std::mem::replace(&mut groups[b].members, Vec::new());
        groups[a].members.extend(taken);
        let seconds = std::mem::take(&mut groups[b].seconds);
        for (k, v) in seconds {
            *groups[a].seconds.entry(k).or_default() += v;
        }
        groups[a].level_seconds += groups[b].level_seconds;
        groups[a].voice = embed::merge(&groups[a].voice, &groups[b].voice);
        alive[b] = false;
        for c in 0..groups.len() {
            if alive[c] {
                let s = embed::similarity(&groups[a].voice, &groups[c].voice);
                sim[a][c] = s;
                sim[c][a] = s;
            }
        }
    }
    let groups: Vec<Group> = groups.into_iter().zip(alive).filter_map(|(g, a)| a.then_some(g)).collect();

    // Each wearer takes their biggest group, the most-heard wearers first. A clearly louder group wins
    // instead, since the wearer's mouth is nearest their microphone.
    let mut labels: Vec<Option<String>> = vec![None; groups.len()];
    let mut wearers: BTreeMap<&str, f32> = BTreeMap::new();
    for l in lines.iter().filter(|l| l.speaker != UNKNOWN) {
        *wearers.entry(&l.speaker).or_default() += l.voice.as_ref().map_or(0.0, Voice::seconds);
    }
    let mut order: Vec<(&str, f32)> = wearers.into_iter().collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (wearer, _) in order {
        let own = |g: usize| groups[g].seconds.get(wearer).copied().unwrap_or(0.0);
        let candidates: Vec<usize> = (0..groups.len())
            .filter(|&g| labels[g].is_none() && own(g) > 0.0 && own(g) >= WEARER_SHARE * groups[g].total())
            .collect();
        let biggest = candidates.iter().copied().max_by(|&a, &b| own(a).total_cmp(&own(b)));
        let loudest = candidates
            .iter()
            .copied()
            .filter(|&g| groups[g].total() >= GUEST_MIN_S)
            .max_by(|&a, &b| groups[a].level().total_cmp(&groups[b].level()));
        let pick = match (biggest, loudest) {
            (Some(b), Some(l)) if groups[l].level() - groups[b].level() >= LOUDER_DB => Some(l),
            (b, _) => b,
        };
        if let Some(g) = pick {
            labels[g] = Some(wearer.to_string());
        }
    }
    let mut guests = 0;
    let mut by_start: Vec<usize> = (0..groups.len()).collect();
    by_start.sort_by_key(|&g| groups[g].members.iter().min().copied());
    for g in by_start {
        if labels[g].is_some() {
            continue;
        }
        let group = &groups[g];
        let mostly_one_wearer = group.main_wearer().filter(|&(_, s)| streams > 1 && s >= 0.8 * group.total());
        if let Some((wearer, _)) = mostly_one_wearer {
            labels[g] = Some(wearer.to_string());
        } else if group.total() >= GUEST_MIN_S {
            guests += 1;
            labels[g] = Some(format!("guest{guests}"));
        }
    }
    for (group, label) in groups.iter().zip(&labels) {
        if let Some(label) = label {
            for &i in &group.members {
                out[i] = label.clone();
            }
        }
    }

    // Short lines join the group they sound most like, when it is close enough.
    let named: Vec<(&Group, &String)> = groups.iter().zip(&labels).filter_map(|(g, l)| l.as_ref().map(|l| (g, l))).collect();
    for (i, line) in lines.iter().enumerate() {
        let clustered = long.binary_search(&i).is_ok();
        let Some(voice) = line.voice.as_ref().filter(|_| !clustered) else { continue };
        let best = named.iter().map(|(g, l)| (embed::similarity(voice, &g.voice), *l)).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, label)) = best.filter(|b| b.0 >= JOIN) {
            out[i] = label.clone();
        }
    }
    out
}

/// Marks lines spoken while someone else was also speaking.
pub fn mark_overlaps(segments: &mut [Segment]) {
    // Doubled-up copies of a line are the same voice again, not someone else talking.
    let spans: Vec<(f64, f64, String)> = segments.iter().filter(|s| s.in_script()).map(|s| (s.start_s, s.end_s, s.speaker.clone())).collect();
    for s in segments.iter_mut() {
        s.overlap = s.in_script() && spans.iter().any(|(start, end, who)| *who != s.speaker && end.min(s.end_s) - start.max(s.start_s) > OVERLAP_S);
    }
}

/// Puts names to speakers from the voice prints: each print and each speaker is used once, best matches
/// first. Returns speaker key to (print id, similarity).
pub fn recognise(speakers: &BTreeMap<String, Voice>, prints: &[VoicePrint]) -> BTreeMap<String, (String, f32)> {
    let mut pairs: Vec<(f32, &String, &VoicePrint)> = speakers
        .iter()
        .flat_map(|(key, voice)| prints.iter().map(move |p| (embed::similarity(voice, &p.voice), key, p)))
        .filter(|(s, _, _)| *s >= RECOGNISE)
        .collect();
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = BTreeMap::new();
    let mut used = Vec::new();
    for (score, key, print) in pairs {
        if !out.contains_key(key) && !used.contains(&&print.id) {
            used.push(&print.id);
            out.insert(key.clone(), (print.id.clone(), score));
        }
    }
    out
}

/// Lines still with `from` that sound like `like`, which the person may want to move as well.
pub fn similar_lines(segments: &[Segment], voices: &BTreeMap<usize, Voice>, like: &Voice, from: &str) -> Vec<usize> {
    segments
        .iter()
        .filter(|s| s.speaker == from && !s.edited)
        .filter(|s| voices.get(&s.id).is_some_and(|v| embed::similarity(v, like) >= SIMILAR_LINE))
        .map(|s| s.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(x: f32, seconds: f32) -> Option<Voice> {
        Some(Voice { mean: vec![x, 0.0], var: vec![1.0, 1.0], frames: (seconds * 100.0) as u32 })
    }

    fn line(speaker: &str, x: f32, seconds: f32) -> Line {
        Line { speaker: speaker.into(), voice: voice(x, seconds), level_db: -30.0 }
    }

    #[test]
    fn one_microphone_with_two_voices_finds_a_guest() {
        let lines = vec![line("input1", 0.0, 4.0), line("input1", 3.0, 4.0), line("input1", 0.1, 4.0), line("input1", 3.1, 2.0)];
        assert_eq!(assign(&lines, 1), ["input1", "guest1", "input1", "guest1"]);
    }

    #[test]
    fn on_one_microphone_the_clearly_louder_voice_is_the_wearer() {
        let mut lines = vec![line("input1", 0.0, 4.0), line("input1", 3.0, 8.0)];
        lines[0].level_db = -20.0;
        assert_eq!(assign(&lines, 1), ["input1", "guest1"]);
    }

    #[test]
    fn wearers_on_different_microphones_are_never_merged() {
        // Alike voices, but each came from its own microphone.
        let lines = vec![line("input1", 0.0, 4.0), line("input2", 0.05, 4.0), line(UNKNOWN, 3.0, 4.0)];
        assert_eq!(assign(&lines, 2), ["input1", "input2", "guest1"]);
    }

    #[test]
    fn short_lines_join_the_closest_voice_or_keep_the_microphone_answer() {
        let lines = vec![line("input1", 0.0, 4.0), line(UNKNOWN, 3.0, 4.0), line("input1", 3.0, 0.5), line(UNKNOWN, 9.0, 0.5), Line { speaker: UNKNOWN.into(), voice: None, level_db: -30.0 }];
        assert_eq!(assign(&lines, 1), ["input1", "guest1", "guest1", UNKNOWN, UNKNOWN]);
    }

    #[test]
    fn each_print_names_one_speaker() {
        let print = |id: &str, x: f32| VoicePrint { id: id.into(), name: id.into(), created: 0, consent_at: 0, voice: voice(x, 30.0).unwrap(), samples: vec![] };
        let speakers = BTreeMap::from([("input1".to_string(), voice(0.0, 10.0).unwrap()), ("guest1".to_string(), voice(0.1, 10.0).unwrap())]);
        let found = recognise(&speakers, &[print("alice", 0.05), print("bob", 9.0)]);
        assert_eq!(found.len(), 1);
        assert!(found.values().all(|(id, _)| id == "alice"));
    }

    #[test]
    fn overlapping_lines_from_different_speakers_are_marked() {
        let seg = |speaker: &str, start_s: f64, end_s: f64| Segment { id: 0, speaker: speaker.into(), stream: "a".into(), start_s, end_s, text: String::new(), confidence: 1.0, edited: false, overlap: false, ..Default::default() };
        let mut segments = vec![seg("a", 0.0, 2.0), seg("b", 1.5, 3.0), seg("b", 3.0, 4.0), seg("a", 3.9, 5.0)];
        mark_overlaps(&mut segments);
        assert_eq!(segments.iter().map(|s| s.overlap).collect::<Vec<_>>(), [true, true, false, false]);
    }
}
