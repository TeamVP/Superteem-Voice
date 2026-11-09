//! One voice heard on several microphones becomes one line, not one per microphone.
//!
//! Two people sitting side by side, each with a microphone, are both picked up by both. When the levels are
//! close, attribution can give the same words to both microphones. Here those segments are found and joined:
//!
//! 1. Before transcribing: segments on different microphones that overlap in time and rise and fall
//!    together (the loudness of the same syllables, heard twice) are one stretch of speech. It is
//!    transcribed from the microphone that heard it most clearly, and from up to two others.
//! 2. The transcripts are lined up word by word. Words the microphones agree on are more certain; where they
//!    differ, the clearer microphone's word wins unless the others outvote it.
//! 3. After transcribing, any two lines on different microphones at the same time with mostly the same words
//!    are joined as well, in case the loudness didn't show it.
//!
//! Two people talking at once, each loudest on their own microphone, rise and fall separately and say
//! different words, so both lines are kept.

use crate::attribute::Segment;

/// Loudness curves this alike are the same sound heard twice.
pub const SAME_SOUND: f32 = 0.6;
/// Windows of overlap needed to judge loudness curves (50 ms each).
const MIN_WINDOWS: usize = 6;
/// Lines whose words match this well (of the shorter) are the same thing said once.
pub const SAME_WORDS: f32 = 0.6;
/// More microphones than this add little and cost a transcription each.
pub const MAX_READINGS: usize = 3;

/// A stretch of speech heard on one or more microphones, to be transcribed once.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub speaker_source: Option<usize>,
    pub start_window: usize,
    pub end_window: usize,
    pub confidence: f32,
    /// Microphones that heard it, clearest first, with how far above their background they heard it (dB).
    pub sources: Vec<(usize, f32)>,
}

impl Utterance {
    pub fn best(&self) -> usize {
        self.sources[0].0
    }
}

