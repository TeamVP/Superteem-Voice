//! Speech ownership from aligned per-source energy. Results are metadata: raw audio is never changed.

#[derive(Debug, Clone)]
pub struct Config {
    /// Above the source's noise floor counts as speech.
    pub vad_snr_db: f32,
    /// Sources within this of the strongest source share the window, which keeps overlapping speech.
    pub overlap_margin_db: f32,
    /// Further than this below the source's own speech level means it is not the wearer talking.
    pub wearer_margin_db: f32,
    /// Silent windows bridged inside a segment.
    pub hang_windows: usize,
    pub min_segment_windows: usize,
    pub gate_db: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vad_snr_db: 10.0,
            overlap_margin_db: 3.0,
            wearer_margin_db: 8.0,
            hang_windows: 6,
            min_segment_windows: 2,
            gate_db: -18.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The wearer's source, or None for a guest with no microphone of their own.
    pub speaker_source: Option<usize>,
    /// The source to transcribe from.
    pub audio_source: usize,
    pub start_window: usize,
    pub end_window: usize,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GainRange {
    pub source: usize,
    pub start_window: usize,
    pub end_window: usize,
    pub gain_db: f32,
}

#[derive(Debug, Default)]
pub struct Attribution {
    pub segments: Vec<Segment>,
    pub gains: Vec<GainRange>,
}

/// RMS level in dBFS per window.
pub fn window_db(pcm: &[i16], window: usize) -> Vec<f32> {
    pcm.chunks(window)
        .map(|c| {
            let power = c.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / c.len() as f64;
            (10.0 * (power.max(1.0) / (32768.0f64 * 32768.0)).log10()) as f32
        })
        .collect()
}

fn percentile(values: &[f32], p: f32) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() - 1) as f32 * p) as usize]
}

struct Run {
    speaker: Option<usize>,
    audio: usize,
    start: usize,
    last: usize,
    confidence_sum: f32,
    count: usize,
}

