//! On-device speech to text with whisper.cpp. Audio never leaves the machine.
//!
//! English is transcribed with the English-only model, which is the best English at its size. Any other
//! language needs a multilingual model, downloaded when the person first chooses such a language: the
//! large turbo model on PCs, the small one on phones.

use crate::library;
use crate::process::RATE;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use stv_core::lang;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const _: () = assert!(RATE == 16_000, "whisper needs 16 kHz audio");

/// Telling two languages apart needs a few seconds of speech; a shorter clip is taken as the main one.
const DETECT_MIN_S: f32 = 2.5;

/// A speech model the app can download, pinned to an exact file.
#[derive(Debug, Clone, Serialize)]
pub struct SpeechModel {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    /// Empty for the English model, which older versions downloaded unpinned.
    pub sha256: &'static str,
    pub multilingual: bool,
    pub about: &'static str,
}

pub const MODELS: &[SpeechModel] = &[
    SpeechModel {
        id: "base.en",
        name: "English",
        file: "ggml-base.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        size: 147_964_211,
        sha256: "",
        multilingual: false,
        about: "English only. Quick, and the best English at its size.",
    },
    SpeechModel {
        id: "turbo",
        name: "Multilingual (large)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        multilingual: true,
        about: "About 100 languages, and Chinese mixed with English, most accurately. Best with a graphics card.",
    },
    SpeechModel {
        id: "small",
        name: "Multilingual (small)",
        file: "ggml-small-q8_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small-q8_0.bin",
        size: 264_464_607,
        sha256: "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
        multilingual: true,
        about: "About 100 languages, less accurately but several times faster. For phones and PCs without a graphics card.",
    },
];

const ENGLISH: &str = "base.en";

pub fn model(id: &str) -> Option<&'static SpeechModel> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn path_of(m: &SpeechModel) -> PathBuf {
    library::models_dir().join(m.file)
}

pub fn is_ready(m: &SpeechModel) -> bool {
    std::fs::metadata(path_of(m)).is_ok_and(|f| f.len() >= m.size * 9 / 10)
}

pub fn model_path() -> PathBuf {
    path_of(model(ENGLISH).expect("listed"))
}

pub fn model_file() -> &'static str {
    model(ENGLISH).expect("listed").file
}

pub fn model_url() -> &'static str {
    model(ENGLISH).expect("listed").url
}

/// Some speech model is downloaded, so something can be transcribed.
pub fn model_ready() -> bool {
    MODELS.iter().any(is_ready)
}

/// The multilingual model this device suits: the large one on a PC with a graphics card and the memory
/// for it, as on the processor alone it takes about three times as long as the speech.
pub fn suggested_multilingual() -> &'static SpeechModel {
    let phone = cfg!(any(target_os = "android", target_os = "ios"));
    let device = crate::llm::device();
    if phone || device.gpu.is_none() || device.ram_gb < 6.0 { model("small").expect("listed") } else { model("turbo").expect("listed") }
}

/// The model that transcribes `languages`, and whether it's downloaded. English alone uses the English
/// model, or a multilingual one when that's all there is. Anything else uses the multilingual model the
/// person chose, else the best one downloaded, else the one this device suits (not yet downloaded).
pub fn model_for(languages: &[String]) -> (&'static SpeechModel, bool) {
    let english = model(ENGLISH).expect("listed");
    let chosen = model(&crate::prefs::load().speech_model).filter(|m| m.multilingual && is_ready(m));
    let best = chosen.or_else(|| MODELS.iter().find(|m| m.multilingual && is_ready(m)));
    if lang::english_only(languages) {
        return match (is_ready(english), best) {
            (false, Some(m)) => (m, true),
            _ => (english, is_ready(english)),
        };
    }
    match best {
        Some(m) => (m, true),
        None => (suggested_multilingual(), false),
    }
}

