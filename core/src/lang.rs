//! Languages: the catalogue of those Voice can work in, what works for each, and text handling that
//! doesn't assume English (Chinese has no spaces between words, Arabic runs right to left, and so on).
//!
//! A note or meeting is in one or two languages. The first decides the speech model and how analyses
//! run; with two, each line is heard as one of them.

use serde::Serialize;
use std::collections::BTreeSet;

/// How well something works in a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    None,
    Weak,
    Fair,
    Good,
}

/// How analyses run for a language: written straight from the transcript, or from its English
/// translation and translated back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Analysis {
    Direct,
    ViaEnglish,
}

/// The writing system a language is mostly written in, used to tell languages apart in text and to spot
/// a translation that came back untranslated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Script {
    Latin,
    Han,
    Kana,
    Hangul,
    Arabic,
    Hebrew,
    Cyrillic,
    Greek,
    Devanagari,
    Bengali,
    Tamil,
    Telugu,
    Thai,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct Language {
    /// The code Voice stores ("cy", "zh-Hant").
    pub code: &'static str,
    /// Its name in English.
    pub name: &'static str,
    /// Its name in itself.
    pub native: &'static str,
    /// The code whisper knows it by.
    pub whisper: &'static str,
    pub script: Script,
    pub rtl: bool,
    /// How well the multilingual speech model transcribes it.
    pub speech: Quality,
    pub analysis: Analysis,
}

const fn lang(code: &'static str, name: &'static str, native: &'static str, whisper: &'static str, script: Script, speech: Quality, analysis: Analysis) -> Language {
    let rtl = matches!(script, Script::Arabic | Script::Hebrew);
    Language { code, name, native, whisper, script, rtl, speech, analysis }
}

use Analysis::{Direct, ViaEnglish};
use Quality::{Fair, Good, Weak};
use Script::*;

