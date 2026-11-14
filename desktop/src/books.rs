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
    /// The natural voice the book suggests reading it in (a voice id, as `tts::Voice::id`); empty for none.
    /// Offered the first time the book is read aloud on a device that has natural voices.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub voice: String,
    /// Why that voice, in a sentence, for the offer.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub voice_why: String,
    /// How to say words the voices get wrong, for this book only: each word as written ("Phocion") to how
    /// it's spelt for the voice ("Foe-shee-on"). Whole words, matching case; the page shows the words as
    /// written.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub pronounce: std::collections::BTreeMap<String, String>,
    /// Notes about the book as a whole (Markdown), hidden until asked for, then shown above its contents.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commentary: String,
    /// Who wrote it, as it should be shown ("Plutarch").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Who translated it, and from which edition, as it should be shown; empty when it isn't a translation.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub translator: String,
    /// What the book is, in a sentence or two.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The language it's written in, as a code ("en").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// Where the text came from: a web address or a description of the printed edition.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Who may copy it: "public-domain", a Creative Commons code such as "cc-by-4.0", or
    /// "all-rights-reserved"; empty when not said.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub licence: String,
    /// The cover: an image file kept in the book's own folder (`books/<id>.files/`); empty for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cover: String,
    /// Counts up each time the book's content is published again; 0 for a book that isn't published.
    #[serde(skip_serializing_if = "is_zero")]
    pub version: u32,
    /// What changed in each version, newest last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<BookChange>,
    /// The book's own files (images, audio, scans), kept in its folder and shown only from there. Chapters
    /// show one as `attachment:book/<name>`. Each is listed with its fingerprint, so a published version can't
    /// change under a reader.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<BookFile>,
    /// The web pages the book may link to. In a book kept for reading, a link to anything else is shown as
    /// plain text; these open only in the system's browser, after saying they leave the app.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<BookLink>,
}

/// One of a book's own files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookFile {
    /// Its name in the book's folder ("attica-map.jpg").
    pub name: String,
    /// What it is, as a media type ("image/jpeg").
    pub kind: String,
    pub size: u64,
    /// Its SHA-256, in hex.
    pub sha256: String,
    /// What it shows, for someone who can't see it.
    pub alt: String,
    pub caption: String,
    /// Who made it.
    pub credit: String,
    /// As a book's `licence`.
    pub licence: String,
}

/// A web page outside the library that a book links to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookLink {
    /// The address, exactly as the chapters write it. Only https:// (or mailto:).
    pub url: String,
    pub title: String,
    /// What's there, in a sentence.
    pub about: String,
    /// When the librarian last checked it said what it should ("2026-11-14"); empty when never.
    pub checked: String,
    /// A copy kept elsewhere (an archive's address), for when the page changes or goes; empty for none.
    pub archive: String,
}

/// Whether an address may be opened outside the app: a web page (https://, or http:// with a warning) or an
/// email (mailto:), whole, with nothing in it that could be taken for anything else.
pub fn outside_address(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let host_ok = match rest {
        Some(r) => r.split(['/', '?', '#']).next().is_some_and(|h| !h.is_empty() && !h.contains('@')),
        None => url.strip_prefix("mailto:").is_some_and(|m| m.contains('@')),
    };
    host_ok && url.len() <= 2048 && !url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '}' | '|'))
}

/// What's wrong with a book's files: each listed file that's missing, or whose size or fingerprint differs
/// from its listing. Empty when all is as listed.
pub fn file_problems(id: &str) -> Result<Vec<String>, String> {
    use sha2::Digest;
    let book = load(id)?;
    let mut problems = Vec::new();
    for f in &book.files {
        let Ok(bytes) = file_path(id, &f.name).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) else {
            problems.push(format!("{} is missing.", f.name));
            continue;
        };
        let hash: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if bytes.len() as u64 != f.size || !hash.eq_ignore_ascii_case(&f.sha256) {
            problems.push(format!("{} isn't the file the book lists.", f.name));
        }
    }
    Ok(problems)
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// One version of a book, in its changelog.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookChange {
    pub version: u32,
    /// "2026-11-14"; empty when not known.
    pub date: String,
    /// What changed, in a sentence or two.
    pub summary: String,
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
    /// The note's own pronunciations, read on top of the book's.
    pub pronounce: std::collections::BTreeMap<String, String>,
    /// The note's commentary (Markdown); empty for none.
    pub commentary: String,
    /// Who wrote this chapter, when not the book's author; empty for the book's own.
    pub author: String,
}

