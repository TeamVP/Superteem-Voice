//! Analyses of meetings and notes: which templates there are, the stack of results kept beside each item,
//! running a template, and the person's edits.
//!
//! Results live in a sidecar, never in `transcript.json`: `analysis.json` in a meeting's folder, and
//! `<id>.analysis.json` beside a note. A meeting or note has at most one analysis per template, kept in
//! the order they were added, which is the order on the page and in the exports.
//!
//! Citations are stored as anchors that survive reprocessing and editing: `s:<stream>:<start_s>` for a
//! meeting line (matched to the nearest line on that stream), and `n:<offset>:<hash>` for a note's
//! sentence (matched by its fingerprint when it has moved).

use crate::library::{self, Meta, Segment};
use crate::{notes, process, vocabulary};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use stv_core::analysis::{self as engine, Bookmark, Context, LineIn, Model, Template, Turn};

/// A line this close in time to where it was is the same line after reprocessing.
const SAME_LINE_S: f64 = 1.0;

// ---------------------------------------------------------------------------------------------------------
// Templates

/// Where people's own templates live: beside the meetings on Windows, the Mac and iPhone (where Files shows
/// them), so any text editor can open them; in the app's own folder on Android.
pub fn templates_dir() -> PathBuf {
    if cfg!(target_os = "android") { library::data_dir().join("templates") } else { library::root().join("Templates") }
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// "built-in", "custom", or "customised" (a custom file replacing a built-in).
    pub source: String,
    pub file: Option<String>,
    pub applies_to: Vec<String>,
    pub manual_only: bool,
    /// Problems that stop a custom template from running.
    pub errors: Vec<String>,
    pub template: Option<Template>,
}

/// Built-ins, then the person's own, which replace a built-in with the same id. Read fresh each time, so
/// edits in a text editor show up without restarting.
pub fn templates() -> Vec<TemplateInfo> {
    let mut out: Vec<TemplateInfo> = engine::built_in()
        .into_iter()
        .map(|t| TemplateInfo {
            id: t.id.clone(),
            name: t.name.clone(),
            description: t.description.clone(),
            source: "built-in".into(),
            file: None,
            applies_to: t.applies_to.clone(),
            manual_only: t.manual_only,
            errors: vec![],
            template: Some(t),
        })
        .collect();
    let mut files: Vec<PathBuf> = std::fs::read_dir(templates_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .take(100)
        .collect();
    files.sort();
    for path in files {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let source = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= 64 * 1024)
            .and_then(|_| std::fs::read_to_string(&path).ok())
            .unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match engine::parse(&source) {
            Ok(t) => {
                let info = TemplateInfo {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    source: "custom".into(),
                    file,
                    applies_to: t.applies_to.clone(),
                    manual_only: t.manual_only,
                    errors: vec![],
                    template: Some(t),
                };
                match out.iter_mut().find(|o| o.id == info.id) {
                    Some(existing) if existing.source == "built-in" => *existing = TemplateInfo { source: "customised".into(), ..info },
                    Some(_) => out.push(TemplateInfo { errors: vec![format!("Another file already uses the id \"{}\".", info.id)], template: None, ..info }),
                    None => out.push(info),
                }
            }
            Err(errors) => out.push(TemplateInfo {
                id: stem.clone(),
                name: stem,
                description: String::new(),
                source: "custom".into(),
                file,
                applies_to: vec![],
                manual_only: true,
                errors,
                template: None,
            }),
        }
    }
    out
}

pub fn template(id: &str) -> Result<Template, String> {
    templates()
        .into_iter()
        .find(|t| t.id == id)
        .and_then(|t| t.template)
        .ok_or_else(|| format!("The template \"{id}\" isn't available or has problems; see Settings › Templates."))
}

fn template_path(file: &str) -> Result<PathBuf, String> {
    let plain = file.ends_with(".toml") && !file.contains(['/', '\\']) && !file.starts_with('.');
    if plain { Ok(templates_dir().join(file)) } else { Err("That isn't a template file.".into()) }
}

