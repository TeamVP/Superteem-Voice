//! Optimising Voice for this device: the speech model, the analysis model, the graphics processor and a
//! reading voice for each of the person's languages, chosen for what this device can run well and the room
//! it has, then downloaded and tried.
//!
//! The choice is a plan made from a measured profile of the device and what's installed, by a function
//! with no side effects, so it can be checked against any device. Applying it downloads what it needs,
//! removes what nothing uses any more, loads each model once (which also gets the graphics processor's
//! one-off setup out of the way) and keeps how fast each ran, for the AI page.

use crate::{library, llm, prefs, transcribe, tts};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use stv_core::lang;

const GB: f64 = 1e9;
/// Free space always left on the disk, in bytes.
const KEEP_FREE: u64 = 5_000_000_000;

/// What the plan is made from: the device.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    /// "NVIDIA GeForce RTX 4060 Ti", when models can run on a graphics processor.
    pub gpu: Option<String>,
    /// Its own memory, in GB; 0 for one that shares the computer's memory.
    pub gpu_memory_gb: f32,
    /// Built into the processor, sharing its memory.
    pub integrated: bool,
    pub ram_gb: f32,
    /// Free space where models are kept, in bytes.
    pub free: u64,
    pub cores: usize,
    pub phone: bool,
}

/// What's installed and chosen now.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// Speech model ids downloaded.
    pub speech: Vec<String>,
    /// Language model files, with their catalogue id when they're catalogued.
    pub llm: Vec<(String, Option<String>, u64)>,
    /// The language model file analyses run with.
    pub llm_chosen: Option<String>,
    /// Analyses run on a server the person chose.
    pub server: bool,
    /// Natural voice ids downloaded.
    pub voices: Vec<String>,
    /// Models are set to run on the graphics processor.
    pub gpu_on: bool,
    /// The multilingual speech model chosen; empty for the best one downloaded.
    pub speech_chosen: String,
    /// The primary language reads with a natural voice, and which.
    pub natural_on: bool,
    pub main_voice: String,
    pub primary: String,
    /// Natural voices run on this device (Windows for now).
    pub natural_voices: bool,
}

/// One thing the plan does.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Step {
    /// "gpu", "speech", "analysis", "voice" or "remove-llm" / "remove-speech".
    pub kind: String,
    /// The model, voice or file it's about.
    pub id: String,
    pub title: String,
    pub reason: String,
    /// To download, in bytes.
    pub download: u64,
    /// Freed by removing, in bytes.
    pub frees: u64,
    /// Already so: shown, nothing to do.
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub languages: Vec<String>,
    pub profile: Profile,
    pub steps: Vec<Step>,
    /// Things it couldn't do, said plainly.
    pub notes: Vec<String>,
}

impl Plan {
    /// Something to do: the nudge only shows when there is.
    pub fn worth_it(&self) -> bool {
        self.steps.iter().any(|s| !s.done && !s.kind.starts_with("remove"))
    }
}

/// The device as models see it.
pub fn profile() -> Profile {
    let device = llm::device();
    let gpu = llm::gpu_device();
    let phone = cfg!(any(target_os = "android", target_os = "ios"));
    Profile {
        gpu: gpu.as_ref().map(|g| g.0.clone()),
        gpu_memory_gb: gpu.as_ref().map_or(0.0, |g| g.1),
        integrated: gpu.as_ref().is_some_and(|g| g.2),
        ram_gb: device.ram_gb,
        free: llm::free_space_for_models().unwrap_or(u64::MAX),
        cores: std::thread::available_parallelism().map_or(4, |n| n.get()),
        phone,
    }
}

pub fn installed() -> Installed {
    let p = prefs::load();
    Installed {
        speech: transcribe::MODELS.iter().filter(|m| transcribe::is_ready(m)).map(|m| m.id.to_string()).collect(),
        llm: llm::installed().into_iter().map(|i| (i.file, i.catalogued.map(str::to_string), i.size)).collect(),
        llm_chosen: llm::chosen(&p.llm_model).and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())),
        server: !p.llm_server.trim().is_empty(),
        voices: tts::voices().into_iter().filter(tts::ready).map(|v| v.id).collect(),
        gpu_on: p.use_gpu.unwrap_or(true),
        speech_chosen: p.speech_model.clone(),
        natural_on: p.natural_voice,
        main_voice: tts::chosen_in(&p, &tts::voices()).id,
        primary: p.primary_language(),
        natural_voices: tts::supported(),
    }
}

