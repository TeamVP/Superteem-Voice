//! Typed and dictated notes, one JSON file each, kept beside the recordings.

use crate::library;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub created: u64,
    pub updated: u64,
    /// Keeps each dictation's audio for a week; otherwise it is dropped once written down.
    pub keep_audio: bool,
    /// Hidden from the list until unarchived; still found by search.
    pub archived: bool,
    /// Where the note is in the person's own workflow; empty when not set.
    pub status: String,
    /// The language older versions kept, before a note could be in two; read into `languages`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// The languages it's in, the main one first; empty for the person's primary language.
    pub languages: Vec<String>,
    /// "md" when `body` is Markdown, as the formatting editor writes it; empty for plain text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub format: String,
    /// The editor's own document, which keeps what Markdown can't (alignment). `body` is the same note
    /// as Markdown; when this is missing, as after the assistant rewrites the note, `body` is the note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<serde_json::Value>,
}

impl Note {
    pub fn langs(&self) -> Vec<String> {
        crate::prefs::item_languages(&self.languages, &self.language)
    }

    /// A line of the body as it reads, without Markdown's marks when the body is Markdown.
    fn line(&self, line: &str) -> String {
        if self.format == "md" { plain_line(line) } else { line.trim().to_string() }
    }
}

#[derive(Debug, Serialize)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub created: u64,
    pub updated: u64,
    pub words: usize,
    pub archived: bool,
    pub status: String,
}

fn dir() -> PathBuf {
    library::data_dir().join("notes")
}

fn audio_dir() -> PathBuf {
    dir().join("audio")
}

/// Kept dictation audio is deleted after this long.
const AUDIO_KEEP_S: u64 = 7 * 24 * 3600;

/// Keeps one dictation's audio beside its note, when the note asks for it.
pub fn keep_dictation(id: &str, pcm: &[i16], rate: u32) -> Result<(), String> {
    path(id)?;
    if !load(id)?.keep_audio || pcm.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(audio_dir()).map_err(|e| e.to_string())?;
    let file = audio_dir().join(format!("{id}_{}.wav", library::now()));
    stv_core::export::write_wav(&file, pcm, rate).map_err(|e| e.to_string())
}