/// The text of a template, for editing: a custom file's own text, or a built-in's as a starting point.
pub fn template_source(id: &str) -> Result<(String, Option<String>), String> {
    let all = templates();
    let info = all.iter().find(|t| t.id == id || t.file.as_deref() == Some(id)).ok_or("That template no longer exists.")?;
    match &info.file {
        Some(file) => Ok((std::fs::read_to_string(template_path(file)?).map_err(|e| e.to_string())?, Some(file.clone()))),
        None => {
            let source = engine::BUILT_IN.iter().find(|s| engine::parse(s).is_ok_and(|t| t.id == info.id)).ok_or("Built-in not found.")?;
            Ok((source.to_string(), None))
        }
    }
}

/// Saves a template's text after checking it, as `file` or as a new file named after its id. Returns the
/// file name.
pub fn save_template(source: &str, file: Option<&str>) -> Result<String, String> {
    let t = engine::parse(source).map_err(|e| e.join("\n"))?;
    std::fs::create_dir_all(templates_dir()).map_err(|e| e.to_string())?;
    let file = match file {
        Some(f) => f.to_string(),
        None => format!("{}.toml", t.id),
    };
    library::write_text(&template_path(&file)?, source)?;
    Ok(file)
}

pub fn delete_template(file: &str) -> Result<(), String> {
    std::fs::remove_file(template_path(file)?).map_err(|e| e.to_string())
}

pub fn new_template(name: &str, headings: &[(String, engine::HeadingKind)]) -> Result<String, String> {
    let source = engine::from_headings(name, headings)?;
    let t = engine::parse(&source).map_err(|e| e.join(" "))?;
    if templates().iter().any(|x| x.id == t.id) {
        return Err(format!("There's already a template called \"{name}\"."));
    }
    save_template(&source, None)?;
    Ok(t.id)
}

// ---------------------------------------------------------------------------------------------------------
// Items

/// A meeting or a note, which can each carry analyses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum Item {
    Meeting(String),
    Note(String),
}

impl Item {
    pub fn new(kind: &str, id: &str) -> Result<Self, String> {
        match kind {
            "meeting" => library::session_dir(id).map(|_| Item::Meeting(id.into())),
            "note" => notes::load(id).map(|_| Item::Note(id.into())),
            _ => Err(format!("Unknown kind: {kind}")),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Item::Meeting(_) => "meeting",
            Item::Note(_) => "note",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Item::Meeting(id) | Item::Note(id) => id,
        }
    }

    fn path(&self) -> Result<PathBuf, String> {
        Ok(match self {
            Item::Meeting(id) => library::session_dir(id)?.join("analysis.json"),
            Item::Note(id) => {
                notes::load(id)?;
                library::data_dir().join("notes").join(format!("{id}.analysis.json"))
            }
        })
    }
}

/// Removes a note's analyses with it.
pub fn forget_note(id: &str) {
    let _ = std::fs::remove_file(library::data_dir().join("notes").join(format!("{id}.analysis.json")));
    crate::translations::forget_note(id);
}

/// What a template reads, and how to find its citations again.
pub struct Input {
    pub turns: Vec<Turn>,
    pub title: String,
    pub created: u64,
    pub duration_s: f64,
    pub speakers: Vec<String>,
    pub bookmarks: Vec<Bookmark>,
    pub is_note: bool,
    /// For a meeting: its lines, to turn citations into anchors and back.
    segments: Vec<Segment>,
    meta: Meta,
    body: String,
}

fn is_action(m: &library::Marker) -> bool {
    m.kind == "action" || (m.kind.is_empty() && m.text == "Action item")
}

