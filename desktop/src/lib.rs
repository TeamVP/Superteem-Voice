//! Voice by Superteem, shared by Windows, the Mac, Android and the iPhone.

pub mod analysis;
#[cfg(not(target_os = "android"))]
pub mod capture;
pub mod dictation;
pub mod exports;
pub mod gpu;
pub mod imports;
pub mod keyphrases;
pub mod library;
pub mod llm;
#[cfg(any(windows, target_os = "macos"))]
pub mod isolate;
pub mod listen;
pub mod notes;
pub mod optimise;
pub mod prefs;
pub mod probe;
pub mod process;
pub mod recorder;
pub mod script;
pub mod shell;
pub mod transcribe;
pub mod translations;
pub mod tts;
pub mod update;
pub mod vocabulary;
pub mod voices;

use library::{Meta, PrintLink, Segment};
use recorder::Recorder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// Recording someone reading a script for their voice print.
struct Enrolment {
    recorder: Recorder,
    device: String,
    name: String,
    print: Option<String>,
}

/// Recording someone saying a key phrase several times to train it.
struct Training {
    recorder: Recorder,
    device: String,
    print: String,
    phrase: String,
    replace: bool,
}

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recorder>>,
    /// The inputs chosen for the current recording, to remember which worked.
    chosen: Mutex<Vec<String>>,
    enrolling: Mutex<Option<Enrolment>>,
    training: Mutex<Option<Training>>,
    /// The session being processed, so the library does not offer to process it again.
    processing: Mutex<Option<String>>,
    /// Sessions waiting to be processed, oldest first. One worker takes them in turn.
    queue: Mutex<std::collections::VecDeque<String>>,
    /// Edits to one session are read, changed and written as a unit.
    edit: Mutex<()>,
    /// Analyses waiting their turn, and the one running.
    analysis_queue: Mutex<std::collections::VecDeque<Job>>,
    analysing: Mutex<Option<Job>>,
    analysis_worker: std::sync::atomic::AtomicBool,
    analysis_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The catalogue model being downloaded.
    llm_downloading: Mutex<Option<String>>,
    llm_download_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    /// Only one thing records at a time.
    fn busy(&self) -> bool {
        self.recording.lock().unwrap().is_some()
            || self.enrolling.lock().unwrap().is_some()
            || self.training.lock().unwrap().is_some()
    }
}

fn id_of(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

/// Runs a blocking job off the UI thread, which Android needs for calls into the capture service.
async fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(job).await.map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct Progress {
    id: String,
    stage: String,
    fraction: f32,
}

#[derive(Serialize, Clone)]
struct QueueState {
    /// Being processed now.
    current: Option<String>,
    /// Waiting their turn, in order.
    waiting: Vec<String>,
}

fn queue_state(state: &AppState) -> QueueState {
    QueueState {
        current: state.processing.lock().unwrap().clone(),
        waiting: state.queue.lock().unwrap().iter().cloned().collect(),
    }
}

/// True while a session is being processed or waiting to be.
fn is_queued(state: &AppState, id: &str) -> bool {
    state.processing.lock().unwrap().as_deref() == Some(id) || state.queue.lock().unwrap().iter().any(|q| q == id)
}

/// Adds a session to the processing queue, starting the worker if it is idle. Each finished session
/// is announced with a "processed" event, so recording can carry on meanwhile.
fn enqueue<R: Runtime>(app: &AppHandle<R>, id: String) {
    let state = app.state::<AppState>();
    if is_queued(&state, &id) {
        return;
    }
    let start_worker = {
        let mut queue = state.queue.lock().unwrap();
        queue.push_back(id);
        // The worker is idle when nothing is being processed and this is the only one waiting.
        state.processing.lock().unwrap().is_none() && queue.len() == 1
    };
    let _ = app.emit("queue", queue_state(&state));
    if !start_worker {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        let state = app.state::<AppState>();
        let next = {
            let mut queue = state.queue.lock().unwrap();
            let next = queue.pop_front();
            // Set under the queue lock, so enqueue never sees an idle worker that is about to run.
            *state.processing.lock().unwrap() = next.clone();
            next
        };
        let Some(id) = next else { break };
        let _ = app.emit("queue", queue_state(&state));
        let summary = library::session_dir(&id).and_then(|dir| {
            let report = |stage: &str, fraction: f32| {
                let _ = app.emit("processing", Progress { id: id.clone(), stage: stage.into(), fraction });
            };
            process::process_session(&dir, &report)
        });
        let ok = summary.is_ok();
        let _ = app.emit("processed", Processed { id: id.clone(), summary });
        // The default analysis follows a processed meeting, when the person wants that and a model is here.
        let p = prefs::load();
        let model_here = !p.llm_server.trim().is_empty() || !llm::installed().is_empty();
        if ok && p.auto_analyse() && model_here {
            enqueue_analysis(&app, Job { kind: "meeting".into(), id: id.clone(), template: stv_core::analysis::DEFAULT_TEMPLATE.into() });
        }
        // Then the translations the person wants, when they want them made by themselves.
        if ok && p.auto_translate() && model_here {
            let languages = library::session_dir(&id).map(|d| p.effective(&library::load_meta(&d).langs())).unwrap_or_default();
            for to in p.translate_targets(&languages) {
                enqueue_analysis(&app, Job { kind: "meeting".into(), id: id.clone(), template: format!("{TRANSLATE}{to}") });
            }
        }
    });
}

#[tauri::command]
fn processing_queue(state: State<AppState>) -> QueueState {
    queue_state(&state)
}

#[tauri::command]
fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    }
}

#[tauri::command]
async fn list_devices() -> Result<recorder::Inputs, String> {
    blocking(recorder::inputs).await?
}

#[tauri::command]
fn model_ready() -> bool {
    transcribe::model_ready()
}

#[derive(Serialize)]
struct ModelInfo {
    ready: bool,
    folder: String,
    file: &'static str,
    url: &'static str,
    /// The proxy downloads go through, shown without its password, and where it was set.
    proxy: Option<String>,
    proxy_source: Option<&'static str>,
}

#[tauri::command]
fn model_info() -> ModelInfo {
    let proxy = transcribe::proxy();
    ModelInfo {
        ready: transcribe::model_ready(),
        folder: library::models_dir().to_string_lossy().into_owned(),
        file: transcribe::model_file(),
        url: transcribe::model_url(),
        proxy_source: proxy.as_ref().map(|p| p.source),
        proxy: proxy.map(|p| p.shown),
    }
}

/// Opens the folder the speech model goes in, creating it, so the file can be put there by hand.
#[tauri::command]
fn open_models_folder() -> Result<(), String> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return shell::open("");
    }
    let dir = library::models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    shell::open(dir)
}

#[tauri::command]
async fn download_model<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    blocking(move || transcribe::download_model(|f| drop(app.emit("model-download", f)))).await?
}

#[tauri::command]
async fn start_recording<R: Runtime>(app: AppHandle<R>, devices: Vec<String>, pins: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    // Consent is asked once recording has started, so a spoken "yes" is on the recording too.
    if devices.is_empty() && !pins {
        return Err("Choose at least one input.".into());
    }
    // The microphone test lets go of the devices before they are recorded.
    probe::stop();
    if state.busy() {
        return Err("Already recording.".into());
    }
    let (chosen_devices, chosen_pins) = (devices.clone(), pins);
    let _ = prefs::update(|p| {
        p.inputs = chosen_devices;
        p.pins = chosen_pins;
    });
    let mut chosen = devices.clone();
    if pins {
        chosen.push(prefs::PINS.into());
    }
    let dir = library::new_session_dir().map_err(|e| e.to_string())?;
    let started = blocking(move || recorder::start(dir, &devices, pins)).await??;
    *state.recording.lock().unwrap() = Some(started);
    *state.chosen.lock().unwrap() = chosen;
    Ok(())
}

/// Remembers which of the chosen inputs recorded audio, from the streams' final counts.
fn remember_outcomes(chosen: &[String], status: &recorder::Status) {
    let outcomes: Vec<prefs::Outcome> = status
        .streams
        .iter()
        .filter_map(|s| {
            let device = recorder::device_of(&s.name, chosen)?;
            let quiet_state = s.state == "recording" || s.state == "stopped";
            let problem = if s.detail.is_empty() && !quiet_state { s.state.clone() } else { s.detail.clone() };
            Some(prefs::Outcome { device, frames: s.frames, lost: s.lost, problem })
        })
        .collect();
    let _ = prefs::record_outcomes(chosen, &outcomes);
}

#[derive(Serialize)]
struct RecordingStatus {
    recording: bool,
    #[serde(flatten)]
    status: recorder::Status,
}

#[tauri::command]
async fn recording_status<R: Runtime>(app: AppHandle<R>) -> Result<RecordingStatus, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let recording = state.recording.lock().unwrap();
        if let Some(r) = recording.as_ref() {
            return Ok(RecordingStatus { recording: true, status: r.status()? });
        }
        if let Some(e) = state.enrolling.lock().unwrap().as_ref() {
            return Ok(RecordingStatus { recording: true, status: e.recorder.status()? });
        }
        let training = state.training.lock().unwrap();
        match training.as_ref() {
            Some(t) => Ok(RecordingStatus { recording: true, status: t.recorder.status()? }),
            None => Ok(RecordingStatus { recording: false, status: Default::default() }),
        }
    })
    .await?
}

#[derive(Serialize, Clone)]
struct Processed {
    id: String,
    summary: Result<String, String>,
}

/// Stops recording and queues the session for processing, unless it is being discarded.
/// Returns its id at once; the transcript follows in a "processed" event.
#[tauri::command]
async fn stop_recording<R: Runtime>(app: AppHandle<R>, discard: Option<bool>) -> Result<String, String> {
    let r = app.state::<AppState>().recording.lock().unwrap().take().ok_or("Not recording.")?;
    let chosen = std::mem::take(&mut *app.state::<AppState>().chosen.lock().unwrap());
    blocking(move || {
        let dir = r.dir().to_path_buf();
        let last = r.status();
        r.stop()?;
        if let Ok(status) = last {
            remember_outcomes(&chosen, &status);
        }
        let id = id_of(&dir);
        if !discard.unwrap_or(false) {
            enqueue(&app, id.clone());
        }
        Ok(id)
    })
    .await?
}

/// Processes a session again, or for the first time after a crash. Edited lines are kept.
#[tauri::command]
fn process_session<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    if library::load_meta(&library::session_dir(&id)?).compacted {
        return Err(process::COMPACTED.into());
    }
    enqueue(&app, id);
    Ok(())
}

#[tauri::command]
fn list_sessions(state: State<AppState>, query: String) -> Vec<library::Summary> {
    let mut busy: Vec<String> = state.recording.lock().unwrap().as_ref().map(|r| id_of(r.dir())).into_iter().collect();
    busy.extend(state.processing.lock().unwrap().clone());
    busy.extend(state.queue.lock().unwrap().iter().cloned());
    library::list(&query, &busy)
}

