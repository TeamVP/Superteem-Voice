//! Books: a named, ordered list of notes. A book is its own JSON file pointing at note ids; the notes
//! stay where they are, so a note can be in more than one book and removing it from one keeps the note.
//! `parts` groups those notes under headings such as "Book 1" and "Chapter 2". A book without parts is
//! still a flat list.

use crate::{library, notes};
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The example book, compiled into the app. Android's resource path is `asset://localhost/`, which is
/// not a folder, so a copy from `resource_dir` never ran there and the book never appeared.
static EXAMPLE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/resources/example-books");

fn editable_by_default() -> bool {
    true
}

/// One life inside a part: the note, and the label the filename's second number became ("Chapter 2").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookChapter {
    pub note: String,
    /// "Chapter 2", or empty when this note isn't a numbered chapter.
    pub label: String,
}

/// A group of chapters, from the filename's first number ("Book 1") or a front piece such as "Preface".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookPart {
    pub title: String,
    pub chapters: Vec<BookChapter>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    /// The notes in it, in reading order. When `parts` is set, this follows that order.
    pub notes: Vec<String>,
    /// Book and chapter labels. Empty for a book that is only a list of notes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<BookPart>,
    /// False for a book that is only read: its chapters stay put, and nothing writes to its notes.
    /// Missing in a book saved before this meant it can be edited.
    #[serde(default = "editable_by_default")]
    pub editable: bool,
}

#[derive(Debug, Serialize)]
pub struct BookSummary {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub notes: Vec<String>,
    pub editable: bool,
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
    /// "Book 1" when the note sits in a part; empty for a flat list.
    pub part: String,
    /// "Chapter 2" within that part; empty when the note has no chapter number.
    pub label: String,
}

/// A chapter as the contents list shows it, without the text.
#[derive(Debug, Serialize)]
pub struct ChapterHead {
    pub id: String,
    pub title: String,
    pub part: String,
    pub label: String,
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
    align(&mut book);
    Ok(book)
}

