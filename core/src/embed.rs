//! A basic description of how someone sounds: the mean and spread of their MFCCs over voiced audio.
//! Two voices are compared with the Bhattacharyya distance between those diagonal Gaussians. It needs no
//! model download and separates distinct voices on one microphone well; it is weaker across different
//! microphones, so a neural speaker model can replace it later behind the same `Voice` type.

use serde::{Deserialize, Serialize};

pub(crate) const FRAME: usize = 400; // 25 ms at 16 kHz
pub(crate) const HOP: usize = 160; // 10 ms
pub(crate) const FFT: usize = 512;
pub(crate) const MELS: usize = 26;
/// Cepstra 1..=CEPS; c0 is loudness, which says more about the microphone than the person.
const CEPS: usize = 19;
/// Frames this far below the loudest are breath, room and other people, not the voice.
const VOICED_RANGE_DB: f32 = 30.0;
const MIN_FRAME_DB: f32 = -55.0;
/// Fewer voiced frames than this (0.4 s) give statistics too noisy to compare.
pub const MIN_FRAMES: u32 = 40;
/// Distance at which similarity falls to half; calibrated on distinct and matching voices in real meetings,
/// so the same person scores as they did before the background was taken off.
const HALF_DISTANCE: f32 = 0.055;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    pub mean: Vec<f32>,
    pub var: Vec<f32>,
    /// Voiced 10 ms frames behind the statistics, used to weight merges.
    pub frames: u32,
}

impl Voice {
    pub fn seconds(&self) -> f32 {
        self.frames as f32 * HOP as f32 / 16_000.0
    }
}

pub(crate) fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

fn mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Triangular filters over the FFT bins from 100 Hz to 7.6 kHz.
pub(crate) fn filterbank(rate: u32) -> Vec<Vec<(usize, f32)>> {
    let (low, high) = (mel(100.0), mel(7600.0f32.min(rate as f32 / 2.0)));
    let edges: Vec<f32> = (0..MELS + 2)
        .map(|i| {
            let m = low + (high - low) * i as f32 / (MELS + 1) as f32;
            700.0 * (10f32.powf(m / 2595.0) - 1.0) * FFT as f32 / rate as f32
        })
        .collect();
    (0..MELS)
        .map(|m| {
            (0..=FFT / 2)
                .filter_map(|bin| {
                    let b = bin as f32;
                    let w = if b < edges[m + 1] { (b - edges[m]) / (edges[m + 1] - edges[m]) } else { (edges[m + 2] - b) / (edges[m + 2] - edges[m + 1]) };
                    (w > 0.0).then_some((bin, w))
                })
                .collect()
        })
        .collect()
}

/// Pitch is looked for from 80 to 400 Hz, which covers adult and children's voices.
const MIN_PITCH_HZ: f32 = 80.0;
const MAX_PITCH_HZ: f32 = 400.0;
/// A frame is voice when it repeats itself at a speaking pitch at least this closely (Boersma's threshold).
/// Vowels and other voiced sounds do; taps, clicks, knocks, breath and fan noise don't.
const VOICED_CORRELATION: f32 = 0.45;
/// The room's steady background in each band is the level of its quieter frames at this percentile.
const NOISE_PERCENTILE: f32 = 0.2;
/// Frames between the speech that are needed to measure the background; fewer, and nothing is taken off.
const MIN_NOISE_FRAMES: usize = 20;
/// Taking the background off leaves at least this share of a band, so a band is never emptied.
const NOISE_KEEP: f32 = 0.1;
/// The autocorrelation is taken over twice the frame, so long pitch periods don't wrap around.
const ACF: usize = 1024;

