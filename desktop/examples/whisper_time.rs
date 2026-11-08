//! How long whisper takes on a short command, as the listener runs it:
//! cargo run --release --example whisper_time -- <16 kHz mono WAV> [seconds]
use superteem_voice_lib::transcribe::Transcriber;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let d = std::fs::read(&args[0]).unwrap();
    let at = d.windows(4).position(|w| w == b"data").unwrap() + 8;
    let pcm: Vec<i16> = d[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let seconds: f32 = args.get(1).map_or(2.5, |s| s.parse().unwrap());
    let clip = &pcm[..((seconds * 16_000.0) as usize).min(pcm.len())];
    let t = Transcriber::load().unwrap();
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let text = t.transcribe_with_prompt(clip, "Hey Superteem.").unwrap();
        println!("{:.2} s for {seconds} s of audio: {text}", started.elapsed().as_secs_f32());
    }
}
