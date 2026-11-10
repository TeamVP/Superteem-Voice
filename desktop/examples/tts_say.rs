//! Says text with the natural voice into a WAV, after downloading the voice if it isn't there, and reports
//! how long it took against how long the speech lasts.
//!
//!   cargo run --release --example tts_say -- "Some text to say." out.wav [voice id]
//!
//! With a voice id, that voice is downloaded if needed and chosen first.
use std::time::Instant;
use superteem_voice_lib::tts;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(id) = args.get(2) {
        superteem_voice_lib::prefs::update(|p| p.tts_voice = id.clone()).unwrap();
    }
    let voice = tts::chosen();
    if !tts::ready(voice) {
        println!("downloading {}…", voice.name);
        tts::download(voice.id, |_| {}).unwrap();
    }
    let started = Instant::now();
    let wav = tts::speak(&args[0], 1.0).unwrap();
    let took = started.elapsed().as_secs_f32();
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap()) as f32;
    let seconds = (wav.len() - 44) as f32 / 2.0 / rate;
    std::fs::write(&args[1], &wav).unwrap();
    println!("{seconds:.1} s of speech at {rate} Hz in {took:.2} s ({:.1}x real time)", seconds / took);
}
