//! Keyword spotting: hears the wake word and a few key phrases as they are said, apart from speech to text.
//! Each phrase is learnt from a handful of takes by one person, and live audio is matched against those
//! takes with streaming dynamic time warping over normalised cepstra. It needs no model download, costs a
//! small fraction of one core, and decides about a tenth of a second after the phrase ends. Takes from one
//! speaker make it work best for that speaker, which is why the phrases belong to a voice print.

use crate::embed::{fft, filterbank, FFT, FRAME, HOP, MELS};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const RATE: u32 = 16_000;
/// Cepstra c0..c12. c0 is kept here: after normalisation it is the loudness contour of the phrase.
const CEPS: usize = 13;
/// Each frame is the cepstra and how fast they change.
pub const DIM: usize = 2 * CEPS;
/// Live features are normalised over about the last 3 s, so a different microphone or room still matches.
const NORM_FRAMES: f32 = 300.0;
/// The normalisation starts as if it had already seen this many frames, so the first second is usable.
const NORM_PRIOR: f32 = 10.0;
const VAR_FLOOR: f32 = 1e-2;
/// Frames either side used for the rate of change, which delays every frame by this much.
const DELTA: usize = 2;
/// A path advances 0, 1 or 2 take frames per live frame, so a phrase is heard said up to twice as fast or
/// as slow as a take. Skipping costs a little so paths don't skip out of habit.
const SKIP_COST: f32 = 0.15;
/// A match is reported once nothing better has turned up for this many frames (100 ms).
const SETTLE: u64 = 10;
/// After a phrase is heard, nothing else is for this many frames, so one phrase is one hit.
const HOLDOFF: u64 = 30;
/// A phrase that needs the wake word must start this soon after it.
pub const REQUIRE_WINDOW_S: f32 = 6.0;
/// Each take is also kept with noise added at these signal-to-noise ratios (dB), so a phrase trained in a
/// quiet room is still heard in a noisy one.
const NOISY_SNR_DB: [f32; 2] = [24.0, 14.0];
/// Audio kept for each hit, for checking who said it.
const KEEP_AUDIO: usize = 4 * RATE as usize;

/// One frame of features.
pub type Frame = [f32; DIM];

