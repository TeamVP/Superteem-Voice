//! Sound other apps play, recorded beside the microphones: a call in Teams or Zoom, a video in the browser.
//! Each is named like a device, so it is chosen, remembered and recorded as a track the same way.
//!
//! - "Sound from Microsoft Teams (ms-teams.exe)": one app and every process it started, through Windows'
//!   per-app loopback (Windows 10 2004 and later). The app is found again by its program name each time,
//!   so it can be closed and reopened during a meeting.
//! - "Everything playing on Speakers (Realtek Audio)": all sound sent to an output device.
//!
//! Apps appear in the list as soon as they open a sound session, which the meeting page asks for
//! several times a second. Windows only; elsewhere the list is empty.

use serde::Serialize;

pub const APP_PREFIX: &str = "Sound from ";
pub const OUTPUT_PREFIX: &str = "Everything playing on ";

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    /// The name it is recorded and remembered by.
    pub name: String,
    /// Shown to the person: "Microsoft Teams", or the output device's name.
    pub label: String,
    /// "app" or "output".
    pub kind: String,
    /// Sound is playing from it right now.
    pub active: bool,
    /// Peak level from 0 to 1, for the meter.
    pub level: f32,
}

/// The name is an app's or an output's sound rather than a microphone.
pub fn is_source(name: &str) -> bool {
    name.starts_with(APP_PREFIX) || name.starts_with(OUTPUT_PREFIX)
}

/// The program an app source records: "ms-teams.exe" from "Sound from Microsoft Teams (ms-teams.exe)".
pub fn app_program(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(APP_PREFIX)?;
    let open = rest.rfind(" (")?;
    rest[open + 2..].strip_suffix(')').filter(|p| !p.is_empty())
}

/// The output device an output source records.
pub fn output_device(name: &str) -> Option<&str> {
    name.strip_prefix(OUTPUT_PREFIX).filter(|d| !d.is_empty())
}

pub fn app_name(label: &str, program: &str) -> String {
    format!("{APP_PREFIX}{label} ({program})")
}

