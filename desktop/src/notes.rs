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
    /// The language it's in, as a code ("cy"); empty for the person's primary language.
    pub language: String,
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

/// The first line stands in for a title the person never typed.
pub fn display_title(note: &Note) -> String {
    let title = note.title.trim();
    if !title.is_empty() {
        return title.into();
    }
    let first = note.body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
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

/// Saves the note, creating it when `id` is None, and returns its id.
pub fn save(id: Option<String>, title: String, body: String, keep_audio: bool) -> Result<String, String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let now = library::now();
    let mut note = match &id {
        Some(id) => load(id)?,
        None => Note {
            id: format!("n{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7)),
            created: now,
            ..Default::default()
        },
    };
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

/// Like a status, a language doesn't count as an edit.
pub fn set_language(id: &str, language: &str) -> Result<(), String> {
    let mut note = load(id)?;
    note.language = crate::prefs::check_language(language)?;
    library::write_json(&path(id)?, &note)
}

pub fn delete(id: &str) -> Result<(), String> {
    remove_audio(Some(id));
    std::fs::remove_file(path(id)?).map_err(|e| e.to_string())
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
            let preview: String = n.body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
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
    fn only_plain_ids_are_accepted() {
        assert!(path("n123-4").is_ok());
        assert!(path("../prefs").is_err());
        assert!(path("").is_err());
    }
}
