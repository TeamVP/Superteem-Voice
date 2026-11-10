//! Natural voices for reading aloud, as an option: models from sherpa-onnx's collection
//! (https://k2-fsa.github.io/sherpa/onnx/tts/all/) run offline, each downloaded once when the person
//! chooses it. Without one, the system's own voice reads. Windows for now.
//!
//! A voice is a model and, for a model with many speakers, which one. The app offers some; the person can
//! add others from the collection, naming them as they like. Piper (VITS), Kokoro, Matcha and Kitten models
//! run; which one a folder holds is worked out from its files.

use crate::{library, prefs, transcribe};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Voice {
    /// What Settings stores: the model's name, then "#speaker" when it isn't the first.
    pub id: String,
    pub name: String,
    /// As shown: "British English", "Chinese English".
    pub language: String,
    /// "Female", "Male", or empty when not said.
    #[serde(default)]
    pub gender: String,
    /// What it can read, as language codes ("en", "zh").
    #[serde(default)]
    pub langs: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
    /// The model's folder, which is also its archive's name in the collection.
    pub model: String,
    /// Which of a model's speakers this is.
    #[serde(default)]
    pub speaker: i32,
    /// Added by the person.
    #[serde(default)]
    pub custom: bool,
    /// The download, in bytes; 0 for one added by the person.
    #[serde(default)]
    pub size: u64,
}

/// A model the app offers, pinned to its file.
struct Model {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODELS: &[Model] = &[
    Model { name: "vits-piper-en_GB-alba-medium", size: 67_212_349, sha256: "fcd45962906933eec4431d3688f7d74aaac8713c87c6717f91fd3b23463aa1a1" },
    Model { name: "vits-piper-en_GB-alan-medium", size: 67_220_121, sha256: "a48d4017da0f77668b27bed63fe6e04dd64c6397e1fadad4f460efb0ef7c9012" },
    Model { name: "vits-piper-en_GB-cori-medium", size: 67_257_412, sha256: "49c9a5361bbdd95d7ca9687c4de11e5908481f65e7c7c368960df79949fdac2b" },
    Model { name: "vits-piper-en_GB-jenny_dioco-medium", size: 67_225_842, sha256: "a0888024569bafbefc05a4b48ddf8419d8dbbf3205f4af37cf7c6f1a87cc20c5" },
    Model { name: "vits-piper-en_GB-northern_english_male-medium", size: 67_210_490, sha256: "2bb2c1e709f58c11f17c693b3b38f500e110e7f54f2651774ec48b8d41f12c55" },
    Model { name: "vits-piper-en_US-amy-medium", size: 67_223_746, sha256: "9a5d1fc497f85e8022b785bff5f8105203b1e33099ee6265203efc70b0cb0264" },
    Model { name: "vits-piper-en_US-hfc_male-medium", size: 67_214_049, sha256: "76388f84acfca8ba5c0ed1636a26ada14c598abd52e76f110d4756fe326fc5f2" },
    Model { name: "vits-piper-en_US-joe-medium", size: 67_169_394, sha256: "261b903a1fb2ce621e2b5b14233cd41223c3b9f9590f896c11daee4df22d9bd3" },
    Model { name: "vits-piper-en_US-lessac-medium", size: 67_230_653, sha256: "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e" },
    Model { name: "vits-piper-en_US-norman-medium", size: 67_203_672, sha256: "1f32065d480abe9abc7c7f91442125d0b34c1cc065d1e600466cac408eabf3b8" },
    Model { name: "vits-piper-en_US-ryan-medium", size: 67_213_100, sha256: "c546af78b6395b4e7c4ce1ed899438b64426a362f5d4ec5fecd090ded9ad7505" },
    Model { name: "kokoro-multi-lang-v1_1", size: 364_855_331, sha256: "b78e1ac42b852e839cceaafbe88f66806bf5860992d1d7b9a3f2ad1198427aed" },
    Model { name: "matcha-icefall-zh-en", size: 79_033_838, sha256: "271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef" },
];

/// Matcha models make sound through a separate vocoder, shared between them.
struct Vocoder {
    file: &'static str,
    size: u64,
    /// Empty where the collection doesn't publish one.
    sha256: &'static str,
}

const VOCODER_16K: Vocoder = Vocoder { file: "vocos-16khz-univ.onnx", size: 53_882_848, sha256: "b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e" };
const VOCODER_22K: Vocoder = Vocoder { file: "vocos-22khz-univ.onnx", size: 53_884_024, sha256: "" };

/// The Chinese and English Matcha model was trained with the 16 kHz vocoder; the others with the 22 kHz one.
fn vocoder_for(model: &str) -> &'static Vocoder {
    if model.contains("zh-en") { &VOCODER_16K } else { &VOCODER_22K }
}