#[derive(Serialize)]
struct PrintInfo {
    id: String,
    name: String,
    /// Similarity when recognised automatically; None when the person linked it.
    score: Option<f32>,
}

#[derive(Serialize)]
struct SpeakerOption {
    key: String,
    name: String,
    print: Option<PrintInfo>,
    /// Seconds of clear speech, which decides whether a voice print can be made from it.
    voiced_s: f32,
}

#[derive(Serialize)]
struct SessionView {
    id: String,
    meta: Meta,
    segments: Vec<Segment>,
    speakers: Vec<SpeakerOption>,
    /// Stream name to the WAV path, which the page turns into an audio URL.
    audio: BTreeMap<String, PathBuf>,
    low_confidence: f32,
    transcribed: bool,
    needs_processing: bool,
    model_ready: bool,
    /// Every voice print, for linking a speaker to one.
    voiceprints: Vec<PrintChoice>,
    /// Key phrases heard in the session.
    keyphrases: Vec<keyphrases::Heard>,
    /// Bytes of WAVs and journals, freed by compacting.
    audio_bytes: u64,
    /// Being recorded or processed, so it can't be compacted.
    busy: bool,
    /// Tracks added after recording, with where each starts.
    imports: Vec<imports::Import>,
}

#[derive(Serialize)]
struct PrintChoice {
    id: String,
    name: String,
}

#[tauri::command]
fn get_session(state: State<AppState>, id: String) -> Result<SessionView, String> {
    let dir = library::session_dir(&id)?;
    let meta = library::load_meta(&dir);
    let segments = library::load_segments(&dir);
    let mut audio = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "wav") {
            audio.insert(path.file_stem().unwrap().to_string_lossy().into_owned(), path);
        }
    }
    let mut keys: Vec<String> = audio.keys().cloned().collect();
    keys.extend(meta.names.keys().cloned());
    keys.extend(segments.iter().map(|s| s.speaker.clone()));
    keys.push(library::UNKNOWN.into());
    keys.sort();
    keys.dedup();
    let prints = voices::list();
    let line_voices = voices::load_line_voices(&dir);
    let speakers = keys
        .into_iter()
        .map(|key| {
            let print = meta.prints.get(&key).and_then(|l| {
                prints.iter().find(|p| p.id == l.id).map(|p| PrintInfo { id: p.id.clone(), name: p.name.clone(), score: l.score })
            });
            let voiced_s = voices::speaker_voice(&segments, &line_voices, &key).map_or(0.0, |v| v.seconds());
            SpeakerOption { name: meta.name(&key), print, voiced_s, key }
        })
        .collect();
    let transcribed = segments.iter().any(|s| !s.text.is_empty());
    let busy = is_queued(&state, &id)
        || state.recording.lock().unwrap().as_ref().is_some_and(|r| r.dir() == dir);
    Ok(SessionView {
        needs_processing: !busy && process::needs_processing(&dir),
        model_ready: transcribe::model_ready(),
        voiceprints: prints.into_iter().map(|p| PrintChoice { id: p.id, name: p.name }).collect(),
        keyphrases: keyphrases::load_session(&dir),
        audio_bytes: library::audio_bytes(&dir),
        imports: imports::load(&dir),
        busy,
        id,
        meta,
        segments,
        speakers,
        audio,
        low_confidence: library::LOW_CONFIDENCE,
        transcribed,
    })
}

fn edit_session(
    state: &AppState,
    id: &str,
    change: impl FnOnce(&mut Meta, &mut Vec<Segment>) -> Result<(), String>,
) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    let dir = library::session_dir(id)?;
    let mut meta = library::load_meta(&dir);
    let mut segments = library::load_segments(&dir);
    change(&mut meta, &mut segments)?;
    library::save(&dir, &meta, &segments)
}

#[tauri::command]
fn set_session_status(state: State<AppState>, id: String, status: String) -> Result<(), String> {
    let status = library::check_status(&status)?;
    edit_session(&state, &id, |meta, _| {
        meta.status = status;
        Ok(())
    })
}

#[tauri::command]
fn rename_session(state: State<AppState>, id: String, title: String) -> Result<(), String> {
    edit_session(&state, &id, |meta, _| {
        meta.title = title.trim().to_string();
        Ok(())
    })
}

/// Renames a speaker everywhere in the session.
#[tauri::command]
fn rename_speaker(state: State<AppState>, id: String, key: String, name: String) -> Result<(), String> {
    edit_session(&state, &id, |meta, _| {
        let name = name.trim();
        // A rename means the person has taken over from automatic recognition.
        if meta.prints.get(&key).is_some_and(|l| l.score.is_some()) {
            meta.prints.remove(&key);
        }
        if name.is_empty() {
            meta.names.remove(&key);
        } else {
            meta.names.insert(key, name.to_string());
        }
        Ok(())
    })
}

/// Adds a person who has no stream, such as a guest the host wants to name.
#[tauri::command]
fn add_speaker(state: State<AppState>, id: String, name: String) -> Result<String, String> {
    let mut key = String::new();
    edit_session(&state, &id, |meta, _| {
        let n = (1..).find(|n| !meta.names.contains_key(&format!("person{n}"))).unwrap();
        key = format!("person{n}");
        meta.names.insert(key.clone(), name.trim().to_string());
        Ok(())
    })?;
    Ok(key)
}

/// Hears one line of a meeting in two languages again, as `language`. The line counts as edited by hand,
/// so transcribing the meeting again keeps it.
#[tauri::command]
async fn set_line_language<R: Runtime>(app: AppHandle<R>, id: String, segment: usize, language: String) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let dir = library::session_dir(&id)?;
        let meta = library::load_meta(&dir);
        if meta.compacted {
            return Err("This meeting's audio was deleted, so its lines can't be transcribed again.".into());
        }
        let languages = prefs::load().effective(&meta.langs());
        if !languages.contains(&language) {
            return Err("That isn't one of this meeting's languages.".into());
        }
        let seg = library::load_segments(&dir).into_iter().find(|s| s.id == segment).ok_or("That line no longer exists.")?;
        let pcm = exports::read_wav(&dir.join(format!("{}.wav", seg.stream)))?;
        let t = transcribe::Transcriber::for_languages(&languages)?;
        let at = |s: f64| ((s * process::RATE as f64) as usize).min(pcm.len());
        let terms = vocabulary::load();
        let prompt = vocabulary::prompt("", &terms);
        let mut words = Vec::new();
        for (a, b) in seg.spans() {
            let heard = t.transcribe(&pcm, at(a), at(b), prompt.as_deref(), &language)?;
            words.push(vocabulary::drop_echo(heard, &terms));
        }
        let text = words.join(" ").trim().to_string();
        edit_session(&state, &id, |_, segments| {
            let s = segments.iter_mut().find(|s| s.id == segment).ok_or("That line no longer exists.")?;
            if !text.is_empty() {
                s.text = text;
                s.fixes.clear();
            }
            s.language = if language == languages[0] { String::new() } else { language };
            s.edited = true;
            Ok(())
        })
    })
    .await?
}

/// Changes a line. When its speaker changes, returns the other lines that sound like it and still have
/// the old speaker, so the page can offer to move them too.
#[tauri::command]
fn update_segment(
    state: State<AppState>,
    id: String,
    segment: usize,
    text: Option<String>,
    speaker: Option<String>,
) -> Result<Vec<usize>, String> {
    let mut similar = Vec::new();
    edit_session(&state, &id, |_, segments| {
        let s = segments.iter_mut().find(|s| s.id == segment).ok_or("That line no longer exists.")?;
        if let Some(text) = text {
            let text = text.trim().to_string();
            // Typed words are the person's own, so the vocabulary's changes no longer line up with them.
            if text != s.text {
                s.fixes.clear();
            }
            s.text = text;
        }
        let mut moved_from = None;
        if let Some(speaker) = speaker {
            if s.speaker != speaker {
                moved_from = Some(std::mem::replace(&mut s.speaker, speaker));
            }
        }
        s.edited = true;
        if let Some(from) = moved_from {
            let line_voices = voices::load_line_voices(&library::session_dir(&id)?);
            if let Some(like) = line_voices.get(&segment) {
                similar = voices::similar_lines(segments, &line_voices, like, &from);
            }
        }
        voices::mark_overlaps(segments);
        Ok(())
    })?;
    Ok(similar)
}

/// Splits a line in two at a character in its text, as when another person cuts in mid-line.
/// There are no word timings, so the time is split in proportion to the text. Returns the new line's id.
#[tauri::command]
fn split_segment(state: State<AppState>, id: String, segment: usize, at: usize) -> Result<usize, String> {
    let mut new_id = 0;
    edit_session(&state, &id, |_, segments| {
        let i = segments.iter().position(|s| s.id == segment).ok_or("That line no longer exists.")?;
        let chars: Vec<char> = segments[i].text.chars().collect();
        let (head, tail): (String, String) = (chars[..at.min(chars.len())].iter().collect(), chars[at.min(chars.len())..].iter().collect());
        let (head, tail) = (head.trim().to_string(), tail.trim().to_string());
        if head.is_empty() || tail.is_empty() {
            return Err("Put the cursor between two words to split the line.".into());
        }
        new_id = segments.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let s = &mut segments[i];
        s.fixes.clear();
        let split_s = s.start_s + (s.end_s - s.start_s) * at as f64 / chars.len() as f64;
        let mut second = s.clone();
        s.text = head;
        s.end_s = split_s;
        s.edited = true;
        if !s.parts.is_empty() {
            let parts = std::mem::take(&mut s.parts);
            s.parts = parts.iter().filter(|p| p.0 < split_s).map(|&(a, b)| (a, b.min(split_s))).collect();
            second.parts = parts.iter().filter(|p| p.1 > split_s).map(|&(a, b)| (a.max(split_s), b)).collect();
        }
        second.id = new_id;
        second.text = tail;
        second.start_s = split_s;
        second.edited = true;
        segments.insert(i + 1, second);
        voices::mark_overlaps(segments);
        Ok(())
    })?;
    Ok(new_id)
}

/// Why two neighbouring lines can't be joined, or None when they can.
fn join_blocker(first: &Segment, second: &Segment) -> Option<&'static str> {
    if first.stream != second.stream {
        Some("These lines were heard on different microphones.")
    } else if first.overlap || second.overlap {
        Some("Someone else was talking at the same time.")
    } else {
        None
    }
}

