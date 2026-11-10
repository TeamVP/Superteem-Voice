//! Says text with a natural voice into a WAV, downloading the voice first if it isn't there, and reports how
//! long it took against how long the speech lasts. Settings aren't changed.
//!
//!   cargo run --release --example tts_say -- "Some text to say." out.wav [voice id]
//!
//! Without a voice id, the one chosen in Settings says it. Voice ids are the model's name, then "#speaker"
//! for a model's other speakers, such as kokoro-multi-lang-v1_1#58.
use std::time::Instant;
use superteem_voice_lib::tts;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let voice = match args.get(2) {
        Some(id) => tts::find_voice(id).unwrap(),
        None => tts::chosen(),
    };
    if !tts::ready(&voice) {
        println!("downloading {}…", voice.name);
        tts::download(&voice.id, |_| {}).unwrap();
    }
    let started = Instant::now();
    let wav = tts::speak_as(&voice, &args[0], 1.0).unwrap();
    let took = started.elapsed().as_secs_f32();
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap()) as f32;
    let seconds = (wav.len() - 44) as f32 / 2.0 / rate;
    std::fs::write(&args[1], &wav).unwrap();
    println!("{}: {seconds:.1} s of speech at {rate} Hz in {took:.2} s ({:.1}x real time)", voice.name, seconds / took);
}
