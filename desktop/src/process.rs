//! After a session: aligns the streams, attributes speech and writes WAVs and a report.

use crate::keyphrases;
use crate::library::{self, Meta, PrintLink, Segment};
use crate::script;
use crate::transcribe;
use crate::vocabulary;
use crate::voices;
use std::collections::BTreeMap;
use serde_json::json;
use std::fs;
use std::path::Path;
use stv_core::attribute::{self, Config};
use stv_core::merge::{self, Utterance};
use stv_core::{export, pipeline};

/// Every source is processed at this rate, whatever it was captured at.
pub const RATE: u32 = 16_000;
/// 20 ms frames, matching the pin protocol.
pub const FRAME_SAMPLES: usize = 320;
/// An edited line is matched to a reprocessed one when they start this close together.
const SAME_LINE_S: f64 = 0.3;

fn seconds(window: usize) -> f64 {
    (window * pipeline::WINDOW_MS) as f64 / 1000.0
}

pub const COMPACTED: &str = "This meeting's audio was deleted, so it can't be processed again.";

/// A session with journals but no transcript, such as one cut short by a crash. A compacted session
/// never does: its audio was deleted on purpose.
pub fn needs_processing(dir: &Path) -> bool {
    if library::load_meta(dir).compacted {
        return false;
    }
    let has_journal = fs::read_dir(dir)
        .map(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|x| x == "stvj")))
        .unwrap_or(false);
    has_journal && !dir.join("transcript.json").exists()
}

/// Text with no words in it: empty, or only Whisper's markers for non-speech such as
/// "[BLANK_AUDIO]", "(silence)" or "[Music]", or stray punctuation.
pub fn is_silence(text: &str) -> bool {
    let mut words = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => words.push(c),
            _ => {}
        }
    }
    !words.chars().any(char::is_alphanumeric)
}

/// Keeps the person's corrections when a session is processed again: the pipeline is deterministic, so
/// an unchanged recording gives the same lines, and each edited line is carried over to its match.
fn carry_over_edits(old: &[Segment], new: &mut [Segment]) {
    for edited in old.iter().filter(|s| s.edited) {
        let matching = new
            .iter_mut()
            .filter(|s| s.stream == edited.stream && (s.start_s - edited.start_s).abs() < SAME_LINE_S)
            .min_by(|a, b| (a.start_s - edited.start_s).abs().total_cmp(&(b.start_s - edited.start_s).abs()));
        if let Some(s) = matching {
            s.text = edited.text.clone();
            s.speaker = edited.speaker.clone();
            s.edited = true;
            s.fixes = edited.fixes.clone();
            // A line the person put back in the script stays there.
            if edited.in_script() {
                s.echo_of = None;
            }
        }
    }
}

/// Keeps vocabulary changes the person undid or accepted when a session is processed again, on lines that
/// came out the same.
fn carry_over_fixes(old: &[Segment], new: &mut [Segment]) {
    for before in old.iter().filter(|s| !s.edited && !s.fixes.is_empty()) {
        let raw = |s: &Segment| {
            let mut words: Vec<String> = s.text.split_whitespace().map(str::to_string).collect();
            for f in s.fixes.iter().rev().filter(|f| f.applied) {
                words.splice(f.at..(f.at + f.words).min(words.len()), f.raw.split_whitespace().map(str::to_string));
            }
            words.join(" ")
        };
        let before_raw = raw(before);
        if let Some(s) = new.iter_mut().find(|s| !s.edited && s.stream == before.stream && (s.start_s - before.start_s).abs() < SAME_LINE_S && s.text == before_raw) {
            s.text = before.text.clone();
            s.fixes = before.fixes.clone();
        }
    }
}

/// Names speakers the voice prints recognise, and lists who the others might be when it isn't sure.
/// Earlier automatic names are cleared first, so a print made since the last run can take over; names and
/// links the person chose are kept. A track only called after its app, as "Microsoft Teams", is matched
/// like any other: the name says what recorded the voice, not who spoke.
fn name_from_prints(meta: &mut Meta, segments: &[Segment], line_voices: &BTreeMap<usize, stv_core::embed::Voice>) {
    name_with_prints(meta, segments, line_voices, &voices::list());
}

