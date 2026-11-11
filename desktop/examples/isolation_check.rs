//! Checks the analysis sandbox: from here, then from inside a worker, whether the network can be reached
//! and a name looked up; then runs a template through a model in the worker.
//!
//!   cargo run --release --example isolation_check -- <session folder> <model.gguf>
#[cfg(any(windows, target_os = "macos"))]
fn main() {
    use superteem_voice_lib::{analysis, isolate, library};
    let args: Vec<String> = std::env::args().collect();
    // This program is also the worker it starts.
    if args.get(1).is_some_and(|a| a == isolate::ARG) {
        return isolate::serve();
    }
    std::env::set_var("STV_PROBE_DNS", "example.com");
    println!("here:         {:?}", isolate::probe());
    println!("in a worker:  {:?}", isolate::check());
    let session = std::path::PathBuf::from(&args[1]);
    library::init(session.parent().unwrap().to_path_buf(), session.parent().unwrap().join("data"));
    let id = session.file_name().unwrap().to_string_lossy().into_owned();
    let started = std::time::Instant::now();
    let mut model = isolate::Isolated::load(std::path::Path::new(&args[2]), 4096, 4, false, Default::default()).unwrap();
    println!("loaded in a worker in {:.1} s", started.elapsed().as_secs_f32());
    let item = analysis::Item::new("meeting", &id).unwrap();
    let engine = analysis::Engine { kind: "local".into(), model: model.name.clone(), host: None };
    let entry = analysis::make(&item, "meeting-summary", &mut model, engine, |_, _, _| {}).unwrap().remove(0);
    println!("analysed in {:.1} s: {}", entry.seconds, entry.result["summary"]);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {}
