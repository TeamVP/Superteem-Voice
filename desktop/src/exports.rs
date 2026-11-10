//! Exporting a meeting or a note to files for other apps: a transcript, or an edit package.
//!
//! An edit package is a folder an audio or video editor can open: every track's audio, the transcript mix
//! rendered to one file, and the same mix as edit decisions, with the transcript and bookmarks carried
//! along as comments and markers:
//!
//! - `<title>.edl`: CMX3600, one event per stretch a track plays in the transcript mix, the words said
//!   there as comments, bookmarks as locators. Opens in most editors.
//! - `<title>.xml`: Final Cut Pro 7 XML, which Premiere Pro and DaVinci Resolve import: a track per
//!   microphone cut to the transcript mix, and each line and bookmark as a marker with its text.
//! - `<title>.rpp`: a REAPER project with every track whole and the transcript mix as volume envelopes, so
//!   it can be undone.
//!
//! The transcript mix is what the meeting page plays by default: each track muted where the script takes
//! its words from another microphone.

use crate::keyphrases;
use crate::library::{self, Marker, Meta, Segment};
use crate::notes;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use stv_core::export::{self, ExportTrack, Line};

/// Timecode frame rate for the edit decision lists.
const FPS: f64 = 25.0;
/// A phrase heard this close to a marker made while recording is the same moment.
const SAME_MARK_S: f64 = 5.0;
/// Each mute in the rendered mix fades in and out over this long, so it doesn't click.
const FADE_S: f64 = 0.01;
/// Muted, for the REAPER envelope.
const MUTED_DB: f32 = -150.0;

/// Where exports are written: a folder beside the meetings.
pub fn dir() -> PathBuf {
    library::root().join("Exports")
}

/// A name that is safe as a file name on every system.
fn file_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.,()'".contains(c) { c } else { '-' })
        .collect();
    let clean = clean.trim().trim_matches('.').trim();
    if clean.is_empty() { "Export".into() } else { clean.chars().take(80).collect() }
}

/// A path in the exports folder that doesn't overwrite an earlier export.
fn fresh(name: &str, extension: &str) -> PathBuf {
    let base = dir();
    let with = |n: &str| if extension.is_empty() { base.join(n) } else { base.join(format!("{n}.{extension}")) };
    let mut path = with(name);
    let mut n = 2;
    while path.exists() {
        path = with(&format!("{name} ({n})"));
        n += 1;
    }
    path
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------- Meetings

/// A moment marked in a meeting, as the meeting page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub at_s: f64,
    pub action: bool,
    pub note: String,
}

fn is_action(m: &Marker) -> bool {
    m.kind == "action" || (m.kind.is_empty() && m.text == "Action item")
}

/// The markers made while recording, and "Bookmark that" or "Action item" heard in the recording, without
/// those the person deleted.
pub fn marks(meta: &Meta, heard: &[keyphrases::Heard]) -> Vec<Mark> {
    let mut out: Vec<Mark> = meta
        .markers
        .iter()
        .filter(|m| !m.removed)
        .map(|m| Mark { at_s: m.at_s, action: is_action(m), note: if m.text == "Action item" { String::new() } else { m.text.clone() } })
        .collect();
    for h in heard.iter().filter(|h| !h.rejected && matches!(h.phrase.as_str(), "bookmark" | "action")) {
        let action = h.phrase == "action";
        let at_s = (h.start_s * 10.0).round() / 10.0;
        if !meta.markers.iter().any(|m| is_action(m) == action && (m.at_s - at_s).abs() < SAME_MARK_S) {
            out.push(Mark { at_s, action, note: String::new() });
        }
    }
    out.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    out
}

fn spans(s: &Segment) -> Vec<(f64, f64)> {
    s.spans()
}

