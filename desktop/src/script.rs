//! One script from every microphone in the room.
//!
//! One person heard on two microphones gives two sets of lines: the clear one from the microphone they
//! spoke into, and fragments from the other, cut wherever the other microphone lost them. Those fragments
//! say again, worse, what the clear line already says. Here they are found and kept out of the script
//! (they stay in the session, greyed out in their track, and can be brought back). Then the clips one
//! person said on one microphone with only short pauses between them are stitched into one line, with
//! "..." where a sound was heard but no words came out of it.

use crate::library::Segment;
use stv_core::merge;

/// A line of speech as transcribed, with every microphone that heard it.
#[derive(Debug, Clone)]
pub struct Clip {
    pub segment: Segment,
    /// Each microphone that heard it and how far above its background (dB), its own microphone first.
    pub heard: Vec<(String, f32)>,
}

impl Clip {
    fn clarity(&self) -> f32 {
        self.heard.first().map_or(0.0, |h| h.1)
    }

    fn heard_on(&self, stream: &str) -> Option<f32> {
        self.heard.iter().find(|(s, _)| s == stream).map(|h| h.1)
    }

    fn duration(&self) -> f64 {
        (self.segment.end_s - self.segment.start_s).max(0.01)
    }
}

/// Sound heard on a microphone that gave no words: mumbling, a word swallowed, or noise.
#[derive(Debug, Clone)]
pub struct Unheard {
    pub stream: String,
    pub start_s: f64,
    pub end_s: f64,
    pub clarity_db: f32,
}

/// Lines on two microphones at once that share this much of the shorter one's words are the same thing.
const SAME_WORDS: f32 = merge::SAME_WORDS;
/// Fewer words than this are too common ("yes", "okay") to say two lines are the same by words alone.
const MIN_WORDS: usize = 3;
/// A clip this much inside a longer line's time is a candidate for being part of it.
const CONTAINED: f64 = 0.6;
/// A clip inside a longer line that shares this much of its words says the same thing.
const ECHO_WORDS: f32 = 0.5;
/// A clip this short inside a longer line that its microphone heard too is a piece of it, whatever whisper
/// made of it ("too different" for "two different").
const FRAGMENT_WORDS: usize = 4;
/// A fragment heard this much more clearly than the longer line was on the same microphone is someone
/// speaking into it, not the longer line's speaker heard from across the table.
const BLEED_DB: f32 = 8.0;

/// Clips of one person on one microphone this close together are one line.
pub const STITCH_GAP_S: f64 = 1.5;
/// A stitched line stops growing at this length, so the script still has paragraphs.
pub const STITCH_MAX_S: f64 = 45.0;
/// Sound this far above a microphone's background between two clips is something said, not room noise.
const UNHEARD_DB: f32 = 20.0;
/// Unheard sound longer than this was more than a word.
const UNINTELLIGIBLE_S: f64 = 1.0;

/// The word without punctuation or case, with spellings that sound the same made the same.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let k = merge::key(word);
        match k.as_str() {
            "" => {}
            "gonna" => out.extend(["going".to_string(), "to".to_string()]),
            "wanna" => out.extend(["want".to_string(), "to".to_string()]),
            "gotta" => out.extend(["got".to_string(), "to".to_string()]),
            "too" | "two" => out.push("to".into()),
            "four" => out.push("for".into()),
            "there's" | "theirs" => out.push("there's".into()),
            "okay" | "ok" => out.push("ok".into()),
            _ => out.push(k),
        }
    }
    out
}

/// Words that are the same once whisper's guesses are allowed for: "speak" and "speaking", "soot" and "soots".
fn alike(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.chars().count() >= 4 && long.starts_with(short)
}

