//! Typed and dictated notes. Their text, versions, bookmarks and what's written about them are rows in the
//! vault (vault.rs); the pictures, files and dictation audio kept with a note are sealed files in the notes
//! folder (sealed.rs).

use crate::{library, vault};
use rusqlite::{Connection, OptionalExtension};
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
    /// How to say words the voices get wrong, in this note only, as a book's `pronounce`: each word as
    /// written to how it's spelt for the voice. Read on top of the book's own, so a note can override one.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub pronounce: std::collections::BTreeMap<String, String>,
    /// Notes about the note (Markdown): background, context, a reader's guide. Hidden until asked for,
    /// then shown above the note. Never read aloud with it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commentary: String,
    /// Who wrote this note, when it isn't the book's author (a preface by its translator, a chapter by a
    /// guest); empty for the book's own.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
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
    if !load(id)?.keep_audio || pcm.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(audio_dir()).map_err(|e| e.to_string())?;
    let file = audio_dir().join(format!("{id}_{}.wav", library::now()));
    crate::sealed::write_wav(&file, pcm, rate)
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

const GONE: &str = "That note no longer exists.";

/// Note ids come from the UI and name a folder of kept files, so only a bare name is accepted.
fn check_id(id: &str) -> Result<(), String> {
    let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if plain { Ok(()) } else { Err(GONE.into()) }
}

fn e(err: rusqlite::Error) -> String {
    err.to_string()
}

/// A note, read through a connection already in hand (inside a transaction, or by the books).
pub fn get(conn: &Connection, id: &str) -> Result<Note, String> {
    check_id(id)?;
    let json: Option<String> = conn.prepare_cached("SELECT json FROM notes WHERE id = ?1").and_then(|mut s| s.query_row([id], |r| r.get(0)).optional()).map_err(e)?;
    let mut note: Note = serde_json::from_str(&json.ok_or(GONE)?).map_err(|_| GONE.to_string())?;
    // The row's id is the note's, even if its JSON lost or never had one.
    note.id = id.to_string();
    Ok(note)
}

/// Whether a note of this id is in the vault.
pub fn exists(conn: &Connection, id: &str) -> bool {
    conn.prepare_cached("SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1)").and_then(|mut s| s.query_row([id], |r| r.get::<_, bool>(0))).unwrap_or(false)
}

/// The note as the list shows it: its title, the start of its text, and how many words it has.
fn summary(note: &Note) -> (String, String, usize) {
    let preview: String = note.body.lines().map(|l| note.line(l)).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ").chars().take(120).collect();
    (display_title(note), preview, note.body.split_whitespace().count())
}

/// Writes the note, with the columns the list and search read kept in step.
pub fn put(conn: &Connection, note: &Note) -> Result<(), String> {
    check_id(&note.id)?;
    let json = serde_json::to_string(note).map_err(|e| e.to_string())?;
    let (title, preview, words) = summary(note);
    let search = format!("{}\n{}", note.title.to_lowercase(), note.body.to_lowercase());
    conn.prepare_cached(
        "INSERT INTO notes(id, json, created, updated, archived, status, title, preview, words, search) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET json = ?2, created = ?3, updated = ?4, archived = ?5, status = ?6, title = ?7, preview = ?8, words = ?9, search = ?10",
    )
    .and_then(|mut s| s.execute((&note.id, json, note.created as i64, note.updated as i64, note.archived, &note.status, title, preview, words as i64, search)))
    .map_err(e)?;
    Ok(())
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
    vault::with(|conn| get(conn, id))
}

/// Changes something about a note that isn't an edit, so it keeps its place by date. A note in a book
/// kept for reading is refused before the vault is touched.
fn change(id: &str, change: impl FnOnce(&mut Note) -> Result<(), String>) -> Result<(), String> {
    crate::books::protect(id)?;
    vault::write(|conn| {
        let mut note = get(conn, id)?;
        change(&mut note)?;
        put(conn, &note)
    })
}

/// What the formatting editor saves with a note's Markdown: its own document.
pub struct Rich {
    pub doc: Option<serde_json::Value>,
}