/// Joins overlapping spans and sorts them.
fn merged(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.retain(|(a, b)| b > a);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `spans` with `holes` taken out of them.
fn minus(spans: &[(f64, f64)], holes: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(a, b) in spans {
        let mut from = a;
        for &(h0, h1) in holes.iter().filter(|(h0, h1)| *h1 > a && *h0 < b) {
            if h0 > from {
                out.push((from, h0));
            }
            from = from.max(h1);
        }
        if from < b {
            out.push((from, b));
        }
    }
    out
}

/// Where a track is muted in the transcript mix: greyed out there (a doubled-up clip, or a line the script
/// takes from another microphone that this one heard too), and not one of the script's own clips.
pub fn quiet_spans(segments: &[Segment], stream: &str) -> Vec<(f64, f64)> {
    let greyed: Vec<(f64, f64)> = segments
        .iter()
        .flat_map(|s| {
            if !s.in_script() && s.stream == stream {
                spans(s)
            } else if s.in_script() && s.stream != stream && s.also_on.iter().any(|o| o == stream) {
                vec![(s.start_s, s.end_s)]
            } else {
                Vec::new()
            }
        })
        .collect();
    let used: Vec<(f64, f64)> = segments.iter().filter(|s| s.in_script() && s.stream == stream).flat_map(spans).collect();
    minus(&merged(greyed), &merged(used))
}

/// Where a track plays in the transcript mix, over the whole meeting.
pub fn playing_spans(segments: &[Segment], stream: &str, duration_s: f64) -> Vec<(f64, f64)> {
    minus(&[(0.0, duration_s)], &quiet_spans(segments, stream))
}

/// The tracks summed, each muted where the transcript mix mutes it, with short fades at each mute.
pub fn render_mix(tracks: &[(Vec<i16>, Vec<(f64, f64)>)], rate: u32) -> Vec<i16> {
    let len = tracks.iter().map(|(pcm, _)| pcm.len()).max().unwrap_or(0);
    let fade = (FADE_S * rate as f64).max(1.0);
    let mut mix = vec![0f32; len];
    for (pcm, quiet) in tracks {
        let mut gain = vec![1f32; pcm.len()];
        for &(a, b) in quiet {
            let (a, b) = ((a * rate as f64) as usize, ((b * rate as f64) as usize).min(pcm.len()));
            for (i, g) in gain.iter_mut().enumerate().take(b).skip(a) {
                // Down to silence over the fade at each end of the muted stretch.
                let edge = (i - a).min(b - 1 - i) as f64;
                *g = g.min((1.0 - (edge + 1.0) / fade).max(0.0) as f32);
            }
        }
        for (i, (&s, g)) in pcm.iter().zip(&gain).enumerate() {
            mix[i] += s as f32 * g;
        }
    }
    mix.into_iter().map(|v| v.round().clamp(-32768.0, 32767.0) as i16).collect()
}

/// 16-bit mono PCM from a WAV written by this app.
fn read_wav(path: &Path) -> Result<Vec<i16>, String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let at = data.windows(4).position(|w| w == b"data").ok_or_else(|| format!("{} is not a WAV file", path.display()))? + 8;
    Ok(data[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// Timecode at FPS, hours:minutes:seconds:frames.
fn timecode(seconds: f64) -> String {
    let frames = (seconds.max(0.0) * FPS).round() as u64;
    let fps = FPS as u64;
    format!("{:02}:{:02}:{:02}:{:02}", frames / (fps * 3600), frames / (fps * 60) % 60, frames / fps % 60, frames % fps)
}

fn frames(seconds: f64) -> u64 {
    (seconds.max(0.0) * FPS).round() as u64
}

/// One microphone's track in an edit package.
pub struct PackageTrack {
    pub stream: String,
    pub name: String,
    /// The audio file's name in the package's audio folder.
    pub file: String,
    pub playing: Vec<(f64, f64)>,
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// CMX3600: an event per stretch each track plays in the transcript mix, with what was said there as
/// comments, and the bookmarks as locators. Tracks past the fourth have no audio channel in CMX3600.
pub fn to_edl(title: &str, tracks: &[PackageTrack], lines: &[(Line, String)], marks: &[Mark]) -> String {
    let mut out = format!("TITLE: {}\nFCM: NON-DROP FRAME\n\n", one_line(title));
    let mut events: Vec<(f64, f64, usize)> = tracks.iter().enumerate().flat_map(|(i, t)| t.playing.iter().map(move |&(a, b)| (a, b, i))).collect();
    events.retain(|&(a, b, i)| i < 4 && frames(b) > frames(a));
    events.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.2.cmp(&y.2)));
    // Each bookmark is a locator on the first event playing at its time, or the last one before it.
    let home = |at: f64| events.iter().position(|e| at >= e.0 && at < e.1).or_else(|| events.iter().rposition(|e| e.0 <= at)).unwrap_or(0);
    for (n, &(a, b, i)) in events.iter().enumerate() {
        let channel = ["A", "A2", "A3", "A4"][i];
        let reel = format!("TRACK{}", i + 1);
        let (tc_in, tc_out) = (timecode(a), timecode(b));
        let _ = writeln!(out, "{:03}  {reel:<8} {channel:<5} C        {tc_in} {tc_out} {tc_in} {tc_out}", n + 1);
        let _ = writeln!(out, "* FROM CLIP NAME: {}", tracks[i].file);
        for (line, _) in lines.iter().filter(|(l, s)| *s == tracks[i].stream && l.start_s < b && l.end_s > a) {
            let _ = writeln!(out, "* {} {}: {}", timecode(line.start_s), one_line(&line.speaker), one_line(&line.text));
        }
        for m in marks.iter().filter(|m| home(m.at_s) == n) {
            let what = if m.action { "ACTION ITEM" } else { "BOOKMARK" };
            let note = if m.note.is_empty() { String::new() } else { format!(": {}", one_line(&m.note)) };
            let _ = writeln!(out, "* LOC: {} WHITE   {what}{note}", timecode(m.at_s));
        }
        out.push('\n');
    }
    if tracks.len() > 4 {
        let _ = writeln!(out, "* Tracks past the fourth are in the XML and REAPER files only.");
    }
    out
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A file URL for FCP XML, which needs absolute paths.
fn path_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let encoded: String = p
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "/-_.:~".contains(c) { c.to_string() } else { c.to_string().bytes().map(|b| format!("%{b:02X}")).collect() })
        .collect();
    if encoded.starts_with('/') { format!("file://localhost{encoded}") } else { format!("file://localhost/{encoded}") }
}

/// Final Cut Pro 7 XML, as Premiere Pro and DaVinci Resolve import it: a sequence with a track per
/// microphone cut to the transcript mix, and each line and bookmark as a marker with its text.
pub fn to_fcp_xml(title: &str, duration_s: f64, rate: u32, audio_dir: &Path, tracks: &[PackageTrack], lines: &[(Line, String)], marks: &[Mark]) -> String {
    let total = frames(duration_s);
    let timebase = FPS as u64;
    let rate_xml = format!("<rate><timebase>{timebase}</timebase><ntsc>FALSE</ntsc></rate>");
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"4\">\n");
    let _ = writeln!(out, "<sequence id=\"sequence-1\"><name>{}</name><duration>{total}</duration>{rate_xml}", xml(title));
    let _ = writeln!(out, "<timecode>{rate_xml}<string>00:00:00:00</string><frame>0</frame><displayformat>NDF</displayformat></timecode>");
    out += "<media><audio>\n";
    let _ = writeln!(out, "<format><samplecharacteristics><depth>16</depth><samplerate>{rate}</samplerate></samplecharacteristics></format>");
    for (i, t) in tracks.iter().enumerate() {
        out += "<track>\n";
        for (k, &(a, b)) in t.playing.iter().enumerate() {
            let (start, end) = (frames(a), frames(b));
            if end <= start {
                continue;
            }
            // The file is described once, on its first clip; later clips refer to it by id.
            let file = if k == 0 {
                format!(
                    "<file id=\"file-{n}\"><name>{name}</name><pathurl>{url}</pathurl>{rate_xml}<duration>{total}</duration><media><audio><samplecharacteristics><depth>16</depth><samplerate>{rate}</samplerate></samplecharacteristics><channelcount>1</channelcount></audio></media></file>",
                    n = i + 1,
                    name = xml(&t.file),
                    url = xml(&path_url(&audio_dir.join(&t.file)))
                )
            } else {
                format!("<file id=\"file-{}\"/>", i + 1)
            };
            let _ = writeln!(
                out,
                "<clipitem id=\"clip-{}-{}\"><name>{}</name><enabled>TRUE</enabled><duration>{total}</duration>{rate_xml}<start>{start}</start><end>{end}</end><in>{start}</in><out>{end}</out>{file}<sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack></clipitem>",
                i + 1,
                k + 1,
                xml(&t.name)
            );
        }
        out += "</track>\n";
    }
    out += "</audio></media>\n";
    for (line, _) in lines {
        let _ = writeln!(
            out,
            "<marker><name>{}</name><comment>{}</comment><in>{}</in><out>{}</out></marker>",
            xml(&line.speaker),
            xml(&one_line(&line.text)),
            frames(line.start_s),
            frames(line.end_s).max(frames(line.start_s) + 1)
        );
    }
    for m in marks {
        let what = if m.action { "Action item" } else { "Bookmark" };
        let _ = writeln!(out, "<marker><name>{what}</name><comment>{}</comment><in>{}</in><out>-1</out></marker>", xml(&one_line(&m.note)), frames(m.at_s));
    }
    out += "</sequence>\n</xmeml>\n";
    out
}