/// How many of `a`'s words `b` says too, in the same order.
fn common(a: &[String], b: &[String]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if alike(x, y) { diagonal + 1 } else { row[j + 1].max(row[j]) };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The share of `a`'s words that `b` says too, in the same order.
fn found_in(a: &str, b: &str) -> f32 {
    let (a, b) = (tokens(a), tokens(b));
    if a.is_empty() {
        return 0.0;
    }
    common(&a, &b) as f32 / a.len() as f32
}

fn overlap(a: &Segment, b: &Segment) -> f64 {
    a.end_s.min(b.end_s) - a.start_s.max(b.start_s)
}

/// Finds the clips that double up another. Returns, for each clip, the one it doubles up, which is never
/// itself a double. Clips on two microphones at once that say the same words are joined first: the
/// clearer keeps both times and becomes more certain.
pub fn find_echoes(clips: &mut [Clip]) -> Vec<Option<usize>> {
    let n = clips.len();
    let mut echo: Vec<Option<usize>> = vec![None; n];

    // The same words on two microphones at the same time.
    for i in 0..n {
        for j in i + 1..n {
            if echo[i].is_some() || echo[j].is_some() {
                continue;
            }
            let (a, b) = (&clips[i].segment, &clips[j].segment);
            if a.stream == b.stream || overlap(a, b) <= 0.0 {
                continue;
            }
            let (wa, wb) = (tokens(&a.text).len(), tokens(&b.text).len());
            if wa.min(wb) < MIN_WORDS {
                continue;
            }
            let shared = if wa <= wb { found_in(&a.text, &b.text) } else { found_in(&b.text, &a.text) };
            if shared < SAME_WORDS {
                continue;
            }
            let (keep, gone) = if clips[j].clarity() > clips[i].clarity() { (j, i) } else { (i, j) };
            let (start, end, confidence) = (clips[gone].segment.start_s, clips[gone].segment.end_s, clips[gone].segment.confidence);
            let extra: Vec<(String, f32)> = clips[gone].heard.clone();
            let kept = &mut clips[keep];
            kept.segment.start_s = kept.segment.start_s.min(start);
            kept.segment.end_s = kept.segment.end_s.max(end);
            kept.segment.confidence = merge::boosted(kept.segment.confidence.max(confidence), shared);
            for (stream, clarity) in extra {
                if kept.heard_on(&stream).is_none() {
                    kept.heard.push((stream, clarity));
                }
            }
            echo[gone] = Some(keep);
        }
    }

    // Pieces of a longer line, cut out of it by the other microphone. Longest lines are settled first, so
    // a piece is always measured against a line that stays.
    let mut order: Vec<usize> = (0..n).filter(|&i| echo[i].is_none()).collect();
    order.sort_by(|&a, &b| clips[b].duration().total_cmp(&clips[a].duration()).then(a.cmp(&b)));
    let mut kept: Vec<usize> = Vec::new();
    for &i in &order {
        let a = &clips[i];
        let words = tokens(&a.segment.text).len();
        let host = kept.iter().copied().find(|&j| {
            let b = &clips[j];
            if overlap(&a.segment, &b.segment) / a.duration() < CONTAINED {
                return false;
            }
            let share = found_in(&a.segment.text, &b.segment.text);
            let bleed = a.segment.stream == b.segment.stream
                || b.heard_on(&a.segment.stream).is_some_and(|c| a.clarity() <= c + BLEED_DB);
            (words >= MIN_WORDS && share >= ECHO_WORDS)
                || (words >= 2 && share >= 0.99)
                || (bleed && (share >= ECHO_WORDS || words <= FRAGMENT_WORDS))
        });
        match host {
            Some(j) => echo[i] = Some(j),
            None => kept.push(i),
        }
    }

    // A double of a double doubles the line that stays.
    for i in 0..n {
        let mut target = echo[i];
        while let Some(t) = target.and_then(|t| echo[t]) {
            target = Some(t);
        }
        echo[i] = target;
    }
    echo
}

/// Why `next` can't continue the line made of `parts` so far, or None when it can.
fn stitch_blocker(line: &Segment, last_start: f64, next: &Segment, marks: &[f64]) -> Option<&'static str> {
    if line.speaker != next.speaker || line.stream != next.stream {
        Some("another voice or microphone")
    } else if line.edited || next.edited {
        Some("edited by hand")
    } else if next.start_s - line.end_s > STITCH_GAP_S {
        Some("too long a pause")
    } else if next.end_s - line.start_s > STITCH_MAX_S {
        Some("long enough")
    } else if marks.iter().any(|&m| m > last_start && m <= next.start_s) {
        Some("a bookmark between them")
    } else {
        None
    }
}

