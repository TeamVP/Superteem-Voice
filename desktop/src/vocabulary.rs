//! The person's vocabulary: words speech to text should spell their way, kept on this device. Whisper is
//! told the words before it listens, and what it writes is corrected afterwards (see `stv_core::vocab`).

use crate::library::{self, Segment};
use std::path::Path;
use std::sync::Mutex;
use stv_core::vocab::{self, Corrector, Term, When};

const FILE: &str = "vocabulary.json";
/// Whisper's prompt holds about 220 tokens; the glossary leaves room for the rest of the prompt.
const GLOSSARY_CHARS: usize = 400;

static LOCK: Mutex<()> = Mutex::new(());

pub fn load() -> Vec<Term> {
    library::read_json(&library::data_dir().join(FILE)).unwrap_or_default()
}

fn store(terms: &[Term]) -> Result<(), String> {
    std::fs::create_dir_all(library::data_dir()).map_err(|e| e.to_string())?;
    library::write_json(&library::data_dir().join(FILE), &terms)
}

/// Tidies a term from the page: trims, drops empty and repeated sounds, and gives a new term an id.
fn tidy(mut term: Term, taken: &[Term]) -> Result<Term, String> {
    term.spelling = term.spelling.trim().to_string();
    if vocab::plain(&term.spelling).is_empty() {
        return Err("Type how the word is spelled.".into());
    }
    let mut sounds: Vec<String> = Vec::new();
    for s in &term.sounds_like {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if !vocab::plain(&s).is_empty() && !sounds.iter().any(|x| vocab::plain(x) == vocab::plain(&s)) {
            sounds.push(s);
        }
    }
    term.sounds_like = sounds;
    if let When::Near(words) = &mut term.when {
        words.retain(|w| !vocab::plain(w).is_empty());
        if words.is_empty() {
            return Err("Add at least one word it's said near, or choose another option.".into());
        }
    }
    if term.id.is_empty() {
        let base: String = vocab::plain(&term.spelling).chars().take(24).collect();
        term.id = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| !taken.iter().any(|t| &t.id == id)).unwrap();
    }
    let same = |t: &&Term| t.id != term.id && vocab::plain(&t.spelling) == vocab::plain(&term.spelling);
    if let Some(other) = taken.iter().find(same) {
        return Err(format!("“{}” is already in the vocabulary.", other.spelling));
    }
    Ok(term)
}

/// Adds a term, or replaces the one with its id. Returns the saved term.
pub fn save(term: Term) -> Result<Term, String> {
    let _guard = LOCK.lock().unwrap();
    let mut terms = load();
    let term = tidy(term, &terms)?;
    match terms.iter_mut().find(|t| t.id == term.id) {
        Some(t) => *t = Term { uses: t.uses, ..term.clone() },
        None => terms.push(term.clone()),
    }
    store(&terms)?;
    Ok(term)
}

pub fn delete(id: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap();
    let mut terms = load();
    terms.retain(|t| t.id != id);
    store(&terms)
}

/// `base` followed by the vocabulary, for steering whisper. None when there is nothing to steer with.
pub fn prompt(base: &str, terms: &[Term]) -> Option<String> {
    let glossary = vocab::glossary(terms, GLOSSARY_CHARS);
    match (base.is_empty(), glossary.is_empty()) {
        (true, true) => None,
        (true, false) => Some(glossary),
        (false, true) => Some(base.to_string()),
        (false, false) => Some(format!("{base} {glossary}")),
    }
}

/// Counts the changes written in, so the most used words steer whisper first.
fn count_uses(fixes: &[&vocab::Fix]) {
    if fixes.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    let mut terms = load();
    for f in fixes.iter().filter(|f| f.applied) {
        if let Some(t) = terms.iter_mut().find(|t| t.id == f.term) {
            t.uses += 1;
        }
    }
    let _ = store(&terms);
}

/// Corrects dictated words. Suggestions are left out, as a note has nowhere to show them.
pub fn correct_text(text: &str) -> String {
    let terms = load();
    if terms.is_empty() {
        return text.to_string();
    }
    if vocab::is_glossary_echo(text, &terms) {
        return String::new();
    }
    let c = Corrector::new(&terms).apply(text);
    count_uses(&c.fixes.iter().collect::<Vec<_>>());
    c.text
}

