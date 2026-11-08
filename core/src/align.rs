//! Clock offset and drift estimation, plus cross-correlation refinement.

/// Maps a source's sample position to host time:
/// arrival_us = offset_us + sample / rate * 1e6 * (1 + drift_ppm / 1e6)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockModel {
    pub offset_us: f64,
    pub drift_ppm: f64,
}

/// Fits a clock to (sample, arrival_us) pairs. Radio delay only ever adds latency, so the fit
/// uses the earliest arrival in each of up to 64 windows rather than every noisy point.
pub fn estimate_clock(observations: &[(u64, u64)], sample_rate: u32) -> Option<ClockModel> {
    if observations.len() < 2 {
        return None;
    }
    let chunk = (observations.len() / 64).max(1);
    let points: Vec<(f64, f64)> = observations
        .chunks(chunk)
        .map(|window| {
            let (sample, arrival) = window
                .iter()
                .map(|&(s, a)| (s as f64 * 1e6 / sample_rate as f64, a as f64))
                .min_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
                .unwrap();
            (sample, arrival - sample)
        })
        .collect();
    let n = points.len() as f64;
    let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let denom: f64 = points.iter().map(|p| (p.0 - mean_x).powi(2)).sum();
    if denom == 0.0 {
        return None;
    }
    let slope = points.iter().map(|p| (p.0 - mean_x) * (p.1 - mean_y)).sum::<f64>() / denom;
    Some(ClockModel { offset_us: mean_y - slope * mean_x, drift_ppm: slope * 1e6 })
}

/// Finds the lag of `b` relative to `a` in samples (positive means `b` is late) and its normalised
/// correlation score. Cost is proportional to length times search range, so callers pass short excerpts.
pub fn best_lag(a: &[f32], b: &[f32], max_lag: usize) -> Option<(i64, f32)> {
    let mut best: Option<(i64, f32)> = None;
    for lag in -(max_lag as i64)..=(max_lag as i64) {
        let (x, y) = if lag >= 0 {
            (a, b.get(lag as usize..).unwrap_or(&[]))
        } else {
            (a.get((-lag) as usize..).unwrap_or(&[]), b)
        };
        let n = x.len().min(y.len());
        if n == 0 {
            continue;
        }
        let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
        for i in 0..n {
            xy += (x[i] * y[i]) as f64;
            xx += (x[i] * x[i]) as f64;
            yy += (y[i] * y[i]) as f64;
        }
        if xx == 0.0 || yy == 0.0 {
            continue;
        }
        let score = (xy / (xx * yy).sqrt()) as f32;
        if best.map_or(true, |(_, s)| score > s) {
            best = Some((lag, score));
        }
    }
    best
}

/// Removes clock drift by resampling with linear interpolation.
/// Good enough for attribution windows; export-quality stems should move to a sinc resampler.
pub fn correct_drift(pcm: &[i16], drift_ppm: f64) -> Vec<i16> {
    if pcm.is_empty() || drift_ppm == 0.0 {
        return pcm.to_vec();
    }
    let ratio = 1.0 + drift_ppm / 1e6;
    let out_len = (pcm.len() as f64 * ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 / ratio;
            let index = pos as usize;
            let frac = pos - index as f64;
            let a = pcm[index.min(pcm.len() - 1)] as f64;
            let b = pcm[(index + 1).min(pcm.len() - 1)] as f64;
            (a + (b - a) * frac).round() as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_drift_and_offset_through_jitter() {
        let rate = 16_000u32;
        let mut seed = 12345u64;
        let observations: Vec<(u64, u64)> = (0..180_000u64) // one hour of 20 ms frames
            .map(|i| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let jitter = (seed >> 33) % 40_000;
                let sample = i * 320;
                let ideal = sample as f64 * 1e6 / rate as f64 * (1.0 + 100e-6);
                (sample, 2_000_000 + ideal as u64 + jitter)
            })
            .collect();
        let clock = estimate_clock(&observations, rate).unwrap();
        assert!((clock.drift_ppm - 100.0).abs() < 1.0, "{clock:?}");
        assert!((clock.offset_us - 2_000_000.0).abs() < 5_000.0, "{clock:?}");
    }

    #[test]
    fn too_few_observations_give_no_clock() {
        assert!(estimate_clock(&[(0, 0)], 16_000).is_none());
    }

    #[test]
    fn finds_a_known_lag_in_both_directions() {
        let signal: Vec<f32> = (0..4000).map(|i| ((i * i) as f32 * 0.0007).sin()).collect();
        let (a, b) = (&signal[100..3000], &signal[60..3000]);
        assert_eq!(best_lag(a, b, 200).unwrap().0, 40);
        assert_eq!(best_lag(b, a, 200).unwrap().0, -40);
    }

    #[test]
    fn drift_correction_changes_length_by_the_drift() {
        let pcm = vec![100i16; 1_000_000];
        assert_eq!(correct_drift(&pcm, 100.0).len(), 1_000_100);
        assert_eq!(correct_drift(&pcm, -100.0).len(), 999_900);
    }
}