fn name_with_prints(meta: &mut Meta, segments: &[Segment], line_voices: &BTreeMap<usize, stv_core::embed::Voice>, prints: &[voices::VoicePrint]) {
    let automatic: Vec<String> = meta.prints.iter().filter(|(_, l)| l.score.is_some()).map(|(k, _)| k.clone()).collect();
    for key in automatic {
        meta.prints.remove(&key);
        match meta.track_name(&key) {
            Some(track) => meta.names.insert(key, track),
            None => meta.names.remove(&key),
        };
    }
    meta.suggested.clear();
    // Someone already linked by hand is not anyone else in this meeting.
    let linked: Vec<&str> =
        meta.prints.values().filter_map(|l| prints.iter().find(|p| p.id == l.id)).map(|p| p.person()).collect();
    let open: Vec<voices::VoicePrint> = prints.iter().filter(|p| !linked.contains(&p.person())).cloned().collect();
    let mut keys: Vec<&String> = segments.iter().map(|s| &s.speaker).filter(|k| **k != library::UNKNOWN).collect();
    keys.sort();
    keys.dedup();
    let speakers: BTreeMap<String, stv_core::embed::Voice> = keys
        .into_iter()
        .filter(|k| !meta.prints.contains_key(*k) && !meta.named(k))
        .filter_map(|k| voices::speaker_voice(segments, line_voices, k).map(|v| (k.clone(), v)))
        .collect();
    for (key, found) in voices::recognise(&speakers, &open) {
        match found {
            voices::Match::Sure(id, score) => {
                let name = prints.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
                meta.names.insert(key.clone(), name);
                meta.prints.insert(key, PrintLink { id, score: Some(score) });
            }
            voices::Match::Ask(list) => {
                meta.suggested.insert(key, list.into_iter().map(|(id, s)| PrintLink { id, score: Some(s) }).collect());
            }
        }
    }
}

