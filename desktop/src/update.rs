//! Finds out whether a newer version has been released. The download page publishes the latest release's
//! details beside its files, which is asked first, as GitHub limits how often one address may ask it.
//! That one request also carries this device's configuration: the app version, operating system, models
//! in use, configured languages, and why the check ran (`auto`, `manual`, or `install`). The GitHub
//! fallback and the installer download do not.
//! On Windows the new installer is downloaded and run, and on the Mac the new disk image is downloaded and
//! opened; on Android the new APK opens in the browser. The iPhone gets updates from the App Store.

use serde::Serialize;

pub const SITE: &str = "https://voice.superteem.com";
/// Static `release.json` on GitHub Pages. The query below is not stored: Pages does not keep it, and this
/// repository has no access log for voice.superteem.com, so aggregate reporting needs a change on the host.
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

/// How long the home page says the app was updated.
const UPDATED_FOR_S: u64 = 24 * 60 * 60;

/// The app was just updated: shown on the home page for a day.
#[derive(Debug, Clone, Serialize)]
pub struct JustUpdated {
    pub version: String,
    /// What changed in this version.
    pub notes: String,
}

/// Notes the version that's running, at start. A version other than the last one run is an update; so is
/// the first launch of one that records it, by someone who has used the app already.
pub fn note_launch() {
    let p = crate::prefs::load();
    if let Some(updated_at) = launched(&p.last_version, p.onboarded, current(), crate::library::now()) {
        let _ = crate::prefs::update(|p| {
            p.last_version = current().to_string();
            p.updated_at = updated_at;
        });
    }
}

/// What starting `current` records, given the version last run: None when it's the same; else when it was
/// updated, 0 for a new install.
fn launched(last: &str, onboarded: bool, current: &str, now: u64) -> Option<u64> {
    (last != current).then(|| if !last.is_empty() || onboarded { now } else { 0 })
}

/// The update to say on the home page, within a day of it.
pub fn just_updated() -> Option<JustUpdated> {
    let p = crate::prefs::load();
    showing(p.updated_at, &p.last_version, current(), crate::library::now())
        .then(|| JustUpdated { version: current().to_string(), notes: format!("https://github.com/TeamVP/Superteem-Voice/releases/tag/v{}", current()) })
}

fn showing(updated_at: u64, last: &str, current: &str, now: u64) -> bool {
    updated_at > 0 && last == current && now.saturating_sub(updated_at) < UPDATED_FOR_S
}

/// "v0.3.1" or "0.3.1" as numbers, so 0.10 comes after 0.9.
fn parse(version: &str) -> Vec<u64> {
    version.trim().trim_start_matches('v').split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (parse(latest), parse(current));
    !a.is_empty() && a > b
}

/// Configuration sent with the Superteem release check. Values are already resolved: `none` where nothing
/// usable is configured, `unknown` where this device couldn't tell.
struct Telemetry {
    app_version: String,
    os: String,
    os_version: String,
    analysis_model: String,
    analysis_mode: String,
    transcription: Vec<(&'static str, String)>,
    languages: String,
    primary_language: String,
    /// `auto`, `manual`, `install`, or `unknown`.
    reason: String,
}

/// `auto` is the daily check, `manual` is a person asking, `install` is the check at the start of an
/// update. Anything else is `unknown`, so a bad label cannot fail the update check.
fn normalise_reason(reason: &str) -> &'static str {
    match reason {
        "auto" => "auto",
        "manual" => "manual",
        "install" => "install",
        _ => "unknown",
    }
}

/// The release URL, including configuration. A failure to collect it still asks the same file.
fn release_check_url(reason: &str) -> String {
    let reason = normalise_reason(reason).to_string();
    let collected = std::panic::catch_unwind(collect_telemetry).ok().map(|mut telemetry| {
        telemetry.reason = reason;
        telemetry
    });
    release_url_from(collected)
}

fn release_url_from(collected: Option<Telemetry>) -> String {
    match collected {
        Some(telemetry) => superteem_release_url(&telemetry),
        None => RELEASE.to_string(),
    }
}

