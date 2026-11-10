//! Natural voices for reading aloud, as an option: Piper voices run by sherpa-onnx, offline, each downloaded
//! once when the person chooses it. Without one, the system's own voice reads. Windows for now.

use crate::{library, prefs, transcribe};
use serde::Serialize;
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Voice {
    /// Its archive's name, and the folder it unpacks to.
    pub id: &'static str,
    pub name: &'static str,
    /// Accent and voice, as shown: "US English, female".
    pub about: &'static str,
    pub size: u64,
    #[serde(skip)]
    pub sha256: &'static str,
}

/// The first is the default. Medium quality: clear, and quick enough to keep up while reading.
pub const VOICES: &[Voice] = &[
    Voice { id: "vits-piper-en_US-lessac-medium", name: "Lessac", about: "US English, female", size: 67_230_653, sha256: "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e" },
    Voice { id: "vits-piper-en_US-ryan-medium", name: "Ryan", about: "US English, male", size: 67_213_100, sha256: "c546af78b6395b4e7c4ce1ed899438b64426a362f5d4ec5fecd090ded9ad7505" },
    Voice { id: "vits-piper-en_US-amy-medium", name: "Amy", about: "US English, female", size: 67_223_746, sha256: "9a5d1fc497f85e8022b785bff5f8105203b1e33099ee6265203efc70b0cb0264" },
    Voice { id: "vits-piper-en_US-joe-medium", name: "Joe", about: "US English, male", size: 67_169_394, sha256: "261b903a1fb2ce621e2b5b14233cd41223c3b9f9590f896c11daee4df22d9bd3" },
    Voice { id: "vits-piper-en_GB-alan-medium", name: "Alan", about: "British English, male", size: 67_220_121, sha256: "a48d4017da0f77668b27bed63fe6e04dd64c6397e1fadad4f460efb0ef7c9012" },
    Voice { id: "vits-piper-en_GB-jenny_dioco-medium", name: "Jenny", about: "British English, female", size: 67_225_842, sha256: "a0888024569bafbefc05a4b48ddf8419d8dbbf3205f4af37cf7c6f1a87cc20c5" },
    Voice { id: "vits-piper-en_GB-cori-medium", name: "Cori", about: "British English, female", size: 67_257_412, sha256: "49c9a5361bbdd95d7ca9687c4de11e5908481f65e7c7c368960df79949fdac2b" },
    Voice { id: "vits-piper-en_GB-northern_english_male-medium", name: "Northern English", about: "British English, male", size: 67_210_490, sha256: "2bb2c1e709f58c11f17c693b3b38f500e110e7f54f2651774ec48b8d41f12c55" },
    Voice { id: "vits-piper-en_GB-alba-medium", name: "Alba", about: "Scottish English, female", size: 67_212_349, sha256: "fcd45962906933eec4431d3688f7d74aaac8713c87c6717f91fd3b23463aa1a1" },
];

pub fn supported() -> bool {
    cfg!(windows)
}

fn voice(id: &str) -> Result<&'static Voice, String> {
    VOICES.iter().find(|v| v.id == id).ok_or_else(|| "That voice isn't one Voice offers.".into())
}

/// The voice chosen in Settings, or the first.
pub fn chosen() -> &'static Voice {
    chosen_in(&prefs::load())
}

pub fn chosen_in(p: &prefs::Prefs) -> &'static Voice {
    VOICES.iter().find(|v| v.id == p.tts_voice).unwrap_or(&VOICES[0])
}

fn dir(v: &Voice) -> PathBuf {
    library::models_dir().join("tts").join(v.id)
}

/// The model inside a voice's folder: "en_US-lessac-medium.onnx" for "vits-piper-en_US-lessac-medium".
fn model_file(v: &Voice) -> String {
    format!("{}.onnx", v.id.trim_start_matches("vits-piper-"))
}

pub fn ready(v: &Voice) -> bool {
    let d = dir(v);
    d.join(model_file(v)).is_file() && d.join("tokens.txt").is_file() && d.join("espeak-ng-data").is_dir()
}

/// Any natural voice downloaded.
pub fn any_ready() -> bool {
    VOICES.iter().any(ready)
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
    pub chosen: &'static str,
    /// The voice being downloaded, if one is.
    pub downloading: Option<String>,
}

static DOWNLOADING: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn status() -> Status {
    Status {
        supported: supported(),
        voices: VOICES.iter().map(|v| VoiceStatus { voice: *v, ready: ready(v) }).collect(),
        chosen: chosen().id,
        downloading: DOWNLOADING.lock().unwrap().clone(),
    }
}

pub fn cancel_download() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn delete(id: &str) -> Result<(), String> {
    let v = voice(id)?;
    if chosen().id == v.id {
        unload();
    }
    let _ = std::fs::remove_dir_all(dir(v));
    Ok(())
}

