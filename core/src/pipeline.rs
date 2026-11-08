//! Turns recorded journals into aligned audio and attribution. Handles raw PCM and IMA ADPCM frames.

use crate::adpcm;
use crate::align::{self, ClockModel};
use crate::attribute::{self, Attribution, Config};
use crate::journal;
use crate::protocol::{Codec, Frame};
use std::io;
use std::path::Path;

pub const WINDOW_MS: usize = 50;

#[derive(Debug)]
pub struct SourceTrack {
    /// Aligned to the session timeline, with lost audio filled by silence.
    pub pcm: Vec<i16>,
    pub clock: Option<ClockModel>,
    pub frames: usize,
    pub lost_frames: u64,
    pub duplicate_frames: usize,
    pub had_bad_tail: bool,
}

pub fn load_source(path: &Path, rate: u32) -> io::Result<SourceTrack> {
    let recovered = journal::read(path)?;
    let mut frames: Vec<(u64, Frame)> = Vec::with_capacity(recovered.records.len());
    let mut duplicate_frames = 0;
    let mut newest = None;
    for record in recovered.records {
        let Ok(frame) = Frame::decode(&record.frame) else { continue };
        if frame.codec == Codec::Opus {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "Opus journals are not supported yet"));
        }
        if newest.is_some_and(|seq| frame.sequence <= seq) {
            duplicate_frames += 1;
            continue;
        }
        newest = Some(frame.sequence);
        frames.push((record.arrival_us, frame));
    }
    let lost_frames = match (frames.first(), frames.last()) {
        (Some(first), Some(last)) => (last.1.sequence - first.1.sequence + 1) as u64 - frames.len() as u64,
        _ => 0,
    };
    let observations: Vec<(u64, u64)> = frames.iter().map(|(a, f)| (f.sample_counter as u64, *a)).collect();
    let clock = align::estimate_clock(&observations, rate);

    let mut pcm: Vec<i16> = Vec::new();
    for (_, frame) in &frames {
        let start = frame.sample_counter as usize;
        let samples = match frame.codec {
            Codec::ImaAdpcm => adpcm::decode(&frame.payload).unwrap_or_default(),
            _ => frame.payload.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect(),
        };
        if pcm.len() < start {
            pcm.resize(start, 0);
        }
        pcm.truncate(start);
        pcm.extend(samples);
    }
    Ok(SourceTrack {
        pcm,
        clock,
        frames: frames.len(),
        lost_frames,
        duplicate_frames,
        had_bad_tail: recovered.had_bad_tail,
    })
}

/// Places every source on one timeline, starting at the earliest source.
pub fn align_sources(sources: &mut [SourceTrack], rate: u32) {
    let earliest = sources.iter().filter_map(|s| s.clock.map(|c| c.offset_us)).fold(f64::INFINITY, f64::min);
    for source in sources.iter_mut() {
        let Some(clock) = source.clock else { continue };
        let corrected = align::correct_drift(&source.pcm, clock.drift_ppm);
        let lead = ((clock.offset_us - earliest) * rate as f64 / 1e6).round() as usize;
        let mut pcm = vec![0i16; lead];
        pcm.extend(corrected);
        source.pcm = pcm;
    }
}

pub fn attribute_sources(sources: &[SourceTrack], rate: u32, cfg: &Config) -> Attribution {
    let window = rate as usize * WINDOW_MS / 1000;
    let levels: Vec<Vec<f32>> = sources.iter().map(|s| attribute::window_db(&s.pcm, window)).collect();
    attribute::attribute(&levels, cfg)
}
