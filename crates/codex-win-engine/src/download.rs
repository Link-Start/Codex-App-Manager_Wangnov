use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::EngineError;

static DOWNLOAD_ACTIVE: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn cancel_active_download() -> bool {
    let active = DOWNLOAD_ACTIVE.load(Ordering::SeqCst);
    if active {
        DOWNLOAD_CANCELLED.store(true, Ordering::SeqCst);
    }
    active
}

fn is_cancelled_error(err: &str) -> bool {
    err == "download cancelled"
}

fn partial_path(dest: &Path) -> PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    dest.with_file_name(format!("{file_name}.part"))
}

fn run_curl(url: &str, dest: &Path, resume: bool) -> Result<(), String> {
    let dest = dest.to_string_lossy().into_owned();
    let mut args = vec![
        "-fL".to_string(),
        "--connect-timeout".to_string(),
        "20".to_string(),
        "--retry".to_string(),
        "2".to_string(),
    ];
    if resume {
        args.extend(["-C".to_string(), "-".to_string()]);
    }
    args.extend(["-o".to_string(), dest, url.to_string()]);

    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);
    let mut child = Command::new("curl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn curl: {e}"))?;

    DOWNLOAD_ACTIVE.store(true, Ordering::SeqCst);
    loop {
        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            DOWNLOAD_ACTIVE.store(false, Ordering::SeqCst);
            return Err("download cancelled".to_string());
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(err) => {
                let _ = child.kill();
                DOWNLOAD_ACTIVE.store(false, Ordering::SeqCst);
                return Err(format!("wait for curl: {err}"));
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(err) => {
            DOWNLOAD_ACTIVE.store(false, Ordering::SeqCst);
            return Err(format!("collect curl output: {err}"));
        }
    };
    DOWNLOAD_ACTIVE.store(false, Ordering::SeqCst);

    if !output.status.success() {
        return Err(format!(
            "curl failed for {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub fn download_to(url: &str, dest: &Path) -> Result<(), EngineError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| EngineError::Io(format!("create staging dir: {e}")))?;
    }

    let part = partial_path(dest);
    let should_resume = part.metadata().map(|m| m.len() > 0).unwrap_or(false);
    let download_result = run_curl(url, &part, should_resume);
    if let Err(first_err) = download_result {
        if is_cancelled_error(&first_err) {
            return Err(EngineError::Io(first_err));
        }
        if should_resume {
            let _ = std::fs::remove_file(&part);
            run_curl(url, &part, false).map_err(|second_err| {
                if is_cancelled_error(&second_err) {
                    return EngineError::Io(second_err);
                }
                EngineError::Io(format!(
                    "resume failed ({first_err}); fresh download failed ({second_err})"
                ))
            })?;
        } else {
            return Err(EngineError::Io(first_err));
        }
    }

    if dest.exists() {
        std::fs::remove_file(dest)
            .map_err(|e| EngineError::Io(format!("remove previous download: {e}")))?;
    }
    std::fs::rename(&part, dest).map_err(|e| EngineError::Io(format!("publish download: {e}")))?;
    Ok(())
}

pub fn read_file(path: &Path) -> Result<Vec<u8>, EngineError> {
    std::fs::read(path).map_err(|e| EngineError::Io(format!("read {}: {e}", path.display())))
}

pub fn sha256_file(path: &Path) -> Result<String, EngineError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| EngineError::Io(format!("open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| EngineError::Io(format!("read {}: {e}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
