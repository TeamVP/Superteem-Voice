//! Voice by Superteem, shared by Windows and Android.

#[cfg(not(target_os = "android"))]
pub mod capture;
pub mod keyphrases;
pub mod library;
pub mod listen;
pub mod notes;
pub mod prefs;
pub mod probe;
pub mod process;
pub mod recorder;
pub mod transcribe;
pub mod update;
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
        let _ = app.emit("processed", Processed { id, summary });
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
    if cfg!(target_os = "android") {
        return Err("Folders can't be opened on Android.".into());
    }
    let dir = library::models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
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
    library::session_dir(&id)?;
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
            s.text = text.trim().to_string();
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
        let split_s = s.start_s + (s.end_s - s.start_s) * at as f64 / chars.len() as f64;
        let mut second = s.clone();
        s.text = head;
        s.end_s = split_s;
        s.edited = true;
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
        // Lines between them that hold no words are hidden on the page, so they go too.
        if j <= i || segments[i + 1..j].iter().any(|s| !process::is_silence(&s.text)) {
            return Err("Only neighbouring lines can be joined.".into());
        }
        if let Some(why) = join_blocker(&segments[i], &segments[j]) {
            return Err(why.into());
        }
        let next = segments.remove(j);
        segments.drain(i + 1..j);
        let s = &mut segments[i];
        s.text = format!("{} {}", s.text.trim(), next.text.trim()).trim().to_string();
        s.end_s = s.end_s.max(next.end_s);
        s.confidence = s.confidence.min(next.confidence);
        s.edited = true;
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
    notes::delete(&id)
}

#[tauri::command]
async fn set_wake(on: bool) -> Result<(), String> {
    blocking(move || listen::set_wake(on)).await?
}

#[tauri::command]
async fn dictate_start() -> Result<(), String> {
    blocking(listen::dictate_start).await?
}

#[tauri::command]
async fn dictate_stop(note: Option<String>) -> Result<String, String> {
    blocking(move || {
        let (words, pcm) = listen::dictate_stop()?;
        // The audio is kept only when the note asks, and only once the words are written down.
        if let Some(id) = note.filter(|_| !words.is_empty()) {
            notes::keep_dictation(&id, &pcm, process::RATE)?;
        }
        Ok(words)
    })
    .await?
}

#[tauri::command]
async fn dictate_cancel() -> Result<(), String> {
    blocking(listen::dictate_cancel).await?
}

#[tauri::command]
fn probe_start(devices: Vec<String>) {
    probe::start(devices);
}

#[tauri::command]
fn probe_status() -> Vec<probe::Probe> {
    probe::status()
}

#[tauri::command]
fn probe_stop() {
    probe::stop();
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

/// Gets the newer version. On Windows the installer starts and the app closes so it can be replaced.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let update = blocking(update::check).await??;
    if !update.newer {
        return Err(format!("Version {} is the latest.", update.current));
    }
    let progress = app.clone();
    blocking(move || update::install(&update, |f| drop(progress.emit("update-progress", f)))).await??;
    if cfg!(not(target_os = "android")) {
        app.exit(0);
    }
    Ok(())
}

/// Opens one of the app's own web pages, such as what changed in a release.
#[tauri::command]
async fn open_link(url: String) -> Result<(), String> {
    if !url.starts_with(update::SITE) && !url.starts_with("https://github.com/TeamVP/Superteem-Voice") {
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

#[tauri::command]
fn open_folder(id: Option<String>) -> Result<(), String> {
    let dir = match id {
        Some(id) => library::session_dir(&id)?,
        None => library::root(),
    };
    if cfg!(target_os = "android") {
        return Err("Folders can't be opened on Android.".into());
    }
    std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
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
    tauri::Builder::default()
        .plugin(capture_plugin())
        .manage(AppState::default())
        .setup(|app| {
            if cfg!(target_os = "android") {
                let data = app.path().app_data_dir()?;
                library::init(data.join("sessions"), data);
            }
            let handle = app.handle().clone();
            let phrases = handle.clone();
            let practice = app.handle().clone();
            listen::on_practice(move |p| drop(practice.emit("wake-practice", p)));
            listen::init(move |text| drop(handle.emit("heard", text)), move |heard| drop(phrases.emit("keyphrase", heard)));
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
            open_folder,
            start_wake_practice,
            stop_wake_practice,
            model_info,
            open_models_folder,
            save_markers,
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
        ])
        .run(tauri::generate_context!())
        .expect("the app failed to start");
}
