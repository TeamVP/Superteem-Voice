//! Finds out whether a newer version has been released. The download page publishes the latest release's
//! details beside its files, which is asked first, as GitHub limits how often one address may ask it.
//! On Windows the new installer is downloaded and run, and on the Mac the new disk image is downloaded and
//! opened; on Android the new APK opens in the browser. The iPhone gets updates from the App Store.

use serde::Serialize;

pub const SITE: &str = "https://voice.superteem.com";
const RELEASE: &str = "https://voice.superteem.com/download/release.json";
const GITHUB: &str = "https://api.github.com/repos/TeamVP/Superteem-Voice/releases/latest";
const USER_AGENT: &str = concat!("voice-by-superteem/", env!("CARGO_PKG_VERSION"));

/// This device's file in each release.
#[cfg(target_os = "android")]
const FILE: &str = "Voice-by-Superteem-Android.apk";
#[cfg(target_os = "macos")]
const FILE: &str = "Voice-by-Superteem-macOS.dmg";
#[cfg(target_os = "ios")]
const FILE: &str = "Voice-by-Superteem-iOS.ipa";
#[cfg(not(any(target_os = "android", target_os = "macos", target_os = "ios")))]
const FILE: &str = "Voice-by-Superteem-Setup-x64.exe";

#[derive(Debug, Clone, Serialize)]
pub struct Update {
    pub current: String,
    pub latest: String,
    /// The latest release is newer than this app and has this device's file.
    pub newer: bool,
    /// This device's file in the latest release.
    pub download: String,
    /// What changed.
    pub notes: String,
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "v0.3.1" or "0.3.1" as numbers, so 0.10 comes after 0.9.
fn parse(version: &str) -> Vec<u64> {
    version.trim().trim_start_matches('v').split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (parse(latest), parse(current));
    !a.is_empty() && a > b
}

fn fetch(url: &str) -> Result<serde_json::Value, String> {
    let text = crate::transcribe::agent()?
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn check() -> Result<Update, String> {
    let (release, from_site) = match fetch(RELEASE) {
        Ok(r) => (r, true),
        Err(_) => (fetch(GITHUB).map_err(|e| format!("Couldn't check for updates: {e}"))?, false),
    };
    let latest = release["tag_name"].as_str().ok_or("The release has no version.")?.trim_start_matches('v').to_string();
    let asset = release["assets"].as_array().and_then(|a| a.iter().find(|a| a["name"] == FILE));
    let download = match asset {
        Some(_) if from_site => format!("{SITE}/download/{FILE}"),
        Some(a) => a["browser_download_url"].as_str().unwrap_or(SITE).to_string(),
        None => SITE.to_string(),
    };
    Ok(Update {
        // The App Store updates the iPhone app; a file from the release can't be installed there.
        newer: asset.is_some() && is_newer(&latest, current()) && cfg!(not(target_os = "ios")),
        current: current().to_string(),
        latest,
        download,
        notes: release["html_url"].as_str().unwrap_or(SITE).to_string(),
    })
}

/// Gets the newer version. On Windows the installer is downloaded and started, and this returns once it's
/// running so the app can close; on the Mac the disk image is downloaded and opened in Finder, to drag the
/// new app over this one; on Android the APK download opens in the browser, and Android offers to install
/// it.
pub fn install(update: &Update, progress: impl Fn(f32)) -> Result<(), String> {
    if !update.download.starts_with(SITE) && !update.download.starts_with("https://github.com/TeamVP/") {
        return Err("That download isn't from the Voice by Superteem site.".into());
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = progress;
        crate::recorder::open_url(&update.download)
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use std::io::{Read, Write};
        let response = crate::transcribe::agent()?
            .get(&update.download)
            .set("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("The download failed: {e}"))?;
        let total: Option<u64> = response.header("Content-Length").and_then(|v| v.parse().ok());
        let name = if cfg!(target_os = "macos") { "macOS.dmg" } else { "Setup-x64.exe" };
        let path = std::env::temp_dir().join(format!("Voice-by-Superteem-{}-{name}", update.latest));
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut reader = response.into_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("The download failed: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if let Some(total) = total {
                progress(done as f32 / total as f32);
            }
        }
        drop(file);
        // The installer and disk image are a few megabytes; anything much smaller is an error page.
        if total.is_some_and(|t| t != done) || done < 1_000_000 {
            let _ = std::fs::remove_file(&path);
            return Err("The download was incomplete. Try again.".into());
        }
        // Finder shows the disk image's window, with the app beside a link to Applications.
        if cfg!(target_os = "macos") {
            return crate::shell::open(&path).map_err(|e| format!("The disk image couldn't open: {e}"));
        }
        // Passive: it runs with a progress bar and nothing to click, then opens the app again.
        std::process::Command::new(&path)
            .args(["/P", "/R", "/UPDATE"])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("The installer couldn't start: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.99.0"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.6", "0.3.0"));
        assert!(!is_newer("nonsense", "0.3.0"));
    }

    /// Asks the real download page. Run with `cargo test --lib -- --ignored`.
    #[test]
    #[ignore]
    fn the_download_page_says_what_is_latest() {
        let update = check().unwrap();
        assert!(!parse(&update.latest).is_empty(), "{update:?}");
        assert!(update.download.ends_with(FILE), "{update:?}");
    }
}