#[allow(clippy::too_many_arguments)]
fn voice(id: &str, name: &str, language: &str, gender: &str, langs: &[&str], recommended: bool, model: &str, speaker: i32) -> Voice {
    Voice {
        id: id.into(),
        name: name.into(),
        language: language.into(),
        gender: gender.into(),
        langs: langs.iter().map(|s| s.to_string()).collect(),
        recommended,
        model: model.into(),
        speaker,
        custom: false,
        size: MODELS.iter().find(|m| m.name == model).map_or(0, |m| m.size + if model.starts_with("matcha") { vocoder_for(model).size } else { 0 }),
    }
}

/// The voices the app offers.
fn built_in() -> Vec<Voice> {
    let piper = |name: &str, language: &str, gender: &str, recommended: bool, model: &str| voice(model, name, language, gender, &["en"], recommended, model, 0);
    vec![
        piper("Alba", "Scottish English", "Female", true, "vits-piper-en_GB-alba-medium"),
        piper("Joe", "US English", "Male", true, "vits-piper-en_US-joe-medium"),
        piper("Alan", "British English", "Male", false, "vits-piper-en_GB-alan-medium"),
        piper("Cori", "British English", "Female", false, "vits-piper-en_GB-cori-medium"),
        piper("Jenny", "British English", "Female", false, "vits-piper-en_GB-jenny_dioco-medium"),
        piper("Northern English", "British English", "Male", false, "vits-piper-en_GB-northern_english_male-medium"),
        piper("Amy", "US English", "Female", false, "vits-piper-en_US-amy-medium"),
        piper("Henrik", "US English", "Male", false, "vits-piper-en_US-hfc_male-medium"),
        piper("Lessac", "US English", "Female", false, "vits-piper-en_US-lessac-medium"),
        piper("Norman", "US English", "Male", false, "vits-piper-en_US-norman-medium"),
        piper("Ryan", "US English", "Male", false, "vits-piper-en_US-ryan-medium"),
        // Speaker 58 is zm_009.
        voice("kokoro-multi-lang-v1_1#58", "Yǔxuān (宇轩)", "Chinese English", "Male", &["zh", "en"], false, "kokoro-multi-lang-v1_1", 58),
        voice("matcha-icefall-zh-en", "Chá (茶)", "Chinese English", "Female", &["zh", "en"], false, "matcha-icefall-zh-en", 0),
    ]
}

/// Recommended first, then by language (British English before US English), then female before male,
/// then by name.
fn sort(voices: &mut [Voice]) {
    let gender = |g: &str| match g {
        "Female" => 0,
        "Male" => 1,
        _ => 2,
    };
    voices.sort_by(|a, b| {
        (!a.recommended, a.language.to_lowercase(), gender(&a.gender), a.name.to_lowercase())
            .cmp(&(!b.recommended, b.language.to_lowercase(), gender(&b.gender), b.name.to_lowercase()))
    });
}

fn root() -> PathBuf {
    library::models_dir().join("tts")
}

fn custom_file() -> PathBuf {
    root().join("custom.json")
}

fn custom() -> Vec<Voice> {
    library::read_json(&custom_file()).unwrap_or_default()
}

/// Every voice, in the order they're listed.
pub fn voices() -> Vec<Voice> {
    let mut all = built_in();
    all.extend(custom());
    sort(&mut all);
    all
}