/// A chapter as the contents list shows it, without the text.
#[derive(Debug, Serialize)]
pub struct ChapterHead {
    pub id: String,
    pub title: String,
    pub part: String,
    pub label: String,
    pub words: usize,
    pub author: String,
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
    let mut book = Book { id: format!("b{}-{}", now, std::process::id() ^ (now as u32).rotate_left(7)), title, created: now, updated: now, editable: true, ..Default::default() };
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

/// Changes a book's commentary or pronunciations, whichever is given.
pub fn set_extras(id: &str, commentary: Option<String>, pronounce: Option<std::collections::BTreeMap<String, String>>) -> Result<(), String> {
    let mut book = load(id)?;
    reading_only(&book)?;
    if let Some(text) = commentary {
        book.commentary = notes::clean_commentary(text);
    }
    if let Some(words) = pronounce {
        book.pronounce = notes::clean_pronounce(words);
    }
    write(&mut book, true)
}

/// What a book says about itself, as its About panel shows and its page changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BookDetails {
    pub author: String,
    pub translator: String,
    pub description: String,
    pub language: String,
    pub source: String,
    pub licence: String,
}

pub fn set_details(id: &str, d: BookDetails) -> Result<(), String> {
    let mut book = load(id)?;
    reading_only(&book)?;
    book.author = notes::clean_line(&d.author, 200);
    book.translator = notes::clean_line(&d.translator, 300);
    book.description = notes::clean_line(&d.description, 1000);
    book.language = notes::clean_line(&d.language, 20);
    book.source = notes::clean_line(&d.source, 500);
    book.licence = notes::clean_line(&d.licence, 60);
    write(&mut book, true)
}

// ---------------------------------------------------------------------------------------------------------
// A book's bookmarks, all its chapters' at once: to pass to someone else, to take in theirs, and which to show

/// Every bookmark in a book, by chapter, as a file to give someone. Bookmarks of the person's own carry
/// `author`, the name they gave; those they imported keep who made them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BookmarkFile {
    /// Always "superteem-bookmarks", so another kind of file isn't taken for one.
    pub kind: String,
    pub format: u32,
    pub book: String,
    pub title: String,
    /// The version of the book they were made in, when it has one.
    pub version: u32,
    pub exported: u64,
    /// Chapter (note id) to its bookmarks.
    pub notes: std::collections::BTreeMap<String, Vec<notes::Bookmark>>,
}

pub const BOOKMARK_FILE_KIND: &str = "superteem-bookmarks";

/// Every bookmark in the book, for showing which colours and people there are; by chapter.
pub fn bookmarks(id: &str) -> Result<std::collections::BTreeMap<String, Vec<notes::Bookmark>>, String> {
    let book = load(id)?;
    Ok(book.notes.iter().filter_map(|n| notes::bookmarks(n).ok().filter(|m| !m.is_empty()).map(|m| (n.clone(), m))).collect())
}

/// The book's bookmarks as a file to give someone; the person's own carry `author`.
pub fn bookmark_file(id: &str, author: &str) -> Result<BookmarkFile, String> {
    let book = load(id)?;
    let author = notes::clean_line(author, 80);
    let mut all = bookmarks(id)?;
    for marks in all.values_mut() {
        for m in marks.iter_mut().filter(|m| m.author.is_empty()) {
            m.author = author.clone();
        }
    }
    Ok(BookmarkFile { kind: BOOKMARK_FILE_KIND.into(), format: 1, book: book.id.clone(), title: title_of(&book), version: book.version, exported: library::now(), notes: all })
}

/// What an import did: how many bookmarks came in, how many were already here, and how many were for
/// chapters this book doesn't have.
#[derive(Debug, Default, Serialize)]
pub struct BookmarkImport {
    pub added: usize,
    pub already: usize,
    pub elsewhere: usize,
}

/// Takes in someone's bookmarks for this book, beside the person's own. One already here (the same one, or
/// the same place, words and maker) isn't added twice. Bookmarks keep who made them; any without a maker
/// get `author`, so they never pass for the person's own.
pub fn import_bookmarks(id: &str, file: BookmarkFile, author: &str) -> Result<BookmarkImport, String> {
    if file.kind != BOOKMARK_FILE_KIND {
        return Err("That isn't a file of bookmarks.".into());
    }
    let book = load(id)?;
    let fallback = match notes::clean_line(author, 80) {
        a if a.is_empty() => "Someone else".to_string(),
        a => a,
    };
    let mut result = BookmarkImport::default();
    for (note, incoming) in file.notes {
        if !book.notes.contains(&note) {
            result.elsewhere += incoming.len();
            continue;
        }
        let mut marks = notes::bookmarks(&note)?;
        for mut m in incoming {
            if m.author.trim().is_empty() {
                m.author = fallback.clone();
            }
            let same = |x: &notes::Bookmark| x.id == m.id || (x.at == m.at && x.quote == m.quote && x.author == m.author);
            if marks.iter().any(same) {
                result.already += 1;
                continue;
            }
            if m.id.is_empty() || marks.iter().any(|x| x.id == m.id) {
                m.id = format!("i{}{}", library::now(), marks.len());
            }
            marks.push(m);
            result.added += 1;
        }
        notes::save_bookmarks(&note, marks)?;
    }
    Ok(result)
}