/// Removes kept audio that is over a week old, or belongs to `only`.
fn remove_audio(only: Option<&str>) {
    let now = library::now();
    for entry in std::fs::read_dir(audio_dir()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((id, rest)) = name.split_once('_') else { continue };
        let at: u64 = rest.trim_end_matches(".wav").parse().unwrap_or(0);
        if only.map_or(now.saturating_sub(at) > AUDIO_KEEP_S, |o| o == id) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The dictation audio kept for a note, oldest first.
pub fn kept_audio(id: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(audio_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.split_once('_').is_some_and(|(owner, _)| owner == id)))
        .collect();
    files.sort();
    files
}

pub fn purge_old_audio() {
    remove_audio(None);
}

/// Note ids come from the UI, so only a bare name is accepted.
fn path(id: &str) -> Result<PathBuf, String> {
    let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if plain {
        Ok(dir().join(format!("{id}.json")))
    } else {
        Err("That note no longer exists.".into())
    }
}

/// One line of a note without Markdown's marks: "## Plan" is "Plan", "- [x] **Call** Tom" is "Call Tom",
/// a picture is its description and a link is its words.
pub fn plain_line(line: &str) -> String {
    let mut text = line.trim();
    loop {
        let before = text;
        text = text.trim_start_matches(['#', '>']).trim_start();
        for mark in ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "] {
            text = text.strip_prefix(mark).unwrap_or(text);
        }
        if text == before {
            break;
        }
    }
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' | '`' | '~' => {}
            '!' if chars.get(i + 1) == Some(&'[') => {}
            '\\' if i + 1 < chars.len() => {
                i += 1;
                out.push(chars[i]);
            }
            '[' => {}
            // "](address)" after a link's or picture's words.
            ']' if chars.get(i + 1) == Some(&'(') => {
                i += chars[i..].iter().position(|c| *c == ')').unwrap_or(chars.len() - i - 1);
            }
            c => out.push(c),
        }
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first line stands in for a title the person never typed.
pub fn display_title(note: &Note) -> String {
    let title = note.title.trim();
    if !title.is_empty() {
        return title.into();
    }
    let first = note.body.lines().map(|l| note.line(l)).find(|l| !l.is_empty()).unwrap_or_default();
    if first.is_empty() {
        "Untitled note".into()
    } else {
        first.chars().take(60).collect()
    }
}

pub fn load(id: &str) -> Result<Note, String> {
    let mut note: Note = library::read_json(&path(id)?).map_err(|_| "That note no longer exists.".to_string())?;
    // The file name is the id, even if the file itself lost or never had one.
    note.id = id.to_string();
    Ok(note)
}

/// What the formatting editor saves with a note's Markdown: its own document.
pub struct Rich {
    pub doc: Option<serde_json::Value>,
}

/// Saves the note, creating it when `id` is None, and returns its id. `rich` is given by the formatting
/// editor, whose `body` is Markdown; without it the body is taken as it is and the note's format kept.
pub fn save(id: Option<String>, title: String, body: String, keep_audio: bool, rich: Option<Rich>) -> Result<String, String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let now = library::now();
    let mut note = match &id {
        Some(id) => load(id)?,
        None => Note {
            id: format!("n{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7)),
            created: now,
            languages: crate::prefs::load().new_item_languages(),
            ..Default::default()
        },
    };
    // Coming back to a note after a break keeps how it was left, as a version.
    if id.is_some() && (note.title != title || note.body != body) && now.saturating_sub(note.updated) > SESSION_GAP_S {
        let mut all = load_versions(&note.id);
        if all.keep(&note, "you", "", note.updated).1 {
            save_versions(&note.id, &all)?;
        }
    }
    match rich {
        Some(rich) => {
            note.format = "md".into();
            note.doc = rich.doc;
        }
        // Text changed by something other than the editor: its document no longer matches.
        None if note.body != body => note.doc = None,
        None => {}
    }
    note.title = title;
    note.body = body;
    note.updated = now;
    if note.keep_audio && !keep_audio {
        remove_audio(Some(&note.id));
    }
    note.keep_audio = keep_audio;
    library::write_json(&path(&note.id)?, &note)?;
    Ok(note.id)
}

/// Archiving doesn't count as an edit, so the note keeps its place by date.
pub fn set_archived(id: &str, archived: bool) -> Result<(), String> {
    let mut note = load(id)?;
    note.archived = archived;
    library::write_json(&path(id)?, &note)
}

/// Like archiving, a status change keeps the note's place by date.
pub fn set_status(id: &str, status: &str) -> Result<(), String> {
    let mut note = load(id)?;
    note.status = library::check_status(status)?;
    library::write_json(&path(id)?, &note)
}

/// Like a status, languages don't count as an edit.
pub fn set_languages(id: &str, languages: Vec<String>) -> Result<(), String> {
    let mut note = load(id)?;
    note.languages = languages;
    note.language.clear();
    library::write_json(&path(id)?, &note)
}

pub fn delete(id: &str) -> Result<(), String> {
    remove_audio(Some(id));
    let file = path(id)?;
    let _ = std::fs::remove_file(file.with_extension("versions.json"));
    let _ = std::fs::remove_file(file.with_extension("chat.json"));
    let _ = std::fs::remove_dir_all(file.with_extension("files"));
    std::fs::remove_file(file).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------------------
// Files kept with a note

/// The most a kept file can be.
pub const MAX_FILE_BYTES: usize = 200 * 1024 * 1024;
/// The most a picture fetched from the web can be.
const MAX_WEB_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

/// A picture or file kept with a note, in `<id>.files/` beside it. The note refers to it as
/// "attachment:<file>", so the note and its files can move, or be wrapped, together.
#[derive(Debug, Serialize)]
pub struct Kept {
    /// Its name on disk: a few random letters, then the name it came with.
    pub file: String,
    /// The name it came with.
    pub name: String,
    pub size: usize,
    /// Shown in the note as a picture rather than a link.
    pub image: bool,
}

fn files_dir(id: &str) -> Result<PathBuf, String> {
    Ok(path(id)?.with_extension("files"))
}

/// The kind of picture these bytes are, by how they start; None for anything else.
fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xff, 0xd8, 0xff, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        [b'B', b'M', ..] => Some("bmp"),
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => Some("avif"),
        _ => None,
    }
}

/// What a kept file is served as. Only pictures are shown in the page; anything else is a download.
pub fn media_type(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    }
}

/// A name safe to keep a file under: letters, digits, dots, dashes and underscores, and no folders.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
    let cleaned = cleaned.trim_matches(['.', '-']).to_string();
    let short: String = cleaned.chars().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
    if short.is_empty() { "file".into() } else { short }
}

/// Where a kept file is. File names come from the note, so only a bare name inside the folder is accepted.
pub fn file_path(id: &str, file: &str) -> Result<PathBuf, String> {
    let plain = !file.is_empty() && !file.starts_with('.') && file.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let found = files_dir(id)?.join(file);
    if plain && found.is_file() {
        Ok(found)
    } else {
        Err("That file is no longer with the note.".into())
    }
}

/// Keeps a file with a note.
pub fn add_file(id: &str, name: &str, bytes: &[u8]) -> Result<Kept, String> {
    load(id)?;
    if bytes.is_empty() {
        return Err("That file is empty.".into());
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("That file is too big to keep in a note: the most is {} MB.", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let image = image_kind(bytes);
    let mut name = safe_name(name);
    // A picture is named for what it is, whatever it was called, so it's served as that.
    if let Some(kind) = image {
        if media_type(&name) == "application/octet-stream" {
            name = format!("{name}.{kind}");
        }
    }
    let dir = files_dir(id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // A few random letters in front, so two files of the same name are both kept.
    use std::hash::{BuildHasher, Hasher};
    let tag = std::collections::hash_map::RandomState::new().build_hasher().finish() as u32;
    let file = format!("{tag:08x}-{name}");
    std::fs::write(dir.join(&file), bytes).map_err(|e| e.to_string())?;
    let image = image.is_some() && media_type(&name) != "application/octet-stream";
    Ok(Kept { file, name, size: bytes.len(), image })
}

/// Fetches a picture from the web, once, and keeps it with the note.
pub fn import_image(id: &str, url: &str) -> Result<Kept, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("A picture's address starts with https://.".into());
    }
    let response = crate::transcribe::agent()?
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .call()
        .map_err(|e| format!("The picture couldn't be fetched: {e}"))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    response.into_reader().take(MAX_WEB_IMAGE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_WEB_IMAGE_BYTES {
        return Err("That picture is too big to keep in a note.".into());
    }
    if image_kind(&bytes).is_none() {
        return Err("That address isn't a picture.".into());
    }
    let name = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    add_file(id, if name.is_empty() { "picture" } else { name }, &bytes)
}

// ---------------------------------------------------------------------------------------------------------
// Versions

/// Editing again after this long keeps the note as it was, as a version.
const SESSION_GAP_S: u64 = 5 * 60;
/// The oldest versions are dropped past this many.
const MAX_VERSIONS: usize = 200;

/// A note as it was at some moment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Version {
    /// Counts up from 1 and is never reused.
    pub n: u32,
    pub at: u64,
    pub title: String,
    pub body: String,
    /// The note's format and editor document then (see `Note`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<serde_json::Value>,
    /// Who made it: "you", "ai" (a rewrite asked for in the chat) or "restore" (an earlier version brought back).
    pub by: String,
    /// What was asked of the assistant, or which version was brought back.
    pub label: String,
}

/// A note's versions, oldest first, kept beside it as `<id>.versions.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Versions {
    pub last: u32,
    pub versions: Vec<Version>,
}

impl Versions {
    /// Keeps the note as a version unless the newest version already is it. Gives the number of the
    /// version that holds this text, and whether one was added.
    fn keep(&mut self, note: &Note, by: &str, label: &str, at: u64) -> (u32, bool) {
        let n = self.number(note);
        if n == 0 || n <= self.last {
            return (n, false);
        }
        self.last = n;
        self.versions.push(Version {
            n: self.last,
            at,
            title: note.title.clone(),
            body: note.body.clone(),
            format: note.format.clone(),
            doc: note.doc.clone(),
            by: by.into(),
            label: label.into(),
        });
        if self.versions.len() > MAX_VERSIONS {
            let extra = self.versions.len() - MAX_VERSIONS;
            self.versions.drain(..extra);
        }
        (self.last, true)
    }

    /// The number of the version that holds this text: the newest when it already does, else the one it
    /// will be kept as. 0 for an empty note, which isn't a version.
    fn number(&self, note: &Note) -> u32 {
        match self.versions.last() {
            Some(v) if v.title == note.title && v.body == note.body => v.n,
            _ if note.title.trim().is_empty() && note.body.trim().is_empty() => 0,
            _ => self.last + 1,
        }
    }
}

/// The note's version number as it is now, as shown beside its title on a phone. Keeps nothing.
pub fn version_number(id: &str) -> Result<u32, String> {
    Ok(load_versions(id).number(&load(id)?))
}

fn load_versions(id: &str) -> Versions {
    path(id).ok().and_then(|p| library::read_json(&p.with_extension("versions.json")).ok()).unwrap_or_default()
}

fn save_versions(id: &str, all: &Versions) -> Result<(), String> {
    let file = path(id)?.with_extension("versions.json");
    // write_json's partial file would otherwise be `<id>.json.part`, the note's own.
    let text = serde_json::to_string_pretty(all).map_err(|e| e.to_string())?;
    let partial = file.with_extension("json.tmp");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &file).map_err(|e| e.to_string())
}