pub fn supported() -> bool {
    cfg!(windows)
}

fn find(id: &str) -> Result<Voice, String> {
    voices().into_iter().find(|v| v.id == id).ok_or_else(|| "That voice isn't in the list.".into())
}

/// The voice chosen in Settings; when none is, the first downloaded, so a voice fetched before there was a
/// choice keeps reading; else the first in the list.
pub fn chosen_in(p: &prefs::Prefs, all: &[Voice]) -> Voice {
    all.iter().find(|v| v.id == p.tts_voice).or_else(|| all.iter().find(|v| ready(v))).unwrap_or(&all[0]).clone()
}

/// The natural voice that reads `language` (a code), or None for the system's voice. A voice set for the
/// language comes first; then the main voice, when it speaks the language; then the first downloaded voice
/// that does. With natural voices off, the system's voice reads everything not given its own voice.
pub fn for_language(p: &prefs::Prefs, language: Option<&str>) -> Option<Voice> {
    let all = voices();
    if let Some(lang) = language {
        match p.voice_for.get(lang).map(String::as_str) {
            Some("system") => return None,
            Some(id) => {
                if let Some(v) = all.iter().find(|v| v.id == id && ready(v)) {
                    return Some(v.clone());
                }
            }
            None => {}
        }
    }
    if !p.natural_voice {
        return None;
    }
    let main = chosen_in(p, &all);
    let speaks = |v: &Voice, lang: &str| v.langs.iter().any(|l| l == lang);
    match language {
        None => ready(&main).then_some(main),
        Some(lang) if speaks(&main, lang) && ready(&main) => Some(main),
        Some(lang) => all.into_iter().find(|v| speaks(v, lang) && ready(v)),
    }
}

pub fn chosen() -> Voice {
    chosen_in(&prefs::load(), &voices())
}

/// What kind of model a folder holds, from its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Vits,
    Kokoro,
    Kitten,
    Matcha,
}

fn files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

fn kind(dir: &Path) -> Option<Kind> {
    let names = files(dir);
    let has = |n: &str| names.iter().any(|f| f == n);
    let folder = dir.file_name()?.to_string_lossy().to_lowercase();
    if !has("tokens.txt") {
        return None;
    }
    if has("voices.bin") {
        return Some(if folder.contains("kitten") { Kind::Kitten } else { Kind::Kokoro });
    }
    if folder.starts_with("matcha") || names.iter().any(|f| f.starts_with("model-steps") && f.ends_with(".onnx")) {
        return Some(Kind::Matcha);
    }
    names.iter().any(|f| f.ends_with(".onnx")).then_some(Kind::Vits)
}

fn model_dir(v: &Voice) -> PathBuf {
    root().join(&v.model)
}

fn vocoder_path(model: &str) -> PathBuf {
    root().join("vocoders").join(vocoder_for(model).file)
}