/// Deletes every bookmark in the book made by `author` (someone else's, from an import).
pub fn remove_bookmarks_by(id: &str, author: &str) -> Result<usize, String> {
    if author.trim().is_empty() {
        return Err("Your own bookmarks are deleted one at a time.".into());
    }
    let mut removed = 0;
    for (note, marks) in bookmarks(id)? {
        let kept: Vec<_> = marks.iter().filter(|m| m.author != author).cloned().collect();
        if kept.len() != marks.len() {
            removed += marks.len() - kept.len();
            notes::save_bookmarks(&note, kept)?;
        }
    }
    Ok(removed)
}

/// Which bookmarks a book shows: those of any colour or maker listed here are hidden. Kept beside the book,
/// as `<id>.marks-shown.json`. A colour is "c01" to "c15", or "" for none; a maker is a name, or "" for the
/// person's own.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BookmarkView {
    pub hidden_colours: Vec<String>,
    pub hidden_authors: Vec<String>,
}

fn view_path(id: &str) -> Result<PathBuf, String> {
    Ok(path(id)?.with_extension("marks-shown.json"))
}

pub fn bookmark_view(id: &str) -> BookmarkView {
    view_path(id).ok().and_then(|p| library::read_json(&p).ok()).unwrap_or_default()
}

pub fn set_bookmark_view(id: &str, view: BookmarkView) -> Result<(), String> {
    load(id)?;
    let file = view_path(id)?;
    if view == BookmarkView::default() {
        let _ = std::fs::remove_file(file);
        return Ok(());
    }
    library::write_json(&file, &view)
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
    let _ = std::fs::remove_file(place_path(id)?);
    let _ = std::fs::remove_file(view_path(id)?);
    let _ = std::fs::remove_dir_all(files_dir(id)?);
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
            Chapter {
                title: notes::display_title(&n),
                words: n.body.split_whitespace().count(),
                id: n.id,
                body: n.body,
                format: n.format,
                updated: n.updated,
                part,
                label,
                pronounce: n.pronounce,
                commentary: n.commentary,
                author: n.author,
            }
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
            ChapterHead { words: n.body.split_whitespace().count(), title: notes::display_title(&n), id: n.id, part, label, author: n.author }
        })
        .collect())
}

/// A chapter holding the words searched for, with a few of the places they are.
#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub part: String,
    pub label: String,
    /// How many times the words are in it.
    pub count: usize,
    pub snippets: Vec<Snippet>,
}

/// The words found, as they are written, and some of the line either side.
#[derive(Debug, Serialize)]
pub struct Snippet {
    pub before: String,
    pub hit: String,
    pub after: String,
}

/// The most places shown for each chapter.
const SNIPPETS_PER_CHAPTER: usize = 3;