fn gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 { format!("{:.1} GB", bytes as f64 / GB) } else { format!("{} MB", (bytes / 1_000_000).max(1)) }
}

/// Whether the graphics processor is worth using for models: one with its own memory, or one built in
/// with plenty of the computer's to share.
fn gpu_worth_it(d: &Profile) -> bool {
    d.gpu.is_some() && (!d.integrated || d.ram_gb >= 16.0)
}

/// The plan for `languages` on device `d` with `have` installed. No side effects.
pub fn plan(languages: &[String], d: &Profile, have: &Installed, voices: &[tts::Voice]) -> Plan {
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    let names = || languages.iter().map(|l| lang::name(l)).collect::<Vec<_>>().join(", ");
    let gpu = gpu_worth_it(d);

    // The graphics processor.
    match (&d.gpu, gpu) {
        (Some(name), true) => steps.push(Step {
            kind: "gpu".into(),
            id: "on".into(),
            title: format!("Use the graphics processor: {name}"),
            reason: "Speech, summaries and translations are made several times faster there.".into(),
            download: 0,
            frees: 0,
            done: have.gpu_on,
        }),
        (Some(name), false) => notes.push(format!("{name} shares this computer's memory, so models run on the processor, which is as fast here.")),
        (None, _) if cfg!(windows) => notes.push("No graphics processor that models can use was found, so they run on the processor. If this computer has a graphics card, updating its driver can let Voice use it.".into()),
        _ => {}
    }

    // Speech to text: English alone has its own quick model; the large multilingual one is the most
    // accurate in every language, English too, but needs a graphics processor to keep up.
    let english_only = lang::english_only(languages);
    let strong = !d.phone && (gpu || (d.ram_gb >= 16.0 && d.cores >= 12));
    let speech = if strong { "turbo" } else if english_only { "base.en" } else { "small" };
    let m = transcribe::model(speech).expect("listed");
    steps.push(Step {
        kind: "speech".into(),
        id: speech.into(),
        title: format!("Speech to text: {}", m.name),
        reason: match speech {
            "turbo" if gpu => format!("The most accurate model, for {}. The graphics processor keeps it quick.", names()),
            "turbo" => format!("The most accurate model, for {}. This processor is fast enough for it.", names()),
            "base.en" => "Quick, and the best English at its size on this device.".into(),
            _ => format!("Transcribes {} at a good speed without a graphics processor.", names()),
        },
        download: if have.speech.iter().any(|s| s == speech) { 0 } else { m.size },
        frees: 0,
        // Downloaded and the one used: English alone uses its own model unless the large one is chosen.
        done: have.speech.iter().any(|s| s == speech) && (speech == "base.en" || have.speech_chosen == speech || (have.speech_chosen.is_empty() && !english_only && speech == "turbo")),
    });

    // Analysis, which translation also runs on.
    if have.server {
        notes.push("Analyses run on the server you chose, so no model is downloaded for them.".into());
    } else if d.ram_gb < 2.5 {
        notes.push("This device has too little memory to run an analysis model well. A local server can do it instead (Settings › Analysis).".into());
    } else {
        let weak: Vec<String> = languages.iter().filter(|l| lang::get(l).is_some_and(|x| x.analysis == lang::Analysis::ViaEnglish)).map(|l| lang::name(l)).collect();
        let room = d.ram_gb >= 8.0 && (!gpu || d.integrated || d.gpu_memory_gb == 0.0 || d.gpu_memory_gb >= 4.0);
        let id = if room && !d.phone { "qwen3.5-4b" } else { "qwen3-1.7b" };
        let c = llm::CATALOGUE.iter().find(|c| c.id == id).expect("catalogued");
        let installed = have.llm.iter().any(|(_, cat, _)| cat.as_deref() == Some(id));
        let chosen = installed && have.llm_chosen.as_deref() == Some(c.file);
        steps.push(Step {
            kind: "analysis".into(),
            id: id.into(),
            title: format!("Analysis and translation: {}", c.name),
            reason: match (id, weak.is_empty()) {
                ("qwen3.5-4b", false) => format!("Writes {} well, which the small model doesn't.", weak.join(" and ")),
                ("qwen3.5-4b", true) => "Better written summaries and translations, and this device has the room for it.".into(),
                _ => "Small and quick, and suits this device's memory.".into(),
            },
            download: if installed { 0 } else { c.size },
            frees: 0,
            done: chosen,
        });
        if id == "qwen3-1.7b" && !weak.is_empty() {
            notes.push(format!("The small model writes {} poorly; this device doesn't have the memory for the larger one.", weak.join(" and ")));
        }
        // Old models nothing uses any more.
        for (file, cat, size) in &have.llm {
            let old = cat.as_deref().is_some_and(|cat| llm::CATALOGUE.iter().any(|c| c.id == cat && !c.offered));
            if old {
                steps.push(Step {
                    kind: "remove-llm".into(),
                    id: file.clone(),
                    title: format!("Remove {file}"),
                    reason: "No longer offered, and not needed with the model above.".into(),
                    download: 0,
                    frees: *size,
                    done: false,
                });
            }
        }
    }
    if speech == "turbo" && have.speech.iter().any(|s| s == "small") {
        let m = transcribe::model("small").expect("listed");
        steps.push(Step {
            kind: "remove-speech".into(),
            id: "small".into(),
            title: format!("Remove {}", m.name),
            reason: "The large model replaces it.".into(),
            download: 0,
            frees: m.size,
            done: false,
        });
    }

    // A natural voice for each language, when one is offered.
    for l in languages.iter().filter(|_| have.natural_voices) {
        let speaks: Vec<&tts::Voice> = voices.iter().filter(|v| v.langs.iter().any(|x| x == l)).collect();
        // The main language reads with the main voice once natural voices are on; others with any downloaded.
        let main = *l == have.primary;
        if let Some(v) = speaks.iter().find(|v| have.voices.contains(&v.id) && (!main || v.id == have.main_voice)).or_else(|| speaks.iter().find(|v| have.voices.contains(&v.id))) {
            let used = !main || (have.natural_on && v.id == have.main_voice);
            steps.push(Step {
                kind: "voice".into(),
                id: v.id.clone(),
                title: format!("Reading {}: {}", lang::name(l), v.name),
                reason: if used { "Downloaded and reading already.".into() } else { format!("Downloaded already; it reads {} instead of the system's voice.", lang::name(l)) },
                download: 0,
                frees: 0,
                done: used,
            });
        } else if let Some(v) = tts::first_for(l).as_ref().or_else(|| speaks.iter().copied().find(|v| !v.custom)) {
            steps.push(Step {
                kind: "voice".into(),
                id: v.id.clone(),
                title: format!("Reading {}: {}", lang::name(l), v.name),
                reason: format!("A natural {} voice{} that reads offline.", lang::name(l), if v.language.is_empty() { String::new() } else { format!(" ({})", v.language) }),
                download: v.size,
                frees: 0,
                done: false,
            });
        } else {
            notes.push(format!("There's no natural {} voice yet, so the system's voice reads it.", lang::name(l)));
        }
    }

    fit(&mut steps, d, &mut notes);
    Plan { languages: languages.to_vec(), profile: d.clone(), steps, notes }
}

