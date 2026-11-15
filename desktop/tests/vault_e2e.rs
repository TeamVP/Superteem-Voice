//! The vault from end to end, on a copy of Voice 0.8.36's folders: the first run bringing notes, books and
//! meetings in; reading them back; the backup copy; bringing things in from another computer; and taking on
//! another computer's vault. Windows only, where the vault is sealed.
//!
//! It runs in its own process (an integration test), so pointing the library at a sandbox can't touch the
//! person's own vault.

#![cfg(windows)]

use serde_json::json;
use std::path::{Path, PathBuf};
use superteem_voice_lib::{books, library, notes, sealed, vault};

fn write(path: &Path, value: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn wav(path: &Path, seconds: f64, freq: f64) -> Vec<u8> {
    let rate = 16_000u32;
    let pcm: Vec<i16> = (0..(seconds * rate as f64) as usize).map(|i| (8000.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin()) as i16).collect();
    let bytes = stv_core::export::wav_bytes(&pcm, rate);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

/// Voice 0.8.36's folders as they were: notes with everything beside them, a book, and a meeting.
struct Sandbox {
    data: PathBuf,
    docs: PathBuf,
    track: Vec<u8>,
    session: String,
}

fn sandbox(root: &Path, now: u64) -> Sandbox {
    let _ = std::fs::remove_dir_all(root);
    let data = root.join("local").join("Voice-OS");
    let docs = root.join("home").join("Documents").join("Voice-OS");
    let notes = data.join("notes");
    let books = data.join("books");
    write(&data.join("preferences.json"), json!({ "onboarded": true, "terms_accepted": now - 86400 }));
    write(&notes.join("n1700000001-1.json"), json!({ "id": "n1700000001-1", "title": "Offsite plan", "body": "## Venue\n\nBook the **hall** for Friday.", "created": now - 7200, "updated": now - 3600, "format": "md", "commentary": "Agreed at standup", "pronounce": { "Phocion": "Foe-shee-on" } }));
    write(&notes.join("n1700000001-1.versions.json"), json!({ "last": 2, "versions": [{ "n": 1, "at": now - 7200, "title": "Offsite plan", "body": "Book the hall", "by": "you" }, { "n": 2, "at": now - 3600, "title": "Offsite plan", "body": "## Venue\n\nBook the **hall** for Friday.", "by": "ai", "label": "Make it a list" }] }));
    write(&notes.join("n1700000001-1.chat.json"), json!({ "messages": [{ "role": "user", "text": "When is it?" }] }));
    write(&notes.join("n1700000001-1.bookmarks.json"), json!([{ "id": "m1", "at": 9, "quote": "Book the", "label": "Venue", "color": "c03", "created": now - 3000 }]));
    write(&notes.join("n1700000001-1.translations").join("fr.json"), json!({ "language": "fr", "updated": now - 3000 }));
    std::fs::create_dir_all(notes.join("n1700000001-1.files")).unwrap();
    std::fs::write(notes.join("n1700000001-1.files").join("ab12cd34-map.png"), [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]).unwrap();
    write(&notes.join("n1700000002-1.json"), json!({ "id": "n1700000002-1", "title": "Groceries", "body": "milk, eggs, bread", "created": now - 5000, "updated": now - 5000 }));
    write(&notes.join("n1700000003-1.json"), json!({ "id": "n1700000003-1", "body": "Chapter one: the beginning of the story.", "created": now - 4000, "updated": now - 4000 }));
    wav(&notes.join("audio").join(format!("n1700000002-1_{}.wav", now - 100)), 1.0, 330.0);
    write(&books.join("b1700000010-1.json"), json!({ "id": "b1700000010-1", "title": "My story", "created": now - 4000, "updated": now - 3900, "notes": ["n1700000003-1"], "editable": true }));
    write(&books.join("b1700000010-1.listened.json"), json!({ "note": "n1700000003-1", "at": 5, "quote": "Chapter one", "created": now - 3800 }));
    write(&books.join("b1700000010-1.marks-shown.json"), json!({ "hidden_colours": ["c05"], "hidden_authors": [] }));
    let session = format!("session-{}", now - 10_000);
    let dir = docs.join(&session);
    let track = wav(&dir.join("input1.wav"), 20.0, 440.0);
    wav(&dir.join("input2.wav"), 20.0, 220.0);
    write(&dir.join("meta.json"), json!({ "title": "Weekly sync", "created": now - 10_000, "names": { "input1": "Bruce", "input2": "Sam" } }));
    write(&dir.join("transcript.json"), json!([
        { "id": 0, "speaker": "input1", "stream": "input1", "start_s": 1.0, "end_s": 4.0, "text": "Let's start with the launch.", "confidence": 0.9 },
        { "id": 1, "speaker": "input2", "stream": "input2", "start_s": 5.0, "end_s": 9.0, "text": "The launch is on Friday.", "confidence": 0.9 },
    ]));
    std::fs::write(dir.join("transcript.txt"), "Bruce: Let's start with the launch.\n").unwrap();
    Sandbox { data, docs, track, session }
}

fn has_text(file: &Path, text: &str) -> bool {
    std::fs::read(file).unwrap().windows(text.len()).any(|w| w == text.as_bytes())
}

/// A vault of its own in `dir`, as another computer's, with one note, put in `backups` as the daily copy
/// is (with its key file beside it). Gives its words and ID.
fn other_vault(dir: &Path, backups: &Path, title: &str, at: u64) -> (String, String) {
    let _ = std::fs::remove_dir_all(dir);
    let opened = vault::Vault::open(dir).ok().expect("another vault opens");
    let note = notes::Note { id: format!("n{at}-9"), title: title.into(), body: format!("{title}, written there."), created: at, updated: at, ..Default::default() };
    notes::put(opened.conn(), &note).unwrap();
    let words = opened.recovery_words().unwrap().join(" ");
    drop(opened);
    let id = vault::id_of_words(&words).unwrap().replace('-', "");
    std::fs::create_dir_all(backups).unwrap();
    std::fs::copy(dir.join("vault.db"), backups.join(format!("vault-{id}-{at}.db"))).unwrap();
    std::fs::copy(dir.join("keys.json"), backups.join(format!("keys-{id}.json"))).unwrap();
    (words, id)
}

#[test]
fn notes_books_and_meetings_come_into_the_vault_and_every_way_back_in_works() {
    let now = library::now();
    let root = std::env::temp_dir().join(format!("stv-e2e-{}", std::process::id()));
    let sb = sandbox(&root.join("a"), now);
    library::init(sb.docs.clone(), sb.data.clone());

    // --- The first run: everything comes in, and a copy goes to the backup folder at once.
    let began = std::time::Instant::now();
    let status = vault::status();
    eprintln!("first open, bringing notes and books in: {} ms", began.elapsed().as_millis());
    assert_eq!((status.state, status.sealed), ("open", true));
    let moved = status.moved.clone().expect("what was moved in is recorded");
    assert_eq!((moved.notes, moved.books, moved.failed.len()), (3, 1, 0));
    assert_eq!((status.notes, status.books, status.own_notes), (3, 1, 3));
    assert!(!status.id.is_empty());
    let id = status.id.replace('-', "");
    let backups = sb.docs.join("Vault backups");
    assert!(std::fs::read_dir(&backups).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with(&format!("vault-{id}-"))), "a copy is in the backup folder from the first minute");
    assert!(backups.join(format!("keys-{id}.json")).is_file());
    assert!(status.backup_latest > 0);

    // The old files are gone, and nothing readable is left in the vault's file.
    let left: Vec<String> = std::fs::read_dir(sb.data.join("notes")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") || n.ends_with(".translations")).collect();
    assert!(left.is_empty(), "plain note files left: {left:?}");
    assert!(!sb.data.join("books").join("b1700000010-1.json").exists());
    let db = sb.data.join("vault").join("vault.db");
    for text in ["Offsite", "Groceries", "My story", "Venue", "Foe-shee-on"] {
        assert!(!has_text(&db, text), "{text} is readable in the vault file");
    }

    // --- Everything reads back as it was.
    let listed = notes::list("");
    assert_eq!(listed.len(), 3);
    let plan = notes::load("n1700000001-1").unwrap();
    assert_eq!((plan.title.as_str(), plan.commentary.as_str(), plan.pronounce.get("Phocion").map(String::as_str)), ("Offsite plan", "Agreed at standup", Some("Foe-shee-on")));
    assert_eq!(notes::bookmarks("n1700000001-1").unwrap()[0].label, "Venue");
    assert!(notes::versions("n1700000001-1").unwrap().iter().any(|v| v.by == "ai"));
    assert_eq!(notes::list("hall").len(), 1, "search reads the text");
    let book = books::list("");
    assert_eq!(book.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["My story"]);
    assert_eq!(books::listen_place("b1700000010-1").unwrap().note, "n1700000003-1");
    assert_eq!(books::bookmark_view("b1700000010-1").hidden_colours, ["c05"]);
    assert_eq!(books::chapters("b1700000010-1").unwrap().len(), 1);

    // A saved note, and a deleted one taking its bookmarks with it.
    let new = notes::save(None, "Written in the vault".into(), "Sealed from the start".into(), false, None).unwrap();
    assert_eq!(notes::load(&new).unwrap().body, "Sealed from the start");
    notes::save_bookmarks(&new, vec![notes::Bookmark { id: "x".into(), quote: "Sealed".into(), ..Default::default() }]).unwrap();
    notes::delete(&new).unwrap();
    assert!(notes::load(&new).is_err() && notes::bookmarks(&new).unwrap().is_empty());

    // The example book goes straight into the vault, its pictures sealed; its chapters can't be changed.
    books::install_example(None);
    let example = books::load("plutarch-lives").expect("the example book is in the vault");
    let first = example.notes.first().unwrap().clone();
    assert!(notes::load(&first).is_ok());
    assert!(notes::save(Some(first.clone()), "x".into(), "y".into(), false, None).is_err(), "a book kept for reading stays as it is");
    let cover = books::file_path("plutarch-lives", &example.cover).unwrap();
    assert!(sealed::is_sealed(&cover) && !sealed::read(&cover).unwrap().is_empty());
    assert_eq!(vault::status().own_notes, 3, "the example book's chapters aren't counted as the person's notes");

    // --- Meetings, the note's picture and the dictation audio are sealed in place.
    let began = std::time::Instant::now();
    sealed::seal_existing();
    eprintln!("sealing what was recorded before: {} ms", began.elapsed().as_millis());
    let session = sb.docs.join(&sb.session);
    for file in ["input1.wav", "input2.wav", "meta.json", "transcript.json"] {
        assert!(sealed::is_sealed(&session.join(file)), "{file} is sealed");
    }
    assert!(!session.join("transcript.txt").exists(), "the readable copy is removed");
    assert!(sealed::is_sealed(&sb.data.join("notes").join("n1700000001-1.files").join("ab12cd34-map.png")));
    let meetings = library::list("", &[]);
    assert_eq!(meetings.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["Weekly sync"]);
    assert_eq!(library::load_segments(&session).len(), 2);
    assert_eq!(library::list("friday", &[]).len(), 1, "search reads sealed transcripts");
    // Playing a stretch of a sealed track gives the very bytes recorded.
    let mut reader = sealed::Reader::open(&session.join("input1.wav")).unwrap();
    assert_eq!(reader.len() as usize, sb.track.len());
    let (from, count) = (300_000usize, 200_000usize);
    assert_eq!(reader.read(from as u64, count).unwrap(), sb.track[from..from + count]);

    // --- The recovery words, the kit, and a backup folder of the person's choosing.
    let words = vault::words().unwrap().join(" ");
    assert_eq!(vault::id_of_words(&words).unwrap(), status.id);
    vault::check_words(&words).unwrap();
    let kit = vault::kit().unwrap();
    assert!(kit.contains(&status.id) && kit.contains("ON A NEW COMPUTER") && words.split(' ').all(|w| kit.contains(w)));
    let usb = root.join("usb drive");
    std::fs::create_dir_all(&usb).unwrap();
    vault::set_backup_dir(&usb.to_string_lossy()).unwrap();
    assert!(std::fs::read_dir(&usb).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with(&format!("vault-{id}-"))), "choosing a folder puts a copy there at once");
    assert!(vault::set_backup_dir(&root.join("missing").to_string_lossy()).is_err());
    vault::set_backup_dir("").unwrap();

    // --- Bringing in from another computer's folders, as they were before the vault.
    let other = sandbox(&root.join("b"), now);
    write(&other.data.join("notes").join("n1700000099-1.json"), json!({ "id": "n1700000099-1", "title": "From the laptop", "body": "Only on the laptop", "created": now, "updated": now }));
    let brought = vault::bring_in_folder(&other.data, "", "").unwrap();
    assert_eq!((brought.notes, brought.books), (1, 0), "only the note that wasn't here: {brought:?}");
    assert!(brought.had >= 4, "the notes and book already here are left alone: {brought:?}");
    assert!(other.data.join("notes").join("n1700000099-1.json").exists(), "the chosen folder isn't changed");
    // Its meetings: one here already, one new.
    let laptop_meeting = format!("session-{}", now - 500);
    std::fs::rename(other.docs.join(&other.session), other.docs.join(&laptop_meeting)).unwrap();
    let brought = vault::bring_in_folder(&other.docs, "", "").unwrap();
    assert_eq!(brought.meetings, 1);
    sealed::seal_existing();
    assert!(sealed::is_sealed(&sb.docs.join(&laptop_meeting).join("input1.wav")));
    assert_eq!(library::list("", &[]).len(), 2);

    // --- Another vault's backup copy, with its words: its notes come in, and its key is kept.
    let elsewhere = root.join("elsewhere");
    let (b_words, b_id) = other_vault(&root.join("vault-b"), &elsewhere, "From the old PC", now - 50);
    let brought = vault::bring_in_folder(&elsewhere, &b_words, "").unwrap();
    assert_eq!(brought.notes, 1);
    assert_eq!(brought.other_vault.replace('-', ""), b_id);
    let status = vault::status();
    assert_eq!(status.keyring, 1);
    assert!(notes::list("").iter().any(|n| n.title == "From the old PC"));
    // Words of a vault whose copy is lost still open what it sealed.
    assert!(vault::add_words(&words).unwrap_err().contains("this vault's own"));

    // --- A new computer: another vault taken on as this one's own.
    let old_pc = root.join("old pc backups");
    let (c_words, c_id) = other_vault(&root.join("vault-c"), &old_pc, "Written on the old computer", now - 20);
    let before = notes::list("").len();
    let result = vault::adopt(&old_pc, &c_words, "").unwrap();
    assert!(result.adopted);
    let status = vault::status();
    assert_eq!(status.id.replace('-', ""), c_id, "this computer's vault is the one taken on");
    assert_eq!(vault::words().unwrap().join(" "), c_words, "its recovery words keep working");
    assert!(status.words_saved > 0, "words that just opened it count as saved");
    assert_eq!(notes::list("").len(), before + 1, "everything that was here came with it");
    assert!(status.keyring >= 2, "the keys of the vault it replaced, and of the one brought in, are kept");
    // What the replaced vault sealed still opens.
    assert_eq!(library::list("", &[]).iter().filter(|m| m.title == "Weekly sync").count(), 2);
    assert_eq!(sealed::Reader::open(&session.join("input1.wav")).unwrap().read(0, 44).unwrap(), sb.track[..44]);
    let aside: Vec<String> = std::fs::read_dir(sb.data.join("vault").join("set-aside")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(aside.iter().any(|n| n.ends_with("-before-taking-on.db")), "the replaced vault is set aside, not deleted: {aside:?}");
    // A copy of the vault taken on is refused as a copy of this one.
    assert!(vault::adopt(&old_pc, &c_words, "").unwrap_err().contains("copy of this vault"));

    let _ = std::fs::remove_dir_all(&root);
}