/// Every language that can be chosen. Speech ratings follow whisper large-v3's published error rates;
/// analyses run directly in the languages the Qwen models write well, and through English otherwise.
pub const LANGUAGES: &[Language] = &[
    lang("af", "Afrikaans", "Afrikaans", "af", Latin, Fair, ViaEnglish),
    lang("ar", "Arabic", "العربية", "ar", Arabic, Fair, ViaEnglish),
    lang("hy", "Armenian", "Հայերեն", "hy", Other, Weak, ViaEnglish),
    lang("az", "Azerbaijani", "Azərbaycanca", "az", Latin, Weak, ViaEnglish),
    lang("eu", "Basque", "Euskara", "eu", Latin, Weak, ViaEnglish),
    lang("be", "Belarusian", "Беларуская", "be", Cyrillic, Weak, ViaEnglish),
    lang("bn", "Bengali", "বাংলা", "bn", Bengali, Weak, ViaEnglish),
    lang("bs", "Bosnian", "Bosanski", "bs", Latin, Fair, ViaEnglish),
    lang("bg", "Bulgarian", "Български", "bg", Cyrillic, Fair, ViaEnglish),
    lang("my", "Burmese", "မြန်မာ", "my", Other, Weak, ViaEnglish),
    lang("yue", "Cantonese", "粵語", "yue", Han, Fair, Direct),
    lang("ca", "Catalan", "Català", "ca", Latin, Good, ViaEnglish),
    lang("zh", "Chinese (Mandarin, Simplified)", "中文（简体）", "zh", Han, Good, Direct),
    lang("zh-Hant", "Chinese (Mandarin, Traditional)", "中文（繁體）", "zh", Han, Good, Direct),
    lang("hr", "Croatian", "Hrvatski", "hr", Latin, Fair, ViaEnglish),
    lang("cs", "Czech", "Čeština", "cs", Latin, Good, ViaEnglish),
    lang("da", "Danish", "Dansk", "da", Latin, Fair, ViaEnglish),
    lang("nl", "Dutch", "Nederlands", "nl", Latin, Good, Direct),
    lang("en", "English", "English", "en", Latin, Good, Direct),
    lang("et", "Estonian", "Eesti", "et", Latin, Weak, ViaEnglish),
    lang("fil", "Filipino", "Filipino", "tl", Latin, Fair, ViaEnglish),
    lang("fi", "Finnish", "Suomi", "fi", Latin, Good, ViaEnglish),
    lang("fr", "French", "Français", "fr", Latin, Good, Direct),
    lang("gl", "Galician", "Galego", "gl", Latin, Good, ViaEnglish),
    lang("ka", "Georgian", "ქართული", "ka", Other, Weak, ViaEnglish),
    lang("de", "German", "Deutsch", "de", Latin, Good, Direct),
    lang("el", "Greek", "Ελληνικά", "el", Greek, Fair, ViaEnglish),
    lang("gu", "Gujarati", "ગુજરાતી", "gu", Other, Weak, ViaEnglish),
    lang("ha", "Hausa", "Hausa", "ha", Latin, Weak, ViaEnglish),
    lang("he", "Hebrew", "עברית", "he", Hebrew, Fair, ViaEnglish),
    lang("hi", "Hindi", "हिन्दी", "hi", Devanagari, Fair, ViaEnglish),
    lang("hu", "Hungarian", "Magyar", "hu", Latin, Fair, ViaEnglish),
    lang("is", "Icelandic", "Íslenska", "is", Latin, Weak, ViaEnglish),
    lang("id", "Indonesian", "Bahasa Indonesia", "id", Latin, Good, ViaEnglish),
    lang("it", "Italian", "Italiano", "it", Latin, Good, Direct),
    lang("ja", "Japanese", "日本語", "ja", Kana, Good, Direct),
    lang("kn", "Kannada", "ಕನ್ನಡ", "kn", Other, Weak, ViaEnglish),
    lang("kk", "Kazakh", "Қазақ тілі", "kk", Cyrillic, Weak, ViaEnglish),
    lang("km", "Khmer", "ខ្មែរ", "km", Other, Weak, ViaEnglish),
    lang("ko", "Korean", "한국어", "ko", Hangul, Good, Direct),
    lang("lo", "Lao", "ລາວ", "lo", Other, Weak, ViaEnglish),
    lang("lv", "Latvian", "Latviešu", "lv", Latin, Fair, ViaEnglish),
    lang("lt", "Lithuanian", "Lietuvių", "lt", Latin, Fair, ViaEnglish),
    lang("mk", "Macedonian", "Македонски", "mk", Cyrillic, Fair, ViaEnglish),
    lang("ms", "Malay", "Bahasa Melayu", "ms", Latin, Good, ViaEnglish),
    lang("ml", "Malayalam", "മലയാളം", "ml", Other, Weak, ViaEnglish),
    lang("mt", "Maltese", "Malti", "mt", Latin, Weak, ViaEnglish),
    lang("mi", "Māori", "Te Reo Māori", "mi", Latin, Weak, ViaEnglish),
    lang("mr", "Marathi", "मराठी", "mr", Devanagari, Weak, ViaEnglish),
    lang("mn", "Mongolian", "Монгол", "mn", Cyrillic, Weak, ViaEnglish),
    lang("ne", "Nepali", "नेपाली", "ne", Devanagari, Weak, ViaEnglish),
    lang("no", "Norwegian", "Norsk", "no", Latin, Fair, ViaEnglish),
    lang("fa", "Persian", "فارسی", "fa", Arabic, Fair, ViaEnglish),
    lang("pl", "Polish", "Polski", "pl", Latin, Good, ViaEnglish),
    lang("pt", "Portuguese", "Português", "pt", Latin, Good, Direct),
    lang("pa", "Punjabi", "ਪੰਜਾਬੀ", "pa", Other, Weak, ViaEnglish),
    lang("ro", "Romanian", "Română", "ro", Latin, Fair, ViaEnglish),
    lang("ru", "Russian", "Русский", "ru", Cyrillic, Good, Direct),
    lang("sr", "Serbian", "Српски", "sr", Cyrillic, Fair, ViaEnglish),
    lang("sk", "Slovak", "Slovenčina", "sk", Latin, Fair, ViaEnglish),
    lang("sl", "Slovenian", "Slovenščina", "sl", Latin, Fair, ViaEnglish),
    lang("so", "Somali", "Soomaali", "so", Latin, Weak, ViaEnglish),
    lang("es", "Spanish", "Español", "es", Latin, Good, Direct),
    lang("sw", "Swahili", "Kiswahili", "sw", Latin, Weak, ViaEnglish),
    lang("sv", "Swedish", "Svenska", "sv", Latin, Good, ViaEnglish),
    lang("ta", "Tamil", "தமிழ்", "ta", Tamil, Weak, ViaEnglish),
    lang("te", "Telugu", "తెలుగు", "te", Telugu, Weak, ViaEnglish),
    lang("th", "Thai", "ไทย", "th", Thai, Fair, ViaEnglish),
    lang("tr", "Turkish", "Türkçe", "tr", Latin, Good, ViaEnglish),
    lang("uk", "Ukrainian", "Українська", "uk", Cyrillic, Good, ViaEnglish),
    lang("ur", "Urdu", "اردو", "ur", Arabic, Weak, ViaEnglish),
    lang("uz", "Uzbek", "Oʻzbek", "uz", Latin, Weak, ViaEnglish),
    lang("vi", "Vietnamese", "Tiếng Việt", "vi", Latin, Good, ViaEnglish),
    lang("cy", "Welsh", "Cymraeg", "cy", Latin, Weak, ViaEnglish),
    lang("yo", "Yoruba", "Yorùbá", "yo", Latin, Weak, ViaEnglish),
];

