//! A person's own words: names, products and acronyms speech to text spells wrong because they sound like
//! everyday words, as "SIBR" sounds like "cyber". Each term says how it's written and what it sounds like;
//! transcripts are corrected after whisper has written them, and every change is kept so it can be undone.

use serde::{Deserialize, Serialize};

/// When a term replaces the everyday word it sounds like.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "words", rename_all = "lowercase")]
pub enum When {
    /// Every time.
    #[default]
    Always,
    /// Only with one of these words close by, as "cyber" means SIBR only when talking about the platform.
    Near(Vec<String>),
    /// Never by itself: the word is marked so the person can accept it with a click.
    Suggest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Term {
    pub id: String,
    /// How it's written, exactly: "SIBR".
    pub spelling: String,
    /// What speech to text writes instead: "cyber", "sigh ber".
    pub sounds_like: Vec<String>,
    /// Also said letter by letter, as "S I B R".
    pub letters: bool,
    pub when: When,
    /// Times it was written into a transcript, so the most used terms steer whisper first.
    pub uses: u32,
}

/// One change to a line, which the person can undo or, for a suggestion, accept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    /// The first word of the change, counting the line's words as they are now.
    pub at: usize,
    /// How many words the change covers as the line is now.
    pub words: usize,
    /// What whisper wrote.
    pub raw: String,
    /// The term's spelling, with the punctuation around the words kept.
    pub fixed: String,
    pub term: String,
    /// The spelling is in the line; false for a suggestion, or a change the person undid.
    pub applied: bool,
}