/// A readable name for a program: well-known call and browser apps by name, others from the file name.
pub fn friendly(program: &str) -> String {
    let stem = program.rsplit_once('.').map_or(program, |(s, _)| s);
    let known = match stem.to_ascii_lowercase().as_str() {
        "ms-teams" | "teams" | "msteams" => "Microsoft Teams",
        "zoom" | "zoom.us" => "Zoom",
        "chrome" => "Google Chrome",
        "msedge" => "Microsoft Edge",
        "firefox" => "Firefox",
        "slack" => "Slack",
        "discord" => "Discord",
        "webex" | "ciscocollabhost" | "webexmta" => "Webex",
        "skype" => "Skype",
        "whatsapp" | "whatsapp.root" => "WhatsApp",
        "spotify" => "Spotify",
        "vlc" => "VLC",
        "brave" => "Brave",
        "opera" => "Opera",
        "signal" => "Signal",
        "telegram" => "Telegram",
        "gotomeeting" | "g2mcomm" => "GoTo Meeting",
        "ringcentral" => "RingCentral",
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    let mut chars = stem.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

#[cfg(windows)]
pub use win::{list, open_app};

#[cfg(not(windows))]
pub fn list() -> Vec<Source> {
    Vec::new()
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::collections::HashMap;
    use std::mem::{size_of, ManuallyDrop};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use windows::core::{Interface, Ref, Result as WinResult, PWSTR};
    use windows_core::implement;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, BLOB, CLSCTX_ALL, COINIT_MULTITHREADED};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;

    /// Programs that host other apps' processes: an app's sound is credited to the app above them.
    const HOSTS: [&str; 1] = ["msedgewebview2.exe"];

    fn com() {
        // Already initialised on this thread (in either model) is fine.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    struct Process {
        program: String,
        parent: u32,
    }

    fn processes() -> HashMap<u32, Process> {
        let mut out = HashMap::new();
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let program = String::from_utf16_lossy(&entry.szExeFile[..len]);
                out.insert(entry.th32ProcessID, Process { program, parent: entry.th32ParentProcessID });
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
        }
        out
    }

    /// The app a process belongs to: climbs while the parent is the same program (browsers' and Electron
    /// apps' helpers) or the process is a host such as WebView2 (new Teams plays its calls through one).
    fn root(pid: u32, all: &HashMap<u32, Process>) -> u32 {
        let mut current = pid;
        for _ in 0..16 {
            let Some(me) = all.get(&current) else { break };
            let Some(parent) = all.get(&me.parent).filter(|_| me.parent != current && me.parent != 0) else { break };
            let hosted = HOSTS.iter().any(|h| me.program.eq_ignore_ascii_case(h));
            if !(hosted || parent.program.eq_ignore_ascii_case(&me.program)) {
                break;
            }
            current = me.parent;
        }
        current
    }

    struct Session {
        pid: u32,
        active: bool,
        level: f32,
    }

    fn sessions(enumerator: &IMMDeviceEnumerator) -> WinResult<Vec<Session>> {
        let mut out = Vec::new();
        unsafe {
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let Ok(device) = devices.Item(i) else { continue };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let Ok(list) = manager.GetSessionEnumerator() else { continue };
                for j in 0..list.GetCount().unwrap_or(0) {
                    let Ok(control) = list.GetSession(j) else { continue };
                    let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                    // S_OK (0) means it is Windows' own system sounds.
                    if control2.IsSystemSoundsSession().0 == 0 {
                        continue;
                    }
                    let state = control.GetState().unwrap_or(AudioSessionStateExpired);
                    if state == AudioSessionStateExpired {
                        continue;
                    }
                    let Ok(pid) = control2.GetProcessId() else { continue };
                    let level = control.cast::<IAudioMeterInformation>().ok().and_then(|m| m.GetPeakValue().ok()).unwrap_or(0.0);
                    out.push(Session { pid, active: state == AudioSessionStateActive, level });
                }
            }
        }
        Ok(out)
    }

    fn device_name(device: &IMMDevice) -> Option<String> {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        use windows::Win32::System::Com::STGM_READ;
        unsafe {
            let store = device.OpenPropertyStore(STGM_READ).ok()?;
            let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
            let text = value.to_string();
            (!text.is_empty()).then_some(text)
        }
    }

    /// Apps with a sound session now, busiest first, then every output device.
    pub fn list() -> Vec<Source> {
        com();
        let Ok(enumerator) = (unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }) else {
            return Vec::new();
        };
        let all = processes();
        let me = root(std::process::id(), &all);
        let mut apps: HashMap<String, Source> = HashMap::new();
        for s in sessions(&enumerator).unwrap_or_default() {
            let top = root(s.pid, &all);
            if top == me || s.pid == 0 {
                continue;
            }
            let Some(program) = all.get(&top).map(|p| p.program.clone()) else { continue };
            let key = program.to_ascii_lowercase();
            let label = friendly(&program);
            let entry = apps.entry(key).or_insert_with(|| Source {
                name: app_name(&label, &program),
                label,
                kind: "app".into(),
                active: false,
                level: 0.0,
            });
            entry.active |= s.active;
            entry.level = entry.level.max(s.level);
        }
        let mut out: Vec<Source> = apps.into_values().collect();
        out.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase())));
        unsafe {
            if let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                for i in 0..devices.GetCount().unwrap_or(0) {
                    let Ok(device) = devices.Item(i) else { continue };
                    let Some(label) = device_name(&device) else { continue };
                    let level = device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok().and_then(|m| m.GetPeakValue().ok()).unwrap_or(0.0);
                    out.push(Source { name: format!("{OUTPUT_PREFIX}{label}"), label, kind: "output".into(), active: level > 0.0, level });
                }
            }
        }
        out
    }

    /// The process to capture for a program: the app that owns a sound session if there is one, else
    /// any running copy that wasn't started by another copy of itself.
    fn target(program: &str) -> Option<u32> {
        let all = processes();
        com();
        let enumerator = unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let owners = enumerator.and_then(|e| sessions(&e).ok()).unwrap_or_default();
        let mine = |pid: &u32| all.get(pid).is_some_and(|p| p.program.eq_ignore_ascii_case(program));
        owners
            .iter()
            .map(|s| root(s.pid, &all))
            .find(mine)
            .or_else(|| all.keys().copied().filter(mine).find(|&pid| root(pid, &all) == pid))
    }

    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Activated(Mutex<Option<Sender<()>>>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
        fn ActivateCompleted(&self, _operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
            if let Some(tx) = self.0.lock().unwrap().take() {
                let _ = tx.send(());
            }
            Ok(())
        }
    }

    const CHANNELS: u16 = 2;
    pub const LOOPBACK_RATE: u32 = 48_000;

    fn activate(pid: u32) -> std::result::Result<IAudioClient, String> {
        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        // Borrows `params`. PROPVARIANT's own Drop would free the blob, which lives on the stack.
        let variant = ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB { cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &params as *const _ as *mut u8 },
                    },
                }),
            },
        });
        let (tx, rx) = mpsc::channel();
        let handler: IActivateAudioInterfaceCompletionHandler = Activated(Mutex::new(Some(tx))).into();
        let operation = unsafe {
            ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*variant as *const _), &handler)
        }
        .map_err(|e| format!("Windows couldn't capture the app ({e}). It needs Windows 10 version 2004 or later."))?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "Windows didn't answer when asked to capture the app.".to_string())?;
        let mut result = windows::core::HRESULT(0);
        let mut unknown = None;
        unsafe { operation.GetActivateResult(&mut result, &mut unknown) }.map_err(|e| e.to_string())?;
        result.ok().map_err(|e| e.to_string())?;
        unknown.ok_or("Windows gave no audio client.")?.cast::<IAudioClient>().map_err(|e| e.to_string())
    }

    /// Records one app on its own thread until dropped, sending mono blocks at 48 kHz.
    pub struct AppCapture {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for AppCapture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Starts recording a program's sound. Fails if it isn't running.
    pub fn open_app(
        program: &str,
        blocks: Sender<Vec<f32>>,
        failure: Arc<Mutex<Option<String>>>,
    ) -> std::result::Result<(AppCapture, u32), String> {
        let pid = target(program).ok_or_else(|| format!("{} isn't running", friendly(program)))?;
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let stop = stop.clone();
            thread::spawn(move || {
                com();
                let ready = ready_tx.clone();
                if let Err(message) = capture_loop(pid, &stop, &blocks, move || drop(ready.send(Ok(())))) {
                    if ready_tx.send(Err(message.clone())).is_err() {
                        *failure.lock().unwrap() = Some(message);
                    }
                }
            })
        };
        let mut capture = AppCapture { stop, thread: Some(thread) };
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok((capture, LOOPBACK_RATE)),
            Ok(Err(message)) => Err(message),
            Err(_) => {
                capture.stop.store(true, Ordering::Relaxed);
                drop(capture.thread.take());
                Err("the app's sound didn't start".into())
            }
        }
    }

    fn capture_loop(pid: u32, stop: &AtomicBool, blocks: &Sender<Vec<f32>>, started: impl FnOnce()) -> std::result::Result<(), String> {
        let client = activate(pid)?;
        let block_align = CHANNELS * 2;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS,
            nSamplesPerSec: LOOPBACK_RATE,
            nAvgBytesPerSec: LOOPBACK_RATE * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                    2_000_000,
                    0,
                    &format,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let event: HANDLE = CreateEventW(None, false, false, PWSTR::null()).map_err(|e| e.to_string())?;
            let result = (|| {
                client.SetEventHandle(event).map_err(|e| e.to_string())?;
                let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
                client.Start().map_err(|e| e.to_string())?;
                started();
                while !stop.load(Ordering::Relaxed) {
                    if WaitForSingleObject(event, 200) != WAIT_OBJECT_0 {
                        continue;
                    }
                    loop {
                        let size = capture.GetNextPacketSize().map_err(|e| e.to_string())?;
                        if size == 0 {
                            break;
                        }
                        let (mut data, mut frames, mut flags) = (std::ptr::null_mut::<u8>(), 0u32, 0u32);
                        capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).map_err(|e| e.to_string())?;
                        let mono: Vec<f32> = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                            vec![0.0; frames as usize]
                        } else {
                            let samples = std::slice::from_raw_parts(data as *const i16, frames as usize * CHANNELS as usize);
                            samples.chunks(CHANNELS as usize).map(|c| c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / CHANNELS as f32).collect()
                        };
                        capture.ReleaseBuffer(frames).map_err(|e| e.to_string())?;
                        if blocks.send(mono).is_err() {
                            return Ok(());
                        }
                    }
                }
                let _ = client.Stop();
                Ok(())
            })();
            let _ = CloseHandle(event);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let name = app_name("Microsoft Teams", "ms-teams.exe");
        assert_eq!(name, "Sound from Microsoft Teams (ms-teams.exe)");
        assert!(is_source(&name));
        assert_eq!(app_program(&name), Some("ms-teams.exe"));
        assert_eq!(app_program("Sound from A (b) (c.exe)"), Some("c.exe"));
        assert_eq!(output_device("Everything playing on Speakers (Realtek)"), Some("Speakers (Realtek)"));
        assert!(!is_source("Microphone (USB)"));
    }

    #[test]
    fn friendly_names() {
        assert_eq!(friendly("ms-teams.exe"), "Microsoft Teams");
        assert_eq!(friendly("Zoom.exe"), "Zoom");
        assert_eq!(friendly("obs64.exe"), "Obs64");
    }
}