/// The note's versions, oldest first. The note as it is now is always the last of them.
pub fn versions(id: &str) -> Result<Vec<Version>, String> {
    let note = load(id)?;
    let mut all = load_versions(id);
    if all.keep(&note, "you", "", note.updated).1 {
        save_versions(id, &all)?;
    }
    Ok(all.versions)
}

/// The newest version that holds the note's text as it is now, if one does. Unlike `versions`, keeps nothing.
pub fn current_version(id: &str) -> Result<Option<u32>, String> {
    let note = load(id)?;
    Ok(load_versions(id).versions.iter().rev().find(|v| v.title == note.title && v.body == note.body).map(|v| v.n))
}

/// Replaces the note's text with the assistant's, as a new version. The text before it is kept as a
/// version too. Gives the new version's number.
pub fn rewrite(id: &str, body: String, label: &str) -> Result<u32, String> {
    let mut note = load(id)?;
    let mut all = load_versions(id);
    all.keep(&note, "you", "", note.updated);
    let now = library::now();
    // The assistant writes Markdown, which is the note from here on; the editor reads it back in.
    note.body = body;
    note.format = "md".into();
    note.doc = None;
    note.updated = now;
    let (n, _) = all.keep(&note, "ai", label, now);
    save_versions(id, &all)?;
    library::write_json(&path(id)?, &note)?;
    Ok(n)
}

