#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use superteem_voice_lib::{capture, keyphrases, library, listen, process, recorder, voices};

/// Records the default input for a set time without a window, to check capture on a real machine.
fn self_test(seconds: u64) -> Result<(), String> {
    let device = capture::default_input().ok_or("no input device")?;
    let dir = library::new_session_dir().map_err(|e| e.to_string())?;
    let r = recorder::start(dir.clone(), &[device], false)?;
    thread::sleep(Duration::from_secs(seconds));
    for s in r.status()?.streams {
        println!("{}: {} frames={} lost={} {}", s.name, s.state, s.frames, s.lost, s.detail);
    }
    r.stop()?;
    println!("{}\n{}", process::process_session(&dir, &|_, _| {})?, dir.display());
    Ok(())
}

fn read_wav(path: &str) -> Result<Vec<i16>, String> {
    let d = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let at = d.windows(4).position(|w| w == b"data").ok_or("not a WAV")? + 8;
    Ok(d[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// Trains every key phrase from `<train>/train_<phrase>.wav` for a new voice print in a scratch data
/// folder, then plays a 16 kHz WAV through the listener as if spoken to it with the wake word on, and prints
/// what it hears and when, with the words of each command and how long whisper took.
fn kws_check(data: &str, train: &str, test: &str) -> Result<(), String> {
    library::init(std::path::Path::new(data).join("sessions"), data.into());
    let mut all = Vec::new();
    for spec in keyphrases::PHRASES {
        all.extend(read_wav(&format!("{train}/train_{}.wav", spec.id))?);
    }
    let voice = stv_core::embed::voice(&all, process::RATE).ok_or("no speech in the training audio")?;
    let print = voices::add_sample(None, "Check", voice, "script")?;
    for spec in keyphrases::PHRASES {
        let result = keyphrases::train(&print, spec.id, &read_wav(&format!("{train}/train_{}.wav", spec.id))?, true)?;
        println!("{:16} {:?}", spec.text, result);
    }
    let mut hearing = listen::Hearing::new();
    let mut events = Vec::new();
    for (i, frame) in read_wav(test)?.chunks_exact(process::FRAME_SAMPLES).enumerate() {
        hearing.push(frame, false, &mut events);
        let at = (i + 1) as f32 * process::FRAME_SAMPLES as f32 / process::RATE as f32;
        for event in events.drain(..) {
            match event {
                listen::Event::Phrase(h) => println!("{at:7.2}s  heard {:16} score {:.2} ({:.0} ms after it ended)", h.text, h.score, h.delay_ms),
                listen::Event::Speech(pcm) => {
                    let started = std::time::Instant::now();
                    let words = listen::transcribe_phrase(&pcm)?;
                    println!("{at:7.2}s  command {:?} ({:.1} s of audio, whisper {:.2} s)", words, pcm.len() as f32 / process::RATE as f32, started.elapsed().as_secs_f32());
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--self-test") {
        return self_test(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5));
    }
    if args.get(1).is_some_and(|a| a == "--kws-check") && args.len() == 5 {
        return kws_check(&args[2], &args[3], &args[4]);
    }
    superteem_voice_lib::run();
    Ok(())
}
