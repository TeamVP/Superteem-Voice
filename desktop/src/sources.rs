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

/// The name a source's track is shown by: "Microsoft Teams", or the output device.
pub fn track_label(name: &str) -> Option<String> {
    if let Some(device) = output_device(name) {
        return Some(device.to_string());
    }
    let rest = name.strip_prefix(APP_PREFIX)?;
    Some(rest.rfind(" (").map_or(rest, |i| &rest[..i]).to_string())
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
        /// The output device it plays to.
        device: String,
    }

    fn sessions(enumerator: &IMMDeviceEnumerator) -> WinResult<Vec<Session>> {
        let mut out = Vec::new();
        unsafe {
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let Ok(device) = devices.Item(i) else { continue };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let device_label = device_name(&device).unwrap_or_default();
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
                    out.push(Session { pid, active: state == AudioSessionStateActive, level, device: device_label.clone() });
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

    /// The top of a run of processes of one program: a browser's or WebView2's main process above its
    /// renderer and audio helpers.
    fn chain_top(pid: u32, all: &HashMap<u32, Process>) -> u32 {
        let mut current = pid;
        for _ in 0..16 {
            let Some(me) = all.get(&current) else { break };
            match all.get(&me.parent) {
                Some(parent) if me.parent != current && parent.program.eq_ignore_ascii_case(&me.program) => current = me.parent,
                _ => break,
            }
        }
        current
    }

    /// The processes to capture for a program. Windows credits an app's sound to the process that plays
    /// it, and doesn't always count a hosted WebView2 as part of the app's tree (new Teams plays calls
    /// through one), so each part of the app with a sound session is captured, plus the app itself.
    fn targets(program: &str) -> Vec<u32> {
        let all = processes();
        com();
        let enumerator = unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let owners = enumerator.and_then(|e| sessions(&e).ok()).unwrap_or_default();
        let mine = |pid: u32| all.get(&pid).is_some_and(|p| p.program.eq_ignore_ascii_case(program));
        let mut out: Vec<u32> = all.keys().copied().filter(|&pid| mine(pid) && root(pid, &all) == pid).collect();
        for s in owners {
            if mine(root(s.pid, &all)) {
                out.push(chain_top(s.pid, &all));
                out.push(s.pid);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// How loud the program's own sound sessions are, and the output device of the loudest.
    fn app_level(program: &str) -> (f32, String) {
        com();
        let all = processes();
        let Ok(e) = (unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }) else { return (0.0, String::new()) };
        sessions(&e)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| all.get(&root(s.pid, &all)).is_some_and(|p| p.program.eq_ignore_ascii_case(program)))
            .fold((0.0, String::new()), |best, s| if s.level > best.0 { (s.level, s.device) } else { best })
    }

    /// Records everything an output device plays, converted to 48 kHz mono, until dropped.
    struct DeviceFallback {
        _stream: cpal::Stream,
        blocks: mpsc::Receiver<Vec<f32>>,
        queue: std::collections::VecDeque<f32>,
        device: String,
    }

    fn open_device(name: &str) -> std::result::Result<DeviceFallback, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().is_ok_and(|n| n == name))
            .ok_or_else(|| format!("{name} isn't connected"))?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let channels = config.channels() as usize;
        let step = config.sample_rate().0 as f64 / LOOPBACK_RATE as f64;
        let (tx, blocks) = mpsc::channel::<Vec<f32>>();
        let mut position = 0f64;
        let mut carried: Vec<f32> = Vec::new();
        let mut convert = move |mono: Vec<f32>| {
            // Interpolates between neighbouring samples from the device's rate to 48 kHz, carrying the
            // last samples over so blocks join without a step.
            carried.extend(mono);
            let mut out = Vec::with_capacity((carried.len() as f64 / step) as usize + 1);
            while position as usize + 1 < carried.len() {
                let i = position as usize;
                let f = (position - i as f64) as f32;
                out.push(carried[i] * (1.0 - f) + carried[i + 1] * f);
                position += step;
            }
            let used = (position as usize).min(carried.len());
            carried.drain(..used);
            position -= used as f64;
            drop(tx.send(out));
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(&config.config(), move |d: &[f32], _: &_| convert(crate::capture::downmix(d, channels, |s| s)), |_| {}, None),
            cpal::SampleFormat::I16 => device.build_input_stream(&config.config(), move |d: &[i16], _: &_| convert(crate::capture::downmix(d, channels, |s| s as f32 / 32768.0)), |_| {}, None),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(DeviceFallback { _stream: stream, blocks, queue: Default::default(), device: name.into() })
    }

    /// For the diagnostic test: every sound session's process, its app and its level.
    #[cfg(test)]
    pub fn session_pids() -> Vec<(u32, u32, String, f32)> {
        com();
        let all = processes();
        let e: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.unwrap();
        sessions(&e).unwrap().into_iter().map(|s| (s.pid, root(s.pid, &all), all.get(&s.pid).map(|p| p.program.clone()).unwrap_or_default(), s.level)).collect()
    }

    /// For the diagnostic test: records one process tree for `seconds` and returns the peak.
    #[cfg(test)]
    pub fn peak_of(pid: u32, seconds: u64) -> std::result::Result<f32, String> {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = thread::spawn(move || { com(); capture_loop(pid, &s2, &tx, || ()) });
        thread::sleep(Duration::from_secs(seconds));
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap()?;
        Ok(rx.try_iter().flatten().fold(0f32, |m, s: f32| m.max(s.abs())))
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

    /// Records one app until dropped, sending mono blocks at 48 kHz. Each of its processes with sound is
    /// captured on a thread of its own, and new ones are picked up every second, so a call that starts
    /// after the recording is heard too. The parts are mixed on the clock, so silence keeps time.
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

    struct Part {
        stop: Arc<AtomicBool>,
        blocks: mpsc::Receiver<Vec<f32>>,
        queue: std::collections::VecDeque<f32>,
        failed: Arc<Mutex<Option<String>>>,
        /// Loudest sample since the last line of the log.
        peak: f32,
        packets: u64,
    }

    /// Notes which processes an app capture listens to and what each hears, every few seconds, in
    /// sources.log beside the app's data, for finding out why a track is silent.
    fn log(line: &str) {
        use std::io::Write;
        let path = crate::library::data_dir().join("sources.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) {
            let _ = std::fs::remove_file(&path);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let _ = writeln!(f, "{secs} {line}");
        }
    }

    fn start_part(pid: u32) -> Part {
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(Mutex::new(None));
        let (tx, blocks) = mpsc::channel();
        {
            let (stop, failed) = (stop.clone(), failed.clone());
            thread::spawn(move || {
                com();
                if let Err(message) = capture_loop(pid, &stop, &tx, || ()) {
                    *failed.lock().unwrap() = Some(message);
                }
            });
        }
        Part { stop, blocks, queue: Default::default(), failed, peak: 0.0, packets: 0 }
    }

    /// Starts recording a program's sound. Fails if it isn't running.
    pub fn open_app(
        program: &str,
        blocks: Sender<Vec<f32>>,
        failure: Arc<Mutex<Option<String>>>,
        events: Arc<Mutex<Vec<String>>>,
    ) -> std::result::Result<(AppCapture, u32), String> {
        let first = targets(program);
        if first.is_empty() {
            return Err(format!("{} isn't running", friendly(program)));
        }
        let program = program.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            thread::spawn(move || mix(&program, first, &stop, &blocks, &failure, &events))
        };
        Ok((AppCapture { stop, thread: Some(thread) }, LOOPBACK_RATE))
    }

    fn mix(program: &str, first: Vec<u32>, stop: &AtomicBool, out: &Sender<Vec<f32>>, failure: &Mutex<Option<String>>, events: &Mutex<Vec<String>>) {
        let event = |text: String| events.lock().unwrap().push(text);
        let mut parts: HashMap<u32, Part> = first.into_iter().map(|pid| (pid, start_part(pid))).collect();
        let mut last_log = std::time::Instant::now();
        // Windows hands some apps' sound to per-app capture as silence (Teams' calls). When the app's own
        // meter shows sound that the capture doesn't hear, its output device is recorded instead.
        let mut fallback: Option<DeviceFallback> = None;
        let mut unheard = 0u32;
        let mut heard_peak = 0f32;
        log(&format!("{program}: capturing {:?}", parts.keys().collect::<Vec<_>>()));
        {
            let all = processes();
            let names: Vec<String> = parts.keys().map(|p| format!("{} {p}", all.get(p).map_or("?", |x| x.program.as_str()))).collect();
            event(format!("Capturing {} directly: {}", friendly(program), names.join(", ")));
        }
        let mut last_scan = std::time::Instant::now();
        // Each part may run up to this far ahead before its oldest sound is dropped.
        let most = LOOPBACK_RATE as usize / 5;
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
            if last_scan.elapsed() >= Duration::from_secs(1) {
                last_scan = std::time::Instant::now();
                for pid in targets(program) {
                    parts.entry(pid).or_insert_with(|| {
                        log(&format!("{program}: also capturing {pid}"));
                        start_part(pid)
                    });
                }
                // A process that ended or couldn't be captured is let go; it comes back if it plays again.
                parts.retain(|_, p| {
                    let gone = p.failed.lock().unwrap().is_some();
                    if gone {
                        log(&format!("{program}: let go of a part: {:?}", p.failed.lock().unwrap()));
                        p.stop.store(true, Ordering::Relaxed);
                    }
                    !gone
                });
                if parts.is_empty() {
                    *failure.lock().unwrap() = Some(format!("{} closed", friendly(program)));
                    return;
                }
                let (level, device) = app_level(program);
                if heard_peak > 0.001 {
                    unheard = 0;
                    if let Some(f) = fallback.take() {
                        log(&format!("{program}: heard directly again; stopped recording {}", f.device));
                        event(format!("Heard {} directly again; stopped recording {}", friendly(program), f.device));
                    }
                } else if level > 0.02 {
                    unheard += 1;
                }
                heard_peak = 0.0;
                let wrong_device = fallback.as_ref().is_some_and(|f| !device.is_empty() && f.device != device && level > 0.02);
                if (unheard >= 2 && fallback.is_none()) || wrong_device {
                    match open_device(&device) {
                        Ok(f) => {
                            log(&format!("{program}: plays at {level:.2} but per-app capture hears nothing; recording {device}"));
                            event(format!("Windows gave only silence from {}, so recording everything on {device} instead", friendly(program)));
                            fallback = Some(f);
                        }
                        Err(e) => {
                            log(&format!("{program}: couldn't record {device}: {e}"));
                            event(format!("Couldn't record {device}: {e}"));
                        }
                    }
                }
            }
            for p in parts.values_mut() {
                for block in p.blocks.try_iter() {
                    p.packets += 1;
                    p.peak = block.iter().fold(p.peak, |m, s| m.max(s.abs()));
                    heard_peak = heard_peak.max(p.peak);
                    p.queue.extend(block);
                }
                let extra = p.queue.len().saturating_sub(most);
                p.queue.drain(..extra);
            }
            if let Some(f) = fallback.as_mut() {
                for block in f.blocks.try_iter() {
                    f.queue.extend(block);
                }
                let extra = f.queue.len().saturating_sub(most);
                f.queue.drain(..extra);
            }
            if last_log.elapsed() >= Duration::from_secs(3) {
                last_log = std::time::Instant::now();
                let all = processes();
                let line: Vec<String> = parts
                    .iter_mut()
                    .map(|(pid, p)| {
                        let name = all.get(pid).map_or("?", |x| x.program.as_str());
                        let text = format!("{pid} {name} packets {} peak {:.3}", p.packets, p.peak);
                        (p.peak, p.packets) = (0.0, 0);
                        text
                    })
                    .collect();
                let via = fallback.as_ref().map_or(String::new(), |f| format!(" (recording {} instead)", f.device));
                log(&format!("{program}: {}{via}", line.join("; ")));
            }
            // The output follows one stream's own clock, the device when it is being recorded, else the
            // loudest part, so its sound is passed on whole. Pacing by the computer's clock instead
            // left tiny gaps whenever the sound arrived a little late, heard as a buzz. The other parts
            // add what they have; the recorder places any pause by the clock.
            let leader_len = match fallback.as_ref() {
                Some(f) => f.queue.len(),
                None => parts.values().max_by(|a, b| a.peak.total_cmp(&b.peak).then(a.queue.len().cmp(&b.queue.len()))).map_or(0, |p| p.queue.len()),
            };
            let n = leader_len;
            if n == 0 {
                continue;
            }
            let mut block = vec![0f32; n];
            for p in parts.values_mut() {
                let take = n.min(p.queue.len());
                for (o, s) in block.iter_mut().zip(p.queue.drain(..take)) {
                    *o += s;
                }
            }
            if let Some(f) = fallback.as_mut() {
                let take = n.min(f.queue.len());
                for (o, s) in block.iter_mut().zip(f.queue.drain(..take)) {
                    *o += s;
                }
            }
            if out.send(block).is_err() {
                break;
            }
        }
        for p in parts.values() {
            p.stop.store(true, Ordering::Relaxed);
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
        assert_eq!(track_label(&name).as_deref(), Some("Microsoft Teams"));
        assert_eq!(track_label("Mic"), None);
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
                match super::open_app(program, tx, failure, Default::default()) {
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
        std::thread::sleep(std::time::Duration::from_secs(8));
        for s in recorder.status().unwrap().streams {
            println!("{} {:?}: {} {} frames level {:.2} {}", s.name, crate::recorder::device_of(&s.name, &names), s.state, s.frames, s.level, s.detail);
        }
        recorder.stop().unwrap();
        if std::env::var("STV_KEEP").is_ok() {
            println!("kept {}", dir.display());
        } else {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Which process of an app carries its sound: `cargo test --lib live_pids -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_pids() {
        let sessions = super::win::session_pids();
        for (pid, root, program, level) in &sessions {
            println!("session pid {pid} ({program}) root {root} level {level:.2}");
        }
        let mut pids: Vec<u32> = sessions.iter().flat_map(|s| [s.0, s.1]).collect::<Vec<u32>>().into_iter().collect();
        pids.extend(std::env::var("STV_PIDS").unwrap_or_default().split(',').filter_map(|p| p.trim().parse::<u32>().ok()));
        pids.sort();
        pids.dedup();
        for pid in pids {
            println!("pid {pid}: {:?}", super::win::peak_of(pid, 2));
        }
    }

    /// Plays a tone to each output and records this process: shows which outputs per-app capture hears.
    #[test]
    #[ignore]
    fn live_device_tone() {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        println!("default output: {:?}", host.default_output_device().and_then(|d| d.name().ok()));
        for device in host.output_devices().unwrap() {
            let name = device.name().unwrap_or_default();
            let Ok(config) = device.default_output_config() else { continue };
            let (rate, channels) = (config.sample_rate().0 as f32, config.channels() as usize);
            let mut t = 0f32;
            let stream = match device.build_output_stream(&config.config(), move |d: &mut [f32], _: &_| {
                for frame in d.chunks_mut(channels) { t += 1.0; let v = (t * 440.0 * 6.283 / rate).sin() * 0.2; frame.iter_mut().for_each(|s| *s = v); }
            }, |_| {}, None) { Ok(s) => s, Err(e) => { println!("{name}: can't play ({e})"); continue } };
            stream.play().unwrap();
            println!("{name}: per-app peak {:?}", super::win::peak_of(std::process::id(), 2));
        }
    }

    /// Waits up to ten minutes for an app's session to play, then records each process of every app
    /// playing: `cargo test --lib live_wait -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_wait() {
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 600 {
            let sessions = super::win::session_pids();
            let root: u32 = std::env::var("STV_ROOT").ok().and_then(|r| r.parse().ok()).unwrap_or(0);
            if sessions.iter().any(|s| s.3 > 0.02 && (root == 0 || s.1 == root)) {
                for (pid, root, program, level) in &sessions {
                    println!("session pid {pid} ({program}) root {root} level {level:.2}");
                }
                let all: Vec<u32> = sessions.iter().flat_map(|s| [s.0, s.1]).collect::<Vec<u32>>();
                let mut pids = all.clone();
                pids.extend(std::env::var("STV_PIDS").unwrap_or_default().split(',').filter_map(|p| p.trim().parse::<u32>().ok()));
                pids.sort();
                pids.dedup();
                for pid in pids {
                    let name = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &format!("(Get-Process -Id {pid}).ProcessName")]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                    println!("pid {pid} {name}: {:?}", super::win::peak_of(pid, 3));
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        println!("nothing played");
    }
}