/// Joins a line onto the one before it. The joined line takes the earlier line's speaker.
#[tauri::command]
fn join_segments(state: State<AppState>, id: String, first: usize, second: usize) -> Result<(), String> {
    edit_session(&state, &id, |_, segments| {
        let i = segments.iter().position(|s| s.id == first).ok_or("That line no longer exists.")?;
        let j = segments.iter().position(|s| s.id == second).ok_or("That line no longer exists.")?;
        // Lines between them that hold no words are hidden on the page, so they go too. Doubled-up copies
        // are hidden as well, but stay for the tracks.
        if j <= i || segments[i + 1..j].iter().any(|s| s.in_script() && !process::is_silence(&s.text)) {
            return Err("Only neighbouring lines can be joined.".into());
        }
        if let Some(why) = join_blocker(&segments[i], &segments[j]) {
            return Err(why.into());
        }
        let next = segments.remove(j);
        let between: Vec<usize> = (i + 1..j).filter(|&k| segments[k].in_script()).collect();
        for k in between.into_iter().rev() {
            segments.remove(k);
        }
        let s = &mut segments[i];
        let mut parts = s.spans();
        parts.extend(next.spans());
        s.parts = parts;
        let shift = s.text.split_whitespace().count();
        s.fixes.extend(next.fixes.iter().map(|f| stv_core::vocab::Fix { at: f.at + shift, ..f.clone() }));
        s.text = format!("{} {}", s.text.trim(), next.text.trim()).trim().to_string();
        s.end_s = s.end_s.max(next.end_s);
        s.confidence = s.confidence.min(next.confidence);
        s.edited = true;
        let joined = s.id;
        // Copies of the line that went now copy the line it joined.
        for copy in segments.iter_mut().filter(|c| c.echo_of == Some(next.id)) {
            copy.echo_of = Some(joined);
        }
        voices::mark_overlaps(segments);
        Ok(())
    })
}

/// Moves several lines to one speaker, as when accepting the lines that sound alike.
#[tauri::command]
fn move_lines(state: State<AppState>, id: String, segments: Vec<usize>, speaker: String) -> Result<(), String> {
    edit_session(&state, &id, |_, all| {
        for s in all.iter_mut().filter(|s| segments.contains(&s.id)) {
            s.speaker = speaker.clone();
            s.edited = true;
        }
        voices::mark_overlaps(all);
        Ok(())
    })
}

/// Two speakers turned out to be one person: every line of `from` goes to `into`.
#[tauri::command]
fn merge_speakers(state: State<AppState>, id: String, from: String, into: String) -> Result<(), String> {
    edit_session(&state, &id, |meta, segments| {
        for s in segments.iter_mut().filter(|s| s.speaker == from) {
            s.speaker = into.clone();
            s.edited = true;
        }
        meta.names.remove(&from);
        meta.prints.remove(&from);
        voices::mark_overlaps(segments);
        Ok(())
    })
}

/// Says which voice print a speaker is, or that they are none (`print` None). With `learn`, this
/// session's speech for them is added to the print so it recognises them better next time.
#[tauri::command]
fn link_speaker(state: State<AppState>, id: String, key: String, print: Option<String>, learn: bool) -> Result<(), String> {
    edit_session(&state, &id, |meta, segments| {
        let Some(print_id) = print else {
            meta.prints.remove(&key);
            return Ok(());
        };
        let found = voices::list().into_iter().find(|p| p.id == print_id).ok_or("That voice print no longer exists.")?;
        if learn {
            let dir = library::session_dir(&id)?;
            let voice = voices::speaker_voice(segments, &voices::load_line_voices(&dir), &key)
                .ok_or("This speaker has no clear speech to learn from.")?;
            voices::add_sample(Some(&print_id), &found.name, voice, &id)?;
        }
        // A print is one person, so another speaker linked to it is unlinked.
        meta.prints.retain(|k, l| k == &key || l.id != print_id);
        meta.prints.insert(key.clone(), PrintLink { id: print_id, score: None });
        meta.names.insert(key, found.name);
        Ok(())
    })
}

/// Starts a voice print from what a speaker said in this session.
#[tauri::command]
fn save_speaker_print(state: State<AppState>, id: String, key: String, name: String, consent: bool) -> Result<(), String> {
    if !consent {
        return Err("Confirm that this person agreed to have a voice print kept.".into());
    }
    edit_session(&state, &id, |meta, segments| {
        let dir = library::session_dir(&id)?;
        let voice = voices::speaker_voice(segments, &voices::load_line_voices(&dir), &key).ok_or("This speaker has no clear speech.")?;
        let print_id = voices::add_sample(None, &name, voice, &id)?;
        meta.prints.insert(key.clone(), PrintLink { id: print_id, score: None });
        meta.names.insert(key, name.trim().to_string());
        Ok(())
    })
}

#[derive(Serialize)]
struct PrintSummary {
    id: String,
    name: String,
    created: u64,
    seconds: f32,
    samples: usize,
    scripts: usize,
    /// Key phrases trained, of how many there are.
    phrases: usize,
    phrases_of: usize,
}

