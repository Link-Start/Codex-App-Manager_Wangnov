//! Thin macOS IO helpers for the read-only slice.
//!
//! NOTE: network fetch currently shells out to `curl` and version reading to
//! `PlistBuddy`. These are placeholders for the scaffold — the production
//! Tauri backend will inject a proper HTTP client adapter and may read the
//! plist with the `plist` crate. Keeping IO behind these functions means the
//! pure parsing/planning logic stays trivially testable.

use std::path::Path;
use std::process::Command;

use crate::EngineError;

/// Fetch a small text resource (the appcast) over HTTPS via system `curl`.
pub fn fetch_text(url: &str) -> Result<String, EngineError> {
    let output = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "20", url])
        .output()
        .map_err(|e| EngineError::Io(format!("spawn curl: {e}")))?;

    if !output.status.success() {
        return Err(EngineError::Io(format!(
            "curl failed for {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    String::from_utf8(output.stdout).map_err(|e| EngineError::Io(e.to_string()))
}

/// Locate an installed `Codex.app` and read its `CFBundleVersion` (build number).
///
/// Returns `(app_path, build)` for the first candidate found, or `None`.
pub fn installed_codex_build() -> Option<(String, u64)> {
    candidate_app_paths()
        .into_iter()
        .find_map(|app| read_bundle_build(&app).map(|build| (app, build)))
}

fn candidate_app_paths() -> Vec<String> {
    let mut paths = vec!["/Applications/Codex.app".to_string()];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(format!("{home}/Applications/Codex.app"));
    }
    paths
}

fn read_bundle_build(app: &str) -> Option<u64> {
    let plist = format!("{app}/Contents/Info.plist");
    if !Path::new(&plist).exists() {
        return None;
    }
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleVersion", &plist])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}