pub fn ready(v: &Voice) -> bool {
    match kind(&model_dir(v)) {
        Some(Kind::Matcha) => vocoder_path(&v.model).is_file(),
        Some(_) => true,
        None => false,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceStatus {
    #[serde(flatten)]
    pub voice: Voice,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub supported: bool,
    pub voices: Vec<VoiceStatus>,
    /// The natural voice that reads when natural voices are on.
    pub chosen: String,
    /// The model being downloaded, if one is.
    pub downloading: Option<String>,
}

static DOWNLOADING: Mutex<Option<String>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn status() -> Status {
    let all = voices();
    Status {
        supported: supported(),
        chosen: chosen_in(&prefs::load(), &all).id,
        voices: all.into_iter().map(|v| VoiceStatus { ready: ready(&v), voice: v }).collect(),
        downloading: DOWNLOADING.lock().unwrap().clone(),
    }
}

pub fn cancel_download() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Deletes a voice's download, and takes a voice the person added out of the list. A model another voice
/// the person added still uses stays.
pub fn delete(id: &str) -> Result<(), String> {
    let v = find(id)?;
    unload();
    if v.custom {
        let rest: Vec<Voice> = custom().into_iter().filter(|c| c.id != v.id).collect();
        library::write_json(&custom_file(), &rest)?;
    }
    let still_used = custom().iter().any(|o| o.id != v.id && o.model == v.model);
    if !still_used {
        let _ = std::fs::remove_dir_all(model_dir(&v));
    }
    Ok(())
}

/// Runs `f` as the one download at a time, named by `what`.
fn one_download<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    {
        let mut busy = DOWNLOADING.lock().unwrap();
        if busy.is_some() {
            return Err("Another voice is downloading. Wait for it to finish.".into());
        }
        *busy = Some(what.to_string());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let result = f();
    *DOWNLOADING.lock().unwrap() = None;
    result
}

/// Downloads `url` to `to`, reporting the fraction done of `size`, checking `sha256` when one is given.
fn fetch(url: &str, to: &Path, size: u64, sha256: &str, progress: &dyn Fn(f32)) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let response = transcribe::agent()?.get(url).call().map_err(|e| format!("The download didn't start: {e}"))?;
    let size = if size > 0 { size } else { response.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(0) };
    let mut reader = response.into_reader();
    let part = to.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let (mut have, mut buffer) = (0u64, vec![0u8; 1 << 16]);
    loop {
        if CANCEL.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err("Download stopped.".into());
        }
        let n = reader.read(&mut buffer).map_err(|e| format!("The download was interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        have += n as u64;
        if size > 0 {
            progress((have as f32 / size as f32).min(1.0));
        }
    }
    drop(file);
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !sha256.is_empty() && hash != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("The download didn't match, so it was deleted. Try again.".into());
    }
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Unpacks a model archive into the voices folder, returning the folder it made.
fn unpack(archive: &Path) -> Result<String, String> {
    let open = || std::fs::File::open(archive).map_err(|e| e.to_string());
    // Its first entry names its folder.
    let mut listing = tar::Archive::new(bzip2::read::BzDecoder::new(open()?));
    let first = listing.entries().map_err(|e| format!("That isn't a model archive: {e}"))?.next().ok_or("The archive is empty.")?;
    let path = first.map_err(|e| e.to_string())?.path().map_err(|e| e.to_string())?.into_owned();
    let folder = path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).ok_or("The archive is empty.")?;
    if folder.contains(['/', '\\']) || folder.starts_with('.') || folder.is_empty() || folder == "vocoders" {
        return Err("That isn't a model archive.".into());
    }
    let _ = std::fs::remove_dir_all(root().join(&folder));
    // Entry by entry: one that would land outside the voices folder is skipped, not followed.
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(open()?));
    for entry in archive.entries().map_err(|e| format!("Couldn't unpack it: {e}"))? {
        let mut entry = entry.map_err(|e| format!("Couldn't unpack it: {e}"))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let relative = entry.path().map_err(|e| e.to_string())?.into_owned();
        if !relative.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            continue;
        }
        let to = root().join(&relative);
        if kind.is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&to).map_err(|e| format!("Couldn't unpack it: {e}"))?;
    }
    Ok(folder)
}

/// The vocoder a Matcha model needs, fetched when it isn't here.
fn ensure_vocoder(model: &str, progress: &dyn Fn(f32)) -> Result<(), String> {
    let path = vocoder_path(model);
    if path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("has a folder")).map_err(|e| e.to_string())?;
    let v = vocoder_for(model);
    fetch(&format!("{RELEASES}vocoder-models/{}", v.file), &path, v.size, v.sha256, progress)
}

