//! Audio tracks added to a meeting after it was recorded, such as a phone's recording of the same room, or
//! the other side of a call recorded by the person on it. Each is kept as it came (16 kHz mono, decoded by
//! the page) beside the meeting's own audio, with where it sits on the meeting's timeline; processing then
//! treats it like any other track.
//!
//! The time of day is what places a track first: when it started (from the file, or typed in) against when
//! the meeting started. Two recordings of one call can share no sound at all, so this is the one thing that
//! always works. Matching the sound (`stv_core::sync`) then fine-tunes it near there, taking up a relay's
//! delay or the distance between microphones, and how fast one device's clock ran against the other's.

use crate::library::{self, Meta};
use crate::process::RATE;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stv_core::export;

const FILE: &str = "imports.json";
/// The imported audio as it came, before it's placed on the timeline. Not ".wav", so it isn't taken for a
/// processed track.
const EXTENSION: &str = "track";
/// How far either side of the time of day matching the sound looks: clocks on two devices are seldom
/// further apart than this.
pub const NEAR_S: f64 = 120.0;

fn one() -> f64 {
    1.0
}

fn is_one(x: &f64) -> bool {
    *x == 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Import {
    /// The stream key: "import-1" and so on.
    pub name: String,
    /// Where on the meeting's timeline it starts, in seconds; negative when it started first.
    pub offset_s: f64,
    /// Meeting seconds per second of this track: off 1 when its device's clock ran fast or slow.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub rate: f64,
    /// When it started, as a time of day (Unix seconds), when that's known.
    #[serde(default)]
    pub started_at: Option<f64>,
    /// What placed it: "time" (the time of day), "sound" (matched to the other tracks), or "hand" (typed in).
    #[serde(default)]
    pub placed_by: String,
    /// How sure the last match of the sound was, when it was matched: the best match over the next best.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// Half-minute stretches that matched, of those tried, when it was matched.
    #[serde(default)]
    pub matched: Option<[u32; 2]>,
    pub seconds: f64,
}

/// How to place a track being added.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// It started at this time of day (Unix seconds); `refine` matches the sound near there too.
    Time { started_at: f64, refine: bool },
    /// Matched to the other tracks wherever it falls.
    Sound,
    /// It starts this many seconds into the meeting.
    At(f64),
}

pub fn load(dir: &Path) -> Vec<Import> {
    library::read_json(&dir.join(FILE)).unwrap_or_default()
}

fn save(dir: &Path, imports: &[Import]) -> Result<(), String> {
    if imports.is_empty() {
        let _ = std::fs::remove_file(dir.join(FILE));
        Ok(())
    } else {
        library::write_json(&dir.join(FILE), &imports)
    }
}

fn source_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

pub fn is_track_file(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == EXTENSION)
}

