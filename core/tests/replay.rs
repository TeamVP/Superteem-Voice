//! End-to-end replay: a simulated meeting goes through journals, alignment, attribution and export.

use stv_core::attribute::Config;
use stv_core::export::{self, ExportTrack};
use stv_core::journal::JournalWriter;
use stv_core::pipeline;
use stv_core::protocol::{Codec, Frame};

const RATE: u32 = 16_000;
const FRAME: usize = 320;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// Alice speaks 5-15 s and 35-45 s, Bob speaks 20-30 s, and each bleeds into the other's pin at -14 dB.
fn pin_audio(own: &[(f32, f32)], other: &[(f32, f32)], seconds: usize, seed: u64) -> Vec<i16> {
    let mut rng = Rng(seed);
    (0..seconds * RATE as usize)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let speaks = |turns: &[(f32, f32)]| turns.iter().any(|&(a, b)| t >= a && t < b);
            let voice = (t * 2.0 * std::f32::consts::PI * 220.0).sin() * 8000.0;
            let mut sample = (rng.next() % 40) as f32 - 20.0;
            if speaks(own) {
                sample += voice;
            }
            if speaks(other) {
                sample += voice * 0.2;
            }
            sample as i16
        })
        .collect()
}

struct RecordOptions {
    start_us: u64,
    drift_ppm: f64,
    lost: std::ops::Range<u32>,
}

fn record(path: &std::path::Path, pcm: &[i16], options: RecordOptions, seed: u64) {
    let mut rng = Rng(seed);
    let mut journal = JournalWriter::create(path).unwrap();
    for (i, chunk) in pcm.chunks_exact(FRAME).enumerate() {
        let sequence = i as u32;
        if options.lost.contains(&sequence) {
            continue;
        }
        let sample = (i * FRAME) as u64;
        let ideal = sample as f64 * 1e6 / RATE as f64 * (1.0 + options.drift_ppm / 1e6);
        let arrival = options.start_us + ideal as u64 + rng.next() % 30_000;
        let frame = Frame {
            flags: 0,
            pin_session_id: 1,
            sequence,
            sample_counter: sample as u32,
            codec: Codec::RawPcm16,
            payload: chunk.iter().flat_map(|s| s.to_le_bytes()).collect(),
        };
        journal.append(arrival, &frame.encode()).unwrap();
    }
    journal.sync().unwrap();
}

fn dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("stv-replay-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn simulated_meeting_is_attributed_and_exported() {
    let dir = dir("meeting");
    let alice_turns = [(5.0, 15.0), (35.0, 45.0)];
    let bob_turns = [(20.0, 30.0)];
    let (a, b) = (dir.join("a.stvj"), dir.join("b.stvj"));
    record(
        &a,
        &pin_audio(&alice_turns, &bob_turns, 60, 1),
        RecordOptions { start_us: 1_000_000, drift_ppm: 50.0, lost: 500..550 },
        2,
    );
    record(
        &b,
        // This pin starts 0.3 s later, so the same moments fall 0.3 s earlier in its audio.
        &pin_audio(&[(19.7, 29.7)], &[(4.7, 14.7), (34.7, 44.7)], 60, 3),
        RecordOptions { start_us: 1_300_000, drift_ppm: -50.0, lost: 0..0 },
        4,
    );

    let mut sources = vec![pipeline::load_source(&a, RATE).unwrap(), pipeline::load_source(&b, RATE).unwrap()];
    assert_eq!(sources[0].lost_frames, 50);
    assert_eq!(sources[1].lost_frames, 0);

    let offset = sources[1].clock.unwrap().offset_us - sources[0].clock.unwrap().offset_us;
    assert!((offset - 300_000.0).abs() < 20_000.0, "offset {offset}");

    pipeline::align_sources(&mut sources, RATE);
    let result = pipeline::attribute_sources(&sources, RATE, &Config::default());

    let who: Vec<_> = result.segments.iter().map(|s| s.speaker_source).collect();
    // Frames 500-550 (10-11 s) are lost from Alice's pin. That speech falls back to Bob's pin as an
    // unknown speaker, keeping the words without crediting them to Bob.
    assert_eq!(who, vec![Some(0), None, Some(0), Some(1), Some(0)], "{:?}", result.segments);
    assert_eq!(result.segments[1].audio_source, 1);
    let to_s = |w: usize| (w * pipeline::WINDOW_MS) as f64 / 1000.0;
    assert!((to_s(result.segments[0].start_window) - 5.0).abs() < 0.2);
    assert!((to_s(result.segments[3].start_window) - 20.0).abs() < 0.2);

    let tracks: Vec<ExportTrack> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| ExportTrack {
            name: format!("Pin {}", i + 1),
            file: format!("pin{i}.wav"),
            duration_s: s.pcm.len() as f64 / RATE as f64,
            gains: result
                .gains
                .iter()
                .filter(|g| g.source == i)
                .map(|g| (to_s(g.start_window), to_s(g.end_window), g.gain_db))
                .collect(),
        })
        .collect();
    export::write_wav(&dir.join("pin0.wav"), &sources[0].pcm, RATE).unwrap();
    assert!(export::to_reaper(&tracks, RATE).contains("VOLENV2"));
    let otio: serde_json::Value = serde_json::from_str(&export::to_otio("meeting", &tracks, RATE)).unwrap();
    assert_eq!(otio["tracks"]["children"].as_array().unwrap().len(), 2);
}

