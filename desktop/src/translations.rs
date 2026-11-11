//! Translations of meetings and notes, kept beside them, one file per language.
//!
//! A translation is made line by line and kept by a fingerprint of each original line, so editing one
//! line, or transcribing the meeting again, only translates the lines that changed. Corrections the person
//! makes are kept by line, with the fingerprint of the original they corrected: when that original
//! changes, the correction stays and is marked so they can check it.

use crate::analysis::Item;
use crate::{library, notes, prefs, process, vocabulary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use stv_core::analysis::{self as engine, Model};
use stv_core::lang;
use stv_core::translate::{self, Done, Line};

/// A translated line, by the fingerprint of the original.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cached {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// A correction the person made to one line.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edited {
    pub text: String,
    /// The fingerprint of the original line when it was corrected.
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub language: String,
    pub lines: BTreeMap<String, Cached>,
    /// By line id: a meeting line's id, or a note sentence's fingerprint.
    pub edits: BTreeMap<String, Edited>,
    pub updated: u64,
    /// The model it was made with.
    pub engine: String,
}

fn dir(item: &Item) -> Result<PathBuf, String> {
    Ok(match item {
        Item::Meeting(id) => library::session_dir(id)?.join("translations"),
        Item::Note(id) => {
            notes::load(id)?;
            library::data_dir().join("notes").join(format!("{id}.translations"))
        }
    })
}

fn path(item: &Item, language: &str) -> Result<PathBuf, String> {
    if !lang::is_known(language) {
        return Err("That language isn't one Voice offers yet.".into());
    }
    Ok(dir(item)?.join(format!("{language}.json")))
}

pub fn load(item: &Item, language: &str) -> Store {
    path(item, language).ok().and_then(|p| library::read_json(&p).ok()).unwrap_or_default()
}

fn save(item: &Item, store: &Store) -> Result<(), String> {
    let p = path(item, &store.language)?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    library::write_json(&p, store)
}

/// The languages an item has translations in.
pub fn languages(item: &Item) -> Vec<String> {
    let Ok(d) = dir(item) else { return vec![] };
    let mut out: Vec<String> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|l| lang::is_known(l))
        .collect();
    out.sort();
    out
}