/// Downloads an offered voice's model, one at a time.
pub fn download(id: &str, progress: impl Fn(f32)) -> Result<(), String> {
    let v = find(id)?;
    let model = MODELS.iter().find(|m| m.name == v.model).ok_or("That voice was added by you; add it again to download it.")?;
    one_download(&v.model, || {
        std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
        let total = v.size.max(1) as f32;
        let share = model.size as f32 / total;
        let archive = root().join(format!("{}.tar.bz2", model.name));
        fetch(&format!("{RELEASES}tts-models/{}.tar.bz2", model.name), &archive, model.size, model.sha256, &|f| progress(f * share * 0.97))?;
        let unpacked = unpack(&archive);
        let _ = std::fs::remove_file(&archive);
        unpacked?;
        if kind(&model_dir(&v)) == Some(Kind::Matcha) {
            ensure_vocoder(&v.model, &|f| progress(share + f * (1.0 - share)))?;
        }
        progress(1.0);
        if ready(&v) { Ok(()) } else { Err("The voice was unpacked but isn't complete.".into()) }
    })
}

/// What the person says about a voice they add.
#[derive(Debug, Clone, Deserialize)]
pub struct NewVoice {
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub speaker: i32,
    /// The model's download address from the collection ...
    #[serde(default)]
    pub url: String,
    /// ... or an archive on this computer.
    #[serde(default)]
    pub file: String,
}

/// Language codes a language's name suggests, for voices the person adds.
fn langs_of(language: &str) -> Vec<String> {
    let l = language.to_lowercase();
    let known = [
        ("arabic", "ar"),
        ("english", "en"),
        ("spanish", "es"),
        ("french", "fr"),
        ("hindi", "hi"),
        ("turkish", "tr"),
        ("welsh", "cy"),
        ("chinese", "zh"),
        ("mandarin", "zh"),
    ];
    let mut out: Vec<String> = known.iter().filter(|(word, _)| l.contains(word)).map(|(_, code)| code.to_string()).collect();
    out.sort();
    out.dedup();
    out
}

/// Adds a voice from the collection: downloads or unpacks its model, checks it speaks, and lists it.
pub fn add(new: NewVoice, progress: impl Fn(f32)) -> Result<Voice, String> {
    let name: String = new.name.trim().chars().take(60).collect();
    if name.is_empty() {
        return Err("Give the voice a name.".into());
    }
    if new.speaker < 0 {
        return Err("The speaker is a number from 0.".into());
    }
    let url = new.url.trim();
    let what = if url.is_empty() {
        Path::new(&new.file).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        url.rsplit('/').next().unwrap_or(url).to_string()
    };
    let folder = one_download(&what, || {
        std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
        let folder = if !url.is_empty() {
            if !(url.starts_with(&format!("{RELEASES}tts-models/")) && url.ends_with(".tar.bz2")) {
                return Err("Use the model's download address from its page in the sherpa-onnx collection: it starts https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/ and ends .tar.bz2.".into());
            }
            let archive = root().join("adding.tar.bz2");
            fetch(url, &archive, 0, "", &|f| progress(f * 0.9))?;
            let unpacked = unpack(&archive);
            let _ = std::fs::remove_file(&archive);
            unpacked?
        } else if new.file.to_lowercase().ends_with(".tar.bz2") {
            progress(0.3);
            unpack(Path::new(&new.file))?
        } else {
            return Err("Choose the model's .tar.bz2 file, or paste its download address.".into());
        };
        let dir = root().join(&folder);
        match kind(&dir) {
            Some(Kind::Matcha) => ensure_vocoder(&folder, &|f| progress(0.9 + f * 0.05))?,
            Some(_) => {}
            None => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err("That model isn't one Voice can run: it needs to be a Piper, Kokoro, Matcha or Kitten model.".into());
            }
        }
        Ok(folder)
    })?;
    let voice = Voice {
        id: if new.speaker == 0 { folder.clone() } else { format!("{folder}#{}", new.speaker) },
        name,
        language: new.language.trim().chars().take(40).collect(),
        gender: new.gender,
        langs: langs_of(&new.language),
        recommended: false,
        model: folder,
        speaker: new.speaker,
        custom: true,
        size: 0,
    };
    if built_in().iter().any(|b| b.id == voice.id) {
        return Err(format!("{} is already in the list.", voice.model));
    }
    // It has to speak before it's listed.
    if let Err(e) = imp::render(&voice, "Hello.", 1.0) {
        unload();
        if !custom().iter().any(|o| o.model == voice.model) {
            let _ = std::fs::remove_dir_all(model_dir(&voice));
        }
        return Err(format!("The model was downloaded but didn't speak: {e}"));
    }
    let mut list: Vec<Voice> = custom().into_iter().filter(|c| c.id != voice.id).collect();
    list.push(voice.clone());
    library::write_json(&custom_file(), &list)?;
    progress(1.0);
    Ok(voice)
}