/// Keeps the downloads within the free space: steps down to smaller models, then leaves voices out.
fn fit(steps: &mut Vec<Step>, d: &Profile, notes: &mut Vec<String>) {
    let need = |steps: &[Step]| steps.iter().filter(|s| !s.done).map(|s| s.download).sum::<u64>();
    let frees = |steps: &[Step]| steps.iter().filter(|s| !s.done).map(|s| s.frees).sum::<u64>();
    let fits = |steps: &[Step]| d.free.saturating_add(frees(steps)) >= need(steps).saturating_add(KEEP_FREE);
    if fits(steps) {
        return;
    }
    for (kind, big, small) in [("analysis", "qwen3.5-4b", "qwen3-1.7b"), ("speech", "turbo", "small")] {
        if let Some(s) = steps.iter_mut().find(|s| s.kind == kind && s.id == big && !s.done) {
            let (title, size) = match kind {
                "analysis" => {
                    let c = llm::CATALOGUE.iter().find(|c| c.id == small).expect("catalogued");
                    (format!("Analysis and translation: {}", c.name), c.size)
                }
                _ => {
                    let m = transcribe::model(small).expect("listed");
                    (format!("Speech to text: {}", m.name), m.size)
                }
            };
            *s = Step { kind: kind.into(), id: small.into(), title, reason: "A smaller model, to leave room on the disk.".into(), download: size, frees: 0, done: false };
            notes.push("There isn't room for the larger models, so smaller ones are chosen. Freeing space and optimising again gets the larger ones.".into());
            if fits(steps) {
                return;
            }
        }
    }
    while !fits(steps) {
        let Some(i) = steps.iter().rposition(|s| s.kind == "voice" && !s.done) else { break };
        let s = steps.remove(i);
        notes.push(format!("{} was left out to leave room on the disk.", s.title));
    }
    if !fits(steps) {
        notes.push(format!("There's too little free space for these downloads ({} needed). Free some space and try again.", gb(need(steps))));
    }
}

