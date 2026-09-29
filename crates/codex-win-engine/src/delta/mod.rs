//! Windows block-level delta update engine (prototype).
//!
//! Rationale and measurements live in the feasibility report produced before
//! this module existed (byte-identical reconstruction proven with a SHA-256
//! match; 8-55%, median ~29%, savings across 8 real consecutive
//! `codex-app-mirror` release pairs). This module turns that prototype
//! (originally a set of standalone Python scripts) into pure, testable Rust:
//!
//!   - [`zip_format`]: raw ZIP / ZIP64 central-directory + EOCD parsing,
//!     independent of the `zip` crate (which does not expose on-disk byte
//!     offsets, only decompressing reads).
//!   - `crate::appx_blockmap` (one level up, shared with `portable.rs`'s
//!     extractor so the XML traversal exists exactly once): parses
//!     `AppxBlockMap.xml` into per-file, per-block hash/size records.
//!   - [`layout`]: combines the two into one per-entry [`layout::PackageLayout`]
//!     the planner can walk directly (absolute byte offset of every block).
//!   - [`planner`]: pure copy-or-fetch planning with gap coalescing. No I/O,
//!     so it is unit-tested without a network or a real MSIX on disk.
//!   - [`executor`]: runs a plan against a real (`executor::CurlRangeFetcher`,
//!     curl-based, honors [`crate::network::NetworkConfig`]) or fake (tests)
//!     range source, assembles the new package in a staging file, and
//!     requires the assembled file's streamed SHA-256 to equal the value the
//!     caller supplies (from the mirror manifest / `SHA256SUMS-windows.txt`)
//!     before ever returning success. Any failure -- a plan not worth using,
//!     a corrupt/unreadable base, a network error, or a SHA-256 mismatch --
//!     is surfaced as an `Err` so the caller falls back to the existing full
//!     download path; this module never partially "commits" a bad file.
//!   - [`retention`]: keeps at most one verified MSIX on disk as the delta
//!     base for the next update (see that module's doc comment for the disk
//!     cost and where the app's update flow currently keeps/discards its
//!     downloaded MSIX).
//!
//! **Not wired into the app's update flow in this PR.** `perform_windows_update*`
//! in `src-tauri/src/app/win_update.rs` still always does a full download; see
//! the PR description for what integrating this would require and why it is
//! deferred.

pub mod executor;
pub mod layout;
pub mod planner;
pub mod retention;
pub mod zip_format;