/// Runs only when the person asks. Uses a temporary name so a failed download is never loaded, and checks
/// the file against its pinned hash. `progress` gets the fraction done.
pub fn download(id: &str, cancel: &AtomicBool, progress: impl Fn(f32)) -> Result<(), String> {
    let m = model(id).ok_or("That speech model isn't one Voice knows.")?;
    let path = path_of(m);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let partial = path.with_extension("part");
    let result = (|| -> Result<(u64, String), String> {
        let response = agent()?.get(m.url).call().map_err(|e| match proxy() {
            Some(p) => format!("{e} (through the proxy {})", p.shown),
            None => e.to_string(),
        })?;
        let total: u64 = response.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(m.size);
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled.".into());
            }
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            hasher.update(&buffer[..n]);
            done += n as u64;
            progress((done as f32 / total.max(1) as f32).min(1.0));
        }
        file.sync_all().map_err(|e| e.to_string())?;
        Ok((done, hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
    })();
    let outcome = match result {
        Ok((bytes, _)) if bytes < m.size * 9 / 10 => Err("the file was incomplete".to_string()),
        Ok((_, hash)) if !m.sha256.is_empty() && hash != m.sha256 => Err("the file didn't match the one expected".to_string()),
        Ok(_) => std::fs::rename(&partial, &path).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    outcome.map_err(|reason| {
        let _ = std::fs::remove_file(&partial);
        if reason == "Cancelled." { reason } else { format!("The speech model could not be downloaded: {reason}") }
    })
}

/// The English model, as Settings has always offered it.
pub fn download_model(progress: impl Fn(f32)) -> Result<(), String> {
    download(ENGLISH, &AtomicBool::new(false), progress)
}

pub fn delete(id: &str) -> Result<(), String> {
    let m = model(id).ok_or("That speech model isn't one Voice knows.")?;
    crate::listen::forget_transcriber();
    match std::fs::remove_file(path_of(m)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Clips shorter than this are read with a window cut to their length, several times faster.
const SHORT_CLIP_S: f32 = 20.0;
/// Whisper's encoder covers 30 s in 1500 positions.
const AUDIO_CTX_PER_S: f32 = 1500.0 / 30.0;
/// About 7.7 s, the shortest window that transcribed short commands without errors.
const MIN_AUDIO_CTX: f32 = 384.0;
/// Silence read after a short clip: a second normally, and more when whisper looped on the first read.
const QUICK_MARGIN_S: f32 = 1.0;
const RETRY_MARGIN_S: f32 = 8.0;
/// Context around each segment, because whisper does poorly on clipped words.
const PADDING_SAMPLES: usize = RATE as usize / 5;

/// A proxy to download through, and how to show it without its password.
pub struct ProxyChoice {
    pub url: String,
    pub shown: String,
    /// Where it came from: Settings, an environment variable or Windows.
    pub source: &'static str,
}

/// The proxy the person set in Settings, else HTTPS_PROXY and friends, else the Windows proxy setting.
pub fn proxy() -> Option<ProxyChoice> {
    let typed = crate::prefs::load().proxy.trim().to_string();
    if !typed.is_empty() {
        return Some(choice(&typed, "Settings"));
    }
    for var in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Some(v) = std::env::var(var).ok().filter(|v| !v.trim().is_empty()) {
            if !no_proxy_for("huggingface.co") {
                return Some(choice(v.trim(), "environment"));
            }
        }
    }
    windows_proxy().map(|url| choice(&url, "Windows"))
}

fn choice(url: &str, source: &'static str) -> ProxyChoice {
    let url = if url.contains("://") { url.to_string() } else { format!("http://{url}") };
    // "http://user:secret@host:8080" is shown as "http://user:•••@host:8080".
    let shown = match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            let creds = &url[scheme + 3..at];
            let user = creds.split(':').next().unwrap_or("");
            format!("{}{}:•••{}", &url[..scheme + 3], user, &url[at..])
        }
        _ => url.clone(),
    };
    ProxyChoice { url, shown, source }
}

fn no_proxy_for(host: &str) -> bool {
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|s| s.trim().trim_start_matches('.')).any(|s| s == "*" || (!s.is_empty() && (host == s || host.ends_with(&format!(".{s}")))))
}

/// The proxy set in Windows' proxy settings, if it is turned on.
#[cfg(windows)]
fn windows_proxy() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let read = |name: &str| -> Option<String> {
        let out = std::process::Command::new("reg").args(["query", key, "/v", name]).creation_flags(NO_WINDOW).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = text.lines().find(|l| l.trim_start().starts_with(name))?;
        line.split_whitespace().nth(2).map(str::to_string)
    };
    let enabled = read("ProxyEnable").is_some_and(|v| v.trim_start_matches("0x") != "0");
    if !enabled {
        return None;
    }
    parse_windows_proxy(&read("ProxyServer")?)
}