/// Puts dictated words on the end of the note, as their own paragraph: for a dictation that went on
/// after its page was left, so nothing said is lost.
pub fn append_dictation(id: &str, words: &str) -> Result<(), String> {
    let words = words.trim();
    if words.is_empty() {
        return Ok(());
    }
    let mut note = load(id)?;
    let body = note.body.trim_end();
    note.body = if body.is_empty() { words.to_string() } else { format!("{body}

{words}") };
    // The editor's document doesn't have these words; it reads the body back in.
    note.doc = None;
    note.updated = library::now();
    library::write_json(&path(id)?, &note)
}

/// Makes an earlier version the note again, as a new version, so nothing in between is lost.
pub fn restore(id: &str, n: u32) -> Result<Note, String> {
    let mut note = load(id)?;
    let mut all = load_versions(id);
    all.keep(&note, "you", "", note.updated);
    let old = all.versions.iter().find(|v| v.n == n).cloned().ok_or("That version is no longer kept.")?;
    let now = library::now();
    note.title = old.title;
    note.body = old.body;
    note.format = old.format;
    note.doc = old.doc;
    note.updated = now;
    all.keep(&note, "restore", &format!("Version {n} brought back"), now);
    save_versions(id, &all)?;
    library::write_json(&path(id)?, &note)?;
    Ok(note)
}

/// A note file found in the folder, or None for anything else kept there.
///
/// Analyses (`<id>.analysis.json`) and half-written saves sit beside the notes, and every
/// field of `Note` has a default, so they would otherwise parse as a blank note dated 1970
/// that can't be opened or deleted. Only `<id>.json` with a plain id counts, and the note
/// takes its id from its file name, so whatever the list shows can be opened and deleted.
fn read_entry(file: &std::path::Path) -> Option<Note> {
    if file.extension()? != "json" {
        return None;
    }
    let id = file.file_stem()?.to_str()?;
    path(id).ok()?;
    let mut note: Note = library::read_json(file).ok()?;
    note.id = id.to_string();
    if note.created == 0 && note.updated == 0 {
        // Undated: fall back to the file's own time rather than show 1970.
        let modified = std::fs::metadata(file).and_then(|m| m.modified()).ok()?;
        let at = modified.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        note.created = at;
        note.updated = at;
    }
    Some(note)
}

pub fn list(query: &str) -> Vec<NoteSummary> {
    let query = query.trim().to_lowercase();
    let mut notes: Vec<Note> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read_entry(&e.path()))
        .filter(|n| query.is_empty() || n.title.to_lowercase().contains(&query) || n.body.to_lowercase().contains(&query))
        .collect();
    notes.sort_by(|a, b| b.updated.cmp(&a.updated));
    notes
        .iter()
        .map(|n| {
            let title = display_title(n);
            let preview: String = n.body.lines().map(|l| n.line(l)).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ").chars().take(120).collect();
            NoteSummary {
                id: n.id.clone(),
                preview,
                created: n.created,
                updated: n.updated,
                words: n.body.split_whitespace().count(),
                archived: n.archived,
                status: n.status.clone(),
                title,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_notes_use_their_first_line() {
        let note = |title: &str, body: &str| Note { title: title.into(), body: body.into(), ..Default::default() };
        assert_eq!(display_title(&note("Plan", "x")), "Plan");
        assert_eq!(display_title(&note(" ", "\n  First line\nsecond")), "First line");
        assert_eq!(display_title(&note("", "")), "Untitled note");
    }

    #[test]
    fn analyses_and_partial_saves_are_not_listed_as_notes() {
        let folder = std::env::temp_dir().join(format!("stv-notes-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let write = |name: &str, text: &str| {
            let file = folder.join(name);
            std::fs::write(&file, text).unwrap();
            file
        };
        let note = write("n5-1.json", r#"{"id":"n5-1","title":"Plan","created":5,"updated":6}"#);
        let analysis = write("n5-1.analysis.json", r#"{"templates":[]}"#);
        let partial = write("n5-1.json.part", r#"{"id":"n5-1"}"#);
        let idless = write("n7-2.json", r#"{"title":"Lost its id"}"#);

        let found = read_entry(&note).unwrap();
        assert_eq!((found.id.as_str(), found.created, found.updated), ("n5-1", 5, 6));
        assert!(read_entry(&analysis).is_none());
        assert!(read_entry(&partial).is_none());
        let found = read_entry(&idless).unwrap();
        assert_eq!(found.id, "n7-2");
        assert!(found.created > 0 && found.updated > 0, "an undated note isn't shown as 1970");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn markdown_lines_read_as_plain_text() {
        assert_eq!(plain_line("## Offsite **plan**"), "Offsite plan");
        assert_eq!(plain_line("- [x] Call *Tom* about `flights`"), "Call Tom about flights");
        assert_eq!(plain_line("> See [the venue](https://example.com/a) and ![a map](attachment:ab12-map.png)"), "See the venue and a map");
        let note = Note { body: "# Plan\n\n- first".into(), format: "md".into(), ..Default::default() };
        assert_eq!(display_title(&note), "Plan");
        let plain = Note { body: "# not a heading".into(), ..Default::default() };
        assert_eq!(display_title(&plain), "# not a heading");
    }

    #[test]
    fn kept_files_get_safe_names_and_pictures_are_told_by_their_bytes() {
        assert_eq!(safe_name("..\\..\\Quarterly report (final).pdf"), "Quarterly-report--final-.pdf");
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("???"), "file");
        assert_eq!(image_kind(&[0x89, b'P', b'N', b'G', 0x0d]), Some("png"));
        assert_eq!(image_kind(b"<svg onload=alert(1)>"), None, "SVG can carry script, so it's a file, not a picture");
        assert_eq!(media_type("ab12-map.PNG"), "image/png");
        assert_eq!(media_type("ab12-page.html"), "application/octet-stream");
    }

    #[test]
    fn versions_keep_each_change_once() {
        let note = |body: &str| Note { body: body.into(), ..Default::default() };
        let mut all = Versions::default();
        assert_eq!(all.keep(&note(""), "you", "", 1), (0, false), "an empty note isn't a version");
        assert_eq!(all.number(&note("first")), 1, "the number a change will be kept as");
        assert_eq!(all.keep(&note("first"), "you", "", 1), (1, true));
        assert_eq!(all.number(&note("first")), 1);
        assert_eq!(all.keep(&note("first"), "you", "", 2), (1, false), "unchanged text isn't kept twice");
        assert_eq!(all.keep(&note("shorter"), "ai", "Make it shorter", 3), (2, true));
        assert_eq!(all.keep(&note("first"), "restore", "Version 1 brought back", 4), (3, true));
        assert_eq!(all.versions.iter().map(|v| v.by.as_str()).collect::<Vec<_>>(), ["you", "ai", "restore"]);
        for i in 0..MAX_VERSIONS + 5 {
            all.keep(&note(&format!("text {i}")), "you", "", 5);
        }
        assert_eq!(all.versions.len(), MAX_VERSIONS);
        assert_eq!(all.versions.last().unwrap().n, all.last);
    }

    #[test]
    fn only_plain_ids_are_accepted() {
        assert!(path("n123-4").is_ok());
        assert!(path("../prefs").is_err());
        assert!(path("").is_err());
    }
}
