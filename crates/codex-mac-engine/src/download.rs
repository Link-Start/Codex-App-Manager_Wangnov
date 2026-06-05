//! Resumable downloader for update artifacts.
//!
//! Scaffold transport: shells out to `curl -C -` (HTTP range resume), which
//! both R2/S3 and oaistatic support. Production will swap in a proper async
//! HTTP client behind the same function signature; keeping it here lets the
//! verify/plan flow be exercised end-to-end today.

use std::path::Path;
use std::process::Command;

use crate::EngineError;

/// Download `url` into `dest`, resuming a partial file if present.
/// Returns the final file size in bytes.
pub fn download_to(url: &str, dest: &Path) -> Result<u64, EngineError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::Io(e.to_string()))?;
    }

    let status = Command::new("curl")
        .args([
            "-fL",
            "--no-progress-meter",
            "-C",
            "-", // resume from wherever the partial file left off
            "--retry",
            "5",
            "--retry-delay",
            "2",
            "--retry-all-errors",
            "--connect-timeout",
            "20",
            "-o",
            &dest.to_string_lossy(),
            url,
        ])
        .status()
        .map_err(|e| EngineError::Io(format!("spawn curl: {e}")))?;

    if !status.success() {
        return Err(EngineError::Io(format!("curl download failed: {url}")));
    }

    let meta = std::fs::metadata(dest).map_err(|e| EngineError::Io(e.to_string()))?;
    Ok(meta.len())
}

/// Read a downloaded artifact into memory for verification.
pub fn read_file(path: &Path) -> Result<Vec<u8>, EngineError> {
    std::fs::read(path).map_err(|e| EngineError::Io(e.to_string()))
}