#[tauri::command]
fn list_voiceprints() -> Vec<PrintSummary> {
    let mut out: Vec<PrintSummary> = voices::list()
        .into_iter()
        .map(|p| PrintSummary {
            seconds: p.voice.seconds(),
            samples: p.samples.len(),
            scripts: p.samples.iter().filter(|s| s.source == "script").count(),
            phrases: keyphrases::trained_count(&p.id),
            phrases_of: keyphrases::PHRASES.len(),
            id: p.id,
            name: p.name,
            created: p.created,
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

#[tauri::command]
fn rename_voiceprint(id: String, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A voice print needs a name.".into());
    }
    voices::edit(|prints| {
        prints.iter_mut().find(|p| p.id == id).ok_or("That voice print no longer exists.")?.name = name;
        Ok(())
    })
}

/// Sessions keep the names already given; new recordings no longer recognise the person.
#[tauri::command]
fn delete_voiceprint(id: String) -> Result<(), String> {
    voices::edit(|prints| {
        prints.retain(|p| p.id != id);
        Ok(())
    })?;
    keyphrases::delete(&id);
    listen::phrases_changed();
    Ok(())
}

#[tauri::command]
async fn start_enrolment(
    state: State<'_, AppState>,
    device: String,
    name: String,
    print: Option<String>,
    consent: bool,
) -> Result<(), String> {
    if !consent {
        return Err("Confirm that this person agreed to have a voice print kept.".into());
    }
    if print.is_none() && name.trim().is_empty() {
        return Err("Give the voice print a name.".into());
    }
    if state.busy() {
        return Err("Already recording.".into());
    }
    let chosen = device.clone();
    let _ = prefs::update(|p| {
        // It becomes the default microphone everywhere, meetings included.
        p.inputs = vec![chosen.clone()];
        p.enrol_input = Some(chosen);
    });
    let dir = library::data_dir().join(format!("enrol-{}", library::now()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let devices = vec![device.clone()];
    let recorder = blocking(move || recorder::start(dir, &devices, false)).await??;
    *state.enrolling.lock().unwrap() = Some(Enrolment { recorder, device, name, print });
    listen::set_paused(true);
    Ok(())
}

/// Stops the script recording. With `save`, the voice is added to the print (a new one if needed).
/// The audio is deleted either way: only the description of the voice is kept.
#[tauri::command]
async fn stop_enrolment<R: Runtime>(app: AppHandle<R>, save: bool) -> Result<Option<String>, String> {
    let e = app.state::<AppState>().enrolling.lock().unwrap().take().ok_or("Not recording.")?;
    listen::set_paused(false);
    blocking(move || {
        let dir = e.recorder.dir().to_path_buf();
        let last = e.recorder.status();
        e.recorder.stop()?;
        if let Ok(status) = last {
            remember_outcomes(std::slice::from_ref(&e.device), &status);
        }
        let result = if save {
            enrolment_voice(&dir).and_then(|v| voices::add_sample(e.print.as_deref(), &e.name, v, "script").map(Some))
        } else {
            Ok(None)
        };
        let _ = std::fs::remove_dir_all(&dir);
        result
    })
    .await?
}

fn enrolment_voice(dir: &Path) -> Result<stv_core::embed::Voice, String> {
    let pcm = longest_recording(dir)?;
    stv_core::embed::voice(&pcm, process::RATE)
        .ok_or_else(|| "No clear speech was heard. Check the microphone and read the script aloud.".into())
}

/// The audio of the longest stream in a folder of journals.
fn longest_recording(dir: &Path) -> Result<Vec<i16>, String> {
    let mut best: Option<Vec<i16>> = None;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if entry.path().extension().is_some_and(|x| x == "stvj") {
            let source = stv_core::pipeline::load_source(&entry.path(), process::RATE).map_err(|e| e.to_string())?;
            if best.as_ref().map_or(true, |b| source.pcm.len() > b.len()) {
                best = Some(source.pcm);
            }
        }
    }
    best.ok_or_else(|| "No audio was recorded. Check the microphone.".into())
}

#[derive(Serialize)]
struct PhraseView {
    id: &'static str,
    text: &'static str,
    does: &'static str,
    how: &'static [&'static str],
    /// The phrase that must come just before, as said.
    after: Option<&'static str>,
    takes: usize,
    separation: Option<f32>,
    updated: u64,
}

#[derive(Serialize)]
struct KeyphrasesView {
    phrases: Vec<PhraseView>,
    sensitivity: f32,
    verify: bool,
    /// Whether anyone has trained a phrase, so live listening can be tried.
    any_trained: bool,
}

#[tauri::command]
fn list_keyphrases(print: Option<String>) -> KeyphrasesView {
    let trained = print.as_deref().map(keyphrases::load).unwrap_or_default();
    let prefs = prefs::load();
    let phrases = keyphrases::PHRASES
        .iter()
        .map(|s| {
            let phrase = trained.phrases.get(s.id);
            PhraseView {
                id: s.id,
                text: s.text,
                does: s.does,
                how: s.how,
                after: s.requires.and_then(keyphrases::spec).map(|r| r.text),
                // A wake word from before takes were checked has to be trained again.
                takes: phrase.filter(|p| p.checked || s.id != keyphrases::WAKE).map_or(0, |p| p.said()),
                separation: phrase.filter(|p| p.usable(s.id)).and_then(|p| p.calibration).map(|c| c.separation),
                updated: phrase.map_or(0, |p| p.updated),
            }
        })
        .collect();
    KeyphrasesView { phrases, sensitivity: prefs.kws_sensitivity(), verify: prefs.kws_verify, any_trained: !keyphrases::models().0.is_empty() }
}

#[tauri::command]
async fn start_phrase_training(
    state: State<'_, AppState>,
    print: String,
    phrase: String,
    device: String,
    replace: bool,
) -> Result<(), String> {
    keyphrases::spec(&phrase).ok_or("That isn't a key phrase.")?;
    voices::list().iter().find(|p| p.id == print).ok_or("That voice print no longer exists.")?;
    if state.busy() {
        return Err("Already recording.".into());
    }
    let chosen = device.clone();
    let _ = prefs::update(|p| p.enrol_input = Some(chosen));
    let dir = library::data_dir().join(format!("phrase-{}", library::now()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let devices = vec![device.clone()];
    let recorder = blocking(move || recorder::start(dir, &devices, false)).await??;
    *state.training.lock().unwrap() = Some(Training { recorder, device, print, phrase, replace });
    listen::set_paused(true);
    Ok(())
}

/// Stops the takes recording. With `save`, the takes are added to the phrase. The audio is deleted.
#[tauri::command]
async fn stop_phrase_training<R: Runtime>(app: AppHandle<R>, save: bool) -> Result<Option<keyphrases::TrainResult>, String> {
    let t = app.state::<AppState>().training.lock().unwrap().take().ok_or("Not recording.")?;
    // Commands stay paused until the takes are learnt, so the last take isn't acted on as it's said.
    let result = blocking(move || {
        let dir = t.recorder.dir().to_path_buf();
        let last = t.recorder.status();
        t.recorder.stop()?;
        if let Ok(status) = last {
            remember_outcomes(std::slice::from_ref(&t.device), &status);
        }
        let result = if save {
            longest_recording(&dir).and_then(|pcm| keyphrases::train(&t.print, &t.phrase, &pcm, t.replace)).map(Some)
        } else {
            Ok(None)
        };
        let _ = std::fs::remove_dir_all(&dir);
        listen::phrases_changed();
        result
    })
    .await;
    listen::set_paused(false);
    result?
}

/// Setup's practice of "Hey Superteem": each time it is said it is checked, and a "wake-practice" event
/// says whether it counted.
#[tauri::command]
fn start_wake_practice(state: State<'_, AppState>) -> Result<(), String> {
    if state.busy() {
        return Err("Already recording.".into());
    }
    listen::set_practising(true)
}

/// Ends the practice. With `print`, its good takes become that voice print's wake word.
#[tauri::command]
async fn stop_wake_practice(print: Option<String>) -> Result<Option<keyphrases::TrainResult>, String> {
    listen::set_practising(false)?;
    let takes = listen::practice_takes();
    let Some(print) = print else { return Ok(None) };
    let result = blocking(move || keyphrases::save_checked(&print, keyphrases::WAKE, takes)).await??;
    listen::phrases_changed();
    Ok(Some(result))
}

#[tauri::command]
fn clear_keyphrase(print: String, phrase: String) -> Result<(), String> {
    keyphrases::clear(&print, &phrase)?;
    listen::phrases_changed();
    Ok(())
}

#[tauri::command]
fn save_keyphrase_settings(sensitivity: f32, verify: bool) -> Result<(), String> {
    prefs::update(|p| {
        p.kws_sensitivity = Some(sensitivity.clamp(0.7, 1.4));
        p.kws_verify = verify;
    })?;
    listen::phrases_changed();
    Ok(())
}


#[tauri::command]
fn get_prefs() -> prefs::Prefs {
    prefs::load()
}

#[tauri::command]
fn save_prefs(
    theme: Option<String>,
    scripts: Option<Vec<prefs::Script>>,
    wake: Option<bool>,
    quiet: Option<bool>,
    onboarded: Option<bool>,
    setup_step: Option<u32>,
    accept_terms: Option<bool>,
    hidden_mics: Option<Vec<String>>,
    pins_added: Option<bool>,
    keep_audio: Option<bool>,
    proxy: Option<String>,
) -> Result<(), String> {
    prefs::update(|p| {
        if let Some(proxy) = proxy {
            p.proxy = proxy.trim().to_string();
        }
        if let Some(keep) = keep_audio {
            p.keep_audio = keep;
        }
        if let Some(hidden) = hidden_mics {
            p.hidden_mics = hidden;
        }
        if let Some(added) = pins_added {
            p.pins_added = added;
        }
        if let Some(wake) = wake {
            p.wake = wake;
        }
        if let Some(quiet) = quiet {
            p.quiet = quiet;
        }
        if let Some(onboarded) = onboarded {
            p.onboarded = onboarded;
        }
        if let Some(step) = setup_step {
            p.setup_step = step;
        }
        if accept_terms == Some(true) {
            p.terms_accepted = library::now();
        }
        if let Some(theme) = theme {
            p.theme = theme;
        }
        if let Some(scripts) = scripts {
            p.scripts = scripts;
        }
    })
    .map(|_| ())
}

#[tauri::command]
fn save_markers(state: State<AppState>, id: String, markers: Vec<library::Marker>) -> Result<(), String> {
    edit_session(&state, &id, |meta, _| {
        meta.markers.extend(markers);
        meta.markers.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
        Ok(())
    })
}

/// Changes the note on a marker, or deletes it. `index` is its place among the saved markers; without one,
/// it is a phrase heard in the recording at `at_s`, which is saved as a marker of its own to change it.
#[tauri::command]
fn edit_marker(
    state: State<AppState>,
    id: String,
    index: Option<usize>,
    at_s: f64,
    kind: String,
    text: String,
    remove: bool,
) -> Result<(), String> {
    edit_session(&state, &id, |meta, _| {
        match index {
            Some(i) => {
                let m = meta.markers.get_mut(i).ok_or("That bookmark no longer exists.")?;
                m.text = text.trim().to_string();
                m.removed = remove;
            }
            None => {
                meta.markers.push(library::Marker { at_s, text: text.trim().to_string(), kind, removed: remove });
                meta.markers.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
            }
        }
        Ok(())
    })
}

/// Puts a doubled-up line back in the script, when it was something said after all.
#[tauri::command]
fn keep_segment(state: State<AppState>, id: String, segment: usize) -> Result<(), String> {
    edit_session(&state, &id, |_, segments| {
        let s = segments.iter_mut().find(|s| s.id == segment).ok_or("That line no longer exists.")?;
        s.echo_of = None;
        s.edited = true;
        voices::mark_overlaps(segments);
        Ok(())
    })
}

/// Exports a meeting as a transcript ("txt", "md", "srt", "vtt") or an edit package ("package"), named
/// after `title` as the app shows it. Returns where it was written.
#[tauri::command]
async fn export_session(id: String, title: String, format: String) -> Result<String, String> {
    blocking(move || {
        let dir = library::session_dir(&id)?;
        let meta = library::load_meta(&dir);
        let segments = library::load_segments(&dir);
        let heard = keyphrases::load_session(&dir);
        let request = exports::MeetingExport { dir: &dir, title: &title, meta: &meta, segments: &segments, heard: &heard };
        exports::export_meeting(&request, &format).map(|p| p.to_string_lossy().into_owned())
    })
    .await?
}

/// Exports a note as "txt" or "md", or with its kept dictation audio ("package").
#[tauri::command]
async fn export_note(id: String, format: String) -> Result<String, String> {
    blocking(move || exports::export_note(&id, &format).map(|p| p.to_string_lossy().into_owned())).await?
}

#[tauri::command]
fn reveal_export(path: String) -> Result<(), String> {
    exports::reveal(std::path::Path::new(&path))
}

#[tauri::command]
fn save_consent(state: State<AppState>, id: String, consent: library::Consent) -> Result<(), String> {
    edit_session(&state, &id, |meta, _| {
        meta.consent = Some(consent);
        Ok(())
    })
}

#[tauri::command]
fn list_notes(query: String) -> Vec<notes::NoteSummary> {
    notes::list(&query)
}

#[tauri::command]
fn get_note(id: String) -> Result<notes::Note, String> {
    notes::load(&id)
}

#[tauri::command]
fn save_note(id: Option<String>, title: String, body: String, keep_audio: Option<bool>) -> Result<String, String> {
    notes::save(id, title, body, keep_audio.unwrap_or(false))
}

#[tauri::command]
fn archive_note(id: String, archived: bool) -> Result<(), String> {
    notes::set_archived(&id, archived)
}

#[tauri::command]
fn set_note_status(id: String, status: String) -> Result<(), String> {
    notes::set_status(&id, &status)
}

#[tauri::command]
fn delete_note(id: String) -> Result<(), String> {
    notes::delete(&id)?;
    analysis::forget_note(&id);
    Ok(())
}

#[tauri::command]
async fn set_wake(on: bool) -> Result<(), String> {
    blocking(move || listen::set_wake(on)).await?
}

#[tauri::command]
async fn dictate_start<R: Runtime>(
    app: AppHandle<R>,
    continuous: Option<bool>,
    note: Option<String>,
    languages: Option<Vec<String>>,
) -> Result<u64, String> {
    let languages = stv_core::lang::clean_list(&languages.unwrap_or_default());
    if !continuous.unwrap_or(false) {
        return blocking(listen::dictate_start).await?.map(|_| 0);
    }
    // Written down piece by piece as the person pauses, and ended by itself after a silence.
    let (clips, words, ended) = (app.clone(), app.clone(), app.clone());
    blocking(move || {
        dictation::start(
            note,
            languages,
            move |c| drop(clips.emit("dictation-clip", c)),
            move |w| drop(words.emit("dictation-words", w)),
            move |e| drop(ended.emit("dictation-ended", e)),
        )
    })
    .await?
}

#[tauri::command]
async fn dictate_stop(note: Option<String>, session: Option<u64>, languages: Option<Vec<String>>) -> Result<String, String> {
    if let Some(id) = session.filter(|&s| s > 0) {
        return blocking(move || dictation::stop(id)).await?;
    }
    let languages = stv_core::lang::clean_list(&languages.unwrap_or_default());
    blocking(move || {
        let (words, pcm) = listen::dictate_stop(&languages)?;
        // The audio is kept only when the note asks, and only once the words are written down.
        if let Some(id) = note.filter(|_| !words.is_empty()) {
            notes::keep_dictation(&id, &pcm, process::RATE)?;
        }
        Ok(words)
    })
    .await?
}

/// The person's vocabulary, as the page lists it.
#[tauri::command]
fn list_vocabulary() -> Vec<stv_core::vocab::Term> {
    let mut terms = vocabulary::load();
    terms.sort_by_key(|t| t.spelling.to_lowercase());
    terms
}

#[tauri::command]
fn save_vocab_term(term: stv_core::vocab::Term) -> Result<stv_core::vocab::Term, String> {
    vocabulary::save(term)
}

#[tauri::command]
fn delete_vocab_term(id: String) -> Result<(), String> {
    vocabulary::delete(&id)
}

/// Undoes a vocabulary change in a line, or makes a suggested one.
#[tauri::command]
fn toggle_fix(state: State<AppState>, id: String, segment: usize, fix: usize) -> Result<(), String> {
    edit_session(&state, &id, |_, segments| {
        let s = segments.iter_mut().find(|s| s.id == segment).ok_or("That line no longer exists.")?;
        vocabulary::toggle(s, fix)
    })
}

/// Corrects every saved meeting, or only the meeting `only`, with the vocabulary, leaving lines the person
/// edited alone. With `dry_run`, only counts the words that would change. Meetings being recorded or
/// processed are skipped.
#[tauri::command]
async fn fix_past_transcripts<R: Runtime>(app: AppHandle<R>, dry_run: bool, only: Option<String>) -> Result<usize, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let terms = vocabulary::load();
        let mut changed = 0;
        for summary in library::list("", &[]).into_iter().filter(|s| only.as_ref().is_none_or(|o| *o == s.id)) {
            let busy = is_queued(&state, &summary.id)
                || state.recording.lock().unwrap().as_ref().is_some_and(|r| r.dir().file_name().is_some_and(|n| n == summary.id.as_str()));
            if busy {
                continue;
            }
            let Ok(dir) = library::session_dir(&summary.id) else { continue };
            let _guard = state.edit.lock().unwrap();
            changed += vocabulary::correct_session(&dir, &terms, dry_run)?;
        }
        Ok(changed)
    })
    .await?
}

/// Ends a "how does whisper write it" test: the words spoken since `dictate_start`, unsteered and uncorrected.
#[tauri::command]
async fn vocab_test_stop() -> Result<String, String> {
    blocking(listen::dictate_stop_plain).await?
}

#[tauri::command]
async fn dictate_cancel(session: Option<u64>) -> Result<(), String> {
    if let Some(id) = session.filter(|&s| s > 0) {
        return blocking(move || dictation::cancel(id)).await?;
    }
    blocking(listen::dictate_cancel).await?
}

/// Reads text aloud with the system's voice, for platforms whose web view has none (Android).
#[tauri::command]
async fn speak_text(text: String, language: Option<String>) -> Result<(), String> {
    let p = prefs::load();
    let (rate, pitch) = (p.read_speed.unwrap_or(1.0), p.read_pitch.unwrap_or(1.0));
    let language = language.unwrap_or_default();
    blocking(move || recorder::speak(&text, rate, pitch, &language)).await?
}

#[tauri::command]
async fn stop_speaking() -> Result<(), String> {
    blocking(recorder::stop_speaking).await?
}

/// A short vibration, where the device has one: on Android, when dictation stops by itself.
#[tauri::command]
async fn haptic() -> Result<(), String> {
    blocking(recorder::vibrate).await?
}

// Off the main thread, as on Android these call the capture plugin and wait for its answer.
#[tauri::command]
async fn probe_start(devices: Vec<String>) -> Result<(), String> {
    blocking(move || probe::start(devices)).await
}

#[tauri::command]
async fn probe_status() -> Result<Vec<probe::Probe>, String> {
    blocking(probe::status).await
}

#[tauri::command]
async fn probe_stop() -> Result<(), String> {
    blocking(probe::stop).await
}

#[tauri::command]
fn listen_mute(muted: bool) {
    listen::set_muted(muted);
}

/// While the voice panel waits for an answer, every phrase is transcribed, not only those after the wake word.
#[tauri::command]
fn listen_attentive(on: bool) {
    listen::set_attentive(on);
}

/// The access the app needs from Android, for the setup and Settings pages. Empty on Windows.
#[tauri::command]
async fn app_permissions() -> Result<serde_json::Value, String> {
    blocking(recorder::permissions).await?
}

#[tauri::command]
async fn request_app_permissions(ids: Vec<String>) -> Result<(), String> {
    blocking(move || recorder::request_permissions(&ids)).await?
}

#[tauri::command]
fn app_version() -> &'static str {
    update::current()
}

