//! Translates lines with a local language model, to compare models and languages:
//!
//!   cargo run --release --example translate_try -- <model file in models/llm> <from> <to> "line one" "line two"
use std::time::Instant;
use stv_core::translate::{self, Line};
use superteem_voice_lib::{library, llm};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = library::models_dir().join("llm").join(&args[0]);
    let device = llm::device();
    let mut model = llm::Local::load(&path, device.context, device.threads, true, Default::default()).unwrap();
    let lines: Vec<Line> = args[3..].iter().enumerate().map(|(i, t)| Line { id: (i + 1).to_string(), text: t.clone() }).collect();
    let started = Instant::now();
    let done = translate::translate(&mut model, &lines, &[args[1].clone()], &args[2], &[], |_, _| {}).unwrap();
    for (l, d) in lines.iter().zip(&done) {
        println!("{} → {}{}", l.text, d.text, d.warning.as_ref().map(|w| format!("  [{w}]")).unwrap_or_default());
    }
    println!("{:.1} s", started.elapsed().as_secs_f32());
}
