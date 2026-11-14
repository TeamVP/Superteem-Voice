//! Saved sessions: the editable transcript, names, search and deletion.
//! `transcript.json` is the source of truth; the text exports are regenerated from it on every edit.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use stv_core::export::{self, Line};

pub const UNKNOWN: &str = "unknown";
const TRANSCRIPT: &str = "transcript.json";
pub const META: &str = "meta.json";
/// Below this, the editor highlights a line for checking.
pub const LOW_CONFIDENCE: f32 = 0.5;

/// Where sessions and models live, set once at startup because Android only knows them from the app.
struct Paths {
    sessions: PathBuf,
    data: PathBuf,
}
static PATHS: OnceLock<Paths> = OnceLock::new();

/// `data` holds this device's own files: the speech model, voice prints and preferences.
pub fn init(sessions: PathBuf, data: PathBuf) {
    let _ = PATHS.set(Paths { sessions, data });
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Defaults to the computer's own locations when `init` was not called, as on Windows and the Mac and in
/// tests and the self-test: Documents\Superteem Voice on Windows, ~/Documents/Superteem Voice on the Mac.
pub fn root() -> PathBuf {
    match PATHS.get() {
        Some(p) => p.sessions.clone(),
        None if cfg!(windows) => env_dir("USERPROFILE").join("Documents").join("Superteem Voice"),
        None => env_dir("HOME").join("Documents").join("Superteem Voice"),
    }
}

pub fn data_dir() -> PathBuf {
    match PATHS.get() {
        Some(p) => p.data.clone(),
        None if cfg!(windows) => env_dir("LOCALAPPDATA").join("Superteem Voice"),
        None => env_dir("HOME").join("Library").join("Application Support").join("Superteem Voice"),
    }
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn new_session_dir() -> std::io::Result<PathBuf> {
    let dir = root().join(format!("session-{}", now()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Session ids come from the UI, so only a bare folder name inside the library is accepted.
pub fn session_dir(id: &str) -> Result<PathBuf, String> {
    let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let dir = root().join(id);
    if plain && dir.is_dir() {
        Ok(dir)
    } else {
        Err("That session no longer exists.".into())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub id: usize,
    /// A stream name for a wearer, or UNKNOWN for a guest.
    pub speaker: String,
    /// The stream whose audio this line was transcribed from, used for playback.
    pub stream: String,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
    pub confidence: f32,
    #[serde(default)]
    pub edited: bool,
    /// Someone else is speaking at the same time.
    #[serde(default)]
    pub overlap: bool,
    /// The line that says the same thing, heard more clearly on another microphone. A line with this is
    /// left out of the script and greyed out in the tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo_of: Option<usize>,
    /// Other microphones that heard this line too, greyed out in their tracks over its time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_on: Vec<String>,
    /// The clips a line was stitched together from, when it is more than one: (start_s, end_s) each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<(f64, f64)>,
    /// Words the vocabulary changed or suggests changing, so each can be undone or accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<stv_core::vocab::Fix>,
    /// The language it was heard in, when the meeting is in two; empty when it's in the meeting's main one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
}

impl Segment {
    /// The stretches of audio this line was heard in.
    pub fn spans(&self) -> Vec<(f64, f64)> {
        if self.parts.is_empty() { vec![(self.start_s, self.end_s)] } else { self.parts.clone() }
    }

    /// Part of the script: not a doubled-up copy of another line.
    pub fn in_script(&self) -> bool {
        self.echo_of.is_none()
    }
}

/// Where a track's sound came from, kept with the meeting so a silent or odd track can be explained.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Track {
    /// "microphone", "app", "output" or "pin".
    pub kind: String,
    /// The name shown: "Microsoft Teams", or the microphone's own name.
    pub label: String,
    /// The full name it was chosen by: "Sound from Microsoft Teams (ms-teams.exe)".
    pub source: String,
    /// The program recorded, for an app.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub program: String,
    /// 20 ms frames recorded, and frames lost.
    pub frames: u64,
    #[serde(default)]
    pub lost: u64,
    /// The loudest it got, 0 to 1; 0 means the track is silent.
    #[serde(default)]
    pub peak: f32,
    /// What happened while recording, each with the time: "0:02 Recording Headset Earphone instead".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    pub created: u64,
    #[serde(default)]
    pub duration_s: f64,
    /// Speaker key to the name shown, set by renaming in the editor.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Speaker key to the voice print it was recognised as or linked to.
    #[serde(default)]
    pub prints: BTreeMap<String, PrintLink>,
    /// Speaker key to the voice prints it might be, most likely first, when recognition wasn't sure
    /// enough to name them and asks the person instead.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub suggested: BTreeMap<String, Vec<PrintLink>>,
    /// Moments marked while recording, with any note typed or spoken there.
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// How everyone agreed to the recording, asked once it had started.
    #[serde(default)]
    pub consent: Option<Consent>,
    /// Where the meeting is in the person's own workflow; empty when not set.
    #[serde(default)]
    pub status: String,
    /// The audio was deleted to save space; only the transcript is left, so it can't be processed again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compacted: bool,
    /// The inputs recorded, in order: input1 is the first. Names the tracks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// What each track was recorded from and how, by stream ("input1", "pin1").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<String, Track>,
    /// The language older versions kept, before a meeting could be in two; read into `languages`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// The languages it's in, as codes ("cy", "en"), the main one first; empty for the person's primary
    /// language.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Tracks taken out since it was last transcribed, by their names then; the transcript still has
    /// their lines until it is transcribed again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks_removed: Vec<String>,
    /// Tracks moved or locked to a speaker since it was last transcribed, by their names then.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks_changed: Vec<String>,
    /// Recorded tracks moved on the timeline by hand: seconds later (or earlier, when negative) than they
    /// were recorded, as for a call app's delay.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_offsets: BTreeMap<String, f64>,
    /// Tracks where everything heard is the track's own speaker, as when each person has their own
    /// microphone or call track.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked: Vec<String>,
    /// Where each track's audio file was put on the timeline when it was last transcribed: a recorded
    /// track's move, or an import's start. A track moved since plays that much later until it's
    /// transcribed again, so a move can be heard at once.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audio_offsets: BTreeMap<String, f64>,
    /// How each track is lifted when it's played back, by stream: for one that was recorded too quietly.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_levels: BTreeMap<String, TrackLevel>,
    /// Where the person changed the transcript mix by hand, by stream: stretches of a track's own recording
    /// played or muted whatever the transcript says. They don't overlap.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mix_edits: BTreeMap<String, Vec<MixEdit>>,
}

/// A stretch of a track (seconds into its own recording) played or muted by hand in the transcript mix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MixEdit {
    pub from_s: f64,
    pub to_s: f64,
    /// Played; otherwise muted.
    pub play: bool,
}

/// A track lifted for playback: louder by `boost_db`, and with `compress` its loud and quiet parts brought
/// closer together and its peaks held back, so it can be lifted further without distorting.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrackLevel {
    #[serde(default)]
    pub boost_db: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compress: bool,
    /// Bottom end cut below this many hertz; 0 for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub low_cut_hz: f64,
    /// Top end cut above this many hertz; 0 for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub high_cut_hz: f64,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// The most a track can be lifted, in decibels.
pub const MAX_BOOST_DB: f64 = 24.0;

/// Workflow states a note or meeting can be in. Empty is the default: no status yet.
pub const STATUSES: [&str; 5] = ["", "in_progress", "ready", "exported", "done"];

pub fn check_status(status: &str) -> Result<String, String> {
    STATUSES.contains(&status).then(|| status.to_string()).ok_or_else(|| format!("Unknown status: {status}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Consent {
    /// Seconds into the recording when the answer was given.
    pub at_s: f64,
    /// "button" when clicked, "spoken" when said aloud.
    pub how: String,
    /// What was heard, for a spoken answer.
    #[serde(default)]
    pub words: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    pub at_s: f64,
    #[serde(default)]
    pub text: String,
    /// "action" for an action item; empty for a bookmark.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Deleted by the person. Kept so the same bookmark heard in the recording doesn't come back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrintLink {
    pub id: String,
    /// Similarity when recognised automatically; None when the person linked it.
    pub score: Option<f32>,
}

impl Meta {
    /// The languages it's in as saved; empty for the person's primary language.
    pub fn langs(&self) -> Vec<String> {
        crate::prefs::item_languages(&self.languages, &self.language)
    }

    pub fn set_langs(&mut self, languages: Vec<String>) {
        self.languages = languages;
        self.language.clear();
    }

    /// The name a track has before anyone is named: an app's track is called after the app, as
    /// "Microsoft Teams". It says what recorded the voice, not who spoke.
    pub fn track_name(&self, speaker: &str) -> Option<String> {
        crate::recorder::device_of(speaker, &self.inputs).and_then(|d| crate::sources::track_label(&d))
    }

    /// Named by the person, rather than only called after the track.
    pub fn named(&self, speaker: &str) -> bool {
        self.names.get(speaker).is_some_and(|n| Some(n) != self.track_name(speaker).as_ref())
    }

    pub fn name(&self, speaker: &str) -> String {
        match self.names.get(speaker) {
            Some(name) => name.clone(),
            None if speaker == UNKNOWN => "Unknown speaker".into(),
            None if speaker.starts_with("guest") => format!("Guest {}", &speaker[5..]),
            None => speaker.into(),
        }
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Written beside the target and renamed, so a crash mid-save never leaves half a file.
    let partial = path.with_extension("json.part");
    fs::write(&partial, text).map_err(|e| e.to_string())?;
    fs::rename(&partial, path).map_err(|e| e.to_string())
}

/// Writes text beside the target and renames it, like `write_json`.
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let partial = path.with_extension("part");
    fs::write(&partial, text).map_err(|e| e.to_string())?;
    fs::rename(&partial, path).map_err(|e| e.to_string())
}

/// Whether a meeting has been saved with its details yet; a new recording hasn't until it's processed.
pub fn has_meta(dir: &Path) -> bool {
    dir.join(META).exists()
}

pub fn load_meta(dir: &Path) -> Meta {
    read_json(&dir.join(META)).unwrap_or_else(|_| {
        let created = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("session-"))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        Meta { title: "Untitled session".into(), created, ..Meta::default() }
    })
}

pub fn load_segments(dir: &Path) -> Vec<Segment> {
    read_json(&dir.join(TRANSCRIPT)).unwrap_or_default()
}

/// Saves both files and regenerates the text exports that people open outside the app.
pub fn save(dir: &Path, meta: &Meta, segments: &[Segment]) -> Result<(), String> {
    write_json(&dir.join(META), meta)?;
    write_json(&dir.join(TRANSCRIPT), &segments)?;
    let lines: Vec<Line> = segments
        .iter()
        .filter(|s| !s.text.is_empty() && s.in_script())
        .map(|s| Line { speaker: meta.name(&s.speaker), start_s: s.start_s, end_s: s.end_s, text: s.text.clone() })
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    let write = |file: &str, text: String| fs::write(dir.join(file), text).map_err(|e| e.to_string());
    // Analyses go between the title and the transcript, and on their own in summary.md.
    let (md, txt) = (crate::analysis::render_meeting(dir, meta, segments, true), crate::analysis::render_meeting(dir, meta, segments, false));
    if md.is_empty() {
        write("transcript.txt", export::to_text(&lines))?;
        write("transcript.md", export::to_markdown(&meta.title, &lines))?;
        let _ = fs::remove_file(dir.join("summary.md"));
    } else {
        let body = export::to_markdown(&meta.title, &lines);
        let body = body.split_once("

").map_or(body.as_str(), |(_, rest)| rest);
        write("transcript.md", format!("# {}

{md}## Transcript

{body}", meta.title))?;
        write("transcript.txt", format!("{}

{txt}TRANSCRIPT
==========

{}", meta.title, export::to_text(&lines)))?;
        write("summary.md", format!("# {}

{md}", meta.title))?;
    }
    write("transcript.srt", export::to_srt(&lines))
}

pub fn new_meta(dir: &Path, duration_s: f64) -> Meta {
    let mut meta = load_meta(dir);
    if meta.created == 0 {
        meta.created = now();
    }
    meta.duration_s = duration_s;
    meta
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub duration_s: f64,
    pub preview: String,
    pub needs_processing: bool,
    pub status: String,
    pub compacted: bool,
}

/// Newest first. A query matches the title, a speaker name or any transcript text, ignoring case.
/// Sessions in `busy` are being recorded or processed, so they are not offered for processing.
pub fn list(query: &str, busy: &[String]) -> Vec<Summary> {
    let query = query.trim().to_lowercase();
    let Ok(entries) = fs::read_dir(root()) else { return Vec::new() };
    let mut out = Vec::new();
    for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        // Exports, Templates and anything else kept beside the meetings aren't meetings.
        if !id.starts_with("session-") {
            continue;
        }
        let meta = load_meta(&dir);
        let segments = load_segments(&dir);
        let line = |s: &Segment| format!("{}: {}", meta.name(&s.speaker), s.text);
        let preview = if query.is_empty() || meta.title.to_lowercase().contains(&query) {
            segments.iter().find(|s| !s.text.is_empty()).map(line).unwrap_or_default()
        } else {
            match segments.iter().map(line).find(|l| l.to_lowercase().contains(&query)) {
                Some(hit) => hit,
                None => continue,
            }
        };
        let needs_processing = !busy.contains(&id) && crate::process::needs_processing(&dir);
        out.push(Summary {
            id,
            title: meta.title,
            created: meta.created,
            duration_s: meta.duration_s,
            preview,
            needs_processing,
            status: meta.status,
            compacted: meta.compacted,
        });
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
    out
}

/// On Windows the folder goes to the Recycle Bin and on the Mac to the Trash, so it can be restored. Phones
/// have neither, so the page warns that deletion there is permanent.
pub fn delete(id: &str) -> Result<(), String> {
    let dir = session_dir(id)?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    return trash::delete(dir).map_err(|e| e.to_string());
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return fs::remove_dir_all(dir).map_err(|e| e.to_string());
}

/// A session's recorded audio: the WAV per microphone and the raw journals they were made from.
pub fn audio_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && (p.extension().is_some_and(|x| x == "wav" || x == "stvj") || crate::imports::is_track_file(p)))
        .collect();
    files.sort();
    files
}

/// Bytes the audio takes, which compacting frees.
pub fn audio_bytes(dir: &Path) -> u64 {
    audio_files(dir).iter().filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).sum()
}

/// Deletes a session's audio for good, keeping its transcript, names, markers and exports.
/// Marked first, so a session left with some journals after a failed delete is never processed
/// from what remains. Returns the bytes freed.
pub fn compact(dir: &Path) -> Result<u64, String> {
    if !dir.join(TRANSCRIPT).exists() {
        return Err("This meeting has no transcript yet, so compacting it would leave nothing.".into());
    }
    let mut meta = load_meta(dir);
    if !meta.compacted {
        meta.compacted = true;
        write_json(&dir.join(META), &meta)?;
    }
    let mut freed = 0;
    for file in audio_files(dir) {
        let len = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        fs::remove_file(&file).map_err(|e| format!("Couldn't delete {}: {e}", file.display()))?;
        freed += len;
    }
    Ok(freed)
}

/// A recorded track's audio files: its WAV, its journal and any journals it continued into ("input1-2.stvj").
fn track_files(dir: &Path, stream: &str) -> Vec<PathBuf> {
    audio_files(dir)
        .into_iter()
        .filter(|p| {
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or_default();
            let stem = p.file_stem().and_then(|n| n.to_str()).unwrap_or_default();
            let continued = stem
                .strip_prefix(stream)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            (ext == "wav" || ext == "stvj") && (stem == stream || (ext == "stvj" && continued))
        })
        .collect()
}

/// Takes a recorded track out of a meeting, as one that heard nothing or doubles another, so processing
/// again hears the meeting without it. Its audio goes to the Recycle Bin or Trash where there is one, and
/// its name and voice print links go. The last track with sound can't be removed.
pub fn delete_track(dir: &Path, stream: &str) -> Result<(), String> {
    let plain = !stream.is_empty() && stream.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let files = if plain { track_files(dir, stream) } else { Vec::new() };
    if files.is_empty() {
        return Err("That track no longer exists.".into());
    }
    let mut meta = load_meta(dir);
    if meta.compacted {
        return Err("This meeting's audio was deleted, so its tracks can't be changed.".into());
    }
    // Processing needs a recorded journal with something in it; an input that recorded nothing leaves a
    // journal of only its header.
    let others = audio_files(dir).into_iter().filter(|p| !files.contains(p) && p.extension().is_some_and(|x| x == "stvj"));
    if others.filter(|p| fs::metadata(p).is_ok_and(|m| m.len() > 64)).count() == 0 {
        return Err("This is the meeting's only track, so removing it would leave nothing to transcribe. Delete the meeting instead.".into());
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    trash::delete_all(&files).map_err(|e| e.to_string())?;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    for file in &files {
        fs::remove_file(file).map_err(|e| e.to_string())?;
    }
    let name = meta.name(stream);
    let shown = if name == stream { meta.track_name(stream).unwrap_or(name) } else { name };
    meta.tracks_removed.push(shown);
    meta.tracks.remove(stream);
    meta.names.remove(stream);
    meta.prints.remove(stream);
    meta.suggested.remove(stream);
    meta.track_offsets.remove(stream);
    meta.track_levels.remove(stream);
    meta.mix_edits.remove(stream);
    meta.locked.retain(|k| k != stream);
    // A meeting not yet processed has no details or transcript to change.
    if has_meta(dir) {
        write_json(&dir.join(META), &meta)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: usize, speaker: &str, text: &str) -> Segment {
        Segment {
            id,
            speaker: speaker.into(),
            stream: "input1".into(),
            start_s: id as f64,
            end_s: id as f64 + 1.0,
            text: text.into(),
            confidence: 1.0,
            edited: false,
            overlap: false,
            ..Default::default()
        }
    }

    #[test]
    fn names_fall_back_to_the_stream_and_guest_label() {
        let mut meta = Meta::default();
        assert_eq!(meta.name("input1"), "input1");
        assert_eq!(meta.name(UNKNOWN), "Unknown speaker");
        meta.names.insert("input1".into(), "Alice".into());
        assert_eq!(meta.name("input1"), "Alice");
    }

    #[test]
    fn saving_regenerates_exports_with_names() {
        let dir = std::env::temp_dir().join(format!("stv-library-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut meta = Meta { title: "Standup".into(), ..Meta::default() };
        meta.names.insert("input1".into(), "Alice".into());
        let segments = vec![segment(0, "input1", "Hello"), segment(1, UNKNOWN, ""), segment(2, UNKNOWN, "Hi")];
        save(&dir, &meta, &segments).unwrap();

        assert_eq!(load_segments(&dir), segments);
        assert_eq!(load_meta(&dir).title, "Standup");
        let text = fs::read_to_string(dir.join("transcript.txt")).unwrap();
        assert_eq!(text, "[00:00:00.000] Alice: Hello\n[00:00:02.000] Unknown speaker: Hi\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compacting_deletes_only_the_audio() {
        let dir = std::env::temp_dir().join(format!("stv-compact-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let meta = Meta { title: "Standup".into(), ..Meta::default() };
        let segments = vec![segment(0, "input1", "Hello")];
        save(&dir, &meta, &segments).unwrap();
        for (file, bytes) in [("input1.wav", 100), ("input2.wav", 50), ("input1.stvj", 30), ("input1-2.stvj", 20)] {
            fs::write(dir.join(file), vec![0u8; bytes]).unwrap();
        }
        fs::write(dir.join("keyphrases.json"), "[]").unwrap();
        assert_eq!(audio_bytes(&dir), 200);

        assert_eq!(compact(&dir).unwrap(), 200);
        assert!(audio_files(&dir).is_empty());
        assert!(load_meta(&dir).compacted);
        assert_eq!(load_meta(&dir).title, "Standup");
        assert_eq!(load_segments(&dir), segments);
        for kept in ["transcript.txt", "transcript.md", "transcript.srt", "keyphrases.json"] {
            assert!(dir.join(kept).exists(), "{kept}");
        }
        // The transcript can still be edited and saved.
        let mut edited = segments.clone();
        edited[0].text = "Hello there".into();
        save(&dir, &load_meta(&dir), &edited).unwrap();
        assert!(load_meta(&dir).compacted);
        assert_eq!(load_segments(&dir)[0].text, "Hello there");
        // Compacting again does nothing more.
        assert_eq!(compact(&dir).unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_without_a_transcript_is_not_compacted() {
        let dir = std::env::temp_dir().join(format!("stv-compact-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("input1.stvj"), [0u8; 10]).unwrap();
        assert!(compact(&dir).is_err());
        assert!(dir.join("input1.stvj").exists());
        assert!(!load_meta(&dir).compacted);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_track_removes_only_its_audio_and_names() {
        let dir = std::env::temp_dir().join(format!("stv-delete-track-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut meta = Meta { title: "Standup".into(), ..Meta::default() };
        meta.names.insert("input1".into(), "Alice".into());
        meta.names.insert("input2".into(), "Teams".into());
        meta.prints.insert("input2".into(), PrintLink { id: "vp".into(), score: Some(0.9) });
        meta.tracks.insert("input2".into(), Track::default());
        save(&dir, &meta, &[segment(0, "input1", "Hello")]).unwrap();
        for file in ["input1.wav", "input1.stvj", "input2.wav", "input2.stvj", "input2-2.stvj", "input12.stvj"] {
            fs::write(dir.join(file), vec![0u8; 100]).unwrap();
        }

        delete_track(&dir, "input2").unwrap();
        let left: Vec<String> = audio_files(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["input1.stvj", "input1.wav", "input12.stvj"]);
        let meta = load_meta(&dir);
        assert_eq!(meta.names.keys().collect::<Vec<_>>(), ["input1"]);
        assert!(meta.prints.is_empty() && meta.tracks.is_empty());
        assert_eq!(meta.tracks_removed, ["Teams"]);

        assert!(delete_track(&dir, "input2").is_err(), "already gone");
        assert!(delete_track(&dir, "../input1").is_err());
        delete_track(&dir, "input12").unwrap();
        assert!(delete_track(&dir, "input1").is_err(), "the last track stays");
        assert_eq!(audio_files(&dir).len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_outside_the_library_are_rejected() {
        for id in ["", "..", "a/b", "..\\x", "C:\\Windows"] {
            assert!(session_dir(id).is_err(), "{id}");
        }
    }
}
