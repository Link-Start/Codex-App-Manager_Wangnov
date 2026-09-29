//! Retention of the last verified MSIX as the block-delta base.
//!
//! **Where the app currently keeps/deletes the downloaded MSIX** (as of this
//! writing, `src-tauri/src/app/win_update.rs` + `src-tauri/src/app/staging.rs`):
//! a successful download lands at `staging::download_cache_path(url, name)`,
//! a path keyed by an FNV-1a hash of the download URL under
//! `staging_root()/downloads/`. Nothing deletes that file right after a
//! successful install -- `staging::clear_download_cache()` (which empties the
//! whole `downloads/` directory) is only called from
//! `discard_windows_download()`, the paused-state *cancel* path. The
//! background sweep in `cleanup_stale_staging` (`staging.rs`) does reclaim it
//! eventually: any file under `downloads/` older than `STALE_AFTER` (30
//! minutes) is removed the next time that sweep runs while no operation is
//! busy. In practice this means the just-installed MSIX already survives
//! for a little while (a very short-lived, accidental form of "retention"),
//! but not for the days-to-weeks between real Codex releases, and it is
//! keyed by URL rather than by "the most recently verified package" -- an
//! interrupted update that downloaded-but-not-installed a package would be
//! just as likely to survive as one that actually got installed.
//!
//! This module is a deliberate, explicit replacement for that accidental
//! survival: a single fixed-name slot (`retain_verified_base` /
//! `retained_base` / `clear_retained_base`) the *caller* populates only once
//! it has verified a downloaded MSIX's SHA-256 against the mirror manifest,
//! and which a future update run can hand straight to
//! [`crate::delta::planner`] as the delta base. It enforces "keep at most
//! one" by always overwriting the previous base's fixed file name rather
//! than accumulating one file per version.
//!
//! **Disk cost**: one retained base is one extra whole MSIX permanently on
//! disk between updates -- currently ~750-900 MB for the x64 build (see the
//! feasibility report's `new_size` figures) -- on top of whatever space the
//! next update's own download/staging transiently needs. That transient
//! peak is unchanged: the new package must still be assembled somewhere
//! before it replaces the base, and its uncompressed content can't be
//! smaller than one build's worth of bytes. Callers on constrained disks
//! should treat retention as opt-in (e.g. gated by the same default-off
//! setting that would gate the rest of this feature -- see the crate's
//! `delta` module doc comment) and call [`clear_retained_base`] whenever the
//! user disables it.
//!
//! No wiring into `win_update.rs` happens in this PR: the functions here are
//! pure filesystem operations the caller drives explicitly, so integrating
//! them is a separate, reviewable change once the executor above has proven
//! itself.

use std::io;
use std::path::{Path, PathBuf};

/// Fixed file name for the retained base inside `base_dir` -- fixed, rather
/// than versioned, so a new call always replaces the previous base instead
/// of accumulating one file per update ("keep at most one").
pub const BASE_FILE_NAME: &str = "delta-base.msix";
/// Sidecar recording the base's own verified SHA-256, so a future update run
/// can identify what it has on disk without re-hashing a ~1 GB file just to
/// check whether it's usable as a base.
pub const BASE_SHA256_FILE_NAME: &str = "delta-base.sha256";

/// Replace the retained delta base (if any) with `verified_msix`.
///
/// `sha256` must already be the value the caller independently verified
/// `verified_msix` against (e.g. the mirror manifest's checksum for that
/// release) -- this function does not re-verify it, it only records it
/// alongside the copy so a later reader does not have to re-hash the file.
///
/// The copy is written to a temp path in `base_dir` first and only renamed
/// onto the fixed [`BASE_FILE_NAME`] once fully written, so a crash or a
/// disk-full error partway through never leaves a truncated file at the
/// name a future update would trust.
///
/// The previous sidecar checksum is removed *before* the new base is
/// renamed into place, and the new sidecar is written *after* -- never the
/// reverse. A crash (or disk-full error) between those two steps therefore
/// always leaves either the still-consistent previous (base, sha256) pair
/// intact (nothing renamed yet) or a base file with no sidecar at all,
/// which [`retained_base`] treats as "no usable base". The one ordering
/// this rules out is the unsafe one: a *new* base ever left paired with the
/// *previous* base's sidecar, which [`retained_base`] would otherwise hand
/// out as a usable pair despite the checksum belonging to different bytes.
///
/// The new sidecar's own write is not a plain [`std::fs::write`]: it goes
/// through the same write-to-temp-then-rename sequence as the base copy
/// above, so a crash or disk-full error partway through *that* write can
/// never leave a truncated-but-nonempty sidecar file behind for
/// [`retained_base`] to read back as if it were a complete SHA-256.
pub fn retain_verified_base(
    base_dir: &Path,
    verified_msix: &Path,
    sha256: &str,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base_dir)?;
    let dest = base_dir.join(BASE_FILE_NAME);
    let tmp = base_dir.join(format!("{BASE_FILE_NAME}.tmp"));
    let sha_path = base_dir.join(BASE_SHA256_FILE_NAME);
    let sha_tmp = base_dir.join(format!("{BASE_SHA256_FILE_NAME}.tmp"));
    std::fs::copy(verified_msix, &tmp)?;
    match std::fs::remove_file(&sha_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::fs::rename(&tmp, &dest)?;
    std::fs::write(&sha_tmp, sha256.trim())?;
    std::fs::rename(&sha_tmp, &sha_path)?;
    Ok(dest)
}

