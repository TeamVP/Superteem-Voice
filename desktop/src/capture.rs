//! Records Windows input devices and BLE pins to journals, one file per stream.

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::StreamExt;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use stv_core::attribute::window_db;
use stv_core::journal::JournalWriter;
use stv_core::protocol::{Codec, Frame};
use uuid::Uuid;

pub use crate::process::{FRAME_SAMPLES, RATE};
const MAX_PINS: usize = 3;
const PIN_SERVICE: Uuid = Uuid::from_u128(0x5354564f_1000_4e00_8000_535456000001);
const PIN_AUDIO: Uuid = Uuid::from_u128(0x5354564f_1001_4e00_8000_535456000001);

#[derive(Clone, Default)]
pub struct StreamStats {
    pub state: String,
    pub frames: u64,
    pub lost: u64,
    pub level_db: f32,
    pub detail: String,
}

pub struct Shared {
    pub stats: Mutex<BTreeMap<String, StreamStats>>,
    pub stop: AtomicBool,
    pub started: Instant,
    pub dir: PathBuf,
}

impl Shared {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self { stats: Mutex::default(), stop: AtomicBool::new(false), started: Instant::now(), dir })
    }

    fn update(&self, name: &str, change: impl FnOnce(&mut StreamStats)) {
        change(self.stats.lock().unwrap().entry(name.to_string()).or_default());
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }
}

pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub fn default_input() -> Option<String> {
    cpal::default_host().default_input_device().and_then(|d| d.name().ok())
}

/// Converts to 16 kHz by averaging, which also serves as a basic anti-alias filter.
pub struct Resampler {
    ratio: f64,
    position: f64,
    sum: f32,
    count: u32,
}

impl Resampler {
    pub fn new(input_rate: u32) -> Self {
        Self { ratio: input_rate as f64 / RATE as f64, position: 0.0, sum: 0.0, count: 0 }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        for &sample in input {
            self.sum += sample;
            self.count += 1;
            self.position += 1.0;
            if self.position >= self.ratio {
                self.position -= self.ratio;
                out.push(((self.sum / self.count as f32).clamp(-1.0, 1.0) * 32767.0) as i16);
                self.sum = 0.0;
                self.count = 0;
            }
        }
    }
}

pub fn downmix<T: Copy>(data: &[T], channels: usize, convert: impl Fn(T) -> f32) -> Vec<f32> {
    data.chunks(channels).map(|c| c.iter().map(|&s| convert(s)).sum::<f32>() / channels as f32).collect()
}

/// Runs on its own thread, which also owns the audio stream, until a stop is requested.
pub fn run_input(shared: Arc<Shared>, device_name: String, label: String) {
    if let Err(message) = record_input(&shared, &device_name, &label) {
        shared.update(&label, |s| {
            s.state = "failed".into();
            s.detail = message;
        });
    }
}

/// How long a microphone may send nothing before it is treated as lost. Windows hands over a buffer
/// every 10 ms or so, even in a silent room.
const STALL: Duration = Duration::from_secs(2);

/// An open microphone stream. Dropping it closes the microphone.
struct Input {
    _stream: cpal::Stream,
    blocks: mpsc::Receiver<Vec<f32>>,
    failure: Arc<Mutex<Option<String>>>,
    rate: u32,
    name: String,
}

fn find_input(name: &str) -> Option<cpal::Device> {
    cpal::default_host().input_devices().ok()?.find(|d| d.name().is_ok_and(|n| n == name))
}

fn open_input(device: cpal::Device) -> Result<Input, String> {
    let name = device.name().map_err(|e| e.to_string())?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let channels = config.channels() as usize;
    let (tx, blocks) = mpsc::channel::<Vec<f32>>();
    let failure = Arc::new(Mutex::new(None::<String>));
    let on_error = {
        let failure = failure.clone();
        move |e: cpal::StreamError| *failure.lock().unwrap() = Some(e.to_string())
    };
    let stream_config = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |d: &[f32], _: &_| drop(tx.send(downmix(d, channels, |s| s))),
            on_error,
            None,
        ),
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
    Ok(Input { _stream: stream, blocks, failure, rate: config.sample_rate().0, name })
}

/// Waits for a microphone to come back after the one being recorded was lost, for example when a
/// call switches a Bluetooth headset to its hands-free microphone or another app takes the device.
/// Prefers the chosen microphone and otherwise takes whatever Windows uses by default now.
/// Returns None once a stop is requested.
fn reconnect(shared: &Shared, device_name: &str, label: &str, reason: &str) -> Option<Input> {
    shared.update(label, |s| {
        s.state = "reconnecting".into();
        s.detail = format!("Lost {device_name} ({reason}), trying again");
    });
    while !shared.stopped() {
        thread::sleep(Duration::from_secs(1));
        let device = find_input(device_name).or_else(|| cpal::default_host().default_input_device());
        match device.ok_or_else(|| "no microphone connected".to_string()).and_then(open_input) {
            Ok(input) => return Some(input),
            Err(message) => shared.update(label, |s| s.detail = format!("Waiting for a microphone: {message}")),
        }
    }
    None
}