/// Downloads a voice, one at a time.
pub fn download(id: &str, progress: impl Fn(f32)) -> Result<(), String> {
    let v = voice(id)?;
    {
        let mut busy = DOWNLOADING.lock().unwrap();
        if busy.is_some() {
            return Err("Another voice is downloading. Wait for it to finish.".into());
        }
        *busy = Some(v.id.to_string());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let result = imp::download(v, &CANCEL, progress);
    *DOWNLOADING.lock().unwrap() = None;
    result
}

#[cfg(windows)]
mod imp {
    use super::*;
    use sha2::{Digest, Sha256};
    use sherpa_rs::tts::{VitsTts, VitsTtsConfig};
    use std::io::{Read, Write};
    use std::sync::Mutex;

    /// The loaded voice, and which it is.
    static ENGINE: Mutex<Option<(&'static str, VitsTts)>> = Mutex::new(None);

    /// Downloads and unpacks a voice, checking it against its fingerprint first.
    pub fn download(v: &Voice, cancel: &AtomicBool, progress: impl Fn(f32)) -> Result<(), String> {
        let root = library::models_dir().join("tts");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let archive = root.join(format!("{}.tar.bz2.part", v.id));
        let url = format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/{}.tar.bz2", v.id);
        let response = transcribe::agent()?.get(&url).call().map_err(|e| format!("The download didn't start: {e}"))?;
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let (mut have, mut buffer) = (0u64, vec![0u8; 1 << 16]);
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&archive);
                return Err("Download stopped.".into());
            }
            let n = reader.read(&mut buffer).map_err(|e| format!("The download was interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            have += n as u64;
            progress((have as f32 / v.size as f32).min(0.95));
        }
        drop(file);
        let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if hash != v.sha256 {
            let _ = std::fs::remove_file(&archive);
            return Err("The downloaded voice didn't match, so it was deleted. Try again.".into());
        }
        let _ = std::fs::remove_dir_all(dir(v));
        let unpacking = std::fs::File::open(&archive).map_err(|e| e.to_string())?;
        tar::Archive::new(bzip2::read::BzDecoder::new(unpacking)).unpack(&root).map_err(|e| format!("Couldn't unpack the voice: {e}"))?;
        let _ = std::fs::remove_file(&archive);
        progress(1.0);
        if ready(v) { Ok(()) } else { Err("The voice was unpacked but isn't complete.".into()) }
    }

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

    /// Speech for `text` in the chosen voice as a 16-bit mono WAV, at `speed` (1 is normal).
    pub fn speak(text: &str, speed: f32) -> Result<Vec<u8>, String> {
        let v = chosen();
        if !ready(v) {
            return Err(format!("The {} voice isn't downloaded. Choose one in Settings › Reading aloud.", v.name));
        }
        if !engine_present() {
            return Err("The natural voice's files are missing from the app. Reinstall Voice to use it.".into());
        }
        let mut slot = ENGINE.lock().unwrap();
        if slot.as_ref().is_none_or(|(id, _)| *id != v.id) {
            *slot = None;
            let d = dir(v);
            let path = |p: &str| d.join(p).to_string_lossy().into_owned();
            let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4)) as i32;
            let config = VitsTtsConfig {
                model: path(&model_file(v)),
                tokens: path("tokens.txt"),
                data_dir: path("espeak-ng-data"),
                length_scale: 1.0,
                noise_scale: 0.667,
                noise_scale_w: 0.8,
                onnx_config: sherpa_rs::OnnxConfig { num_threads: threads, ..Default::default() },
                ..Default::default()
            };
            *slot = Some((v.id, VitsTts::new(config)));
        }
        let audio = slot.as_mut().expect("loaded").1.create(text, 0, speed.clamp(0.5, 2.0)).map_err(|e| e.to_string())?;
        let pcm: Vec<i16> = audio.samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
        let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
        let bytes = (pcm.len() * 2) as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&audio.sample_rate.to_le_bytes());
        wav.extend_from_slice(&(audio.sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&bytes.to_le_bytes());
        for s in pcm {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        Ok(wav)
    }

    /// Lets go of the voice, before its files are deleted.
    pub fn unload() {
        *ENGINE.lock().unwrap() = None;
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn download(_v: &Voice, _cancel: &AtomicBool, _progress: impl Fn(f32)) -> Result<(), String> {
        Err("Natural voices aren't available on this device yet.".into())
    }

    pub fn speak(_text: &str, _speed: f32) -> Result<Vec<u8>, String> {
        Err("Natural voices aren't available on this device yet.".into())
    }

    pub fn unload() {}
}

pub use imp::{speak, unload};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_are_distinct_and_their_models_are_named_from_their_ids() {
        let mut ids: Vec<_> = VOICES.iter().map(|v| v.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), VOICES.len());
        assert_eq!(model_file(&VOICES[0]), "en_US-lessac-medium.onnx");
        assert!(VOICES.iter().all(|v| v.sha256.len() == 64 && v.size > 10_000_000));
    }
}