pub fn attribute(sources: &[Vec<f32>], cfg: &Config) -> Attribution {
    let windows = sources.iter().map(Vec::len).max().unwrap_or(0);
    if windows == 0 {
        return Attribution::default();
    }
    // Levels are compared as SNR because raw pin and headset levels are not comparable.
    let floors: Vec<f32> = sources.iter().map(|s| if s.is_empty() { 0.0 } else { percentile(s, 0.10) }).collect();
    let speech: Vec<f32> = sources.iter().map(|s| if s.is_empty() { 0.0 } else { percentile(s, 0.98) }).collect();

    let mut finished: Vec<Run> = Vec::new();
    let mut open: Vec<Run> = Vec::new();
    for w in 0..windows {
        let snr: Vec<f32> = sources
            .iter()
            .zip(&floors)
            .map(|(s, floor)| s.get(w).map_or(f32::MIN, |db| db - floor))
            .collect();
        let (top, top_snr) = snr.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let mut hits: Vec<(Option<usize>, usize, f32)> = Vec::new();
        if top_snr >= cfg.vad_snr_db {
            let is_owner = |i: usize| {
                snr[i] >= cfg.vad_snr_db
                    && snr[i] >= top_snr - cfg.overlap_margin_db
                    && sources[i][w] >= speech[i] - cfg.wearer_margin_db
            };
            let owners: Vec<usize> = (0..sources.len()).filter(|&i| is_owner(i)).collect();
            let rival = (0..sources.len())
                .filter(|i| !owners.contains(i))
                .map(|i| snr[i])
                .fold(cfg.vad_snr_db - 12.0, f32::max);
            if owners.is_empty() {
                // A quiet syllable mid-turn is still the wearer, not a guest.
                let wearer_talking = open.iter().any(|r| r.speaker == Some(top));
                hits.push((wearer_talking.then_some(top), top, 0.4));
            } else {
                for &o in &owners {
                    let confidence = if owners.len() > 1 { 0.5 } else { ((snr[o] - rival) / 12.0).clamp(0.0, 1.0) };
                    hits.push((Some(o), o, confidence));
                }
            }
        }
        for (speaker, audio, confidence) in hits {
            // A quiet start that turns into the wearer at full level was the wearer all along.
            if speaker.is_some() && !open.iter().any(|r| r.speaker == speaker) {
                if let Some(run) = open.iter_mut().find(|r| r.speaker.is_none() && r.audio == audio) {
                    run.speaker = speaker;
                }
            }
            match open.iter_mut().find(|r| r.speaker == speaker) {
                Some(run) => {
                    run.last = w;
                    run.confidence_sum += confidence;
                    run.count += 1;
                }
                None => open.push(Run { speaker, audio, start: w, last: w, confidence_sum: confidence, count: 1 }),
            }
        }
        let (done, keep): (Vec<Run>, Vec<Run>) = open.drain(..).partition(|r| w - r.last > cfg.hang_windows);
        open = keep;
        finished.extend(done);
    }
    finished.extend(open);

    let mut segments: Vec<Segment> = finished
        .into_iter()
        .filter(|r| r.count >= cfg.min_segment_windows)
        .map(|r| Segment {
            speaker_source: r.speaker,
            audio_source: r.audio,
            start_window: r.start,
            end_window: r.last + 1,
            confidence: r.confidence_sum / r.count as f32,
        })
        .collect();
    segments.sort_by_key(|s| (s.start_window, s.audio_source));

    let mut gains = Vec::new();
    for source in 0..sources.len() {
        let mut open_at = Some(0);
        let mut owned: Vec<(usize, usize)> =
            segments.iter().filter(|s| s.audio_source == source).map(|s| (s.start_window, s.end_window)).collect();
        owned.sort_unstable();
        let mut cursor = 0;
        for (start, end) in owned {
            if start > cursor {
                gains.push(GainRange { source, start_window: cursor, end_window: start, gain_db: cfg.gate_db });
            }
            cursor = cursor.max(end);
            open_at = Some(cursor);
        }
        if let Some(from) = open_at {
            if from < windows {
                gains.push(GainRange { source, start_window: from, end_window: windows, gain_db: cfg.gate_db });
            }
        }
    }
    Attribution { segments, gains }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: f32 = -70.0;
    const LOUD: f32 = -20.0;
    const BLEED: f32 = -34.0;

    /// Two sources, with a background of silence and `f` choosing the level per window.
    fn build(n: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<Vec<f32>> {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for w in 0..n {
            let (x, y) = f(w);
            a.push(x);
            b.push(y);
        }
        vec![a, b]
    }

    fn speakers(a: &Attribution) -> Vec<Option<usize>> {
        a.segments.iter().map(|s| s.speaker_source).collect()
    }

    #[test]
    fn gives_each_turn_to_the_wearer_despite_bleed() {
        let levels = build(300, |w| match w {
            20..=60 => (LOUD, BLEED),
            100..=140 => (BLEED, LOUD),
            _ => (QUIET, QUIET),
        });
        let a = attribute(&levels, &Config::default());
        assert_eq!(speakers(&a), vec![Some(0), Some(1)]);
        assert!(a.segments[0].confidence > 0.9);
    }

    #[test]
    fn keeps_both_speakers_during_overlap() {
        let levels = build(300, |w| match w {
            20..=60 => (LOUD, BLEED),
            61..=90 => (LOUD, LOUD),
            91..=130 => (BLEED, LOUD),
            _ => (QUIET, QUIET),
        });
        let a = attribute(&levels, &Config::default());
        let s0 = a.segments.iter().find(|s| s.speaker_source == Some(0)).unwrap();
        let s1 = a.segments.iter().find(|s| s.speaker_source == Some(1)).unwrap();
        assert!(s0.end_window > s1.start_window, "overlap lost: {:?}", a.segments);
    }

    #[test]
    fn unpinned_neighbour_becomes_a_guest() {
        let levels = build(400, |w| match w {
            20..=60 => (LOUD, BLEED),
            100..=140 => (BLEED, LOUD),
            200..=240 => (-34.0, -40.0),
            _ => (QUIET, QUIET),
        });
        let a = attribute(&levels, &Config::default());
        assert_eq!(speakers(&a), vec![Some(0), Some(1), None]);
        assert_eq!(a.segments[2].audio_source, 0);
    }

    #[test]
    fn quiet_syllables_stay_in_one_wearer_segment() {
        let levels = build(300, |w| match w {
            20..=30 => (-34.0, QUIET),
            31..=120 if w % 10 < 5 => (LOUD, QUIET),
            31..=120 => (-34.0, QUIET),
            _ => (QUIET, QUIET),
        });
        let a = attribute(&levels, &Config::default());
        assert_eq!(speakers(&a), vec![Some(0)]);
        assert_eq!((a.segments[0].start_window, a.segments[0].end_window), (20, 121));
    }

    #[test]
    fn a_silent_wearer_is_not_credited_with_bleed() {
        let levels = build(300, |w| if (20..=120).contains(&w) { (LOUD, BLEED) } else { (QUIET, QUIET) });
        assert_eq!(speakers(&attribute(&levels, &Config::default())), vec![Some(0)]);
    }

    #[test]
    fn gates_everything_a_source_does_not_own() {
        let levels = build(200, |w| match w {
            20..=60 => (LOUD, BLEED),
            _ => (QUIET, QUIET),
        });
        let a = attribute(&levels, &Config::default());
        assert_eq!(a.gains.iter().filter(|g| g.source == 1).count(), 1);
        let first = a.gains.iter().find(|g| g.source == 0).unwrap();
        assert_eq!((first.start_window, first.end_window), (0, 20));
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(attribute(&[], &Config::default()).segments.is_empty());
        assert!(attribute(&[vec![]], &Config::default()).segments.is_empty());
    }
}
