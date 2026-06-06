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

/// Download with periodic progress callbacks. Spawns `curl` and polls the
/// destination file size (curl writes incrementally) every ~300ms, invoking
/// `on_progress(downloaded_bytes)`; resumes a partial file like `download_to`.
pub fn download_to_with_progress(
    url: &str,
    dest: &Path,
    on_progress: &dyn Fn(u64),
) -> Result<u64, EngineError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::Io(e.to_string()))?;
    }

    let mut child = Command::new("curl")
        .args([
            "-fL",
            "--no-progress-meter",
            "-C",
            "-",
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
        .spawn()
        .map_err(|e| EngineError::Io(format!("spawn curl: {e}")))?;

    loop {
        match child
            .try_wait()
            .map_err(|e| EngineError::Io(e.to_string()))?
        {
            Some(status) => {
                if !status.success() {
                    return Err(EngineError::Io(format!("curl download failed: {url}")));
                }
                break;
            }
            None => {
                let downloaded = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                on_progress(downloaded);
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }

    let meta = std::fs::metadata(dest).map_err(|e| EngineError::Io(e.to_string()))?;
    on_progress(meta.len());
    Ok(meta.len())
}

/// Read a downloaded artifact into memory for verification.
pub fn read_file(path: &Path) -> Result<Vec<u8>, EngineError> {
    std::fs::read(path).map_err(|e| EngineError::Io(e.to_string()))
}
