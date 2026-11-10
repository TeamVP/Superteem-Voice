//! Phase 1 harness for analysis templates: runs the default template (summary, decisions, action items,
//! open questions, bookmarks written up)
//! over a saved meeting or note with a local GGUF model, and prints the result, what checking changed and
//! how long it took.
//!
//!   cargo run --release --example analyze -- <session folder | note .json> --model <file.gguf>
//!       [--ctx 8192] [--threads 4] [--json out.json] [--wav clip.wav [--wav-at seconds]]
//!
//! `--wav` also transcribes ten seconds of the clip with Whisper before and after, in the same process, to show both
//! libraries work side by side.
use std::path::{Path, PathBuf};
use std::time::Instant;
use stv_core::analysis::{self, Bookmark, Context, LineIn, Turn};
use superteem_voice_lib::{library, llm, notes, process, transcribe, vocabulary};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn whisper_check(wav: &Path, at_s: f64, when: &str) {
    let data = std::fs::read(wav).expect("read the WAV");
    let at = data.windows(4).position(|w| w == b"data").expect("a WAV file") + 8;
    let pcm: Vec<i16> = data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let from = ((at_s * 16_000.0) as usize).min(pcm.len());
    let clip = &pcm[from..pcm.len().min(from + 16_000 * 10)];
    let t = transcribe::Transcriber::load().expect("the Whisper model is installed");
    let started = Instant::now();
    let text = t.transcribe_plain(clip).expect("Whisper runs");
    println!("Whisper {when} ({:.1} s): {text}", started.elapsed().as_secs_f32());
}

struct Input {
    turns: Vec<Turn>,
    title: String,
    duration_s: f64,
    speakers: Vec<String>,
    is_note: bool,
    bookmarks: Vec<Bookmark>,
}

fn meeting(dir: &Path) -> Input {
    let meta = library::load_meta(dir);
    let segments = library::load_segments(dir);
    let names: Vec<(usize, String, f64, String)> = segments
        .iter()
        .filter(|s| s.in_script() && !process::is_silence(&s.text))
        .map(|s| (s.id, meta.name(&s.speaker), s.start_s, s.text.clone()))
        .collect();
    let lines: Vec<LineIn> =
        names.iter().map(|(id, speaker, start_s, text)| LineIn { id: *id, speaker, start_s: *start_s, text }).collect();
    let mut turns = analysis::meeting_turns(&lines);
    // Action items flagged aloud or with the button are marked on the turn being said at the time.
    for m in meta.markers.iter().filter(|m| !m.removed && (m.kind == "action" || m.text == "Action item")) {
        if let Some(t) = turns.iter_mut().rev().find(|t| t.at_s.is_some_and(|at| at <= m.at_s + 1.0)) {
            t.text += " [action item]";
        }
    }
    // Bookmarks, with anything typed or said there, are written up from what was being said.
    let bookmarks: Vec<Bookmark> = meta
        .markers
        .iter()
        .filter(|m| !m.removed && m.kind.is_empty() && m.text != "Action item")
        .enumerate()
        .map(|(i, m)| Bookmark { n: i + 1, at_s: m.at_s, note: m.text.clone() })
        .collect();
    let mut speakers: Vec<String> = Vec::new();
    for t in &turns {
        if !speakers.contains(&t.speaker) {
            speakers.push(t.speaker.clone());
        }
    }
    Input { turns, title: meta.title, duration_s: meta.duration_s, speakers, is_note: false, bookmarks }
}

fn note(path: &Path) -> Input {
    let note: notes::Note = library::read_json(path).expect("a note file");
    Input {
        turns: analysis::note_turns(&note.body),
        title: note.title,
        duration_s: 0.0,
        speakers: vec![analysis::ME.into()],
        is_note: true,
        bookmarks: Vec::new(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(target) = args.first().map(PathBuf::from) else {
        eprintln!("usage: analyze <session folder | note .json> --model <file.gguf> [--ctx 8192] [--threads 4] [--json out.json] [--wav clip.wav]");
        std::process::exit(2);
    };
    let model_path = PathBuf::from(arg(&args, "--model").expect("--model <file.gguf>"));
    let ctx_tokens: u32 = arg(&args, "--ctx").map_or(8192, |v| v.parse().expect("--ctx is a number"));
    let threads: i32 = arg(&args, "--threads").map_or_else(
        || std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).max(1) as i32),
        |v| v.parse().expect("--threads is a number"),
    );
    let wav = arg(&args, "--wav").map(PathBuf::from);
    let wav_at: f64 = arg(&args, "--wav-at").map_or(0.0, |v| v.parse().expect("--wav-at is seconds"));

    if let Some(w) = &wav {
        whisper_check(w, wav_at, "before");
    }

    let input = if target.is_dir() { meeting(&target) } else { note(&target) };
    if input.turns.is_empty() {
        eprintln!("Nothing to analyse in {}", target.display());
        std::process::exit(1);
    }
    let glossary = stv_core::vocab::glossary(&vocabulary::load(), 300);
    let ctx = Context {
        title: &input.title,
        date: "",
        duration_s: input.duration_s,
        speakers: &input.speakers,
        glossary: &glossary,
        is_note: input.is_note,
        bookmarks: &input.bookmarks,
    };

    let loading = Instant::now();
    let mut model = llm::Local::load(&model_path, ctx_tokens, threads).unwrap_or_else(|e| panic!("{e}"));
    let load_s = loading.elapsed().as_secs_f64();
    let started = Instant::now();
    let result = analysis::analyse(&mut model, &input.turns, &ctx, |k, n| eprintln!("Part {k} of {n}…"));
    let total_s = started.elapsed().as_secs_f64();

    match &result {
        Ok(a) => {
            println!("{}", analysis::to_markdown(a, &input.turns, &input.bookmarks, "Summary"));
            for w in &a.warnings {
                println!("! {w}");
            }
            for p in a.decisions.iter().chain(&a.open_questions).filter(|p| p.check) {
                println!("? check source: {}", p.text);
            }
            for i in a.action_items.iter().filter(|i| i.check) {
                println!("? check source: {}", i.what);
            }
            for b in a.bookmarks.iter().filter(|b| b.check) {
                println!("? check source: bookmark {}", b.bookmark);
            }
            if let Some(out) = arg(&args, "--json") {
                let json = serde_json::json!({ "analysis": a, "turns": input.turns, "bookmarks": input.bookmarks });
                std::fs::write(&out, serde_json::to_string_pretty(&json).unwrap()).expect("write --json");
            }
        }
        Err(e) => println!("Analysis failed: {e}"),
    }

    let model_name = model_path.file_name().unwrap_or_default().to_string_lossy();
    println!("\n--- {model_name}, {} turns, context {}, {threads} threads", input.turns.len(), ctx_tokens);
    println!("load {load_s:.1} s, analysis {total_s:.1} s");
    for (i, s) in model.stats.iter().enumerate() {
        println!(
            "call {}: prompt {} tok in {:.1} s ({:.0} tok/s), output {} tok in {:.1} s ({:.1} tok/s)",
            i + 1,
            s.prompt_tokens,
            s.prompt_s,
            s.prompt_tokens as f64 / s.prompt_s.max(1e-6),
            s.output_tokens,
            s.output_s,
            s.output_tokens as f64 / s.output_s.max(1e-6),
        );
    }
    drop(model);

    if let Some(w) = &wav {
        whisper_check(w, wav_at, "after");
    }
    if result.is_err() {
        std::process::exit(1);
    }
}