/// Turns 16 kHz audio into normalised feature frames as it arrives.
pub struct Features {
    bank: Vec<Vec<(usize, f32)>>,
    window: Vec<f32>,
    buf: Vec<i16>,
    /// The sample before `buf`, for pre-emphasis across pushes.
    before: i16,
    mean: [f32; CEPS],
    var: [f32; CEPS],
    seen: f32,
    statics: VecDeque<[f32; CEPS]>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Default for Features {
    fn default() -> Self {
        Self::new()
    }
}

impl Features {
    pub fn new() -> Self {
        Self {
            bank: filterbank(RATE),
            window: (0..FRAME).map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FRAME - 1) as f32).cos()).collect(),
            buf: Vec::new(),
            before: 0,
            mean: [0.0; CEPS],
            var: [4.0; CEPS],
            seen: 0.0,
            statics: VecDeque::new(),
            re: vec![0.0; FFT],
            im: vec![0.0; FFT],
        }
    }

    /// Frame k of the output starts at sample k * 160 of everything pushed.
    pub fn push(&mut self, pcm: &[i16], out: &mut Vec<Frame>) {
        self.buf.extend_from_slice(pcm);
        let mut at = 0;
        while self.buf.len() - at >= FRAME {
            self.re.fill(0.0);
            self.im.fill(0.0);
            for i in 0..FRAME {
                let prev = if at + i > 0 { self.buf[at + i - 1] } else { self.before } as f32;
                self.re[i] = (self.buf[at + i] as f32 - 0.97 * prev) / 32768.0 * self.window[i];
            }
            fft(&mut self.re, &mut self.im);
            let energies: Vec<f32> = self
                .bank
                .iter()
                .map(|f| f.iter().map(|&(b, w)| w * (self.re[b] * self.re[b] + self.im[b] * self.im[b])).sum::<f32>().max(1e-10).ln())
                .collect();
            let mut c = [0.0; CEPS];
            for (k, value) in c.iter_mut().enumerate() {
                *value = energies
                    .iter()
                    .enumerate()
                    .map(|(m, e)| e * (std::f32::consts::PI * k as f32 * (m as f32 + 0.5) / MELS as f32).cos())
                    .sum::<f32>()
                    * (2.0 / MELS as f32).sqrt();
            }
            self.add_static(c, out);
            at += HOP;
        }
        if at > 0 {
            self.before = self.buf[at - 1];
            self.buf.drain(..at);
        }
    }

    /// Emits the last frames, which were waiting for the frames after them.
    pub fn finish(&mut self, out: &mut Vec<Frame>) {
        if let Some(&last) = self.statics.back() {
            for _ in 0..DELTA {
                self.statics.push_back(last);
                self.emit(out);
            }
        }
    }

    fn add_static(&mut self, c: [f32; CEPS], out: &mut Vec<Frame>) {
        if self.seen == 0.0 {
            self.mean = c;
        }
        self.seen += 1.0;
        let a = 1.0 / (self.seen + NORM_PRIOR).min(NORM_FRAMES);
        let mut normalised = [0.0; CEPS];
        for k in 0..CEPS {
            let d = c[k] - self.mean[k];
            self.mean[k] += a * d;
            self.var[k] += a * (d * d - self.var[k]);
            normalised[k] = (c[k] - self.mean[k]) / self.var[k].max(VAR_FLOOR).sqrt();
        }
        if self.statics.is_empty() {
            // The first frame stands in for the frames before it.
            self.statics.extend([normalised; DELTA]);
        }
        self.statics.push_back(normalised);
        self.emit(out);
    }

    fn emit(&mut self, out: &mut Vec<Frame>) {
        if self.statics.len() < 2 * DELTA + 1 {
            return;
        }
        let q = &self.statics;
        let mut frame = [0.0; DIM];
        for k in 0..CEPS {
            frame[k] = q[2][k];
            frame[CEPS + k] = (q[3][k] - q[1][k] + 2.0 * (q[4][k] - q[0][k])) / 10.0;
        }
        out.push(frame);
        self.statics.pop_front();
    }
}

/// All the feature frames of a recording.
pub fn features_of(pcm: &[i16]) -> Vec<Frame> {
    let mut features = Features::new();
    let mut out = Vec::with_capacity(pcm.len() / HOP + 1);
    features.push(pcm, &mut out);
    features.finish(&mut out);
    out
}

/// One take of a phrase, as feature frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    #[serde(with = "packed")]
    pub data: Vec<f32>,
    /// A copy of a take with noise added, used for listening but not for calibration.
    #[serde(default)]
    pub noisy: bool,
}

impl Template {
    pub fn new(frames: &[Frame]) -> Self {
        Self { data: frames.iter().flatten().copied().collect(), noisy: false }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / DIM
    }

    pub fn seconds(&self) -> f32 {
        (self.frames() * HOP) as f32 / RATE as f32
    }

    fn frame(&self, j: usize) -> &[f32] {
        &self.data[j * DIM..(j + 1) * DIM]
    }

    fn reversed(&self) -> Template {
        Template { data: (0..self.frames()).rev().flat_map(|j| self.frame(j).to_vec()).collect(), noisy: self.noisy }
    }
}

/// Stores feature values as base64 of 16-bit steps of 1/1000, a tenth of the size of JSON numbers.
mod packed {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    const SCALE: f32 = 1000.0;
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(data: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = data.iter().flat_map(|&x| ((x * SCALE).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes()).collect();
        let mut text = String::with_capacity(bytes.len() * 4 / 3 + 4);
        for chunk in bytes.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                text.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        s.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        let text = String::deserialize(d)?;
        let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
        for chunk in text.as_bytes().chunks(4) {
            let mut n = 0u32;
            let mut real = 0;
            for (i, &c) in chunk.iter().enumerate() {
                if c != b'=' {
                    let v = ALPHABET.iter().position(|&a| a == c).ok_or_else(|| D::Error::custom("bad base64"))?;
                    n |= (v as u32) << (18 - 6 * i);
                    real += 1;
                }
            }
            bytes.extend(&[(n >> 16) as u8, (n >> 8) as u8, n as u8][..real.max(1) - 1]);
        }
        Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / SCALE).collect())
    }
}