/// Speech for `text` in the chosen voice at `speed` (1 is normal), as a 16-bit mono WAV.
pub fn speak(text: &str, speed: f32) -> Result<Vec<u8>, String> {
    speak_as(&chosen(), text, speed)
}

/// Speech for `text` in voice `v`.
pub fn speak_as(v: &Voice, text: &str, speed: f32) -> Result<Vec<u8>, String> {
    if !ready(v) {
        return Err(format!("The {} voice isn't downloaded. Choose one in Settings › Reading aloud.", v.name));
    }
    let (samples, rate) = imp::render(v, text, speed)?;
    Ok(wav(&samples, rate))
}

pub fn find_voice(id: &str) -> Result<Voice, String> {
    find(id)
}

fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let pcm: Vec<i16> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
    let bytes = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub use imp::unload;

#[cfg(windows)]
mod imp {
    use super::*;
    use sherpa_onnx::{
        GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKittenModelConfig, OfflineTtsKokoroModelConfig, OfflineTtsMatchaModelConfig,
        OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
    };

    /// The loaded model, by folder.
    static ENGINE: Mutex<Option<(String, OfflineTts)>> = Mutex::new(None);

    /// sherpa-onnx is loaded on first use (see build.rs), so it's looked for first: calling into a DLL that
    /// isn't there would end the app.
    fn engine_present() -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
        }
        let name: Vec<u16> = "sherpa-onnx-c-api.dll".encode_utf16().chain(std::iter::once(0)).collect();
        !unsafe { LoadLibraryW(name.as_ptr()) }.is_null()
    }

    /// The settings for voice `v`'s model, from the files in its folder.
    fn config(v: &Voice) -> Result<OfflineTtsConfig, String> {
        let dir = model_dir(v);
        let names = files(&dir);
        let path = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
        let some = |f: &str| if names.iter().any(|n| n == f) { path(f) } else { None };
        let first = |test: &dyn Fn(&str) -> bool| names.iter().find(|n| test(n)).and_then(|n| path(n));
        let join = |list: Vec<&String>| (!list.is_empty()).then(|| list.iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect::<Vec<_>>().join(","));
        let onnx = |n: &str| n.ends_with(".onnx");
        // Number, date and phone rules for reading Chinese, phone first as the collection lists them.
        let mut fsts: Vec<&String> = names.iter().filter(|n| n.ends_with(".fst")).collect();
        fsts.sort_by_key(|n| (!n.starts_with("phone"), n.to_string()));
        let fars = join(names.iter().filter(|n| n.ends_with(".far")).collect());
        // A model with US and British lexicons reads with the one its voice speaks.
        let lexicons = {
            let language = v.language.to_lowercase();
            let british = language.contains("british") || language.contains("scottish");
            let has = |s: &str| names.iter().any(|n| n.starts_with("lexicon") && n.contains(s));
            // With both English lexicons, the other accent's is left out.
            let other = match (british, has("us-en") && has("gb-en")) {
                (_, false) => None,
                (true, true) => Some("us-en"),
                (false, true) => Some("gb-en"),
            };
            join(
                names
                    .iter()
                    .filter(|n| n.starts_with("lexicon") && n.ends_with(".txt"))
                    .filter(|n| other.is_none_or(|o| !n.contains(o)))
                    .collect(),
            )
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4)) as i32;
        let mut model = OfflineTtsModelConfig { num_threads: threads, ..Default::default() };
        match kind(&dir).ok_or("The voice's files are incomplete.")? {
            Kind::Vits => {
                model.vits = OfflineTtsVitsModelConfig {
                    model: first(&|n| onnx(n)),
                    tokens: some("tokens.txt"),
                    lexicon: some("lexicon.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    ..Default::default()
                }
            }
            Kind::Kokoro => {
                model.kokoro = OfflineTtsKokoroModelConfig {
                    model: first(&|n| onnx(n)),
                    voices: some("voices.bin"),
                    tokens: some("tokens.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    lexicon: lexicons,
                    ..Default::default()
                }
            }
            Kind::Kitten => {
                model.kitten = OfflineTtsKittenModelConfig {
                    model: first(&|n| onnx(n)),
                    voices: some("voices.bin"),
                    tokens: some("tokens.txt"),
                    data_dir: some("espeak-ng-data"),
                    ..Default::default()
                }
            }
            Kind::Matcha => {
                model.matcha = OfflineTtsMatchaModelConfig {
                    acoustic_model: first(&|n| onnx(n) && n.starts_with("model")).or_else(|| first(&|n| onnx(n))),
                    vocoder: Some(vocoder_path(&v.model).to_string_lossy().into_owned()),
                    tokens: some("tokens.txt"),
                    lexicon: some("lexicon.txt"),
                    data_dir: some("espeak-ng-data"),
                    dict_dir: some("dict"),
                    ..Default::default()
                }
            }
        }
        Ok(OfflineTtsConfig { model, rule_fsts: join(fsts), rule_fars: fars, max_num_sentences: 1, ..Default::default() })
    }

    /// Speech for `text` in voice `v`: samples and their rate.
    pub fn render(v: &Voice, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        if !engine_present() {
            return Err("The natural voice's files are missing from the app. Reinstall Voice to use it.".into());
        }
        let mut slot = ENGINE.lock().unwrap();
        if slot.as_ref().is_none_or(|(model, _)| *model != v.model) {
            *slot = None;
            let tts = OfflineTts::create(&config(v)?).ok_or("The voice's model couldn't be loaded.")?;
            *slot = Some((v.model.clone(), tts));
        }
        let tts = &slot.as_ref().expect("loaded").1;
        let speakers = tts.num_speakers();
        if speakers > 0 && v.speaker >= speakers {
            return Err(format!("This model has speakers 0 to {}; {} isn't one.", speakers - 1, v.speaker));
        }
        let generation = GenerationConfig { speed: speed.clamp(0.5, 2.0), sid: v.speaker, ..Default::default() };
        let audio = tts.generate_with_config(text, &generation, None::<fn(&[f32], f32) -> bool>).ok_or("The voice couldn't say that.")?;
        Ok((audio.samples().to_vec(), audio.sample_rate().max(1) as u32))
    }

    /// Lets go of the voice, before its files are deleted.
    pub fn unload() {
        *ENGINE.lock().unwrap() = None;
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn render(_v: &Voice, _text: &str, _speed: f32) -> Result<(Vec<f32>, u32), String> {
        Err("Natural voices aren't available on this device yet.".into())
    }

    pub fn unload() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_are_listed_recommended_then_by_language_gender_and_name() {
        let mut all = built_in();
        sort(&mut all);
        let names: Vec<&str> = all.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            ["Alba", "Joe", "Cori", "Jenny", "Alan", "Northern English", "Chá (茶)", "Yǔxuān (宇轩)", "Amy", "Lessac", "Henrik", "Norman", "Ryan"]
        );
        let mut ids: Vec<_> = all.iter().map(|v| &v.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }

    #[test]
    fn every_offered_voice_has_a_pinned_model() {
        for v in built_in() {
            let m = MODELS.iter().find(|m| m.name == v.model).unwrap_or_else(|| panic!("{}", v.model));
            assert_eq!(m.sha256.len(), 64);
            assert!(v.size >= m.size);
        }
    }

    #[test]
    fn languages_are_read_from_their_names() {
        assert_eq!(langs_of("Chinese English"), ["en", "zh"]);
        assert_eq!(langs_of("Cymraeg / Welsh"), ["cy"]);
        assert!(langs_of("Klingon").is_empty());
    }
}
