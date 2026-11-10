//! Lining up a recording made on another device with a meeting's own tracks: where on the meeting's
//! timeline the other recording starts. Both are reduced to how loud they are moment to moment, which two
//! microphones in one room agree on even when they sound different, and matched by cross-correlation:
//! first every 20 ms across the whole length, then every 2.5 ms around the best match.

use crate::attribute::window_db;

const COARSE: usize = 320;
const FINE: usize = 40;

/// Loudness every `window` samples, centred, with the quiet floor clipped so room noise doesn't count.
fn envelope(pcm: &[i16], window: usize) -> Vec<f32> {
    let db = window_db(pcm, window);
    let mean = db.iter().sum::<f32>() / db.len().max(1) as f32;
    db.iter().map(|&d| (d.max(-60.0) - mean.max(-60.0)).max(-10.0)).collect()
}

#[derive(Clone, Copy)]
struct C(f64, f64);

/// In-place iterative radix-2 FFT; `inverse` for the inverse (unscaled).
fn fft(a: &mut [C], inverse: bool) {
    let n = a.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = 2.0 * std::f64::consts::PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let w = C(angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let mut wk = C(1.0, 0.0);
            for k in 0..len / 2 {
                let u = a[start + k];
                let x = a[start + k + len / 2];
                let v = C(x.0 * wk.0 - x.1 * wk.1, x.0 * wk.1 + x.1 * wk.0);
                a[start + k] = C(u.0 + v.0, u.1 + v.1);
                a[start + k + len / 2] = C(u.0 - v.0, u.1 - v.1);
                wk = C(wk.0 * w.0 - wk.1 * w.1, wk.0 * w.1 + wk.1 * w.0);
            }
        }
        len <<= 1;
    }
}

/// Cross-correlation of `r` and `t` at every lag: element `k` of the result is lag `k - (t.len() - 1)`,
/// the sum over i of t[i] * r[i + lag].
fn cross_correlate(r: &[f32], t: &[f32]) -> Vec<f64> {
    let n = (r.len() + t.len()).next_power_of_two();
    let mut fr: Vec<C> = r.iter().map(|&x| C(x as f64, 0.0)).chain(std::iter::repeat(C(0.0, 0.0))).take(n).collect();
    let mut ft: Vec<C> = t.iter().map(|&x| C(x as f64, 0.0)).chain(std::iter::repeat(C(0.0, 0.0))).take(n).collect();
    fft(&mut fr, false);
    fft(&mut ft, false);
    let mut prod: Vec<C> = fr.iter().zip(&ft).map(|(a, b)| C(a.0 * b.0 + a.1 * b.1, a.1 * b.0 - a.0 * b.1)).collect();
    fft(&mut prod, true);
    let lags = r.len() + t.len() - 1;
    (0..lags)
        .map(|k| {
            let lag = k as i64 - (t.len() as i64 - 1);
            prod[lag.rem_euclid(n as i64) as usize].0 / n as f64
        })
        .collect()
}

/// Where `track` starts on `reference`'s timeline, in seconds (negative when it started earlier), and how
/// sure the match is: the best match's height over the next best, from 1 (no better than chance) up.
/// None when either is too short to tell.
pub fn find_offset(reference: &[i16], track: &[i16], rate: u32) -> Option<(f64, f32)> {
    let (r, t) = (envelope(reference, COARSE), envelope(track, COARSE));
    if r.len() < 100 || t.len() < 100 {
        return None;
    }
    let corr = cross_correlate(&r, &t);
    let best = (0..corr.len()).max_by(|&a, &b| corr[a].total_cmp(&corr[b]))?;
    // The next best match away from this one, for how much it stands out.
    let runner_up = corr
        .iter()
        .enumerate()
        .filter(|(k, _)| k.abs_diff(best) > 25)
        .map(|(_, &c)| c)
        .fold(f64::MIN, f64::max)
        .max(1e-9);
    let confidence = (corr[best] / runner_up).max(0.0) as f32;
    let coarse_lag = best as i64 - (t.len() as i64 - 1);

    // Refined to 2.5 ms around the coarse match.
    let (rf, tf) = (envelope(reference, FINE), envelope(track, FINE));
    let step = (COARSE / FINE) as i64;
    let score = |lag: i64| -> f64 {
        let mut sum = 0.0;
        for (i, &x) in tf.iter().enumerate() {
            let j = i as i64 + lag;
            if j >= 0 && (j as usize) < rf.len() {
                sum += x as f64 * rf[j as usize] as f64;
            }
        }
        sum
    };
    let fine_lag = (coarse_lag * step - step..=coarse_lag * step + step).max_by(|&a, &b| score(a).total_cmp(&score(b)))?;
    Some((fine_lag as f64 * FINE as f64 / rate as f64, confidence))
}

/// How a recording lines up with the meeting along its whole length: meeting time = `offset_s` +
/// track time × `rate`. A rate off 1 is two clocks running at slightly different speeds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub offset_s: f64,
    pub rate: f64,
    /// The typical match's height over the next best, as `find_offset`'s.
    pub confidence: f32,
    /// Stretches of the track that matched, of those tried.
    pub matched: u32,
    pub tried: u32,
}