pub fn meeting_input(meta: &Meta, segments: &[Segment]) -> Input {
    let lines: Vec<(usize, String, f64, String)> = segments
        .iter()
        .filter(|s| s.in_script() && !process::is_silence(&s.text))
        .map(|s| (s.id, meta.name(&s.speaker), s.start_s, s.text.clone()))
        .collect();
    let lines_in: Vec<LineIn> = lines.iter().map(|(id, speaker, start_s, text)| LineIn { id: *id, speaker, start_s: *start_s, text }).collect();
    let mut turns = engine::meeting_turns(&lines_in);
    for m in meta.markers.iter().filter(|m| !m.removed && is_action(m)) {
        if let Some(i) = turns.iter().rposition(|t| t.at_s.is_some_and(|at| at <= m.at_s + 1.0)) {
            turns[i].text += " [action item]";
        }
    }
    let bookmarks = meta
        .markers
        .iter()
        .filter(|m| !m.removed && !is_action(m))
        .enumerate()
        .map(|(i, m)| Bookmark { n: i + 1, at_s: m.at_s, note: m.text.clone() })
        .collect();
    let mut speakers: Vec<String> = Vec::new();
    for t in &turns {
        if !speakers.contains(&t.speaker) {
            speakers.push(t.speaker.clone());
        }
    }
    Input {
        turns,
        title: meta.title.clone(),
        created: meta.created,
        duration_s: meta.duration_s,
        speakers,
        bookmarks,
        is_note: false,
        segments: segments.to_vec(),
        meta: meta.clone(),
        body: String::new(),
    }
}

pub fn note_input(note: &notes::Note) -> Input {
    Input {
        turns: engine::note_turns(&note.body),
        title: notes::display_title(note),
        created: note.created,
        duration_s: 0.0,
        speakers: vec![engine::ME.into()],
        bookmarks: vec![],
        is_note: true,
        segments: vec![],
        meta: Meta::default(),
        body: note.body.clone(),
    }
}

pub fn input(item: &Item) -> Result<Input, String> {
    match item {
        Item::Meeting(id) => {
            let dir = library::session_dir(id)?;
            Ok(meeting_input(&library::load_meta(&dir), &library::load_segments(&dir)))
        }
        Item::Note(id) => Ok(note_input(&notes::load(id)?)),
    }
}

impl Input {
    /// A turn number as a stored anchor.
    fn anchor(&self, n: u64) -> Option<Value> {
        let t = self.turns.iter().find(|t| t.n == n as usize)?;
        match &t.source {
            engine::Source::Segments(ids) => {
                let first = *ids.first()?;
                let s = self.segments.iter().find(|s| s.id == first)?;
                Some(json!(format!("s:{}:{:.2}", s.stream, s.start_s)))
            }
            engine::Source::Range(a, b) => Some(json!(format!("n:{a}:{}", engine::short_hash(self.body.get(*a..*b)?.trim())))),
        }
    }

    /// Where an anchor points now, or None when that line or sentence is gone.
    pub fn resolve(&self, anchor: &str) -> Option<SourceView> {
        let mut parts = anchor.splitn(3, ':');
        match parts.next()? {
            "s" => {
                let rest = &anchor[2..];
                let (stream, at) = rest.rsplit_once(':')?;
                let at: f64 = at.parse().ok()?;
                let script: Vec<&Segment> = self.segments.iter().filter(|s| s.in_script() && !s.text.trim().is_empty()).collect();
                let i = script
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.stream == stream && (s.start_s - at).abs() <= SAME_LINE_S)
                    .min_by(|a, b| (a.1.start_s - at).abs().total_cmp(&(b.1.start_s - at).abs()))?
                    .0;
                let s = script[i];
                let around = |j: Option<usize>| j.and_then(|j| script.get(j)).map(|s| format!("{}: {}", self.meta.name(&s.speaker), s.text));
                Some(SourceView {
                    label: engine::clock(s.start_s),
                    at_s: Some(s.start_s),
                    segment: Some(s.id),
                    range: None,
                    speaker: self.meta.name(&s.speaker),
                    text: s.text.clone(),
                    before: around(i.checked_sub(1)),
                    after: around(Some(i + 1)),
                })
            }
            "n" => {
                let offset: usize = parts.next()?.parse().ok()?;
                let hash = parts.next()?;
                let turns = engine::note_turns(&self.body);
                let range_of = |t: &Turn| match t.source {
                    engine::Source::Range(a, b) => Some((a, b)),
                    _ => None,
                };
                let matches = |t: &Turn| range_of(t).and_then(|(a, b)| self.body.get(a..b)).is_some_and(|s| engine::short_hash(s.trim()) == hash);
                let i = turns
                    .iter()
                    .position(|t| range_of(t).is_some_and(|(a, _)| a == offset) && matches(t))
                    .or_else(|| turns.iter().position(matches))?;
                let t = &turns[i];
                Some(SourceView {
                    label: format!("¶{}", t.n),
                    at_s: None,
                    segment: None,
                    range: range_of(t),
                    speaker: engine::ME.into(),
                    text: t.text.clone(),
                    before: i.checked_sub(1).and_then(|j| turns.get(j)).map(|t| t.text.clone()),
                    after: turns.get(i + 1).map(|t| t.text.clone()),
                })
            }
            _ => None,
        }
    }

    pub fn hash(&self) -> String {
        engine::input_hash(&self.turns)
    }

    fn date(&self) -> String {
        if self.created == 0 {
            return String::new();
        }
        let days = (self.created / 86_400) as i64;
        // Civil date from days since 1970 (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    }
}

