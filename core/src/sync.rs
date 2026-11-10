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
}
