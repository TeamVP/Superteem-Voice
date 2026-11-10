//! Opens folders, files and web pages with the system's own apps: File Explorer on Windows, Finder and the
//! default browser on the Mac. Phones keep the app's folders private, so there's nothing to open there,
//! and their links go through the system (see `recorder::open_url`).

use std::ffi::OsStr;
use std::path::Path;

/// Opens a folder in the file manager, or a web page in the default browser.
pub fn open(target: impl AsRef<OsStr>) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        return Err(unavailable());
    };
    std::process::Command::new(program).arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens the folder a file is in, with the file selected.
pub fn reveal(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        return Err(unavailable());
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn unavailable() -> String {
    if cfg!(target_os = "ios") {
        "Folders can't be opened on iPhone. The app's files are in the Files app, under On My iPhone.".into()
    } else {
        "Folders can't be opened on Android.".into()
    }
}