const WINDOW_S: f64 = 30.0;
const HOP_S: f64 = 15.0;
/// A stretch counts when its match stands out this much.
const CLEAR: f32 = 1.15;
/// Clocks this close are taken as the same; ones further apart than this are a wrong match.
const SAME_RATE: f64 = 1e-5;
const MAX_DRIFT: f64 = 0.005;

fn centred(x: &[f32]) -> Vec<f32> {
    let mean = x.iter().sum::<f32>() / x.len().max(1) as f32;
    x.iter().map(|v| v - mean).collect()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// offset(frame) = a + b × frame, by least squares over (frame, offset, _) points.
fn line(points: &[(f64, f64, f32)]) -> (f64, f64) {
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (my - b * mx, b)
}

/// Lines `track` up with `reference`, starting from it beginning about `around_s` into the meeting and
/// looking up to `search_s` either side: each half-minute of the track is matched on its own, and a line
/// through where they fall gives the start and any drift between the two clocks. None when no stretch
/// matches clearly.
pub fn fit(reference: &[i16], track: &[i16], rate: u32, around_s: f64, search_s: f64) -> Option<Fit> {
    let fps = rate as f64 / COARSE as f64;
    let (r, t) = (envelope(reference, COARSE), envelope(track, COARSE));
    if r.len() < 100 || t.len() < 100 {
        return None;
    }
    let w = ((WINDOW_S * fps) as usize).min(t.len());
    let hop = ((HOP_S * fps) as usize).max(1);
    let reach = (search_s * fps) as i64;
    let around = (around_s * fps).round() as i64;
    // (track frame at the stretch's middle, offset in frames, confidence)
    let mut found: Vec<(f64, f64, f32)> = Vec::new();
    let mut tried = 0;
    let mut start = 0;
    loop {
        let window = &t[start..start + w];
        let expected = start as i64 + around;
        let from = (expected - reach).max(0);
        let to = (expected + w as i64 + reach).min(r.len() as i64);
        // Only where the meeting covers most of the stretch, and the stretch has some talk in it.
        let loudest = window.iter().copied().fold(f32::MIN, f32::max);
        let quietest = window.iter().copied().fold(f32::MAX, f32::min);
        if to - from >= w as i64 * 3 / 4 && loudest - quietest > 6.0 {
            tried += 1;
            let slice = centred(&r[from as usize..to as usize]);
            let corr = cross_correlate(&slice, &centred(window));
            // Lags where the stretch lies at least half within the slice.
            let (lo, hi) = (w / 2, corr.len().saturating_sub(w / 2));
            if let Some(best) = (lo..hi).max_by(|&a, &b| corr[a].total_cmp(&corr[b])) {
                let runner_up = (lo..hi).filter(|k| k.abs_diff(best) > 25).map(|k| corr[k]).fold(f64::MIN, f64::max).max(1e-9);
                let confidence = (corr[best] / runner_up).max(0.0) as f32;
                let lag = best as i64 - (w as i64 - 1);
                if confidence >= CLEAR && corr[best] > 0.0 {
                    found.push(((start + w / 2) as f64, (from + lag - start as i64) as f64, confidence));
                }
            }
        }
        if start + w >= t.len() {
            break;
        }
        start = (start + hop).min(t.len() - w);
    }
    if found.is_empty() {
        return None;
    }
    // Fitted, then fitted again without the stretches far off the line.
    let centre = median(found.iter().map(|p| p.1).collect());
    let mut kept: Vec<_> = found.iter().copied().filter(|p| (p.1 - centre).abs() <= 0.5 * fps).collect();
    if kept.is_empty() {
        kept = found.clone();
    }
    let span = kept.iter().map(|p| p.0).fold(f64::MIN, f64::max) - kept.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let (mut a, mut b) = (median(kept.iter().map(|p| p.1).collect()), 0.0);
    if kept.len() >= 3 && span >= 60.0 * fps {
        (a, b) = line(&kept);
        kept.retain(|p| (p.1 - (a + b * p.0)).abs() <= 0.1 * fps);
        if kept.len() >= 3 {
            (a, b) = line(&kept);
        }
        if b.abs() < SAME_RATE || b.abs() > MAX_DRIFT || kept.len() < 3 {
            b = 0.0;
            a = median(kept.iter().map(|p| p.1).collect::<Vec<_>>().into_iter().chain([a]).collect());
        }
    }

    // The start to 2.5 ms, from the clearest stretch, with the drift taken out.
    let best = kept.iter().copied().max_by(|x, y| x.2.total_cmp(&y.2)).unwrap_or(found[0]);
    let (rf, tf) = (envelope(reference, FINE), envelope(track, FINE));
    let step = (COARSE / FINE) as i64;
    let (fw, fstart) = (w as i64 * step, (best.0 as i64 - w as i64 / 2) * step);
    let predicted = ((a + b * best.0) * step as f64).round() as i64;
    let score = |shift: i64| -> f64 {
        let mut sum = 0.0;
        for i in fstart.max(0)..(fstart + fw).min(tf.len() as i64) {
            let j = i + shift;
            if j >= 0 && (j as usize) < rf.len() {
                sum += tf[i as usize] as f64 * rf[j as usize] as f64;
            }
        }
        sum
    };
    let fine = (predicted - step..=predicted + step).max_by(|&x, &y| score(x).total_cmp(&score(y))).unwrap_or(predicted);
    a += (fine - predicted) as f64 / step as f64;

    let mut confidences: Vec<f32> = kept.iter().map(|p| p.2).collect();
    confidences.sort_by(f32::total_cmp);
    Some(Fit {
        offset_s: a / fps,
        rate: 1.0 + b,
        confidence: confidences.get(confidences.len() / 2).copied().unwrap_or(best.2),
        matched: kept.len() as u32,
        tried,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like bursts: a tone switched on and off at random, over faint noise.
    fn talk(seconds: f32, seed: u32) -> Vec<i16> {
        let mut s = seed;
        let mut next = move || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            s >> 8
        };
        let mut out = Vec::new();
        while out.len() < (seconds * 16_000.0) as usize {
            let on = next() % 3 != 0;
            let len = 1600 + (next() % 8000) as usize;
            let pitch = 0.03 + (next() % 50) as f32 / 1000.0;
            for i in 0..len {
                let noise = (next() % 200) as f32 - 100.0;
                out.push(((if on { (i as f32 * pitch).sin() * 6000.0 } else { 0.0 }) + noise) as i16);
            }
        }
        out
    }

    #[test]
    fn finds_where_a_later_recording_starts() {
        let meeting = talk(60.0, 7);
        // Another device started 12.3 s in, a little quieter and noisier.
        let start = (12.3 * 16_000.0) as usize;
        let phone: Vec<i16> = meeting[start..start + 30 * 16_000].iter().enumerate().map(|(i, &x)| x / 2 + ((i * 7919) % 150) as i16).collect();
        let (offset, confidence) = find_offset(&meeting, &phone, 16_000).unwrap();
        assert!((offset - 12.3).abs() < 0.01, "{offset}");
        assert!(confidence > 1.2, "{confidence}");
    }

    #[test]
    fn finds_a_recording_that_started_first() {
        let long = talk(50.0, 11);
        let meeting = long[(5 * 16_000)..].to_vec();
        let (offset, _) = find_offset(&meeting, &long, 16_000).unwrap();
        assert!((offset + 5.0).abs() < 0.01, "{offset}");
    }

    /// The audio as another device whose clock runs `rate` times as fast heard it.
    fn resampled(pcm: &[i16], rate: f64) -> Vec<i16> {
        let n = (pcm.len() as f64 / rate) as usize;
        (0..n)
            .map(|k| {
                let x = k as f64 * rate;
                let (i, f) = (x as usize, x.fract());
                let (a, b) = (pcm[i.min(pcm.len() - 1)] as f64, pcm[(i + 1).min(pcm.len() - 1)] as f64);
                (a + (b - a) * f) as i16
            })
            .collect()
    }

    #[test]
    fn fits_near_a_time_of_day_guess_and_finds_drift() {
        let meeting = talk(400.0, 3);
        // A phone started 40 s in; its clock runs 0.1 % slow, so its copy of the meeting is a little longer.
        let start = 40 * 16_000;
        let phone: Vec<i16> = resampled(&meeting[start..start + 300 * 16_000], 1.0 / 1.001)
            .iter()
            .enumerate()
            .map(|(i, &x)| x / 2 + ((i * 7919) % 150) as i16)
            .collect();
        // Its file said it started at 0:38, two seconds out.
        let fit = fit(&meeting, &phone, 16_000, 38.0, 60.0).unwrap();
        assert!((fit.offset_s - 40.0).abs() < 0.03, "{fit:?}");
        assert!((fit.rate - 1.0 / 1.001).abs() < 0.0002, "{fit:?}");
        assert!(fit.matched >= 10, "{fit:?}");
    }

    #[test]
    fn same_clocks_have_no_drift() {
        let meeting = talk(200.0, 8);
        let phone: Vec<i16> = meeting[10 * 16_000..190 * 16_000].iter().map(|&x| x / 3).collect();
        let fit = fit(&meeting, &phone, 16_000, 0.0, 30.0).unwrap();
        assert_eq!(fit.rate, 1.0, "{fit:?}");
        assert!((fit.offset_s - 10.0).abs() < 0.01, "{fit:?}");
    }

    #[test]
    fn a_recording_of_something_else_does_not_fit() {
        let meeting = talk(200.0, 5);
        let other = talk(120.0, 99);
        assert!(fit(&meeting, &other, 16_000, 20.0, 30.0).is_none_or(|f| f.matched <= 1));
    }
}