impl Fix {
    fn shown(&self) -> &str {
        if self.applied { &self.fixed } else { &self.raw }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Corrected {
    pub text: String,
    pub fixes: Vec<Fix>,
}

/// Context words are looked for this many words either side.
const NEAR_WORDS: usize = 12;
/// A sound key shorter than this matches too many words to suggest anything.
const MIN_SOUND_KEY: usize = 4;

/// Letters and digits only, lower case: "S.I.B.R." and "cy-ber" read as "sibr" and "cyber".
pub fn plain(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// A rough key for how a word sounds: consonants as they are said, each run of vowels as one "a".
/// "cyber", "saiber" and "sigh ber" all give "sabar". It ignores which vowel, so "saber" gives it too, which
/// is why a match by sound is only ever offered, never written in.
pub fn sound_key(text: &str) -> String {
    let w: Vec<char> = plain(text).chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let mut out: Vec<char> = Vec::new();
    let mut i = 0;
    let at = |i: usize| w.get(i).copied();
    // A silent e at the end: "rate" sounds like "rait".
    let end = if w.len() > 3 && w.last() == Some(&'e') && !is_vowel(w[w.len() - 2]) { w.len() - 1 } else { w.len() };
    while i < end {
        let c = w[i];
        let next = if i + 1 < end { at(i + 1) } else { None };
        let (sound, step) = match (c, next) {
            ('k', Some('n')) | ('g', Some('n')) | ('w', Some('r')) if i == 0 => ("", 1),
            ('p', Some('h')) => ("f", 2),
            ('g', Some('h')) => (if i == 0 { "g" } else { "" }, 2),
            ('c', Some('k')) => ("k", 2),
            ('c', Some('h')) | ('t', Some('c')) if c == 'c' || at(i + 2) == Some('h') => ("ch", if c == 'c' { 2 } else { 3 }),
            ('s', Some('h')) => ("sh", 2),
            ('t', Some('h')) => ("th", 2),
            ('q', Some('u')) => ("kw", 2),
            ('q', _) => ("k", 1),
            ('c', Some('e' | 'i' | 'y')) => ("s", 1),
            ('c', _) => ("k", 1),
            ('x', _) => ("ks", 1),
            ('z', _) => ("s", 1),
            ('d', Some('g')) => ("j", 2),
            ('w', Some(n)) if !is_vowel(n) => ("", 1),
            ('w' | 'h', None) => ("", 1),
            // "y" is a vowel unless it starts a syllable.
            ('y', n) if i > 0 || n.is_none_or(|n| !is_vowel(n)) => ("a", 1),
            (v, _) if is_vowel(v) => ("a", 1),
            ('h', _) if i > 0 => ("", 1),
            _ => (match c { 'b' => "b", 'd' => "d", 'f' => "f", 'g' => "g", 'h' => "h", 'j' => "j", 'k' => "k", 'l' => "l", 'm' => "m", 'n' => "n", 'p' => "p", 'r' => "r", 's' => "s", 't' => "t", 'v' => "v", 'w' => "w", 'y' => "y", _ => "" }, 1),
        };
        for s in sound.chars() {
            // Doubled letters and runs of vowels are one sound.
            if out.last() != Some(&s) {
                out.push(s);
            }
        }
        i += step;
    }
    out.into_iter().collect()
}

/// The ways one term can turn up in whisper's words.
struct Forms {
    /// Unmistakably the term: its own spelling in any case, and its letters said one by one.
    sure: Vec<String>,
    /// The everyday words it sounds like.
    sounds: Vec<String>,
    /// Their sound keys.
    keys: Vec<String>,
    /// The most words any form spans.
    span: usize,
}

fn forms(term: &Term) -> Forms {
    let spelling = plain(&term.spelling);
    let mut sure = vec![spelling.clone()];
    let mut span = term.spelling.split_whitespace().count().max(1);
    if term.letters {
        span = span.max(spelling.chars().count());
    }
    let mut sounds = Vec::new();
    for s in &term.sounds_like {
        let p = plain(s);
        if !p.is_empty() && p != spelling && !sounds.contains(&p) {
            sounds.push(p);
            span = span.max(s.split_whitespace().count());
        }
    }
    let mut keys: Vec<String> = sounds.iter().map(|s| sound_key(s)).filter(|k| k.len() >= MIN_SOUND_KEY).collect();
    keys.dedup();
    sure.retain(|s| !s.is_empty());
    Forms { sure, sounds, keys, span }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sure,
    Sounds,
    Sound,
}

pub struct Corrector {
    terms: Vec<(Term, Forms)>,
    span: usize,
}

/// A word split into what comes before its letters, the letters, and what comes after, possessive included:
/// "(cyber's," is "(", "cyber", "'s,".
fn split_word(word: &str) -> (&str, &str, &str) {
    let start = word.find(|c: char| c.is_alphanumeric()).unwrap_or(word.len());
    let end = word.rfind(|c: char| c.is_alphanumeric()).map_or(start, |i| i + word[i..].chars().next().unwrap().len_utf8());
    let (lead, rest) = word.split_at(start);
    let (mut core, mut trail) = rest.split_at(end - start);
    for s in ["'s", "’s", "'S"] {
        if core.len() > s.len() + 1 && core.ends_with(s) {
            let cut = core.len() - s.len();
            trail = &word[start + cut..];
            core = &core[..cut];
            break;
        }
    }
    (lead, core, trail)
}

impl Corrector {
    pub fn new(terms: &[Term]) -> Self {
        let terms: Vec<(Term, Forms)> =
            terms.iter().filter(|t| !t.spelling.trim().is_empty()).map(|t| (t.clone(), forms(t))).collect();
        let span = terms.iter().map(|(_, f)| f.span).max().unwrap_or(1);
        Self { terms, span }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Corrects whisper's words.
    pub fn apply(&self, text: &str) -> Corrected {
        self.correct(text, &[]).0
    }

    /// Corrects a line that may already have changes, leaving those words alone so an undone change stays
    /// undone. Returns the line with every change in it, old and new, and the new changes on their own.
    pub fn reapply(&self, text: &str, existing: &[Fix]) -> (Corrected, Vec<Fix>) {
        let protected: Vec<(usize, usize)> = existing.iter().map(|f| (f.at, f.at + f.words)).collect();
        let (mut out, map) = self.correct(text, &protected);
        let new = out.fixes.clone();
        let mut fixes: Vec<Fix> = existing.iter().map(|f| Fix { at: map[f.at], ..f.clone() }).collect();
        fixes.append(&mut out.fixes);
        fixes.sort_by_key(|f| f.at);
        out.fixes = fixes;
        (out, new)
    }

    /// Also returns, for each word of `text`, where it is in the corrected line.
    fn correct(&self, text: &str, protected: &[(usize, usize)]) -> (Corrected, Vec<usize>) {
        let words: Vec<&str> = text.split_whitespace().collect();
        let cores: Vec<&str> = words.iter().map(|w| split_word(w).1).collect();
        let keys: Vec<String> = cores.iter().map(|c| plain(c)).collect();
        let mut out: Vec<String> = Vec::new();
        let mut fixes = Vec::new();
        let mut map = vec![0; words.len() + 1];
        let mut i = 0;
        while i < words.len() {
            map[i] = out.len();
            let free = |a: usize, b: usize| !protected.iter().any(|&(s, e)| a < e && s < b) && !keys[a..b].iter().any(String::is_empty);
            let mut found = None;
            'span: for n in (1..=self.span.min(words.len() - i)).rev() {
                if !free(i, i + n) {
                    continue;
                }
                // Words joined without spaces, so "sigh ber" and "sighber" are the same.
                let joined: String = keys[i..i + n].concat();
                for (term, f) in &self.terms {
                    let kind = if f.sure.contains(&joined) && (n == term.spelling.split_whitespace().count() || (term.letters && joined.chars().count() == n)) {
                        Some(Kind::Sure)
                    } else if f.sounds.contains(&joined) {
                        Some(Kind::Sounds)
                    } else if n <= 2 && f.keys.contains(&sound_key(&joined)) {
                        Some(Kind::Sound)
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        found = Some((n, term, kind));
                        break 'span;
                    }
                }
            }
            let Some((n, term, kind)) = found else {
                out.push(words[i].to_string());
                i += 1;
                continue;
            };
            let near = || match &term.when {
                When::Near(context) => {
                    let ctx: Vec<String> = context.iter().map(|c| plain(c)).filter(|c| !c.is_empty()).collect();
                    let from = i.saturating_sub(NEAR_WORDS);
                    let to = (i + n + NEAR_WORDS).min(words.len());
                    (from..to).filter(|&j| j < i || j >= i + n).any(|j| ctx.contains(&keys[j]))
                }
                _ => true,
            };
            let applied = match (kind, &term.when) {
                (Kind::Sure, _) => Some(true),
                (Kind::Sounds, When::Always) => Some(true),
                (Kind::Sounds, When::Near(_)) => near().then_some(true),
                (Kind::Sounds, When::Suggest) => Some(false),
                (Kind::Sound, _) => near().then_some(false),
            };
            let raw = words[i..i + n].join(" ");
            let (lead, _, _) = split_word(words[i]);
            let (_, last, mut trail) = split_word(words[i + n - 1]);
            // "S.I.B.R." ends in the dot of its last letter, not a full stop, unless the line ends there.
            if last.contains('.') && i + n < words.len() {
                trail = trail.strip_prefix('.').unwrap_or(trail);
            }
            let fixed = format!("{lead}{}{trail}", term.spelling.trim());
            match applied {
                Some(applied) if fixed != raw => {
                    let shown = if applied { &fixed } else { &raw };
                    fixes.push(Fix { at: out.len(), words: shown.split_whitespace().count(), raw: raw.clone(), fixed: fixed.clone(), term: term.id.clone(), applied });
                    out.extend(shown.split_whitespace().map(str::to_string));
                }
                _ => out.extend(words[i..i + n].iter().map(|w| w.to_string())),
            }
            for k in i + 1..i + n {
                map[k] = map[i];
            }
            i += n;
        }
        map[words.len()] = out.len();
        (Corrected { text: out.join(" "), fixes }, map)
    }
}

/// Undoes a change, or makes one that was only suggested. The line must still read as the change left it.
pub fn toggle(text: &str, fixes: &[Fix], index: usize) -> Result<Corrected, String> {
    let fix = fixes.get(index).ok_or("That change is no longer in the line.")?;
    let mut words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    let end = fix.at + fix.words;
    if end > words.len() || words[fix.at..end].join(" ") != fix.shown() {
        return Err("The line was changed since, so this can't be undone here.".into());
    }
    let mut flipped = fix.clone();
    flipped.applied = !fix.applied;
    let replacement: Vec<String> = flipped.shown().split_whitespace().map(str::to_string).collect();
    flipped.words = replacement.len();
    let delta = replacement.len() as isize - fix.words as isize;
    words.splice(fix.at..end, replacement);
    let fixes = fixes
        .iter()
        .enumerate()
        .map(|(k, f)| match k {
            _ if k == index => flipped.clone(),
            _ if f.at >= end => Fix { at: (f.at as isize + delta) as usize, ..f.clone() },
            _ => f.clone(),
        })
        .collect();
    Ok(Corrected { text: words.join(" "), fixes })
}

/// The words for whisper's prompt, most used first, kept short as whisper reads at most about 220 tokens of
/// prompt and a long one makes it more likely to repeat the prompt instead of listening.
pub fn glossary(terms: &[Term], max_chars: usize) -> String {
    let mut sorted: Vec<&Term> = terms.iter().filter(|t| !t.spelling.trim().is_empty()).collect();
    sorted.sort_by(|a, b| b.uses.cmp(&a.uses));
    let mut out = String::new();
    for t in sorted {
        let word = t.spelling.trim();
        if out.len() + word.len() + 2 > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(word);
    }
    if !out.is_empty() {
        out.push('.');
    }
    out
}

/// Whisper given a prompt sometimes writes the prompt back when it hears nothing. Two or more of the terms
/// and nothing else is that, not speech.
pub fn is_glossary_echo(text: &str, terms: &[Term]) -> bool {
    let spellings: Vec<String> = terms.iter().map(|t| plain(&t.spelling)).filter(|s| !s.is_empty()).collect();
    let words: Vec<String> = text.split_whitespace().map(plain).filter(|w| !w.is_empty()).collect();
    words.len() >= 2 && words.iter().all(|w| spellings.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sibr(when: When) -> Term {
        Term { id: "sibr".into(), spelling: "SIBR".into(), sounds_like: vec!["cyber".into(), "sigh ber".into()], letters: true, when, uses: 0 }
    }

    fn fixed(terms: &[Term], text: &str) -> String {
        Corrector::new(terms).apply(text).text
    }

    #[test]
    fn sound_keys_match_spellings_that_sound_alike() {
        assert_eq!(sound_key("cyber"), "sabar");
        assert_eq!(sound_key("saiber"), sound_key("cyber"));
        assert_eq!(sound_key("sigh ber"), sound_key("cyber"));
        assert_ne!(sound_key("cypher"), sound_key("cyber"));
        assert_eq!(sound_key("Philly"), sound_key("filly"));
    }

    #[test]
    fn always_replaces_the_word_it_sounds_like() {
        let t = [sibr(When::Always)];
        assert_eq!(fixed(&t, "We shipped cyber today."), "We shipped SIBR today.");
        assert_eq!(fixed(&t, "Cyber, the platform"), "SIBR, the platform");
    }

    #[test]
    fn keeps_punctuation_and_possessives() {
        let t = [sibr(When::Always)];
        assert_eq!(fixed(&t, "(cyber's roadmap)"), "(SIBR's roadmap)");
        assert_eq!(fixed(&t, "Is it cyber?"), "Is it SIBR?");
    }

    #[test]
    fn near_needs_a_context_word() {
        let t = [sibr(When::Near(vec!["platform".into()]))];
        assert_eq!(fixed(&t, "cyber attacks are up"), "cyber attacks are up");
        assert_eq!(fixed(&t, "the cyber platform is live"), "the SIBR platform is live");
    }

    #[test]
    fn split_words_and_letters_are_caught() {
        let t = [sibr(When::Always)];
        assert_eq!(fixed(&t, "we use sigh ber now"), "we use SIBR now");
        assert_eq!(fixed(&t, "it's S I B R."), "it's SIBR.");
        assert_eq!(fixed(&t, "it's S.I.B.R. now"), "it's SIBR now");
        assert_eq!(fixed(&t, "on sibr"), "on SIBR");
        assert_eq!(fixed(&t, "a b r"), "a b r");
    }

    #[test]
    fn letters_are_only_read_as_letters_when_asked() {
        let t = [Term { letters: false, ..sibr(When::Always) }];
        assert_eq!(fixed(&t, "S I B R"), "S I B R");
    }

    #[test]
    fn suggestions_leave_the_words_and_record_the_change() {
        let c = Corrector::new(&[sibr(When::Suggest)]).apply("the cyber team");
        assert_eq!(c.text, "the cyber team");
        assert_eq!(c.fixes, vec![Fix { at: 1, words: 1, raw: "cyber".into(), fixed: "SIBR".into(), term: "sibr".into(), applied: false }]);
    }

    #[test]
    fn a_match_by_sound_is_only_suggested() {
        let c = Corrector::new(&[sibr(When::Always)]).apply("a saiber release");
        assert_eq!(c.text, "a saiber release");
        assert!(!c.fixes[0].applied);
    }

    #[test]
    fn toggling_undoes_and_redoes_a_change_and_keeps_later_ones_in_place() {
        let t = [sibr(When::Always)];
        let c = Corrector::new(&t).apply("sigh ber then cyber again");
        assert_eq!(c.text, "SIBR then SIBR again");
        let undone = toggle(&c.text, &c.fixes, 0).unwrap();
        assert_eq!(undone.text, "sigh ber then SIBR again");
        assert_eq!(undone.fixes[1].at, 3);
        let undone_both = toggle(&undone.text, &undone.fixes, 1).unwrap();
        assert_eq!(undone_both.text, "sigh ber then cyber again");
        assert_eq!(toggle(&undone_both.text, &undone_both.fixes, 0).unwrap().text, "SIBR then cyber again");
        assert!(toggle("something else", &c.fixes, 0).is_err());
    }

    #[test]
    fn reapplying_leaves_undone_changes_undone() {
        let t = [sibr(When::Always)];
        let c = Corrector::new(&t).apply("cyber one");
        let undone = toggle(&c.text, &c.fixes, 0).unwrap();
        let (again, new) = Corrector::new(&t).reapply(&format!("{} and cyber two", undone.text), &undone.fixes);
        assert_eq!(new.len(), 1);
        assert_eq!(again.text, "cyber one and SIBR two");
        assert_eq!(again.fixes.len(), 2);
        assert!(!again.fixes[0].applied && again.fixes[1].applied);
        assert_eq!(again.fixes[1].at, 3);
    }

    #[test]
    fn the_glossary_puts_most_used_first_and_stays_short() {
        let a = Term { spelling: "SIBR".into(), uses: 1, ..Default::default() };
        let b = Term { spelling: "Kubernetes".into(), uses: 5, ..Default::default() };
        assert_eq!(glossary(&[a.clone(), b.clone()], 100), "Kubernetes, SIBR.");
        assert_eq!(glossary(&[a.clone(), b.clone()], 12), "Kubernetes.");
        assert!(is_glossary_echo("Kubernetes, SIBR.", &[a.clone(), b.clone()]));
        assert!(!is_glossary_echo("SIBR.", &[a.clone(), b.clone()]));
        assert!(!is_glossary_echo("SIBR is up", &[a, b]));
    }

    #[test]
    fn terms_round_trip_through_json() {
        let t = sibr(When::Near(vec!["platform".into()]));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Term>(&json).unwrap(), t);
    }
}