fn distance(a: &[f32], b: &[f32]) -> f32 {
    (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / DIM as f32).sqrt()
}

/// Every warping path through one take that ends at the latest live frame, kept per take frame.
#[derive(Clone)]
struct Track {
    cost: Vec<f32>,
    len: Vec<u32>,
    start: Vec<u64>,
    /// Whether the path stayed on this take frame last time; it may not stay twice running.
    stalled: Vec<bool>,
}

impl Track {
    fn new(frames: usize) -> Self {
        Self { cost: vec![f32::INFINITY; frames], len: vec![0; frames], start: vec![0; frames], stalled: vec![false; frames] }
    }

    fn reset(&mut self) {
        self.cost.fill(f32::INFINITY);
    }

    /// Adds live frame number `at`. Returns the average cost and start frame of the best path that has
    /// just reached the end of the take. Paths are compared by average cost, since their lengths differ.
    fn step(&mut self, take: &Template, x: &[f32], at: u64) -> Option<(f32, u64)> {
        let m = self.cost.len();
        // From the last take frame down, so each entry still holds the previous frame's path when read.
        for j in (0..m).rev() {
            let d = distance(x, take.frame(j));
            let mut best = (f32::INFINITY, f32::INFINITY, 0u32, 0u64, false);
            let mut consider = |cost: f32, len: u32, start: u64, stalled: bool| {
                if cost.is_finite() && cost / (len as f32) < best.0 {
                    best = (cost / len as f32, cost, len, start, stalled);
                }
            };
            if j == 0 {
                consider(d, 1, at, false);
            }
            if j >= 1 {
                consider(self.cost[j - 1] + d, self.len[j - 1] + 1, self.start[j - 1], false);
            }
            if j >= 2 {
                consider(self.cost[j - 2] + d + SKIP_COST, self.len[j - 2] + 1, self.start[j - 2], false);
            }
            if !self.stalled[j] {
                consider(self.cost[j] + d, self.len[j] + 1, self.start[j], true);
            }
            (self.cost[j], self.len[j], self.start[j], self.stalled[j]) = (best.1, best.2, best.3, best.4);
        }
        self.cost[m - 1].is_finite().then(|| (self.cost[m - 1] / self.len[m - 1] as f32, self.start[m - 1]))
    }
}

/// The lowest average cost of `take` matched anywhere within `input`.
pub fn match_cost(take: &Template, input: &Template) -> f32 {
    let mut track = Track::new(take.frames());
    (0..input.frames()).filter_map(|i| track.step(take, input.frame(i), i as u64)).map(|(c, _)| c).fold(f32::INFINITY, f32::min)
}

