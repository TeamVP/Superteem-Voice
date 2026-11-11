//! Runs an analysis template over a saved meeting or note with a local GGUF model, through the same code
//! the app uses, and prints the result, what checking changed and how long it took. For comparing models
//! and testing templates.
//!
//!   cargo run --release --example analyze -- <session folder | note .json> --model <file.gguf>
//!       [--template meeting-summary | --template-file my.toml] [--ctx 8192] [--threads 4] [--json out.json]
use std::path::{Path, PathBuf};
use std::time::Instant;
use stv_core::analysis::{self as engine, Context, Template};
use superteem_voice_lib::analysis::{meeting_input, note_input, Input};
use superteem_voice_lib::{library, llm, notes, vocabulary};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn load(target: &Path) -> Input {
    if target.is_dir() {
        meeting_input(&library::load_meta(target), &library::load_segments(target))
    } else {
        let note: notes::Note = library::read_json(target).expect("a note file");
        note_input(&note)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(target) = args.first().map(PathBuf::from) else {
        eprintln!("usage: analyze <session folder | note .json> --model <file.gguf> [--template id | --template-file file.toml] [--ctx 8192] [--threads 4] [--json out.json]");
        std::process::exit(2);
    };
    let model_path = PathBuf::from(arg(&args, "--model").expect("--model <file.gguf>"));
    let device = llm::device();
    let ctx_tokens: u32 = arg(&args, "--ctx").map_or(device.context, |v| v.parse().expect("--ctx is a number"));
    let threads: i32 = arg(&args, "--threads").map_or(device.threads, |v| v.parse().expect("--threads is a number"));
    let template: Template = match arg(&args, "--template-file") {
        Some(file) => engine::parse(&std::fs::read_to_string(file).expect("read the template")).unwrap_or_else(|e| panic!("{}", e.join("\n"))),
        None => {
            let id = arg(&args, "--template").unwrap_or_else(|| engine::DEFAULT_TEMPLATE.into());
            engine::built_in().into_iter().find(|t| t.id == id).unwrap_or_else(|| panic!("no built-in template {id}"))
        }
    };

    let input = load(&target);
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
        language: "",
    };

    let loading = Instant::now();
    let mut model = llm::Local::load(&model_path, ctx_tokens, threads, true, Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let load_s = loading.elapsed().as_secs_f64();
    let started = Instant::now();
    let result = engine::analyse(&mut model, &template, &input.turns, &ctx, |k, n| eprintln!("Part {k} of {n}…"));
    let total_s = started.elapsed().as_secs_f64();

    match &result {
        Ok(a) => {
            let turns = &input.turns;
            let label = |v: &serde_json::Value| {
                v.as_u64().and_then(|n| turns.iter().find(|t| t.n == n as usize)).map(|t| t.at_s.map_or(format!("¶{}", t.n), engine::clock))
            };
            println!("{}", engine::render(&template, &a.result, &label, true));
            for w in &a.warnings {
                println!("! {w}");
            }
            let flagged = serde_json::to_string(&a.result).unwrap().matches("\"_check\":true").count();
            if flagged > 0 {
                println!("? {flagged} item(s) flagged to check against their source");
            }
            if let Some(out) = arg(&args, "--json") {
                let json = serde_json::json!({ "result": a.result, "warnings": a.warnings, "turns": input.turns, "bookmarks": input.bookmarks });
                std::fs::write(&out, serde_json::to_string_pretty(&json).unwrap()).expect("write --json");
            }
        }
        Err(e) => println!("Analysis failed: {e}"),
    }

    let model_name = model_path.file_name().unwrap_or_default().to_string_lossy();
    println!("\n--- {model_name}, template {}, {} turns, context {ctx_tokens}, {threads} threads", template.id, input.turns.len());
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
    if result.is_err() {
        std::process::exit(1);
    }
}
