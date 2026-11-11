//! Starts, watches and stops capture. Windows, the Mac and the iPhone record in this process; Android
//! records in the Kotlin foreground service, which writes the same journals into the session folder.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    pub name: String,
    pub state: String,
    pub detail: String,
    pub frames: u64,
    pub lost: u64,
    /// Input level from 0 to 1 for the meter.
    pub level: f32,
    /// The last change of microphone to tell the person about ("Switched to the phone microphone"), numbered
    /// so each is told once. Android only.
    #[serde(default)]
    pub notice: String,
    #[serde(default)]
    pub notice_id: u32,
    /// Frames recorded by the chosen input itself, where the stream may have fallen back to another.
    #[serde(default)]
    pub own_frames: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub elapsed_s: u64,
    pub streams: Vec<Stream>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Inputs {
    pub devices: Vec<String>,
    pub default: Option<String>,
}

/// The input a stream came from: Windows numbers inputs in the order chosen, Android names its two.
pub fn device_of(label: &str, devices: &[String]) -> Option<String> {
    if label.starts_with("pin") {
        return Some(crate::prefs::PINS.into());
    }
    if let Some(n) = label.strip_prefix("input").and_then(|n| n.parse::<usize>().ok()) {
        return devices.get(n.checked_sub(1)?).cloned();
    }
    match label {
        "phone" => Some("Phone microphone".into()),
        "headset" => Some("Bluetooth headset".into()),
        _ => None,
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;
    use crate::capture::{self, Shared};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};

    pub struct Recorder {
        shared: Arc<Shared>,
        workers: Vec<JoinHandle<()>>,
    }

    pub fn inputs() -> Result<Inputs, String> {
        // The microphone checked while making the voice print is the default, while it is connected.
        let devices = capture::input_devices();
        let chosen = crate::prefs::load().enrol_input.filter(|d| devices.contains(d));
        Ok(Inputs { default: chosen.or_else(capture::default_input), devices })
    }

    /// Nothing is asked up front. Windows uses the microphone as it is; the Mac and the iPhone ask the
    /// first time a microphone starts, with the reason in the app's Info.plist.
    pub fn permissions() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "permissions": [] }))
    }

    pub fn request_permissions(_ids: &[String]) -> Result<(), String> {
        Ok(())
    }

    /// The Mac's microphone privacy settings, or the app's page in the iPhone's Settings.
    /// Computers don't vibrate; the page plays a chirp instead.
    pub fn vibrate() -> Result<(), String> {
        Ok(())
    }

    /// The page reads aloud itself where the web view has a voice.
    pub fn speak(_text: &str, _rate: f32, _pitch: f32, _language: &str) -> Result<(), String> {
        Err("Read aloud in the page.".into())
    }

    pub fn stop_speaking() -> Result<(), String> {
        Ok(())
    }

    pub fn open_app_settings() -> Result<(), String> {
        if cfg!(target_os = "macos") {
            crate::shell::open("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        } else if cfg!(target_os = "ios") {
            open_url("app-settings:")
        } else {
            Err("There are no app permissions to change on Windows.".into())
        }
    }

    /// Opens a web page in the default browser.
    #[cfg(not(target_os = "ios"))]
    pub fn open_url(url: &str) -> Result<(), String> {
        crate::shell::open(url)
    }

    #[cfg(target_os = "ios")]
    static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

    /// The app, for opening links through the system.
    #[cfg(target_os = "ios")]
    pub fn set_app(app: tauri::AppHandle) {
        let _ = APP.set(app);
    }

    #[cfg(target_os = "ios")]
    pub fn open_url(url: &str) -> Result<(), String> {
        use tauri_plugin_opener::OpenerExt;
        let app = APP.get().ok_or("The app hasn't started yet.")?;
        app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
    }

    /// The iPhone gives an app silence from the microphone until it says it records, and stops it when the
    /// screen locks unless it also plays. So the app's audio session plays and records, through the speaker
    /// rather than the earpiece, from a Bluetooth headset when one is connected.
    #[cfg(target_os = "ios")]
    pub fn start_audio_session() -> Result<(), String> {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject, Bool};
        use std::ptr::null_mut;

        #[link(name = "AVFAudio", kind = "framework")]
        extern "C" {}

        const ALLOW_BLUETOOTH: usize = 0x4;
        const DEFAULT_TO_SPEAKER: usize = 0x8;
        let session_class = AnyClass::get(c"AVAudioSession").ok_or("AVAudioSession is missing.")?;
        let string_class = AnyClass::get(c"NSString").ok_or("NSString is missing.")?;
        unsafe {
            let session: *mut AnyObject = msg_send![session_class, sharedInstance];
            // AVAudioSessionCategoryPlayAndRecord is this string.
            let category: *mut AnyObject = msg_send![string_class, stringWithUTF8String: c"AVAudioSessionCategoryPlayAndRecord".as_ptr()];
            let no_error = null_mut::<*mut AnyObject>();
            let set: Bool = msg_send![session, setCategory: category, withOptions: ALLOW_BLUETOOTH | DEFAULT_TO_SPEAKER, error: no_error];
            if !set.as_bool() {
                return Err("The audio session couldn't be set to record.".into());
            }
            let active: Bool = msg_send![session, setActive: Bool::YES, error: no_error];
            if !active.as_bool() {
                return Err("The audio session couldn't start.".into());
            }
        }
        Ok(())
    }

    pub fn start(dir: PathBuf, devices: &[String], pins: bool) -> Result<Recorder, String> {
        let shared = Shared::new(dir);
        let mut workers = Vec::new();
        for (i, device) in devices.iter().enumerate() {
            let (shared, device, label) = (shared.clone(), device.clone(), format!("input{}", i + 1));
            workers.push(thread::spawn(move || capture::run_input(shared, device, label)));
        }
        if pins {
            let shared = shared.clone();
            workers.push(thread::spawn(move || capture::run_pins(shared)));
        }
        Ok(Recorder { shared, workers })
    }

    impl Recorder {
        pub fn dir(&self) -> &Path {
            &self.shared.dir
        }

        pub fn status(&self) -> Result<Status, String> {
            let streams = self
                .shared
                .stats
                .lock()
                .unwrap()
                .iter()
                .map(|(name, s)| Stream {
                    name: name.clone(),
                    state: s.state.clone(),
                    detail: s.detail.clone(),
                    frames: s.frames,
                    lost: s.lost,
                    level: ((s.level_db + 70.0) / 70.0).clamp(0.0, 1.0),
                    notice: String::new(),
                    notice_id: 0,
                    own_frames: None,
                })
                .collect();
            Ok(Status { elapsed_s: self.shared.started.elapsed().as_secs(), streams })
        }

        /// Returns once every journal is flushed and closed.
        pub fn stop(self) -> Result<(), String> {
            self.shared.stop.store(true, Ordering::Relaxed);
            for worker in self.workers {
                let _ = worker.join();
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use std::sync::OnceLock;
    use tauri::plugin::PluginHandle;
    use tauri::Wry;

    static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();

    pub fn set_plugin(handle: PluginHandle<Wry>) {
        let _ = PLUGIN.set(handle);
    }

    fn call<T: for<'de> Deserialize<'de>>(command: &str, payload: impl Serialize) -> Result<T, String> {
        PLUGIN.get().ok_or("Capture is not ready.")?.run_mobile_plugin(command, payload).map_err(|e| e.to_string())
    }

    #[derive(Serialize)]
    struct Empty {}

    #[derive(Serialize)]
    struct StartArgs<'a> {
        dir: &'a str,
        inputs: &'a [String],
        pins: bool,
    }

    #[derive(Deserialize)]
    struct Done {}

    pub struct Recorder {
        dir: PathBuf,
    }

    pub fn inputs() -> Result<Inputs, String> {
        call("inputs", Empty {})
    }

    #[derive(Serialize)]
    struct Ids<'a> {
        ids: &'a [String],
    }

    /// Each kind of access the app asks Android for, and whether it is allowed.
    pub fn permissions() -> Result<serde_json::Value, String> {
        call("permissions", Empty {})
    }

    pub fn request_permissions(ids: &[String]) -> Result<(), String> {
        call::<Done>("askPermissions", Ids { ids }).map(|_| ())
    }

    pub fn open_app_settings() -> Result<(), String> {
        call::<Done>("openAppSettings", Empty {}).map(|_| ())
    }

    /// A short buzz, to say dictation stopped by itself.
    pub fn vibrate() -> Result<(), String> {
        call::<Done>("vibrate", Empty {}).map(|_| ())
    }

    /// Reads text aloud with the phone's voice, returning once it's said or stopped.
    /// `language` is a BCP 47 tag such as "cy" or "zh-CN"; empty for the phone's own.
    pub fn speak(text: &str, rate: f32, pitch: f32, language: &str) -> Result<(), String> {
        call::<Done>("speak", Speak { text, rate, pitch, language }).map(|_| ())
    }

    #[derive(Serialize)]
    struct Speak<'a> {
        text: &'a str,
        rate: f32,
        pitch: f32,
        language: &'a str,
    }

    pub fn stop_speaking() -> Result<(), String> {
        call::<Done>("stopSpeaking", Empty {}).map(|_| ())
    }

    #[derive(Serialize)]
    struct Url<'a> {
        url: &'a str,
    }

    /// Opens a web page in the browser.
    pub fn open_url(url: &str) -> Result<(), String> {
        call::<Done>("openUrl", Url { url }).map(|_| ())
    }

    #[derive(Serialize)]
    struct ProbeArgs<'a> {
        inputs: &'a [String],
    }

    #[derive(Deserialize)]
    pub struct ProbeLevel {
        pub name: String,
        pub state: String,
        pub detail: String,
        pub level: f32,
    }

    #[derive(Deserialize)]
    struct ProbeLevels {
        probes: Vec<ProbeLevel>,
    }

    /// Listens to microphones before a meeting, without recording, for their meters.
    pub fn probe_start(inputs: &[String]) -> Result<(), String> {
        call::<Done>("probeStart", ProbeArgs { inputs }).map(|_| ())
    }

    pub fn probe_status() -> Result<Vec<ProbeLevel>, String> {
        call::<ProbeLevels>("probeStatus", Empty {}).map(|p| p.probes)
    }

    pub fn probe_stop() -> Result<(), String> {
        call::<Done>("probeStop", Empty {}).map(|_| ())
    }

    pub fn start(dir: PathBuf, devices: &[String], pins: bool) -> Result<Recorder, String> {
        let path = dir.to_str().ok_or("The session path is not valid text.")?;
        call::<Done>("start", StartArgs { dir: path, inputs: devices, pins })?;
        Ok(Recorder { dir })
    }

    impl Recorder {
        pub fn dir(&self) -> &Path {
            &self.dir
        }

        pub fn status(&self) -> Result<Status, String> {
            call("status", Empty {})
        }

        pub fn stop(self) -> Result<(), String> {
            call::<Done>("stop", Empty {}).map(|_| ())
        }
    }
}

pub use imp::*;