/// A citation as the page shows it: hovering shows the line and its neighbours, clicking goes there.
#[derive(Debug, Clone, Serialize)]
pub struct SourceView {
    pub label: String,
    pub at_s: Option<f64>,
    pub segment: Option<usize>,
    /// Byte range in a note's body.
    pub range: Option<(usize, usize)>,
    pub speaker: String,
    pub text: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

// ---------------------------------------------------------------------------------------------------------
// The stack

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRef {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engine {
    /// "local" or "server".
    pub kind: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub template: TemplateRef,
    pub engine: Engine,
    pub created: u64,
    pub input_hash: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub seconds: f64,
    pub result: Value,
    /// The language it's written in; empty for English, as before languages.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// What it was made from: "original", "translation" (the item's English translation) or
    /// "translated" (another entry, translated into this language). Empty is "original".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub via: String,
    /// For a translated entry: when the entry it was translated from was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_created: Option<u64>,
}

impl Entry {
    /// How the stack knows it: the template's id, with the language after an @ for a translation of
    /// another entry ("meeting-summary@cy").
    pub fn key(&self) -> String {
        if self.via == "translated" { format!("{}@{}", self.template.id, self.language) } else { self.template.id.clone() }
    }
}

/// A key's template and, for a translated entry, its language.
pub fn split_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once('@') {
        Some((t, l)) => (t, Some(l)),
        None => (key, None),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stack {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub stack: Vec<Entry>,
}

fn one() -> u32 {
    1
}

pub fn load(item: &Item) -> Stack {
    item.path().ok().and_then(|p| library::read_json(&p).ok()).unwrap_or_default()
}

fn save(item: &Item, stack: &Stack) -> Result<(), String> {
    let path = item.path()?;
    if stack.stack.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        library::write_json(&path, stack)?;
    }
    // The documents people open outside the app carry the analyses, so they're written again.
    if let Item::Meeting(id) = item {
        let dir = library::session_dir(id)?;
        library::save(&dir, &library::load_meta(&dir), &library::load_segments(&dir))?;
    }
    Ok(())
}

/// The languages an item is in, main one first.
pub fn item_languages(item: &Item) -> Result<Vec<String>, String> {
    let p = crate::prefs::load();
    Ok(match item {
        Item::Meeting(id) => p.effective(&library::load_meta(&library::session_dir(id)?).langs()),
        Item::Note(id) => p.effective(&notes::load(id)?.langs()),
    })
}

/// The language an item's analyses are written in: its main language when the model writes it well,
/// else English, from the item's English translation.
pub fn analysis_language(item: &Item) -> Result<String, String> {
    let main = item_languages(item)?.remove(0);
    Ok(match stv_core::lang::get(&main).map(|l| l.analysis) {
        Some(stv_core::lang::Analysis::Direct) | None => main,
        Some(stv_core::lang::Analysis::ViaEnglish) => stv_core::lang::ENGLISH.into(),
    })
}

/// What a template reads in `language`: the item itself, or its translation with the same lines, so
/// citations still point at the original.
fn input_in(item: &Item, language: &str) -> Result<Input, String> {
    let (texts, _) = crate::translations::texts(item, language)?;
    match item {
        Item::Meeting(id) => {
            let dir = library::session_dir(id)?;
            let meta = library::load_meta(&dir);
            let mut segments = library::load_segments(&dir);
            for s in segments.iter_mut() {
                if let Some(t) = texts.get(&s.id.to_string()) {
                    s.text = t.clone();
                }
            }
            let mut translated = meeting_input(&meta, &segments);
            // Citations are found again in the original lines.
            translated.segments = library::load_segments(&dir);
            Ok(translated)
        }
        Item::Note(id) => {
            let note = notes::load(id)?;
            let mut i = note_input(&note);
            for t in i.turns.iter_mut() {
                if let Some(x) = texts.get(&stv_core::translate::fingerprint(&t.text)) {
                    t.text = x.clone();
                }
            }
            Ok(i)
        }
    }
}

/// Runs a template on an item, or translates one of its analyses. A key "template@cy" makes the
/// template's analysis first when it's missing or out of date, then translates it into Welsh. The
/// result goes into the stack with `put`, which is quick, so edits made meanwhile aren't held up for the
/// minute or so this takes. `progress` gets a step name, the part and how many parts.
pub fn make(item: &Item, key: &str, model: &mut dyn Model, engine: Engine, mut progress: impl FnMut(&str, usize, usize)) -> Result<Vec<Entry>, String> {
    let (template_id, into) = split_key(key);
    let t = template(template_id)?;
    if !t.applies_to(item.kind()) {
        return Err(format!("\"{}\" isn't for {}s.", t.name, item.kind()));
    }
    let language = analysis_language(item)?;
    let original_hash = input(item)?.hash();
    let mut made = Vec::new();
    let base = load(item).stack.into_iter().find(|e| e.key() == template_id && e.input_hash == original_hash);
    let base = match (base, into) {
        (Some(b), Some(_)) => b,
        _ => {
            // Lines in another language are read in English translation; those already translated are kept.
            let translate_first = language == stv_core::lang::ENGLISH && item_languages(item)?.iter().any(|l| l != stv_core::lang::ENGLISH);
            if translate_first {
                crate::translations::update(item, &language, model, &engine.model, |k, n| progress("Translating the transcript", k, n))?;
                // An analysis of lines left untranslated would be guesswork.
                let v = crate::translations::view(item, &language)?;
                let failed = v.lines.iter().filter(|l| !l.same && (l.warning.is_some() || l.text.is_empty())).count();
                let translated = v.lines.iter().filter(|l| !l.same).count();
                if translated > 0 && failed * 2 > translated {
                    return Err(v.advice.unwrap_or_else(|| {
                        "Most lines couldn't be translated into English, so an analysis would be guesswork. Check the translation, or try a larger model in Settings › Analysis.".into()
                    }));
                }
            }
            let input = if translate_first { input_in(item, &language)? } else { input(item)? };
            let glossary = stv_core::vocab::glossary(&vocabulary::load(), 300);
            let date = input.date();
            let ctx = Context {
                title: &input.title,
                date: &date,
                duration_s: input.duration_s,
                speakers: &input.speakers,
                glossary: &glossary,
                is_note: input.is_note,
                bookmarks: &input.bookmarks,
                language: &language,
            };
            let started = std::time::Instant::now();
            let checked = engine::analyse(model, &t, &input.turns, &ctx, |k, n| progress("Analysing", k, n))?;
            let mut result = checked.result;
            engine::map_evidence(&t, &mut result, |v| v.as_u64().and_then(|n| input.anchor(n)));
            let entry = Entry {
                template: TemplateRef { id: t.id.clone(), name: t.name.clone(), version: t.version, hash: t.hash.clone() },
                engine: engine.clone(),
                created: library::now(),
                input_hash: original_hash.clone(),
                warnings: checked.warnings,
                collapsed: false,
                seconds: started.elapsed().as_secs_f64(),
                result,
                language: if language == stv_core::lang::ENGLISH { String::new() } else { language.clone() },
                via: if translate_first { "translation".into() } else { String::new() },
                from_created: None,
            };
            made.push(entry.clone());
            entry
        }
    };
    if let Some(to) = into {
        let from = if base.language.is_empty() { stv_core::lang::ENGLISH.to_string() } else { base.language.clone() };
        if to != from {
            let started = std::time::Instant::now();
            let lines = stv_core::translate::analysis_lines(&t, &base.result);
            let keep: Vec<String> = vocabulary::load().into_iter().map(|t| t.spelling).collect();
            let done = stv_core::translate::translate(model, &lines, &[from], to, &keep, |k, n| progress("Translating", k, n))?;
            let warnings = done.iter().filter_map(|d| d.warning.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            made.push(Entry {
                result: stv_core::translate::apply(&base.result, &done),
                language: to.to_string(),
                via: "translated".into(),
                from_created: Some(base.created),
                created: library::now(),
                seconds: started.elapsed().as_secs_f64(),
                warnings,
                engine,
                collapsed: false,
                ..base
            });
        }
    }
    Ok(made)
}

/// Puts a result in an item's stack: in place of the last result with the same key (keeping what the
/// person edited, ticked, added or deleted there), or at the end; a translation goes just after the entry
/// it was translated from.
pub fn put(item: &Item, mut entry: Entry) -> Result<(), String> {
    let t = template(&entry.template.id)?;
    let mut stack = load(item);
    let key = entry.key();
    match stack.stack.iter_mut().find(|e| e.key() == key) {
        Some(old) => {
            engine::carry_over(&t, &old.result, &mut entry.result);
            entry.collapsed = old.collapsed;
            *old = entry;
        }
        None => {
            let after = stack.stack.iter().rposition(|e| e.template.id == entry.template.id).map_or(stack.stack.len(), |i| i + 1);
            stack.stack.insert(after, entry);
        }
    }
    save(item, &stack)
}

/// An entry as the page shows it, with its template, whether it's out of date, and its citations resolved.
#[derive(Debug, Clone, Serialize)]
pub struct EntryView {
    #[serde(flatten)]
    pub entry: Entry,
    pub key: String,
    pub definition: Option<Template>,
    /// The transcript or note changed since it was made, or for a translation, the entry it came from
    /// was made again.
    pub stale: bool,
    /// The template changed since it was made.
    pub template_changed: bool,
    pub sources: BTreeMap<String, Option<SourceView>>,
}

pub fn view(item: &Item) -> Vec<EntryView> {
    let stack = load(item);
    if stack.stack.is_empty() {
        return vec![];
    }
    let Ok(input) = input(item) else { return vec![] };
    let all = templates();
    let hash = input.hash();
    let made: BTreeMap<String, u64> = stack.stack.iter().filter(|e| e.via != "translated").map(|e| (e.template.id.clone(), e.created)).collect();
    stack
        .stack
        .into_iter()
        .map(|entry| {
            let definition = all.iter().find(|t| t.id == entry.template.id).and_then(|t| t.template.clone());
            let mut sources = BTreeMap::new();
            collect_anchors(&entry.result, &mut |a| {
                sources.entry(a.to_string()).or_insert_with(|| input.resolve(a));
            });
            let source_remade = entry.from_created.is_some_and(|c| made.get(&entry.template.id).is_some_and(|m| *m != c));
            EntryView {
                stale: entry.input_hash != hash || source_remade,
                template_changed: definition.as_ref().is_some_and(|d| d.hash != entry.template.hash),
                key: entry.key(),
                definition,
                sources,
                entry,
            }
        })
        .collect()
}

fn collect_anchors(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) if s.starts_with("s:") || s.starts_with("n:") => f(s),
        Value::Array(a) => a.iter().for_each(|x| collect_anchors(x, f)),
        Value::Object(o) => o.values().for_each(|x| collect_anchors(x, f)),
        _ => {}
    }
}

/// Changes one entry of an item's stack.
fn change(item: &Item, key: &str, f: impl FnOnce(&mut Entry) -> Result<(), String>) -> Result<(), String> {
    let mut stack = load(item);
    let entry = stack.stack.iter_mut().find(|e| e.key() == key).ok_or("That analysis no longer exists.")?;
    f(entry)?;
    save(item, &stack)
}

/// One edit to an analysis, from the page.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    /// Sets a value: `field` alone for a top-level field, or with `index` and `key` for one item's.
    Set { field: String, index: Option<usize>, key: Option<String>, value: Value },
    Done { field: String, index: usize, done: bool },
    Remove { field: String, index: usize },
    /// A task or point the person adds themselves.
    Add { field: String, item: Value },
    Collapse { collapsed: bool },
}