#[cfg(not(windows))]
fn windows_proxy() -> Option<String> {
    None
}

/// Windows keeps either "host:port" for everything or "http=host:port;https=host:port;socks=host:port".
fn parse_windows_proxy(server: &str) -> Option<String> {
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    if !server.contains('=') {
        return Some(server.to_string());
    }
    let parts: Vec<(&str, &str)> = server.split(';').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
    let find = |k: &str| parts.iter().find(|(key, v)| key.eq_ignore_ascii_case(k) && !v.is_empty()).map(|(_, v)| v.to_string());
    find("https").or_else(|| find("http")).or_else(|| find("socks").map(|v| format!("socks5://{v}")))
}

/// Mozilla's roots, built into the app so they work everywhere, plus whatever the system trusts, so a
/// company proxy whose certificate is installed in Windows is trusted too. Android's store isn't readable
/// this way, which is why the built-in roots are always there.
fn tls() -> Result<std::sync::Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    if let Ok(system) = rustls_native_certs::load_native_certs() {
        roots.add_parsable_certificates(system);
    }
    let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(std::sync::Arc::new(config))
}

pub(crate) fn agent() -> Result<ureq::Agent, String> {
    let mut builder = ureq::AgentBuilder::new().tls_config(tls()?);
    if let Some(p) = proxy() {
        let proxy = ureq::Proxy::new(&p.url).map_err(|e| format!("The proxy {} isn't usable: {e}", p.shown))?;
        builder = builder.proxy(proxy);
    }
    Ok(builder.build())
}

//// Whisper labels non-speech as "[BLANK_AUDIO]" or "(wind blowing)", which is not what anyone said.
fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' | '(' | '（' | '【' => depth += 1,
            ']' | ')' | '）' | '】' => depth = (depth as i32 - 1).max(0) as usize,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let tokens = lang::tokens(&out);
    let keep = collapse_repeats(&tokens.iter().map(|(t, _)| *t).collect::<Vec<_>>());
    lang::join(&keep.iter().map(|&i| tokens[i]).collect::<Vec<_>>())
}

/// A run of words said this many times over is whisper stuck in a loop, not speech. A single word needs
/// more, as people do say "no, no, no".
const LOOP_REPEATS: usize = 3;
const LOOP_REPEATS_ONE_WORD: usize = 4;
const LOOP_MAX_WORDS: usize = 8;

fn loop_key(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Keeps one copy of a phrase whisper repeated over and over ("bookmark that bookmark that bookmark…"),
/// including the cut-off copy at the end where it ran out of tokens. Returns the positions of the words
/// kept. A word is a character in Chinese or Japanese.
fn collapse_repeats(words: &[&str]) -> Vec<usize> {
    let keys: Vec<String> = words.iter().map(|w| loop_key(w)).collect();
    let mut out = Vec::new();
    let mut i = 0;
    'words: while i < words.len() {
        for n in 1..=LOOP_MAX_WORDS.min((words.len() - i) / LOOP_REPEATS) {
            let unit = &keys[i..i + n];
            let mut copies = 1;
            while i + (copies + 1) * n <= words.len() && keys[i + copies * n..i + (copies + 1) * n] == *unit {
                copies += 1;
            }
            if copies >= if n == 1 { LOOP_REPEATS_ONE_WORD } else { LOOP_REPEATS } {
                out.extend(i..i + n);
                let mut next = i + copies * n;
                // A last copy cut short.
                let partial = keys[next..].iter().zip(unit).take_while(|(a, b)| a == b).count();
                if partial < n {
                    next += partial;
                }
                i = next;
                continue 'words;
            }
        }
        out.push(i);
        i += 1;
    }
    out
}

pub struct Transcriber {
    ctx: WhisperContext,
    /// The id of the model loaded, from `MODELS`.
    pub model: &'static str,
    multilingual: bool,
}