/// Whether a newer version has been released.
#[tauri::command]
async fn check_update() -> Result<update::Update, String> {
    blocking(update::check).await?
}

/// Gets the newer version. On Windows the installer starts and the app closes so it can be replaced; on the
/// Mac the disk image opens and the app closes, so the new copy can be dragged over it.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let update = blocking(update::check).await??;
    if !update.newer {
        return Err(format!("Version {} is the latest.", update.current));
    }
    let progress = app.clone();
    blocking(move || update::install(&update, |f| drop(progress.emit("update-progress", f)))).await??;
    if cfg!(any(windows, target_os = "macos")) {
        app.exit(0);
    }
    Ok(())
}

/// Opens one of the app's own web pages, such as what changed in a release, or sherpa-onnx's collection of
/// voices to add.
#[tauri::command]
async fn open_link(url: String) -> Result<(), String> {
    let allowed = [update::SITE, "https://github.com/TeamVP/Superteem-Voice", "https://k2-fsa.github.io/sherpa/onnx/tts/"];
    if !allowed.iter().any(|a| url.starts_with(a)) {
        return Err("Only the app's own pages can be opened.".into());
    }
    blocking(move || recorder::open_url(&url)).await?
}

#[tauri::command]
async fn open_app_settings() -> Result<(), String> {
    blocking(recorder::open_app_settings).await?
}

/// Why the dictation microphone hears nothing, when the system is the reason.
#[tauri::command]
async fn dictation_problem() -> Result<Option<String>, String> {
    blocking(listen::problem).await
}

/// Off the UI thread, as on Android it asks the capture service.
#[tauri::command]
async fn listen_level() -> Result<f32, String> {
    blocking(listen::level).await
}

#[tauri::command]
fn delete_session(state: State<AppState>, id: String) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    if state.processing.lock().unwrap().as_deref() == Some(id.as_str()) {
        return Err("This session is being processed.".into());
    }
    state.queue.lock().unwrap().retain(|q| q != &id);
    library::delete(&id)
}

/// Deletes a meeting's audio for good and keeps its transcript. Returns the bytes freed.
#[tauri::command]
fn compact_session(state: State<AppState>, id: String) -> Result<u64, String> {
    let _guard = state.edit.lock().unwrap();
    let dir = library::session_dir(&id)?;
    if state.recording.lock().unwrap().as_ref().is_some_and(|r| r.dir() == dir) {
        return Err("This meeting is being recorded.".into());
    }
    if is_queued(&state, &id) {
        return Err("This meeting is being processed.".into());
    }
    library::compact(&dir)
}

#[tauri::command]
fn open_folder(id: Option<String>) -> Result<(), String> {
    let dir = match id {
        Some(id) => library::session_dir(&id)?,
        None => library::root(),
    };
    shell::open(dir)
}

// ---------- Analyses

/// A job whose template is this and a language code translates the item into that language.
const TRANSLATE: &str = "translate:";

/// One analysis or translation waiting to run, or running.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
struct Job {
    kind: String,
    id: String,
    template: String,
}

#[derive(Serialize, Clone)]
struct AnalysisQueue {
    current: Option<Job>,
    waiting: Vec<Job>,
}

fn analysis_queue_state(state: &AppState) -> AnalysisQueue {
    AnalysisQueue { current: state.analysing.lock().unwrap().clone(), waiting: state.analysis_queue.lock().unwrap().iter().cloned().collect() }
}

#[derive(Serialize, Clone)]
struct AnalysisProgress {
    #[serde(flatten)]
    job: Job,
    /// What it's doing: "Analysing", "Translating".
    step: String,
    part: usize,
    parts: usize,
}

#[derive(Serialize, Clone)]
struct Analysed {
    #[serde(flatten)]
    job: Job,
    error: Option<String>,
}

/// The model analyses run on: a local server when one is chosen, else the chosen model file here.
fn analysis_engine(cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<(Box<dyn stv_core::analysis::Model>, analysis::Engine), String> {
    let p = prefs::load();
    if !p.llm_server.trim().is_empty() {
        let server = llm::Endpoint::new(&p.llm_server, &p.llm_server_model, cancel)?;
        let engine = analysis::Engine { kind: "server".into(), model: server.model.clone(), host: Some(server.host()) };
        return Ok((Box::new(server), engine));
    }
    let path = llm::chosen(&p.llm_model).ok_or("No language model is installed. Get one in Settings › Analysis.")?;
    let device = llm::device();
    let gpu = p.use_gpu.unwrap_or(true);
    // On a computer, in a separate process that can't reach the network (see isolate.rs).
    #[cfg(any(windows, target_os = "macos"))]
    let local = isolate::Isolated::load(&path, device.context, device.threads, gpu, cancel)?;
    #[cfg(not(any(windows, target_os = "macos")))]
    let local = llm::Local::load(&path, device.context, device.threads, gpu, cancel)?;
    let engine = analysis::Engine { kind: "local".into(), model: local.name.clone(), host: None };
    Ok((Box::new(local), engine))
}

/// Adds an analysis to the queue, starting the worker if it is idle. Analyses wait while anything is
/// recorded or processed, since speech to text needs the processor and the memory, and the model is let go
/// once the queue is empty.
fn enqueue_analysis<R: Runtime>(app: &AppHandle<R>, job: Job) {
    let state = app.state::<AppState>();
    if state.analysing.lock().unwrap().as_ref() == Some(&job) || state.analysis_queue.lock().unwrap().contains(&job) {
        return;
    }
    let start_worker = {
        let mut queue = state.analysis_queue.lock().unwrap();
        queue.push_back(job);
        state.analysing.lock().unwrap().is_none() && !state.analysis_worker.swap(true, std::sync::atomic::Ordering::SeqCst)
    };
    let _ = app.emit("analysis-queue", analysis_queue_state(&state));
    if !start_worker {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut engine: Option<(Box<dyn stv_core::analysis::Model>, analysis::Engine)> = None;
        loop {
            while state.busy() || state.processing.lock().unwrap().is_some() {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            let next = {
                let mut queue = state.analysis_queue.lock().unwrap();
                let next = queue.pop_front();
                *state.analysing.lock().unwrap() = next.clone();
                if next.is_none() {
                    state.analysis_worker.store(false, std::sync::atomic::Ordering::SeqCst);
                }
                next
            };
            let _ = app.emit("analysis-queue", analysis_queue_state(&state));
            let Some(job) = next else { break };
            state.analysis_cancel.store(false, std::sync::atomic::Ordering::SeqCst);
            let outcome = (|| {
                if engine.is_none() {
                    engine = Some(analysis_engine(state.analysis_cancel.clone())?);
                }
                let (model, info) = engine.as_mut().expect("just loaded");
                let item = analysis::Item::new(&job.kind, &job.id)?;
                let report = |step: &str, part, parts| drop(app.emit("analysing", AnalysisProgress { job: job.clone(), step: step.into(), part, parts }));
                if let Some(language) = job.template.strip_prefix(TRANSLATE) {
                    translations::update(&item, language, model.as_mut(), &info.model, |k, n| report("Translating", k, n))?;
                    return Ok(());
                }
                for entry in analysis::make(&item, &job.template, model.as_mut(), info.clone(), report)? {
                    let _guard = state.edit.lock().unwrap();
                    analysis::put(&item, entry)?;
                }
                Ok(())
            })();
            let _ = app.emit("analysed", Analysed { job, error: outcome.err() });
        }
        *state.analysing.lock().unwrap() = None;
    });
}

#[derive(Serialize)]
struct AnalysesView {
    entries: Vec<analysis::EntryView>,
    templates: Vec<analysis::TemplateInfo>,
    queue: AnalysisQueue,
    /// A model is installed here, or a server is chosen.
    ready: bool,
    /// Template ids offered first, in order.
    starred: Vec<String>,
    /// Starred templates for the other kind of page, shown in the menu but not usable here.
    elsewhere: Vec<analysis::TemplateInfo>,
}

#[tauri::command]
fn get_analyses(state: State<AppState>, kind: String, id: String) -> Result<AnalysesView, String> {
    let item = analysis::Item::new(&kind, &id)?;
    let p = prefs::load();
    let starred = p.starred();
    let (templates, elsewhere): (Vec<_>, Vec<_>) = analysis::templates()
        .into_iter()
        .filter(|t| t.errors.is_empty())
        .map(|t| analysis::TemplateInfo { template: None, ..t })
        .partition(|t| t.applies_to.contains(&kind));
    Ok(AnalysesView {
        entries: analysis::view(&item),
        templates,
        queue: analysis_queue_state(&state),
        ready: !p.llm_server.trim().is_empty() || !llm::installed().is_empty(),
        elsewhere: elsewhere.into_iter().filter(|t| starred.contains(&t.id)).collect(),
        starred,
    })
}

#[tauri::command]
fn analyse<R: Runtime>(app: AppHandle<R>, kind: String, id: String, template: String) -> Result<(), String> {
    analysis::Item::new(&kind, &id)?;
    let (t, language) = analysis::split_key(&template);
    analysis::template(t)?;
    if language.is_some_and(|l| !stv_core::lang::is_known(l)) {
        return Err("That language isn't one Voice offers yet.".into());
    }
    enqueue_analysis(&app, Job { kind, id, template });
    Ok(())
}

#[tauri::command]
fn cancel_analysis<R: Runtime>(app: AppHandle<R>, kind: String, id: String, template: String) {
    let state = app.state::<AppState>();
    let job = Job { kind, id, template };
    state.analysis_queue.lock().unwrap().retain(|j| j != &job);
    if state.analysing.lock().unwrap().as_ref() == Some(&job) {
        state.analysis_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let _ = app.emit("analysis-queue", analysis_queue_state(&state));
}

#[tauri::command]
fn edit_analysis(state: State<AppState>, kind: String, id: String, template: String, edit: analysis::Edit) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    analysis::edit(&analysis::Item::new(&kind, &id)?, &template, edit)
}

#[tauri::command]
fn remove_analysis(state: State<AppState>, kind: String, id: String, template: String) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    analysis::remove(&analysis::Item::new(&kind, &id)?, &template)
}