// ---------------------------------------------------------------------------------------------------------
// Applying a plan

/// How optimising is going, for the page.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub running: bool,
    /// The step being done, by its title.
    pub step: String,
    pub index: usize,
    pub count: usize,
    pub fraction: f32,
    pub error: Option<String>,
    /// Steps that didn't work, said plainly; the rest went ahead.
    pub problems: Vec<String>,
    pub finished: bool,
}

/// How fast models ran after optimising, kept for the AI page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Measured {
    pub at: u64,
    pub languages: Vec<String>,
    /// The device it was done for, to notice when it changes.
    pub gpu: Option<String>,
    pub ram_gb: f32,
    /// Seconds to transcribe 10 s of speech.
    pub speech_s: Option<f32>,
    pub speech_on_gpu: bool,
    /// Words of analysis written each second.
    pub words_per_s: Option<f32>,
    pub analysis_on_gpu: bool,
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress { running: false, step: String::new(), index: 0, count: 0, fraction: 0.0, error: None, problems: Vec::new(), finished: false });
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn progress() -> Progress {
    PROGRESS.lock().unwrap().clone()
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    tts::cancel_download();
}

fn result_path() -> std::path::PathBuf {
    library::data_dir().join("optimised.json")
}

pub fn last() -> Option<Measured> {
    library::read_json(&result_path()).ok()
}

/// Whether to suggest optimising: never done, or done for a different device or set of languages.
pub fn stale(p: &Profile, languages: &[String]) -> bool {
    match last() {
        None => true,
        Some(r) => r.gpu != p.gpu || (r.ram_gb - p.ram_gb).abs() > 1.0 || !languages.iter().all(|l| r.languages.contains(l)),
    }
}

/// Does the steps of `plan` named in `chosen` (by kind and id), reporting through `tell`. Runs on the
/// caller's thread; one at a time.
pub fn apply(plan: &Plan, chosen: &[(String, String)], tell: impl Fn(&Progress)) -> Result<Measured, String> {
    {
        let mut p = PROGRESS.lock().unwrap();
        if p.running {
            return Err("Voice is already being optimised.".into());
        }
        *p = Progress { running: true, ..Default::default() };
    }
    CANCEL.store(false, Ordering::SeqCst);
    let steps: Vec<&Step> = plan.steps.iter().filter(|s| !s.done && chosen.iter().any(|(k, i)| *k == s.kind && *i == s.id)).collect();
    let count = steps.len() + 1;
    let update = |f: &dyn Fn(&mut Progress)| {
        let mut p = PROGRESS.lock().unwrap();
        f(&mut p);
        tell(&p);
    };
    let mut problems = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        if CANCEL.load(Ordering::SeqCst) {
            break;
        }
        update(&|p| {
            p.step = step.title.clone();
            p.index = index + 1;
            p.count = count;
            p.fraction = 0.0;
        });
        let fraction = |f: f32| update(&|p| p.fraction = f);
        let done = match step.kind.as_str() {
            "gpu" => prefs::update(|p| p.use_gpu = Some(true)).map(|_| ()),
            "speech" => match transcribe::model(&step.id) {
                Some(m) if transcribe::is_ready(m) => Ok(()),
                _ => transcribe::download(&step.id, &CANCEL, fraction),
            }
            .and_then(|_| {
                prefs::update(|p| {
                    if step.id != "base.en" {
                        p.speech_model = step.id.clone();
                    }
                })
                .map(|_| ())
            }),
            "analysis" => match llm::CATALOGUE.iter().find(|c| c.id == step.id) {
                None => Err("That model isn't in the list.".to_string()),
                Some(c) => {
                    let have = llm::installed().iter().any(|i| i.catalogued == Some(c.id));
                    let got = if have { Ok(()) } else { llm::download(c.id, &CANCEL, fraction) };
                    got.and_then(|_| prefs::update(|p| p.llm_model = c.file.to_string()).map(|_| ()))
                }
            },
            // Old models go only once the new one is in place.
            "remove-llm" if failed.iter().any(|k| k == "analysis") || !llm::installed().iter().any(|i| i.catalogued.is_some_and(|c| llm::CATALOGUE.iter().any(|x| x.id == c && x.offered))) => Ok(()),
            "remove-speech" if failed.iter().any(|k| k == "speech") => Ok(()),
            "voice" => {
                let ready = tts::find_voice(&step.id).is_ok_and(|v| tts::ready(&v));
                let got = if ready { Ok(()) } else { tts::download(&step.id, fraction) };
                // A voice for the main language becomes the main voice.
                let primary = prefs::load().primary_language();
                let speaks_main = tts::find_voice(&step.id).is_ok_and(|v| v.langs.contains(&primary));
                got.and_then(|_| if speaks_main { prefs::update(|p| { p.tts_voice = step.id.clone(); p.natural_voice = true; }).map(|_| ()) } else { Ok(()) })
            }
            "remove-llm" => llm::delete(&step.id),
            "remove-speech" => transcribe::delete(&step.id),
            _ => Ok(()),
        };
        if let Err(e) = done {
            problems.push(format!("{}: {e}", step.title));
            failed.push(step.kind.clone());
        }
    }
    let cancelled = CANCEL.load(Ordering::SeqCst);
    // Trying each model once: how fast it is, and the graphics processor's setup done now rather than in
    // the next meeting. A graphics processor that fails here is turned off.
    let mut result = Measured { at: library::now(), languages: plan.languages.clone(), gpu: plan.profile.gpu.clone(), ram_gb: plan.profile.ram_gb, ..Default::default() };
    if !cancelled {
        update(&|p| {
            p.step = "Trying the models (the first time on the graphics processor takes a little longer)".into();
            p.index = count;
            p.count = count;
            p.fraction = 0.0;
        });
        let gpu = prefs::load().use_gpu.unwrap_or(true) && plan.profile.gpu.is_some();
        match try_speech(&plan.languages) {
            Ok(s) => {
                result.speech_s = Some(s);
                result.speech_on_gpu = gpu && cfg!(windows);
            }
            Err(e) => problems.push(format!("Speech to text: {e}")),
        }
        update(&|p| p.fraction = 0.5);
        match try_analysis() {
            Ok(Some(w)) => {
                result.words_per_s = Some(w);
                result.analysis_on_gpu = gpu;
            }
            Ok(None) => {}
            Err(e) => problems.push(format!("Analysis: {e}")),
        }
        let _ = library::write_json(&result_path(), &result);
    }
    update(&|p| {
        p.running = false;
        p.finished = true;
        p.fraction = 1.0;
        p.problems = problems.clone();
        p.error = cancelled.then(|| "Stopped. What was downloaded is kept; optimise again to finish.".to_string());
    });
    Ok(result)
}