fn record_input(shared: &Shared, device_name: &str, label: &str) -> Result<(), String> {
    let mut input = open_input(find_input(device_name).ok_or("device not found")?)?;
    let mut journal = JournalWriter::create(&shared.dir.join(format!("{label}.stvj"))).map_err(|e| e.to_string())?;
    let mut resampler = Resampler::new(input.rate);
    let mut pending = Vec::new();
    let mut sequence = 0u32;
    // Frames are placed by sample position, so a gap while reconnecting moves the position on and
    // becomes silence in the meeting rather than pulling later speech earlier.
    let mut sample = 0u32;
    let mut origin_us: Option<u64> = None;
    let mut last_sync = Instant::now();
    let mut last_block = Instant::now();
    let mut connected = true;
    while !shared.stopped() {
        if connected {
            connected = false;
            shared.update(label, |s| {
                s.state = "recording".into();
                s.detail = format!("{} ({} Hz)", input.name, input.rate);
            });
        }
        let mut lost = input.failure.lock().unwrap().take();
        if lost.is_none() {
            match input.blocks.recv_timeout(Duration::from_millis(200)) {
                Ok(block) => {
                    last_block = Instant::now();
                    resampler.push(&block, &mut pending);
                }
                Err(RecvTimeoutError::Timeout) if last_block.elapsed() < STALL => continue,
                Err(RecvTimeoutError::Timeout) => lost = Some("it stopped sending sound".into()),
                Err(RecvTimeoutError::Disconnected) => lost = Some("the stream closed".into()),
            }
        }
        if let Some(reason) = lost {
            journal.sync().map_err(|e| e.to_string())?;
            drop(input);
            let Some(next) = reconnect(shared, device_name, label, &reason) else { break };
            input = next;
            resampler = Resampler::new(input.rate);
            pending.clear();
            if let Some(origin) = origin_us {
                sample = sample.max(position_at(shared.now_us(), origin));
            }
            last_block = Instant::now();
            connected = true;
            continue;
        }
        let arrival = shared.now_us();
        origin_us.get_or_insert_with(|| {
            arrival.saturating_sub((sample as u64 + pending.len() as u64) * 1_000_000 / RATE as u64)
        });
        for pcm in pending.chunks_exact(FRAME_SAMPLES) {
            let frame = Frame {
                flags: 0,
                pin_session_id: 1,
                sequence,
                sample_counter: sample,
                codec: Codec::RawPcm16,
                payload: pcm.iter().flat_map(|s| s.to_le_bytes()).collect(),
            };
            journal.append(arrival, &frame.encode()).map_err(|e| e.to_string())?;
            sequence += 1;
            sample = sample.wrapping_add(FRAME_SAMPLES as u32);
            let level = window_db(pcm, FRAME_SAMPLES)[0];
            shared.update(label, |s| {
                s.frames += 1;
                s.level_db = level;
            });
        }
        let used = pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        pending.drain(..used);
        if last_sync.elapsed() >= Duration::from_secs(1) {
            journal.sync().map_err(|e| e.to_string())?;
            last_sync = Instant::now();
        }
    }
    journal.sync().map_err(|e| e.to_string())?;
    shared.update(label, |s| s.state = "stopped".into());
    Ok(())
}

/// The sample position, on whole frames, that a stream which began at `origin_us` reaches at `now_us`.
fn position_at(now_us: u64, origin_us: u64) -> u32 {
    let samples = now_us.saturating_sub(origin_us) * RATE as u64 / 1_000_000;
    (samples / FRAME_SAMPLES as u64 * FRAME_SAMPLES as u64) as u32
}

/// Finds and records pins until a stop is requested. Windows apps cannot use L2CAP channels through
/// this library, so pins stream using the firmware's GATT notification fallback.
pub fn run_pins(shared: Arc<Shared>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            shared.update("pins", |s| {
                s.state = "failed".into();
                s.detail = e.to_string();
            });
            return;
        }
    };
    if let Err(message) = runtime.block_on(scan_for_pins(shared.clone())) {
        shared.update("pins", |s| {
            s.state = "failed".into();
            s.detail = message;
        });
    }
}

