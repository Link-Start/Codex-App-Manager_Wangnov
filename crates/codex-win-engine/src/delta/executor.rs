//! Runs a [`crate::delta::planner`] plan against a real or fake range
//! source, assembling the reconstructed package on disk and requiring its
//! streamed SHA-256 to match before ever returning success.
//!
//! [`RangeFetcher`] is the seam that makes this testable: production code
//! reads ranges over HTTPS via [`CurlRangeFetcher`] (curl, `NetworkConfig`-aware,
//! matching `download.rs`'s conventions -- presigned mirror URLs reject HEAD,
//! so every probe here is itself a ranged GET); tests substitute an
//! in-memory fetcher and exercise the full plan-then-assemble-then-verify
//! pipeline with no network and no real curl binary required.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::delta::layout::build_package_layout;
use crate::delta::planner::{build_reuse_index, plan_delta, DeltaPlan, PlannerConfig};
use crate::delta::zip_format::{ByteSource, InMemorySource};
use crate::network::{is_schannel_revocation_offline, NetworkConfig, SchannelRevocationCheck};
use crate::process::{curl_exe, hidden_command, run_capturing, run_with_progress, RunError, RunLimits};
use crate::EngineError;

/// Anything the delta engine can pull an arbitrary byte range from. Kept
/// separate from [`ByteSource`] (which never fails to know its own length)
/// because a real fetcher's length comes from a network probe that can
/// itself fail.
pub trait RangeFetcher {
    /// Total size of the remote resource, in bytes.
    fn total_len(&self) -> Result<u64, EngineError>;
    /// Fetch exactly `len` bytes starting at `offset`.
    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, EngineError>;
    /// Like [`fetch_range`](Self::fetch_range), but writes the bytes
    /// directly to `dest` at `dest`'s current seek position instead of
    /// returning them, so a caller assembling a large destination file does
    /// not have to hold an entire fetched range (up to several hundred MB
    /// for a coalesced block run) in memory at once, on top of the ~900 MB
    /// base package [`crate::delta::executor::execute_delta`] already holds
    /// resident. The default implementation just delegates to
    /// `fetch_range` -- fine for small reads (layout probing) and for the
    /// fake fetcher tests use; [`CurlRangeFetcher`] overrides it to stream
    /// its curl output file straight into `dest` in bounded chunks instead.
    /// Returns the number of bytes written (equal to `len` on success).
    fn fetch_range_into(&self, offset: u64, len: u64, dest: &mut File) -> Result<u64, EngineError> {
        let bytes = self.fetch_range(offset, len)?;
        dest.write_all(&bytes)
            .map_err(|err| EngineError::Io(format!("write fetched range: {err}")))?;
        Ok(bytes.len() as u64)
    }
}

/// Wraps any [`RangeFetcher`] and records bytes fetched / requests made --
/// the two headline numbers the feasibility report and the example binary
/// both report, plus the final reconstructed SHA-256.
struct CountingFetcher<'a, F: RangeFetcher> {
    inner: &'a F,
    bytes_fetched: Mutex<u64>,
    request_count: Mutex<usize>,
}

impl<'a, F: RangeFetcher> CountingFetcher<'a, F> {
    fn new(inner: &'a F) -> Self {
        Self {
            inner,
            bytes_fetched: Mutex::new(0),
            request_count: Mutex::new(0),
        }
    }

    fn bytes_fetched(&self) -> u64 {
        *self.bytes_fetched.lock().unwrap()
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }
}

impl<F: RangeFetcher> RangeFetcher for CountingFetcher<'_, F> {
    fn total_len(&self) -> Result<u64, EngineError> {
        let len = self.inner.total_len()?;
        // The length probe is itself a real curl invocation (a ranged GET
        // for `CurlRangeFetcher`, since presigned mirror URLs reject
        // `HEAD`), so it must count toward `request_count` the same as
        // every `fetch_range`/`fetch_range_into` call -- otherwise
        // `DeltaOutcome.request_count` (and the example binary's headline
        // number) undercounts the actual number of curl invocations by one.
        *self.request_count.lock().unwrap() += 1;
        Ok(len)
    }

    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, EngineError> {
        let data = self.inner.fetch_range(offset, len)?;
        if data.len() as u64 != len {
            return Err(EngineError::Io(format!(
                "range fetch returned {} bytes, expected {len} (offset={offset})",
                data.len()
            )));
        }
        *self.bytes_fetched.lock().unwrap() += data.len() as u64;
        *self.request_count.lock().unwrap() += 1;
        Ok(data)
    }

    fn fetch_range_into(&self, offset: u64, len: u64, dest: &mut File) -> Result<u64, EngineError> {
        let written = self.inner.fetch_range_into(offset, len, dest)?;
        if written != len {
            return Err(EngineError::Io(format!(
                "range fetch wrote {written} bytes, expected {len} (offset={offset})"
            )));
        }
        *self.bytes_fetched.lock().unwrap() += written;
        *self.request_count.lock().unwrap() += 1;
        Ok(written)
    }
}