/// Seconds the speech model takes for 10 s of audio.
fn try_speech(languages: &[String]) -> Result<f32, String> {
    let t = transcribe::Transcriber::for_languages(languages)?;
    // Quiet noise rather than silence, so the whole model runs as it does on speech.
    let mut seed = 12345u32;
    let pcm: Vec<i16> = (0..160_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((seed >> 16) as i16) / 64
        })
        .collect();
    let language = languages.first().map(String::as_str).unwrap_or("en");
    // The first run sets things up; the second is timed.
    t.transcribe(&pcm, 0, pcm.len(), None, language)?;
    let started = std::time::Instant::now();
    t.transcribe(&pcm, 0, pcm.len(), None, language)?;
    Ok(started.elapsed().as_secs_f32())
}

/// Words a second the analysis model writes, or None when analyses run on a server or there's no model.
fn try_analysis() -> Result<Option<f32>, String> {
    let p = prefs::load();
    if !p.llm_server.trim().is_empty() {
        return Ok(None);
    }
    let Some(path) = llm::chosen(&p.llm_model) else { return Ok(None) };
    let device = llm::device();
    let gpu = p.use_gpu.unwrap_or(true);
    #[cfg(any(windows, target_os = "macos"))]
    let mut model = crate::isolate::Isolated::load(&path, 2048, device.threads, gpu, Default::default())?;
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut model = llm::Local::load(&path, 2048, device.threads, gpu, Default::default())?;
    use stv_core::analysis::Model;
    let grammar = r#"root ::= [a-zA-Z .,']+"#;
    let schema = serde_json::Value::Null;
    model.complete("You are concise.", "Say hello.", grammar, &schema, 8)?;
    let started = std::time::Instant::now();
    let text = model.complete("You write plainly.", "Describe a busy harbour in about eighty words.", grammar, &schema, 120)?;
    let words = text.split_whitespace().count() as f32;
    Ok(Some(words / started.elapsed().as_secs_f32().max(0.01)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(gpu: Option<(&str, f32, bool)>, ram_gb: f32, free_gb: f32) -> Profile {
        Profile {
            gpu: gpu.map(|g| g.0.to_string()),
            gpu_memory_gb: gpu.map_or(0.0, |g| g.1),
            integrated: gpu.is_some_and(|g| g.2),
            ram_gb,
            free: (free_gb as f64 * GB) as u64,
            cores: 8,
            phone: false,
        }
    }

    fn langs(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn pick<'a>(p: &'a Plan, kind: &str) -> Option<&'a Step> {
        p.steps.iter().find(|s| s.kind == kind)
    }

    #[test]
    fn a_gaming_pc_gets_the_large_models_and_its_graphics_card() {
        let p = plan(&langs(&["en", "es"]), &device(Some(("RTX 4060 Ti", 8.0, false)), 32.0, 200.0), &Installed { natural_voices: true, ..Default::default() }, &tts::offered());
        assert_eq!(pick(&p, "speech").unwrap().id, "turbo");
        assert_eq!(pick(&p, "analysis").unwrap().id, "qwen3.5-4b");
        assert!(pick(&p, "gpu").is_some());
        let voices: Vec<&str> = p.steps.iter().filter(|s| s.kind == "voice").map(|s| s.id.as_str()).collect();
        assert_eq!(voices.len(), 2, "{voices:?}");
        assert!(p.worth_it());
    }

    #[test]
    fn a_laptop_without_a_graphics_card_gets_quick_models() {
        let p = plan(&langs(&["en"]), &device(None, 8.0, 100.0), &Installed { natural_voices: true, ..Default::default() }, &tts::offered());
        assert_eq!(pick(&p, "speech").unwrap().id, "base.en");
        assert!(pick(&p, "gpu").is_none());
        let p = plan(&langs(&["en", "fr"]), &device(None, 6.0, 100.0), &Installed { natural_voices: true, ..Default::default() }, &tts::offered());
        assert_eq!(pick(&p, "speech").unwrap().id, "small");
        assert_eq!(pick(&p, "analysis").unwrap().id, "qwen3-1.7b");
    }

    #[test]
    fn built_in_graphics_with_little_memory_stays_on_the_processor() {
        let p = plan(&langs(&["en"]), &device(Some(("Intel UHD", 0.0, true)), 8.0, 100.0), &Installed { natural_voices: true, ..Default::default() }, &tts::offered());
        assert!(pick(&p, "gpu").is_none());
        assert!(p.notes.iter().any(|n| n.contains("shares")));
    }

    #[test]
    fn a_full_disk_steps_down_then_leaves_voices_out() {
        let p = plan(&langs(&["en", "es"]), &device(Some(("RTX", 8.0, false)), 32.0, 6.0), &Installed { natural_voices: true, ..Default::default() }, &tts::offered());
        assert_eq!(pick(&p, "analysis").unwrap().id, "qwen3-1.7b");
        let need: u64 = p.steps.iter().filter(|s| !s.done).map(|s| s.download).sum();
        assert!(need + KEEP_FREE <= 6_000_000_000 || p.notes.iter().any(|n| n.contains("too little")));
    }

    #[test]
    fn what_is_already_installed_is_not_downloaded_again() {
        let have = Installed {
            speech: vec!["turbo".into()],
            llm: vec![("Qwen3.5-4B-Q4_K_M.gguf".into(), Some("qwen3.5-4b".into()), 2_740_937_888), ("qwen2.5-1.5b-instruct-q4_k_m.gguf".into(), Some("qwen2.5-1.5b".into()), 1_117_320_736)],
            llm_chosen: Some("Qwen3.5-4B-Q4_K_M.gguf".into()),
            server: false,
            voices: vec!["vits-piper-en_GB-alba-medium".into()],
            gpu_on: true,
            speech_chosen: "turbo".into(),
            natural_on: true,
            main_voice: "vits-piper-en_GB-alba-medium".into(),
            primary: "en".into(),
            natural_voices: true,
        };
        let p = plan(&langs(&["en"]), &device(Some(("RTX", 8.0, false)), 32.0, 200.0), &have, &tts::offered());
        assert!(pick(&p, "gpu").unwrap().done);
        assert!(pick(&p, "speech").unwrap().done);
        assert!(pick(&p, "analysis").unwrap().done);
        assert!(pick(&p, "voice").unwrap().done);
        assert_eq!(pick(&p, "remove-llm").unwrap().id, "qwen2.5-1.5b-instruct-q4_k_m.gguf");
    }
}