pub fn delete(item: &Item, language: &str) -> Result<(), String> {
    match std::fs::remove_file(path(item, language)?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Removes a note's translations with it.
pub fn forget_note(id: &str) {
    let _ = std::fs::remove_dir_all(library::data_dir().join("notes").join(format!("{id}.translations")));
}

/// One line of an item to translate.
#[derive(Debug, Clone)]
pub struct SourceLine {
    /// A meeting line's id, or a note sentence's fingerprint.
    pub id: String,
    pub text: String,
    /// The language it was heard in, when known.
    pub language: String,
    pub speaker: String,
    pub at_s: Option<f64>,
}

/// The lines of a meeting or note, in order, with the languages it's in (main one first).
pub fn source(item: &Item) -> Result<(Vec<SourceLine>, Vec<String>, String), String> {
    let p = prefs::load();
    match item {
        Item::Meeting(id) => {
            let d = library::session_dir(id)?;
            let meta = library::load_meta(&d);
            let languages = p.effective(&meta.langs());
            let lines = library::load_segments(&d)
                .into_iter()
                .filter(|s| s.in_script() && !process::is_silence(&s.text))
                .map(|s| SourceLine {
                    id: s.id.to_string(),
                    language: if s.language.is_empty() { languages[0].clone() } else { s.language.clone() },
                    speaker: meta.name(&s.speaker),
                    at_s: Some(s.start_s),
                    text: s.text,
                })
                .collect();
            Ok((lines, languages, meta.title))
        }
        Item::Note(id) => {
            let note = notes::load(id)?;
            let languages = p.effective(&note.langs());
            let lines = engine::note_turns(&note.body)
                .into_iter()
                .map(|t| SourceLine { id: translate::fingerprint(&t.text), language: languages[0].clone(), speaker: String::new(), at_s: None, text: t.text })
                .collect();
            Ok((lines, languages, notes::display_title(&note)))
        }
    }
}

/// A line as the page shows it.
#[derive(Debug, Clone, Serialize)]
pub struct LineView {
    pub id: String,
    pub speaker: String,
    pub at_s: Option<f64>,
    pub original: String,
    /// Empty when it isn't translated yet.
    pub text: String,
    pub edited: bool,
    /// Corrected by the person, and the original has changed since.
    pub original_changed: bool,
    pub warning: Option<String>,
    /// Already in the language: shown as it is.
    pub same: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub language: String,
    pub lines: Vec<LineView>,
    /// Lines with no translation yet.
    pub missing: usize,
    pub updated: u64,
    pub engine: String,
    /// A better model to use, when the one chosen writes these languages poorly.
    pub advice: Option<String>,
}

/// Advice when the language model chosen is one of the small ones, which translate only the languages
/// it writes well (those analyses run in directly); others come back garbled or untranslated.
pub fn advice(languages: &[String]) -> Option<String> {
    let p = prefs::load();
    if !p.llm_server.trim().is_empty() {
        return None;
    }
    let file = crate::llm::chosen(&p.llm_model)?.file_name()?.to_string_lossy().to_lowercase();
    let small = ["1.7b", "1.5b", "0.5b"].iter().any(|s| file.contains(s));
    let weak: Vec<String> = languages
        .iter()
        .filter(|l| lang::get(l).is_some_and(|x| x.analysis == lang::Analysis::ViaEnglish))
        .map(|l| lang::name(l))
        .collect();
    (small && !weak.is_empty()).then(|| {
        format!("The small language model translates {} poorly. Qwen3.5 4B does much better: choose it in Settings › Analysis.", weak.join(" and "))
    })
}

/// Whether a line needs no translating into `to`: heard in that language, or the item is only in it.
fn already_in(line: &SourceLine, to: &str) -> bool {
    line.language == to
}

pub fn view(item: &Item, language: &str) -> Result<View, String> {
    let (lines, languages, _) = source(item)?;
    let store = load(item, language);
    let mut missing = 0;
    let lines = lines
        .into_iter()
        .map(|l| {
            let fp = translate::fingerprint(&l.text);
            let same = already_in(&l, language);
            let cached = store.lines.get(&fp);
            let edit = store.edits.get(&l.id);
            let text = match (same, edit, cached) {
                (_, Some(e), _) => e.text.clone(),
                (true, None, _) => l.text.clone(),
                (false, None, Some(c)) => c.text.clone(),
                (false, None, None) => {
                    missing += 1;
                    String::new()
                }
            };
            LineView {
                original_changed: edit.is_some_and(|e| e.source != fp),
                edited: edit.is_some(),
                warning: if edit.is_some() || same { None } else { cached.and_then(|c| c.warning.clone()) },
                id: l.id,
                speaker: l.speaker,
                at_s: l.at_s,
                original: l.text,
                text,
                same,
            }
        })
        .collect();
    let mut involved = languages;
    involved.push(language.to_string());
    Ok(View { language: language.into(), lines, missing, updated: store.updated, engine: store.engine, advice: advice(&involved) })
}

/// The text of each line in `language`, by line id, falling back to the original where there's no
/// translation; and whether every line was translated.
pub fn texts(item: &Item, language: &str) -> Result<(BTreeMap<String, String>, bool), String> {
    let v = view(item, language)?;
    let complete = v.missing == 0;
    Ok((v.lines.into_iter().map(|l| (l.id, if l.text.is_empty() { l.original } else { l.text })).collect(), complete))
}

/// Names and terms that come through translation unchanged: the person's vocabulary and the names of the
/// people in a meeting.
fn kept_terms(item: &Item) -> Vec<String> {
    let mut keep: Vec<String> = vocabulary::load().into_iter().map(|t| t.spelling).filter(|s| !s.trim().is_empty()).collect();
    if let Item::Meeting(id) = item {
        if let Ok(d) = library::session_dir(id) {
            keep.extend(library::load_meta(&d).names.into_values().filter(|n| !n.trim().is_empty()));
        }
    }
    keep.sort();
    keep.dedup();
    keep.truncate(60);
    keep
}

/// Translates the lines of an item not yet translated into `language`, and keeps them. Returns how many
/// lines were translated. `progress` is told the batch and how many there are.
pub fn update(item: &Item, language: &str, model: &mut dyn Model, engine_name: &str, progress: impl FnMut(usize, usize)) -> Result<usize, String> {
    let (lines, languages, _) = source(item)?;
    let mut store = load(item, language);
    store.language = language.to_string();
    let mut todo: Vec<Line> = Vec::new();
    for l in lines.iter().filter(|l| !already_in(l, language)) {
        let fp = translate::fingerprint(&l.text);
        if !store.lines.contains_key(&fp) && !todo.iter().any(|t| t.id == fp) {
            todo.push(Line { id: fp, text: l.text.clone() });
        }
    }
    // Lines that no longer exist aren't kept, except the corrections the person made.
    let wanted: std::collections::BTreeSet<String> = lines.iter().map(|l| translate::fingerprint(&l.text)).collect();
    store.lines.retain(|fp, _| wanted.contains(fp));
    let from: Vec<String> = languages.into_iter().filter(|l| l != language).collect();
    let done: Vec<Done> = if todo.is_empty() { vec![] } else { translate::translate(model, &todo, &from, language, &kept_terms(item), progress)? };
    let n = done.len();
    for d in done {
        store.lines.insert(d.id, Cached { text: d.text, warning: d.warning });
    }
    store.updated = library::now();
    store.engine = engine_name.to_string();
    save(item, &store)?;
    Ok(n)
}

/// Keeps the person's correction to one line, or with an empty text, drops it.
pub fn edit(item: &Item, language: &str, line: &str, text: &str) -> Result<(), String> {
    let (lines, _, _) = source(item)?;
    let l = lines.iter().find(|l| l.id == line).ok_or("That line no longer exists.")?;
    let mut store = load(item, language);
    store.language = language.to_string();
    if text.trim().is_empty() {
        store.edits.remove(line);
    } else {
        store.edits.insert(line.to_string(), Edited { text: text.trim().to_string(), source: translate::fingerprint(&l.text) });
    }
    save(item, &store)
}

/// The translation as a document: a meeting's lines with times and speakers, or a note's sentences.
pub fn render(item: &Item, language: &str, md: bool) -> Result<(String, String), String> {
    let (_, _, title) = source(item)?;
    let v = view(item, language)?;
    let heading = format!("{title} ({})", lang::name(language));
    let mut out = if md { format!("# {}\n\n", engine::escape_md(&heading)) } else { format!("{heading}\n\n") };
    let note = format!("Machine translation into {}. Check it against the original.", lang::name(language));
    out += &if md { format!("_{note}_\n\n") } else { format!("{note}\n\n") };
    let is_note = matches!(item, Item::Note(_));
    for l in &v.lines {
        let text = if l.text.is_empty() { format!("[not translated] {}", l.original) } else { l.text.clone() };
        let text = if md { engine::escape_md(&text) } else { text };
        if is_note {
            out += &format!("{text}\n");
        } else {
            let at = l.at_s.map(engine::clock).unwrap_or_default();
            out += &if md { format!("**{} {}:** {text}\n\n", at, engine::escape_md(&l.speaker)) } else { format!("[{at}] {}: {text}\n", l.speaker) };
        }
    }
    Ok((heading, out))
}