/// Adapts a [`RangeFetcher`] into a [`ByteSource`] for `delta::layout` /
/// `delta::zip_format`, which only know about byte ranges, not URLs or
/// fetch-call accounting.
struct FetcherSource<'a, F: RangeFetcher> {
    fetcher: &'a F,
    len: u64,
}

impl<'a, F: RangeFetcher> FetcherSource<'a, F> {
    fn new(fetcher: &'a F) -> Result<Self, EngineError> {
        let len = fetcher.total_len()?;
        Ok(Self { fetcher, len })
    }
}

impl<F: RangeFetcher> ByteSource for FetcherSource<'_, F> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_range(&self, start: u64, len: u64) -> Result<Vec<u8>, EngineError> {
        self.fetcher.fetch_range(start, len)
    }
}

/// Result of a successful delta reconstruction.
#[derive(Debug, Clone)]
pub struct DeltaOutcome {
    /// Total bytes pulled over the network -- layout-probing reads (the
    /// remote tail/EOCD scan, the central directory, `AppxBlockMap.xml`)
    /// plus every planned [`crate::delta::planner::FetchStep`].
    pub bytes_fetched: u64,
    /// Number of curl invocations (or fake-fetcher calls in tests) this
    /// reconstruction made in total.
    pub request_count: usize,
    /// The assembled file's verified SHA-256 (lowercase hex) -- equal to
    /// `expected_sha256` by construction, since a mismatch is an `Err`.
    pub sha256: String,
    /// [`DeltaPlan::savings_pct`] for the plan that was executed.
    pub savings_pct: f64,
    pub new_size: u64,
}

/// Plan and execute a block-level delta reconstruction of `dest_path` from
/// `base_path` (a local file already on disk, fully trusted only after this
/// function's final SHA-256 check) plus `new_fetcher` (the new package,
/// somewhere over the network).
///
/// Returns `Err` on anything that should make the caller fall back to a full
/// download instead of trusting this path: a corrupt/unreadable base or
/// remote layout, a plan that does not clear `min_savings_pct` (the
/// feasibility report's worst observed real pair saved only 8.38%, so a
/// fixed threshold like 15% is a reasonable default -- see that report's
/// caveat (a)), any I/O or network failure, or -- the final safety net -- a
/// reconstructed file whose SHA-256 does not equal `expected_sha256`. A
/// partially written `dest_path` is removed before returning that last
/// error so a caller can never mistake it for a usable file.
pub fn execute_delta<F: RangeFetcher>(
    base_path: &Path,
    new_fetcher: &F,
    dest_path: &Path,
    expected_sha256: &str,
    config: &PlannerConfig,
    min_savings_pct: f64,
) -> Result<DeltaOutcome, EngineError> {
    // The base is read fully into memory rather than streamed: at up to
    // ~900 MB (current x64 MSIX sizes) this is a real but bounded and
    // one-shot cost, and it lets `InMemorySource` serve both the layout
    // parse and every `CopyStep` read with zero extra syscalls. A future,
    // memory-constrained caller could swap this for an `mmap`-backed
    // `ByteSource` without changing anything downstream of `base_source`.
    let base_bytes = std::fs::read(base_path)
        .map_err(|err| EngineError::Io(format!("read base {}: {err}", base_path.display())))?;
    let base_source = InMemorySource::new(&base_bytes);
    let base_layout = build_package_layout(&base_source)
        .map_err(|err| EngineError::Msix(format!("base package is unusable as a delta base: {err}")))?;

    let counting = CountingFetcher::new(new_fetcher);
    let remote_source = FetcherSource::new(&counting)?;
    let new_layout = build_package_layout(&remote_source)?;

    let reuse_index = build_reuse_index(&base_layout);
    let plan = plan_delta(&reuse_index, &new_layout, config);
    if !plan.worth_using(min_savings_pct) {
        return Err(EngineError::Msix(format!(
            "delta plan not worth using: {:.1}% savings is below the {min_savings_pct:.1}% threshold ({} of {} blocks reused)",
            plan.savings_pct(),
            plan.reused_blocks,
            plan.total_blocks,
        )));
    }

    if let Err(err) = assemble(&plan, &base_bytes, &counting, dest_path) {
        let _ = std::fs::remove_file(dest_path);
        return Err(err);
    }

    let actual_sha256 = match crate::download::sha256_file(dest_path) {
        Ok(sha256) => sha256,
        Err(err) => {
            let _ = std::fs::remove_file(dest_path);
            return Err(err);
        }
    };
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = std::fs::remove_file(dest_path);
        return Err(EngineError::Msix(format!(
            "delta reconstruction SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256} -- discarding and falling back to a full download"
        )));
    }

    Ok(DeltaOutcome {
        bytes_fetched: counting.bytes_fetched(),
        request_count: counting.request_count(),
        sha256: actual_sha256,
        savings_pct: plan.savings_pct(),
        new_size: plan.new_size,
    })
}