/// Every chapter of the book holding `query`, whatever its case, in reading order.
pub fn search(id: &str, query: &str) -> Result<Vec<SearchHit>, String> {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let want: Vec<char> = query.trim().chars().map(lower).collect();
    if want.len() < 2 {
        return Ok(Vec::new());
    }
    let book = load(id)?;
    let mut hits = Vec::new();
    for n in book.notes.iter().filter_map(|n| notes::load(n).ok()) {
        let mut count = 0;
        let mut snippets = Vec::new();
        for raw in n.body.lines() {
            let line = if n.format == "md" { notes::plain_line(raw) } else { raw.trim().to_string() };
            let chars: Vec<char> = line.chars().collect();
            if chars.len() < want.len() {
                continue;
            }
            let folded: Vec<char> = chars.iter().map(|&c| lower(c)).collect();
            let mut i = 0;
            while i + want.len() <= folded.len() {
                if folded[i..i + want.len()] != want[..] {
                    i += 1;
                    continue;
                }
                count += 1;
                if snippets.len() < SNIPPETS_PER_CHAPTER {
                    let end = i + want.len();
                    // Some words either side, started and ended at a space where there is one near.
                    let mut from = i.saturating_sub(48);
                    if from > 0 {
                        from = chars[from..i].iter().position(|c| c.is_whitespace()).map_or(from, |p| from + p + 1);
                    }
                    let mut to = (end + 72).min(chars.len());
                    if to < chars.len() {
                        to = chars[end..to].iter().rposition(|c| c.is_whitespace()).map_or(to, |p| end + p);
                    }
                    snippets.push(Snippet {
                        before: format!("{}{}", if from > 0 { "…" } else { "" }, chars[from..i].iter().collect::<String>()),
                        hit: chars[i..end].iter().collect(),
                        after: format!("{}{}", chars[end..to].iter().collect::<String>(), if to < chars.len() { "…" } else { "" }),
                    });
                }
                i += want.len();
            }
        }
        if count > 0 {
            let (part, label) = place(&book, &n.id);
            hits.push(SearchHit { title: notes::display_title(&n), id: n.id, part, label, count, snippets });
        }
    }
    Ok(hits)
}

// ---------------------------------------------------------------------------------------------------------
// Where reading aloud last stopped, kept beside the book as `<id>.listened.json`. One per book: stopping again
// moves it. A book kept for reading can have one too, since it doesn't change the book.

/// The place reading aloud stopped: the chapter's note, and the place in its text found as a bookmark's is
/// (`at`, and the words either side). `quote` is the sentence that was being read, to show.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ListenPlace {
    pub note: String,
    pub at: usize,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub created: u64,
}

/// The book's own files, such as its cover.
fn files_dir(id: &str) -> Result<PathBuf, String> {
    Ok(path(id)?.with_extension("files"))
}

/// A file of the book's own, by its name: only a plain name, so nothing outside the book's folder.
pub fn file_path(id: &str, file: &str) -> Result<PathBuf, String> {
    let plain = !file.is_empty() && !file.starts_with('.') && !file.contains(['/', '\\', ':']) && file != "..";
    if !plain {
        return Err("That file isn't the book's.".into());
    }
    Ok(files_dir(id)?.join(file))
}

fn place_path(id: &str) -> Result<PathBuf, String> {
    Ok(path(id)?.with_extension("listened.json"))
}

pub fn listen_place(id: &str) -> Option<ListenPlace> {
    let place: ListenPlace = library::read_json(&place_path(id).ok()?).ok()?;
    (!place.note.is_empty()).then_some(place)
}