fn superteem_release_url(telemetry: &Telemetry) -> String {
    let mut pairs = vec![
        ("app_version", telemetry.app_version.as_str()),
        ("os", telemetry.os.as_str()),
        ("os_version", telemetry.os_version.as_str()),
        ("analysis_model", telemetry.analysis_model.as_str()),
        ("analysis_mode", telemetry.analysis_mode.as_str()),
    ];
    for (name, value) in &telemetry.transcription {
        pairs.push((name, value.as_str()));
    }
    pairs.push(("languages", telemetry.languages.as_str()));
    pairs.push(("primary_language", telemetry.primary_language.as_str()));
    pairs.push(("reason", telemetry.reason.as_str()));
    let query: Vec<String> = pairs.iter().map(|(name, value)| format!("{}={}", encode_query(name), encode_query(value))).collect();
    format!("{RELEASE}?{}", query.join("&"))
}

/// Percent-encoding for one query component. Space is `%20`.
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn collect_telemetry() -> Telemetry {
    let prefs = crate::prefs::load();
    let (languages, primary_language) = language_fields(&prefs);
    let (analysis_mode, analysis_model) = analysis_fields(&prefs);
    let (os, os_version) = os_info();
    Telemetry {
        app_version: current().to_string(),
        os,
        os_version,
        analysis_model,
        analysis_mode,
        transcription: transcription_fields(&prefs.languages()),
        languages,
        primary_language,
        reason: "unknown".into(),
    }
}

/// Configured languages, from the preference helpers. Empty or unknown settings are English.
fn language_fields(prefs: &crate::prefs::Prefs) -> (String, String) {
    let languages = prefs.languages();
    let primary = prefs.primary_language();
    let languages = if languages.is_empty() { "unknown".into() } else { languages.join(",") };
    let primary = if primary.is_empty() { "unknown".into() } else { primary };
    (languages, primary)
}

/// The analysis model that would run, without loading it. A configured server wins, and its address is
/// never reported. An empty local choice is the first installed model, as `llm::chosen` does.
fn analysis_fields(prefs: &crate::prefs::Prefs) -> (String, String) {
    let installed = crate::llm::installed();
    let picked = crate::llm::pick(&prefs.llm_model, &installed);
    effective_analysis(&prefs.llm_server, &prefs.llm_server_model, picked)
}

fn effective_analysis(server: &str, server_model: &str, picked: Option<&crate::llm::Installed>) -> (String, String) {
    if !server.trim().is_empty() {
        let model = server_model.trim();
        return ("server".into(), if model.is_empty() { "unknown".into() } else { model.into() });
    }
    match picked {
        Some(installed) => {
            let named = installed.catalogued.map(str::trim).filter(|id| !id.is_empty()).map(str::to_string).unwrap_or_else(|| installed.file.trim().to_string());
            ("local".into(), if named.is_empty() { "unknown".into() } else { named })
        }
        None => ("none".into(), "none".into()),
    }
}

/// Speech models that would transcribe the configured languages, without loading them.
/// English and any other language can resolve to different models. Both are named when they differ;
/// a single `transcription_model` is sent only when one id covers the configured languages.
fn transcription_fields(languages: &[String]) -> Vec<(&'static str, String)> {
    let configured = crate::transcribe::model_for(languages);
    let split = !stv_core::lang::english_only(languages) && languages.iter().any(|code| code == stv_core::lang::ENGLISH);
    let english = if split { crate::transcribe::model_for(&[stv_core::lang::ENGLISH.to_string()]) } else { configured };
    transcription_pairs(split, (english.0.id, english.1), (configured.0.id, configured.1))
}

fn transcription_pairs(split: bool, english: (&str, bool), other: (&str, bool)) -> Vec<(&'static str, String)> {
    if !split {
        return vec![("transcription_model", usable_model(other.0, other.1))];
    }
    let english = usable_model(english.0, english.1);
    let other = usable_model(other.0, other.1);
    if english == other {
        vec![("transcription_model", english)]
    } else {
        vec![("transcription_model_en", english), ("transcription_model_other", other)]
    }
}