/// The currently retained base, if one is present and its sidecar checksum
/// is readable and non-empty. A base file with a missing/empty sidecar (an
/// interrupted [`retain_verified_base`], or manual tampering) is treated as
/// "no usable base" rather than handed to the planner unverified -- the
/// planner's own base-layout parse and the executor's final whole-file
/// SHA-256 check are the real safety net either way, but there is no reason
/// to spend a delta-plan attempt on a base this module cannot itself vouch
/// for the provenance of.
pub fn retained_base(base_dir: &Path) -> Option<(PathBuf, String)> {
    let path = base_dir.join(BASE_FILE_NAME);
    if !path.is_file() {
        return None;
    }
    let sha256 = std::fs::read_to_string(base_dir.join(BASE_SHA256_FILE_NAME))
        .ok()?
        .trim()
        .to_string();
    if sha256.is_empty() {
        return None;
    }
    Some((path, sha256))
}

/// Drop the retained base entirely (both the MSIX and its sidecar). Used
/// when the user disables delta updates, clears the cache, or a base fails
/// its final verification and should not be reused as-is for the next
/// attempt. Missing files are not an error -- this is also how a "nothing
/// retained yet" state gets normalized after a manual cleanup.
pub fn clear_retained_base(base_dir: &Path) -> io::Result<()> {
    for name in [BASE_FILE_NAME, BASE_SHA256_FILE_NAME] {
        match std::fs::remove_file(base_dir.join(name)) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "codex-win-engine-retention-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn retains_and_reads_back_a_base() {
        let base_dir = temp_dir("retain");
        let source_dir = temp_dir("source");
        let source = source_dir.join("Codex.msix");
        std::fs::write(&source, b"fake msix bytes").unwrap();

        assert!(retained_base(&base_dir).is_none());
        let dest = retain_verified_base(&base_dir, &source, "abc123").unwrap();
        assert_eq!(dest, base_dir.join(BASE_FILE_NAME));
        assert_eq!(std::fs::read(&dest).unwrap(), b"fake msix bytes");

        let (path, sha256) = retained_base(&base_dir).unwrap();
        assert_eq!(path, dest);
        assert_eq!(sha256, "abc123");

        std::fs::remove_dir_all(&base_dir).ok();
        std::fs::remove_dir_all(&source_dir).ok();
    }

    #[test]
    fn a_second_retain_call_replaces_rather_than_accumulates() {
        let base_dir = temp_dir("replace");
        let source_dir = temp_dir("replace-source");
        let first = source_dir.join("first.msix");
        let second = source_dir.join("second.msix");
        std::fs::write(&first, b"version one").unwrap();
        std::fs::write(&second, b"version two, longer content").unwrap();

        retain_verified_base(&base_dir, &first, "hash-one").unwrap();
        retain_verified_base(&base_dir, &second, "hash-two").unwrap();

        // Exactly one base file + one sidecar -- never one per version.
        let entries: Vec<_> = std::fs::read_dir(&base_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 2, "{entries:?}");

        let (path, sha256) = retained_base(&base_dir).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"version two, longer content");
        assert_eq!(sha256, "hash-two");

        std::fs::remove_dir_all(&base_dir).ok();
        std::fs::remove_dir_all(&source_dir).ok();
    }

    #[test]
    fn a_replace_interrupted_before_the_new_sidecar_is_written_is_not_usable() {
        let base_dir = temp_dir("interrupted-replace");
        let source_dir = temp_dir("interrupted-replace-source");
        let first = source_dir.join("first.msix");
        let second = source_dir.join("second.msix");
        std::fs::write(&first, b"version one").unwrap();
        std::fs::write(&second, b"version two").unwrap();

        retain_verified_base(&base_dir, &first, "hash-one").unwrap();

        // Replay `retain_verified_base`'s first two steps for the second
        // call (copy the new content into place, drop the old sidecar) but
        // stop short of writing the new sidecar -- simulating a crash right
        // there. The old sidecar's value ("hash-one") must never come back
        // out paired with the new base's bytes.
        std::fs::copy(&second, base_dir.join(BASE_FILE_NAME)).unwrap();
        std::fs::remove_file(base_dir.join(BASE_SHA256_FILE_NAME)).unwrap();

        assert!(
            retained_base(&base_dir).is_none(),
            "an interrupted replace must never surface a (new base, stale sha256) pair"
        );

        std::fs::remove_dir_all(&base_dir).ok();
        std::fs::remove_dir_all(&source_dir).ok();
    }

    #[test]
    fn a_base_with_no_sidecar_checksum_is_not_usable() {
        let base_dir = temp_dir("no-sidecar");
        std::fs::write(base_dir.join(BASE_FILE_NAME), b"orphaned base").unwrap();
        assert!(retained_base(&base_dir).is_none());
        std::fs::remove_dir_all(&base_dir).ok();
    }

    #[test]
    fn clear_removes_both_files_and_tolerates_being_called_twice() {
        let base_dir = temp_dir("clear");
        let source_dir = temp_dir("clear-source");
        let source = source_dir.join("Codex.msix");
        std::fs::write(&source, b"bytes").unwrap();
        retain_verified_base(&base_dir, &source, "deadbeef").unwrap();

        clear_retained_base(&base_dir).unwrap();
        assert!(retained_base(&base_dir).is_none());
        assert!(!base_dir.join(BASE_FILE_NAME).exists());
        assert!(!base_dir.join(BASE_SHA256_FILE_NAME).exists());

        // Idempotent: clearing an already-empty directory is not an error.
        clear_retained_base(&base_dir).unwrap();

        std::fs::remove_dir_all(&base_dir).ok();
        std::fs::remove_dir_all(&source_dir).ok();
    }
}