/// What sound came between two clips on a microphone without words: None, "...", or "...(unintelligible)...".
fn gap_note(stream: &str, from: f64, to: f64, unheard: &[Unheard], script: &[&Segment]) -> Option<&'static str> {
    let mut missed = 0.0;
    for u in unheard.iter().filter(|u| u.stream == stream && u.clarity_db >= UNHEARD_DB) {
        let (start, end) = (u.start_s.max(from), u.end_s.min(to));
        if end <= start {
            continue;
        }
        // Sound that another line in the script says was that line, heard from across the room.
        let said = script.iter().any(|s| s.stream != stream && s.end_s.min(u.end_s) - s.start_s.max(u.start_s) > 0.5 * (u.end_s - u.start_s));
        if !said {
            missed += end - start;
        }
    }
    if missed <= 0.0 {
        None
    } else if missed >= UNINTELLIGIBLE_S {
        Some("...(unintelligible)...")
    } else {
        Some("...")
    }
}

/// The text without whisper's "..." at the end, where the clip was cut off; a full stop stays.
fn trailing_ellipsis(text: &str) -> &str {
    let t = text.trim_end();
    let cut = t.strip_suffix("...").or_else(|| t.strip_suffix('…'));
    cut.map_or(t, str::trim_end)
}

/// `next` without the words at its start that `prev` already ends with, as when two clips overlap in time.
fn without_repeat(prev: &str, next: &str) -> String {
    let prev_tokens = tokens(prev);
    let words: Vec<&str> = next.split_whitespace().collect();
    // Each of next's tokens, with the word it came from.
    let next_tokens: Vec<(String, usize)> = words.iter().enumerate().flat_map(|(i, w)| tokens(w).into_iter().map(move |t| (t, i))).collect();
    let most = prev_tokens.len().min(next_tokens.len()).min(8);
    for k in (2..=most).rev() {
        let tail = &prev_tokens[prev_tokens.len() - k..];
        let whole_word = k == next_tokens.len() || next_tokens[k].1 != next_tokens[k - 1].1;
        if whole_word && tail.iter().zip(&next_tokens[..k]).all(|(a, (b, _))| alike(a, b)) {
            return words[next_tokens[k - 1].1 + 1..].join(" ");
        }
    }
    next.to_string()
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
}

/// Two clips' words as one line.
fn join_text(prev: &str, next: &str, overlapping: bool, note: Option<&str>) -> String {
    let next = if overlapping { without_repeat(prev, next) } else { next.trim().to_string() };
    let ends_sentence = |t: &str| t.trim_end().ends_with(['.', '?', '!']) && !t.trim_end().ends_with("...") && !t.trim_end().ends_with('…');
    let text = match note {
        Some(note) => format!("{} {note} {}", trailing_ellipsis(prev), next.trim()),
        None if next.trim().is_empty() => prev.trim().to_string(),
        // Whisper's "..." where a clip was cut off is mended by the clip that carries on.
        None if !ends_sentence(prev) => format!("{} {}", trailing_ellipsis(prev), next.trim()),
        None => format!("{} {}", prev.trim(), capitalised(next.trim())),
    };
    text.trim().to_string()
}