/// What the person asked for when exporting a meeting.
pub struct MeetingExport<'a> {
    pub dir: &'a Path,
    /// The title shown in the app, which names the files.
    pub title: &'a str,
    pub meta: &'a Meta,
    pub segments: &'a [Segment],
    pub heard: &'a [keyphrases::Heard],
}

fn script_lines(meta: &Meta, segments: &[Segment]) -> Vec<(Line, String)> {
    segments
        .iter()
        .filter(|s| s.in_script() && !s.text.trim().is_empty())
        .map(|s| (Line { speaker: meta.name(&s.speaker), start_s: s.start_s, end_s: s.end_s, text: s.text.clone() }, s.stream.clone()))
        .collect()
}

/// The transcript, with bookmarks and action items in their places for the text formats.
fn meeting_transcript(m: &MeetingExport, format: &str) -> Result<String, String> {
    let lines: Vec<Line> = script_lines(m.meta, m.segments).into_iter().map(|(l, _)| l).collect();
    let marks = marks(m.meta, m.heard);
    Ok(match format {
        "txt" | "md" => {
            // Markers go in as lines of their own, before the first line that starts after them.
            let mut with: Vec<Line> = lines.clone();
            for k in &marks {
                let what = if k.action { "Action item" } else { "Bookmark" };
                let speaker = if format == "md" { format!("{} {what}", if k.action { "☑" } else { "🔖" }) } else { format!("[{what}]") };
                with.push(Line { speaker, start_s: k.at_s, end_s: k.at_s, text: k.note.clone() });
            }
            with.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
            if format == "txt" { format!("{}\n\n{}", m.title, export::to_text(&with)) } else { export::to_markdown(m.title, &with) }
        }
        "srt" => export::to_srt(&lines),
        "vtt" => export::to_vtt(&lines),
        "json" => export::to_json(&lines),
        _ => return Err(format!("Unknown export format: {format}")),
    })
}