impl Transcriber {
    pub fn load(m: &'static SpeechModel) -> Result<Self, String> {
        let path = path_of(m);
        let path = path.to_str().ok_or("The model path is not valid text.")?;
        crate::gpu::prepare();
        // On a PC's graphics processor when there is one and the person hasn't turned it off; on the
        // processor if it can't be used. (Not yet tried on the Mac's.)
        let gpu = cfg!(windows) && crate::prefs::load().use_gpu.unwrap_or(true) && crate::llm::device().gpu.is_some();
        let with = |gpu: bool| {
            let mut params = WhisperContextParameters::default();
            params.use_gpu(gpu);
            WhisperContext::new_with_params(path, params)
        };
        let ctx = if gpu { with(true).or_else(|_| with(false)) } else { with(false) }.map_err(|e| e.to_string())?;
        Ok(Self { ctx, model: m.id, multilingual: m.multilingual })
    }

    /// Loads the model for `languages`, or says which to download.
    pub fn for_languages(languages: &[String]) -> Result<Self, String> {
        match model_for(languages) {
            (m, true) => Self::load(m),
            (m, false) => Err(format!("{} needs the {} speech model. Download it in Settings › Languages.", names(languages), m.name)),
        }
    }

    pub fn multilingual(&self) -> bool {
        self.multilingual
    }

    /// Returns the words between two sample positions, or an empty string if there are none, in
    /// `language` (a code from the catalogue). `prompt` steers spelling, as with the person's vocabulary.
    pub fn transcribe(&self, pcm: &[i16], start: usize, end: usize, prompt: Option<&str>, language: &str) -> Result<String, String> {
        let from = start.saturating_sub(PADDING_SAMPLES).min(pcm.len());
        let to = (end + PADDING_SAMPLES).min(pcm.len());
        self.run(&pcm[from..to], prompt, language)
    }

    /// Transcribes all of `pcm`, with `prompt` as the words heard just before, to steer spelling.
    pub fn transcribe_with_prompt(&self, pcm: &[i16], prompt: &str, language: &str) -> Result<String, String> {
        self.run(pcm, Some(prompt), language)
    }

    /// Transcribes all of `pcm` with nothing to steer it.
    pub fn transcribe_plain(&self, pcm: &[i16], language: &str) -> Result<String, String> {
        self.run(pcm, None, language)
    }

