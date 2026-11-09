//! Transcript, WAV, REAPER and OpenTimelineIO export.

use serde::Serialize;
use serde_json::json;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub speaker: String,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

pub fn write_wav(path: &Path, pcm: &[i16], rate: u32) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let bytes = (pcm.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + bytes).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 2).to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&bytes.to_le_bytes())?;
    for s in pcm {
        w.write_all(&s.to_le_bytes())?;
    }
    w.flush()
}

fn clock(seconds: f64, millis_sep: char) -> String {
    let ms = (seconds * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, millis_sep, ms % 1000)
}

pub fn to_text(lines: &[Line]) -> String {
    lines.iter().map(|l| format!("[{}] {}: {}\n", clock(l.start_s, '.'), l.speaker, l.text)).collect()
}

pub fn to_markdown(title: &str, lines: &[Line]) -> String {
    let body: String =
        lines.iter().map(|l| format!("**{}** `{}`\n\n{}\n\n", l.speaker, clock(l.start_s, '.'), l.text)).collect();
    format!("# {title}\n\n{body}")
}

pub fn to_srt(lines: &[Line]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            format!("{}\n{} --> {}\n{}: {}\n\n", i + 1, clock(l.start_s, ','), clock(l.end_s, ','), l.speaker, l.text)
        })
        .collect()
}

/// WebVTT subtitles, with the speaker as the cue's voice.
pub fn to_vtt(lines: &[Line]) -> String {
    let cues: String = lines
        .iter()
        .map(|l| format!("{} --> {}
<v {}>{}

", clock(l.start_s, '.'), clock(l.end_s, '.'), l.speaker.replace('>', ""), l.text))
        .collect();
    format!("WEBVTT

{cues}")
}

pub fn to_json(lines: &[Line]) -> String {
    serde_json::to_string_pretty(lines).expect("lines always serialise")
}

#[derive(Debug, Clone)]
pub struct ExportTrack {
    pub name: String,
    pub file: String,
    pub duration_s: f64,
    /// (start_s, end_s, gain_db), sorted and not overlapping.
    pub gains: Vec<(f64, f64, f32)>,
}

fn quote_rpp(s: &str) -> String {
    s.replace('"', "'")
}

/// Bleed gating becomes volume envelope points, so an editor can undo it without touching the audio.
pub fn to_reaper(tracks: &[ExportTrack], rate: u32) -> String {
    let mut out = format!("<REAPER_PROJECT 0.1 \"6.0\" 0\n  SAMPLERATE {rate} 0 0\n");
    for t in tracks {
        out += &format!("  <TRACK\n    NAME \"{}\"\n    <VOLENV2\n      ACT 1 -1\n      VIS 1 1 1\n      ARM 0\n", quote_rpp(&t.name));
        out += "      PT 0 1 1\n";
        for &(start, end, db) in &t.gains {
            out += &format!("      PT {start:.6} {:.6} 1\n", 10f64.powf(db as f64 / 20.0));
            out += &format!("      PT {end:.6} 1 1\n");
        }
        out += "    >\n";
        out += &format!(
            "    <ITEM\n      POSITION 0\n      LENGTH {:.6}\n      NAME \"{}\"\n      <SOURCE WAVE\n        FILE \"{}\"\n      >\n    >\n  >\n",
            t.duration_s,
            quote_rpp(&t.name),
            quote_rpp(&t.file)
        );
    }
    out + ">\n"
}

/// Gains are kept as metadata because OTIO has no standard volume automation.
pub fn to_otio(name: &str, tracks: &[ExportTrack], rate: u32) -> String {
    let rational = |seconds: f64| {
        json!({"OTIO_SCHEMA": "RationalTime.1", "rate": rate as f64, "value": (seconds * rate as f64).round()})
    };
    let children: Vec<_> = tracks
        .iter()
        .map(|t| {
            json!({
                "OTIO_SCHEMA": "Track.1",
                "name": t.name,
                "kind": "Audio",
                "metadata": {},
                "effects": [],
                "markers": [],
                "source_range": null,
                "children": [{
                    "OTIO_SCHEMA": "Clip.2",
                    "name": t.name,
                    "metadata": {"superteem": {"gain_ranges": t.gains}},
                    "effects": [],
                    "markers": [],
                    "source_range": {
                        "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": rational(0.0),
                        "duration": rational(t.duration_s)
                    },
                    "active_media_reference_key": "DEFAULT_MEDIA",
                    "media_references": {"DEFAULT_MEDIA": {
                        "OTIO_SCHEMA": "ExternalReference.1",
                        "target_url": t.file,
                        "metadata": {},
                        "available_range": null
                    }}
                }]
            })
        })
        .collect();
    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": name,
        "metadata": {},
        "global_start_time": null,
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "metadata": {},
            "effects": [],
            "markers": [],
            "source_range": null,
            "children": children
        }
    });
    serde_json::to_string_pretty(&timeline).expect("timeline always serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<Line> {
        vec![Line { speaker: "Alice".into(), start_s: 3661.5, end_s: 3662.25, text: "Hello".into() }]
    }

    fn tracks() -> Vec<ExportTrack> {
        vec![ExportTrack {
            name: "Alice \"Pin\"".into(),
            file: "audio/alice.wav".into(),
            duration_s: 10.0,
            gains: vec![(1.0, 2.0, -20.0)],
        }]
    }

    #[test]
    fn srt_uses_comma_timestamps() {
        assert_eq!(to_srt(&lines()), "1\n01:01:01,500 --> 01:01:02,250\nAlice: Hello\n\n");
    }

    #[test]
    fn vtt_names_the_voice() {
        assert_eq!(to_vtt(&lines()), "WEBVTT

01:01:01.500 --> 01:01:02.250
<v Alice>Hello

");
    }

    #[test]
    fn text_and_markdown_name_the_speaker() {
        assert_eq!(to_text(&lines()), "[01:01:01.500] Alice: Hello\n");
        assert!(to_markdown("Standup", &lines()).starts_with("# Standup\n\n**Alice**"));
    }

    #[test]
    fn wav_header_is_correct() {
        let path = std::env::temp_dir().join(format!("stv-{}.wav", std::process::id()));
        write_wav(&path, &[1, -1], 16_000).unwrap();
        let data = std::fs::read(&path).unwrap();
        assert_eq!(data.len(), 48);
        assert_eq!(&data[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(data[24..28].try_into().unwrap()), 16_000);
        assert_eq!(&data[44..], &[1, 0, 255, 255]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reaper_envelope_converts_db_to_linear() {
        let rpp = to_reaper(&tracks(), 16_000);
        assert!(rpp.contains("PT 1.000000 0.100000 1"));
        assert!(rpp.contains("PT 2.000000 1 1"));
        assert!(rpp.contains("NAME \"Alice 'Pin'\""));
        assert_eq!(rpp.matches('<').count(), rpp.matches('>').count() - rpp.matches("-->").count());
    }

    #[test]
    fn otio_is_valid_json_with_sample_accurate_time() {
        let parsed: serde_json::Value = serde_json::from_str(&to_otio("t", &tracks(), 16_000)).unwrap();
        let clip = &parsed["tracks"]["children"][0]["children"][0];
        assert_eq!(clip["source_range"]["duration"]["value"], 160_000.0);
        assert_eq!(clip["media_references"]["DEFAULT_MEDIA"]["target_url"], "audio/alice.wav");
    }
}
