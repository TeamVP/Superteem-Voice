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
    /// The other person's session it came from ("session-1794553387"), when a whole session was imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_session: Option<String>,
    /// The bookmarks and action items made in that session, on its own timeline (seconds into it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<library::Marker>,
}

impl Import {
    /// Where a moment of its own recording falls in the meeting.
    pub fn meeting_time(&self, own_s: f64) -> f64 {
        self.offset_s + own_s * self.rate
    }
}

/// The tracks recorded with `name`: all of one imported session's, which move together, or just itself.
fn group_of(imports: &[Import], name: &str) -> Vec<usize> {
    let from = imports.iter().find(|i| i.name == name).and_then(|i| i.from_session.clone());
    (0..imports.len())
        .filter(|&k| imports[k].name == name || (from.is_some() && imports[k].from_session == from))
        .collect()
}

/// Applies a new placing to `name` and the tracks recorded with it.
fn place_group(dir: &Path, name: &str, change: impl Fn(&mut Import)) -> Result<Import, String> {
    let mut imports = load(dir);
    let group = group_of(&imports, name);
    if group.is_empty() {
        return Err("That track no longer exists.".into());
    }
    for &k in &group {
        change(&mut imports[k]);
    }
    let found = imports.iter().find(|i| i.name == name).cloned().expect("in its group");
    save(dir, &imports)?;
    Ok(found)
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
    let fit = fit_of(dir, name, near)?;
    place_group(dir, name, |i| apply_fit(i, &fit))
}

fn apply_fit(import: &mut Import, fit: &stv_core::sync::Fit) {
    import.offset_s = fit.offset_s;
    import.rate = fit.rate;
    import.confidence = Some(fit.confidence);
    import.matched = Some([fit.matched, fit.tried]);
    import.placed_by = "sound".into();
}