/// How closely each windowed frame repeats itself at a speaking pitch, from 0 to 1.
struct Pitch {
    window_acf: Vec<f32>,
    lags: std::ops::RangeInclusive<usize>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Pitch {
    fn new(window: &[f32], rate: u32) -> Self {
        let window_acf = (0..FRAME).map(|lag| window[..FRAME - lag].iter().zip(&window[lag..]).map(|(a, b)| a * b).sum()).collect();
        let lags = (rate as f32 / MAX_PITCH_HZ) as usize..=((rate as f32 / MIN_PITCH_HZ) as usize).min(FRAME / 2);
        Self { window_acf, lags, re: vec![0.0; ACF], im: vec![0.0; ACF] }
    }

    /// The autocorrelation of the windowed frame, corrected for the window's own (Boersma, 1993).
    fn voicing(&mut self, frame: &[i16], window: &[f32]) -> f32 {
        self.re.fill(0.0);
        self.im.fill(0.0);
        let mean = frame.iter().map(|&x| x as f32).sum::<f32>() / FRAME as f32;
        for i in 0..FRAME {
            self.re[i] = (frame[i] as f32 - mean) * window[i];
        }
        fft(&mut self.re, &mut self.im);
        for i in 0..ACF {
            self.re[i] = self.re[i] * self.re[i] + self.im[i] * self.im[i];
            self.im[i] = 0.0;
        }
        // The inverse transform of a real, even spectrum is the forward one, scaled.
        fft(&mut self.re, &mut self.im);
        let zero = self.re[0].max(1e-9);
        self.lags.clone().map(|lag| self.re[lag] / zero / (self.window_acf[lag] / self.window_acf[0]).max(1e-3)).fold(0.0f32, f32::max).min(1.0)
    }
}

/// MFCCs of the frames where someone is speaking: loud enough and voiced, with the steady background of
/// the room taken off each band first.
fn mfccs(pcm: &[i16], rate: u32) -> Vec<[f32; CEPS]> {
    if pcm.len() < FRAME {
        return Vec::new();
    }
    let window: Vec<f32> = (0..FRAME).map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FRAME - 1) as f32).cos()).collect();
    let bank = filterbank(rate);
    let starts: Vec<usize> = (0..=(pcm.len() - FRAME) / HOP).map(|i| i * HOP).collect();
    let db = |s: usize| {
        let p = pcm[s..s + FRAME].iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / FRAME as f64;
        (10.0 * (p.max(1.0) / (32768.0f64 * 32768.0)).log10()) as f32
    };
    let levels: Vec<f32> = starts.iter().map(|&s| db(s)).collect();
    let loudest = levels.iter().copied().fold(f32::MIN, f32::max);
    let floor = (loudest - VOICED_RANGE_DB).max(MIN_FRAME_DB);

    // Each frame's band energies, and whether it is someone speaking.
    let mut pitch = Pitch::new(&window, rate);
    let (mut re, mut im) = (vec![0.0; FFT], vec![0.0; FFT]);
    let mut bands: Vec<Vec<f32>> = Vec::with_capacity(starts.len());
    let mut speech: Vec<bool> = Vec::with_capacity(starts.len());
    for (&start, &level) in starts.iter().zip(&levels) {
        re.fill(0.0);
        im.fill(0.0);
        let frame = &pcm[start..start + FRAME];
        for i in 0..FRAME {
            // Pre-emphasis lifts the higher formants that tell voices apart.
            let prev = if i > 0 { frame[i - 1] as f32 } else { 0.0 };
            re[i] = (frame[i] as f32 - 0.97 * prev) / 32768.0 * window[i];
        }
        fft(&mut re, &mut im);
        bands.push(bank.iter().map(|f| f.iter().map(|&(b, w)| w * (re[b] * re[b] + im[b] * im[b])).sum::<f32>()).collect());
        speech.push(level >= floor && pitch.voicing(frame, &window) >= VOICED_CORRELATION);
    }

    // The background is measured in the frames that aren't speech, when there are enough of them.
    let quiet: Vec<&Vec<f32>> = bands.iter().zip(&speech).filter(|(_, &s)| !s).map(|(b, _)| b).collect();
    let noise: Vec<f32> = if quiet.len() >= MIN_NOISE_FRAMES {
        (0..MELS)
            .map(|m| {
                let mut values: Vec<f32> = quiet.iter().map(|b| b[m]).collect();
                values.sort_by(f32::total_cmp);
                values[((values.len() - 1) as f32 * NOISE_PERCENTILE) as usize]
            })
            .collect()
    } else {
        vec![0.0; MELS]
    };

    let mut out = Vec::new();
    for (energies, _) in bands.iter().zip(&speech).filter(|(_, &s)| s) {
        let logs: Vec<f32> = energies.iter().zip(&noise).map(|(&e, &n)| (e - n).max(e * NOISE_KEEP).max(1e-10).ln()).collect();
        let mut c = [0.0; CEPS];
        for (k, value) in c.iter_mut().enumerate() {
            let n = (k + 1) as f32;
            *value = logs
                .iter()
                .enumerate()
                .map(|(m, e)| e * (std::f32::consts::PI * n * (m as f32 + 0.5) / MELS as f32).cos())
                .sum::<f32>()
                * (2.0 / MELS as f32).sqrt();
        }
        out.push(c);
    }
    out
}

