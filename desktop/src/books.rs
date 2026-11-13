//! Books: a named, ordered list of notes. A book is its own JSON file pointing at note ids; the notes
//! stay where they are, so a note can be in more than one book and removing it from one keeps the note.

use crate::{library, notes};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    /// The notes in it, in reading order.
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BookSummary {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub notes: Vec<String>,
}

/// One note of a book as it is read: the note, or a gap where it was deleted from elsewhere.
#[derive(Debug, Serialize)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub body: String,
    pub format: String,
    pub updated: u64,
    pub words: usize,
}

fn dir() -> PathBuf {
    library::data_dir().join("books")
}

fn path(id: &str) -> Result<PathBuf, String> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(dir().join(format!("{id}.json")))
    } else {
        Err("That book no longer exists.".into())
    }
}

fn title_of(book: &Book) -> String {
    if book.title.trim().is_empty() { "Untitled book".into() } else { book.title.trim().to_string() }
}

pub fn load(id: &str) -> Result<Book, String> {
    let mut book: Book = library::read_json(&path(id)?).map_err(|_| "That book no longer exists.".to_string())?;
    book.id = id.to_string();
    Ok(book)
}

fn write(book: &mut Book, touch: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    if touch {
        book.updated = library::now();
    }
    library::write_json(&path(&book.id)?, book)
}

pub fn create(title: String) -> Result<String, String> {
    let now = library::now();
    let mut book = Book { id: format!("b{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7)), title, created: now, updated: now, notes: vec![] };
    write(&mut book, false)?;
    Ok(book.id)
}

pub fn rename(id: &str, title: String) -> Result<(), String> {
    let mut book = load(id)?;
    book.title = title;
    write(&mut book, true)
}

/// Adds a note at the end, once.
pub fn add_note(id: &str, note: &str) -> Result<(), String> {
    notes::load(note)?;
    let mut book = load(id)?;
    if !book.notes.iter().any(|n| n == note) {
        book.notes.push(note.to_string());
    }
    write(&mut book, true)
}

pub fn remove_note(id: &str, note: &str) -> Result<(), String> {
    let mut book = load(id)?;
    book.notes.retain(|n| n != note);
    write(&mut book, true)
}

/// Puts the book's notes in this order. Only notes already in it are kept; any left out go at the end,
/// so a reorder can't drop a note.
pub fn reorder(id: &str, order: Vec<String>) -> Result<(), String> {
    let mut book = load(id)?;
    let mut next: Vec<String> = Vec::new();
    for n in order.into_iter().chain(book.notes.clone()) {
        if book.notes.contains(&n) && !next.contains(&n) {
            next.push(n);
        }
    }
    book.notes = next;
    write(&mut book, true)
}

/// Deletes the book only; its notes stay.
pub fn delete(id: &str) -> Result<(), String> {
    std::fs::remove_file(path(id)?).map_err(|e| e.to_string())
}

fn all() -> Vec<Book> {
    std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let id = p.file_name()?.to_str()?.strip_suffix(".json")?.to_string();
            if id.contains('.') {
                return None;
            }
            load(&id).ok()
        })
        .collect()
}

/// Every book, newest change first. With a query, those whose title matches.
pub fn list(query: &str) -> Vec<BookSummary> {
    let query = query.trim().to_lowercase();
    let mut books: Vec<Book> = all().into_iter().filter(|b| query.is_empty() || b.title.to_lowercase().contains(&query)).collect();
    books.sort_by(|a, b| b.updated.cmp(&a.updated));
    books.into_iter().map(|b| BookSummary { title: title_of(&b), id: b.id, created: b.created, updated: b.updated, notes: b.notes }).collect()
}

/// The book's notes in order, as they read. Notes deleted since are left out.
pub fn chapters(id: &str) -> Result<Vec<Chapter>, String> {
    Ok(load(id)?
        .notes
        .iter()
        .filter_map(|n| notes::load(n).ok())
        .map(|n| Chapter { title: notes::display_title(&n), words: n.body.split_whitespace().count(), id: n.id, body: n.body, format: n.format, updated: n.updated })
        .collect())
}

/// Takes a deleted note out of every book it was in.
pub fn forget_note(note: &str) {
    for mut book in all() {
        if book.notes.iter().any(|n| n == note) {
            book.notes.retain(|n| n != note);
            let _ = write(&mut book, false);
        }
    }
}
