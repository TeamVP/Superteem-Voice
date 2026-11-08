//! Key phrase spotting on WAVs: trains each phrase from <train-dir>/train_<phrase>.wav (the phrase said
//! several times with pauses), then lists what it hears in a test recording. "stop" needs "wake" first.
//! Make test audio with kws_tts.ps1. 16 kHz mono 16-bit WAVs.
//! cargo run --release --example kws_eval -- <train-dir> <test.wav> [sensitivity] [noise-db]
use stv_core::kws::{calibrate, scan, takes_from, PhraseModel, RATE};
fn read(path: &str) -> Vec<i16> {
    let d = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let at = d.windows(4).position(|w| w == b"data").unwrap() + 8;
    d[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}
/// Adds white noise at `db` below full scale, to see how the spotter copes with a noisy room.
fn noisy(pcm: &mut [i16], db: f32) {
    let amp = 32768.0 * 10f32.powf(db / 20.0) * 1.7;
    let mut x = 0x2545_f491u32;
    for s in pcm {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *s = (*s as f32 + amp * ((x as f32 / u32::MAX as f32) - 0.5)).clamp(-32768.0, 32767.0) as i16;
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sensitivity: f32 = args.get(2).map_or(1.0, |s| s.parse().unwrap());
    let noise: Option<f32> = args.get(3).map(|s| s.parse().unwrap());
    let ids = ["wake", "bookmark", "action", "stop"];
    let takes: Vec<_> = ids.iter().map(|id| takes_from(&read(&format!("{}/train_{id}.wav", args[0])))).collect();
    let mut phrases = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let others: Vec<_> = takes.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, t)| t.clone()).collect();
        let c = calibrate(&takes[i], &others).expect("at least two takes");
        let lengths: Vec<String> = takes[i].iter().map(|t| format!("{:.2}", t.seconds())).collect();
        println!("{id:9} {} takes [{}] threshold {:.3} separation {:.2}", takes[i].len(), lengths.join(" "), c.threshold, c.separation);
        let requires = (*id == "stop").then(|| "wake".to_string());
        phrases.push(PhraseModel { id: id.to_string(), owner: "me".into(), takes: takes[i].clone(), threshold: c.threshold * sensitivity, requires });
    }
    let mut pcm = read(&args[1]);
    if let Some(db) = noise {
        noisy(&mut pcm, db);
    }
    let started = std::time::Instant::now();
    let hits = scan(&pcm, phrases);
    let seconds = pcm.len() as f32 / RATE as f32;
    for h in &hits {
        println!("{:7.2}s-{:6.2}s  {:9} score {:.2}  delay {:.0} ms", h.start as f32 / RATE as f32, h.end as f32 / RATE as f32, h.phrase, h.score, h.delay_ms());
    }
    let took = started.elapsed().as_secs_f32();
    println!("{} hits in {seconds:.1} s of audio; ran at {:.0}x real time", hits.len(), seconds / took);
}