fn list<'a>(result: &'a mut Value, field: &str) -> Result<&'a mut Vec<Value>, String> {
    result.get_mut(field).and_then(Value::as_array_mut).ok_or_else(|| "That section no longer exists.".to_string())
}

pub fn edit(item: &Item, key: &str, edit: Edit) -> Result<(), String> {
    change(item, key, |entry| {
        let result = &mut entry.result;
        match edit {
            Edit::Set { field, index: None, value, .. } => {
                if field.starts_with('_') || result.get(&field).is_none() {
                    return Err("That section no longer exists.".into());
                }
                result[&field] = value;
                let mut edited = result["_edited"].as_array().cloned().unwrap_or_default();
                if !edited.contains(&json!(field)) {
                    edited.push(json!(field));
                }
                result["_edited"] = Value::Array(edited);
            }
            Edit::Set { field, index: Some(i), key, value } => {
                let key = key.ok_or("Which part?")?;
                if key.starts_with('_') && key != "_who_print" {
                    return Err("That can't be changed.".into());
                }
                let it = list(result, &field)?.get_mut(i).ok_or("That item no longer exists.")?;
                it[&key] = value;
                it["_edited"] = Value::Bool(true);
            }
            Edit::Done { field, index, done } => {
                list(result, &field)?.get_mut(index).ok_or("That item no longer exists.")?["_done"] = Value::Bool(done);
            }
            Edit::Remove { field, index } => {
                let items = list(result, &field)?;
                let it = items.get_mut(index).ok_or("That item no longer exists.")?;
                // Kept as removed, so making it again doesn't bring it back; one the person added just goes.
                if it["_added"].as_bool().unwrap_or(false) {
                    items.remove(index);
                } else {
                    it["_removed"] = Value::Bool(true);
                }
            }
            Edit::Add { field, mut item } => {
                if !item.is_object() {
                    return Err("That isn't an item.".into());
                }
                item["_added"] = Value::Bool(true);
                list(result, &field)?.push(item);
            }
            Edit::Collapse { collapsed } => entry.collapsed = collapsed,
        }
        Ok(())
    })
}