fn usable_model(id: &str, ready: bool) -> String {
    if !ready {
        "none".into()
    } else if id.trim().is_empty() {
        "unknown".into()
    } else {
        id.to_string()
    }
}

/// Operating system name and its release version. `sysinfo` is already how the app reads this machine.
/// On Android its "name" is the phone model, and on macOS it is "Darwin", so those use the platform name.
fn os_info() -> (String, String) {
    let read = std::panic::catch_unwind(|| {
        let version = sysinfo::System::os_version().filter(|version| !version.trim().is_empty()).or_else(sysinfo::System::long_os_version).unwrap_or_default();
        (os_name(), version)
    });
    match read {
        Ok((name, version)) => (known(name), known(version)),
        Err(_) => ("unknown".into(), "unknown".into()),
    }
}

fn os_name() -> String {
    if cfg!(target_os = "macos") {
        "macOS".into()
    } else if cfg!(target_os = "ios") {
        "iOS".into()
    } else if cfg!(target_os = "android") {
        "Android".into()
    } else if cfg!(target_os = "windows") {
        sysinfo::System::name().unwrap_or_else(|| "Windows".into())
    } else {
        sysinfo::System::name().filter(|name| !name.trim().is_empty()).unwrap_or_else(|| std::env::consts::OS.to_string())
    }
}

fn known(value: String) -> String {
    let value = value.trim();
    if value.is_empty() { "unknown".into() } else { value.to_string() }
}