fn assemble<F: RangeFetcher>(
    plan: &DeltaPlan,
    base_bytes: &[u8],
    fetcher: &CountingFetcher<'_, F>,
    dest_path: &Path,
) -> Result<(), EngineError> {
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| EngineError::Io(format!("create staging dir: {err}")))?;
    }
    let mut out = File::create(dest_path)
        .map_err(|err| EngineError::Io(format!("create {}: {err}", dest_path.display())))?;
    out.set_len(plan.new_size)
        .map_err(|err| EngineError::Io(format!("preallocate {}: {err}", dest_path.display())))?;

    for copy in &plan.copies {
        let start = usize::try_from(copy.base_offset)
            .map_err(|_| EngineError::Msix("copy step base offset overflows usize".to_string()))?;
        let len = usize::try_from(copy.len)
            .map_err(|_| EngineError::Msix("copy step length overflows usize".to_string()))?;
        let slice = base_bytes.get(start..start + len).ok_or_else(|| {
            EngineError::Msix(format!(
                "copy step reads past the end of the base file (offset={start} len={len} base_len={})",
                base_bytes.len()
            ))
        })?;
        out.seek(SeekFrom::Start(copy.new_offset))
            .map_err(|err| EngineError::Io(format!("seek: {err}")))?;
        out.write_all(slice)
            .map_err(|err| EngineError::Io(format!("write: {err}")))?;
    }

    for fetch in &plan.fetches {
        out.seek(SeekFrom::Start(fetch.offset))
            .map_err(|err| EngineError::Io(format!("seek: {err}")))?;
        // `fetch_range_into` (not `fetch_range`) so a large coalesced range
        // streams straight into `out` instead of first materializing as an
        // owned `Vec` on top of the ~900 MB `base_bytes` already resident.
        fetcher.fetch_range_into(fetch.offset, fetch.len, &mut out)?;
    }

    out.flush()
        .map_err(|err| EngineError::Io(format!("flush {}: {err}", dest_path.display())))?;
    Ok(())
}

/// A curl-backed [`RangeFetcher`] over HTTPS, matching `download.rs`'s curl
/// conventions (`-fL`, HTTPS-only, `NetworkConfig`-aware proxy args, the same
/// Schannel-revocation-offline retry). Presigned mirror URLs reject `HEAD`
/// (confirmed in the feasibility report), so [`total_len`](Self::total_len)
/// itself is a ranged GET for `bytes=0-0` (the first byte), reading the
/// total size back out of the response's `Content-Range` header rather than
/// issuing a separate request kind. A *suffix* range (`bytes=-1`, "the last
/// byte") would do the same with a smaller/simpler-looking request, but real
/// presigned URLs disagree on supporting it -- observed live: GitHub
/// Releases' current backend (Azure Blob Storage) answers a suffix range
/// with `501 Not Implemented`, while the S3-backed mirror the feasibility
/// report measured against accepted it. A plain forward range starting at 0
/// is the one form every backend observed so far accepts.
pub struct CurlRangeFetcher<'a> {
    url: &'a str,
    network: &'a NetworkConfig,
    tmp_dir: PathBuf,
}

impl<'a> CurlRangeFetcher<'a> {
    pub fn new(url: &'a str, network: &'a NetworkConfig, tmp_dir: impl Into<PathBuf>) -> Self {
        Self {
            url,
            network,
            tmp_dir: tmp_dir.into(),
        }
    }

    /// `progress_path`, when given, is polled for its file size to detect a
    /// stalled transfer (`limits.stall`, if set, is otherwise never enforced
    /// -- `run_capturing`'s no-progress-callback form only ever checks the
    /// total deadline). It should be the path curl is writing its `-o`
    /// output to; growth in that file's size is curl making progress.
    fn run_curl(
        &self,
        extra_args: &[String],
        limits: RunLimits,
        progress_path: Option<&Path>,
    ) -> Result<std::process::Output, EngineError> {
        let attempt = |revocation: SchannelRevocationCheck| -> Result<std::process::Output, RunError> {
            let mut command = hidden_command(curl_exe());
            let mut args = self.network.curl_args_with_schannel_revocation(revocation);
            args.extend([
                "-fL".to_string(),
                "--proto".to_string(),
                "=https".to_string(),
                "--proto-redir".to_string(),
                "=https".to_string(),
                "-sS".to_string(),
                "--connect-timeout".to_string(),
                "20".to_string(),
            ]);
            args.extend_from_slice(extra_args);
            args.push(self.url.to_string());
            command.args(args);
            match progress_path {
                Some(path) => run_with_progress(
                    command,
                    limits,
                    None,
                    &|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
                    &|_| {},
                ),
                None => run_capturing(command, limits, None),
            }
        };

        match attempt(SchannelRevocationCheck::Strict) {
            Ok(output) if output.status.success() => Ok(output),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if is_schannel_revocation_offline(output.status.code(), &stderr) {
                    let retried = attempt(SchannelRevocationCheck::Disabled)
                        .map_err(|err| EngineError::Io(format!("curl: {}", err.message())))?;
                    if retried.status.success() {
                        Ok(retried)
                    } else {
                        Err(EngineError::Io(format!(
                            "curl failed (exit={:?}): {}",
                            retried.status.code(),
                            String::from_utf8_lossy(&retried.stderr).trim()
                        )))
                    }
                } else {
                    Err(EngineError::Io(format!(
                        "curl failed (exit={:?}): {}",
                        output.status.code(),
                        stderr.trim()
                    )))
                }
            }
            Err(err) => Err(EngineError::Io(format!("curl: {}", err.message()))),
        }
    }

    fn unique_tmp_path(&self, prefix: &str) -> PathBuf {
        self.tmp_dir.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }
}