fn floor(levels: &[f32]) -> f32 {
    if levels.is_empty() {
        return 0.0;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[(sorted.len() - 1) / 10]
}

/// How far above its background a microphone heard a stretch, from its loudest half of windows.
fn clarity(levels: &[f32], floor: f32, start: usize, end: usize) -> f32 {
    let end = end.min(levels.len());
    if start >= end {
        return f32::MIN;
    }
    let mut span: Vec<f32> = levels[start..end].to_vec();
    span.sort_by(|a, b| b.total_cmp(a));
    let loud = &span[..span.len().div_ceil(2)];
    loud.iter().sum::<f32>() / loud.len() as f32 - floor
}

/// How alike two loudness curves are over a stretch: 1 for the same sound, near 0 for unrelated sounds.
/// Each is clipped at its background, so quiet hiss doesn't count as agreement.
pub fn same_sound(a: &[f32], b: &[f32], floor_a: f32, floor_b: f32, start: usize, end: usize) -> Option<f32> {
    let end = end.min(a.len()).min(b.len());
    if end < start + MIN_WINDOWS {
        return None;
    }
    let x: Vec<f64> = a[start..end].iter().map(|&v| v.max(floor_a) as f64).collect();
    let y: Vec<f64> = b[start..end].iter().map(|&v| v.max(floor_b) as f64).collect();
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (p, q) in x.iter().zip(&y) {
        sxy += (p - mx) * (q - my);
        sxx += (p - mx).powi(2);
        syy += (q - my).powi(2);
    }
    if sxx < 1e-6 || syy < 1e-6 {
        return None;
    }
    Some((sxy / (sxx * syy).sqrt()) as f32)
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut i = i;
    while parent[i] != r {
        let next = parent[i];
        parent[i] = r;
        i = next;
    }
    r
}

/// Joins segments that are one voice heard on several microphones. `levels` are each microphone's loudness
/// per window, as given to attribution.
pub fn utterances(segments: &[Segment], levels: &[Vec<f32>]) -> Vec<Utterance> {
    let floors: Vec<f32> = levels.iter().map(|l| floor(l)).collect();
    let mut parent: Vec<usize> = (0..segments.len()).collect();
    for i in 0..segments.len() {
        for j in i + 1..segments.len() {
            let (a, b) = (&segments[i], &segments[j]);
            if a.audio_source == b.audio_source {
                continue;
            }
            let (start, end) = (a.start_window.max(b.start_window), a.end_window.min(b.end_window));
            if start >= end {
                continue;
            }
            // Judged where they overlap, so someone cutting in over a long turn isn't taken for an echo of it.
            // A short overlap is widened a little to have enough to go on.
            let short = (2 * MIN_WINDOWS).saturating_sub(end - start).div_ceil(2);
            let (from, to) = (start.saturating_sub(short), end + short);
            let (sa, sb) = (a.audio_source, b.audio_source);
            if same_sound(&levels[sa], &levels[sb], floors[sa], floors[sb], from, to).is_some_and(|r| r >= SAME_SOUND) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..segments.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut out: Vec<Utterance> = groups
        .into_values()
        .map(|members| {
            let start = members.iter().map(|&i| segments[i].start_window).min().unwrap();
            let end = members.iter().map(|&i| segments[i].end_window).max().unwrap();
            let mut sources: Vec<(usize, f32)> = Vec::new();
            for &i in &members {
                let s = segments[i].audio_source;
                if !sources.iter().any(|(x, _)| *x == s) {
                    sources.push((s, clarity(&levels[s], floors[s], start, end)));
                }
            }
            sources.sort_by(|a, b| b.1.total_cmp(&a.1));
            let best = sources[0].0;
            // The person is whoever owns the clearest microphone's part; a guest if that part was a guest.
            let lead = members.iter().copied().filter(|&i| segments[i].audio_source == best).max_by_key(|&i| segments[i].end_window - segments[i].start_window).unwrap();
            let confidence = members.iter().map(|&i| segments[i].confidence).fold(0.0, f32::max);
            Utterance { speaker_source: segments[lead].speaker_source, start_window: start, end_window: end, confidence, sources }
        })
        .collect();
    out.sort_by_key(|u| (u.start_window, u.best()));
    out
}

/// The word without punctuation or case, for comparing.
pub fn key(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase()
}

fn keys(text: &str) -> Vec<String> {
    text.split_whitespace().map(key).filter(|k| !k.is_empty()).collect()
}

/// Lines up two word lists, allowing words to differ, be missing or be extra. Each pair is a position in
/// `a` and one in `b`, either of which may be absent.
pub fn align(a: &[String], b: &[String]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (a.len(), b.len());
    let mut cost = vec![vec![0u32; m + 1]; n + 1];
    for (i, row) in cost.iter_mut().enumerate() {
        row[0] = i as u32;
    }
    for j in 0..=m {
        cost[0][j] = j as u32;
    }
    for i in 1..=n {
        for j in 1..=m {
            let change = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cost[i][j] = (cost[i - 1][j - 1] + change).min(cost[i - 1][j] + 1).min(cost[i][j - 1] + 1);
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && cost[i][j] == cost[i - 1][j - 1] + u32::from(a[i - 1] != b[j - 1]) {
            out.push((Some(i - 1), Some(j - 1)));
            i -= 1;
            j -= 1;
        } else if i > 0 && cost[i][j] == cost[i - 1][j] + 1 {
            out.push((Some(i - 1), None));
            i -= 1;
        } else {
            out.push((None, Some(j - 1)));
            j -= 1;
        }
    }
    out.reverse();
    out
}

/// The share of the shorter text's words that the other text says too, in the same order.
pub fn shared_words(a: &str, b: &str) -> f32 {
    let (a, b) = (keys(a), keys(b));
    let shorter = a.len().min(b.len());
    if shorter == 0 {
        return 0.0;
    }
    let same = align(&a, &b).iter().filter(|(i, j)| matches!((i, j), (Some(i), Some(j)) if a[*i] == b[*j])).count();
    same as f32 / shorter as f32
}

/// One transcript made from several microphones' readings of the same speech.
#[derive(Debug, Clone, PartialEq)]
pub struct Combined {
    pub text: String,
    /// The share of words another microphone heard the same, or None with only one reading.
    pub agreement: Option<f32>,
}

/// Combines readings of one stretch of speech, each with a weight (how clearly its microphone heard it),
/// clearest first. Every word position is a vote: a word the readings share is kept and counts as agreed;
/// where they differ, the word with the most weight behind it wins, so with two readings the clearer one is
/// kept and with three a word two of them heard outvotes the third.
pub fn combine(readings: &[(String, f32)]) -> Combined {
    let readings: Vec<&(String, f32)> = readings.iter().filter(|(t, _)| !keys(t).is_empty()).collect();
    let Some((base, _)) = readings.first().copied() else {
        return Combined { text: String::new(), agreement: None };
    };
    if readings.len() == 1 {
        return Combined { text: base.clone(), agreement: None };
    }
    // Weights are made positive and at least a little apart from zero, so a quiet microphone still votes.
    let weight = |w: f32| w.max(0.0) + 1.0;
    let base_words: Vec<&str> = base.split_whitespace().filter(|w| !key(w).is_empty()).collect();
    let base_keys: Vec<String> = base_words.iter().map(|w| key(w)).collect();
    // For each base word: the candidates (surface word, weight). Between words: insertions offered.
    let mut slots: Vec<Vec<(String, f32)>> = base_words.iter().map(|w| vec![(w.to_string(), weight(readings[0].1))]).collect();
    let mut gaps: Vec<Vec<(Vec<String>, f32)>> = vec![Vec::new(); base_words.len() + 1];
    let mut agreed = vec![false; base_words.len()];
    let total: f32 = readings.iter().map(|(_, w)| weight(*w)).sum();
    for &(text, w) in &readings[1..] {
        let words: Vec<&str> = text.split_whitespace().filter(|w| !key(w).is_empty()).collect();
        let other: Vec<String> = words.iter().map(|w| key(w)).collect();
        let mut pending: Vec<String> = Vec::new();
        let mut at = 0;
        for (i, j) in align(&base_keys, &other) {
            match (i, j) {
                (Some(i), Some(j)) => {
                    if !pending.is_empty() {
                        gaps[i].push((std::mem::take(&mut pending), weight(*w)));
                    }
                    if base_keys[i] == other[j] {
                        agreed[i] = true;
                    }
                    slots[i].push((words[j].to_string(), weight(*w)));
                    at = i + 1;
                }
                (Some(i), None) => {
                    if !pending.is_empty() {
                        gaps[i].push((std::mem::take(&mut pending), weight(*w)));
                    }
                    slots[i].push((String::new(), weight(*w)));
                    at = i + 1;
                }
                (None, Some(j)) => pending.push(words[j].to_string()),
                (None, None) => {}
            }
        }
        if !pending.is_empty() {
            gaps[at].push((pending, weight(*w)));
        }
    }
    let mut out: Vec<String> = Vec::new();
    for i in 0..=base_words.len() {
        // Words only other microphones heard are added when most of the weight is behind them.
        for (words, w) in &gaps[i] {
            let backing: f32 = gaps[i].iter().filter(|(o, _)| o.iter().map(|x| key(x)).eq(words.iter().map(|x| key(x)))).map(|(_, w)| w).sum();
            if backing * 2.0 > total && *w > 0.0 && !out.ends_with(words) {
                out.extend(words.iter().cloned());
            }
        }
        if i == base_words.len() {
            break;
        }
        // The word with the most weight behind it, by its spelling without punctuation; the clearest
        // microphone's own spelling is kept when it wins.
        let mut tally: Vec<(String, f32, String)> = Vec::new();
        for (word, w) in &slots[i] {
            let k = key(word);
            match tally.iter_mut().find(|(t, _, _)| *t == k) {
                Some(entry) => entry.1 += w,
                None => tally.push((k, *w, word.clone())),
            }
        }
        let (_, _, word) = tally.iter().fold(&tally[0], |best, t| if t.1 > best.1 { t } else { best });
        if !word.is_empty() {
            out.push(word.clone());
        }
    }
    let agreement = agreed.iter().filter(|&&a| a).count() as f32 / agreed.len().max(1) as f32;
    Combined { text: out.join(" "), agreement: Some(agreement) }
}

/// Confidence once other microphones have agreed on a share of the words: agreement makes a line more
/// certain, up to most of the way from where it was to 1.
pub fn boosted(confidence: f32, agreement: f32) -> f32 {
    (confidence + (1.0 - confidence) * 0.7 * agreement.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(source: usize, start: usize, end: usize) -> Segment {
        Segment { speaker_source: Some(source), audio_source: source, start_window: start, end_window: end, confidence: 0.5 }
    }

    /// A syllable rhythm, as loudness in dB per window: about four syllables a second.
    fn speech(w: usize, level: f32) -> f32 {
        let phase = (w % 5) as f32;
        level - 4.0 * phase
    }

    #[test]
    fn one_voice_on_two_microphones_is_one_utterance() {
        // Both hear the same syllables, the second 3 dB quieter.
        let a: Vec<f32> = (0..200).map(|w| if (20..120).contains(&w) { speech(w, -20.0) } else { -70.0 }).collect();
        let b: Vec<f32> = (0..200).map(|w| if (20..120).contains(&w) { speech(w, -23.0) } else { -72.0 }).collect();
        let segments = vec![seg(0, 20, 120), seg(1, 30, 80), seg(1, 90, 118)];
        let u = utterances(&segments, &[a, b]);
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!((u[0].start_window, u[0].end_window, u[0].best(), u[0].speaker_source), (20, 120, 0, Some(0)));
        assert_eq!(u[0].sources.len(), 2);
    }

    #[test]
    fn two_people_talking_at_once_stay_two_lines() {
        // Each is loudest on their own microphone, with their own rhythm; the other is faint bleed.
        let a: Vec<f32> = (0..200).map(|w| if (20..120).contains(&w) { speech(w, -20.0) } else { -70.0 }).collect();
        let b: Vec<f32> = (0..200).map(|w| if (20..120).contains(&w) { -20.0 - 4.0 * ((w * 7 / 3) % 4) as f32 } else { -70.0 }).collect();
        let u = utterances(&[seg(0, 20, 120), seg(1, 20, 120)], &[a, b]);
        assert_eq!(u.len(), 2, "{u:?}");
    }

    #[test]
    fn someone_cutting_in_over_a_long_turn_stays_their_own_line() {
        // A talks throughout, and B's microphone hears A as bleed, until B cuts in with their own rhythm.
        let a: Vec<f32> = (0..300).map(|w| if (20..280).contains(&w) { speech(w, -20.0) } else { -70.0 }).collect();
        let b: Vec<f32> = (0..300)
            .map(|w| match w {
                140..=170 => -18.0 - 4.0 * ((w * 7 / 3) % 4) as f32,
                20..=279 => speech(w, -34.0),
                _ => -72.0,
            })
            .collect();
        let u = utterances(&[seg(0, 20, 280), seg(1, 140, 171)], &[a, b]);
        assert_eq!(u.len(), 2, "{u:?}");
    }

    #[test]
    fn turns_on_different_microphones_stay_apart() {
        let a: Vec<f32> = (0..200).map(|w| if (20..60).contains(&w) { speech(w, -20.0) } else { -70.0 }).collect();
        let b: Vec<f32> = (0..200).map(|w| if (80..120).contains(&w) { speech(w, -20.0) } else { -70.0 }).collect();
        assert_eq!(utterances(&[seg(0, 20, 60), seg(1, 80, 120)], &[a, b]).len(), 2);
    }

    #[test]
    fn the_clearer_reading_wins_ties_and_agreement_is_counted() {
        let c = combine(&[("We meet on Tuesday at ten.".into(), 20.0), ("we meet on Thursday at ten".into(), 15.0)]);
        assert_eq!(c.text, "We meet on Tuesday at ten.");
        assert!((c.agreement.unwrap() - 5.0 / 6.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn three_readings_outvote_a_misheard_word() {
        let c = combine(&[
            ("We meet on Thursday at ten.".into(), 20.0),
            ("we meet on Tuesday at ten".into(), 18.0),
            ("We meet on Tuesday, at ten.".into(), 16.0),
        ]);
        assert_eq!(c.text, "We meet on Tuesday at ten.");
    }

    #[test]
    fn a_word_only_the_quieter_microphone_heard_needs_a_majority() {
        let two = combine(&[("Send the report".into(), 20.0), ("Send the final report".into(), 10.0)]);
        assert_eq!(two.text, "Send the report");
        let three = combine(&[("Send the report".into(), 20.0), ("Send the final report".into(), 18.0), ("send the final report".into(), 17.0)]);
        assert_eq!(three.text, "Send the final report");
    }

    #[test]
    fn an_empty_reading_is_ignored() {
        let c = combine(&[("".into(), 25.0), ("Hello there.".into(), 10.0)]);
        assert_eq!(c, Combined { text: "Hello there.".into(), agreement: None });
    }

    #[test]
    fn shared_words_measure_the_shorter_line() {
        assert!((shared_words("so we agreed to ship it on Friday", "we agreed to ship it") - 1.0).abs() < 1e-6);
        assert!(shared_words("we agreed to ship it", "the weather is lovely today") < 0.3);
        assert_eq!(shared_words("", "anything"), 0.0);
    }

    #[test]
    fn agreement_raises_confidence() {
        assert!(boosted(0.5, 1.0) > 0.8);
        assert_eq!(boosted(0.5, 0.0), 0.5);
    }
}