/// None when there is too little voiced audio to describe the voice.
pub fn voice(pcm: &[i16], rate: u32) -> Option<Voice> {
    let frames = mfccs(pcm, rate);
    if (frames.len() as u32) < MIN_FRAMES {
        return None;
    }
    let n = frames.len() as f32;
    let mean: Vec<f32> = (0..CEPS).map(|k| frames.iter().map(|f| f[k]).sum::<f32>() / n).collect();
    let var: Vec<f32> = (0..CEPS).map(|k| (frames.iter().map(|f| (f[k] - mean[k]).powi(2)).sum::<f32>() / n).max(1e-4)).collect();
    Some(Voice { mean, var, frames: frames.len() as u32 })
}

/// Bhattacharyya distance per dimension; 0 for identical voices.
pub fn distance(a: &Voice, b: &Voice) -> f32 {
    if a.mean.len() != b.mean.len() {
        return f32::INFINITY;
    }
    let total: f32 = (0..a.mean.len())
        .map(|k| {
            let v = (a.var[k] + b.var[k]) / 2.0;
            (a.mean[k] - b.mean[k]).powi(2) / (8.0 * v) + 0.5 * (v / (a.var[k] * b.var[k]).sqrt()).ln()
        })
        .sum();
    total / a.mean.len() as f32
}

/// From 1 for the same voice towards 0 for very different ones.
pub fn similarity(a: &Voice, b: &Voice) -> f32 {
    0.5f32.powf(distance(a, b) / HALF_DISTANCE)
}