fn read_pcm(path: &Path) -> Result<Vec<i16>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let at = data.windows(4).position(|w| w == b"data").ok_or("Not a WAV file.")? + 8;
    Ok(data[at.min(data.len())..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// When the meeting started, as a time of day.
fn meeting_start(dir: &Path) -> f64 {
    library::load_meta(dir).created as f64
}

/// The imported audio placed on the meeting's timeline: silence before it when it started later, trimmed
/// when it started first, and stretched or squeezed when its clock ran at another speed.
pub fn placed(dir: &Path, import: &Import) -> Result<Vec<i16>, String> {
    let pcm = read_pcm(&source_path(dir, &import.name))?;
    Ok(place(&pcm, import.offset_s, import.rate))
}

fn place(pcm: &[i16], offset_s: f64, rate: f64) -> Vec<i16> {
    if pcm.is_empty() {
        return vec![];
    }
    let shift = offset_s * RATE as f64;
    let end = (shift + pcm.len() as f64 * rate).ceil().max(0.0) as usize;
    if rate == 1.0 {
        let shift = shift.round() as i64;
        return if shift >= 0 {
            let mut out = vec![0i16; shift as usize];
            out.extend_from_slice(pcm);
            out
        } else {
            pcm.get((-shift) as usize..).map(<[i16]>::to_vec).unwrap_or_default()
        };
    }
    (0..end)
        .map(|k| {
            let x = (k as f64 - shift) / rate;
            if x < 0.0 || x >= (pcm.len() - 1) as f64 {
                return 0;
            }
            let (i, f) = (x as usize, x.fract());
            (pcm[i] as f64 + (pcm[i + 1] as f64 - pcm[i] as f64) * f).round() as i16
        })
        .collect()
}

/// The meeting's other tracks, mixed, to match an import against.
fn reference(dir: &Path, except: &str) -> Result<Vec<i16>, String> {
    let mut mix: Vec<i32> = Vec::new();
    let imported: Vec<String> = load(dir).into_iter().map(|i| i.name).collect();
    let mut any = false;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // Recorded tracks only: another import may itself be placed wrongly.
        if path.extension().is_none_or(|x| x != "wav") || stem == except || imported.contains(&stem) {
            continue;
        }
        let pcm = read_pcm(&path)?;
        if mix.len() < pcm.len() {
            mix.resize(pcm.len(), 0);
        }
        for (m, s) in mix.iter_mut().zip(pcm) {
            *m += s as i32;
        }
        any = true;
    }
    if !any {
        return Err("This meeting has no processed tracks to match it to yet.".into());
    }
    Ok(mix.into_iter().map(|m| m.clamp(i16::MIN as i32, i16::MAX as i32) as i16).collect())
}

/// Matches an import's sound to the recorded tracks and keeps where it fits. `near`: only close to where it
/// is now, as placed by the time of day; otherwise anywhere in the meeting. When nothing matches clearly it
/// stays where it was, and the error says so.
pub fn sync(dir: &Path, name: &str, near: bool) -> Result<Import, String> {
    let mut imports = load(dir);
    let import = imports.iter_mut().find(|i| i.name == name).ok_or("That track no longer exists.")?;
    let reference = reference(dir, name)?;
    let pcm = read_pcm(&source_path(dir, name))?;
    let fit = if near {
        stv_core::sync::fit(&reference, &pcm, RATE, import.offset_s, NEAR_S).ok_or(
            "Its sound didn't clearly match the other tracks within two minutes of where it is, so it stays there. \
             Recordings of two sides of a call often share no sound: the time of day is what lines them up.",
        )?
    } else {
        let (around, _) = stv_core::sync::find_offset(&reference, &pcm, RATE).ok_or("The track or the meeting is too short to match.")?;
        stv_core::sync::fit(&reference, &pcm, RATE, around, 3.0)
            .ok_or("Its sound didn't clearly match the other tracks anywhere, so it stays where it was.")?
    };
    import.offset_s = fit.offset_s;
    import.rate = fit.rate;
    import.confidence = Some(fit.confidence);
    import.matched = Some([fit.matched, fit.tried]);
    import.placed_by = "sound".into();
    let found = import.clone();
    save(dir, &imports)?;
    Ok(found)
}

/// Adds a track: `pcm` is 16 kHz mono. It's named by `label`, the name shown for it, and placed as `place`
/// says. Placed by the time of day and fine-tuned, it stays at the time of day when the sound doesn't match.
pub fn add(dir: &Path, label: &str, pcm: &[i16], place: Place) -> Result<Import, String> {
    if pcm.len() < RATE as usize {
        return Err("That recording is less than a second long.".into());
    }
    let mut imports = load(dir);
    let n = (1..).find(|n| !imports.iter().any(|i| i.name == format!("import-{n}")) && !dir.join(format!("import-{n}.wav")).exists()).unwrap_or(1);
    let name = format!("import-{n}");
    export::write_wav(&source_path(dir, &name), pcm, RATE).map_err(|e| e.to_string())?;
    let start = meeting_start(dir);
    let (offset_s, started_at, placed_by) = match place {
        Place::Time { started_at, .. } => (started_at - start, Some(started_at), "time"),
        Place::At(offset) => (offset, Some(start + offset), "hand"),
        Place::Sound => (0.0, None, "sound"),
    };
    imports.push(Import {
        name: name.clone(),
        offset_s,
        rate: 1.0,
        started_at,
        placed_by: placed_by.into(),
        confidence: None,
        matched: None,
        seconds: pcm.len() as f64 / RATE as f64,
    });
    save(dir, &imports)?;
    let mut meta: Meta = library::load_meta(dir);
    let label = label.trim();
    if !label.is_empty() {
        meta.names.insert(name.clone(), label.chars().take(60).collect());
        library::write_json(&dir.join("meta.json"), &meta)?;
    }
    match place {
        Place::Sound => return sync(dir, &name, false),
        Place::Time { refine: true, .. } => {
            if let Ok(found) = sync(dir, &name, true) {
                return Ok(found);
            }
        }
        _ => {}
    }
    Ok(load(dir).into_iter().find(|i| i.name == name).expect("just added"))
}

/// Places a track by hand: it starts `offset_s` into the meeting, at the meeting's own speed.
pub fn set_offset(dir: &Path, name: &str, offset_s: f64) -> Result<(), String> {
    let start = meeting_start(dir);
    let mut imports = load(dir);
    let import = imports.iter_mut().find(|i| i.name == name).ok_or("That track no longer exists.")?;
    import.offset_s = offset_s;
    import.started_at = Some(start + offset_s);
    import.rate = 1.0;
    import.confidence = None;
    import.matched = None;
    import.placed_by = "hand".into();
    save(dir, &imports)
}

/// Places a track by the time of day it started (Unix seconds).
pub fn set_start_time(dir: &Path, name: &str, started_at: f64) -> Result<(), String> {
    let start = meeting_start(dir);
    let mut imports = load(dir);
    let import = imports.iter_mut().find(|i| i.name == name).ok_or("That track no longer exists.")?;
    import.offset_s = started_at - start;
    import.started_at = Some(started_at);
    import.rate = 1.0;
    import.confidence = None;
    import.matched = None;
    import.placed_by = "time".into();
    save(dir, &imports)
}

/// Takes an imported track out of the meeting. Its lines leave the transcript when it's processed again.
pub fn remove(dir: &Path, name: &str) -> Result<(), String> {
    let mut imports = load(dir);
    imports.retain(|i| i.name != name);
    save(dir, &imports)?;
    let _ = std::fs::remove_file(source_path(dir, name));
    let _ = std::fs::remove_file(dir.join(format!("{name}.wav")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stv-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_are_placed_on_the_timeline() {
        let dir = temp("place");
        let pcm: Vec<i16> = (0..32_000).map(|i| (i % 100) as i16).collect();
        let import = add(&dir, "Phone", &pcm, Place::At(0.5)).unwrap();
        assert_eq!(import.name, "import-1");
        assert_eq!(library::load_meta(&dir).names["import-1"], "Phone");
        assert_eq!(placed(&dir, &import).unwrap().len(), 32_000 + 8_000);
        set_offset(&dir, "import-1", -1.0).unwrap();
        assert_eq!(placed(&dir, &load(&dir)[0]).unwrap().len(), 16_000);
        remove(&dir, "import-1").unwrap();
        assert!(load(&dir).is_empty() && !dir.join("import-1.track").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_time_of_day_places_a_track_against_the_meetings_start() {
        let dir = temp("time");
        library::write_json(&dir.join("meta.json"), &Meta { created: 1_000_000, ..Default::default() }).unwrap();
        let pcm = vec![100i16; 32_000];
        // No recorded tracks to match: it stays at its time of day.
        let import = add(&dir, "Their side", &pcm, Place::Time { started_at: 1_000_062.5, refine: true }).unwrap();
        assert_eq!((import.offset_s, import.placed_by.as_str(), import.started_at), (62.5, "time", Some(1_000_062.5)));
        set_start_time(&dir, "import-1", 999_990.0).unwrap();
        assert_eq!(load(&dir)[0].offset_s, -10.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fast_clock_is_stretched_onto_the_timeline() {
        let pcm: Vec<i16> = (0..16_000).map(|i| (i % 1000) as i16).collect();
        let out = place(&pcm, 1.0, 1.01);
        assert_eq!(out.len(), 16_000 + 16_160);
        assert!(out[..16_000].iter().all(|&s| s == 0));
        assert_eq!(out[16_000], pcm[0]);
    }
}