/// Where the installer comes from. The Superteem copy is preferred; a GitHub asset URL is used only on
/// the fallback, unchanged. Neither gets the configuration query.
fn installer_url(from_site: bool, asset_present: bool, browser_download_url: Option<&str>) -> String {
    if from_site && asset_present {
        format!("{SITE}/download/{FILE}")
    } else {
        browser_download_url.unwrap_or(SITE).to_string()
    }
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

pub fn check(reason: &str) -> Result<Update, String> {
    let (release, from_site) = match fetch(&release_check_url(reason)) {
        Ok(r) => (r, true),
        Err(_) => (fetch(GITHUB).map_err(|e| format!("Couldn't check for updates: {e}"))?, false),
    };
    let latest = release["tag_name"].as_str().ok_or("The release has no version.")?.trim_start_matches('v').to_string();
    let asset = release["assets"].as_array().and_then(|a| a.iter().find(|a| a["name"] == FILE));
    let download = installer_url(from_site, asset.is_some(), asset.and_then(|a| a["browser_download_url"].as_str()));
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
        let update = check("auto").unwrap();
        assert!(!parse(&update.latest).is_empty(), "{update:?}");
        assert!(update.download.ends_with(FILE), "{update:?}");
    }

    #[test]
    fn an_update_shows_for_a_day() {
        // Same version: nothing changes.
        assert_eq!(launched("0.8.5", true, "0.8.5", 1000), None);
        // From an earlier version, or the first that records it for someone already set up: updated now.
        assert_eq!(launched("0.8.4", true, "0.8.5", 1000), Some(1000));
        assert_eq!(launched("", true, "0.8.5", 1000), Some(1000));
        // A new install isn't an update.
        assert_eq!(launched("", false, "0.8.5", 1000), Some(0));
        let day = 24 * 60 * 60;
        assert!(showing(1000, "0.8.5", "0.8.5", 1000 + day - 1));
        assert!(!showing(1000, "0.8.5", "0.8.5", 1000 + day));
        assert!(!showing(0, "0.8.5", "0.8.5", 1000));
        assert!(!showing(1000, "0.8.4", "0.8.5", 1001));
    }

    fn sample(analysis_model: &str, analysis_mode: &str, transcription: Vec<(&'static str, String)>, languages: &str, primary: &str) -> Telemetry {
        Telemetry {
            app_version: "0.8.7".into(),
            os: "Windows".into(),
            os_version: "10.0.26200".into(),
            analysis_model: analysis_model.into(),
            analysis_mode: analysis_mode.into(),
            transcription,
            languages: languages.into(),
            primary_language: primary.into(),
            reason: "auto".into(),
        }
    }

    #[test]
    fn the_release_query_encodes_reserved_characters() {
        let url = superteem_release_url(&sample("org/Qwen 3:1.7b", "server", vec![("transcription_model", "base.en".into())], "en", "en"));
        assert_eq!(
            url,
            "https://voice.superteem.com/download/release.json?app_version=0.8.7&os=Windows&os_version=10.0.26200&analysis_model=org%2FQwen%203%3A1.7b&analysis_mode=server&transcription_model=base.en&languages=en&primary_language=en&reason=auto"
        );
        let awkward = superteem_release_url(&sample("a b/c:d?e&f#g%h", "local", vec![("transcription_model", "none".into())], "en", "en"));
        assert!(awkward.contains("analysis_model=a%20b%2Fc%3Ad%3Fe%26f%23g%25h"));
        assert!(!awkward.contains(' ') && !awkward.contains("analysis_model=a b"));
    }

    #[test]
    fn analysis_reports_the_model_that_would_run() {
        let local = crate::llm::Installed { file: "Qwen3-1.7B-Q4_K_M.gguf".into(), size: 1, catalogued: Some("qwen3-1.7b") };
        let added = crate::llm::Installed { file: "My Model.gguf".into(), size: 2, catalogued: None };
        let blank = crate::llm::Installed { file: "  ".into(), size: 3, catalogued: None };
        assert_eq!(effective_analysis("", "", Some(&local)), ("local".into(), "qwen3-1.7b".into()));
        assert_eq!(effective_analysis("", "", Some(&added)), ("local".into(), "My Model.gguf".into()));
        assert_eq!(effective_analysis("", "", Some(&blank)), ("local".into(), "unknown".into()));
        assert_eq!(effective_analysis("  ", "", None), ("none".into(), "none".into()));
        // The server address is how the choice is made, and is not part of the report.
        assert_eq!(effective_analysis("http://secret-host.example:11434/v1", "qwen3:1.7b", Some(&local)), ("server".into(), "qwen3:1.7b".into()));
        assert_eq!(effective_analysis("http://secret-host.example:11434/v1", "  ", None), ("server".into(), "unknown".into()));
        let url = superteem_release_url(&sample("qwen3:1.7b", "server", vec![("transcription_model", "base.en".into())], "en", "en"));
        assert!(!url.contains("secret-host") && !url.contains("11434") && !url.contains("Qwen3-1.7B"));
        assert!(url.contains("analysis_mode=server") && url.contains("analysis_model=qwen3%3A1.7b"));
    }

    #[test]
    fn transcription_names_each_path_when_the_models_differ() {
        let one = |id: &str, ready: bool| transcription_pairs(false, ("base.en", false), (id, ready));
        assert_eq!(one("base.en", true), vec![("transcription_model", "base.en".into())]);
        assert_eq!(one("turbo", false), vec![("transcription_model", "none".into())]);
        assert_eq!(transcription_pairs(true, ("turbo", true), ("turbo", true)), vec![("transcription_model", "turbo".into())]);
        assert_eq!(
            transcription_pairs(true, ("base.en", true), ("turbo", true)),
            vec![("transcription_model_en", "base.en".into()), ("transcription_model_other", "turbo".into())]
        );
        assert_eq!(
            transcription_pairs(true, ("base.en", true), ("small", false)),
            vec![("transcription_model_en", "base.en".into()), ("transcription_model_other", "none".into())]
        );
        let url = superteem_release_url(&sample(
            "none",
            "none",
            vec![("transcription_model_en", "base.en".into()), ("transcription_model_other", "none".into())],
            "en,cy",
            "en",
        ));
        assert!(url.contains("transcription_model_en=base.en"));
        assert!(url.contains("transcription_model_other=none"));
        assert!(!url.contains("transcription_model="));
    }

    #[test]
    fn configured_languages_follow_the_preference_helpers() {
        let mut prefs = crate::prefs::Prefs::default();
        assert_eq!(language_fields(&prefs), ("en".into(), "en".into()));
        prefs.languages = vec!["cy".into(), "en".into(), "cy".into(), "zz".into()];
        prefs.primary_language = "cy".into();
        assert_eq!(language_fields(&prefs), ("cy,en".into(), "cy".into()));
        prefs.languages.clear();
        prefs.primary_language = "nope".into();
        assert_eq!(language_fields(&prefs), ("en".into(), "en".into()));
    }

    #[test]
    fn missing_details_still_ask_the_release_file() {
        let url = superteem_release_url(&Telemetry {
            app_version: "0.8.7".into(),
            os: "unknown".into(),
            os_version: "unknown".into(),
            analysis_model: "unknown".into(),
            analysis_mode: "local".into(),
            transcription: vec![("transcription_model", "none".into())],
            languages: "en".into(),
            primary_language: "en".into(),
            reason: "unknown".into(),
        });
        assert!(url.starts_with(RELEASE));
        assert!(url.contains("os=unknown") && url.contains("os_version=unknown") && url.contains("analysis_model=unknown"));
        assert_eq!(release_url_from(None), RELEASE);
    }

    #[test]
    fn telemetry_is_only_on_the_superteem_release_check() {
        let url = superteem_release_url(&sample("qwen3-1.7b", "local", vec![("transcription_model", "base.en".into())], "en", "en"));
        assert!(url.starts_with(&format!("{RELEASE}?")));
        assert!(url.contains("analysis_model=qwen3-1.7b"));
        assert!(!GITHUB.contains('?') && !GITHUB.contains("analysis_model") && !GITHUB.contains("languages"));
        let site_installer = installer_url(true, true, Some("https://github.com/TeamVP/Superteem-Voice/releases/download/v0.8.7/ignored.exe"));
        let github_installer = installer_url(false, true, Some("https://github.com/TeamVP/Superteem-Voice/releases/download/v0.8.7/Voice-by-Superteem-Setup-x64.exe"));
        let plain_site = installer_url(false, false, None);
        for download in [site_installer.as_str(), github_installer.as_str(), plain_site.as_str(), GITHUB] {
            assert!(!download.contains("analysis_model") && !download.contains("os_version") && !download.contains("languages=") && !download.contains("reason="), "{download}");
        }
        assert_eq!(site_installer, format!("{SITE}/download/{FILE}"));
        assert!(github_installer.starts_with("https://github.com/TeamVP/"));
    }

    #[test]
    fn reason_is_auto_manual_or_install() {
        assert_eq!(normalise_reason("auto"), "auto");
        assert_eq!(normalise_reason("manual"), "manual");
        assert_eq!(normalise_reason("install"), "install");
        assert_eq!(normalise_reason("daily"), "unknown");
        assert_eq!(normalise_reason(""), "unknown");
        for reason in ["auto", "manual", "install"] {
            let mut telemetry = sample("qwen3-1.7b", "local", vec![("transcription_model", "base.en".into())], "en", "en");
            telemetry.reason = reason.into();
            let url = superteem_release_url(&telemetry);
            assert!(url.ends_with(&format!("&reason={reason}")), "{url}");
            assert_eq!(url.matches("reason=").count(), 1, "{url}");
        }
    }

    #[test]
    fn the_live_release_check_stays_on_the_superteem_site() {
        let url = release_check_url("auto");
        assert!(url.starts_with(RELEASE), "{url}");
        assert!(!url.contains("github.com"), "{url}");
        if url != RELEASE {
            for key in ["app_version=", "os=", "os_version=", "analysis_model=", "analysis_mode=", "languages=", "primary_language=", "reason=auto"] {
                assert!(url.contains(key), "{key} missing from {url}");
            }
            assert!(url.contains("transcription_model"));
        }
    }

    #[test]
    fn os_info_names_the_system_and_its_release() {
        let (name, version) = os_info();
        assert!(!name.is_empty() && name != "unknown" && name != "Darwin", "{name}");
        assert!(version.chars().any(|c| c.is_ascii_digit()), "{name} {version}");
    }
}