async fn scan_for_pins(shared: Arc<Shared>) -> Result<(), String> {
    let manager = Manager::new().await.map_err(|e| e.to_string())?;
    let adapter: Adapter =
        manager.adapters().await.map_err(|e| e.to_string())?.into_iter().next().ok_or("no Bluetooth adapter")?;
    adapter.start_scan(ScanFilter { services: vec![PIN_SERVICE] }).await.map_err(|e| e.to_string())?;
    shared.update("pins", |s| {
        s.state = "searching".into();
        s.detail = "Pair pins in Windows Bluetooth settings first".into();
    });
    let mut seen = HashSet::new();
    let mut tasks = Vec::new();
    while !shared.stopped() {
        for peripheral in adapter.peripherals().await.map_err(|e| e.to_string())? {
            let is_pin = matches!(peripheral.properties().await, Ok(Some(p)) if p.services.contains(&PIN_SERVICE));
            if is_pin && seen.len() < MAX_PINS && seen.insert(peripheral.id()) {
                let label = format!("pin{}", seen.len());
                tasks.push(tokio::spawn(run_pin(shared.clone(), peripheral, label)));
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = adapter.stop_scan().await;
    for task in tasks {
        let _ = task.await;
    }
    shared.update("pins", |s| s.state = "stopped".into());
    Ok(())
}

async fn run_pin(shared: Arc<Shared>, peripheral: Peripheral, label: String) {
    let mut attempt = 0;
    while !shared.stopped() {
        shared.update(&label, |s| s.state = "connecting".into());
        if let Err(message) = stream_pin(&shared, &peripheral, &label, attempt).await {
            shared.update(&label, |s| s.detail = message);
        }
        let _ = peripheral.disconnect().await;
        attempt += 1;
        if !shared.stopped() {
            shared.update(&label, |s| s.state = "reconnecting".into());
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    shared.update(&label, |s| s.state = "stopped".into());
}

async fn stream_pin(shared: &Shared, peripheral: &Peripheral, label: &str, attempt: u32) -> Result<(), String> {
    peripheral.connect().await.map_err(|e| e.to_string())?;
    peripheral.discover_services().await.map_err(|e| e.to_string())?;
    let audio = peripheral.characteristics().into_iter().find(|c| c.uuid == PIN_AUDIO).ok_or("not a Superteem pin")?;
    let mut notifications = peripheral.notifications().await.map_err(|e| e.to_string())?;
    peripheral.subscribe(&audio).await.map_err(|e| e.to_string())?;
    shared.update(label, |s| s.state = "recording".into());

    let mut journal =
        JournalWriter::create(&shared.dir.join(format!("{label}-{attempt}.stvj"))).map_err(|e| e.to_string())?;
    let mut last_sequence: Option<u32> = None;
    let mut last_sync = Instant::now();
    while !shared.stopped() {
        let next = tokio::time::timeout(Duration::from_millis(500), notifications.next()).await;
        let Ok(next) = next else { continue };
        let Some(notification) = next else { break };
        journal.append(shared.now_us(), &notification.value).map_err(|e| e.to_string())?;
        if let Ok(frame) = Frame::decode(&notification.value) {
            let lost = last_sequence.map_or(0, |last| frame.sequence.saturating_sub(last + 1) as u64);
            last_sequence = Some(frame.sequence);
            shared.update(label, |s| {
                s.frames += 1;
                s.lost += lost;
            });
        }
        if last_sync.elapsed() >= Duration::from_secs(1) {
            journal.sync().map_err(|e| e.to_string())?;
            last_sync = Instant::now();
        }
    }
    journal.sync().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resampled_len(input_rate: u32, seconds: usize) -> usize {
        let mut out = Vec::new();
        let mut resampler = Resampler::new(input_rate);
        for _ in 0..seconds * 100 {
            resampler.push(&vec![0.5; input_rate as usize / 100], &mut out);
        }
        out.len()
    }

    #[test]
    fn resampler_produces_16khz_from_common_rates() {
        for rate in [16_000, 44_100, 48_000] {
            assert!(resampled_len(rate, 10).abs_diff(160_000) <= 1, "rate {rate}");
        }
    }

    #[test]
    fn resampler_keeps_level_and_clamps() {
        let mut out = Vec::new();
        Resampler::new(48_000).push(&[0.5; 30], &mut out);
        assert!(out.iter().all(|&s| (s - 16383).abs() <= 1));
        out.clear();
        Resampler::new(16_000).push(&[2.0], &mut out);
        assert_eq!(out, [32767]);
    }

    #[test]
    fn position_after_a_gap_lands_on_a_whole_frame() {
        assert_eq!(position_at(1_000_000, 0), 16_000);
        assert_eq!(position_at(2_500_010, 500_000) % FRAME_SAMPLES as u32, 0);
        assert_eq!(position_at(2_500_010, 500_000), 32_000);
        assert_eq!(position_at(10, 20), 0);
    }

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[1.0f32, 0.0, 0.5, 0.5], 2, |s| s), vec![0.5, 0.5]);
    }
}