impl RangeFetcher for CurlRangeFetcher<'_> {
    fn total_len(&self) -> Result<u64, EngineError> {
        std::fs::create_dir_all(&self.tmp_dir)
            .map_err(|err| EngineError::Io(format!("create tmp dir: {err}")))?;
        let body = self.unique_tmp_path("delta-probe-body");
        let headers = self.unique_tmp_path("delta-probe-headers");
        let body_str = body.to_string_lossy().into_owned();
        let headers_str = headers.to_string_lossy().into_owned();

        // "bytes=0-0": the server's first byte. A short, cheap probe whose
        // response header carries the resource's total size, without
        // relying on `HEAD` (rejected by the presigned mirror URLs this is
        // built for, and by GitHub's own release-asset redirect target) or
        // downloading the resource itself. Same aliasing risk as the payload
        // fetches below: an origin or proxy that ignores `Range` and answers
        // `200` with the whole package would otherwise make this probe write
        // the entire ~900 MB response to disk before the missing/mismatched
        // `Content-Range` check below ever ran. `--max-filesize` bounds that
        // to one probe-sized body instead (curl aborts with a non-zero exit,
        // caught by `result?`, once the response exceeds the cap).
        const PROBE_MAX_BODY_BYTES: u64 = 64 * 1024;
        let result = self.run_curl(
            &[
                "-r".to_string(),
                "0-0".to_string(),
                "--max-filesize".to_string(),
                PROBE_MAX_BODY_BYTES.to_string(),
                "-D".to_string(),
                headers_str.clone(),
                "-o".to_string(),
                body_str.clone(),
            ],
            RunLimits::total(Duration::from_secs(30)),
            None,
        );
        let header_text = std::fs::read_to_string(&headers).unwrap_or_default();
        let _ = std::fs::remove_file(&body);
        let _ = std::fs::remove_file(&headers);
        result?;

        // Require an actual `206 Partial Content` for exactly `bytes 0-0`,
        // not just *a* Content-Range-shaped header -- the same aliasing an
        // ignored Range could produce if a proxy happened to echo one back
        // on a `200`.
        validate_range_response(&header_text, 0, 0).map_err(|err| {
            EngineError::Io(format!(
                "length probe for {} did not behave like a Range-capable server: {err}",
                self.url
            ))
        })?;

        parse_content_range_total(&header_text).ok_or_else(|| {
            EngineError::Io(format!(
                "no Content-Range header in response for {} -- does it support HTTP Range requests?",
                self.url
            ))
        })
    }

    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, EngineError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let body = self.fetch_range_to_temp_file(offset, len)?;
        // Clean up the temp file on every path -- a failed or timed-out curl
        // invocation can still have written a partial body before erroring,
        // and repeated failed delta attempts must not accumulate large
        // partial-range files in `tmp_dir`.
        let read_result =
            std::fs::read(&body).map_err(|err| EngineError::Io(format!("read fetched range: {err}")));
        let _ = std::fs::remove_file(&body);
        read_result
    }

    fn fetch_range_into(&self, offset: u64, len: u64, dest: &mut File) -> Result<u64, EngineError> {
        if len == 0 {
            return Ok(0);
        }
        let body = self.fetch_range_to_temp_file(offset, len)?;
        // Stream the curl output file into `dest` in bounded chunks (via
        // `io::copy`'s fixed-size internal buffer) rather than reading it
        // fully into an owned `Vec` first -- the whole point of this method
        // over `fetch_range` is to keep a large coalesced range (up to
        // several hundred MB) from ever being resident in memory at once.
        let copy_result = (|| -> Result<u64, EngineError> {
            let mut reader = std::fs::File::open(&body)
                .map_err(|err| EngineError::Io(format!("open fetched range: {err}")))?;
            std::io::copy(&mut reader, dest)
                .map_err(|err| EngineError::Io(format!("copy fetched range: {err}")))
        })();
        let _ = std::fs::remove_file(&body);
        copy_result
    }
}

