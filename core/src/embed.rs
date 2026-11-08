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
/// Distance at which similarity falls to half; calibrated on distinct and matching voices.
const HALF_DISTANCE: f32 = 0.05;

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

/// MFCCs of the voiced frames.
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
    let mut out = Vec::new();
    let (mut re, mut im) = (vec![0.0; FFT], vec![0.0; FFT]);
    for (&start, &level) in starts.iter().zip(&levels) {
        if level < floor {
            continue;
        }
        re.fill(0.0);
        im.fill(0.0);
        let frame = &pcm[start..start + FRAME];
        for i in 0..FRAME {
            // Pre-emphasis lifts the higher formants that tell voices apart.
            let prev = if i > 0 { frame[i - 1] as f32 } else { 0.0 };
            re[i] = (frame[i] as f32 - 0.97 * prev) / 32768.0 * window[i];
        }
        fft(&mut re, &mut im);
        let energies: Vec<f32> = bank
            .iter()
            .map(|f| f.iter().map(|&(b, w)| w * (re[b] * re[b] + im[b] * im[b])).sum::<f32>().max(1e-10).ln())
            .collect();
        let mut c = [0.0; CEPS];
        for (k, value) in c.iter_mut().enumerate() {
            let n = (k + 1) as f32;
            *value = energies
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
