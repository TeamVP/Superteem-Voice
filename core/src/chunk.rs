//! Cutting a recording that never stops into pieces to transcribe: each cut falls in a pause between
//! words, so the recording carries on while what came before is written down and nothing is lost.

use crate::attribute::window_db;

/// 20 ms frames at 16 kHz.
pub const FRAME: usize = 320;
const FRAMES_PER_S: usize = 50;
/// A pause this long ends a clip once it's long enough: the gap between sentences, not between words.
const PAUSE_FRAMES: usize = 40;
/// A clip shorter than this carries on through pauses between sentences, so a thought isn't cut into pieces
/// that read as separate sentences: whisper writes better with the whole of it.
const MIN_CLIP_FRAMES: usize = 30 * FRAMES_PER_S;
/// A pause this long ends a clip however short it is: the person has stopped, not paused to think.
const LONG_PAUSE_FRAMES: usize = 4 * FRAMES_PER_S;
/// Audio with no speech is let go in pieces this long, to notice a long silence.
const SILENT_PIECE_FRAMES: usize = 3 * FRAMES_PER_S;
/// Someone who doesn't pause is cut by this length, at the quietest moment of the last few seconds, so the
/// words keep coming. Whisper reads a longer clip 30 s at a time by itself.
const LONGEST_FRAMES: usize = 55 * FRAMES_PER_S;
const SEARCH_QUIET_FRAMES: usize = 10 * FRAMES_PER_S;
/// Speech is this far above the background noise, and at least this loud.
const ABOVE_NOISE_DB: f32 = 12.0;
const SPEECH_FLOOR_DB: f32 = -50.0;

/// Where to cut the audio waiting to be transcribed, and whether that piece has any speech in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// Samples from the start that make up the piece.
    pub at: usize,
    pub speech: bool,
}

/// Which 20 ms frames are speech, against the background noise heard so far (`noise_db`, the quiet level of
/// this recording; pass None to work it out from these frames).
pub fn speech_frames(pcm: &[i16], noise_db: Option<f32>) -> Vec<bool> {
    let db = window_db(pcm, FRAME);
    let noise = noise_db.unwrap_or_else(|| noise_level(&db));
    let threshold = (noise + ABOVE_NOISE_DB).max(SPEECH_FLOOR_DB);
    db.iter().map(|&d| d > threshold).collect()
}

/// The background level: the quietest twentieth of the frames, so a little quiet among a lot of talk is enough.
pub fn noise_level(db: &[f32]) -> f32 {
    if db.is_empty() {
        return -70.0;
    }
    let mut sorted = db.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[sorted.len() / 20].max(-80.0)
}

/// Where to cut `pcm`, the audio not yet transcribed, if anywhere yet: in the middle of the first pause of
/// 0.8 s once the clip has 30 s in it, or of 4 s at any length; after a few seconds of no speech at all; or,
/// for someone talking without a break, at the quietest moment before 55 s.
pub fn find_cut(pcm: &[i16], noise_db: Option<f32>) -> Option<Cut> {
    let speech = speech_frames(pcm, noise_db);
    let Some(first) = speech.iter().position(|&s| s) else {
        // Nothing said: let it go once there's a few seconds, keeping the last half second in case a word
        // is starting.
        return (speech.len() >= SILENT_PIECE_FRAMES).then(|| Cut { at: (speech.len() - FRAMES_PER_S / 2) * FRAME, speech: false });
    };
    let mut run = 0;
    for (i, &s) in speech.iter().enumerate().skip(first) {
        if s {
            run = 0;
            continue;
        }
        run += 1;
        let pause_start = i + 1 - run;
        if (run >= PAUSE_FRAMES && pause_start >= MIN_CLIP_FRAMES) || run >= LONG_PAUSE_FRAMES {
            return Some(Cut { at: (pause_start + run / 2) * FRAME, speech: true });
        }
    }
    if speech.len() >= LONGEST_FRAMES {
        let db = window_db(pcm, FRAME);
        let from = LONGEST_FRAMES - SEARCH_QUIET_FRAMES;
        let quietest = (from..LONGEST_FRAMES).min_by(|&a, &b| db[a].total_cmp(&db[b])).unwrap_or(LONGEST_FRAMES);
        return Some(Cut { at: quietest * FRAME, speech: true });
    }
    None
}

/// Less speech than this in a clip is a click, a breath or a knock, not a word.
const MIN_SPEECH_FRAMES: usize = 8;
/// Kept around the speech when a clip is trimmed: quiet sounds at the edges of words (an "s", a "f") fall
/// under the speech level, so a little either side keeps them.
const LEAD_FRAMES: usize = FRAMES_PER_S / 4;
const TAIL_FRAMES: usize = FRAMES_PER_S * 2 / 5;