impl CurlRangeFetcher<'_> {
    /// Range-fetch `len` bytes starting at `offset` into a freshly named
    /// temp file under `tmp_dir` and return its path. The caller owns
    /// removing that file (on every path, including error, since a
    /// failed/timed-out curl invocation can still have written a partial
    /// body).
    fn fetch_range_to_temp_file(&self, offset: u64, len: u64) -> Result<PathBuf, EngineError> {
        std::fs::create_dir_all(&self.tmp_dir)
            .map_err(|err| EngineError::Io(format!("create tmp dir: {err}")))?;
        let body = self.unique_tmp_path("delta-range-body");
        let headers = self.unique_tmp_path("delta-range-headers");
        let body_str = body.to_string_lossy().into_owned();
        let headers_str = headers.to_string_lossy().into_owned();
        let end_inclusive = offset + len - 1;

        let result = self.run_curl(
            &[
                "-r".to_string(),
                format!("{offset}-{end_inclusive}"),
                // If an origin or proxy ignores the Range request and
                // answers `200` with the entire package, curl's exit status
                // alone would still look like success -- `-fL` only treats
                // HTTP error *statuses* (>=400) as failure, not an ignored
                // Range. `--max-filesize` bounds the resulting waste: curl
                // aborts (a non-zero exit, caught below) once the response
                // body exceeds `len` bytes, instead of downloading the
                // whole ~900 MB package before the final byte-count check
                // in `CountingFetcher`/`FetcherSource` would have caught it
                // anyway.
                "--max-filesize".to_string(),
                len.to_string(),
                "-D".to_string(),
                headers_str.clone(),
                "-o".to_string(),
                body_str.clone(),
            ],
            RunLimits::with_stall(Duration::from_secs(30 * 60), Duration::from_secs(90)),
            Some(&body),
        );
        let header_text = std::fs::read_to_string(&headers).unwrap_or_default();
        let _ = std::fs::remove_file(&headers);
        let validated = result.and_then(|_| {
            validate_range_response(&header_text, offset, end_inclusive).map_err(|err| {
                EngineError::Msix(format!(
                    "range fetch for bytes {offset}-{end_inclusive} of {}: {err}",
                    self.url
                ))
            })
        });
        match validated {
            Ok(()) => Ok(body),
            Err(err) => {
                let _ = std::fs::remove_file(&body);
                Err(err)
            }
        }
    }
}

/// Require the response curl just wrote `body` from to actually be the
/// `206 Partial Content` response for exactly `bytes {offset}-{end_inclusive}`
/// that was requested -- not, say, a `200` with the full resource because an
/// origin or proxy silently ignored the `Range` header. Checked against the
/// *last* status/`Content-Range` header block in a `-fL` header dump so a
/// redirect chain's final response is what gets validated.
fn validate_range_response(headers: &str, offset: u64, end_inclusive: u64) -> Result<(), String> {
    match parse_last_status_code(headers) {
        Some(206) => {}
        Some(other) => return Err(format!("expected HTTP 206 Partial Content, got {other}")),
        None => return Err("no HTTP status line in response headers".to_string()),
    }
    match parse_content_range_start_end(headers) {
        Some((start, end)) if start == offset && end == end_inclusive => Ok(()),
        Some((start, end)) => Err(format!(
            "Content-Range bytes {start}-{end} does not match the requested {offset}-{end_inclusive}"
        )),
        None => Err("no Content-Range header in response".to_string()),
    }
}

/// Parse curl's `-D` header dump for the last `HTTP/<version> <code> ...`
/// status line (may contain one per redirect hop -- the final hop's status
/// is what matters).
fn parse_last_status_code(headers: &str) -> Option<u16> {
    headers.lines().rev().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("HTTP/")?
            .split_whitespace()
            .nth(1)?
            .parse::<u16>()
            .ok()
    })
}

/// Like [`parse_content_range_total`], but returns the response's declared
/// `(start, end)` byte range instead of the total resource size.
fn parse_content_range_start_end(headers: &str) -> Option<(u64, u64)> {
    headers.lines().rev().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("content-range") {
            return None;
        }
        let range = value.trim().strip_prefix("bytes ")?.split_once('/')?.0;
        let (start, end) = range.split_once('-')?;
        Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
    })
}