/// Returns a short summary for the screen. `progress` gets a stage name and the fraction done.
pub fn process_session(dir: &Path, progress: &dyn Fn(&str, f32)) -> Result<String, String> {
    // Processing a session whose audio is gone would replace its transcript with an empty one.
    if library::load_meta(dir).compacted {
        return Err(COMPACTED.into());
    }
    progress("Loading audio", 0.0);
    let mut names = Vec::new();
    let mut sources = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries.iter().filter(|p| p.extension().is_some_and(|e| e == "stvj")) {
        let source = pipeline::load_source(path, RATE).map_err(|e| format!("{}: {e}", path.display()))?;
        if !source.pcm.is_empty() {
            names.push(path.file_stem().unwrap().to_string_lossy().into_owned());
            sources.push(source);
        }
    }
    if sources.is_empty() {
        return Err("No audio was recorded.".into());
    }

    progress("Aligning and finding speakers", 0.05);
    pipeline::align_sources(&mut sources, RATE);
    // Tracks added afterwards, placed where they start on the meeting's timeline.
    for import in crate::imports::load(dir) {
        let pcm = crate::imports::placed(dir, &import)?;
        if !pcm.is_empty() {
            names.push(import.name.clone());
            sources.push(pipeline::SourceTrack { pcm, clock: None, frames: 0, lost_frames: 0, duplicate_frames: 0, had_bad_tail: false });
        }
    }
    let window = RATE as usize * pipeline::WINDOW_MS / 1000;
    let levels: Vec<Vec<f32>> = sources.iter().map(|s| attribute::window_db(&s.pcm, window)).collect();
    let attribution = attribute::attribute(&levels, &Config::default());
    // One voice heard on several microphones is one line, transcribed from the clearest of them.
    let utterances: Vec<Utterance> = merge::utterances(&attribution.segments, &levels);
    for (name, source) in names.iter().zip(&sources) {
        export::write_wav(&dir.join(format!("{name}.wav")), &source.pcm, RATE).map_err(|e| e.to_string())?;
    }

    let name_of = |i: Option<usize>| i.map_or("Unknown speaker".to_string(), |i| names[i].clone());

    // The audio is already saved, so a transcription failure is reported without losing the session.
    let samples = |window: usize| window * pipeline::WINDOW_MS * RATE as usize / 1000;
    let mut texts = vec![String::new(); utterances.len()];
    let mut confidences: Vec<f32> = utterances.iter().map(|u| u.confidence).collect();
    // The person's vocabulary steers whisper's spelling, and corrects what it still gets wrong below.
    let terms = vocabulary::load();
    let prompt = vocabulary::prompt("", &terms);
    // The meeting's languages decide the speech model; with two, each line is heard as whichever it
    // sounds like, judged on the clearest microphone that heard it.
    // A meeting processed for the first time starts in the languages the person set for new ones.
    let p = crate::prefs::load();
    let first_time = !library::has_meta(dir);
    let mut item_languages = library::load_meta(dir).langs();
    if item_languages.is_empty() && first_time {
        item_languages = p.new_item_languages();
    }
    let languages = p.effective(&item_languages);
    let mut line_language = vec![String::new(); utterances.len()];
    let transcription = if transcribe::model_ready() {
        transcribe::Transcriber::for_languages(&languages).and_then(|t| {
            let readings: Vec<usize> = utterances.iter().map(|u| u.sources.len().min(merge::MAX_READINGS)).collect();
            let count = readings.iter().sum::<usize>().max(1) as f32;
            let mut done = 0;
            for (i, u) in utterances.iter().enumerate() {
                let (start, end) = (samples(u.start_window), samples(u.end_window));
                let language = match u.sources.first() {
                    Some(&(clearest, _)) => t.detect(&sources[clearest].pcm, start, end, &languages),
                    None => None,
                }
                .unwrap_or_else(|| languages[0].clone());
                // Each microphone that heard it reads it, clearest first; they then vote on the words.
                let mut heard = Vec::new();
                for &(source, clarity) in &u.sources[..readings[i]] {
                    progress("Transcribing", 0.1 + 0.85 * done as f32 / count);
                    done += 1;
                    let words = t.transcribe(&sources[source].pcm, start, end, prompt.as_deref(), &language)?;
                    heard.push((vocabulary::drop_echo(words, &terms), clarity));
                }
                let combined = merge::combine(&heard);
                texts[i] = combined.text;
                if let Some(agreement) = combined.agreement {
                    confidences[i] = merge::boosted(u.confidence, agreement);
                }
                if language != languages[0] {
                    line_language[i] = language;
                }
            }
            Ok("transcript saved")
        })
    } else {
        Ok("no transcript: speech model not downloaded")
    };
    let transcribed = matches!(transcription, Ok("transcript saved"));

    let report = json!({
        "transcribed": transcribed,
        "streams": names.iter().zip(&sources).map(|(name, s)| json!({
            "name": name,
            "seconds": s.pcm.len() as f64 / RATE as f64,
            "frames": s.frames,
            "lost_frames": s.lost_frames,
            "drift_ppm": s.clock.map(|c| c.drift_ppm),
        })).collect::<Vec<_>>(),
        "segments": utterances.iter().zip(&texts).zip(&confidences).map(|((u, text), confidence)| json!({
            "text": text,
            "speaker": name_of(u.speaker_source),
            "audio": names[u.best()],
            "heard_on": u.sources.iter().map(|(s, clarity)| json!({ "stream": names[*s], "clarity_db": clarity })).collect::<Vec<_>>(),
            "start_s": seconds(u.start_window),
            "end_s": seconds(u.end_window),
            "confidence": confidence,
        })).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(dir.join("session.json"), text).map_err(|e| e.to_string())?;

    progress("Recognising voices", 0.95);
    let mut clips: Vec<script::Clip> = utterances
        .iter()
        .zip(texts)
        .zip(confidences)
        .zip(line_language)
        .map(|(((u, text), confidence), language)| {
            let speaker = u.speaker_source.map_or(library::UNKNOWN.to_string(), |i| names[i].clone());
            let segment = Segment {
                speaker,
                stream: names[u.best()].clone(),
                start_s: seconds(u.start_window),
                end_s: seconds(u.end_window),
                text,
                confidence,
                language,
                ..Default::default()
            };
            script::Clip { segment, heard: u.sources.iter().map(|(s, clarity)| (names[*s].clone(), *clarity)).collect() }
        })
        .collect();
    // Only people talking matter: with a transcript, lines where nothing was said are silence or noise,
    // and are dropped before voices are grouped so they cannot become an "Unknown speaker". Where they
    // fall between someone's words, the script notes that something wasn't caught.
    let mut unheard = Vec::new();
    if transcribed {
        clips.retain(|c| {
            let silent = is_silence(&c.segment.text);
            if silent {
                let s = &c.segment;
                unheard.push(script::Unheard { stream: s.stream.clone(), start_s: s.start_s, end_s: s.end_s, clarity_db: c.heard.first().map_or(0.0, |h| h.1) });
            }
            !silent
        });
    }
    clips.sort_by(|a, b| a.segment.start_s.total_cmp(&b.segment.start_s));
    // What one microphone heard more clearly than another is written once; the other's copy stays,
    // greyed out, for the tracks.
    let echoes = script::find_echoes(&mut clips);
    let doubled = echoes.iter().flatten().count();
    let stream_index = |name: &str| names.iter().position(|n| n == name).unwrap_or(0);
    let kept: Vec<usize> = (0..clips.len()).filter(|&i| echoes[i].is_none()).collect();
    let lines: Vec<voices::Line> = kept
        .iter()
        .map(|&i| {
            let segment = &clips[i].segment;
            let pcm = &sources[stream_index(&segment.stream)].pcm;
            let at = |s: f64| ((s * RATE as f64) as usize).min(pcm.len());
            let span = &pcm[at(segment.start_s)..at(segment.end_s)];
            let power = span.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / span.len().max(1) as f64;
            let level_db = (10.0 * (power.max(1.0) / (32768.0f64 * 32768.0)).log10()) as f32;
            voices::Line { speaker: segment.speaker.clone(), voice: stv_core::embed::voice(span, RATE), level_db }
        })
        .collect();
    let assigned = voices::assign(&lines, names.len());
    let mut speaker_of: Vec<String> = clips.iter().map(|c| c.segment.speaker.clone()).collect();
    let mut voice_of: BTreeMap<usize, stv_core::embed::Voice> = BTreeMap::new();
    for ((&i, speaker), line) in kept.iter().zip(assigned).zip(lines) {
        speaker_of[i] = speaker;
        if let Some(v) = line.voice {
            voice_of.insert(i, v);
        }
    }
    // A doubled-up copy belongs to whoever said the line it copies.
    for (i, echo) in echoes.iter().enumerate() {
        if let Some(t) = echo {
            speaker_of[i] = speaker_of[*t].clone();
        }
    }
    let segments: Vec<Segment> = clips
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let also_on = c.heard.iter().skip(1).map(|(s, _)| s.clone()).filter(|s| *s != c.segment.stream).collect();
            Segment { id: i, speaker: speaker_of[i].clone(), echo_of: echoes[i], also_on, ..c.segment }
        })
        .collect();

    // Key phrases first, so a line is never stitched across a bookmark said aloud.
    progress("Listening for key phrases", 0.96);
    let streams: Vec<(String, &[i16])> = names.iter().cloned().zip(sources.iter().map(|s| s.pcm.as_slice())).collect();
    let phrases = keyphrases::scan_session(&streams);
    library::write_json(&dir.join(keyphrases::SESSION_FILE), &phrases)?;
    let mut marks: Vec<f64> = library::load_meta(dir).markers.iter().filter(|m| !m.removed).map(|m| m.at_s).collect();
    marks.extend(phrases.iter().filter(|h| !h.rejected && matches!(h.phrase.as_str(), "bookmark" | "action")).map(|h| h.start_s));

    // Each person's clips on one microphone, a breath apart, become one line.
    let stitched = script::stitch(segments, &unheard, &marks);
    let mut new_id = vec![0; stitched.iter().map(|(_, m)| m.len()).sum()];
    for (k, (_, members)) in stitched.iter().enumerate() {
        for &m in members {
            new_id[m] = k;
        }
    }
    let mut line_voices = BTreeMap::new();
    let mut segments: Vec<Segment> = stitched
        .into_iter()
        .enumerate()
        .map(|(k, (mut segment, members))| {
            segment.id = k;
            segment.echo_of = segment.echo_of.map(|o| new_id[o]);
            // The line sounds like its longest clip.
            if let Some(v) = members.iter().filter_map(|m| voice_of.remove(m)).max_by_key(|v| v.frames) {
                line_voices.insert(k, v);
            }
            segment
        })
        .collect();
    let old = library::load_segments(dir);
    carry_over_edits(&old, &mut segments);
    carry_over_fixes(&old, &mut segments);
    vocabulary::correct_segments(&mut segments, &terms);
    voices::mark_overlaps(&mut segments);

    progress("Saving", 0.98);
    let duration = sources.iter().map(|s| s.pcm.len()).max().unwrap_or(0) as f64 / RATE as f64;
    let mut meta = library::new_meta(dir, duration);
    meta.tracks_removed.clear();
    if first_time && meta.langs().is_empty() {
        meta.set_langs(item_languages);
    }
    name_from_prints(&mut meta, &segments, &line_voices);
    library::write_json(&dir.join(voices::LINE_VOICES), &line_voices)?;
    library::save(dir, &meta, &segments)?;

    let outcome = transcription.map_or_else(|e| format!("transcription failed: {e}"), str::to_string);
    let shared = attribution.segments.len() - utterances.len();
    let script_lines = segments.iter().filter(|s| s.in_script()).count();
    Ok(format!(
        "{} stream(s), {script_lines} line(s) in the script, {shared} joined from more than one microphone before transcribing, {doubled} doubled-up clip(s) greyed out, {outcome}",
        names.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stv_core::journal::JournalWriter;
    use stv_core::protocol::{Codec, Frame};

    fn line(stream: &str, start_s: f64, text: &str, edited: bool) -> Segment {
        Segment {
            id: 0,
            speaker: stream.into(),
            stream: stream.into(),
            start_s,
            end_s: start_s + 1.0,
            text: text.into(),
            confidence: 1.0,
            edited,
            overlap: false,
            ..Default::default()
        }
    }

    #[test]
    fn reprocessing_keeps_edited_lines_only() {
        let old = vec![line("input1", 1.0, "Fixed by hand", true), line("input2", 3.0, "stale", false)];
        let mut new = vec![line("input1", 1.1, "machine text", false), line("input2", 3.0, "new text", false)];
        new[0].speaker = "input1".into();
        let mut renamed = old.clone();
        renamed[0].speaker = "person1".into();
        carry_over_edits(&renamed, &mut new);
        assert_eq!((new[0].text.as_str(), new[0].speaker.as_str(), new[0].edited), ("Fixed by hand", "person1", true));
        assert_eq!(new[1].text, "new text");
    }

    #[test]
    fn an_edit_is_not_moved_to_a_far_or_other_stream_line() {
        let old = vec![line("input1", 1.0, "Fixed", true)];
        let mut new = vec![line("input2", 1.0, "a", false), line("input1", 2.0, "b", false)];
        carry_over_edits(&old, &mut new);
        assert_eq!((new[0].text.as_str(), new[1].text.as_str()), ("a", "b"));
    }

    #[test]
    fn a_session_without_a_transcript_needs_processing() {
        let dir = std::env::temp_dir().join(format!("stv-needs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(!needs_processing(&dir));
        write_journal(&dir.join("input1.stvj"), &[0; FRAME_SAMPLES]);
        assert!(needs_processing(&dir));
        fs::write(dir.join("transcript.json"), "[]").unwrap();
        assert!(!needs_processing(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_compacted_session_is_never_processed() {
        let dir = std::env::temp_dir().join(format!("stv-compacted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let meta = Meta { title: "Kept".into(), ..Meta::default() };
        let segments = vec![Segment { text: "Hello".into(), stream: "input1".into(), speaker: "input1".into(), ..Default::default() }];
        library::save(&dir, &meta, &segments).unwrap();
        write_journal(&dir.join("input1.stvj"), &[0; FRAME_SAMPLES]);
        library::compact(&dir).unwrap();
        assert!(!needs_processing(&dir));
        assert_eq!(process_session(&dir, &|_, _| {}).unwrap_err(), COMPACTED);
        assert_eq!(library::load_segments(&dir), segments);

        // Even a journal left behind, with the transcript gone, isn't offered for processing.
        write_journal(&dir.join("input1.stvj"), &[0; FRAME_SAMPLES]);
        fs::remove_file(dir.join("transcript.json")).unwrap();
        assert!(!needs_processing(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    fn read_wav(path: &str) -> Vec<i16> {
        let data = fs::read(path).unwrap();
        let at = data.windows(4).position(|w| w == b"data").unwrap() + 8;
        data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }

    /// Writes PCM as a journal the way live capture does, with arrival times at real-time pace.
    /// The meeting where a call app's track was never matched, because it was already called "Microsoft
    /// Teams". Both tracks sound most like one print, so the app asks rather than guesses.
    #[test]
    fn a_call_apps_track_is_matched_and_unsure_matches_are_asked_about() {
        let voice = |x: f32, seconds: f32| stv_core::embed::Voice { mean: vec![x, 0.0], var: vec![1.0, 1.0], frames: (seconds * 100.0) as u32 };
        let print = |id: &str, x: f32| voices::VoicePrint {
            id: id.into(), name: id.into(), created: 0, consent_at: 0, voice: voice(x, 30.0), samples: vec![], person: String::new(), label: String::new(),
        };
        let mut meta = Meta {
            inputs: vec!["Microphone (Samson C01U Pro Mic)".into(), "Sound from Microsoft Teams (ms-teams.exe)".into()],
            ..Default::default()
        };
        assert_eq!(meta.track_name("input2").as_deref(), Some("Microsoft Teams"));
        meta.names.insert("input2".into(), "Microsoft Teams".into());
        let seg = |id: usize, speaker: &str| Segment { id, speaker: speaker.into(), stream: speaker.into(), ..Default::default() };
        let segments = vec![seg(0, "input2"), seg(1, "input1")];

        // Clearly one person: the Teams track is named after them.
        let prints = [print("bruce", 0.0), print("fg", 3.0)];
        let line_voices = BTreeMap::from([(0, voice(2.95, 12.0)), (1, voice(0.05, 1.0))]);
        name_with_prints(&mut meta, &segments, &line_voices, &prints);
        assert_eq!(meta.name("input2"), "fg");
        assert_eq!(meta.prints["input2"].id, "fg");
        // A few words on the microphone are asked about, however alike.
        assert_eq!(meta.suggested["input1"].iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["bruce"]);

        // Both sound like the same print: both are asked about, and the Teams track gets its app's name back.
        let line_voices = BTreeMap::from([(0, voice(0.1, 12.0)), (1, voice(0.05, 1.0))]);
        name_with_prints(&mut meta, &segments, &line_voices, &prints);
        assert_eq!(meta.name("input2"), "Microsoft Teams");
        assert!(meta.prints.is_empty());
        assert_eq!(meta.suggested.len(), 2);

        // A name the person chose is kept and not asked about.
        meta.names.insert("input2".into(), "Liam".into());
        name_with_prints(&mut meta, &segments, &line_voices, &prints);
        assert_eq!(meta.name("input2"), "Liam");
        assert!(!meta.suggested.contains_key("input2"));
    }

    fn write_journal(path: &Path, pcm: &[i16]) {
        let mut journal = JournalWriter::create(path).unwrap();
        for (i, chunk) in pcm.chunks_exact(FRAME_SAMPLES).enumerate() {
            let frame = Frame {
                flags: 0,
                pin_session_id: 1,
                sequence: i as u32,
                sample_counter: (i * FRAME_SAMPLES) as u32,
                codec: Codec::RawPcm16,
                payload: chunk.iter().flat_map(|s| s.to_le_bytes()).collect(),
            };
            let arrival = (i * FRAME_SAMPLES) as u64 * 1_000_000 / RATE as u64;
            journal.append(arrival as _, &frame.encode()).unwrap();
        }
        journal.sync().unwrap();
    }

    /// Two people taking turns, each with their own mic that also picks up the other at -20 dB.
    /// Needs the model and two 16 kHz mono WAVs in STV_TEST_WAV and STV_TEST_WAV2. Set STV_DEMO_KEEP
    /// to leave the session in the library for trying the editor.
    #[test]
    #[ignore]
    fn two_worn_mics_attribute_each_speaker() {
        let (a, b) = (read_wav(&std::env::var("STV_TEST_WAV").unwrap()), read_wav(&std::env::var("STV_TEST_WAV2").unwrap()));
        let gap = RATE as usize / 2;
        let total = gap + a.len() + gap + b.len() + gap + a.len() + gap;
        let mut voice_a = vec![0i32; total];
        let mut voice_b = vec![0i32; total];
        let b_at = gap + a.len() + gap;
        let a2_at = b_at + b.len() + gap;
        for (i, &s) in a.iter().enumerate() {
            voice_a[gap + i] = s as i32;
            voice_a[a2_at + i] = s as i32;
        }
        for (i, &s) in b.iter().enumerate() {
            voice_b[b_at + i] = s as i32;
        }
        // A little room noise, so each mic has a realistic floor.
        let noise = |i: usize| ((i.wrapping_mul(2_654_435_761) >> 7) % 61) as i32 - 30;
        let mic = |own: &[i32], other: &[i32]| -> Vec<i16> {
            (0..total).map(|i| (own[i] + other[i] / 10 + noise(i)).clamp(-32768, 32767) as i16).collect()
        };

        let keep = std::env::var("STV_DEMO_KEEP").is_ok();
        let dir = if keep { library::new_session_dir().unwrap() } else { std::env::temp_dir().join(format!("stv-two-{}", std::process::id())) };
        fs::create_dir_all(&dir).unwrap();
        write_journal(&dir.join("input1.stvj"), &mic(&voice_a, &voice_b));
        write_journal(&dir.join("input2.stvj"), &mic(&voice_b, &voice_a));
        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let segments = library::load_segments(&dir);
        for s in &segments {
            println!("{:>6.2}-{:>6.2} {:<8} {:.2} {}", s.start_s, s.end_s, s.speaker, s.confidence, s.text);
        }
        if keep {
            let mut meta = library::load_meta(&dir);
            meta.title = "Demo: two voices".into();
            library::save(&dir, &meta, &segments).unwrap();
            println!("kept {}", dir.display());
        } else {
            let _ = fs::remove_dir_all(&dir);
        }
        let speakers: Vec<&str> = segments.iter().filter(|s| !s.text.is_empty()).map(|s| s.speaker.as_str()).collect();
        assert_eq!(speakers.first(), Some(&"input1"));
        assert!(speakers.contains(&"input2"));
        assert!(!speakers.contains(&library::UNKNOWN), "{speakers:?}");
    }

    /// Two people sitting side by side, each with a microphone that hears the other almost as loudly (2 dB
    /// down, a fraction of a millisecond later), taking turns. Every thing said must appear once.
    /// Needs the model and two 16 kHz mono WAVs in STV_TEST_WAV and STV_TEST_WAV2.
    #[test]
    #[ignore]
    fn side_by_side_microphones_write_each_thing_once() {
        let (a, b) = (read_wav(&std::env::var("STV_TEST_WAV").unwrap()), read_wav(&std::env::var("STV_TEST_WAV2").unwrap()));
        let gap = RATE as usize / 2;
        let total = gap + a.len() + gap + b.len() + gap + a.len() + gap;
        let (mut voice_a, mut voice_b) = (vec![0i32; total], vec![0i32; total]);
        let b_at = gap + a.len() + gap;
        let a2_at = b_at + b.len() + gap;
        for (i, &s) in a.iter().enumerate() {
            voice_a[gap + i] = s as i32;
            voice_a[a2_at + i] = s as i32;
        }
        for (i, &s) in b.iter().enumerate() {
            voice_b[b_at + i] = s as i32;
        }
        let noise = |i: usize| ((i.wrapping_mul(2_654_435_761) >> 7) % 61) as i32 - 30;
        // 2 dB down and 5 samples late on the other microphone.
        let mic = |own: &[i32], other: &[i32]| -> Vec<i16> {
            (0..total).map(|i| (own[i] + other[i.saturating_sub(5)] * 79 / 100 + noise(i)).clamp(-32768, 32767) as i16).collect()
        };
        let dir = std::env::temp_dir().join(format!("stv-side-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_journal(&dir.join("input1.stvj"), &mic(&voice_a, &voice_b));
        write_journal(&dir.join("input2.stvj"), &mic(&voice_b, &voice_a));
        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let segments = library::load_segments(&dir);
        for s in &segments {
            println!("{:>6.2}-{:>6.2} {:<8} {:<8} {:.2} {}", s.start_s, s.end_s, s.speaker, s.stream, s.confidence, s.text);
        }
        let _ = fs::remove_dir_all(&dir);
        for (i, x) in segments.iter().enumerate() {
            for y in &segments[i + 1..] {
                let overlap = x.end_s.min(y.end_s) - x.start_s.max(y.start_s);
                assert!(!(overlap > 0.0 && merge::shared_words(&x.text, &y.text) >= 0.5), "said twice:\n{x:?}\n{y:?}");
            }
        }
        let all: String = segments.iter().map(|s| s.text.to_lowercase()).collect::<Vec<_>>().join(" ");
        assert_eq!(all.matches("friday").count(), 2, "{all}");
        assert_eq!(all.matches("thursday").count(), 1, "{all}");
    }

    /// Two people side by side talking over each other, each heard 6 dB down on the other's microphone:
    /// both must be written down, once each. Needs the same as the test above.
    #[test]
    #[ignore]
    fn side_by_side_microphones_keep_people_talking_at_once() {
        let (a, b) = (read_wav(&std::env::var("STV_TEST_WAV").unwrap()), read_wav(&std::env::var("STV_TEST_WAV2").unwrap()));
        let gap = RATE as usize / 2;
        let b_at = gap + a.len() / 2;
        let total = (gap + a.len()).max(b_at + b.len()) + gap;
        let (mut voice_a, mut voice_b) = (vec![0i32; total], vec![0i32; total]);
        for (i, &s) in a.iter().enumerate() {
            voice_a[gap + i] = s as i32;
        }
        for (i, &s) in b.iter().enumerate() {
            voice_b[b_at + i] = s as i32;
        }
        let noise = |i: usize| ((i.wrapping_mul(2_654_435_761) >> 7) % 61) as i32 - 30;
        let mic = |own: &[i32], other: &[i32]| -> Vec<i16> {
            (0..total).map(|i| (own[i] + other[i.saturating_sub(5)] / 2 + noise(i)).clamp(-32768, 32767) as i16).collect()
        };
        let dir = std::env::temp_dir().join(format!("stv-at-once-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_journal(&dir.join("input1.stvj"), &mic(&voice_a, &voice_b));
        write_journal(&dir.join("input2.stvj"), &mic(&voice_b, &voice_a));
        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let segments = library::load_segments(&dir);
        for s in &segments {
            println!("{:>6.2}-{:>6.2} {:<8} {:.2} {}", s.start_s, s.end_s, s.stream, s.confidence, s.text);
        }
        let _ = fs::remove_dir_all(&dir);
        let all: String = segments.iter().map(|s| s.text.to_lowercase()).collect::<Vec<_>>().join(" ");
        assert_eq!(all.matches("friday").count(), 1, "{all}");
        assert_eq!(all.matches("thursday").count(), 1, "{all}");
    }

    /// One microphone hearing two people take turns: voices alone must tell them apart.
    /// Needs two 16 kHz mono WAVs of different voices in STV_TEST_WAV and STV_TEST_WAV2.
    #[test]
    #[ignore]
    fn one_microphone_tells_two_voices_apart() {
        let (a, b) = (read_wav(&std::env::var("STV_TEST_WAV").unwrap()), read_wav(&std::env::var("STV_TEST_WAV2").unwrap()));
        let gap = vec![0i16; RATE as usize];
        let pcm: Vec<i16> = [&a[..], &gap, &b, &gap, &a, &gap, &b].concat();
        let keep = std::env::var("STV_DEMO_KEEP").is_ok();
        let dir = if keep { library::new_session_dir().unwrap() } else { std::env::temp_dir().join(format!("stv-one-mic-{}", std::process::id())) };
        fs::create_dir_all(&dir).unwrap();
        write_journal(&dir.join("input1.stvj"), &pcm);
        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let segments = library::load_segments(&dir);
        let b_start = (a.len() + gap.len()) as f64 / RATE as f64;
        let a2_start = b_start + (b.len() + gap.len()) as f64 / RATE as f64;
        let b2_start = a2_start + (a.len() + gap.len()) as f64 / RATE as f64;
        // Equally loud voices on one microphone: either may be called the wearer, but each keeps one label.
        let a_turn = |s: &Segment| s.start_s < b_start - 0.5 || (s.start_s > a2_start - 0.5 && s.start_s < b2_start - 0.5);
        let a_label = segments.iter().find(|s| a_turn(s)).unwrap().speaker.clone();
        let mut wrong = 0;
        for s in &segments {
            println!("{:>6.2}-{:>6.2} {:<8} {} {}", s.start_s, s.end_s, s.speaker, if a_turn(s) { "A" } else { "B" }, s.text);
            wrong += usize::from(a_turn(s) != (s.speaker == a_label));
        }
        if keep {
            let mut meta = library::load_meta(&dir);
            meta.title = "Demo: one microphone, two voices".into();
            library::save(&dir, &meta, &segments).unwrap();
        } else {
            let _ = fs::remove_dir_all(&dir);
        }
        let labels: std::collections::BTreeSet<&str> = segments.iter().map(|s| s.speaker.as_str()).collect();
        assert_eq!(labels.len(), 2, "{labels:?}");
        assert!(wrong * 10 <= segments.len(), "{wrong} of {} lines went to the wrong voice", segments.len());
    }

    /// A voice print made from one passage names that person when they say something else.
    /// STV_TEST_WAV and STV_TEST_WAV2 are two voices; STV_TEST_ENROL is the first voice reading other text.
    #[test]
    #[ignore]
    fn a_voice_print_names_its_speaker() {
        let base = std::env::temp_dir().join(format!("stv-print-{}", std::process::id()));
        library::init(base.join("sessions"), base.join("data"));
        let enrol = read_wav(&std::env::var("STV_TEST_ENROL").unwrap());
        let voice = stv_core::embed::voice(&enrol, RATE).unwrap();
        voices::add_sample(None, "Alice", voice, "script").unwrap();

        let (a, b) = (read_wav(&std::env::var("STV_TEST_WAV").unwrap()), read_wav(&std::env::var("STV_TEST_WAV2").unwrap()));
        let pcm: Vec<i16> = [&a[..], &vec![0i16; RATE as usize], &b].concat();
        let dir = library::new_session_dir().unwrap();
        write_journal(&dir.join("input1.stvj"), &pcm);
        process_session(&dir, &|_, _| {}).unwrap();
        let meta = library::load_meta(&dir);
        let first = &library::load_segments(&dir)[0];
        println!("names {:?} prints {:?}", meta.names, meta.prints);
        let _ = fs::remove_dir_all(&base);
        assert_eq!(meta.name(&first.speaker), "Alice");
        assert_eq!(meta.names.values().filter(|n| *n == "Alice").count(), 1);
    }

    /// Processes a copy of a recorded session, such as one shared in examples/, and prints its script and
    /// the clips greyed out as doubles: set STV_EXAMPLE to the session folder and run with --ignored.
    #[test]
    #[ignore]
    fn an_example_session_is_one_script() {
        let from = std::path::PathBuf::from(std::env::var("STV_EXAMPLE").unwrap());
        let dir = std::env::temp_dir().join(format!("stv-example-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for entry in fs::read_dir(&from).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "stvj") || path.file_name().is_some_and(|n| n == "meta.json") {
                fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
            }
        }
        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let segments = library::load_segments(&dir);
        let meta = library::load_meta(&dir);
        for s in &segments {
            let parts = if s.parts.len() > 1 { format!(" ({} clips)", s.parts.len()) } else { String::new() };
            match s.echo_of {
                None => println!("{:>6.2}-{:>6.2} {:<7} {}{parts}: {}", s.start_s, s.end_s, s.stream, meta.name(&s.speaker), s.text),
                Some(t) => println!("   {:>6.2}-{:>6.2} {:<7} greyed, doubles line {t}: {}", s.start_s, s.end_s, s.stream, s.text),
            }
        }
        println!("recognised {:?}\nasking about {:?}", meta.prints, meta.suggested);
        // STV_EXAMPLE_KEEP keeps the processed copy there, to look at in the app or a browser.
        match std::env::var("STV_EXAMPLE_KEEP") {
            Ok(keep) => {
                let _ = fs::remove_dir_all(&keep);
                fs::rename(&dir, &keep).unwrap();
            }
            Err(_) => {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }

    /// Needs the model and a 16 kHz mono 16-bit WAV: set STV_TEST_WAV and run with --ignored.
    #[test]
    #[ignore]
    fn a_recorded_session_produces_a_transcript() {
        let pcm = read_wav(&std::env::var("STV_TEST_WAV").unwrap());
        let dir = std::env::temp_dir().join(format!("stv-session-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_journal(&dir.join("input1.stvj"), &pcm);

        println!("{}", process_session(&dir, &|_, _| {}).unwrap());
        let text = fs::read_to_string(dir.join("transcript.txt")).unwrap();
        println!("{text}");
        let expected = std::env::var("STV_TEST_EXPECT").unwrap().to_lowercase();
        let found = text.to_lowercase().contains(&expected);
        let _ = fs::remove_dir_all(&dir);
        assert!(found);
    }
}