/// Where an import's sound fits against the recorded tracks, without moving it.
fn fit_of(dir: &Path, name: &str, near: bool) -> Result<stv_core::sync::Fit, String> {
    let imports = load(dir);
    let import = imports.iter().find(|i| i.name == name).ok_or("That track no longer exists.")?;
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
    Ok(fit)
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
        from_session: None,
        markers: Vec::new(),
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
/// The tracks recorded with it move too.
pub fn set_offset(dir: &Path, name: &str, offset_s: f64) -> Result<(), String> {
    let start = meeting_start(dir);
    place_group(dir, name, |import| {
        import.offset_s = offset_s;
        import.started_at = Some(start + offset_s);
        import.rate = 1.0;
        import.confidence = None;
        import.matched = None;
        import.placed_by = "hand".into();
    })
    .map(|_| ())
}

/// The tracks recorded with `name`, with where each starts now.
pub fn group_offsets(dir: &Path, name: &str) -> Vec<(String, f64)> {
    let imports = load(dir);
    group_of(&imports, name).into_iter().map(|k| (imports[k].name.clone(), imports[k].offset_s)).collect()
}

/// Moves a track by hand to start `offset_s` into the meeting, keeping the speed matching found; the
/// tracks recorded with it move too.
pub fn move_to(dir: &Path, name: &str, offset_s: f64) -> Result<Import, String> {
    let start = meeting_start(dir);
    place_group(dir, name, |import| {
        import.offset_s = offset_s;
        import.started_at = Some(start + offset_s);
        import.confidence = None;
        import.matched = None;
        import.placed_by = "hand".into();
    })
}

/// Lines a track up by a moment marked in both recordings, such as a bookmark everyone made at once:
/// `own_s` into its own recording is `meeting_s` into the meeting. The tracks recorded with it move too, and
/// keep the speed matching their sound found.
pub fn line_up(dir: &Path, name: &str, own_s: f64, meeting_s: f64) -> Result<Import, String> {
    let start = meeting_start(dir);
    place_group(dir, name, |import| {
        import.offset_s = meeting_s - own_s * import.rate;
        import.started_at = Some(start + import.offset_s);
        import.confidence = None;
        import.matched = None;
        import.placed_by = "bookmark".into();
    })
}

/// Places a track by the time of day it started (Unix seconds).
pub fn set_start_time(dir: &Path, name: &str, started_at: f64) -> Result<(), String> {
    let start = meeting_start(dir);
    place_group(dir, name, |import| {
        import.offset_s = started_at - start;
        import.started_at = Some(started_at);
        import.rate = 1.0;
        import.confidence = None;
        import.matched = None;
        import.placed_by = "time".into();
    })
    .map(|_| ())
}

/// Louder than this somewhere, or the track heard nothing worth adding (about -56 dB).
const SILENT_PEAK: u16 = 50;

/// What importing another person's session added.
#[derive(Debug, Serialize)]
pub struct SessionImport {
    pub added: Vec<Import>,
    /// Their tracks left out because they heard nothing.
    pub silent: usize,
    /// When their recording started (Unix seconds), which placed it first.
    pub started_at: f64,
    /// Its sound matched this meeting's, so it's placed to the fraction of a second.
    pub matched: bool,
}

/// The folder in `path`, or under it, that holds a recorded session: journals, or processed tracks with
/// their details. A folder of several sessions is refused, as which one is meant isn't known.
fn session_root(path: &Path, depth: usize) -> Result<Option<PathBuf>, String> {
    let entries: Vec<PathBuf> = std::fs::read_dir(path).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    let has = |ext: &str| entries.iter().any(|p| p.is_file() && p.extension().is_some_and(|x| x == ext));
    if has("stvj") || (has("wav") && path.join(library::META).exists()) {
        return Ok(Some(path.to_path_buf()));
    }
    if depth == 0 {
        return Ok(None);
    }
    let mut found = Vec::new();
    for sub in entries.iter().filter(|p| p.is_dir()) {
        if let Some(root) = session_root(sub, depth - 1)? {
            found.push(root);
        }
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err("That holds more than one session. Choose the one recorded at this meeting.".into()),
    }
}

/// Unpacks a session's audio and details from a zip: nothing else is taken, and nothing outside `to`.
fn unzip(zip_path: &Path, to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "That isn't a zip file that can be read.")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let wanted = rel.file_name().is_some_and(|n| n == library::META) || rel.extension().is_some_and(|x| x == "wav" || x == "stvj");
        if entry.is_dir() || !wanted {
            continue;
        }
        let out = to.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Each of a session's tracks, from the start of its recording: its processed tracks, or its journals
/// lined up the way processing does when it was never processed.
fn session_tracks(root: &Path) -> Result<Vec<(String, Vec<i16>)>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    files.sort();
    let of = |ext: &str| files.iter().filter(|p| p.extension().is_some_and(|x| x == ext)).cloned().collect::<Vec<_>>();
    let stem = |p: &Path| p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let wavs = of("wav");
    if !wavs.is_empty() {
        return wavs.iter().map(|p| Ok((stem(p), read_pcm(p)?))).collect();
    }
    let mut names = Vec::new();
    let mut sources = Vec::new();
    for path in of("stvj") {
        let source = stv_core::pipeline::load_source(&path, RATE).map_err(|e| format!("{}: {e}", path.display()))?;
        if !source.pcm.is_empty() {
            names.push(stem(&path));
            sources.push(source);
        }
    }
    stv_core::pipeline::align_sources(&mut sources, RATE);
    Ok(names.into_iter().zip(sources.into_iter().map(|s| s.pcm)).collect())
}

/// When a session started (Unix seconds, the same the world over): from its details, else its folder's name.
fn session_start(root: &Path, fallback_name: &str) -> Option<u64> {
    let created = library::read_json::<Meta>(&root.join(library::META)).map(|m| m.created).unwrap_or(0);
    let from_name = |n: &str| n.strip_prefix("session-").and_then(|n| n.parse().ok());
    (created > 0)
        .then_some(created)
        .or_else(|| root.file_name().and_then(|n| n.to_str()).and_then(from_name))
        .or_else(|| from_name(fallback_name))
}