    /// Which of `among` (catalogue codes) the speech between two sample positions is in, or None when it
    /// can't tell: one language given, an English-only model, or too short a clip to judge.
    pub fn detect(&self, pcm: &[i16], start: usize, end: usize, among: &[String]) -> Option<String> {
        if among.len() < 2 || !self.multilingual {
            return None;
        }
        let from = start.min(pcm.len());
        let to = end.min(pcm.len());
        if ((to - from) as f32) < DETECT_MIN_S * RATE as f32 {
            return None;
        }
        // Whisper judges from the first 30 s.
        let audio: Vec<f32> = pcm[from..to.min(from + RATE as usize * 30)].iter().map(|&s| s as f32 / 32768.0).collect();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
        let mut state = self.ctx.create_state().ok()?;
        state.pcm_to_mel(&audio, threads).ok()?;
        let (_, probs) = state.lang_detect(0, threads).ok()?;
        among
            .iter()
            .filter_map(|code| {
                let l = lang::get(code)?;
                let id = whisper_rs::get_lang_id(l.whisper)?;
                Some((code.clone(), probs.get(id as usize).copied().unwrap_or(0.0)))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(code, _)| code)
    }

    fn run(&self, pcm: &[i16], prompt: Option<&str>, language: &str) -> Result<String, String> {
        // The English-only model writes only English.
        let language = if self.multilingual && lang::is_known(language) { language } else { lang::ENGLISH };
        let whisper = lang::get(language).map_or("en", |l| l.whisper);
        // A sentence in the right script steers Chinese to Simplified or Traditional characters.
        let steer = lang::whisper_prompt(language);
        let joined = match (steer, prompt) {
            (Some(s), Some(p)) => Some(format!("{s} {p}")),
            (Some(s), None) => Some(s.to_string()),
            (None, p) => p.map(str::to_string),
        };
        let prompt = joined.as_deref();
        let mut audio: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
        // Whisper rejects clips under a second, which a short command can be.
        audio.resize(audio.len().max(RATE as usize * 11 / 10), 0.0);
        let seconds = audio.len() as f32 / RATE as f32;
        let tidy = |text: &str| lang::tidy(&clean(text), language);
        if seconds < SHORT_CLIP_S {
            // Read quickly, whisper sometimes loops on a sentence until it runs out of tokens. A wider window
            // usually stops it at little cost, so only those clips are read again.
            for margin in [QUICK_MARGIN_S, RETRY_MARGIN_S] {
                let (text, capped) = self.decode(&audio, prompt, whisper, Some(margin))?;
                if !capped && !looped(&text) {
                    return Ok(tidy(&text));
                }
            }
        }
        Ok(tidy(&self.decode(&audio, prompt, whisper, None)?.0))
    }

    /// Returns the words, and whether whisper stopped only because it ran out of tokens. With `margin`, the
    /// window is cut to the clip plus that many seconds; without, whisper reads its full 30 s.
    fn decode(&self, audio: &[f32], prompt: Option<&str>, whisper: &str, margin: Option<f32>) -> Result<(String, bool), String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        params.set_n_threads(cores.clamp(1, 8) as i32);
        let seconds = audio.len() as f32 / RATE as f32;
        // Even fast speech is under 6 tokens a second in English; Chinese and other scripts take more
        // tokens a second, so more means whisper is stuck repeating itself.
        let per_s = if whisper == "en" { 8.0 } else { 16.0 };
        let max_tokens = (16.0 + per_s * seconds) as i32;
        if let Some(margin) = margin {
            // Whisper reads 30 s of audio whatever the clip. A short clip is read with a shorter window, which
            // is four times faster; much shorter than 8 s and whisper starts repeating itself or dropping words.
            let window = ((seconds + margin) * AUDIO_CTX_PER_S).ceil().clamp(MIN_AUDIO_CTX, 1500.0);
            params.set_audio_ctx(window as i32);
            params.set_single_segment(true);
            params.set_no_timestamps(true);
            // Retrying at higher temperatures when unsure costs seconds, and short clips rarely need it.
            params.set_temperature_inc(0.0);
            params.set_max_tokens(max_tokens);
        }
        if let Some(prompt) = prompt {
            params.set_initial_prompt(prompt);
        }
        // Always the language given: left to guess, whisper sometimes translates into English instead.
        params.set_language(Some(whisper));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
        state.full(params, audio).map_err(|e| e.to_string())?;
        let mut tokens = 0;
        let text: String = state
            .as_iter()
            .map(|segment| {
                tokens += segment.n_tokens();
                segment.to_str_lossy().map(|t| t.into_owned()).unwrap_or_default()
            })
            .collect();
        Ok((text, margin.is_some() && tokens >= max_tokens))
    }
}

/// "Welsh and English", for messages.
pub fn names(languages: &[String]) -> String {
    languages.iter().map(|c| lang::name(c)).collect::<Vec<_>>().join(" and ")
}

/// A sentence or more said three times running is whisper looping, not speech: people repeat a phrase for
/// emphasis, but rarely a whole sentence three times. Shorter loops are collapsed by `collapse_repeats`.
const SENTENCE_LOOP_MIN_WORDS: usize = 6;
const SENTENCE_LOOP_MAX_WORDS: usize = 40;