pub fn remove(item: &Item, key: &str) -> Result<(), String> {
    let mut stack = load(item);
    stack.stack.retain(|e| e.key() != key);
    save(item, &stack)
}

/// Moves an analysis up (-1) or down (+1) the stack.
pub fn shift(item: &Item, key: &str, by: i32) -> Result<(), String> {
    let mut stack = load(item);
    let i = stack.stack.iter().position(|e| e.key() == key).ok_or("That analysis no longer exists.")?;
    let j = (i as i32 + by).clamp(0, stack.stack.len() as i32 - 1) as usize;
    stack.stack.swap(i, j);
    save(item, &stack)
}

// Every analysis of an item as document sections, Markdown or plain text, for the exports. Citations show
/// as times in a meeting and paragraph numbers in a note.
pub fn render(stack: &Stack, input: &Input, md: bool) -> String {
    let all = templates();
    let label = |v: &Value| v.as_str().and_then(|a| input.resolve(a)).map(|s| s.label);
    let mut out = String::new();
    for e in &stack.stack {
        let Some(t) = all.iter().find(|t| t.id == e.template.id).and_then(|t| t.template.clone()) else { continue };
        out += &engine::render(&t, &e.result, &label, md);
        let place = if e.engine.kind == "local" { "this device" } else { e.engine.host.as_deref().unwrap_or("a server") };
        let how = match e.via.as_str() {
            "translated" => format!(" In {}, machine-translated.", stv_core::lang::name(&e.language)),
            "translation" => " Made from an English translation of the transcript.".to_string(),
            _ => String::new(),
        };
        let made = format!("Made on {place} with {}.{how} Check it against the transcript.", e.engine.model);
        out += &if md { format!("_{}_\n\n", engine::escape_md(&made)) } else { format!("{made}\n\n") };
    }
    out
}

