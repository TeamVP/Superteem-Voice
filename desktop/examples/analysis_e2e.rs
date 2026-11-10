//! End-to-end check of analyses through the app's own storage: copies a meeting into a scratch library,
//! runs a template with a local model, edits the result, makes it again, and prints what the page gets and
//! what the documents say.
//!
//!   cargo run --release --example analysis_e2e -- <session folder> --model <file.gguf> --out <dir> [--template id]
use std::path::PathBuf;
use superteem_voice_lib::analysis::{self, Edit, Engine, Item};
use superteem_voice_lib::{library, llm};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = PathBuf::from(&args[0]);
    let model_path = PathBuf::from(arg(&args, "--model").expect("--model"));
    let out = PathBuf::from(arg(&args, "--out").expect("--out"));
    let template = arg(&args, "--template").unwrap_or_else(|| "meeting-summary".into());
    let root = out.join("library");
    let _ = std::fs::remove_dir_all(&root);
    let session = root.join("session-1794000000");
    std::fs::create_dir_all(&session).unwrap();
    for f in ["transcript.json", "meta.json"] {
        std::fs::copy(source.join(f), session.join(f)).unwrap();
    }
    library::init(root.clone(), out.join("data"));
    let item = Item::new("meeting", "session-1794000000").unwrap();

    let device = llm::device();
    let mut model = llm::Local::load(&model_path, device.context, device.threads, Default::default()).unwrap();
    let engine = Engine { kind: "local".into(), model: model.name.clone(), host: None };
    let entry = analysis::make(&item, &template, &mut model, engine.clone(), |k, n| eprintln!("part {k}/{n}")).unwrap();
    analysis::put(&item, entry).unwrap();

    // The person ticks the first task, reassigns it and adds their own; then it's made again.
    analysis::edit(&item, &template, Edit::Done { field: "action_items".into(), index: 0, done: true }).unwrap();
    analysis::edit(&item, &template, Edit::Add { field: "action_items".into(), item: serde_json::json!({"who": "Me", "what": "Book the room", "when": null, "evidence": []}) }).unwrap();
    let again = analysis::make(&item, &template, &mut model, engine, |_, _| {}).unwrap();
    analysis::put(&item, again).unwrap();

    let view = analysis::view(&item);
    std::fs::write(out.join("view.json"), serde_json::to_string_pretty(&serde_json::json!({
        "entries": view,
        "templates": analysis::templates().into_iter().filter(|t| t.applies_to.contains(&"meeting".to_string())).map(|t| analysis::TemplateInfo { template: None, ..t }).collect::<Vec<_>>(),
        "queue": {"current": null, "waiting": []},
        "ready": true,
    })).unwrap()).unwrap();
    println!("{}", std::fs::read_to_string(session.join("transcript.md")).unwrap());
    let items = &view[0].entry.result["action_items"];
    println!("--- after making again: first task done = {}, own task kept = {}", items[0]["_done"], items.as_array().unwrap().iter().any(|i| i["what"] == "Book the room"));
    println!("--- sources resolved: {}/{}", view[0].sources.values().filter(|s| s.is_some()).count(), view[0].sources.len());
}