/// Stitches each person's clips on one microphone into lines. `segments` are in time order, with ids equal
/// to their positions and `echo_of` pointing at positions. `marks` are bookmark times: a line never runs
/// past one, so the bookmark sits between the lines. Returns each new line with the positions it was made
/// from; doubled-up clips pass through on their own.
pub fn stitch(segments: Vec<Segment>, unheard: &[Unheard], marks: &[f64]) -> Vec<(Segment, Vec<usize>)> {
    let script: Vec<&Segment> = segments.iter().filter(|s| s.in_script()).collect();
    let mut out: Vec<(Segment, Vec<usize>)> = Vec::new();
    // The line being built: its position in `out` and the start of its last clip.
    let mut open: Option<(usize, f64)> = None;
    for (i, next) in segments.iter().enumerate() {
        if !next.in_script() {
            out.push((next.clone(), vec![i]));
            continue;
        }
        if let Some((at, last_start)) = open {
            let line = &out[at].0;
            if stitch_blocker(line, last_start, next, marks).is_none() {
                let note = gap_note(&next.stream, line.end_s, next.start_s, unheard, &script);
                let overlapping = next.start_s < line.end_s;
                let (line, members) = &mut out[at];
                let (len_a, len_b) = (line.end_s - line.start_s, next.end_s - next.start_s);
                let mut parts = line.spans();
                parts.extend(next.spans());
                line.text = join_text(&line.text, &next.text, overlapping, note);
                line.confidence = ((line.confidence as f64 * len_a + next.confidence as f64 * len_b) / (len_a + len_b).max(0.01)) as f32;
                line.end_s = line.end_s.max(next.end_s);
                line.parts = parts;
                for stream in &next.also_on {
                    if !line.also_on.contains(stream) {
                        line.also_on.push(stream.clone());
                    }
                }
                members.push(i);
                open = Some((at, next.start_s));
                continue;
            }
        }
        out.push((next.clone(), vec![i]));
        open = Some((out.len() - 1, next.start_s));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(stream: &str, start_s: f64, end_s: f64, text: &str, heard: &[(&str, f32)]) -> Clip {
        Clip {
            segment: Segment { stream: stream.into(), speaker: stream.into(), start_s, end_s, text: text.into(), confidence: 0.5, ..Default::default() },
            heard: heard.iter().map(|(s, c)| (s.to_string(), *c)).collect(),
        }
    }

    /// The lines of examples/session-1794235955: one person speaking into two microphones, the second
    /// of which cut the speech into pieces that each said part of it again.
    fn two_microphones_one_voice() -> Vec<Clip> {
        vec![
            clip("input1", 0.0, 1.4, "So I'm testing the", &[("input1", 40.4)]),
            clip("input2", 0.8, 3.6, "So I'm testing this recording with...", &[("input2", 53.3)]),
            clip("input1", 1.8, 2.05, "this recording", &[("input1", 41.9)]),
            clip("input1", 3.8, 16.65, "two different microphones and I'm just wondering whether this is going to work okay. I'm speaking into the Samsung Samsung microphone.", &[("input1", 52.9), ("input2", 46.8)]),
            clip("input2", 4.0, 4.9, "too different", &[("input2", 53.4)]),
            clip("input2", 9.7, 10.45, "work okay", &[("input2", 47.2)]),
            clip("input2", 10.85, 11.35, "I speak here.", &[("input2", 43.6)]),
            clip("input2", 11.75, 11.85, "in", &[("input2", 39.8)]),
            clip("input1", 15.8, 16.55, "microphone", &[("input1", 52.0)]),
            clip("input1", 17.2, 18.15, "It's up close.", &[("input1", 49.0)]),
            clip("input2", 17.3, 18.1, "It's up close", &[("input2", 46.0)]),
            clip("input1", 18.5, 22.05, "and a bookmark that", &[("input1", 47.0)]),
            clip("input2", 20.05, 22.3, "a bookmark", &[("input2", 42.0)]),
            clip("input1", 23.1, 30.65, "I actually have soot in the room. I didn't realize that. Soots in here with me. So, pussycat.", &[("input1", 49.0), ("input2", 45.0)]),
            clip("input2", 27.95, 30.75, "I'm sitting here with me, so, pause again.", &[("input2", 46.0)]),
            clip("input2", 31.05, 33.2, "And.", &[("input2", 51.0), ("input1", 46.0)]),
            clip("input1", 33.6, 35.0, "I would like to be able to edit", &[("input1", 44.0)]),
            clip("input2", 34.0, 42.85, "I would like to be able to edit the bookmarks. I think you should be able to click on a bookmark and edit it as you're working.", &[("input2", 47.0), ("input1", 46.0)]),
            clip("input2", 43.0, 44.5, "Um.", &[("input2", 49.0), ("input1", 48.0)]),
            clip("input1", 44.35, 50.7, "Beyond that, I think everything's going well, the question is whether or not it's going to handle...", &[("input1", 50.0), ("input2", 48.0)]),
            clip("input1", 49.5, 53.6, "it's gonna handle this so there's one", &[("input1", 48.0), ("input2", 47.0)]),
            clip("input1", 53.85, 56.55, "Clear recording or it's gonna be a mess.", &[("input1", 50.0), ("input2", 50.0)]),
        ]
    }

    /// Runs both steps as processing does, with each line's own stream as its speaker.
    fn script(mut clips: Vec<Clip>, unheard: &[Unheard], marks: &[f64]) -> Vec<Segment> {
        let echoes = find_echoes(&mut clips);
        let segments: Vec<Segment> = clips
            .into_iter()
            .enumerate()
            .map(|(i, c)| Segment { id: i, echo_of: echoes[i], ..c.segment })
            .collect();
        stitch(segments, unheard, marks).into_iter().map(|(s, _)| s).filter(|s| s.in_script()).collect()
    }

    #[test]
    fn one_voice_on_two_microphones_is_said_once() {
        let mut clips = two_microphones_one_voice();
        let echoes = find_echoes(&mut clips);
        let texts = |keep: bool| -> Vec<&str> {
            clips.iter().zip(&echoes).filter(|(_, e)| e.is_none() == keep).map(|(c, _)| c.segment.text.as_str()).collect()
        };
        assert_eq!(
            texts(true),
            [
                "So I'm testing this recording with...",
                "two different microphones and I'm just wondering whether this is going to work okay. I'm speaking into the Samsung Samsung microphone.",
                "It's up close.",
                "and a bookmark that",
                "I actually have soot in the room. I didn't realize that. Soots in here with me. So, pussycat.",
                "And.",
                "I would like to be able to edit the bookmarks. I think you should be able to click on a bookmark and edit it as you're working.",
                "Um.",
                "Beyond that, I think everything's going well, the question is whether or not it's going to handle...",
                "it's gonna handle this so there's one",
                "Clear recording or it's gonna be a mess.",
            ]
        );
        // Every double points at a line that stays.
        assert!(echoes.iter().flatten().all(|&t| echoes[t].is_none()));
    }

    #[test]
    fn the_example_becomes_one_script() {
        let lines = script(two_microphones_one_voice(), &[], &[21.6, 30.6]);
        let shown: Vec<(&str, &str)> = lines.iter().map(|s| (s.stream.as_str(), s.text.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("input2", "So I'm testing this recording with..."),
                ("input1", "two different microphones and I'm just wondering whether this is going to work okay. I'm speaking into the Samsung Samsung microphone. It's up close. And a bookmark that"),
                ("input1", "I actually have soot in the room. I didn't realize that. Soots in here with me. So, pussycat."),
                ("input2", "And. I would like to be able to edit the bookmarks. I think you should be able to click on a bookmark and edit it as you're working. Um."),
                ("input1", "Beyond that, I think everything's going well, the question is whether or not it's going to handle this so there's one Clear recording or it's gonna be a mess."),
            ]
        );
        assert_eq!(lines[1].parts.len(), 3);
    }

    #[test]
    fn people_talking_at_once_on_their_own_microphones_are_both_kept() {
        let mut clips = vec![
            clip("input1", 0.0, 10.0, "I think we should wait until the numbers come in next week", &[("input1", 50.0)]),
            // Said into the other microphone, much louder there than the first voice could be.
            clip("input2", 4.0, 5.0, "No way", &[("input2", 52.0)]),
            clip("input2", 6.0, 8.0, "can we ship it today", &[("input2", 50.0)]),
        ];
        assert_eq!(find_echoes(&mut clips), [None, None, None]);
    }

    #[test]
    fn a_short_word_heard_louder_than_the_bleed_is_someone_else() {
        let mut clips = vec![
            clip("input1", 0.0, 10.0, "so the plan is to ship on Friday and then review it", &[("input1", 50.0), ("input2", 30.0)]),
            clip("input2", 4.0, 4.5, "yes", &[("input2", 48.0)]),
        ];
        assert_eq!(find_echoes(&mut clips), [None, None]);
    }

    #[test]
    fn the_same_words_on_two_microphones_are_one_line() {
        let mut clips = vec![
            clip("input1", 1.0, 2.0, "Let's ship the new version on Friday", &[("input1", 18.0)]),
            clip("input2", 1.2, 2.2, "let's ship the new version on Friday", &[("input2", 12.0)]),
            clip("input2", 3.0, 4.0, "Sounds good to me", &[("input2", 20.0)]),
            clip("input1", 3.1, 3.4, "Okay", &[("input1", 5.0)]),
        ];
        let echoes = find_echoes(&mut clips);
        assert_eq!(echoes[..3], [None, Some(0), None]);
        assert_eq!(clips[0].segment.end_s, 2.2);
        assert!(clips[0].segment.confidence > 0.8);
        assert!(clips[0].heard_on("input2").is_some());
    }

    #[test]
    fn sound_without_words_between_clips_is_marked() {
        let line = |start_s: f64, end_s: f64, text: &str| clip("input1", start_s, end_s, text, &[("input1", 40.0)]);
        let mumble = |start_s: f64, end_s: f64| Unheard { stream: "input1".into(), start_s, end_s, clarity_db: 35.0 };
        let lines = script(vec![line(0.0, 2.0, "We could move it"), line(2.4, 3.0, "Thursday."), line(4.4, 5.0, "or never")], &[mumble(2.05, 2.3), mumble(3.0, 4.4)], &[]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "We could move it ... Thursday. ...(unintelligible)... or never");
        // Room noise is not something said.
        let quiet = Unheard { stream: "input1".into(), start_s: 2.05, end_s: 2.3, clarity_db: 10.0 };
        assert_eq!(script(vec![line(0.0, 2.0, "We could move it"), line(2.4, 3.0, "to Thursday.")], &[quiet], &[])[0].text, "We could move it to Thursday.");
    }

    #[test]
    fn long_pauses_other_voices_and_bookmarks_end_a_line() {
        let a = |start_s: f64, text: &str| clip("input1", start_s, start_s + 1.0, text, &[("input1", 40.0)]);
        assert_eq!(script(vec![a(0.0, "One."), a(4.0, "Two.")], &[], &[]).len(), 2);
        assert_eq!(script(vec![a(0.0, "One."), a(1.5, "Two.")], &[], &[1.2]).len(), 2);
        assert_eq!(script(vec![a(0.0, "One."), a(1.5, "Two.")], &[], &[]).len(), 1);
        let mut other = a(1.5, "Two.");
        other.segment.speaker = "guest1".into();
        assert_eq!(script(vec![a(0.0, "One."), other], &[], &[]).len(), 2);
    }

    #[test]
    fn overlapping_clips_say_the_shared_words_once() {
        assert_eq!(without_repeat("whether it's going to handle...", "it's gonna handle this so"), "this so");
        assert_eq!(without_repeat("we will see", "nothing in common"), "nothing in common");
    }
}