#[tauri::command]
fn move_analysis(state: State<AppState>, kind: String, id: String, template: String, by: i32) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    analysis::shift(&analysis::Item::new(&kind, &id)?, &template, by)
}

/// Sets the folder exports go to; None goes back to the Exports folder beside the meetings.
#[tauri::command]
fn set_export_dir(path: Option<String>) -> Result<String, String> {
    if let Some(p) = path.as_deref() {
        if !Path::new(p).is_dir() {
            return Err("That folder doesn't exist.".into());
        }
    }
    prefs::update(|prefs| prefs.export_dir = path.unwrap_or_default())?;
    Ok(exports::dir().to_string_lossy().into_owned())
}

#[tauri::command]
fn export_dir() -> String {
    exports::dir().to_string_lossy().into_owned()
}

#[tauri::command]
fn tts_status() -> tts::Status {
    tts::status()
}

#[derive(Serialize, Clone)]
struct TtsDownload {
    id: String,
    fraction: f32,
}

#[tauri::command]
async fn download_tts<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let handle = app.clone();
    blocking(move || {
        let voice = id.clone();
        tts::download(&id, |fraction| drop(handle.emit("tts-download", TtsDownload { id: voice.clone(), fraction })))
    })
    .await?
}

/// Adds a voice the person chose from sherpa-onnx's collection; progress goes out as "tts-download".
#[tauri::command]
async fn add_tts_voice<R: Runtime>(app: AppHandle<R>, voice: tts::NewVoice) -> Result<tts::Voice, String> {
    let handle = app.clone();
    blocking(move || tts::add(voice, |fraction| drop(handle.emit("tts-download", TtsDownload { id: "new".into(), fraction })))).await?
}

#[tauri::command]
fn cancel_tts_download() {
    tts::cancel_download();
}

#[tauri::command]
fn delete_tts(id: String) -> Result<(), String> {
    tts::delete(&id)?;
    if !tts::voices().iter().any(tts::ready) {
        prefs::update(|p| p.natural_voice = false)?;
    }
    Ok(())
}

/// Optimising for this device: the plan for the person's languages, how it went last time, and whether
/// to suggest it (once a note and a meeting have been made, when it hasn't been done for this device and
/// would change something).
#[derive(Serialize)]
struct OptimiseView {
    plan: optimise::Plan,
    last: Option<optimise::Measured>,
    stale: bool,
    suggest: bool,
    progress: optimise::Progress,
    current: AiNow,
}

/// What each job runs with now, for the AI page.
#[derive(Serialize)]
struct AiNow {
    speech: String,
    speech_ready: bool,
    analysis: Option<String>,
    server: Option<String>,
    gpu_on: bool,
    /// Each language, with what reads it aloud ("Alba", or None for the system's voice), how well it's
    /// transcribed, and whether analyses are written in it directly.
    languages: Vec<(String, Option<String>, stv_core::lang::Quality, bool)>,
    /// Space the models take, in bytes.
    storage: u64,
}

fn ai_now(languages: &[String]) -> AiNow {
    let p = prefs::load();
    let (speech, speech_ready) = transcribe::model_for(languages);
    let models = library::models_dir();
    let storage = std::fs::read_dir(&models).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum::<u64>()
        + llm::installed().iter().map(|i| i.size).sum::<u64>()
        + tts::status().voices.iter().filter(|v| v.ready).map(|v| v.voice.size).sum::<u64>();
    AiNow {
        speech: speech.name.to_string(),
        speech_ready,
        // The catalogue's name for it ("Qwen3.5 4B"), else the file's.
        analysis: llm::chosen(&p.llm_model).and_then(|f| {
            let file = f.file_name()?.to_string_lossy().into_owned();
            Some(llm::CATALOGUE.iter().find(|c| c.file == file).map_or_else(|| f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or(file.clone()), |c| c.name.to_string()))
        }),
        server: (!p.llm_server.trim().is_empty()).then(|| p.llm_server.clone()),
        gpu_on: p.use_gpu.unwrap_or(true),
        languages: languages
            .iter()
            .map(|l| {
                let info = stv_core::lang::get(l);
                (
                    l.clone(),
                    tts::for_language(&p, Some(l)).map(|v| v.name),
                    info.map_or(stv_core::lang::Quality::None, |x| x.speech),
                    info.is_some_and(|x| x.analysis == stv_core::lang::Analysis::Direct),
                )
            })
            .collect(),
        storage,
    }
}

#[tauri::command]
async fn optimise_plan() -> Result<OptimiseView, String> {
    blocking(|| {
        let p = prefs::load();
        let languages = p.languages();
        let profile = optimise::profile();
        let plan = optimise::plan(&languages, &profile, &optimise::installed(), &tts::voices());
        let stale = optimise::stale(&profile, &languages);
        let used = !notes::list("").is_empty() && library::list("", &[]).iter().any(|m| !m.needs_processing && !m.preview.is_empty());
        OptimiseView { suggest: stale && used && !p.optimise_dismissed && plan.worth_it(), stale, last: optimise::last(), current: ai_now(&languages), plan, progress: optimise::progress() }
    })
    .await
}

/// Does the chosen steps of a fresh plan, in the background; progress goes out as "optimise-progress".
#[tauri::command]
fn optimise_apply<R: Runtime>(app: AppHandle<R>, steps: Vec<(String, String)>) -> Result<(), String> {
    if optimise::progress().running {
        return Err("Voice is already being optimised.".into());
    }
    std::thread::spawn(move || {
        let languages = prefs::load().languages();
        let plan = optimise::plan(&languages, &optimise::profile(), &optimise::installed(), &tts::voices());
        let handle = app.clone();
        let _ = optimise::apply(&plan, &steps, |p| drop(handle.emit("optimise-progress", p.clone())));
        listen::forget_transcriber();
        let _ = app.emit("optimise-progress", optimise::progress());
    });
    Ok(())
}

#[tauri::command]
fn optimise_cancel() {
    optimise::cancel();
}

#[tauri::command]
fn optimise_dismiss() -> Result<(), String> {
    prefs::update(|p| p.optimise_dismissed = true).map(|_| ())
}

/// The natural voice offered first for `language`, to download.
#[tauri::command]
fn offered_voice(language: String) -> Option<tts::Voice> {
    tts::first_for(&language)
}

/// The natural voice that reads `language`, or None when the system's voice does.
#[tauri::command]
fn reading_voice(language: Option<String>) -> Option<tts::Voice> {
    tts::for_language(&prefs::load(), language.as_deref())
}

/// Speech for `text` in a natural voice (`voice`, else the main one) at the chosen speed, as a WAV for the
/// page to play.
#[tauri::command]
async fn natural_speak(text: String, voice: Option<String>) -> Result<tauri::ipc::Response, String> {
    let speed = prefs::load().read_speed.unwrap_or(1.0);
    let wav = blocking(move || match voice {
        Some(id) => tts::speak_as(&tts::find_voice(&id)?, &text, speed),
        None => tts::speak(&text, speed),
    })
    .await??;
    Ok(tauri::ipc::Response::new(wav))
}

#[derive(Serialize)]
struct Languages {
    /// Every language that can be chosen: (code, name).
    all: Vec<(&'static str, &'static str)>,
    /// The same, with what works in each.
    catalogue: &'static [stv_core::lang::Language],
    chosen: Vec<String>,
    primary: String,
    /// What new notes and meetings start in; empty for the primary language.
    defaults: Vec<String>,
    translate_into: Vec<String>,
    auto_translate: bool,
    speech_model: String,
}

#[tauri::command]
fn get_languages() -> Languages {
    let p = prefs::load();
    Languages {
        all: stv_core::lang::LANGUAGES.iter().map(|l| (l.code, l.name)).collect(),
        catalogue: stv_core::lang::LANGUAGES,
        chosen: p.languages(),
        primary: p.primary_language(),
        defaults: p.new_item_languages(),
        translate_into: p.translate_targets(&[]),
        auto_translate: p.auto_translate(),
        speech_model: p.speech_model.clone(),
    }
}

/// The languages the person uses and the primary one, which must be among them.
#[tauri::command]
fn save_languages(languages: Vec<String>, primary: String) -> Result<Languages, String> {
    if !stv_core::lang::is_known(&primary) {
        return Err("That language isn't one Voice offers yet.".into());
    }
    let mut chosen: Vec<String> = Vec::new();
    for l in languages.into_iter().filter(|l| stv_core::lang::is_known(l)) {
        if !chosen.contains(&l) {
            chosen.push(l);
        }
    }
    if !chosen.contains(&primary) {
        chosen.insert(0, primary.clone());
    }
    prefs::update(|p| {
        // Languages the person no longer uses aren't defaults or translated into.
        p.default_languages.retain(|l| chosen.contains(l));
        p.translate_into.retain(|l| chosen.contains(l));
        p.languages = chosen;
        p.primary_language = primary;
    })?;
    Ok(get_languages())
}

