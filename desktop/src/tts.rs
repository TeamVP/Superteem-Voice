//! A natural voice for reading aloud, as an option: a Piper voice run by sherpa-onnx, offline, downloaded
//! once when the person turns it on. Without it, the system's own voice reads. Windows for now.

use crate::{library, transcribe};
use serde::Serialize;
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};

pub const VOICE: &str = "vits-piper-en_US-lessac-medium";
const URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-lessac-medium.tar.bz2";
const SIZE: u64 = 67_230_653;
const SHA256: &str = "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e";

pub fn supported() -> bool {
    cfg!(windows)
}

fn dir() -> PathBuf {
    library::models_dir().join("tts").join(VOICE)
}

pub fn ready() -> bool {
    let d = dir();
    d.join("en_US-lessac-medium.onnx").is_file() && d.join("tokens.txt").is_file() && d.join("espeak-ng-data").is_dir()
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub supported: bool,
    pub ready: bool,
    pub size: u64,
    pub name: &'static str,
}

pub fn status() -> Status {
    Status { supported: supported(), ready: ready(), size: SIZE, name: "Lessac (Piper, US English)" }
}

pub fn delete() -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir());
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use sha2::{Digest, Sha256};
    use sherpa_rs::tts::{VitsTts, VitsTtsConfig};
    use std::io::{Read, Write};
    use std::sync::Mutex;

    static ENGINE: Mutex<Option<VitsTts>> = Mutex::new(None);

    /// Downloads and unpacks the voice, checking it against its fingerprint first.
    pub fn download(cancel: &AtomicBool, progress: impl Fn(f32)) -> Result<(), String> {
        let root = library::models_dir().join("tts");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let archive = root.join(format!("{VOICE}.tar.bz2.part"));
        let response = transcribe::agent()?.get(URL).call().map_err(|e| format!("The download didn't start: {e}"))?;
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let (mut have, mut buffer) = (0u64, vec![0u8; 1 << 16]);
        loop {
            if cancel.load(Ordering::Relaxed) {
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
            progress((have as f32 / SIZE as f32).min(0.95));
        }
        drop(file);
        let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if hash != SHA256 {
            let _ = std::fs::remove_file(&archive);
            return Err("The downloaded voice didn't match, so it was deleted. Try again.".into());
        }
        let _ = std::fs::remove_dir_all(dir());
        let unpacking = std::fs::File::open(&archive).map_err(|e| e.to_string())?;
        tar::Archive::new(bzip2::read::BzDecoder::new(unpacking)).unpack(&root).map_err(|e| format!("Couldn't unpack the voice: {e}"))?;
        let _ = std::fs::remove_file(&archive);
        progress(1.0);
        if ready() { Ok(()) } else { Err("The voice was unpacked but isn't complete.".into()) }
    }

    /// sherpa-onnx is loaded on first use (see build.rs), so it's looked for first: calling into a DLL that
    /// isn't there would end the app.
    #[cfg(windows)]
    fn engine_present() -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
        }
        let name: Vec<u16> = "sherpa-onnx-c-api.dll".encode_utf16().chain(std::iter::once(0)).collect();
        !unsafe { LoadLibraryW(name.as_ptr()) }.is_null()
    }

    #[cfg(not(windows))]
    fn engine_present() -> bool {
        true
    }

    /// Speech for `text` as a 16-bit mono WAV, at `speed` (1 is normal).
    pub fn speak(text: &str, speed: f32) -> Result<Vec<u8>, String> {
        if !ready() {
            return Err("The natural voice isn't downloaded.".into());
        }
        if !engine_present() {
            return Err("The natural voice's files are missing from the app. Reinstall Voice to use it.".into());
        }
        let mut slot = ENGINE.lock().unwrap();
        if slot.is_none() {
            let d = dir();
            let path = |p: &str| d.join(p).to_string_lossy().into_owned();
            let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4)) as i32;
            let config = VitsTtsConfig {
                model: path("en_US-lessac-medium.onnx"),
                tokens: path("tokens.txt"),
                data_dir: path("espeak-ng-data"),
                length_scale: 1.0,
                noise_scale: 0.667,
                noise_scale_w: 0.8,
                onnx_config: sherpa_rs::OnnxConfig { num_threads: threads, ..Default::default() },
                ..Default::default()
            };
            *slot = Some(VitsTts::new(config));
        }
        let audio = slot.as_mut().expect("loaded").create(text, 0, speed.clamp(0.5, 2.0)).map_err(|e| e.to_string())?;
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

    pub fn download(_cancel: &AtomicBool, _progress: impl Fn(f32)) -> Result<(), String> {
        Err("The natural voice isn't available on this device yet.".into())
    }

    pub fn speak(_text: &str, _speed: f32) -> Result<Vec<u8>, String> {
        Err("The natural voice isn't available on this device yet.".into())
    }

    pub fn unload() {}
}

pub use imp::*;
