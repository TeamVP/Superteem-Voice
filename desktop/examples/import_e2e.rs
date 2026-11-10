//! End-to-end check of importing a track: copies a meeting into a scratch library, imports part of its own
//! audio as if a phone had started recording `--at` seconds in, by a clock 3 s out, places it by that time of
//! day and fine-tunes it by its sound, processes the meeting and reports
//! where the track was placed and what was transcribed from it.
//!
//!   cargo run --release --example import_e2e -- <session folder> --out <dir> [--at 12.3]
use std::path::PathBuf;
use superteem_voice_lib::{imports, library, process};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = PathBuf::from(&args[0]);
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let out = PathBuf::from(arg("--out").expect("--out"));
    let at: f64 = arg("--at").map_or(12.3, |v| v.parse().unwrap());
    let root = out.join("library");
    let _ = std::fs::remove_dir_all(&root);
    let session = root.join("session-1794000001");
    std::fs::create_dir_all(&session).unwrap();
    for entry in std::fs::read_dir(&source).unwrap().flatten() {
        std::fs::copy(entry.path(), session.join(entry.file_name())).unwrap();
    }
    library::init(root, out.join("data"));

    let wav = std::fs::read_dir(&session).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "wav")).unwrap();
    let data = std::fs::read(&wav).unwrap();
    let start = data.windows(4).position(|w| w == b"data").unwrap() + 8;
    let pcm: Vec<i16> = data[start..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    // The "phone": from `at` seconds on, quieter, with its own hiss.
    let from = (at * 16_000.0) as usize;
    let phone: Vec<i16> = pcm[from..].iter().enumerate().map(|(i, &x)| x / 2 + ((i * 7919) % 120) as i16 - 60).collect();

    let started_at = library::load_meta(&session).created as f64 + at - 3.0;
    let import = imports::add(&session, "Phone", &phone, imports::Place::Time { started_at, refine: true }).unwrap();
    println!("placed by {}: starts at {:.3} s (true {at}), rate {:.6}, confidence {:.2}, matched {:?}",
        import.placed_by, import.offset_s, import.rate, import.confidence.unwrap_or(0.0), import.matched);
    let summary = process::process_session(&session, &|_, _| {}).unwrap();
    println!("processed: {summary}");
    let segments = library::load_segments(&session);
    let on_phone = segments.iter().filter(|s| s.stream == "import-1" || s.also_on.iter().any(|a| a == "import-1")).count();
    println!("{} lines; {on_phone} heard on the imported track; tracks: {:?}", segments.len(),
        std::fs::read_dir(&session).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".wav")).collect::<Vec<_>>());
}
