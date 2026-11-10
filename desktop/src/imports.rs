//! Audio tracks added to a meeting after it was recorded, such as a phone's recording of the same room.
//! Each is kept as it came (16 kHz mono, decoded by the page) beside the meeting's own audio, with where it
//! starts on the meeting's timeline; processing then treats it like any other track. The start can be typed
//! in or found by matching the track against the others (`stv_core::sync`).

use crate::library::{self, Meta};
use crate::process::RATE;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stv_core::export;

const FILE: &str = "imports.json";
/// The imported audio as it came, before it's placed on the timeline. Not ".wav", so it isn't taken for a
/// processed track.
const EXTENSION: &str = "track";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Import {
    /// The stream key: "import-1" and so on.
    pub name: String,
    /// Where on the meeting's timeline it starts, in seconds; negative when it started first.
    pub offset_s: f64,
    /// How sure the last sync was, when it was synced: the best match over the next best.
    #[serde(default)]
    pub confidence: Option<f32>,
    pub seconds: f64,
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

/// The imported audio placed on the meeting's timeline: silence before it when it started later, trimmed
/// when it started first.
pub fn placed(dir: &Path, import: &Import) -> Result<Vec<i16>, String> {
    let pcm = read_pcm(&source_path(dir, &import.name))?;
    let shift = (import.offset_s * RATE as f64).round() as i64;
    Ok(if shift >= 0 {
        let mut out = vec![0i16; shift as usize];
        out.extend(pcm);
        out
    } else {
        pcm.get((-shift) as usize..).map(<[i16]>::to_vec).unwrap_or_default()
    })
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
        return Err("This meeting has no processed tracks to sync to yet.".into());
    }
    Ok(mix.into_iter().map(|m| m.clamp(i16::MIN as i32, i16::MAX as i32) as i16).collect())
}

/// Finds where an import starts by matching it against the recorded tracks, and keeps it.
pub fn sync(dir: &Path, name: &str) -> Result<Import, String> {
    let mut imports = load(dir);
    let import = imports.iter_mut().find(|i| i.name == name).ok_or("That track no longer exists.")?;
    let reference = reference(dir, name)?;
    let pcm = read_pcm(&source_path(dir, name))?;
    let (offset, confidence) =
        stv_core::sync::find_offset(&reference, &pcm, RATE).ok_or("The track or the meeting is too short to sync.")?;
    import.offset_s = offset;
    import.confidence = Some(confidence);
    let found = import.clone();
    save(dir, &imports)?;
    Ok(found)
}

/// Adds a track: `pcm` is 16 kHz mono. It's named by `label`, the name shown for it, and starts at
/// `offset_s`, or where syncing finds when `sync` is set.
pub fn add(dir: &Path, label: &str, pcm: &[i16], offset_s: f64, sync_it: bool) -> Result<Import, String> {
    if pcm.len() < RATE as usize {
        return Err("That recording is less than a second long.".into());
    }
    let mut imports = load(dir);
    let n = (1..).find(|n| !imports.iter().any(|i| i.name == format!("import-{n}")) && !dir.join(format!("import-{n}.wav")).exists()).unwrap_or(1);
    let name = format!("import-{n}");
    export::write_wav(&source_path(dir, &name), pcm, RATE).map_err(|e| e.to_string())?;
    imports.push(Import { name: name.clone(), offset_s, confidence: None, seconds: pcm.len() as f64 / RATE as f64 });
    save(dir, &imports)?;
    let mut meta: Meta = library::load_meta(dir);
    let label = label.trim();
    if !label.is_empty() {
        meta.names.insert(name.clone(), label.chars().take(60).collect());
        library::write_json(&dir.join("meta.json"), &meta)?;
    }
    if sync_it {
        return sync(dir, &name);
    }
    Ok(imports.pop().expect("just added"))
}

pub fn set_offset(dir: &Path, name: &str, offset_s: f64) -> Result<(), String> {
    let mut imports = load(dir);
    let import = imports.iter_mut().find(|i| i.name == name).ok_or("That track no longer exists.")?;
    import.offset_s = offset_s;
    import.confidence = None;
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

    #[test]
    fn imports_are_placed_on_the_timeline() {
        let dir = std::env::temp_dir().join(format!("stv-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pcm: Vec<i16> = (0..32_000).map(|i| (i % 100) as i16).collect();
        let import = add(&dir, "Phone", &pcm, 0.5, false).unwrap();
        assert_eq!(import.name, "import-1");
        assert_eq!(library::load_meta(&dir).names["import-1"], "Phone");
        assert_eq!(placed(&dir, &import).unwrap().len(), 32_000 + 8_000);
        set_offset(&dir, "import-1", -1.0).unwrap();
        assert_eq!(placed(&dir, &load(&dir)[0]).unwrap().len(), 16_000);
        remove(&dir, "import-1").unwrap();
        assert!(load(&dir).is_empty() && !dir.join("import-1.track").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