/// Saves the note, creating it when `id` is None, and returns its id. `rich` is given by the formatting
/// editor, whose `body` is Markdown; without it the body is taken as it is and the note's format kept.
pub fn save(id: Option<String>, title: String, body: String, keep_audio: bool, rich: Option<Rich>) -> Result<String, String> {
    if let Some(id) = &id {
        crate::books::protect(id)?;
    }
    // New notes take the person's languages; read before the vault is held, as preferences are a file.
    let languages = if id.is_none() { crate::prefs::load().new_item_languages() } else { Vec::new() };
    let now = library::now();
    let (id, drop_audio) = vault::write(|conn| {
        let mut note = match &id {
            Some(id) => get(conn, id)?,
            None => {
                // Two notes made in the same second would otherwise share an id, and the second replace the first.
                let base = format!("n{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7));
                let id = std::iter::once(base.clone()).chain((2..).map(|n| format!("{base}-{n}"))).find(|id| !exists(conn, id)).unwrap_or(base);
                Note { id, created: now, languages, ..Default::default() }
            }
        };
        // Coming back to a note after a break keeps how it was left, as a version.
        if id.is_some() && (note.title != title || note.body != body) && now.saturating_sub(note.updated) > SESSION_GAP_S {
            keep(conn, &note, "you", "", note.updated)?;
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
        let drop_audio = note.keep_audio && !keep_audio;
        note.keep_audio = keep_audio;
        put(conn, &note)?;
        Ok((note.id, drop_audio))
    })?;
    if drop_audio {
        remove_audio(Some(&id));
    }
    Ok(id)
}

/// Archiving doesn't count as an edit, so the note keeps its place by date.
pub fn set_archived(id: &str, archived: bool) -> Result<(), String> {
    change(id, |note| {
        note.archived = archived;
        Ok(())
    })
}

/// Like archiving, a status change keeps the note's place by date.
pub fn set_status(id: &str, status: &str) -> Result<(), String> {
    let status = library::check_status(status)?;
    change(id, |note| {
        note.status = status;
        Ok(())
    })
}

/// Like a status, languages don't count as an edit.
pub fn set_languages(id: &str, languages: Vec<String>) -> Result<(), String> {
    change(id, |note| {
        note.languages = languages;
        note.language.clear();
        Ok(())
    })
}

/// Deletes the note with its versions and everything written about it: its chat, analyses, translations
/// and bookmarks, all in one go. Its kept files and dictation audio go after.
pub fn delete(id: &str) -> Result<(), String> {
    crate::books::protect(id)?;
    vault::write(|conn| {
        get(conn, id)?;
        conn.execute("DELETE FROM notes WHERE id = ?1", [id]).map_err(e)?;
        conn.execute("DELETE FROM versions WHERE note = ?1", [id]).map_err(e)?;
        conn.execute("DELETE FROM docs WHERE owner = ?1 AND kind IN ('chat', 'analysis', 'translation', 'bookmarks')", [id]).map_err(e)?;
        Ok(())
    })?;
    remove_audio(Some(id));
    let _ = std::fs::remove_dir_all(dir().join(format!("{id}.files")));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Commentary and pronunciations: written about the note, beside its text

/// The most pronunciations kept, and the longest commentary.
const MAX_PRONOUNCE: usize = 1000;
const MAX_COMMENTARY: usize = 50_000;

/// Pronunciations as typed, tidied: each word trimmed, one word of letters (with ' or -), and a spelling that
/// says something other than the word. Others are dropped.
pub fn clean_pronounce(words: std::collections::BTreeMap<String, String>) -> std::collections::BTreeMap<String, String> {
    words
        .into_iter()
        .map(|(w, s)| (w.trim().to_string(), s.split_whitespace().collect::<Vec<_>>().join(" ")))
        .filter(|(w, s)| {
            !w.is_empty()
                && w.chars().count() <= 60
                && w.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '’' || c == '-')
                && !s.is_empty()
                && s.chars().count() <= 120
                && s != w
        })
        .take(MAX_PRONOUNCE)
        .collect()
}

/// One line as typed: its spaces tidied, and kept to `max` characters.
pub fn clean_line(text: &str, max: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Commentary as typed, trimmed and kept to a length.
pub fn clean_commentary(text: String) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_COMMENTARY) {
        Some((i, _)) => text[..i].to_string(),
        None => text.to_string(),
    }
}

/// Changes a note's commentary, pronunciations or author, whichever is given. Not its text, so no version is
/// kept.
pub fn set_extras(id: &str, commentary: Option<String>, pronounce: Option<std::collections::BTreeMap<String, String>>, author: Option<String>) -> Result<(), String> {
    change(id, |note| {
        if let Some(name) = author {
            note.author = clean_line(&name, 200);
        }
        if let Some(text) = commentary {
            note.commentary = clean_commentary(text);
        }
        if let Some(words) = pronounce {
            note.pronounce = clean_pronounce(words);
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------------------------------------
// Bookmarks: places and passages marked in a note, kept in the vault with it (before the vault, beside it
// as `<id>.bookmarks.json`)

/// One bookmark. It finds its place again by the words it covers (`quote`) and those around it, so it
/// holds through edits, and `at` (where it began in the note's text) chooses between repeats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmark {
    pub id: String,
    pub at: usize,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub label: String,
    /// "c01" to "c15", or empty for no colour.
    pub color: String,
    pub created: u64,
    /// Who made it, when someone else did and it came in an import; empty for the person's own.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
}

/// The most bookmarks a note keeps, and the longest a label or quote is kept.
const MAX_BOOKMARKS: usize = 2000;
const MAX_LABEL: usize = 120;
const MAX_QUOTE: usize = 4000;

pub fn bookmarks(id: &str) -> Result<Vec<Bookmark>, String> {
    check_id(id)?;
    Ok(vault::doc("bookmarks", id, ""))
}

/// Bookmarks can be kept on a note in a reading book too: marking a place doesn't change it.
pub fn save_bookmarks(id: &str, mut marks: Vec<Bookmark>) -> Result<(), String> {
    marks.truncate(MAX_BOOKMARKS);
    let clip = |s: &mut String, n: usize| {
        if let Some((i, _)) = s.char_indices().nth(n) {
            s.truncate(i);
        }
    };
    for m in &mut marks {
        clip(&mut m.label, MAX_LABEL);
        m.author = clean_line(&m.author, 80);
        clip(&mut m.quote, MAX_QUOTE);
        clip(&mut m.prefix, 64);
        clip(&mut m.suffix, 64);
        if !(m.color.len() == 3 && m.color.starts_with('c') && matches!(m.color[1..].parse::<u8>(), Ok(1..=15))) {
            m.color.clear();
        }
    }
    vault::write(|conn| {
        get(conn, id)?;
        if marks.is_empty() { vault::drop_doc(conn, "bookmarks", id, "") } else { vault::put_doc(conn, "bookmarks", id, "", &marks) }
    })
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
    check_id(id)?;
    Ok(dir().join(format!("{id}.files")))
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
    crate::books::protect(id)?;
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
    crate::sealed::write(&dir.join(&file), bytes)?;
    let image = image.is_some() && media_type(&name) != "application/octet-stream";
    Ok(Kept { file, name, size: bytes.len(), image })
}

/// Fetches a picture from the web, once, and keeps it with the note.
pub fn import_image(id: &str, url: &str) -> Result<Kept, String> {
    crate::books::protect(id)?;
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

/// A note's versions as they were kept before the vault, in `<id>.versions.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct VersionsFile {
    versions: Vec<Version>,
}

/// The note's versions, oldest first; `newest` gives only the last.
fn kept_versions(conn: &Connection, id: &str, newest: bool) -> Result<Vec<Version>, String> {
    let sql = if newest { "SELECT json FROM versions WHERE note = ?1 ORDER BY n DESC LIMIT 1" } else { "SELECT json FROM versions WHERE note = ?1 ORDER BY n" };
    let rows: Vec<String> = conn.prepare_cached(sql).and_then(|mut s| s.query_map([id], |r| r.get(0))?.collect()).map_err(e)?;
    Ok(rows.iter().filter_map(|json| serde_json::from_str(json).ok()).collect())
}

fn put_version(conn: &Connection, id: &str, version: &Version) -> Result<(), String> {
    let json = serde_json::to_string(version).map_err(|e| e.to_string())?;
    conn.prepare_cached("INSERT OR REPLACE INTO versions(note, n, json) VALUES(?1, ?2, ?3)").and_then(|mut s| s.execute((id, version.n, json))).map_err(e)?;
    Ok(())
}

/// The number of the version that holds this text: the newest when it already does, else the one it will
/// be kept as. 0 for an empty note, which isn't a version.
fn number(newest: Option<&Version>, note: &Note) -> u32 {
    match newest {
        Some(v) if v.title == note.title && v.body == note.body => v.n,
        _ if note.title.trim().is_empty() && note.body.trim().is_empty() => 0,
        // Numbers count up and are never reused: the newest version is never the one dropped.
        _ => newest.map_or(0, |v| v.n) + 1,
    }
}

/// Keeps the note as a version unless the newest version already is it. Gives the number of the
/// version that holds this text, and whether one was added.
fn keep(conn: &Connection, note: &Note, by: &str, label: &str, at: u64) -> Result<(u32, bool), String> {
    let newest = kept_versions(conn, &note.id, true)?.pop();
    let n = number(newest.as_ref(), note);
    if n == 0 || newest.as_ref().is_some_and(|v| n <= v.n) {
        return Ok((n, false));
    }
    put_version(
        conn,
        &note.id,
        &Version { n, at, title: note.title.clone(), body: note.body.clone(), format: note.format.clone(), doc: note.doc.clone(), by: by.into(), label: label.into() },
    )?;
    conn.execute(
        "DELETE FROM versions WHERE note = ?1 AND n NOT IN (SELECT n FROM versions WHERE note = ?1 ORDER BY n DESC LIMIT ?2)",
        (&note.id, MAX_VERSIONS as i64),
    )
    .map_err(e)?;
    Ok((n, true))
}

/// The note's version number as it is now, as shown beside its title on a phone. Keeps nothing.
pub fn version_number(id: &str) -> Result<u32, String> {
    vault::with(|conn| {
        let note = get(conn, id)?;
        Ok(number(kept_versions(conn, id, true)?.first(), &note))
    })
}

/// The note's versions, oldest first. The note as it is now is always the last of them.
pub fn versions(id: &str) -> Result<Vec<Version>, String> {
    crate::books::protect(id)?;
    vault::write(|conn| {
        let note = get(conn, id)?;
        keep(conn, &note, "you", "", note.updated)?;
        kept_versions(conn, id, false)
    })
}

/// The newest version that holds the note's text as it is now, if one does. Unlike `versions`, keeps nothing.
pub fn current_version(id: &str) -> Result<Option<u32>, String> {
    vault::with(|conn| {
        let note = get(conn, id)?;
        Ok(kept_versions(conn, id, false)?.iter().rev().find(|v| v.title == note.title && v.body == note.body).map(|v| v.n))
    })
}

/// Replaces the note's text with the assistant's, as a new version. The text before it is kept as a
/// version too. Gives the new version's number.
pub fn rewrite(id: &str, body: String, label: &str) -> Result<u32, String> {
    crate::books::protect(id)?;
    vault::write(|conn| {
        let mut note = get(conn, id)?;
        keep(conn, &note, "you", "", note.updated)?;
        let now = library::now();
        // The assistant writes Markdown, which is the note from here on; the editor reads it back in.
        note.body = body;
        note.format = "md".into();
        note.doc = None;
        note.updated = now;
        let (n, _) = keep(conn, &note, "ai", label, now)?;
        put(conn, &note)?;
        Ok(n)
    })
}

/// Puts dictated words on the end of the note, as their own paragraph: for a dictation that went on
/// after its page was left, so nothing said is lost.
pub fn append_dictation(id: &str, words: &str) -> Result<(), String> {
    let words = words.trim();
    if words.is_empty() {
        return Ok(());
    }
    crate::books::protect(id)?;
    vault::write(|conn| {
        let mut note = get(conn, id)?;
        let body = note.body.trim_end();
        note.body = if body.is_empty() { words.to_string() } else { format!("{body}\n\n{words}") };
        // The editor's document doesn't have these words; it reads the body back in.
        note.doc = None;
        note.updated = library::now();
        put(conn, &note)
    })
}

/// Makes an earlier version the note again, as a new version, so nothing in between is lost.
pub fn restore(id: &str, n: u32) -> Result<Note, String> {
    crate::books::protect(id)?;
    vault::write(|conn| {
        let mut note = get(conn, id)?;
        keep(conn, &note, "you", "", note.updated)?;
        let old = kept_versions(conn, id, false)?.into_iter().find(|v| v.n == n).ok_or("That version is no longer kept.")?;
        let now = library::now();
        note.title = old.title;
        note.body = old.body;
        note.format = old.format;
        note.doc = old.doc;
        note.updated = now;
        keep(conn, &note, "restore", &format!("Version {n} brought back"), now)?;
        put(conn, &note)?;
        Ok(note)
    })
}

// ---------------------------------------------------------------------------------------------------------
// Notes kept as files, before the vault

/// A note file found in a notes folder, or None for anything else kept there.
///
/// Analyses (`<id>.analysis.json`), bookmarks and half-written saves sit beside the notes, and every
/// field of `Note` has a default, so they would otherwise parse as a blank note dated 1970. Only
/// `<id>.json` with a plain id counts, and the note takes its id from its file name.
fn read_entry(file: &std::path::Path) -> Option<Note> {
    if file.extension()? != "json" {
        return None;
    }
    let id = file.file_stem()?.to_str()?;
    check_id(id).ok()?;
    let mut note: Note = library::read_json(file).ok()?;
    // Any JSON object reads as a note, as every field has a default: a file with neither title nor text
    // (a key file, a setting) isn't taken for one.
    if note.title.trim().is_empty() && note.body.trim().is_empty() {
        return None;
    }
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

/// What bringing one note's files in did.
#[derive(Debug, PartialEq, Eq)]
pub enum Came {
    /// It's in the vault now, with everything kept beside it.
    New,
    /// A note of that id was already in the vault, and stands as it is.
    Had,
}

/// Runs `f` as one step inside whatever transaction is open (a savepoint): all of it, or none, without
/// ending the transaction around it. Outside one, it is a transaction of its own.
pub fn step<T>(conn: &Connection, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    conn.execute_batch("SAVEPOINT step").map_err(e)?;
    match f() {
        Ok(out) => {
            conn.execute_batch("RELEASE step").map_err(e)?;
            Ok(out)
        }
        Err(why) => {
            let _ = conn.execute_batch("ROLLBACK TO step; RELEASE step");
            Err(why)
        }
    }
}

/// Brings one note's files into the vault: the note, its versions, chat, analyses, translations and
/// bookmarks, all or none. Gives what happened, and the files that are then no longer needed.
fn bring_in(conn: &Connection, file: &std::path::Path, note: &Note) -> Result<(Came, Vec<PathBuf>), String> {
    step(conn, || bring_in_step(conn, file, note))
}

fn bring_in_step(tx: &Connection, file: &std::path::Path, note: &Note) -> Result<(Came, Vec<PathBuf>), String> {
    let read = |path: &std::path::Path| library::read_json::<serde_json::Value>(path).ok();
    let mut done = vec![file.to_path_buf()];
    // A note already in the vault was brought in before, and has been edited there since.
    let known = exists(&tx, &note.id);
    if !known {
        put(&tx, note)?;
    }
    let versions = file.with_extension("versions.json");
    if let Ok(kept) = library::read_json::<VersionsFile>(&versions) {
        for version in kept.versions.iter().filter(|_| !known) {
            put_version(&tx, &note.id, version)?;
        }
        done.push(versions);
    }
    for kind in ["chat", "analysis", "bookmarks"] {
        let path = file.with_extension(format!("{kind}.json"));
        if let Some(value) = read(&path) {
            if !known {
                vault::put_doc(&tx, kind, &note.id, "", &value)?;
            }
            done.push(path);
        }
    }
    let translations = file.with_extension("translations");
    let mut unread = false;
    for entry in std::fs::read_dir(&translations).into_iter().flatten().flatten() {
        let path = entry.path();
        match (path.file_stem().and_then(|s| s.to_str()), read(&path)) {
            (Some(language), Some(value)) if path.extension().is_some_and(|x| x == "json") => {
                if !known {
                    vault::put_doc(&tx, "translation", &note.id, language, &value)?;
                }
                done.push(path);
            }
            _ => unread = true,
        }
    }
    // What was written is read back before anything is removed.
    let back = get(&tx, &note.id)?;
    let same = known || serde_json::to_value(&back).ok() == serde_json::to_value(note).ok();
    if !same {
        return Err("it didn't read back the same".into());
    }
    if !unread && translations.is_dir() {
        done.push(translations);
    }
    Ok((if known { Came::Had } else { Came::New }, done))
}

/// What was found bringing notes in from a folder.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Brought {
    /// Notes now in the vault that weren't.
    pub added: usize,
    /// Notes found that the vault already had, left as they are there.
    pub had: usize,
    /// Notes that couldn't be read or written, by id; their files are left where they are.
    pub failed: Vec<String>,
}

/// Brings the notes kept as files in `folder` into the vault. With `consume`, as when this version first
/// runs on this device's own notes folder, each note's files are deleted once it's safely in (or, where the
/// vault isn't sealed, moved to `before-vault`). Otherwise, as for a copy from a backup or another computer,
/// the folder is only read. Kept pictures and files come along, sealed.
///
/// Every note goes in as a step of one transaction, saved to disk once at the end: a library of thousands
/// comes in in moments even on a spinning disk, where saving each note on its own would take minutes. No
/// file is removed until that one save has finished.
pub fn bring_in_folder(conn: &Connection, folder: &std::path::Path, consume: bool) -> Brought {
    let mut out = Brought::default();
    let files: Vec<PathBuf> = std::fs::read_dir(folder).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let Ok(tx) = conn.unchecked_transaction() else { return out };
    let mut landed: Vec<(String, Came, Vec<PathBuf>)> = Vec::new();
    for file in files {
        let Some(note) = read_entry(&file) else { continue };
        match bring_in(&tx, &file, &note) {
            Ok((came, done)) => landed.push((note.id.clone(), came, done)),
            Err(why) => {
                eprintln!("The note {} wasn't brought into the vault: {why}", note.id);
                out.failed.push(note.id.clone());
            }
        }
    }
    if let Err(why) = tx.commit() {
        eprintln!("The notes weren't brought into the vault: {why}");
        out.failed.extend(landed.into_iter().map(|(id, _, _)| id));
        return out;
    }
    for (id, came, done) in landed {
        match came {
            Came::New => out.added += 1,
            Came::Had => out.had += 1,
        }
        if !consume {
            // A note's pictures and files come with it, when the folder isn't this device's own.
            if came == Came::New {
                copy_kept_files(&folder.join(format!("{id}.files")), &id);
            }
            continue;
        }
        for path in done {
            let gone = if vault::SEALED {
                if path.is_dir() { std::fs::remove_dir(&path) } else { std::fs::remove_file(&path) }
            } else {
                let aside = folder.join("before-vault");
                std::fs::create_dir_all(&aside).and_then(|_| std::fs::rename(&path, aside.join(path.file_name().unwrap_or_default())))
            };
            if let Err(why) = gone {
                eprintln!("{} is in the vault but its file is still there: {why}", path.display());
            }
        }
    }
    out
}

/// Copies a note's kept files from another folder into its own, sealed as they're written.
fn copy_kept_files(from: &std::path::Path, id: &str) {
    let Ok(to) = files_dir(id) else { return };
    for entry in std::fs::read_dir(from).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(name) = path.file_name() else { continue };
        if !path.is_file() || to.join(name).exists() {
            continue;
        }
        let copied = crate::sealed::read(&path).and_then(|bytes| {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            crate::sealed::write(&to.join(name), &bytes)
        });
        if let Err(why) = copied {
            eprintln!("{} wasn't brought in: {why}", path.display());
        }
    }
}

/// Brings this device's notes still kept as files into the vault, as when this version first runs. Where
/// the vault is sealed the files are then deleted, since they are the notes in the clear; elsewhere they
/// are moved to `notes/before-vault`.
pub fn bring_in_files(conn: &Connection) -> Brought {
    bring_in_folder(conn, &dir(), true)
}

/// How many notes are still kept as files in this device's notes folder, waiting to be brought in.
pub fn files_waiting() -> usize {
    std::fs::read_dir(dir()).into_iter().flatten().flatten().filter(|e| read_entry(&e.path()).is_some()).count()
}

fn listed(conn: &Connection, query: &str) -> Result<Vec<NoteSummary>, String> {
    conn.prepare_cached(
        "SELECT id, title, preview, created, updated, words, archived, status FROM notes
         WHERE ?1 = '' OR instr(search, ?1) > 0 ORDER BY updated DESC",
    )
    .and_then(|mut s| {
        s.query_map([query], |r| {
            Ok(NoteSummary {
                id: r.get(0)?,
                title: r.get(1)?,
                preview: r.get(2)?,
                created: r.get::<_, i64>(3)? as u64,
                updated: r.get::<_, i64>(4)? as u64,
                words: r.get::<_, i64>(5)? as usize,
                archived: r.get(6)?,
                status: r.get(7)?,
            })
        })?
        .collect()
    })
    .map_err(e)
}

/// The notes, newest first; those whose title or text has `query` in it when one is given. Empty while
/// the vault is shut.
pub fn list(query: &str) -> Vec<NoteSummary> {
    let query = query.trim().to_lowercase();
    vault::with(|conn| listed(conn, &query)).unwrap_or_default()
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
        let marks = write("n5-1.bookmarks.json", r#"[]"#);
        let partial = write("n5-1.json.part", r#"{"id":"n5-1"}"#);
        let idless = write("n7-2.json", r#"{"title":"Lost its id"}"#);

        let found = read_entry(&note).unwrap();
        assert_eq!((found.id.as_str(), found.created, found.updated), ("n5-1", 5, 6));
        assert!(read_entry(&analysis).is_none());
        assert!(read_entry(&marks).is_none());
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
        let conn = vault::memory();
        let note = |body: &str| Note { id: "n1".into(), body: body.into(), ..Default::default() };
        let keep = |body: &str, by: &str, label: &str, at: u64| keep(&conn, &note(body), by, label, at).unwrap();
        let number = |body: &str| number(kept_versions(&conn, "n1", true).unwrap().first(), &note(body));
        assert_eq!(keep("", "you", "", 1), (0, false), "an empty note isn't a version");
        assert_eq!(number("first"), 1, "the number a change will be kept as");
        assert_eq!(keep("first", "you", "", 1), (1, true));
        assert_eq!(number("first"), 1);
        assert_eq!(keep("first", "you", "", 2), (1, false), "unchanged text isn't kept twice");
        assert_eq!(keep("shorter", "ai", "Make it shorter", 3), (2, true));
        assert_eq!(keep("first", "restore", "Version 1 brought back", 4), (3, true));
        let all = kept_versions(&conn, "n1", false).unwrap();
        assert_eq!(all.iter().map(|v| v.by.as_str()).collect::<Vec<_>>(), ["you", "ai", "restore"]);
        for i in 0..MAX_VERSIONS + 5 {
            keep(&format!("text {i}"), "you", "", 5);
        }
        let all = kept_versions(&conn, "n1", false).unwrap();
        assert_eq!(all.len(), MAX_VERSIONS);
        assert_eq!(all.last().unwrap().n as usize, MAX_VERSIONS + 8, "numbers are never reused");
        assert_eq!(all.first().unwrap().n, 9, "the oldest are the ones dropped");
        assert!(kept_versions(&conn, "n2", false).unwrap().is_empty(), "another note's versions are its own");
    }

    #[test]
    fn only_plain_ids_are_accepted() {
        assert!(check_id("n123-4").is_ok());
        assert!(check_id("../prefs").is_err());
        assert!(check_id("").is_err());
    }

    #[test]
    fn the_list_and_search_read_what_was_saved() {
        let conn = vault::memory();
        let note = |id: &str, title: &str, body: &str, updated: u64| Note { id: id.into(), title: title.into(), body: body.into(), updated, format: "md".into(), ..Default::default() };
        put(&conn, &note("n1", "", "## Offsite **plan**\n\nBook the ÉCOLE hall", 5)).unwrap();
        put(&conn, &note("n2", "Groceries", "- [ ] milk", 9)).unwrap();
        let all = listed(&conn, "").unwrap();
        assert_eq!(all.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["n2", "n1"], "newest first");
        assert_eq!((all[1].title.as_str(), all[1].preview.as_str(), all[1].words), ("Offsite plan", "Offsite plan Book the ÉCOLE hall", 7));
        // Search ignores case, accents included, and reads titles as well as text.
        assert_eq!(listed(&conn, "école").unwrap().len(), 1);
        assert_eq!(listed(&conn, "groc").unwrap()[0].id, "n2");
        assert!(listed(&conn, "absent").unwrap().is_empty());
        // Saving again replaces the row rather than adding one.
        put(&conn, &note("n1", "Renamed", "x", 12)).unwrap();
        assert_eq!(listed(&conn, "").unwrap().iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Renamed", "Groceries"]);
        assert_eq!(get(&conn, "n1").unwrap().body, "x");
        assert!(get(&conn, "n9").is_err());
    }

    #[test]
    fn a_note_kept_as_files_is_brought_in_whole() {
        let folder = std::env::temp_dir().join(format!("stv-notes-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("n5-1.translations")).unwrap();
        let write = |name: &str, text: &str| std::fs::write(folder.join(name), text).unwrap();
        write(
            "n5-1.json",
            r#"{"id":"n5-1","title":"Plan","body":"Ship on Friday","created":5,"updated":6,"format":"md","doc":{"type":"doc"},"commentary":"Why Friday","pronounce":{"Phocion":"Foe-shee-on"}}"#,
        );
        write("n5-1.versions.json", r#"{"last":2,"versions":[{"n":1,"at":5,"body":"Ship","by":"you"},{"n":2,"at":6,"body":"Ship on Friday","by":"ai","label":"Say when"}]}"#);
        write("n5-1.chat.json", r#"{"messages":[{"role":"user","text":"When?"}]}"#);
        write("n5-1.analysis.json", r#"{"stack":[{"template":"summary"}]}"#);
        write("n5-1.bookmarks.json", r#"[{"id":"m1","at":0,"quote":"Ship","label":"Start","color":"c03","created":7}]"#);
        write("n5-1.translations/fr.json", r#"{"language":"fr","updated":7}"#);
        let file = folder.join("n5-1.json");
        let note = read_entry(&file).unwrap();

        let conn = vault::memory();
        let (came, done) = bring_in(&conn, &file, &note).unwrap();
        assert_eq!(came, Came::New);
        assert_eq!(done.len(), 7, "the note, its versions, chat, analyses, bookmarks, one translation and their folder: {done:?}");
        let back = get(&conn, "n5-1").unwrap();
        assert_eq!((back.title.as_str(), back.body.as_str(), back.created, back.doc.is_some()), ("Plan", "Ship on Friday", 5, true));
        assert_eq!((back.commentary.as_str(), back.pronounce.get("Phocion").map(String::as_str)), ("Why Friday", Some("Foe-shee-on")));
        let versions = kept_versions(&conn, "n5-1", false).unwrap();
        assert_eq!(versions.iter().map(|v| (v.n, v.by.as_str())).collect::<Vec<_>>(), [(1, "you"), (2, "ai")]);
        let chat: serde_json::Value = vault::get_doc(&conn, "chat", "n5-1", "").unwrap();
        assert_eq!(chat["messages"][0]["text"], "When?");
        assert!(vault::get_doc::<serde_json::Value>(&conn, "analysis", "n5-1", "").is_some());
        let marks: Vec<Bookmark> = vault::get_doc(&conn, "bookmarks", "n5-1", "").unwrap();
        assert_eq!((marks[0].label.as_str(), marks[0].color.as_str()), ("Start", "c03"));
        assert_eq!(vault::doc_names(&conn, "translation", "n5-1"), ["fr"]);

        // Brought in a second time, as when its files couldn't be removed, what's in the vault stands.
        put(&conn, &Note { body: "Edited since".into(), ..back }).unwrap();
        assert_eq!(bring_in(&conn, &file, &note).unwrap().0, Came::Had);
        assert_eq!(get(&conn, "n5-1").unwrap().body, "Edited since");
        assert_eq!(kept_versions(&conn, "n5-1", false).unwrap().len(), 2);

        // A copy from elsewhere is only read: its files stay, and the count says what came in.
        let other = vault::memory();
        let brought = bring_in_folder(&other, &folder, false);
        assert_eq!((brought.added, brought.had, brought.failed.len()), (1, 0, 0));
        assert!(file.exists(), "a folder that isn't this device's own is left as it was");
        assert_eq!(bring_in_folder(&other, &folder, false).had, 1);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