/// Parse `Content-Range: bytes X-Y/TOTAL` out of a raw curl `-D` header dump
/// (which, with `-fL`, may contain one header block per redirect hop) and
/// return `TOTAL`. The *last* occurrence in the text is used so a redirect
/// chain's final response wins over an intermediate hop's headers.
fn parse_content_range_total(headers: &str) -> Option<u64> {
    headers.lines().rev().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("content-range") {
            return None;
        }
        value.trim().rsplit('/').next()?.trim().parse::<u64>().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::DeflateEncoder;
    use flate2::Compression;

    #[test]
    fn parses_content_range_total_from_a_header_dump() {
        let headers = "HTTP/2 206\r\ncontent-range: bytes 876623360-876623360/876623361\r\naccept-ranges: bytes\r\n\r\n";
        assert_eq!(parse_content_range_total(headers), Some(876623361));
    }

    #[test]
    fn parses_content_range_total_preferring_the_final_redirect_hop() {
        let headers = "\
HTTP/2 302\r\nlocation: https://mirror.example/final\r\n\r\n\
HTTP/2 206\r\nContent-Range: bytes 0-0/123456\r\n\r\n";
        assert_eq!(parse_content_range_total(headers), Some(123456));
    }

    #[test]
    fn missing_content_range_header_is_none() {
        let headers = "HTTP/2 200\r\ncontent-length: 42\r\n\r\n";
        assert_eq!(parse_content_range_total(headers), None);
    }

    #[test]
    fn validate_range_response_accepts_a_matching_206() {
        let headers = "HTTP/2 206\r\ncontent-range: bytes 100-199/876623361\r\n\r\n";
        assert!(validate_range_response(headers, 100, 199).is_ok());
    }

    #[test]
    fn validate_range_response_rejects_an_ignored_range_answered_with_200() {
        // An origin/proxy that ignores `Range` and returns the whole
        // resource -- exactly the failure mode a `--max-filesize` cap and
        // this status check exist to catch quickly instead of trusting a
        // merely-successful curl exit.
        let headers = "HTTP/2 200\r\ncontent-length: 876623361\r\n\r\n";
        let err = validate_range_response(headers, 100, 199).unwrap_err();
        assert!(err.contains("206"), "{err}");
    }

    #[test]
    fn validate_range_response_rejects_a_content_range_for_the_wrong_bytes() {
        let headers = "HTTP/2 206\r\ncontent-range: bytes 0-99/876623361\r\n\r\n";
        let err = validate_range_response(headers, 100, 199).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn validate_range_response_prefers_the_final_redirect_hops_status() {
        let headers = "\
HTTP/2 302\r\nlocation: https://mirror.example/final\r\n\r\n\
HTTP/2 206\r\ncontent-range: bytes 100-199/876623361\r\n\r\n";
        assert!(validate_range_response(headers, 100, 199).is_ok());
    }

    // ---- Fake, in-memory RangeFetcher + synthetic MSIX-like ZIP builder for
    // full plan -> assemble -> verify pipeline tests with no network at all.

    struct FakeFetcher {
        data: Vec<u8>,
    }

    impl RangeFetcher for FakeFetcher {
        fn total_len(&self) -> Result<u64, EngineError> {
            Ok(self.data.len() as u64)
        }

        fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, EngineError> {
            let start = usize::try_from(offset)
                .map_err(|_| EngineError::Msix("offset overflows usize".to_string()))?;
            let len = usize::try_from(len)
                .map_err(|_| EngineError::Msix("len overflows usize".to_string()))?;
            self.data
                .get(start..start + len)
                .map(|slice| slice.to_vec())
                .ok_or_else(|| {
                    EngineError::Msix(format!(
                        "fake fetch out of range: start={start} len={len} total={}",
                        self.data.len()
                    ))
                })
        }
    }

    fn le16(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn le32(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    struct BlockSpec {
        content: Vec<u8>,
        hash: &'static str,
        /// `true`: independently deflate this block's bytes (mirrors real
        /// MSIX block compression -- every 64 KiB chunk is its own deflate
        /// stream). `false`: store the raw bytes, matching how
        /// `appx_blockmap`'s parser derives a stored block's implied size.
        compress: bool,
    }

    struct FileSpec {
        name: &'static str,
        blocks: Vec<BlockSpec>,
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// Build a full, real, byte-level synthetic MSIX-like ZIP: one local
    /// header + on-disk block data per [`FileSpec`], a real (deflate
    /// compressed, matching production MSIX packages)
    /// `AppxBlockMap.xml` entry describing those files' blocks, and a
    /// standard (non-ZIP64 -- ZIP64 parsing itself is covered byte-for-byte
    /// in `zip_format`'s own tests) central directory + EOCD.
    fn build_package(files: &[FileSpec]) -> Vec<u8> {
        struct Resolved {
            name: &'static str,
            lho: u32,
            csize: u32,
            usize_: u32,
            method: u16,
        }

        let mut out = Vec::new();
        let mut resolved = Vec::new();
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<BlockMap xmlns=\"http://schemas.microsoft.com/appx/2010/blockmap\" HashMethod=\"http://www.w3.org/2001/04/xmlenc#sha256\">\n",
        );

        for file in files {
            let lho = out.len() as u32;
            let mut data = Vec::new();
            let mut usize_total = 0u64;
            let mut method = 0u16;
            let mut block_xml = String::new();
            for block in &file.blocks {
                usize_total += block.content.len() as u64;
                let on_disk = if block.compress {
                    method = 8;
                    deflate(&block.content)
                } else {
                    block.content.clone()
                };
                if block.compress {
                    block_xml.push_str(&format!(
                        "    <Block Hash=\"{}\" Size=\"{}\" />\n",
                        block.hash,
                        on_disk.len()
                    ));
                } else {
                    block_xml.push_str(&format!("    <Block Hash=\"{}\" />\n", block.hash));
                }
                data.extend_from_slice(&on_disk);
            }
            let name_bytes = file.name.as_bytes();
            let lfh_size = 30 + name_bytes.len() as u64;

            out.extend_from_slice(&le32(0x0403_4b50));
            out.extend_from_slice(&le16(20));
            out.extend_from_slice(&le16(0));
            out.extend_from_slice(&le16(method));
            out.extend_from_slice(&le16(0));
            out.extend_from_slice(&le16(0));
            out.extend_from_slice(&le32(0)); // crc32: never checked by the delta engine
            out.extend_from_slice(&le32(data.len() as u32));
            out.extend_from_slice(&le32(usize_total as u32));
            out.extend_from_slice(&le16(name_bytes.len() as u16));
            out.extend_from_slice(&le16(0));
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(&data);

            resolved.push(Resolved {
                name: file.name,
                lho,
                csize: data.len() as u32,
                usize_: usize_total as u32,
                method,
            });
            xml.push_str(&format!(
                "  <File Name=\"{}\" Size=\"{}\" LfhSize=\"{}\">\n{}  </File>\n",
                file.name.replace('/', "\\"),
                usize_total,
                lfh_size,
                block_xml
            ));
        }
        xml.push_str("</BlockMap>");

        let xml_bytes = xml.into_bytes();
        let xml_compressed = deflate(&xml_bytes);
        let bm_name = b"AppxBlockMap.xml";
        let bm_lho = out.len() as u32;
        out.extend_from_slice(&le32(0x0403_4b50));
        out.extend_from_slice(&le16(20));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(8));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le32(0));
        out.extend_from_slice(&le32(xml_compressed.len() as u32));
        out.extend_from_slice(&le32(xml_bytes.len() as u32));
        out.extend_from_slice(&le16(bm_name.len() as u16));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(bm_name);
        out.extend_from_slice(&xml_compressed);

        let mut cd = Vec::new();
        let cd_offset = out.len() as u32;
        let cd_record = |cd: &mut Vec<u8>, name: &[u8], method: u16, csize: u32, usize_: u32, lho: u32| {
            cd.extend_from_slice(&le32(0x0201_4b50));
            cd.extend_from_slice(&le16(20));
            cd.extend_from_slice(&le16(20));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le16(method));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le32(0));
            cd.extend_from_slice(&le32(csize));
            cd.extend_from_slice(&le32(usize_));
            cd.extend_from_slice(&le16(name.len() as u16));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le16(0));
            cd.extend_from_slice(&le32(0));
            cd.extend_from_slice(&le32(lho));
            cd.extend_from_slice(name);
        };
        for r in &resolved {
            cd_record(&mut cd, r.name.as_bytes(), r.method, r.csize, r.usize_, r.lho);
        }
        cd_record(
            &mut cd,
            bm_name,
            8,
            xml_compressed.len() as u32,
            xml_bytes.len() as u32,
            bm_lho,
        );

        let cd_size = cd.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(0));
        let total_entries = (resolved.len() + 1) as u16;
        out.extend_from_slice(&le16(total_entries));
        out.extend_from_slice(&le16(total_entries));
        out.extend_from_slice(&le32(cd_size));
        out.extend_from_slice(&le32(cd_offset));
        out.extend_from_slice(&le16(0));

        out
    }

    fn write_temp_file(name: &str, data: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, data).unwrap();
        path
    }

    /// Deterministic, poorly-compressible filler content (a simple LCG), so
    /// deflate cannot collapse a whole block down to a handful of bytes the
    /// way it would for e.g. a block of all-zero bytes. Real MSIX payloads
    /// (executables, native binaries) are similarly incompressible, and the
    /// test below needs the package's *on-disk* size to be large enough that
    /// the layout-discovery reads (the EOCD tail scan reads
    /// `min(file_size, 1 MiB)` -- see `zip_format::parse_zip_layout` --
    /// which is negligible overhead against a real ~800 MB MSIX but would
    /// dwarf a tiny all-zero-block test fixture) stay a small fraction of
    /// the whole package, so the assembled fetch/reuse numbers reflect the
    /// plan's real savings rather than test-fixture-scale noise.
    fn filler(seed: u32, len: usize) -> Vec<u8> {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect()
    }

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn end_to_end_reconstructs_byte_identical_package_with_deflated_blocks() {
        // Base: one large block-compressed file, real (deflate) block
        // compression like a real MSIX -- mirrors chrome.dll/node.exe-style
        // "unchanged shell" reuse from the feasibility report. ~1.5MB/block
        // of low-compressibility filler so the package's on-disk size makes
        // the fixed ~1MB layout-discovery overhead a minority of the total.
        const BLOCK_LEN: usize = 1_500_000;
        let unchanged_1 = filler(1, BLOCK_LEN);
        let unchanged_2 = filler(2, BLOCK_LEN);
        let old_block_2 = filler(3, BLOCK_LEN);
        let new_block_2 = filler(4, BLOCK_LEN);
        let base = build_package(&[FileSpec {
            name: "app/chrome.dll",
            blocks: vec![
                BlockSpec { content: unchanged_1.clone(), hash: "h-unchanged-1", compress: true },
                BlockSpec { content: old_block_2, hash: "h-will-change", compress: true },
                BlockSpec { content: unchanged_2.clone(), hash: "h-unchanged-2", compress: true },
            ],
        }]);
        // New: block 2 changed (Codex's own rebuilt payload, per the
        // feasibility report's per-file breakdown), plus a brand-new
        // ancillary file the base never had at all.
        let new_pkg = build_package(&[
            FileSpec {
                name: "app/chrome.dll",
                blocks: vec![
                    BlockSpec { content: unchanged_1, hash: "h-unchanged-1", compress: true },
                    BlockSpec { content: new_block_2, hash: "h-changed-now", compress: true },
                    BlockSpec { content: unchanged_2, hash: "h-unchanged-2", compress: true },
                ],
            },
            FileSpec {
                name: "app/resources/new-file.bin",
                blocks: vec![BlockSpec { content: vec![0xEE; 1_000], hash: "h-brand-new", compress: false }],
            },
        ]);

        let base_path = write_temp_file("base", &base);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-dest-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let expected_sha256 = sha256_hex(&new_pkg);
        let fetcher = FakeFetcher { data: new_pkg.clone() };

        let outcome = execute_delta(
            &base_path,
            &fetcher,
            &dest_path,
            &expected_sha256,
            &PlannerConfig::default(),
            0.0,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest_path).unwrap(), new_pkg, "byte-identical reconstruction");
        assert_eq!(outcome.sha256, expected_sha256);
        assert_eq!(outcome.new_size, new_pkg.len() as u64);
        // Two 1.5MB blocks were reused out of a ~4.5MB+ package: real,
        // meaningful savings even after the fixed layout-discovery overhead.
        assert!(
            outcome.bytes_fetched < new_pkg.len() as u64,
            "expected savings, fetched {} of {} bytes",
            outcome.bytes_fetched,
            new_pkg.len()
        );
        assert!(outcome.savings_pct > 0.0);
        assert!(outcome.request_count > 0);

        let _ = std::fs::remove_file(&base_path);
        let _ = std::fs::remove_file(&dest_path);
    }

    #[test]
    fn corrupted_base_file_fails_closed_so_caller_can_fall_back() {
        let base_path = write_temp_file("corrupt-base", b"this is not a zip file at all");
        let new_pkg = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: vec![1u8; 1_000], hash: "h1", compress: false }],
        }]);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-dest-corrupt-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let expected_sha256 = sha256_hex(&new_pkg);
        let fetcher = FakeFetcher { data: new_pkg };

        let err = execute_delta(
            &base_path,
            &fetcher,
            &dest_path,
            &expected_sha256,
            &PlannerConfig::default(),
            0.0,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("unusable as a delta base"),
            "unexpected error: {err}"
        );
        assert!(!dest_path.exists());

        let _ = std::fs::remove_file(&base_path);
    }

    #[test]
    fn a_block_whose_compression_changed_is_fetched_fresh_not_reused() {
        // Base stores this file's payload uncompressed (`stored`); the new
        // package block-compresses the very same logical content. Per the
        // feasibility report's caveat (b) -- "a future upstream compression
        // /packaging change... could change block sizes/hashes and silently
        // reduce reuse" -- the (hash, on-disk-size) key the planner looks
        // up on therefore does not match, and the block must be fetched
        // fresh rather than (incorrectly) copied from the base's differently
        // encoded bytes.
        let payload = vec![0x42; 50_000];
        let base = build_package(&[FileSpec {
            name: "app/codex.exe",
            blocks: vec![BlockSpec { content: payload.clone(), hash: "h-stored", compress: false }],
        }]);
        let new_pkg = build_package(&[FileSpec {
            name: "app/codex.exe",
            blocks: vec![BlockSpec { content: payload, hash: "h-deflated", compress: true }],
        }]);

        let base_path = write_temp_file("recompressed-base", &base);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-dest-recompressed-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let expected_sha256 = sha256_hex(&new_pkg);
        let fetcher = FakeFetcher { data: new_pkg.clone() };

        let outcome = execute_delta(
            &base_path,
            &fetcher,
            &dest_path,
            &expected_sha256,
            &PlannerConfig::default(),
            // No savings expected here (the only payload block changed
            // encoding) -- only metadata (LFH + central directory) would
            // ever be "reused" by omission, so allow any non-negative plan.
            f64::MIN,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest_path).unwrap(), new_pkg);
        assert_eq!(outcome.sha256, expected_sha256);
        // The payload was entirely re-fetched -- reused_blocks would be 0 in
        // the underlying plan; observable here as fetched bytes covering
        // essentially the whole new package.
        assert!(outcome.bytes_fetched as f64 >= new_pkg.len() as f64 * 0.9);

        let _ = std::fs::remove_file(&base_path);
        let _ = std::fs::remove_file(&dest_path);
    }
}