/// Corrects the lines the person hasn't edited, keeping changes they undid. Returns how many words changed.
pub fn correct_segments(segments: &mut [Segment], terms: &[Term]) -> usize {
    let corrector = Corrector::new(terms);
    if corrector.is_empty() {
        return 0;
    }
    let mut new_fixes = Vec::new();
    for s in segments.iter_mut().filter(|s| !s.edited && !s.text.is_empty()) {
        let (c, new) = corrector.reapply(&s.text, &s.fixes);
        if !new.is_empty() {
            new_fixes.extend(new);
            s.text = c.text;
            s.fixes = c.fixes;
        }
    }
    count_uses(&new_fixes.iter().collect::<Vec<_>>());
    new_fixes.iter().filter(|f| f.applied).count()
}

/// Drops whisper writing the vocabulary back to itself over silence.
pub fn drop_echo(text: String, terms: &[Term]) -> String {
    if vocab::is_glossary_echo(&text, terms) { String::new() } else { text }
}

/// Corrects one saved meeting, or with `dry_run` only counts what would change.
pub fn correct_session(dir: &Path, terms: &[Term], dry_run: bool) -> Result<usize, String> {
    let mut segments = library::load_segments(dir);
    if segments.is_empty() {
        return Ok(0);
    }
    if dry_run {
        let corrector = Corrector::new(terms);
        return Ok(segments
            .iter()
            .filter(|s| !s.edited)
            .map(|s| corrector.reapply(&s.text, &s.fixes).1.iter().filter(|f| f.applied).count())
            .sum());
    }
    let changed = correct_segments(&mut segments, terms);
    if changed > 0 {
        library::save(dir, &library::load_meta(dir), &segments)?;
    }
    Ok(changed)
}

/// Undoes a change in a line, or makes a suggested one.
pub fn toggle(segment: &mut Segment, fix: usize) -> Result<(), String> {
    let c = vocab::toggle(&segment.text, &segment.fixes, fix)?;
    segment.text = c.text;
    segment.fixes = c.fixes;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(spelling: &str, sounds: &[&str]) -> Term {
        Term { spelling: spelling.into(), sounds_like: sounds.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn terms_are_tidied_and_given_ids() {
        let t = tidy(term("  SIBR ", &["cyber", " Cyber ", "", "sigh  ber"]), &[]).unwrap();
        assert_eq!((t.id.as_str(), t.spelling.as_str()), ("sibr", "SIBR"));
        assert_eq!(t.sounds_like, vec!["cyber", "sigh ber"]);
        let taken = [Term { id: "sibr".into(), spelling: "Sibr Labs".into(), ..Default::default() }];
        assert_eq!(tidy(term("SIBR", &[]), &taken).unwrap().id, "sibr-2");
        assert!(tidy(term("sibr labs", &[]), &taken).is_err());
        assert!(tidy(term(" . ", &[]), &[]).is_err());
        assert!(tidy(Term { when: When::Near(vec![" ".into()]), ..term("SIBR", &[]) }, &[]).is_err());
    }

    #[test]
    fn the_prompt_adds_the_vocabulary_to_the_base() {
        let terms = [term("SIBR", &["cyber"])];
        assert_eq!(prompt("Hey Superteem.", &terms).as_deref(), Some("Hey Superteem. SIBR."));
        assert_eq!(prompt("", &terms).as_deref(), Some("SIBR."));
        assert_eq!(prompt("", &[]), None);
    }

    #[test]
    fn edited_lines_are_left_alone() {
        let terms = [Term { id: "sibr".into(), ..term("SIBR", &["cyber"]) }];
        let mut segments = vec![
            Segment { text: "cyber is live".into(), ..Default::default() },
            Segment { text: "cyber typed by hand".into(), edited: true, ..Default::default() },
        ];
        let corrector = Corrector::new(&terms);
        for s in segments.iter_mut().filter(|s| !s.edited) {
            let (c, _) = corrector.reapply(&s.text, &s.fixes);
            s.text = c.text;
            s.fixes = c.fixes;
        }
        assert_eq!(segments[0].text, "SIBR is live");
        assert_eq!(segments[1].text, "cyber typed by hand");
        toggle(&mut segments[0], 0).unwrap();
        assert_eq!(segments[0].text, "cyber is live");
    }
}