/// The analyses of a meeting, rendered for its documents, or empty when it has none.
pub fn render_meeting(dir: &Path, meta: &Meta, segments: &[Segment], md: bool) -> String {
    let stack: Stack = library::read_json(&dir.join("analysis.json")).unwrap_or_default();
    if stack.stack.is_empty() {
        return String::new();
    }
    render(&stack, &meeting_input(meta, segments), md)
}

pub fn render_note(note: &notes::Note, md: bool) -> String {
    let stack = load(&Item::Note(note.id.clone()));
    if stack.stack.is_empty() {
        return String::new();
    }
    render(&stack, &note_input(note), md)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: usize, stream: &str, start_s: f64, text: &str) -> Segment {
        Segment { id, speaker: stream.into(), stream: stream.into(), start_s, end_s: start_s + 2.0, text: text.into(), confidence: 0.9, ..Default::default() }
    }

    #[test]
    fn meeting_anchors_survive_renumbering() {
        let meta = Meta { title: "T".into(), ..Default::default() };
        let segs = vec![seg(0, "alice", 1.0, "Hello there."), seg(1, "raj", 5.0, "I'll send it by Friday.")];
        let input = meeting_input(&meta, &segs);
        let anchor = input.anchor(2).unwrap();
        assert_eq!(anchor, json!("s:raj:5.00"));
        // Reprocessed: ids shifted and the start moved a little.
        let again = vec![seg(0, "alice", 0.5, "Hi."), seg(1, "alice", 1.0, "Hello there."), seg(2, "raj", 5.3, "I'll send it by Friday.")];
        let found = meeting_input(&meta, &again).resolve(anchor.as_str().unwrap()).unwrap();
        assert_eq!(found.segment, Some(2));
        assert_eq!(found.label, "00:05");
        assert_eq!(found.before.as_deref(), Some("alice: Hello there."));
    }

    #[test]
    fn note_anchors_follow_a_moved_sentence() {
        let note = notes::Note { id: "n1".into(), body: "Buy milk. Call Sam tomorrow.".into(), ..Default::default() };
        let anchor = note_input(&note).anchor(2).unwrap();
        let moved = notes::Note { body: "A new first line.\nBuy milk. Call Sam tomorrow.".into(), ..note };
        let input = note_input(&moved);
        let found = input.resolve(anchor.as_str().unwrap()).unwrap();
        assert_eq!(found.text, "Call Sam tomorrow.");
        let (a, b) = found.range.unwrap();
        assert_eq!(&moved.body[a..b], "Call Sam tomorrow.");
    }

    #[test]
    fn dates_read_like_people_write_them() {
        let mut input = note_input(&notes::Note::default());
        input.created = 1_794_000_000;
        assert_eq!(input.date(), "6 Nov 2026");
    }
}