/// The voice of both together, as if their audio had been measured at once.
pub fn merge(a: &Voice, b: &Voice) -> Voice {
    let (na, nb) = (a.frames as f32, b.frames as f32);
    let n = na + nb;
    let mean: Vec<f32> = (0..a.mean.len()).map(|k| (na * a.mean[k] + nb * b.mean[k]) / n).collect();
    let var = (0..a.mean.len())
        .map(|k| ((na * (a.var[k] + a.mean[k].powi(2)) + nb * (b.var[k] + b.mean[k].powi(2))) / n - mean[k].powi(2)).max(1e-4))
        .collect();
    Voice { mean, var, frames: a.frames + b.frames }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buzzy vowel-like tone: a pitch with harmonics shaped by two formants.
    fn vowel(pitch: f32, f1: f32, f2: f32, seconds: f32, seed: u32) -> Vec<i16> {
        let rate = 16_000.0;
        (0..(seconds * rate) as usize)
            .map(|i| {
                let t = i as f32 / rate;
                let wobble = 1.0 + 0.02 * (t * 5.0 + seed as f32).sin();
                let mut s = 0.0;
                for h in 1..40 {
                    let f = pitch * wobble * h as f32;
                    let gain = 1.0 / (1.0 + ((f - f1) / 150.0).powi(2)) + 0.6 / (1.0 + ((f - f2) / 200.0).powi(2));
                    s += gain * (2.0 * std::f32::consts::PI * f * t).sin();
                }
                (s * 3000.0) as i16
            })
            .collect()
    }

    #[test]
    fn the_same_voice_is_closer_than_a_different_one() {
        let a1 = voice(&vowel(120.0, 700.0, 1200.0, 2.0, 1), 16_000).unwrap();
        let a2 = voice(&vowel(122.0, 700.0, 1200.0, 2.0, 7), 16_000).unwrap();
        let b = voice(&vowel(220.0, 400.0, 2300.0, 2.0, 3), 16_000).unwrap();
        assert!(similarity(&a1, &a2) > 0.7, "{}", similarity(&a1, &a2));
        assert!(similarity(&a1, &b) < 0.3, "{}", similarity(&a1, &b));
    }

    fn noise(len: usize, seed: u32) -> impl Iterator<Item = f32> {
        let mut x = seed.max(1);
        (0..len).map(move |_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 2.0
        })
    }

    /// Speech with pauses, as someone reading: half a second of voice, then a quarter of a second of quiet.
    fn reading(seconds: f32) -> Vec<i16> {
        let voice = vowel(120.0, 700.0, 1200.0, seconds, 1);
        voice.iter().enumerate().map(|(i, &s)| if i % 12_000 < 8_000 { s } else { 0 }).collect()
    }

    #[test]
    fn typing_taps_hardly_change_a_voice() {
        let clean = reading(6.0);
        // A loud click every 150 ms: a decaying burst of noise, louder than the voice.
        let mut tapped = clean.clone();
        for (start, n) in (0..tapped.len() - 1200).step_by(2_400).zip(0u32..) {
            for (i, v) in noise(1200, n + 1).enumerate() {
                let s = &mut tapped[start + i];
                *s = (*s as f32 + v * 20_000.0 * (-(i as f32) / 90.0).exp()).clamp(-32768.0, 32767.0) as i16;
            }
        }
        let (a, b) = (voice(&clean, 16_000).unwrap(), voice(&tapped, 16_000).unwrap());
        // Counting loud frames rather than voiced ones scored this 0.77.
        assert!(similarity(&a, &b) > 0.8, "{}", similarity(&a, &b));
        // Taps alone are no voice at all.
        let only_taps: Vec<i16> = tapped.iter().zip(&clean).map(|(&t, &c)| t.saturating_sub(c)).collect();
        assert!(voice(&only_taps, 16_000).is_none());
    }

    #[test]
    fn noise_between_the_words_is_not_counted_as_voice() {
        let clean = reading(6.0);
        // A fan heard in the pauses, loud enough to have counted as voice by level alone.
        let fan: Vec<i16> = clean.iter().zip(noise(clean.len(), 5)).map(|(&s, n)| if s == 0 { (n * 1500.0) as i16 } else { s }).collect();
        let (a, b) = (voice(&clean, 16_000).unwrap(), voice(&fan, 16_000).unwrap());
        assert_eq!(a.frames, b.frames, "only the voiced frames are used");
        // How much the noise is taken off the voice was measured on real meetings with a fan mixed in,
        // where the same stretch of speech went from 0.37 alike to 0.69; a steady test tone can't show it.
    }

    #[test]
    fn silence_and_short_clips_have_no_voice() {
        assert!(voice(&[0; 16_000], 16_000).is_none());
        assert!(voice(&vowel(120.0, 700.0, 1200.0, 0.2, 1), 16_000).is_none());
    }

    #[test]
    fn merging_weights_by_frames() {
        let a = Voice { mean: vec![0.0], var: vec![1.0], frames: 100 };
        let b = Voice { mean: vec![2.0], var: vec![1.0], frames: 300 };
        let m = merge(&a, &b);
        assert!((m.mean[0] - 1.5).abs() < 1e-5);
        assert!((m.var[0] - 1.75).abs() < 1e-5);
        assert_eq!(m.frames, 400);
    }
}