/// Where takes of a phrase are, in a recording of someone saying it several times with pauses between.
/// Returns sample ranges.
pub fn find_takes(pcm: &[i16]) -> Vec<(usize, usize)> {
    const BLOCK: usize = HOP;
    /// Pauses shorter than this are inside a phrase, such as between "hey" and "Superteem".
    const GAP: usize = 30;
    const MIN_LEN: usize = 25;
    const MAX_LEN: usize = 300;
    const PAD: usize = 3;
    let db: Vec<f32> = pcm
        .chunks_exact(BLOCK)
        .map(|b| {
            let p = b.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / BLOCK as f64;
            (10.0 * (p.max(1.0) / (32768.0f64 * 32768.0)).log10()) as f32
        })
        .collect();
    if db.is_empty() {
        return Vec::new();
    }
    let mut sorted = db.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted[sorted.len() / 10];
    let loud = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)];
    let floor = (noise + 12.0).max(loud - 35.0);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, &level) in db.iter().enumerate() {
        if level < floor {
            continue;
        }
        match runs.last_mut() {
            Some(last) if i - last.1 <= GAP => last.1 = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    runs.into_iter()
        .filter(|(s, e)| (MIN_LEN..=MAX_LEN).contains(&(e - s)))
        .map(|(s, e)| (s.saturating_sub(PAD) * BLOCK, ((e + PAD).min(db.len())) * BLOCK))
        .collect()
}

/// The takes in a recording of someone saying a phrase several times, ready to keep: each as said, then
/// each again with noise added.
pub fn takes_from(pcm: &[i16]) -> Vec<Template> {
    takes_at(pcm, &find_takes(pcm))
}

/// The takes at the given sample ranges of a recording, as `takes_from` makes them.
pub fn takes_at(pcm: &[i16], spans: &[(usize, usize)]) -> Vec<Template> {
    let cut = |frames: Vec<Frame>, noisy: bool| -> Vec<Template> {
        spans
            .iter()
            .map(|&(s, e)| Template { noisy, ..Template::new(&frames[(s / HOP).min(frames.len())..(e / HOP).min(frames.len())]) })
            .filter(|t| t.frames() > 0)
            .collect()
    };
    let mut out = cut(features_of(pcm), false);
    let (power, count) = spans.iter().flat_map(|&(s, e)| &pcm[s..e]).fold((0.0f64, 0usize), |(p, n), &x| (p + (x as f64).powi(2), n + 1));
    let speech_rms = (power / count.max(1) as f64).sqrt() as f32;
    for (i, snr) in NOISY_SNR_DB.into_iter().enumerate() {
        out.extend(cut(features_of(&with_noise(pcm, speech_rms / 10f32.powf(snr / 20.0), i as u32 + 1)), true));
    }
    out
}

/// Analysis frames for voicing: 30 ms every 15 ms.
const VOICE_FRAME: usize = 480;
const VOICE_HOP: usize = 240;
/// Pitch from 80 to 400 Hz, which covers adult and children's voices.
const MIN_LAG: usize = RATE as usize / 400;
const MAX_LAG: usize = RATE as usize / 80;
/// A frame is voiced when it repeats itself this closely at some pitch.
const VOICED_CORRELATION: f32 = 0.55;

/// Seconds of voiced sound (vowels and other sounds with a pitch) among the loud parts of `pcm`. Speech is
/// mostly voiced; keyboard taps, clicks, knocks and breaths have no pitch, so they score near zero.
pub fn voiced_seconds(pcm: &[i16]) -> f32 {
    if pcm.len() < VOICE_FRAME + MAX_LAG {
        return 0.0;
    }
    let x: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
    let energy = |f: &[f32]| f.iter().map(|v| v * v).sum::<f32>();
    let frames: Vec<usize> = (0..x.len() - VOICE_FRAME - MAX_LAG).step_by(VOICE_HOP).collect();
    // Only frames within 30 dB of the loudest count, so a quiet room's hum isn't taken as a voice.
    let loudest = frames.iter().map(|&i| energy(&x[i..i + VOICE_FRAME])).fold(0.0f32, f32::max);
    if loudest <= 1e-9 {
        return 0.0;
    }
    let voiced = frames
        .iter()
        .filter(|&&i| {
            let a = &x[i..i + VOICE_FRAME];
            let ea = energy(a);
            if ea < loudest * 1e-3 {
                return false;
            }
            (MIN_LAG..=MAX_LAG).any(|lag| {
                let b = &x[i + lag..i + lag + VOICE_FRAME];
                let r = a.iter().zip(b).map(|(p, q)| p * q).sum::<f32>() / (ea * energy(b)).sqrt().max(1e-12);
                r > VOICED_CORRELATION
            })
        })
        .count();
    (voiced * VOICE_HOP) as f32 / RATE as f32
}

/// Adds noise of the given loudness, half hiss and half rumble, like a room with a fan in it.
fn with_noise(pcm: &[i16], rms: f32, seed: u32) -> Vec<i16> {
    let mut x = seed.wrapping_mul(0x9e37_79b9) | 1;
    let mut low = 0.0f32;
    pcm.iter()
        .map(|&s| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // Uniform noise of unit power, then a one-pole low-pass for the rumble, rescaled to unit power.
            let white = (x as f32 / u32::MAX as f32 - 0.5) * 12f32.sqrt();
            low = 0.95 * low + 0.05 * white;
            let n = (white + low * 6.24) * std::f32::consts::FRAC_1_SQRT_2;
            (s as f32 + rms * n).clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Average cost under which live audio counts as the phrase.
    pub threshold: f32,
    /// How much worse the nearest thing that is not the phrase matches than the takes match each other.
    /// Under about 1.3 the phrase is easily confused.
    pub separation: f32,
}

/// Sets how close live audio must be to the takes. Each take must be accepted by the others, and the
/// takes of the person's other phrases and the takes played backwards (the same sounds in the wrong
/// order) must not be. None with fewer than two takes.
pub fn calibrate(takes: &[Template], others: &[Template]) -> Option<Calibration> {
    let takes: Vec<Template> = takes.iter().filter(|t| !t.noisy).cloned().collect();
    if takes.len() < 2 {
        return None;
    }
    let worst_take = (0..takes.len())
        .map(|i| (0..takes.len()).filter(|&j| j != i).map(|j| match_cost(&takes[j], &takes[i])).fold(f32::INFINITY, f32::min))
        .fold(0.0f32, f32::max);
    if !worst_take.is_finite() {
        return None;
    }
    let negatives: Vec<Template> = others.iter().filter(|t| !t.noisy).cloned().chain(takes.iter().map(Template::reversed)).collect();
    let nearest_other = takes
        .iter()
        .flat_map(|t| negatives.iter().map(move |n| match_cost(t, n)))
        .fold(f32::INFINITY, f32::min);
    let threshold = (worst_take + 0.5 * (nearest_other - worst_take)).min(worst_take * 1.2).max(worst_take * 1.02);
    let separation = if nearest_other.is_finite() { nearest_other / worst_take.max(1e-6) } else { 9.99 };
    Some(Calibration { threshold, separation })
}

/// A phrase to listen for.
#[derive(Debug, Clone)]
pub struct PhraseModel {
    pub id: String,
    /// Whose takes these are, such as a voice print id.
    pub owner: String,
    pub takes: Vec<Template>,
    pub threshold: f32,
    /// Only counts when this phrase was heard just before it, such as a command after the wake word.
    pub requires: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub phrase: String,
    pub owner: String,
    /// Samples since the first one pushed.
    pub start: u64,
    pub end: u64,
    /// When the spotter was sure, in samples; less `end`, this is the delay it added.
    pub decided: u64,
    /// Below 1; lower is a closer match.
    pub score: f32,
    /// The phrase's audio, for checking who said it.
    pub audio: Vec<i16>,
}

impl Hit {
    pub fn delay_ms(&self) -> f32 {
        self.decided.saturating_sub(self.end) as f32 * 1000.0 / RATE as f32
    }
}

struct Candidate {
    phrase: usize,
    ratio: f32,
    start: u64,
    end: u64,
}

/// Listens to one stream of audio for every phrase.
pub struct Spotter {
    features: Features,
    phrases: Vec<PhraseModel>,
    tracks: Vec<Vec<Track>>,
    frame: u64,
    candidate: Option<Candidate>,
    quiet_until: u64,
    /// Recent hits as (phrase, start frame), for phrases that need another first.
    heard: Vec<(String, u64)>,
    audio: VecDeque<i16>,
    /// The sample number of `audio[0]`.
    audio_start: u64,
    scratch: Vec<Frame>,
}

impl Spotter {
    pub fn new(phrases: Vec<PhraseModel>) -> Self {
        let phrases: Vec<PhraseModel> = phrases.into_iter().filter(|p| !p.takes.is_empty()).collect();
        let tracks = phrases.iter().map(|p| p.takes.iter().map(|t| Track::new(t.frames())).collect()).collect();
        Self {
            features: Features::new(),
            phrases,
            tracks,
            frame: 0,
            candidate: None,
            quiet_until: 0,
            heard: Vec::new(),
            audio: VecDeque::new(),
            audio_start: 0,
            scratch: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty()
    }

    pub fn push(&mut self, pcm: &[i16]) -> Vec<Hit> {
        self.audio.extend(pcm);
        let excess = self.audio.len().saturating_sub(KEEP_AUDIO);
        self.audio.drain(..excess);
        self.audio_start += excess as u64;
        let mut frames = std::mem::take(&mut self.scratch);
        frames.clear();
        self.features.push(pcm, &mut frames);
        let mut hits = Vec::new();
        for x in &frames {
            self.step(x, &mut hits);
        }
        self.scratch = frames;
        hits
    }

    fn step(&mut self, x: &Frame, hits: &mut Vec<Hit>) {
        let at = self.frame;
        self.frame += 1;
        for (p, phrase) in self.phrases.iter().enumerate() {
            for (track, take) in self.tracks[p].iter_mut().zip(&phrase.takes) {
                let Some((cost, start)) = track.step(take, x, at) else { continue };
                let ratio = cost / phrase.threshold;
                let better = self.candidate.as_ref().map_or(true, |c| ratio < c.ratio);
                if ratio < 1.0 && at >= self.quiet_until && better {
                    self.candidate = Some(Candidate { phrase: p, ratio, start, end: at });
                }
            }
        }
        if self.candidate.as_ref().is_some_and(|c| at - c.end >= SETTLE) {
            let c = self.candidate.take().unwrap();
            self.fire(c, at, hits);
        }
    }

    fn fire(&mut self, c: Candidate, at: u64, hits: &mut Vec<Hit>) {
        for track in self.tracks.iter_mut().flatten() {
            track.reset();
        }
        self.quiet_until = at + HOLDOFF;
        let window = (REQUIRE_WINDOW_S * RATE as f32 / HOP as f32) as u64;
        self.heard.retain(|(_, start)| c.start.saturating_sub(*start) <= window);
        let phrase = &self.phrases[c.phrase];
        if let Some(needed) = &phrase.requires {
            if !self.heard.iter().any(|(id, _)| id == needed) {
                return;
            }
        }
        self.heard.push((phrase.id.clone(), c.start));
        let (start, end) = (c.start * HOP as u64, c.end * HOP as u64 + FRAME as u64);
        let from = start.saturating_sub(self.audio_start) as usize;
        let to = (end.saturating_sub(self.audio_start) as usize).min(self.audio.len());
        hits.push(Hit {
            phrase: phrase.id.clone(),
            owner: phrase.owner.clone(),
            start,
            end,
            decided: (at + DELTA as u64) * HOP as u64 + FRAME as u64,
            score: c.ratio,
            audio: self.audio.range(from.min(to)..to).copied().collect(),
        });
    }
}

/// Every phrase heard in a whole recording.
pub fn scan(pcm: &[i16], phrases: Vec<PhraseModel>) -> Vec<Hit> {
    let mut spotter = Spotter::new(phrases);
    if spotter.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for chunk in pcm.chunks(RATE as usize) {
        hits.extend(spotter.push(chunk));
    }
    // Trailing silence lets a phrase at the very end settle.
    hits.extend(spotter.push(&vec![0; RATE as usize / 2]));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up word: vowel-like sounds, each a pitch with harmonics shaped by two formants, gliding
    /// from one to the next. `speed` above 1 says it faster.
    fn word(syllables: &[(f32, f32)], pitch: f32, speed: f32, seed: u32) -> Vec<i16> {
        let per = (0.16 * RATE as f32 / speed) as usize;
        let mut out = Vec::new();
        let mut phase = 0.0f32;
        let mut noise = seed.wrapping_mul(2654435761).max(1);
        for (n, pair) in syllables.windows(2).chain(std::iter::once(&[syllables[syllables.len() - 1]; 2][..])).enumerate() {
            for i in 0..per {
                let glide = (i as f32 / per as f32).powi(3);
                let f1 = pair[0].0 + (pair[1].0 - pair[0].0) * glide;
                let f2 = pair[0].1 + (pair[1].1 - pair[0].1) * glide;
                let t = (n * per + i) as f32 / RATE as f32;
                let f0 = pitch * (1.0 + 0.03 * (t * 4.0 + seed as f32).sin());
                phase += f0 / RATE as f32;
                let mut s = 0.0;
                for h in 1..30 {
                    let f = f0 * h as f32;
                    let gain = 1.0 / (1.0 + ((f - f1) / 120.0).powi(2)) + 0.7 / (1.0 + ((f - f2) / 180.0).powi(2));
                    s += gain * (2.0 * std::f32::consts::PI * phase * h as f32).sin();
                }
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let hiss = (noise % 2000) as f32 - 1000.0;
                let envelope = (std::f32::consts::PI * i as f32 / per as f32).sin().sqrt();
                out.push((s * 2500.0 * envelope + hiss * 0.3) as i16);
            }
        }
        out
    }

    fn silence(seconds: f32, seed: u32) -> Vec<i16> {
        let mut noise = seed.max(1);
        (0..(seconds * RATE as f32) as usize)
            .map(|_| {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                ((noise % 400) as i32 - 200) as i16
            })
            .collect()
    }

    const WAKE: &[(f32, f32)] = &[(700.0, 1200.0), (300.0, 2300.0), (500.0, 900.0), (300.0, 2300.0)];
    const BOOKMARK: &[(f32, f32)] = &[(400.0, 800.0), (650.0, 1100.0), (350.0, 1900.0)];
    const CHATTER: &[(f32, f32)] = &[(600.0, 1700.0), (450.0, 1000.0), (300.0, 2300.0), (700.0, 1200.0)];

    /// A recording of the phrase said several ways with pauses, as made when training it.
    fn training(syllables: &[(f32, f32)], seed: u32) -> Vec<i16> {
        let mut pcm = silence(1.0, seed);
        for (i, (speed, pitch)) in [(1.0, 120.0), (0.85, 115.0), (1.15, 125.0), (1.0, 130.0)].into_iter().enumerate() {
            pcm.extend(word(syllables, pitch, speed, seed + i as u32));
            pcm.extend(silence(0.8, seed + 10 + i as u32));
        }
        pcm
    }

    fn model(id: &str, syllables: &[(f32, f32)], others: &[Template], requires: Option<&str>) -> (PhraseModel, Calibration) {
        let takes = takes_from(&training(syllables, id.len() as u32));
        assert_eq!(takes.iter().filter(|t| !t.noisy).count(), 4, "{id}");
        assert_eq!(takes.len(), 4 * (1 + NOISY_SNR_DB.len()), "{id}");
        let calibration = calibrate(&takes, others).unwrap();
        let phrase = PhraseModel { id: id.into(), owner: "me".into(), takes, threshold: calibration.threshold, requires: requires.map(Into::into) };
        (phrase, calibration)
    }

    #[test]
    fn finds_each_take_in_a_training_recording() {
        let takes = find_takes(&training(WAKE, 3));
        assert_eq!(takes.len(), 4);
        for (s, e) in takes {
            let seconds = (e - s) as f32 / RATE as f32;
            assert!((0.4..1.0).contains(&seconds), "{seconds}");
        }
    }

    #[test]
    fn hears_the_phrase_in_running_audio_and_not_other_sounds() {
        let (bookmark, _) = model("bookmark", BOOKMARK, &[], None);
        let (wake, calibration) = model("wake", WAKE, &bookmark.takes, None);
        assert!(calibration.separation > 1.0, "{calibration:?}");
        let mut pcm = silence(2.0, 50);
        pcm.extend(word(CHATTER, 118.0, 1.0, 51));
        pcm.extend(silence(1.0, 52));
        let said_at = pcm.len() as u64;
        pcm.extend(word(WAKE, 122.0, 0.95, 53));
        let said_until = pcm.len() as u64;
        pcm.extend(silence(1.0, 54));
        let reversed: Vec<i16> = word(WAKE, 120.0, 1.0, 55).into_iter().rev().collect();
        pcm.extend(reversed);
        pcm.extend(silence(1.5, 56));

        let hits = scan(&pcm, vec![wake, bookmark]);
        assert_eq!(hits.len(), 1, "{:?}", hits.iter().map(|h| (&h.phrase, h.start, h.score)).collect::<Vec<_>>());
        let hit = &hits[0];
        assert_eq!(hit.phrase, "wake");
        let tolerance = RATE as u64 / 5;
        assert!(hit.start.abs_diff(said_at) < tolerance && hit.end.abs_diff(said_until) < tolerance, "{} {}", hit.start, hit.end);
        assert!(hit.delay_ms() < 200.0, "{}", hit.delay_ms());
        assert!(!hit.audio.is_empty());
    }

    #[test]
    fn a_command_needs_the_wake_word_first() {
        let (wake, _) = model("wake", WAKE, &[], None);
        let (bookmark, _) = model("bookmark", BOOKMARK, &wake.takes, Some("wake"));
        let mut alone = silence(1.5, 60);
        alone.extend(word(BOOKMARK, 120.0, 1.0, 61));
        alone.extend(silence(1.0, 62));
        assert!(scan(&alone, vec![wake.clone(), bookmark.clone()]).is_empty());

        let mut after = silence(1.5, 63);
        after.extend(word(WAKE, 120.0, 1.0, 64));
        after.extend(silence(0.5, 65));
        after.extend(word(BOOKMARK, 120.0, 1.0, 66));
        after.extend(silence(1.0, 67));
        let heard: Vec<String> = scan(&after, vec![wake, bookmark]).into_iter().map(|h| h.phrase).collect();
        assert_eq!(heard, ["wake", "bookmark"]);
    }

    #[test]
    fn streaming_in_small_pieces_matches_one_push() {
        let mut pcm = silence(0.5, 70);
        pcm.extend(word(WAKE, 120.0, 1.0, 71));
        let whole = features_of(&pcm);
        let mut features = Features::new();
        let mut pieces = Vec::new();
        for chunk in pcm.chunks(37) {
            features.push(chunk, &mut pieces);
        }
        features.finish(&mut pieces);
        assert_eq!(whole, pieces);
    }

    #[test]
    fn takes_round_trip_through_json_to_a_thousandth() {
        for n in [0, 1, 2, 3, 26, 27] {
            let t = Template { data: (0..n).map(|i| i as f32 * 0.3711 - 4.0).collect(), noisy: n % 2 == 0 };
            let back: Template = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
            assert_eq!(back.data.len(), t.data.len());
            assert!(back.data.iter().zip(&t.data).all(|(a, b)| (a - b).abs() <= 0.0005), "{n}");
            assert_eq!(back.noisy, t.noisy);
        }
    }

    #[test]
    fn taps_are_not_voiced_and_speech_is() {
        let speech = word(WAKE, 140.0, 1.0, 3);
        assert!(voiced_seconds(&speech) > 0.3, "{}", voiced_seconds(&speech));
        // Keyboard taps: a click every 120 ms, a decaying burst of noise each.
        let mut taps = vec![0i16; RATE as usize];
        let mut x = 12345u32;
        for start in (0..taps.len() - 800).step_by(1920) {
            for i in 0..800 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let n = (x as f32 / u32::MAX as f32 - 0.5) * 20000.0 * (-(i as f32) / 120.0).exp();
                taps[start + i] = n as i16;
            }
        }
        assert!(voiced_seconds(&taps) < 0.1, "{}", voiced_seconds(&taps));
        assert_eq!(voiced_seconds(&[0; 16000]), 0.0);
    }

    #[test]
    fn calibration_needs_two_takes() {
        let takes = takes_from(&training(WAKE, 80));
        assert!(calibrate(&takes[..1], &[]).is_none());
        let c = calibrate(&takes, &[]).unwrap();
        assert!(c.threshold > 0.0 && c.separation > 1.0, "{c:?}");
    }
}
