//! After a session: aligns the streams, attributes speech and writes WAVs and a report.

use crate::keyphrases;
use crate::library::{self, Meta, PrintLink, Segment};
use crate::transcribe;
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

/// A session with journals but no transcript, such as one cut short by a crash.
pub fn needs_processing(dir: &Path) -> bool {
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
        }
    }
}

/// Names speakers the voice prints recognise. Earlier automatic names are cleared first, so a print made
/// since the last run can take over; names and links the person chose are kept.
fn name_from_prints(meta: &mut Meta, segments: &[Segment], line_voices: &BTreeMap<usize, stv_core::embed::Voice>) {
    let automatic: Vec<String> = meta.prints.iter().filter(|(_, l)| l.score.is_some()).map(|(k, _)| k.clone()).collect();
    for key in automatic {
        meta.prints.remove(&key);
        meta.names.remove(&key);
    }
    let prints = voices::list();
    let linked: Vec<&String> = meta.prints.values().map(|l| &l.id).collect();
    let open: Vec<voices::VoicePrint> = prints.iter().filter(|p| !linked.contains(&&p.id)).cloned().collect();
    let mut keys: Vec<&String> = segments.iter().map(|s| &s.speaker).filter(|k| **k != library::UNKNOWN).collect();
    keys.sort();
    keys.dedup();
    let speakers: BTreeMap<String, stv_core::embed::Voice> = keys
        .into_iter()
        .filter(|k| !meta.prints.contains_key(*k) && !meta.names.contains_key(*k))
        .filter_map(|k| voices::speaker_voice(segments, line_voices, k).map(|v| (k.clone(), v)))
        .collect();
    for (key, (id, score)) in voices::recognise(&speakers, &open) {
        let name = prints.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        meta.names.insert(key.clone(), name);
        meta.prints.insert(key, PrintLink { id, score: Some(score) });
    }
}