/// Reading order follows the parts. A note listed only in `notes` stays at the end, so adding one
/// doesn't drop it into a numbered book.
fn align(book: &mut Book) {
    if book.parts.is_empty() {
        return;
    }
    let mut notes = Vec::new();
    for part in &book.parts {
        for chapter in &part.chapters {
            if !chapter.note.is_empty() && !notes.contains(&chapter.note) {
                notes.push(chapter.note.clone());
            }
        }
    }
    for note in book.notes.clone() {
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
    book.notes = notes;
}

fn place(book: &Book, id: &str) -> (String, String) {
    for part in &book.parts {
        if let Some(chapter) = part.chapters.iter().find(|c| c.note == id) {
            return (part.title.clone(), chapter.label.clone());
        }
    }
    (String::new(), String::new())
}

fn drop_note(book: &mut Book, note: &str) {
    book.notes.retain(|n| n != note);
    for part in &mut book.parts {
        part.chapters.retain(|c| c.note != note);
    }
    book.parts.retain(|p| !p.chapters.is_empty());
}

fn write(book: &mut Book, touch: bool) -> Result<(), String> {
    align(book);
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    if touch {
        book.updated = library::now();
    }
    library::write_json(&path(&book.id)?, book)
}

pub fn create(title: String) -> Result<String, String> {
    let now = library::now();
    let mut book = Book { id: format!("b{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7)), title, created: now, updated: now, notes: vec![], parts: vec![], editable: true };
    write(&mut book, false)?;
    Ok(book.id)
}

fn reading_only(book: &Book) -> Result<(), String> {
    if book.editable { Ok(()) } else { Err("This book is for reading.".into()) }
}

/// Refuses a change to a note that belongs to a book kept for reading.
pub fn protect(note: &str) -> Result<(), String> {
    if all().iter().any(|b| !b.editable && b.notes.iter().any(|n| n == note)) {
        Err("This book is for reading.".into())
    } else {
        Ok(())
    }
}

/// The reading book a note belongs to, when one is keeping it from being changed.
pub fn book_for_note(note: &str) -> Option<String> {
    all().into_iter().find(|b| !b.editable && b.notes.iter().any(|n| n == note)).map(|b| b.id)
}

pub fn rename(id: &str, title: String) -> Result<(), String> {
    let mut book = load(id)?;
    reading_only(&book)?;
    book.title = title;
    write(&mut book, true)
}

/// Adds a note at the end, once.
pub fn add_note(id: &str, note: &str) -> Result<(), String> {
    notes::load(note)?;
    let mut book = load(id)?;
    reading_only(&book)?;
    if !book.notes.iter().any(|n| n == note) {
        book.notes.push(note.to_string());
    }
    write(&mut book, true)
}

pub fn remove_note(id: &str, note: &str) -> Result<(), String> {
    let mut book = load(id)?;
    reading_only(&book)?;
    drop_note(&mut book, note);
    write(&mut book, true)
}

/// Puts the book's notes in this order. Only notes already in it are kept; any left out go at the end,
/// so a reorder can't drop a note.
pub fn reorder(id: &str, order: Vec<String>) -> Result<(), String> {
    let mut book = load(id)?;
    reading_only(&book)?;
    let mut next: Vec<String> = Vec::new();
    for n in order.into_iter().chain(book.notes.clone()) {
        if book.notes.contains(&n) && !next.contains(&n) {
            next.push(n);
        }
    }
    book.notes = next;
    if !book.parts.is_empty() {
        for part in &mut book.parts {
            part.chapters.sort_by_key(|c| book.notes.iter().position(|n| n == &c.note).unwrap_or(usize::MAX));
        }
        book.parts.sort_by_key(|p| p.chapters.iter().filter_map(|c| book.notes.iter().position(|n| n == &c.note)).min().unwrap_or(usize::MAX));
        align(&mut book);
    }
    write(&mut book, true)
}

/// Deletes the book only; its notes stay.
pub fn delete(id: &str) -> Result<(), String> {
    let book = load(id)?;
    reading_only(&book)?;
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
    books.into_iter().map(|b| BookSummary { title: title_of(&b), id: b.id, created: b.created, updated: b.updated, notes: b.notes, editable: b.editable }).collect()
}

/// The book's notes in order, as they read. Notes deleted since are left out.
pub fn chapters(id: &str) -> Result<Vec<Chapter>, String> {
    let book = load(id)?;
    let loaded: Vec<_> = book.notes.iter().filter_map(|n| notes::load(n).ok()).collect();
    Ok(loaded
        .into_iter()
        .map(|n| {
            let (part, label) = place(&book, &n.id);
            Chapter { title: notes::display_title(&n), words: n.body.split_whitespace().count(), id: n.id, body: n.body, format: n.format, updated: n.updated, part, label }
        })
        .collect())
}

/// The book's notes in order, without their text, for the contents list.
pub fn contents(id: &str) -> Result<Vec<ChapterHead>, String> {
    let book = load(id)?;
    let loaded: Vec<_> = book.notes.iter().filter_map(|n| notes::load(n).ok()).collect();
    Ok(loaded
        .into_iter()
        .map(|n| {
            let (part, label) = place(&book, &n.id);
            ChapterHead { words: n.body.split_whitespace().count(), title: notes::display_title(&n), id: n.id, part, label }
        })
        .collect())
}

/// Takes a deleted note out of every book it was in. A book kept for reading holds onto its chapters.
pub fn forget_note(note: &str) {
    for mut book in all() {
        if book.editable && book.notes.iter().any(|n| n == note) {
            drop_note(&mut book, note);
            let _ = write(&mut book, false);
        }
    }
}

/// Copies the example book into this device's library, and stamps it with now so it shows as just
/// added. `resource_dir` is unused: the book is compiled in, which is the only place Android can read it.
pub fn install_example(_resource_dir: Option<&std::path::Path>) {
    if crate::prefs::load().example_books >= EXAMPLE_BOOK {
        return;
    }
    if let Err(e) = install_bundled(library::now()) {
        eprintln!("The example book wasn't added: {e}");
        return;
    }
    let _ = crate::prefs::update(|p| p.example_books = EXAMPLE_BOOK);
}

fn install_bundled(now: u64) -> Result<(), String> {
    let file = EXAMPLE.get_file("plutarch-lives.json").ok_or("The example book is missing.")?;
    let mut book: Book = serde_json::from_slice(file.contents()).map_err(|e| e.to_string())?;
    book.id = EXAMPLE_ID.into();
    book.editable = false;
    book.created = now;
    book.updated = now;
    let notes_dir = EXAMPLE.get_dir("notes").ok_or("The example book is missing.")?;
    let notes_dst = library::data_dir().join("notes");
    std::fs::create_dir_all(&notes_dst).map_err(|e| e.to_string())?;
    for file in notes_dir.files() {
        let Some(name) = file.path().file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".json") {
            continue;
        }
        let dest = notes_dst.join(name);
        if dest.is_file() {
            continue;
        }
        let mut note: serde_json::Value = serde_json::from_slice(file.contents()).map_err(|e| e.to_string())?;
        if let Some(obj) = note.as_object_mut() {
            obj.insert("created".into(), serde_json::json!(now));
            obj.insert("updated".into(), serde_json::json!(now));
        }
        library::write_json(&dest, &note)?;
    }
    let book_file = library::data_dir().join("books").join(format!("{EXAMPLE_ID}.json"));
    std::fs::create_dir_all(book_file.parent().unwrap()).map_err(|e| e.to_string())?;
    library::write_json(&book_file, &book)
}

const EXAMPLE_BOOK: u32 = 2;
const EXAMPLE_ID: &str = "plutarch-lives";

fn example_dir(resource_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    let beside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("example-books");
    let from_app = resource_dir.map(|d| d.join("example-books"));
    from_app.into_iter().chain(std::iter::once(beside)).find(|dir| dir.join(format!("{EXAMPLE_ID}.json")).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_book_saved_before_the_flag_can_still_be_edited() {
        let book: Book = serde_json::from_str(r#"{"title":"Mine"}"#).unwrap();
        assert!(book.editable);
        let locked: Book = serde_json::from_str(r#"{"editable":false}"#).unwrap();
        assert!(!locked.editable);
    }

    #[test]
    fn parts_set_the_reading_order_and_the_labels() {
        let mut book = Book {
            notes: vec!["b".into()],
            parts: vec![BookPart {
                title: "Book 1".into(),
                chapters: vec![
                    BookChapter { note: "a".into(), label: "Chapter 1".into() },
                    BookChapter { note: "b".into(), label: "Chapter 2".into() },
                ],
            }],
            ..Default::default()
        };
        align(&mut book);
        assert_eq!(book.notes, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(place(&book, "b"), ("Book 1".into(), "Chapter 2".into()));
        drop_note(&mut book, "a");
        assert_eq!(book.notes, vec!["b".to_string()]);
        assert_eq!(book.parts[0].chapters.len(), 1);
    }

    #[test]
    fn the_example_book_reads_book_then_chapter_then_title() {
        let dir = example_dir(None).expect("the example book is beside the crate");
        let book: Book = library::read_json(&dir.join("plutarch-lives.json")).unwrap();
        assert_eq!(book.title, "Plutarch's Lives");
        assert_eq!(book.parts[0].title, "Preface");
        let theseus = &book.parts.iter().find(|p| p.title == "Book 1").unwrap().chapters;
        assert_eq!(theseus[0].label, "Chapter 1");
        assert_eq!(theseus[0].note, "plutarch-01-01-theseus");
        assert_eq!(theseus[1].label, "Chapter 2");
        assert_eq!(theseus[1].note, "plutarch-01-02-romulus");
        let note: notes::Note = library::read_json(&dir.join("notes").join("plutarch-01-01-theseus.json")).unwrap();
        assert_eq!(note.title, "Theseus");
        assert_eq!(note.format, "md");
        assert!(!note.body.contains("PROJECT GUTENBERG"));
        let ids: Vec<_> = book.parts.iter().flat_map(|p| p.chapters.iter().map(|c| c.note.clone())).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "zero-padded ids stay in reading order when sorted");
        assert!(book.parts.iter().all(|p| p.title != "Book 23"));
        assert_eq!(book.parts.iter().find(|p| p.title == "Book 17").unwrap().chapters.len(), 2);
        assert!(book.parts.iter().any(|p| p.title == "Book 24"));
        assert!(!book.editable);
    }
}
