//! The .voiceprint bundle: manifest.json plus reference audio clips. See PRD section 8.

use serde::{Deserialize, Serialize};
use std::io::{self, Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_BUNDLE_BYTES: usize = 220 * 1024;
pub const MAX_AUDIO_SECONDS: f32 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    pub model: String,
    pub model_hash: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipInfo {
    pub id: String,
    pub duration: f32,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub display_name: String,
    pub created_at: String,
    pub consent_at: String,
    pub cached_embeddings: Vec<Embedding>,
    pub audio_manifest: Vec<ClipInfo>,
}

fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn validate(manifest: &Manifest) -> io::Result<()> {
    if manifest.format_version != FORMAT_VERSION {
        return Err(invalid("unsupported voiceprint version"));
    }
    if manifest.consent_at.is_empty() {
        return Err(invalid("a voiceprint requires recorded consent"));
    }
    if manifest.audio_manifest.iter().map(|c| c.duration).sum::<f32>() > MAX_AUDIO_SECONDS {
        return Err(invalid("more than 60 seconds of reference audio"));
    }
    Ok(())
}

/// `clips` holds Opus bytes in `audio_manifest` order. Stored without compression because Opus is already compressed.
pub fn write(manifest: &Manifest, clips: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    validate(manifest)?;
    if clips.len() != manifest.audio_manifest.len() {
        return Err(invalid("clip count does not match the manifest"));
    }
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("manifest.json", options).map_err(invalid)?;
    zip.write_all(serde_json::to_string(manifest).map_err(invalid)?.as_bytes())?;
    for (info, clip) in manifest.audio_manifest.iter().zip(clips) {
        zip.start_file(format!("audio/{}.opus", info.id), options).map_err(invalid)?;
        zip.write_all(clip)?;
    }
    let bytes = zip.finish().map_err(invalid)?.into_inner();
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("bundle is larger than 220 KB"));
    }
    Ok(bytes)
}

pub fn read(bytes: &[u8]) -> io::Result<(Manifest, Vec<Vec<u8>>)> {
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("bundle is larger than 220 KB"));
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(invalid)?;
    let mut text = String::new();
    zip.by_name("manifest.json").map_err(invalid)?.take(MAX_BUNDLE_BYTES as u64).read_to_string(&mut text)?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(invalid)?;
    validate(&manifest)?;
    let mut clips = Vec::new();
    for info in &manifest.audio_manifest {
        let mut clip = Vec::new();
        zip.by_name(&format!("audio/{}.opus", info.id))
            .map_err(invalid)?
            .take(MAX_BUNDLE_BYTES as u64)
            .read_to_end(&mut clip)?;
        clips.push(clip);
    }
    Ok((manifest, clips))
}

/// Returns None for mismatched or zero vectors, as vectors from different models are not comparable.
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt();
    (norm > 0.0).then(|| dot / norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            format_version: FORMAT_VERSION,
            display_name: "Alice".into(),
            created_at: "2026-11-07T00:00:00Z".into(),
            consent_at: "2026-11-07T00:00:00Z".into(),
            cached_embeddings: vec![Embedding { model: "ecapa".into(), model_hash: "abc".into(), vector: vec![0.5; 192] }],
            audio_manifest: vec![ClipInfo { id: "clip_01".into(), duration: 6.2, category: "statement".into() }],
        }
    }

    #[test]
    fn round_trips_at_realistic_size() {
        let clip = vec![7u8; 18_000];
        let bytes = write(&manifest(), &[clip.clone()]).unwrap();
        let (m, clips) = read(&bytes).unwrap();
        assert_eq!(m, manifest());
        assert_eq!(clips, vec![clip]);
    }

    #[test]
    fn enforces_consent_size_and_clip_rules() {
        let mut no_consent = manifest();
        no_consent.consent_at.clear();
        assert!(write(&no_consent, &[vec![0]]).is_err());
        assert!(write(&manifest(), &[vec![0; MAX_BUNDLE_BYTES]]).is_err());
        assert!(write(&manifest(), &[]).is_err());
        let mut too_long = manifest();
        too_long.audio_manifest[0].duration = 61.0;
        assert!(write(&too_long, &[vec![0]]).is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(read(b"not a zip").is_err());
    }

    #[test]
    fn cosine_handles_edge_cases() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]).unwrap() - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).unwrap().abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), None);
        assert_eq!(cosine(&[0.0], &[0.0]), None);
    }
}