/// Adds another person's recording of this meeting: their whole Superteem Voice session, as its folder, a
/// folder holding it, or a zip of either. Each of their tracks that heard something becomes a track here,
/// named after who or what it recorded, with `owner` ("Lin") in front when it only names a device. They go
/// where their recording started by the clock, which is the same instant wherever they were, and are then
/// matched by sound; all of a session's tracks move together, as they were recorded together.
pub fn add_session(dir: &Path, path: &Path, owner: &str) -> Result<SessionImport, String> {
    let temp = std::env::temp_dir().join(format!("stv-session-import-{}-{}", std::process::id(), library::now()));
    let result = add_session_from(dir, path, owner, &temp);
    let _ = std::fs::remove_dir_all(&temp);
    result
}

fn add_session_from(dir: &Path, path: &Path, owner: &str, temp: &Path) -> Result<SessionImport, String> {
    let is_zip = path.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip"));
    let base = if is_zip {
        unzip(path, temp)?;
        temp.to_path_buf()
    } else if path.is_dir() {
        path.to_path_buf()
    } else {
        return Err("Choose a session folder or a .zip of one.".into());
    };
    let root = session_root(&base, 3)?.ok_or("No recording was found there. Choose a session folder (session-…) or a zip of one.")?;
    if root.canonicalize().ok() == dir.canonicalize().ok() {
        return Err("That's this meeting's own folder.".into());
    }
    let fallback = path.file_stem().and_then(|n| n.to_str()).unwrap_or_default();
    let start = session_start(&root, fallback).ok_or("When that recording started isn't known: it has no details and its folder isn't named session-….")?;
    let from = format!("session-{start}");
    if load(dir).iter().any(|i| i.from_session.as_deref() == Some(from.as_str())) {
        return Err("That session is already in this meeting.".into());
    }
    let theirs = if root.join(library::META).exists() { library::load_meta(&root) } else { Meta::default() };
    let tracks = session_tracks(&root)?;
    let total = tracks.len();
    let heard: Vec<(String, Vec<i16>)> = tracks.into_iter().filter(|(_, pcm)| pcm.iter().any(|s| s.unsigned_abs() > SILENT_PEAK)).collect();
    if heard.is_empty() {
        return Err("None of that session's tracks heard anything.".into());
    }
    let owner = owner.trim();
    let mut added = Vec::new();
    for (stream, pcm) in &heard {
        let label = if theirs.named(stream) {
            theirs.name(stream)
        } else {
            let device = theirs.track_name(stream).or_else(|| theirs.tracks.get(stream).map(|t| t.label.clone()));
            let what = device.unwrap_or_else(|| stream.clone());
            match (owner.is_empty(), heard.len()) {
                (true, _) => what,
                (false, 1) => owner.to_string(),
                (false, _) => format!("{owner} · {what}"),
            }
        };
        added.push(add(dir, &label, pcm, Place::Time { started_at: start as f64, refine: false })?.name);
    }
    // The track whose sound matches most surely places them all.
    let best = added.iter().filter_map(|name| fit_of(dir, name, true).ok()).max_by(|a, b| a.confidence.total_cmp(&b.confidence));
    let markers: Vec<library::Marker> = theirs.markers.iter().filter(|m| !m.removed).cloned().collect();
    let by_time = start as f64 - meeting_start(dir);
    let mut imports = load(dir);
    for import in imports.iter_mut().filter(|i| added.contains(&i.name)) {
        import.from_session = Some(from.clone());
        import.markers = markers.clone();
        match &best {
            Some(b) => apply_fit(import, b),
            None => {
                import.offset_s = by_time;
                import.rate = 1.0;
                import.confidence = None;
                import.matched = None;
                import.placed_by = "time".into();
            }
        }
    }
    save(dir, &imports)?;
    Ok(SessionImport {
        added: imports.into_iter().filter(|i| added.contains(&i.name)).collect(),
        silent: total - heard.len(),
        started_at: start as f64,
        matched: best.is_some(),
    })
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

    /// Bursts of noise a syllable long with pauses between, like speech, the same for the same seed.
    fn talk(seconds: usize, seed: u64) -> Vec<i16> {
        let mut x = seed;
        let mut rand = move || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) as u32
        };
        let mut out = Vec::new();
        while out.len() < seconds * RATE as usize {
            let burst = 1600 + (rand() % 4800) as usize;
            let loud = 2000 + (rand() % 8000) as i32;
            out.extend((0..burst).map(|_| ((rand() % 2001) as i32 - 1000) * loud / 1000).map(|s| s as i16));
            out.extend(std::iter::repeat_n(0, 800 + (rand() % 4000) as usize));
        }
        out.truncate(seconds * RATE as usize);
        out
    }

    #[test]
    fn another_persons_session_comes_in_matched_by_sound() {
        let mine = temp("mine");
        library::write_json(&mine.join("meta.json"), &Meta { created: 1_000_000, ..Default::default() }).unwrap();
        let meeting = talk(90, 7);
        export::write_wav(&mine.join("input1.wav"), &meeting, RATE).unwrap();

        // Their recording really started 4.2 s after mine, but their clock says 3 s.
        let outer = temp("theirs");
        let theirs = outer.join("session-1000003");
        std::fs::create_dir_all(&theirs).unwrap();
        let mut meta = Meta { created: 1_000_003, ..Default::default() };
        meta.names.insert("input1".into(), "Lin".into());
        meta.markers.push(library::Marker { at_s: 10.0, text: "Sync".into(), kind: String::new(), removed: false });
        library::write_json(&theirs.join("meta.json"), &meta).unwrap();
        export::write_wav(&theirs.join("input1.wav"), &meeting[(4.2 * RATE as f64) as usize..], RATE).unwrap();
        export::write_wav(&theirs.join("input2.wav"), &vec![3i16; 60 * RATE as usize], RATE).unwrap();

        // Given the folder holding theirs, as unzipped.
        let done = add_session(&mine, &outer, "").unwrap();
        assert_eq!((done.added.len(), done.silent, done.started_at, done.matched), (1, 1, 1_000_003.0, true));
        let import = &done.added[0];
        assert!((import.offset_s - 4.2).abs() < 0.05, "{}", import.offset_s);
        assert_eq!(import.from_session.as_deref(), Some("session-1000003"));
        assert_eq!(library::load_meta(&mine).names[&import.name], "Lin");
        assert!(add_session(&mine, &theirs, "").unwrap_err().contains("already"));
        // Their bookmarks come too, and can line them up: their "Sync" at 10 s is 14 s into the meeting.
        assert_eq!(import.markers.len(), 1);
        assert!((import.meeting_time(10.0) - 14.2).abs() < 0.06);
        let moved = line_up(&mine, &import.name, 10.0, 14.0).unwrap();
        assert!((moved.meeting_time(10.0) - 14.0).abs() < 1e-9 && moved.placed_by == "bookmark");

        // The same as a zip, named by its owner when it only names a device.
        remove(&mine, &import.name).unwrap();
        library::write_json(&theirs.join("meta.json"), &Meta { created: 1_000_003, ..Default::default() }).unwrap();
        let zipped = outer.join("theirs.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zipped).unwrap());
        for file in ["meta.json", "input1.wav", "input2.wav"] {
            zip.start_file(format!("session-1000003/{file}"), zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zip, &std::fs::read(theirs.join(file)).unwrap()).unwrap();
        }
        zip.finish().unwrap();
        let done = add_session(&mine, &zipped, "Lin").unwrap();
        assert_eq!(library::load_meta(&mine).names[&done.added[0].name], "Lin");
        assert!((done.added[0].offset_s - 4.2).abs() < 0.05);

        assert!(add_session(&mine, &mine, "").is_err(), "not its own folder");
        let _ = std::fs::remove_dir_all(&mine);
        let _ = std::fs::remove_dir_all(&outer);
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