pub const ENGLISH: &str = "en";
/// Languages one note or meeting can be in.
pub const MAX_PER_ITEM: usize = 2;

pub fn get(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code == code)
}

pub fn name(code: &str) -> String {
    get(code).map_or_else(|| code.to_string(), |l| l.name.to_string())
}

pub fn is_known(code: &str) -> bool {
    get(code).is_some()
}

/// Known codes, without repeats, at most `MAX_PER_ITEM`, in the order given.
pub fn clean_list(codes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in codes.iter().map(|c| c.trim()) {
        if is_known(c) && !out.iter().any(|o| o == c) && out.len() < MAX_PER_ITEM {
            out.push(c.to_string());
        }
    }
    out
}

/// Only English: the English-only speech model serves it best.
pub fn english_only(codes: &[String]) -> bool {
    codes.iter().all(|c| c == ENGLISH)
}

/// The language the person writes Chinese in, steered by a sentence whisper continues in that script.
pub fn whisper_prompt(code: &str) -> Option<&'static str> {
    match code {
        "zh" => Some("以下是普通话的句子，使用简体中文。"),
        "zh-Hant" => Some("以下是普通話的句子，使用繁體中文。"),
        "yue" => Some("以下係廣東話嘅句子。"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------
// Text

/// Written without spaces between words: each character is a word for counting and comparing.
pub fn unspaced(c: char) -> bool {
    matches!(script(c), Some(Script::Han | Script::Kana | Script::Thai))
}

pub fn script(c: char) -> Option<Script> {
    let u = c as u32;
    Some(match u {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
        0x400..=0x52F => Script::Cyrillic,
        0x590..=0x5FF => Script::Hebrew,
        0x600..=0x6FF | 0x750..=0x77F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x900..=0x97F => Script::Devanagari,
        0x980..=0x9FF => Script::Bengali,
        0xB80..=0xBFF => Script::Tamil,
        0xC00..=0xC7F => Script::Telugu,
        0xE00..=0xE7F => Script::Thai,
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Han,
        _ if c.is_alphabetic() => Script::Other,
        _ => return None,
    })
}

/// The share of letters in each script, largest first.
pub fn scripts(text: &str) -> Vec<(Script, f32)> {
    let mut counts: std::collections::BTreeMap<Script, usize> = Default::default();
    let mut total = 0;
    for s in text.chars().filter_map(script) {
        *counts.entry(s).or_default() += 1;
        total += 1;
    }
    let mut out: Vec<(Script, f32)> = counts.into_iter().map(|(s, n)| (s, n as f32 / total.max(1) as f32)).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

/// Whether text is mostly in `s`. Japanese mixes kana with Han, so either counts for it.
pub fn mostly(text: &str, s: Script) -> bool {
    let share: f32 = scripts(text).iter().filter(|(x, _)| *x == s || (s == Script::Kana && *x == Script::Han)).map(|(_, f)| f).sum();
    share >= 0.5
}

/// Splits text into tokens for comparing and counting: words, except that each character of a language
/// written without spaces is its own token. Each token comes with whether a space came before it, so the
/// text can be put back together.
pub fn tokens(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let mut spaced = true;
        let mut start = 0;
        let mut iter = word.char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            if unspaced(c) {
                if start < i {
                    out.push((&word[start..i], spaced));
                    spaced = false;
                }
                let end = iter.peek().map_or(word.len(), |(j, _)| *j);
                out.push((&word[i..end], spaced));
                spaced = false;
                start = end;
            }
        }
        if start < word.len() {
            out.push((&word[start..], spaced));
        }
    }
    out
}

/// Puts tokens back together, with a space where there was one.
pub fn join(tokens: &[(&str, bool)]) -> String {
    let mut out = String::new();
    for (i, (t, spaced)) in tokens.iter().enumerate() {
        if i > 0 && *spaced {
            out.push(' ');
        }
        out.push_str(t);
    }
    out
}

/// Words in text, counting each character of Chinese, Japanese or Thai as one.
pub fn word_count(text: &str) -> usize {
    tokens(text).iter().filter(|(t, _)| t.chars().any(char::is_alphanumeric)).count()
}

/// Puts a space between Chinese or Japanese characters and Latin letters or digits next to them
/// ("用Python写" → "用 Python 写"), as Chinese typesetting does.
pub fn space_mixed(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev: Option<char> = None;
    for c in text.chars() {
        if let Some(p) = prev {
            let latin = |x: char| x.is_ascii_alphanumeric();
            let cjk = |x: char| matches!(script(x), Some(Script::Han | Script::Kana));
            if (cjk(p) && latin(c)) || (latin(p) && cjk(c)) {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// Chinese text uses full-width punctuation; whisper sometimes writes half-width marks between Chinese
/// characters.
pub fn full_width_punctuation(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let cjk = |x: Option<&char>| x.is_some_and(|c| matches!(script(*c), Some(Script::Han | Script::Kana)));
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let before = (0..i).rev().map(|j| &chars[j]).find(|c| !c.is_whitespace());
        let after = chars[i + 1..].iter().find(|c| !c.is_whitespace());
        let wide = match c {
            ',' => Some('，'),
            '.' => Some('。'),
            '?' => Some('？'),
            '!' => Some('！'),
            ':' => Some('：'),
            ';' => Some('；'),
            _ => None,
        };
        // A decimal point or a mark inside English words stays as it is.
        let digits = c == '.' && before.is_some_and(|b| b.is_ascii_digit()) && after.is_some_and(|a| a.is_ascii_digit());
        // A full stop goes after Chinese; other marks between Chinese and anything.
        let chinese = cjk(before) || (c != '.' && cjk(after));
        match wide {
            Some(w) if chinese && !digits => {
                out.push(w);
                // The space after a half-width mark isn't used after a full-width one.
                while chars.get(i + 1).is_some_and(|c| *c == ' ') {
                    i += 1;
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Tidies a line whisper wrote in Chinese or Japanese.
pub fn tidy(text: &str, code: &str) -> String {
    match get(code).map(|l| l.script) {
        Some(Script::Han | Script::Kana) => space_mixed(&full_width_punctuation(text)),
        _ => text.to_string(),
    }
}

/// Keys for telling whether two pieces of text share content: words, roughly stemmed and without common
/// English ones, and pairs of characters for text without spaces, as one Chinese character alone says
/// little.
pub fn content_keys(text: &str, stop: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut BTreeSet<String>| {
        if run.len() == 1 {
            out.insert(run[0].to_string());
        }
        for pair in run.windows(2) {
            out.insert(pair.iter().collect());
        }
        run.clear();
    };
    for piece in text.split(|c: char| !c.is_alphanumeric()) {
        let mut word = String::new();
        for c in piece.chars() {
            if unspaced(c) {
                if !word.is_empty() {
                    add_word(&word, stop, &mut out);
                    word.clear();
                }
                run.push(c);
            } else {
                flush(&mut run, &mut out);
                word.push(c);
            }
        }
        flush(&mut run, &mut out);
        if !word.is_empty() {
            add_word(&word, stop, &mut out);
        }
    }
    out
}

fn add_word(word: &str, stop: &[&str], out: &mut BTreeSet<String>) {
    let w = word.to_lowercase();
    if w.chars().count() > 2 && !stop.contains(&w.as_str()) {
        out.insert(w.chars().take(5).collect());
    }
}

/// Said only to keep a conversation going, in each language, for dropping from what an analysis reads.
pub fn fillers(code: &str) -> &'static [&'static str] {
    match code.split('-').next().unwrap_or(code) {
        "en" => &["yeah", "yes", "yep", "ok", "okay", "mm", "mhm", "mm-hm", "uh-huh", "right", "sure", "um", "uh", "hmm", "so", "cool", "great", "thanks", "thank you"],
        "cy" => &["ie", "iawn", "ok", "okay", "ia", "na", "wel", "diolch", "mm", "ym", "yym", "reit", "grêt", "do"],
        "es" => &["sí", "si", "vale", "ok", "okay", "claro", "bueno", "eh", "mm", "ajá", "gracias", "pues", "vale vale"],
        "fr" => &["oui", "ouais", "ok", "okay", "d'accord", "euh", "bon", "bah", "mm", "merci", "voilà", "ben"],
        "de" => &["ja", "jo", "ok", "okay", "genau", "äh", "ähm", "mhm", "mm", "danke", "gut", "also"],
        "it" => &["sì", "si", "ok", "okay", "va bene", "ehm", "mm", "grazie", "allora", "certo"],
        "pt" => &["sim", "ok", "okay", "tá", "certo", "é", "hum", "obrigado", "obrigada", "então"],
        "nl" => &["ja", "ok", "okay", "oké", "uh", "eh", "mm", "dank je", "goed", "nou"],
        "zh" | "yue" => &["嗯", "啊", "哦", "好", "好的", "对", "對", "是", "是的", "那个", "那個", "就是", "谢谢", "謝謝", "ok", "okay", "嗯嗯"],
        "ja" => &["はい", "うん", "ええ", "えっと", "あの", "そう", "そうですね", "なるほど", "ありがとう"],
        "ko" => &["네", "예", "응", "음", "아", "그래", "맞아", "감사합니다"],
        "ar" => &["نعم", "أيوه", "اه", "طيب", "تمام", "ok", "يعني", "شكرا", "ممم"],
        "hi" => &["हाँ", "हां", "जी", "ठीक", "ठीक है", "अच्छा", "हम्म", "ok", "धन्यवाद"],
        "tr" => &["evet", "tamam", "hı hı", "ok", "peki", "şey", "yani", "teşekkürler"],
        "ru" => &["да", "ага", "угу", "ок", "ладно", "хорошо", "ну", "спасибо", "так"],
        _ => &["ok", "okay", "mm", "mhm", "hmm"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_has_no_repeats_and_english_first_among_equals() {
        let codes: BTreeSet<&str> = LANGUAGES.iter().map(|l| l.code).collect();
        assert_eq!(codes.len(), LANGUAGES.len());
        assert!(get("cy").is_some() && get("ar").unwrap().rtl && !get("en").unwrap().rtl);
        assert_eq!(get("zh-Hant").unwrap().whisper, "zh");
    }

    #[test]
    fn lists_keep_known_languages_once_and_at_most_two() {
        let l = |v: &[&str]| clean_list(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(l(&["cy", "en", "cy", "zz", "fr"]), vec!["cy", "en"]);
        assert!(l(&["xx"]).is_empty());
        assert!(english_only(&["en".into()]) && english_only(&[]) && !english_only(&["en".into(), "zh".into()]));
    }

    #[test]
    fn chinese_is_split_into_characters_and_english_into_words() {
        let t = tokens("这个 deadline 我们要push一下");
        let words: Vec<&str> = t.iter().map(|(w, _)| *w).collect();
        assert_eq!(words, ["这", "个", "deadline", "我", "们", "要", "push", "一", "下"]);
        assert_eq!(join(&t), "这个 deadline 我们要push一下");
        assert_eq!(word_count("这个 deadline 我们要push一下"), 9);
        assert_eq!(word_count("Hello there, friend."), 3);
    }

    #[test]
    fn chinese_and_latin_get_a_space_between_them() {
        assert_eq!(space_mixed("用Python写3个"), "用 Python 写 3 个");
        assert_eq!(space_mixed("Python is fine"), "Python is fine");
    }

    #[test]
    fn punctuation_between_chinese_characters_becomes_full_width() {
        assert_eq!(full_width_punctuation("我们开会, 然后吃饭."), "我们开会，然后吃饭。");
        assert_eq!(full_width_punctuation("版本3.5很好"), "版本3.5很好");
        assert_eq!(full_width_punctuation("Hello, world."), "Hello, world.");
        assert_eq!(tidy("我们用Python, 好吗?", "zh"), "我们用 Python，好吗？");
    }

    #[test]
    fn content_keys_pair_up_chinese_characters() {
        let a = content_keys("下周发布新版本", &[]);
        let b = content_keys("我们下周要发布", &[]);
        assert!(a.contains("发布") && b.contains("发布"));
        assert!(content_keys("Sending the pricing sheet", &["the"]).contains("sendi"));
    }

    #[test]
    fn scripts_are_told_apart() {
        assert!(mostly("مرحبا بكم", Script::Arabic));
        assert!(mostly("我们的截止日期是 Friday", Script::Han));
        assert!(!mostly("Bore da, sut mae?", Script::Han));
        assert!(mostly("こんにちは世界", Script::Kana));
    }
}
