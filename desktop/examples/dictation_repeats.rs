//! Transcribes dictations as the app does and flags any that came out longer than what was said, which is
//! whisper repeating itself: cargo run --release --example dictation_repeats -- <16 kHz mono WAV>...
use superteem_voice_lib::transcribe::Transcriber;
fn main() {
    let t = Transcriber::for_languages(&["en".to_string()]).unwrap();
    for path in std::env::args().skip(1) {
        let d = std::fs::read(&path).unwrap();
        let at = d.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<i16> = d[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let started = std::time::Instant::now();
        let text = t.transcribe_with_prompt(&pcm, "Hey Superteem.", "en").unwrap();
        println!("{path}: {:.1} s of audio in {:.2} s, {} words\n  {text}", pcm.len() as f32 / 16_000.0, started.elapsed().as_secs_f32(), text.split_whitespace().count());
    }
}