/// Returns a short summary for the screen. `progress` gets a stage name and the fraction done.
pub fn process_session(dir: &Path, progress: &dyn Fn(&str, f32)) -> Result<String, String> {
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
    let transcription = if transcribe::model_ready() {
        transcribe::Transcriber::load().and_then(|t| {
            let readings: Vec<usize> = utterances.iter().map(|u| u.sources.len().min(merge::MAX_READINGS)).collect();
            let count = readings.iter().sum::<usize>().max(1) as f32;
            let mut done = 0;
            for (i, u) in utterances.iter().enumerate() {
                // Each microphone that heard it reads it, clearest first; they then vote on the words.
                let mut heard = Vec::new();
                for &(source, clarity) in &u.sources[..readings[i]] {
                    progress("Transcribing", 0.1 + 0.85 * done as f32 / count);
                    done += 1;
                    heard.push((t.transcribe(&sources[source].pcm, samples(u.start_window), samples(u.end_window))?, clarity));
                }
                let combined = merge::combine(&heard);
                texts[i] = combined.text;
                if let Some(agreement) = combined.agreement {
                    confidences[i] = merge::boosted(u.confidence, agreement);
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
    let mut lines: Vec<(Segment, f32)> = utterances
        .iter()
        .zip(texts)
        .zip(confidences)
        .map(|((u, text), confidence)| {
            let speaker = u.speaker_source.map_or(library::UNKNOWN.to_string(), |i| names[i].clone());
            let segment = Segment {
                id: 0,
                speaker,
                stream: names[u.best()].clone(),
                start_s: seconds(u.start_window),
                end_s: seconds(u.end_window),
                text,
                confidence,
                edited: false,
                overlap: false,
            };
            (segment, u.sources[0].1)
        })
        .collect();
    // Only people talking matter: with a transcript, lines where nothing was said are silence or noise,
    // and are dropped before voices are grouped so they cannot become an "Unknown speaker".
    if transcribed {
        lines.retain(|(s, _)| !is_silence(&s.text));
    }
    let joined = join_same_words(&mut lines);
    let stream_index = |name: &str| names.iter().position(|n| n == name).unwrap_or(0);
    let mut lines: Vec<(Segment, voices::Line)> = lines
        .into_iter()
        .map(|(segment, _)| {
            let pcm = &sources[stream_index(&segment.stream)].pcm;
            let at = |s: f64| ((s * RATE as f64) as usize).min(pcm.len());
            let span = &pcm[at(segment.start_s)..at(segment.end_s)];
            let power = span.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / span.len().max(1) as f64;
            let level_db = (10.0 * (power.max(1.0) / (32768.0f64 * 32768.0)).log10()) as f32;
            let line = voices::Line { speaker: segment.speaker.clone(), voice: stv_core::embed::voice(span, RATE), level_db };
            (segment, line)
        })
        .collect();
    lines.sort_by(|a, b| a.0.start_s.total_cmp(&b.0.start_s));
    let (segments, lines): (Vec<Segment>, Vec<voices::Line>) = lines.into_iter().unzip();
    let speakers = voices::assign(&lines, names.len());
    let mut line_voices = BTreeMap::new();
    let mut segments: Vec<Segment> = segments
        .into_iter()
        .zip(lines)
        .zip(speakers)
        .enumerate()
        .map(|(id, ((mut segment, line), speaker))| {
            segment.id = id;
            segment.speaker = speaker;
            if let Some(v) = line.voice {
                line_voices.insert(id, v);
            }
            segment
        })
        .collect();
    carry_over_edits(&library::load_segments(dir), &mut segments);
    voices::mark_overlaps(&mut segments);

    progress("Saving", 0.97);
    let duration = sources.iter().map(|s| s.pcm.len()).max().unwrap_or(0) as f64 / RATE as f64;
    let mut meta = library::new_meta(dir, duration);
    name_from_prints(&mut meta, &segments, &line_voices);
    library::write_json(&dir.join(voices::LINE_VOICES), &line_voices)?;
    library::save(dir, &meta, &segments)?;

    progress("Listening for key phrases", 0.98);
    let streams: Vec<(String, &[i16])> = names.iter().cloned().zip(sources.iter().map(|s| s.pcm.as_slice())).collect();
    library::write_json(&dir.join(keyphrases::SESSION_FILE), &keyphrases::scan_session(&streams))?;
    let outcome = transcription.map_or_else(|e| format!("transcription failed: {e}"), str::to_string);
    let shared = attribution.segments.len() - utterances.len() + joined;
    Ok(format!("{} stream(s), {} speech segment(s), {shared} joined from more than one microphone, {outcome}", names.len(), utterances.len() - joined))
}

/// The last check against the same words twice: lines on different microphones at the same time that say
/// mostly the same thing are one line, kept from the microphone that heard it more clearly and made more
/// certain by the other. `lines` pairs each line with how clearly it was heard. Returns how many were joined.
fn join_same_words(lines: &mut Vec<(Segment, f32)>) -> usize {
    /// Fewer words than this are too common ("yes", "okay") to say two lines are the same.
    const MIN_WORDS: usize = 3;
    let mut joined = 0;
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() {
            let (a, b) = (&lines[i].0, &lines[j].0);
            let overlap = a.end_s.min(b.end_s) - a.start_s.max(b.start_s);
            let shorter = a.text.split_whitespace().count().min(b.text.split_whitespace().count());
            if a.stream == b.stream || overlap <= 0.0 || shorter < MIN_WORDS {
                j += 1;
                continue;
            }
            let shared = merge::shared_words(&a.text, &b.text);
            if shared < merge::SAME_WORDS {
                j += 1;
                continue;
            }
            // The clearer line is moved to i, and the other is taken out; lines are sorted by time later.
            if lines[j].1 > lines[i].1 {
                lines.swap(i, j);
            }
            let gone = lines.remove(j).0;
            let kept = &mut lines[i].0;
            kept.start_s = kept.start_s.min(gone.start_s);
            kept.end_s = kept.end_s.max(gone.end_s);
            kept.confidence = merge::boosted(kept.confidence.max(gone.confidence), shared);
            joined += 1;
            // Line i is now whichever was kept; compare it with the rest again from the next one.
            j = i + 1;
        }
        i += 1;
    }
    joined
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
    fn the_same_words_on_two_microphones_are_one_line() {
        let mut lines = vec![
            (line("input1", 1.0, "Let's ship the new version on Friday", false), 18.0),
            (line("input2", 1.2, "let's ship the new version on Friday", false), 12.0),
            (line("input2", 3.0, "Sounds good to me", false), 20.0),
            (line("input1", 3.1, "Okay", false), 5.0),
        ];
        assert_eq!(join_same_words(&mut lines), 1);
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].0.stream.as_str(), lines[0].0.end_s), ("input1", 2.2));
        assert!(lines[0].0.confidence > 0.9);
        // Different words at the same time are two people talking at once, and both are kept.
        let mut talking = vec![(line("input1", 1.0, "I think we should wait", false), 18.0), (line("input2", 1.0, "can we ship it today", false), 18.0)];
        assert_eq!(join_same_words(&mut talking), 0);
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

    fn read_wav(path: &str) -> Vec<i16> {
        let data = fs::read(path).unwrap();
        let at = data.windows(4).position(|w| w == b"data").unwrap() + 8;
        data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }

    /// Writes PCM as a journal the way live capture does, with arrival times at real-time pace.
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