/// What new items start in, what to translate into and whether to by itself, and the multilingual speech
/// model to use; each is left as it is when not given.
#[tauri::command]
fn save_language_settings(
    defaults: Option<Vec<String>>,
    translate_into: Option<Vec<String>>,
    auto_translate: Option<bool>,
    speech_model: Option<String>,
) -> Result<Languages, String> {
    if let Some(m) = speech_model.as_deref().filter(|m| !m.is_empty()) {
        transcribe::model(m).filter(|m| m.multilingual).ok_or("That speech model isn't one Voice knows.")?;
    }
    prefs::update(|p| {
        if let Some(d) = defaults {
            p.default_languages = stv_core::lang::clean_list(&d);
        }
        if let Some(t) = translate_into {
            p.translate_into = t.into_iter().filter(|l| stv_core::lang::is_known(l)).collect();
            p.translate_into.dedup();
        }
        if let Some(a) = auto_translate {
            p.auto_translate = Some(a);
        }
        if let Some(m) = speech_model {
            p.speech_model = m;
        }
    })?;
    Ok(get_languages())
}

#[derive(Serialize, Clone)]
struct SpeechModelView {
    #[serde(flatten)]
    model: &'static transcribe::SpeechModel,
    ready: bool,
    /// The multilingual model this device suits.
    suggested: bool,
}

fn speech_model_view(m: &'static transcribe::SpeechModel) -> SpeechModelView {
    SpeechModelView { model: m, ready: transcribe::is_ready(m), suggested: m.multilingual && m.id == transcribe::suggested_multilingual().id }
}

#[tauri::command]
fn speech_models() -> Vec<SpeechModelView> {
    transcribe::MODELS.iter().map(speech_model_view).collect()
}

#[derive(Serialize, Clone)]
struct SpeechDownload {
    id: String,
    fraction: f32,
}

static SPEECH_DOWNLOAD_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SPEECH_DOWNLOADING: Mutex<Option<String>> = Mutex::new(None);

/// Downloads a speech model; progress goes out as "speech-model-download". One at a time.
#[tauri::command]
async fn download_speech_model<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    {
        let mut busy = SPEECH_DOWNLOADING.lock().unwrap();
        if busy.is_some() {
            return Err("A speech model is already downloading.".into());
        }
        *busy = Some(id.clone());
    }
    SPEECH_DOWNLOAD_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    let handle = app.clone();
    let result = blocking(move || {
        let which = id.clone();
        transcribe::download(&id, &SPEECH_DOWNLOAD_CANCEL, |fraction| drop(handle.emit("speech-model-download", SpeechDownload { id: which.clone(), fraction })))
    })
    .await;
    *SPEECH_DOWNLOADING.lock().unwrap() = None;
    result?
}

#[tauri::command]
fn cancel_speech_download() {
    SPEECH_DOWNLOAD_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn delete_speech_model(id: String) -> Result<(), String> {
    transcribe::delete(&id)?;
    prefs::update(|p| {
        if p.speech_model == id {
            p.speech_model.clear();
        }
    })
    .map(|_| ())
}

/// What changed when an item's languages were set, and what the page should offer next.
#[derive(Serialize)]
struct ItemLanguages {
    /// As saved: empty for the primary language alone.
    languages: Vec<String>,
    /// The languages it's in, main one first.
    effective: Vec<String>,
    /// Languages added to the person's own because this item is in them.
    added: Vec<String>,
    /// The speech model these languages need.
    model: SpeechModelView,
    /// The languages differ from those it was transcribed in, and there's a transcript.
    changed: bool,
    /// It can be transcribed again in the new languages.
    can_retranscribe: bool,
    /// Why it can't, when it can't.
    reason: Option<String>,
}

/// Sets the languages a note or meeting is in, one or two, main one first. A language the person hasn't
/// chosen yet becomes one of theirs.
#[tauri::command]
fn set_item_languages(state: State<AppState>, kind: String, id: String, languages: Vec<String>) -> Result<ItemLanguages, String> {
    let languages = stv_core::lang::clean_list(&languages);
    if languages.is_empty() {
        return Err("Choose a language.".into());
    }
    let p = prefs::load();
    let saved = if languages.len() == 1 && languages[0] == p.primary_language() { Vec::new() } else { languages.clone() };
    let mine = p.languages();
    let added: Vec<String> = languages.iter().filter(|l| !mine.contains(l)).cloned().collect();
    if !added.is_empty() {
        prefs::update(|p| p.languages.extend(added.iter().cloned()))?;
    }
    let (model, _) = transcribe::model_for(&languages);
    let (changed, can_retranscribe, reason) = match kind.as_str() {
        "meeting" => {
            let dir = library::session_dir(&id)?;
            let before = p.effective(&library::load_meta(&dir).langs());
            edit_session(&state, &id, |meta, _| {
                meta.set_langs(saved.clone());
                Ok(())
            })?;
            let has_lines = !library::load_segments(&dir).is_empty();
            let compacted = library::load_meta(&dir).compacted;
            let reason = if compacted {
                Some("Its audio was deleted to save space, so it can't be transcribed again. New analyses and translations use the new languages.".to_string())
            } else {
                None
            };
            (before != languages && has_lines, !compacted, reason)
        }
        "note" => {
            let before = p.effective(&notes::load(&id)?.langs());
            notes::set_languages(&id, saved.clone())?;
            let reason = "Notes are written down as you dictate, so what's already here stays as it is. New dictation is heard in the new languages.";
            (before != languages, false, Some(reason.to_string()))
        }
        _ => return Err(format!("Unknown kind: {kind}")),
    };
    Ok(ItemLanguages { languages: saved, effective: languages, added, model: speech_model_view(model), changed, can_retranscribe, reason })
}

/// A translation of a note or meeting as the page shows it, and the languages it has translations in.
#[derive(Serialize)]
struct TranslationView {
    #[serde(flatten)]
    view: translations::View,
    available: Vec<String>,
}

#[tauri::command]
fn get_translation(kind: String, id: String, language: String) -> Result<TranslationView, String> {
    let item = analysis::Item::new(&kind, &id)?;
    Ok(TranslationView { view: translations::view(&item, &language)?, available: translations::languages(&item) })
}

/// Translates a note or meeting into `language`, in turn with analyses.
#[tauri::command]
fn translate_item<R: Runtime>(app: AppHandle<R>, kind: String, id: String, language: String) -> Result<(), String> {
    analysis::Item::new(&kind, &id)?;
    if !stv_core::lang::is_known(&language) {
        return Err("That language isn't one Voice offers yet.".into());
    }
    let p = prefs::load();
    if p.llm_server.trim().is_empty() && llm::installed().is_empty() {
        return Err("Translating needs a language model. Get one in Settings › Analysis.".into());
    }
    enqueue_analysis(&app, Job { kind, id, template: format!("{TRANSLATE}{language}") });
    Ok(())
}

#[tauri::command]
fn edit_translation(state: State<AppState>, kind: String, id: String, language: String, line: String, text: String) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    translations::edit(&analysis::Item::new(&kind, &id)?, &language, &line, &text)
}

#[tauri::command]
fn delete_translation(state: State<AppState>, kind: String, id: String, language: String) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    translations::delete(&analysis::Item::new(&kind, &id)?, &language)
}

#[tauri::command]
async fn export_translation(kind: String, id: String, language: String, format: String) -> Result<String, String> {
    blocking(move || {
        let item = analysis::Item::new(&kind, &id)?;
        exports::export_translation(&item, &language, &format).map(|p| p.to_string_lossy().into_owned())
    })
    .await?
}

/// Reading aloud and model settings; each is left as it is when not given.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn save_voice_settings(
    natural: Option<bool>,
    voice: Option<String>,
    speed: Option<f32>,
    pitch: Option<f32>,
    system_voice: Option<String>,
    read_speakers: Option<bool>,
    use_gpu: Option<bool>,
    voice_for: Option<(String, String)>,
) -> Result<(), String> {
    if let Some((lang, v)) = &voice_for {
        prefs::check_language(lang)?;
        if !(v.is_empty() || v == "system" || tts::voices().iter().any(|x| x.id == *v)) {
            return Err("That voice isn't in the list.".into());
        }
    }
    if let Some(v) = &voice {
        if !tts::voices().iter().any(|x| x.id == *v) {
            return Err("That voice isn't one Voice offers.".into());
        }
    }
    prefs::update(|p| {
        if let Some(v) = voice {
            p.tts_voice = v;
        }
        if let Some(n) = natural {
            p.natural_voice = n && tts::ready(&tts::chosen_in(p, &tts::voices()));
        }
        if let Some(s) = speed {
            p.read_speed = Some(s.clamp(0.5, 2.0));
        }
        if let Some(s) = pitch {
            p.read_pitch = Some(s.clamp(0.5, 2.0));
        }
        if let Some(v) = system_voice {
            p.system_voice = v.chars().take(200).collect();
        }
        if let Some(r) = read_speakers {
            p.read_speakers = Some(r);
        }
        if let Some(g) = use_gpu {
            p.use_gpu = Some(g);
        }
        if let Some((lang, v)) = voice_for {
            if v.is_empty() {
                p.voice_for.remove(&lang);
            } else {
                p.voice_for.insert(lang, v);
            }
        }
    })
    .map(|_| ())
}

#[derive(Serialize)]
struct LlmStatus {
    catalogue: &'static [llm::Catalogued],
    installed: Vec<llm::Installed>,
    /// The file analyses run with.
    chosen: Option<String>,
    device: llm::Device,
    server: String,
    server_model: String,
    auto: bool,
    use_gpu: bool,
    downloading: Option<String>,
    folder: String,
}

#[tauri::command]
fn llm_status(state: State<AppState>) -> LlmStatus {
    let p = prefs::load();
    LlmStatus {
        catalogue: llm::CATALOGUE,
        installed: llm::installed(),
        chosen: llm::chosen(&p.llm_model).and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned())),
        device: llm::device(),
        auto: p.auto_analyse(),
        use_gpu: p.use_gpu.unwrap_or(true),
        server: p.llm_server,
        server_model: p.llm_server_model,
        downloading: state.llm_downloading.lock().unwrap().clone(),
        folder: llm::dir().to_string_lossy().into_owned(),
    }
}

#[derive(Serialize, Clone)]
struct LlmDownload {
    id: String,
    fraction: f32,
}