/// Writes a meeting's transcript or edit package to the exports folder and returns where.
pub fn export_meeting(m: &MeetingExport, format: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let name = file_name(m.title);
    if format != "package" {
        let path = fresh(&name, format);
        write(&path, &meeting_transcript(m, format)?)?;
        return Ok(path);
    }

    let folder = fresh(&format!("{name} - edit package"), "");
    let audio = folder.join("audio");
    fs::create_dir_all(&audio).map_err(|e| e.to_string())?;
    let mut streams: Vec<String> = fs::read_dir(m.dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    streams.sort();
    if streams.is_empty() {
        return Err("This meeting has no audio to export.".into());
    }
    let rate = crate::process::RATE;
    let mut tracks = Vec::new();
    let mut pcm_tracks = Vec::new();
    let mut duration_s = m.meta.duration_s;
    let mut used_names: Vec<String> = Vec::new();
    for (i, stream) in streams.iter().enumerate() {
        let pcm = read_wav(&m.dir.join(format!("{stream}.wav")))?;
        duration_s = duration_s.max(pcm.len() as f64 / rate as f64);
        let name = m.meta.name(stream);
        // Two microphones for one person still need two file names.
        let mut file = format!("{} - {}.wav", i + 1, file_name(&name));
        if used_names.contains(&file) {
            file = format!("{} - {} ({stream}).wav", i + 1, file_name(&name));
        }
        used_names.push(file.clone());
        fs::copy(m.dir.join(format!("{stream}.wav")), audio.join(&file)).map_err(|e| e.to_string())?;
        let quiet = quiet_spans(m.segments, stream);
        tracks.push(PackageTrack { stream: stream.clone(), name, file, playing: Vec::new() });
        pcm_tracks.push((pcm, quiet));
    }
    for t in &mut tracks {
        t.playing = playing_spans(m.segments, &t.stream, duration_s);
    }
    let mix_file = "Transcript mix.wav";
    export::write_wav(&audio.join(mix_file), &render_mix(&pcm_tracks, rate), rate).map_err(|e| e.to_string())?;

    let lines = script_lines(m.meta, m.segments);
    let marks = marks(m.meta, m.heard);
    write(&folder.join(format!("{name}.edl")), &to_edl(m.title, &tracks, &lines, &marks))?;
    write(&folder.join(format!("{name}.xml")), &to_fcp_xml(m.title, duration_s, rate, &audio, &tracks, &lines, &marks))?;
    let reaper: Vec<ExportTrack> = tracks
        .iter()
        .zip(&pcm_tracks)
        .map(|(t, (_, quiet))| ExportTrack {
            name: t.name.clone(),
            file: format!("audio/{}", t.file),
            duration_s,
            gains: quiet.iter().map(|&(a, b)| (a, b, MUTED_DB)).collect(),
        })
        .collect();
    write(&folder.join(format!("{name}.rpp")), &export::to_reaper(&reaper, rate))?;
    for format in ["txt", "srt"] {
        write(&folder.join(format!("{name}.{format}")), &meeting_transcript(m, format)?)?;
    }
    write(&folder.join("README.txt"), &package_readme(m.title, &name, &tracks, mix_file))?;
    Ok(folder)
}

fn package_readme(title: &str, name: &str, tracks: &[PackageTrack], mix_file: &str) -> String {
    let list: String = tracks.iter().map(|t| format!("  audio/{}  ({})\n", t.file, t.name)).collect();
    format!(
        "{title}\nExported from Voice by Superteem.\n\n\
         Audio, 16 kHz mono WAV, all starting at the same moment:\n{list}  audio/{mix_file}  (the transcript mix: every microphone, each muted where the transcript takes its words from another one)\n\n\
         Edit decisions for the transcript mix:\n\
         \x20 {name}.xml  Premiere Pro and DaVinci Resolve (File > Import). A track per microphone, cut to the transcript mix; every line of the transcript and every bookmark is a marker with its text.\n\
         \x20 {name}.edl  CMX3600 EDL for other editors. The words said in each event are comments; bookmarks are locators. Relink the reels TRACK1, TRACK2... to the files in audio/.\n\
         \x20 {name}.rpp  REAPER. Every track whole, with the transcript mix as volume envelopes you can change.\n\n\
         Transcript: {name}.txt, and {name}.srt for subtitles.\n"
    )
}

// ---------- Notes

/// Writes a note as text or Markdown, or as a folder with any dictation audio still kept for it.
pub fn export_note(id: &str, format: &str) -> Result<PathBuf, String> {
    let note = notes::load(id)?;
    let title = notes::display_title(&note);
    let name = file_name(&title);
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let body = note.body.trim_end();
    let text = |format: &str| match format {
        "md" => format!("# {title}\n\n{body}\n"),
        _ => format!("{title}\n\n{body}\n"),
    };
    match format {
        "txt" | "md" => {
            let path = fresh(&name, format);
            write(&path, &text(format))?;
            Ok(path)
        }
        "package" => {
            let audio = notes::kept_audio(id);
            if audio.is_empty() {
                return Err("No dictation audio is kept for this note. Turn on “Keep audio” to keep it for a week.".into());
            }
            let folder = fresh(&format!("{name} - with audio"), "");
            fs::create_dir_all(folder.join("audio")).map_err(|e| e.to_string())?;
            write(&folder.join(format!("{name}.md")), &text("md"))?;
            for (i, file) in audio.iter().enumerate() {
                fs::copy(file, folder.join("audio").join(format!("Dictation {}.wav", i + 1))).map_err(|e| e.to_string())?;
            }
            Ok(folder)
        }
        _ => Err(format!("Unknown export format: {format}")),
    }
}

/// Shows an exported file or folder in File Explorer or Finder.
pub fn reveal(path: &Path) -> Result<(), String> {
    if !path.starts_with(dir()) {
        return Err("Only exports can be shown.".into());
    }
    crate::shell::reveal(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: usize, stream: &str, start_s: f64, end_s: f64, echo_of: Option<usize>, also_on: &[&str]) -> Segment {
        Segment {
            id,
            stream: stream.into(),
            speaker: stream.into(),
            start_s,
            end_s,
            text: format!("line {id}"),
            echo_of,
            also_on: also_on.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn meeting() -> Vec<Segment> {
        vec![
            seg(0, "input1", 1.0, 5.0, None, &["input2"]),
            seg(1, "input2", 2.0, 3.0, Some(0), &[]),
            seg(2, "input2", 6.0, 8.0, None, &[]),
            seg(3, "input1", 6.5, 7.0, Some(2), &[]),
        ]
    }

    #[test]
    fn each_track_is_quiet_where_another_has_the_words() {
        assert_eq!(quiet_spans(&meeting(), "input2"), [(1.0, 5.0)]);
        assert_eq!(quiet_spans(&meeting(), "input1"), [(6.5, 7.0)]);
        assert_eq!(playing_spans(&meeting(), "input1", 10.0), [(0.0, 6.5), (7.0, 10.0)]);
    }

    #[test]
    fn the_mix_mutes_with_short_fades() {
        let rate = 1000;
        let tone = vec![1000i16; 3000];
        let mix = render_mix(&[(tone.clone(), vec![(1.0, 2.0)]), (tone, vec![])], rate);
        assert_eq!(mix[500], 2000);
        assert_eq!(mix[1500], 1000);
        // Fading, not a step, at the edge of the mute.
        assert!(mix[1002] > 1000 && mix[1002] < 2000, "{}", mix[1002]);
    }

    #[test]
    fn the_edl_is_cmx3600_with_the_words_as_comments() {
        let tracks = vec![
            PackageTrack { stream: "input1".into(), name: "Ann".into(), file: "1 - Ann.wav".into(), playing: vec![(0.0, 6.5)] },
            PackageTrack { stream: "input2".into(), name: "Bo".into(), file: "2 - Bo.wav".into(), playing: vec![(5.0, 10.0)] },
        ];
        let lines = vec![(Line { speaker: "Ann".into(), start_s: 1.0, end_s: 5.0, text: "Hello\nthere".into() }, "input1".to_string())];
        let marks = vec![Mark { at_s: 2.0, action: false, note: "Pricing".into() }];
        let edl = to_edl("Standup", &tracks, &lines, &marks);
        assert!(edl.starts_with("TITLE: Standup\nFCM: NON-DROP FRAME\n"));
        assert!(edl.contains("001  TRACK1   A     C        00:00:00:00 00:00:06:13 00:00:00:00 00:00:06:13\n"), "{edl}");
        assert!(edl.contains("002  TRACK2   A2    C        00:00:05:00 00:00:10:00"), "{edl}");
        assert!(edl.contains("* 00:00:01:00 Ann: Hello there\n"));
        assert!(edl.contains("* LOC: 00:00:02:00 WHITE   BOOKMARK: Pricing\n"));
    }

    #[test]
    fn the_xml_has_a_clip_per_playing_stretch_and_markers() {
        let tracks = vec![PackageTrack { stream: "input1".into(), name: "Ann & co".into(), file: "1 - Ann.wav".into(), playing: vec![(0.0, 2.0), (3.0, 4.0)] }];
        let lines = vec![(Line { speaker: "Ann".into(), start_s: 1.0, end_s: 2.0, text: "a < b".into() }, "input1".to_string())];
        let xml = to_fcp_xml("T", 4.0, 16_000, Path::new("C:/Exports/T/audio"), &tracks, &lines, &[]);
        assert_eq!(xml.matches("<clipitem").count(), 2);
        assert!(xml.contains("<start>75</start><end>100</end>"));
        assert!(xml.contains("<pathurl>file://localhost/C:/Exports/T/audio/1%20-%20Ann.wav</pathurl>"), "{xml}");
        assert!(xml.contains("<name>Ann &amp; co</name>"));
        assert!(xml.contains("<comment>a &lt; b</comment>"));
        assert_eq!(xml.matches("<file id=\"file-1\">").count(), 1);
    }

    /// Exports a processed meeting folder (STV_EXPORT_FROM) as every format into STV_EXPORT_TO.
    #[test]
    #[ignore]
    fn an_example_meeting_exports() {
        let from = PathBuf::from(std::env::var("STV_EXPORT_FROM").unwrap());
        let to = PathBuf::from(std::env::var("STV_EXPORT_TO").unwrap());
        library::init(to.clone(), to.join("data"));
        let (meta, segments, heard) = (library::load_meta(&from), library::load_segments(&from), keyphrases::load_session(&from));
        let request = MeetingExport { dir: &from, title: "1- Tuesday 10 Nov 2026", meta: &meta, segments: &segments, heard: &heard };
        for format in ["txt", "md", "srt", "vtt", "package"] {
            println!("{}", export_meeting(&request, format).unwrap().display());
        }
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("1- Tuesday 10 Nov 2026"), "1- Tuesday 10 Nov 2026");
        assert_eq!(file_name("a/b:c?"), "a-b-c-");
        assert_eq!(file_name("  "), "Export");
    }
}