/// Keeps where reading aloud stopped, in place of the last; `None` forgets it.
pub fn set_listen_place(id: &str, place: Option<ListenPlace>) -> Result<(), String> {
    let book = load(id)?;
    let file = place_path(id)?;
    let Some(mut place) = place else {
        let _ = std::fs::remove_file(file);
        return Ok(());
    };
    if !book.notes.contains(&place.note) {
        return Err("That chapter isn't in this book.".into());
    }
    let clip = |s: &mut String, n: usize| {
        if let Some((i, _)) = s.char_indices().nth(n) {
            s.truncate(i);
        }
    };
    clip(&mut place.quote, 400);
    clip(&mut place.prefix, 64);
    clip(&mut place.suffix, 64);
    if place.created == 0 {
        place.created = library::now();
    }
    library::write_json(&file, &place)
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
    // A newer copy than the one installed, by the book's own version, replaces it.
    let installed = load(EXAMPLE_ID).map(|b| b.version).unwrap_or(0);
    if crate::prefs::load().example_books >= EXAMPLE_BOOK && installed >= EXAMPLE_BOOK {
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
        // A newer copy of the book replaces its notes: they are only read, so nothing of the person's is lost
        // (their bookmarks are kept beside each note).
        let dest = notes_dst.join(name);
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

/// The example book's version, which is its `version`: what changed in each is in its `changes`.
const EXAMPLE_BOOK: u32 = 7;
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
    fn the_example_books_pronunciations_are_words_in_it() {
        let dir = example_dir(None).expect("the example book is beside the crate");
        let book: Book = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{EXAMPLE_ID}.json"))).unwrap()).unwrap();
        assert!(!book.voice.is_empty() && !book.pronounce.is_empty());
        let mut text = String::new();
        for f in std::fs::read_dir(dir.join("notes")).unwrap().flatten() {
            text.push_str(&std::fs::read_to_string(f.path()).unwrap());
        }
        let words: std::collections::HashSet<&str> = text.split(|c: char| !c.is_alphabetic()).collect();
        for (word, said) in &book.pronounce {
            assert!(word.chars().all(char::is_alphabetic), "{word} is one word");
            assert!(!said.trim().is_empty() && said != word, "{word} has a spelling for the voice");
            assert!(words.contains(word.as_str()), "{word} is in the book");
        }
    }

    #[test]
    fn the_example_book_says_what_it_is_and_which_version() {
        let dir = example_dir(None).expect("the example book is beside the crate");
        let book: Book = library::read_json(&dir.join(format!("{EXAMPLE_ID}.json"))).unwrap();
        assert_eq!(book.version, EXAMPLE_BOOK, "the book's version is the one the app installs");
        assert_eq!(book.changes.last().map(|c| c.version), Some(book.version));
        assert!(book.changes.windows(2).all(|w| w[0].version < w[1].version));
        assert!(!book.author.is_empty() && !book.source.is_empty() && book.licence == "public-domain");
        let preface: notes::Note = library::read_json(&dir.join("notes").join("plutarch-00-preface.json")).unwrap();
        assert_eq!(preface.author, "A. H. Clough");
    }

    #[test]
    fn only_whole_web_and_email_addresses_go_outside() {
        for good in ["https://www.gutenberg.org/ebooks/674", "http://example.com", "mailto:someone@example.com", "https://a.b/c?d=e#f"] {
            assert!(outside_address(good), "{good}");
        }
        for bad in ["javascript:alert(1)", "file:///C:/x", "data:text/html,x", "https://", "https://a b", "https://x\"y", "ftp://x", "https://user@evil", "stvnote://localhost/x", "mailto:nobody"] {
            assert!(!outside_address(bad), "{bad}");
        }
    }

    #[test]
    fn the_example_books_links_can_go_outside() {
        let dir = example_dir(None).expect("the example book is beside the crate");
        let book: Book = library::read_json(&dir.join(format!("{EXAMPLE_ID}.json"))).unwrap();
        assert!(book.links.iter().any(|l| l.url == book.source), "its source is a link it may open");
        assert!(book.links.iter().all(|l| outside_address(&l.url) && l.url.starts_with("https://")));
    }

    #[test]
    fn a_bookmark_file_reads_back() {
        let mark = notes::Bookmark { id: "b1".into(), at: 4, quote: "Theseus".into(), author: "Alex".into(), ..Default::default() };
        let file = BookmarkFile { kind: BOOKMARK_FILE_KIND.into(), format: 1, book: "b".into(), notes: [("n".to_string(), vec![mark.clone()])].into(), ..Default::default() };
        let back: BookmarkFile = serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(back.notes["n"][0].author, "Alex");
        assert_eq!(back.notes["n"][0].quote, "Theseus");
        // One of the person's own has no author in its file.
        let own = serde_json::to_string(&notes::Bookmark { id: "b2".into(), ..Default::default() }).unwrap();
        assert!(!own.contains("author"));
    }

    #[test]
    fn a_books_files_stay_in_its_folder() {
        assert!(file_path("b1", "cover.jpg").is_ok());
        for bad in ["", "../b2.json", "a/b.jpg", "..", ".hidden", "c:x"] {
            assert!(file_path("b1", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pronunciations_as_typed_are_tidied() {
        let typed: std::collections::BTreeMap<String, String> = [
            (" Phocion ", "Foe  shee on "),
            ("two words", "x"),
            ("Same", "Same"),
            ("Empty", "  "),
            ("O'Neill", "Oh Neel"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let kept = notes::clean_pronounce(typed);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept["Phocion"], "Foe shee on");
        assert_eq!(kept["O'Neill"], "Oh Neel");
    }

    #[test]
    fn a_notes_own_pronunciations_and_the_commentary_are_there() {
        let dir = example_dir(None).expect("the example book is beside the crate");
        let book: Book = library::read_json(&dir.join(format!("{EXAMPLE_ID}.json"))).unwrap();
        assert!(!book.commentary.trim().is_empty());
        let note: notes::Note = library::read_json(&dir.join("notes").join("plutarch-01-01-theseus.json")).unwrap();
        assert!(!note.commentary.trim().is_empty());
        let words: std::collections::HashSet<&str> = note.body.split(|c: char| !c.is_alphabetic()).collect();
        assert!(!note.pronounce.is_empty());
        for (word, said) in &note.pronounce {
            assert!(words.contains(word.as_str()), "{word} is in Theseus");
            assert!(!said.trim().is_empty() && said != word && !book.pronounce.contains_key(word), "{word} is the note's own");
        }
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