#[tauri::command]
async fn download_llm<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut downloading = state.llm_downloading.lock().unwrap();
        if downloading.is_some() {
            return Err("A model is already downloading.".into());
        }
        *downloading = Some(id.clone());
        state.llm_download_cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    let handle = app.clone();
    let result = blocking(move || {
        let state = handle.state::<AppState>();
        let cancel = state.llm_download_cancel.clone();
        llm::download(&id, &cancel, |fraction| drop(handle.emit("llm-download", LlmDownload { id: id.clone(), fraction })))
    })
    .await;
    *app.state::<AppState>().llm_downloading.lock().unwrap() = None;
    result?
}

#[tauri::command]
fn cancel_llm_download(state: State<AppState>) {
    state.llm_download_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn delete_llm(file: String) -> Result<(), String> {
    llm::delete(&file)
}

#[tauri::command]
fn save_analysis_settings(model: Option<String>, auto: Option<bool>, server: Option<String>, server_model: Option<String>) -> Result<(), String> {
    if let Some(s) = server.as_deref().filter(|s| !s.trim().is_empty()) {
        llm::Endpoint::new(s, "", Default::default())?;
    }
    prefs::update(|p| {
        if let Some(m) = model {
            p.llm_model = m;
        }
        if let Some(a) = auto {
            p.auto_analyse = Some(a);
        }
        if let Some(s) = server {
            p.llm_server = s.trim().to_string();
        }
        if let Some(m) = server_model {
            p.llm_server_model = m.trim().to_string();
        }
    })
    .map(|_| ())
}

/// Adds an audio track to a meeting. The body is the audio, 16 kHz mono 16-bit, decoded by the page; the
/// headers say which meeting (`x-id`), the track's name (`x-label`, URI-encoded) and how to place it
/// (`x-place`): "time" at the time of day it started (`x-started-at`, Unix seconds), fine-tuned by its sound
/// when `x-refine` is 1; "sound" by matching it anywhere; or "at" a point in the meeting (`x-offset`, seconds).
#[tauri::command]
async fn import_track<R: Runtime>(app: AppHandle<R>, request: tauri::ipc::Request<'_>) -> Result<imports::Import, String> {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let id = header("x-id");
    let label = percent_decode(&header("x-label"));
    let place = match header("x-place").as_str() {
        "time" => imports::Place::Time {
            started_at: header("x-started-at").parse().map_err(|_| "When it started wasn't sent.")?,
            refine: header("x-refine") == "1",
        },
        "sound" => imports::Place::Sound,
        _ => imports::Place::At(header("x-offset").parse().unwrap_or(0.0)),
    };
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("No audio was sent.".into()) };
    let pcm: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let dir = library::session_dir(&id)?;
    if library::load_meta(&dir).compacted {
        return Err(process::COMPACTED.into());
    }
    let import = blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.edit.lock().unwrap();
        imports::add(&dir, &label, &pcm, place)
    })
    .await??;
    Ok(import)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Finds where an imported track fits by matching its sound to the recorded tracks: `near` where it is now,
/// or anywhere.
#[tauri::command]
async fn sync_track(state: State<'_, AppState>, id: String, name: String, near: Option<bool>) -> Result<imports::Import, String> {
    let dir = library::session_dir(&id)?;
    let _guard = state.edit.lock().unwrap();
    imports::sync(&dir, &name, near.unwrap_or(true))
}

/// Places an imported track by the time of day it started (Unix seconds).
#[tauri::command]
fn set_track_start(state: State<AppState>, id: String, name: String, started_at: f64) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    imports::set_start_time(&library::session_dir(&id)?, &name, started_at)
}

#[tauri::command]
fn set_track_offset(state: State<AppState>, id: String, name: String, offset_s: f64) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    imports::set_offset(&library::session_dir(&id)?, &name, offset_s)
}

#[tauri::command]
fn remove_track(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    let _guard = state.edit.lock().unwrap();
    imports::remove(&library::session_dir(&id)?, &name)
}

fn open_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    shell::open(dir)
}

#[tauri::command]
fn open_llm_folder() -> Result<(), String> {
    open_dir(&llm::dir())
}

#[tauri::command]
fn open_templates_folder() -> Result<(), String> {
    open_dir(&analysis::templates_dir())
}

#[derive(Serialize)]
struct TemplateList {
    templates: Vec<analysis::TemplateInfo>,
    starred: Vec<String>,
}

#[tauri::command]
fn list_templates() -> TemplateList {
    TemplateList { templates: analysis::templates(), starred: prefs::load().starred() }
}

/// Stars or unstars a template, so it's offered first on meetings and notes.
#[tauri::command]
fn star_template(id: String, starred: bool) -> Result<(), String> {
    prefs::update(|p| {
        let mut list = p.starred();
        list.retain(|t| t != &id);
        if starred {
            list.push(id);
        }
        p.starred_templates = Some(list);
    })
    .map(|_| ())
}

#[derive(Serialize)]
struct TemplateSource {
    source: String,
    file: Option<String>,
}

#[tauri::command]
fn template_source(id: String) -> Result<TemplateSource, String> {
    analysis::template_source(&id).map(|(source, file)| TemplateSource { source, file })
}

#[derive(Serialize)]
struct SavedTemplate {
    file: String,
    id: String,
    name: String,
}

/// Saves a template. `was` is the id it had: when the edit changed it, its star moves with it, so it stays
/// offered first on meetings and notes.
#[tauri::command]
fn save_template(source: String, file: Option<String>, was: Option<String>) -> Result<SavedTemplate, String> {
    let file = analysis::save_template(&source, file.as_deref())?;
    let t = stv_core::analysis::parse(&source).map_err(|e| e.join("
"))?;
    if let Some(was) = was.filter(|w| *w != t.id) {
        prefs::update(|p| {
            let list = p.starred();
            if list.contains(&was) {
                p.starred_templates = Some(list.into_iter().map(|s| if s == was { t.id.clone() } else { s }).collect());
            }
        })?;
    }
    Ok(SavedTemplate { file, id: t.id, name: t.name })
}

#[tauri::command]
fn delete_template(file: String) -> Result<(), String> {
    analysis::delete_template(&file)
}

#[tauri::command]
fn new_template(name: String, headings: Vec<(String, stv_core::analysis::HeadingKind)>) -> Result<String, String> {
    let id = analysis::new_template(&name, &headings)?;
    // Someone who makes a template means to use it: it's starred, so it's offered first.
    star_template(id.clone(), true)?;
    Ok(id)
}

fn capture_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("capture")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api.register_android_plugin("io.superteem.voice", "CapturePlugin")?;
                // The app runs on Wry, so this cast always holds on Android.
                let any: Box<dyn std::any::Any> = Box::new(handle);
                if let Ok(handle) = any.downcast::<tauri::plugin::PluginHandle<tauri::Wry>>() {
                    recorder::set_plugin(*handle);
                }
            }
            Ok(())
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // The iPhone opens web pages and its Settings app through the system, which this plugin asks.
    #[cfg(target_os = "ios")]
    let builder = builder.plugin(tauri_plugin_opener::init());
    builder
        .plugin(capture_plugin())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // On Windows the app opens filling the screen.
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.maximize();
            }
            if cfg!(target_os = "android") {
                let data = app.path().app_data_dir()?;
                library::init(data.join("sessions"), data);
            }
            // Meetings and notes go in the app's Documents folder, which the Files app shows under On My
            // iPhone. The model, voice prints and preferences stay in its private Application Support.
            #[cfg(target_os = "ios")]
            {
                library::init(app.path().document_dir()?, app.path().app_data_dir()?);
                recorder::set_app(app.handle().clone());
                if let Err(e) = recorder::start_audio_session() {
                    eprintln!("The audio session didn't start: {e}");
                }
            }
            let handle = app.handle().clone();
            let phrases = handle.clone();
            let practice = app.handle().clone();
            listen::on_practice(move |p| drop(practice.emit("wake-practice", p)));
            listen::init(move |text| drop(handle.emit("heard", text)), move |heard| drop(phrases.emit("keyphrase", heard)));
            // After the library is set up (above, on phones), so the vocabulary is in the right place.
            vocabulary::add_examples();
            std::thread::spawn(notes::purge_old_audio);
            let root = library::root();
            std::fs::create_dir_all(&root)?;
            app.asset_protocol_scope().allow_directory(&root, true)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform,
            list_devices,
            model_ready,
            download_model,
            start_recording,
            recording_status,
            stop_recording,
            process_session,
            list_sessions,
            get_session,
            rename_session,
            set_session_status,
            rename_speaker,
            add_speaker,
            update_segment,
            move_lines,
            merge_speakers,
            link_speaker,
            save_speaker_print,
            list_voiceprints,
            rename_voiceprint,
            delete_voiceprint,
            start_enrolment,
            stop_enrolment,
            listen_attentive,
            list_keyphrases,
            start_phrase_training,
            stop_phrase_training,
            clear_keyphrase,
            save_keyphrase_settings,
            get_prefs,
            save_prefs,
            delete_session,
            compact_session,
            open_folder,
            start_wake_practice,
            stop_wake_practice,
            model_info,
            open_models_folder,
            save_markers,
            edit_marker,
            keep_segment,
            export_session,
            export_note,
            reveal_export,
            save_consent,
            processing_queue,
            split_segment,
            join_segments,
            list_notes,
            get_note,
            save_note,
            delete_note,
            set_wake,
            dictate_start,
            dictate_stop,
            dictate_cancel,
            list_vocabulary,
            save_vocab_term,
            delete_vocab_term,
            toggle_fix,
            fix_past_transcripts,
            vocab_test_stop,
            probe_start,
            archive_note,
            set_note_status,
            probe_status,
            probe_stop,
            listen_level,
            listen_mute,
            app_permissions,
            request_app_permissions,
            open_app_settings,
            check_update,
            app_version,
            install_update,
            open_link,
            dictation_problem,
            haptic,
            tts_status,
            set_export_dir,
            export_dir,
            download_tts,
            cancel_tts_download,
            add_tts_voice,
            reading_voice,
            get_languages,
            save_languages,
            set_line_language,
            save_language_settings,
            set_item_languages,
            speech_models,
            download_speech_model,
            cancel_speech_download,
            delete_speech_model,
            optimise_plan,
            offered_voice,
            optimise_apply,
            optimise_cancel,
            optimise_dismiss,
            get_translation,
            translate_item,
            edit_translation,
            delete_translation,
            export_translation,
            delete_tts,
            natural_speak,
            save_voice_settings,
            import_track,
            sync_track,
            set_track_start,
            set_track_offset,
            remove_track,
            speak_text,
            stop_speaking,
            get_analyses,
            analyse,
            cancel_analysis,
            edit_analysis,
            remove_analysis,
            move_analysis,
            llm_status,
            download_llm,
            cancel_llm_download,
            delete_llm,
            save_analysis_settings,
            open_llm_folder,
            open_templates_folder,
            list_templates,
            star_template,
            template_source,
            save_template,
            delete_template,
            new_template,
        ])
        .run(tauri::generate_context!())
        .expect("the app failed to start");
}
