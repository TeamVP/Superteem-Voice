//! Starts, watches and stops capture. Windows records in this process; Android records in the
//! Kotlin foreground service, which writes the same journals into the session folder.

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

    /// Windows asks for nothing up front: the microphone is used as it is.
    pub fn permissions() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "permissions": [] }))
    }

    pub fn request_permissions(_ids: &[String]) -> Result<(), String> {
        Ok(())
    }

    pub fn open_app_settings() -> Result<(), String> {
        Err("There are no app permissions to change on Windows.".into())
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
