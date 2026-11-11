//! Replays a 16 kHz mono WAV as if it were being dictated, through the same pause finder and transcriber as
//! continuous dictation, and prints each piece as it would be written in, and where it would stop by itself.
//!
//!   cargo run --release --example dictation_replay -- <file.wav> [--from seconds]
use stv_core::attribute::window_db;
use stv_core::chunk;
use superteem_voice_lib::{listen, process};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = std::fs::read(&args[0]).expect("read the WAV");
    let at = data.windows(4).position(|w| w == b"data").expect("a WAV file") + 8;
    let mut pcm: Vec<i16> = data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let from = args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)).map_or(0.0, |v| v.parse::<f32>().unwrap());
    pcm.drain(..((from * 16_000.0) as usize).min(pcm.len()));
    // Six seconds of room tone after it, to see it stop by itself.
    let tail: Vec<i16> = pcm.iter().take(16_000).map(|s| s / 40).cycle().take(6 * 16_000).collect();
    pcm.extend(tail);

    let step = 16_000 * 3 / 10;
    let (mut heard, mut pending, mut noise, mut last_empty, mut taken) = (false, Vec::new(), None::<f32>, false, 0usize);
    for (tick, block) in pcm.chunks(step).enumerate() {
        let now = (tick + 1) as f32 * 0.3;
        pending.extend_from_slice(block);
        if pending.len() >= chunk::FRAME * 25 {
            let est = chunk::noise_level(&window_db(&pending, chunk::FRAME));
            noise = Some(noise.map_or(est, |n: f32| if est < n { est } else { n + (est - n) * 0.05 }));
        }
        while let Some(cut) = chunk::find_cut(&pending, noise) {
            let piece: Vec<i16> = pending.drain(..cut.at).collect();
            let start = taken as f32 / 16_000.0;
            taken += cut.at;
            let text = if cut.speech && piece.len() > 4000 { listen::transcribe_all(&piece, &[]).unwrap_or_default() } else { String::new() };
            let text = if process::is_silence(&text) { String::new() } else { text };
            println!("{now:6.1}s  piece {start:5.1}-{:5.1}s  {}", taken as f32 / 16_000.0, if text.is_empty() { "(nothing)".into() } else { text.clone() });
            last_empty = text.is_empty();
            heard |= !text.is_empty();
        }
        let quiet = chunk::trailing_silence_s(&pending, noise);
        if last_empty && heard && quiet >= 2.5 {
            println!("{now:6.1}s  stops by itself ({quiet:.1}s of quiet after an empty piece)");
            return;
        }
    }
    println!("ran out of audio without stopping");
}