/// The samples of `pcm` from just before its first speech to just after its last, or None when it has too
/// little speech to hold a word. Whisper, given silence or noise, writes something anyway, often the words
/// it was prompted with; trimming the quiet off the ends leaves it nothing to make words from.
pub fn speech_span(pcm: &[i16], noise_db: Option<f32>) -> Option<std::ops::Range<usize>> {
    let speech = speech_frames(pcm, noise_db);
    if speech.iter().filter(|&&s| s).count() < MIN_SPEECH_FRAMES {
        return None;
    }
    let first = speech.iter().position(|&s| s)?;
    let last = speech.iter().rposition(|&s| s)?;
    let start = first.saturating_sub(LEAD_FRAMES) * FRAME;
    let end = ((last + 1 + TAIL_FRAMES) * FRAME).min(pcm.len());
    Some(start..end)
}

/// How long the end of `pcm` has been without speech, in seconds.
pub fn trailing_silence_s(pcm: &[i16], noise_db: Option<f32>) -> f32 {
    let speech = speech_frames(pcm, noise_db);
    speech.iter().rev().take_while(|s| !**s).count() as f32 / FRAMES_PER_S as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone for speech, faint noise for quiet.
    fn audio(parts: &[(bool, f32)]) -> Vec<i16> {
        let mut out = Vec::new();
        let mut seed = 1u32;
        for &(loud, seconds) in parts {
            for i in 0..(seconds * 16_000.0) as usize {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let noise = ((seed >> 16) as i16 % 60) as f32;
                let tone = if loud { (i as f32 * 0.07).sin() * 8000.0 } else { 0.0 };
                out.push((tone + noise) as i16);
            }
        }
        out
    }

    #[test]
    fn cuts_in_the_middle_of_a_pause_after_speech() {
        let pcm = audio(&[(false, 0.5), (true, 30.0), (false, 1.0), (true, 1.0)]);
        let cut = find_cut(&pcm, Some(-40.0)).unwrap();
        assert!(cut.speech);
        // The pause starts at 30.5 s; 0.8 s of it is enough once the clip has 30 s, so the cut is 0.4 s in.
        let at_s = cut.at as f32 / 16_000.0;
        assert!((30.8..31.0).contains(&at_s), "{at_s}");
    }

    #[test]
    fn pauses_between_sentences_dont_cut_a_short_clip() {
        // Sentences with pauses of a second or two between them stay together until there's 30 s.
        let talk = audio(&[(true, 4.0), (false, 1.5), (true, 6.0), (false, 2.5), (true, 5.0), (false, 1.0), (true, 3.0), (false, 0.3)]);
        assert_eq!(find_cut(&talk, Some(-40.0)), None);
        // A 4 s silence is a stop, so it's cut there.
        let cut = find_cut(&audio(&[(true, 5.0), (false, 4.2), (true, 0.5)]), Some(-40.0)).unwrap();
        assert!(cut.speech && (cut.at as f32 / 16_000.0 - 7.0).abs() < 0.05, "{}", cut.at as f32 / 16_000.0);
    }

    #[test]
    fn waits_while_someone_is_still_talking() {
        assert_eq!(find_cut(&audio(&[(false, 0.3), (true, 3.0)]), None), None);
        assert_eq!(find_cut(&audio(&[(true, 2.0), (false, 0.3)]), Some(-40.0)), None);
    }

    #[test]
    fn speech_span_trims_the_quiet_and_ignores_clicks() {
        let span = speech_span(&audio(&[(false, 2.0), (true, 1.0), (false, 3.0)]), Some(-40.0)).unwrap();
        let s = |n: usize| n as f32 / 16_000.0;
        assert!((s(span.start) - 1.75).abs() < 0.03, "{}", s(span.start));
        assert!((s(span.end) - 3.4).abs() < 0.03, "{}", s(span.end));
        // Nothing but quiet, or a 60 ms click in it, holds no words.
        assert_eq!(speech_span(&audio(&[(false, 2.0)]), Some(-40.0)), None);
        assert_eq!(speech_span(&audio(&[(false, 1.0), (true, 0.06), (false, 1.0)]), Some(-40.0)), None);
    }

    #[test]
    fn lets_silence_go_in_pieces() {
        let cut = find_cut(&audio(&[(false, 3.2)]), Some(-40.0)).unwrap();
        assert!(!cut.speech);
        assert_eq!(cut.at, (160 - 25) * FRAME);
        assert!(trailing_silence_s(&audio(&[(true, 1.0), (false, 2.0)]), Some(-40.0)) > 1.9);
    }

    #[test]
    fn someone_who_never_pauses_is_cut_within_a_minute() {
        assert_eq!(find_cut(&audio(&[(true, 50.0)]), Some(-40.0)), None);
        let cut = find_cut(&audio(&[(true, 57.0)]), Some(-40.0)).unwrap();
        assert!(cut.speech && cut.at <= 55 * 16_000 && cut.at >= 45 * 16_000);
    }
}