/// Reads a journal written by the Android code, when its unit tests have produced one.
#[test]
fn reads_journal_written_by_android() {
    let path = std::path::Path::new("../android/app/build/journal-sample.stvj");
    if !path.exists() {
        eprintln!("skipped: run the Android unit tests first");
        return;
    }
    let source = pipeline::load_source(path, RATE).unwrap();
    assert_eq!(source.frames, 5);
    assert_eq!(source.lost_frames, 0);
    assert!(!source.had_bad_tail);
    assert_eq!(source.pcm.len(), 5 * FRAME);
}

#[test]
fn adpcm_journals_decode_and_attribute() {
    let dir = dir("adpcm");
    let mut sources = Vec::new();
    for (i, (own, other)) in [([(5.0, 15.0)], [(20.0, 30.0)]), ([(20.0, 30.0)], [(5.0, 15.0)])].iter().enumerate() {
        let path = dir.join(format!("{i}.stvj"));
        let pcm = pin_audio(own, other, 40, i as u64 + 1);
        let mut journal = JournalWriter::create(&path).unwrap();
        let mut index = 0;
        for (n, chunk) in pcm.chunks_exact(FRAME).enumerate() {
            let frame = Frame {
                flags: 0,
                pin_session_id: 1,
                sequence: n as u32,
                sample_counter: (n * FRAME) as u32,
                codec: Codec::ImaAdpcm,
                payload: stv_core::adpcm::encode(chunk, &mut index),
            };
            journal.append(n as u64 * 20_000, &frame.encode()).unwrap();
        }
        journal.sync().unwrap();
        sources.push(pipeline::load_source(&path, RATE).unwrap());
    }
    assert_eq!(sources[0].pcm.len(), 40 * RATE as usize);
    let result = pipeline::attribute_sources(&sources, RATE, &Config::default());
    let who: Vec<_> = result.segments.iter().map(|s| s.speaker_source).collect();
    assert_eq!(who, vec![Some(0), Some(1)], "{:?}", result.segments);
}

#[test]
fn a_crash_mid_frame_loses_only_the_torn_frame() {
    let dir = dir("crash");
    let path = dir.join("a.stvj");
    let pcm = pin_audio(&[(0.0, 5.0)], &[], 5, 1);
    record(&path, &pcm, RecordOptions { start_us: 0, drift_ppm: 0.0, lost: 0..0 }, 2);
    let size = std::fs::metadata(&path).unwrap().len();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(size - 100).unwrap();
    drop(file);

    let source = pipeline::load_source(&path, RATE).unwrap();
    assert!(source.had_bad_tail);
    assert_eq!(source.frames, 249);
    assert_eq!(source.pcm.len(), 249 * FRAME);
}
