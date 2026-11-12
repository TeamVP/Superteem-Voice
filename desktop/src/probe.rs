//! Tests microphones before a meeting: each one is opened, nothing is saved, and it counts as working
//! once it hears something clearly above its own background.

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Probe {
    pub name: String,
    /// "opening", "listening", "failed", or on Android "absent" (no headset connected right now).
    pub state: String,
    /// 0 to 1, for the meter.
    pub level: f32,
    /// It has heard sound, so it can be used as a track.
    pub heard: bool,
    pub detail: String,
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::Probe;
    use crate::capture::{downmix, FRAME_SAMPLES};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use stv_core::attribute::window_db;

    /// Sound this far above the background, for this many 20 ms frames, shows the microphone works.
    const ABOVE_FLOOR_DB: f32 = 12.0;
    const MIN_DB: f32 = -50.0;
    const HEARD_FRAMES: usize = 5;

    struct Running {
        stop: Arc<AtomicBool>,
        probes: Arc<Mutex<BTreeMap<String, Probe>>>,
    }

    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
    /// Microphones heard working since the app started, so they needn't be tested again.
    static HEARD: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());

    /// Starts testing these microphones; any heard working earlier in this run count as working.
    pub fn start(devices: Vec<String>) {
        stop();
        let stop = Arc::new(AtomicBool::new(false));
        let probes = Arc::new(Mutex::new(BTreeMap::new()));
        for name in devices {
            let heard = HEARD.lock().unwrap().contains(&name);
            probes.lock().unwrap().insert(name.clone(), Probe { name: name.clone(), state: "opening".into(), heard, ..Default::default() });
            let (stop, probes) = (stop.clone(), probes.clone());
            thread::spawn(move || {
                if let Err(e) = listen(&name, &stop, &probes) {
                    if let Some(p) = probes.lock().unwrap().get_mut(&name) {
                        p.state = "failed".into();
                        p.level = 0.0;
                        p.detail = e;
                    }
                }
            });
        }
        *RUNNING.lock().unwrap() = Some(Running { stop, probes });
    }

    pub fn status() -> Vec<Probe> {
        RUNNING.lock().unwrap().as_ref().map(|r| r.probes.lock().unwrap().values().cloned().collect()).unwrap_or_default()
    }

    /// Closes every microphone being tested.
    pub fn stop() {
        if let Some(r) = RUNNING.lock().unwrap().take() {
            r.stop.store(true, Ordering::Relaxed);
        }
    }

    fn listen(name: &str, stop: &AtomicBool, probes: &Mutex<BTreeMap<String, Probe>>) -> Result<(), String> {
        let device = cpal::default_host()
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().is_ok_and(|n| n == name))
            .ok_or("it is no longer connected")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let channels = config.channels() as usize;
        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let failure = Arc::new(Mutex::new(None::<String>));
        let on_error = {
            let failure = failure.clone();
            move |e: cpal::StreamError| *failure.lock().unwrap() = Some(e.to_string())
        };
        let stream_config = config.config();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(&stream_config, move |d: &[f32], _: &_| drop(tx.send(downmix(d, channels, |s| s))), on_error, None),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |d: &[i16], _: &_| drop(tx.send(downmix(d, channels, |s| s as f32 / 32768.0))),
                on_error,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        let update = |change: &dyn Fn(&mut Probe)| {
            if let Some(p) = probes.lock().unwrap().get_mut(name) {
                change(p);
            }
        };
        update(&|p| p.state = "listening".into());

        // The level is measured at the device's own rate; frames are about 20 ms long.
        let frame = (config.sample_rate().0 as usize * FRAME_SAMPLES / crate::process::RATE as usize).max(1);
        let mut pending: Vec<i16> = Vec::new();
        let mut floor_db = -60.0f32;
        let mut loud = 0usize;
        let mut silent_since = std::time::Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if let Some(message) = failure.lock().unwrap().take() {
                return Err(message);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(block) => pending.extend(block.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // A device that delivers nothing at all is not working.
                    if silent_since.elapsed() > Duration::from_secs(3) {
                        return Err("it isn't sending any sound".into());
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            silent_since = std::time::Instant::now();
            while pending.len() >= frame {
                let chunk: Vec<i16> = pending.drain(..frame).collect();
                let db = window_db(&chunk, frame)[0];
                if db > MIN_DB && db > floor_db + ABOVE_FLOOR_DB {
                    loud += 1;
                }
                floor_db += if db < floor_db { (db - floor_db) * 0.2 } else { (db - floor_db) * 0.002 };
                let heard = loud >= HEARD_FRAMES;
                if heard && HEARD.lock().unwrap().insert(name.to_string()) {
                    // Remembered, so it is offered as a meeting track from now on.
                    let _ = crate::prefs::update(|p| {
                        if !p.working_mics.iter().any(|m| m == name) {
                            p.working_mics.push(name.to_string());
                        }
                    });
                }
                let level = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
                update(&|p| {
                    p.level = level;
                    p.heard |= heard;
                });
            }
        }
        drop(stream);
        Ok(())
    }
}

#[cfg(target_os = "android")]
mod imp {
    use super::Probe;
    use crate::recorder;

    /// Android listens through the capture plugin, which opens each microphone without recording it.
    pub fn start(devices: Vec<String>) {
        let _ = recorder::probe_start(&devices);
    }

    pub fn status() -> Vec<Probe> {
        recorder::probe_status()
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let state = match p.state.as_str() {
                    "recording" => "listening",
                    "starting" | "stopped" => "opening",
                    // No headset connected: not a fault, and it comes back by itself when one connects.
                    "no headset" | "waiting" => "absent",
                    _ => "failed",
                };
                Probe { heard: p.level > 0.35, name: p.name, state: state.into(), level: p.level, detail: p.detail }
            })
            .collect()
    }

    pub fn stop() {
        let _ = recorder::probe_stop();
    }
}

pub use imp::*;