/// Lists sources and records each playing app for two seconds: `cargo test --lib live_sources -- --ignored --nocapture`.
#[cfg(all(test, windows))]
mod live {
    #[test]
    #[ignore]
    fn live_sources() {
        for s in super::list() {
            println!("{:<70} active={} level={:.2}", s.name, s.active, s.level);
            if let (Some(program), true) = (super::app_program(&s.name), s.active) {
                let (tx, rx) = std::sync::mpsc::channel();
                let failure = std::sync::Arc::new(std::sync::Mutex::new(None));
                match super::open_app(program, tx, failure) {
                    Ok((capture, rate)) => {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        drop(capture);
                        let samples: Vec<f32> = rx.try_iter().flatten().collect();
                        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
                        println!("    captured {} samples at {rate} Hz, peak {peak:.3}", samples.len());
                    }
                    Err(e) => println!("    capture failed: {e}"),
                }
            }
        }
    }

    /// Records every source through the meeting recorder for three seconds.
    #[test]
    #[ignore]
    fn live_recording() {
        let names: Vec<String> = super::list().into_iter().filter(|s| s.active).map(|s| s.name).collect();
        let dir = std::env::temp_dir().join(format!("stv-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let recorder = crate::recorder::start(dir.clone(), &names, false).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        for s in recorder.status().unwrap().streams {
            println!("{} {:?}: {} {} frames level {:.2} {}", s.name, crate::recorder::device_of(&s.name, &names), s.state, s.frames, s.level, s.detail);
        }
        recorder.stop().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