fn looped(text: &str) -> bool {
    let keys: Vec<String> = lang::tokens(text).iter().map(|(w, _)| loop_key(w)).filter(|w| !w.is_empty()).collect();
    (SENTENCE_LOOP_MIN_WORDS..=SENTENCE_LOOP_MAX_WORDS).any(|n| {
        (0..keys.len().saturating_sub(3 * n - 1)).any(|i| keys[i..i + n] == keys[i + n..i + 2 * n] && keys[i..i + n] == keys[i + 2 * n..i + 3 * n])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_always_trust_the_built_in_roots() {
        assert!(tls().is_ok());
        assert!(webpki_roots::TLS_SERVER_ROOTS.len() > 100);
    }

    /// Reaches the real server, so it only runs when asked: cargo test -- --ignored reaches_the_model
    #[test]
    #[ignore]
    fn reaches_the_model_server() {
        let response = agent().unwrap().get(model_url()).set("Range", "bytes=0-15").call().expect("connects over TLS");
        assert!(matches!(response.status(), 200 | 206), "{}", response.status());
        // As on Android, where the system's store can't be read: the built-in roots alone are enough.
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let bare = ureq::AgentBuilder::new().tls_config(std::sync::Arc::new(config)).build();
        let response = bare.get(model_url()).set("Range", "bytes=0-15").call().expect("built-in roots are enough");
        assert!(matches!(response.status(), 200 | 206), "{}", response.status());
    }

    #[test]
    fn windows_proxy_settings_are_read() {
        assert_eq!(parse_windows_proxy("proxy.corp:8080").as_deref(), Some("proxy.corp:8080"));
        assert_eq!(parse_windows_proxy("http=a:80;https=b:443").as_deref(), Some("b:443"));
        assert_eq!(parse_windows_proxy("http=a:80;ftp=c:21").as_deref(), Some("a:80"));
        assert_eq!(parse_windows_proxy("socks=s:1080").as_deref(), Some("socks5://s:1080"));
        assert_eq!(parse_windows_proxy(" "), None);
    }

    #[test]
    fn proxy_passwords_are_not_shown() {
        let p = choice("http://me:secret@proxy:8080", "Settings");
        assert_eq!(p.url, "http://me:secret@proxy:8080");
        assert_eq!(p.shown, "http://me:•••@proxy:8080");
        assert_eq!(choice("proxy:3128", "Windows").url, "http://proxy:3128");
    }

    #[test]
    fn non_speech_labels_are_removed() {
        assert_eq!(clean(" [BLANK_AUDIO] "), "");
        assert_eq!(clean("(wind blowing) Hello  there [music]"), "Hello there");
        assert_eq!(clean("stray) words"), "stray words");
    }

    #[test]
    fn repetition_loops_are_collapsed() {
        let looped = format!("{}bookmark", "bookmark that ".repeat(15));
        assert_eq!(clean(&looped), "bookmark that");
        assert_eq!(clean("So, bookmark that. Bookmark that. Bookmark that. Then we ship."), "So, bookmark that. Then we ship.");
        assert_eq!(clean("no no no"), "no no no", "a word said three times is speech");
        assert_eq!(clean("go go go go go"), "go");
        assert_eq!(clean("that that is it"), "that that is it");
    }

    #[test]
    fn chinese_loops_are_collapsed_and_spacing_kept() {
        assert_eq!(clean("我们开会吧我们开会吧我们开会吧我们开会吧"), "我们开会吧");
        assert_eq!(clean("这个 deadline 我们要push一下"), "这个 deadline 我们要push一下");
        assert_eq!(clean("（音乐）你好"), "你好");
        assert!(looped(&"我们今天讨论一下新的版本。".repeat(3)));
    }

    #[test]
    fn multilingual_items_need_a_multilingual_model() {
        let (m, _) = model_for(&["cy".into()]);
        assert!(m.multilingual);
        assert!(MODELS.iter().filter(|m| m.multilingual).all(|m| m.sha256.len() == 64 && m.url.contains(m.file)));
    }

    #[test]
    fn a_sentence_said_three_times_running_is_a_loop() {
        let once = "Testing, testing, one, two, three, testing, testing, you and me.";
        assert!(!looped(&format!("{once} {once}")), "said twice on purpose");
        assert!(looped(&format!("{once} {once} {once} Testing, testing, one")));
        assert!(!looped("go go go go go"), "short repeats are collapse_repeats' job");
        assert!(!looped("We met on Monday. Then we met on Tuesday. Then we met on Wednesday."));
    }

    /// Needs the model and a 16 kHz mono 16-bit WAV: set STV_TEST_WAV and run with --ignored.
    #[test]
    #[ignore]
    fn transcribes_a_real_file() {
        let data = std::fs::read(std::env::var("STV_TEST_WAV").unwrap()).unwrap();
        let at = data.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<i16> = data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let language = std::env::var("STV_TEST_LANGUAGE").unwrap_or_else(|_| "en".into());
        let t = Transcriber::for_languages(&[language.clone()]).unwrap();
        let text = t.transcribe(&pcm, 0, pcm.len(), None, &language).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains(&std::env::var("STV_TEST_EXPECT").unwrap().to_lowercase()));
    }
}
