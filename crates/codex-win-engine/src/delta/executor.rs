//! Runs a [`crate::delta::planner`] plan against a real or fake range
//! source, assembling the reconstructed package on disk and requiring its
//! streamed SHA-256 to match before ever returning success.
//!
//! [`RangeFetcher`] is the seam that makes this testable: production code
//! reads ranges over HTTPS via [`CurlRangeFetcher`] (see [`crate::delta::http`]:
//! curl, `NetworkConfig`-aware, redirect pinning and bounded retries --
//! presigned mirror URLs reject HEAD, so every probe is itself a ranged GET);
//! tests substitute an
//! in-memory fetcher and exercise the full plan-then-assemble-then-verify
//! pipeline with no network and no real curl binary required.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use crate::delta::layout::build_package_layout;
use crate::delta::planner::{build_reuse_index, plan_delta, DeltaPlan, PlannerConfig};
use crate::delta::zip_format::{ByteSource, InMemorySource};
pub use crate::delta::http::{CurlRangeFetcher, RetryPolicy, RetryStats};
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
    /// Retries and URL re-resolutions performed so far, for reporting. Fakes
    /// that never retry keep the zero default.
    fn retry_stats(&self) -> RetryStats {
        RetryStats::default()
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

    fn retry_stats(&self) -> RetryStats {
        self.inner.retry_stats()
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
    /// Number of successful requests (curl invocations, or fake-fetcher calls
    /// in tests): the length probe plus every layout-probing and planned
    /// range fetch. Retried and re-resolve requests are reported separately
    /// in [`retry_stats`](Self::retry_stats); total curl invocations are
    /// `request_count + retry_stats.retries + retry_stats.re_resolves`.
    pub request_count: usize,
    /// Retries after transient failures and URL re-resolutions.
    pub retry_stats: RetryStats,
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
    // A quick, friendly rejection for the obvious misuse -- the same literal
    // path for both arguments (nothing upstream of this function stops a
    // caller; the example binary's optional destination argument in
    // particular makes it one flag away). This is a diagnostic, not the
    // safety net: `canonicalize` resolves symlinks but not a *hard* link
    // (two distinct directory entries genuinely sharing one inode, which
    // canonicalize cannot see through), so it cannot catch every form of
    // aliasing on its own. The actual safety net is structural, below: this
    // function never writes through `dest_path` at all until the very last
    // step, once the whole reconstruction is already fetched, assembled,
    // and SHA-256-verified in an independent staging file -- so even an
    // aliasing form this check misses can, at worst, replace `dest_path`
    // (and whatever else happens to share its inode) with fully verified
    // bytes at the very end, never a truncated or partially written base.
    let same_file = std::fs::canonicalize(base_path)
        .ok()
        .zip(std::fs::canonicalize(dest_path).ok())
        .map(|(base, dest)| base == dest)
        .unwrap_or_else(|| base_path == dest_path);
    if same_file {
        return Err(EngineError::Msix(format!(
            "delta destination {} must not be the same file as the base {}",
            dest_path.display(),
            base_path.display()
        )));
    }

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

    // Assemble and verify in an independent staging file next to
    // `dest_path` (so the final rename below stays on one filesystem),
    // never `dest_path` itself: this is what actually makes aliasing safe
    // regardless of its form (same path, symlink, or a hard link the
    // `canonicalize` check above cannot see through) -- `dest_path` (and
    // therefore `base_path`, if a caller did alias them) is not written to
    // at all until the single rename at the very end, by which point the
    // reconstructed bytes are already fully fetched, assembled, and
    // SHA-256-verified. A staging file left behind by any failure along the
    // way is this function's own to clean up; `dest_path` is never touched.
    let mut staging_name = dest_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("delta-staging"));
    staging_name.push(format!(".part-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
    let staging_path = dest_path.with_file_name(staging_name);

    if let Err(err) = assemble(&plan, &base_bytes, &counting, &staging_path) {
        let _ = std::fs::remove_file(&staging_path);
        return Err(err);
    }

    let actual_sha256 = match crate::download::sha256_file(&staging_path) {
        Ok(sha256) => sha256,
        Err(err) => {
            let _ = std::fs::remove_file(&staging_path);
            return Err(err);
        }
    };
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = std::fs::remove_file(&staging_path);
        return Err(EngineError::Msix(format!(
            "delta reconstruction SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256} -- discarding and falling back to a full download"
        )));
    }

    if let Err(err) = std::fs::rename(&staging_path, dest_path) {
        let _ = std::fs::remove_file(&staging_path);
        return Err(EngineError::Io(format!(
            "rename verified reconstruction to {}: {err}",
            dest_path.display()
        )));
    }

    Ok(DeltaOutcome {
        bytes_fetched: counting.bytes_fetched(),
        request_count: counting.request_count(),
        retry_stats: counting.retry_stats(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::path::PathBuf;

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
    fn reconstructs_through_a_pinned_url_with_retries_and_a_mid_run_re_resolve() {
        use crate::delta::http::test_support::{http_failure, new_session, transport_failure, Call, FakeTransport};

        // Five blocks; blocks 1 and 3 change, so with a zero coalescing gap
        // the plan has separate fetch ranges in addition to the layout reads.
        const BLOCK_LEN: usize = 300_000;
        let hashes = ["h0", "h1-old", "h2", "h3-old", "h4"];
        let new_hashes = ["h0", "h1-new", "h2", "h3-new", "h4"];
        let make = |hs: &[&'static str], seed_shift: u32| {
            build_package(&[FileSpec {
                name: "app/big.dll",
                blocks: hs
                    .iter()
                    .enumerate()
                    .map(|(i, h)| {
                        let changed = h.ends_with("-new");
                        BlockSpec {
                            content: filler(i as u32 + if changed { seed_shift } else { 0 }, BLOCK_LEN),
                            hash: h,
                            compress: true,
                        }
                    })
                    .collect(),
            }])
        };
        let base = make(&hashes, 0);
        let new_pkg = make(&new_hashes, 100);
        let base_path = write_temp_file("pinned-base", &base);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-dest-pinned-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let expected_sha256 = sha256_hex(&new_pkg);

        let transport = FakeTransport::new(new_pkg.clone(), true);
        // The first two layout reads hit a 429 (Retry-After) and a curl
        // exit 56; then the presigned URL "expires" before the 5th range call.
        transport.fail_ranges(vec![http_failure(429, Some(1)), transport_failure(56)]);
        *transport.expire_before_range_call.lock().unwrap() = Some(6);
        let (session, sleeps) = new_session(transport, RetryPolicy::default());

        let outcome = execute_delta(
            &base_path,
            &session,
            &dest_path,
            &expected_sha256,
            &PlannerConfig { coalesce_gap: 0 },
            0.0,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest_path).unwrap(), new_pkg, "byte-identical");
        assert_eq!(outcome.sha256, expected_sha256);
        assert_eq!(outcome.retry_stats.retries, 2);
        assert_eq!(outcome.retry_stats.re_resolves, 1);
        assert_eq!(sleeps.lock().unwrap().len(), 2);
        assert!(outcome.savings_pct > 0.0, "some blocks were reused");

        let transport = session.transport();
        // Only probes ever touch the router URL; every range goes to a
        // presigned URL.
        for call in transport.calls() {
            if let Call::Range { url, .. } = call {
                assert!(url.starts_with("https://cdn.example/"), "{url}");
            }
        }
        assert_eq!(transport.probe_count(), 2, "initial resolve + one re-resolve");

        let _ = std::fs::remove_file(&base_path);
        let _ = std::fs::remove_file(&dest_path);
    }

    #[test]
    fn persistent_throttling_surfaces_an_error_and_leaves_no_destination() {
        use crate::delta::http::test_support::{http_failure, new_session, FakeTransport};

        let base = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: filler(1, 100_000), hash: "h1", compress: true }],
        }]);
        let new_pkg = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: filler(2, 100_000), hash: "h2", compress: true }],
        }]);
        let base_path = write_temp_file("throttled-base", &base);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-dest-throttled-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let transport = FakeTransport::new(new_pkg.clone(), true);
        transport.fail_ranges((0..50).map(|_| http_failure(429, Some(1))).collect());
        let (session, _) = new_session(transport, RetryPolicy::default());

        let err = execute_delta(
            &base_path,
            &session,
            &dest_path,
            &sha256_hex(&new_pkg),
            &PlannerConfig::default(),
            0.0,
        )
        .unwrap_err();
        assert!(err.to_string().contains("HTTP 429"), "{err}");
        assert!(!dest_path.exists());
        assert_eq!(session.transport().range_calls().len(), 4, "bounded by max_attempts");

        let _ = std::fs::remove_file(&base_path);
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
    fn a_destination_that_aliases_the_base_is_rejected_before_touching_either_file() {
        // A caller that (mistakenly, or via the example binary's optional
        // destination argument) passes the same literal path for both
        // `base_path` and `dest_path` gets a clear, immediate error rather
        // than silently having that base replaced -- see the comment at the
        // top of `execute_delta`. The retained base's real bytes on disk are
        // the signal this test checks: they must be completely untouched.
        // (A hard-link alias, which this literal-path check cannot see
        // through, is covered separately below: the staging-file mechanism
        // that check's own comment describes handles that case safely too,
        // without ever rejecting it.)
        let base_content = b"this is a base file, not touched by this call at all".to_vec();
        let base_path = write_temp_file("alias-base", &base_content);
        let new_pkg = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: vec![1u8; 1_000], hash: "h1", compress: false }],
        }]);
        let expected_sha256 = sha256_hex(&new_pkg);
        let fetcher = FakeFetcher { data: new_pkg };

        let err = execute_delta(
            &base_path,
            &fetcher,
            &base_path,
            &expected_sha256,
            &PlannerConfig::default(),
            0.0,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("must not be the same file"),
            "unexpected error: {err}"
        );
        assert_eq!(
            std::fs::read(&base_path).unwrap(),
            base_content,
            "the base file must be byte-for-byte untouched by a rejected call"
        );

        let _ = std::fs::remove_file(&base_path);
    }

    #[test]
    fn a_hard_linked_destination_leaves_the_base_intact_after_a_successful_reconstruction() {
        // `canonicalize` (used by the literal-path check above) cannot see
        // through a hard link: `dest_path` here is a second directory entry
        // for the exact same inode as `base_path`, under a different name,
        // so that check does not fire and this call proceeds. Safety here
        // comes from the structural fix instead -- `execute_delta` never
        // writes through `dest_path` (or anything sharing its inode) until
        // the final rename of an already fully verified staging file, so
        // `base_path`'s *original* directory entry -- unaffected by a
        // rename that only ever replaces `dest_path`'s entry -- must still
        // read back the untouched old content even though it shares an
        // inode with a destination that just got legitimately overwritten.
        // Must be a real, parseable package -- `execute_delta` parses
        // `base_path` as a ZIP/AppxBlockMap layout before anything else.
        let base_content = build_package(&[FileSpec {
            name: "app/original.bin",
            blocks: vec![BlockSpec { content: vec![1u8; 1_000], hash: "h-original", compress: false }],
        }]);
        let base_path = write_temp_file("hardlink-base", &base_content);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-hardlink-dest-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::hard_link(&base_path, &dest_path).unwrap();

        let new_pkg = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: vec![2u8; 1_000], hash: "h1", compress: false }],
        }]);
        let expected_sha256 = sha256_hex(&new_pkg);
        let fetcher = FakeFetcher { data: new_pkg.clone() };

        execute_delta(&base_path, &fetcher, &dest_path, &expected_sha256, &PlannerConfig::default(), 0.0)
            .unwrap();

        assert_eq!(
            std::fs::read(&base_path).unwrap(),
            base_content,
            "base_path's own directory entry must still be the untouched original bytes"
        );
        assert_eq!(
            std::fs::read(&dest_path).unwrap(),
            new_pkg,
            "dest_path must hold the new, verified reconstruction"
        );

        let _ = std::fs::remove_file(&base_path);
        let _ = std::fs::remove_file(&dest_path);
    }

    #[test]
    fn a_hard_linked_destination_survives_a_failed_reconstruction() {
        // Same setup as above, but the reconstruction itself fails (a
        // deliberately wrong `expected_sha256`, standing in for any
        // assemble/verify failure). Neither name for the shared inode may
        // be touched: `execute_delta` only ever cleans up its own staging
        // file on failure, never `dest_path`.
        let base_content = build_package(&[FileSpec {
            name: "app/original.bin",
            blocks: vec![BlockSpec { content: vec![1u8; 1_000], hash: "h-original", compress: false }],
        }]);
        let base_path = write_temp_file("hardlink-base-fail", &base_content);
        let dest_path = std::env::temp_dir().join(format!(
            "codex-win-engine-executor-test-hardlink-dest-fail-{}-{}.msix",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::hard_link(&base_path, &dest_path).unwrap();

        let new_pkg = build_package(&[FileSpec {
            name: "app/a.bin",
            blocks: vec![BlockSpec { content: vec![3u8; 1_000], hash: "h1", compress: false }],
        }]);
        let fetcher = FakeFetcher { data: new_pkg };

        let err = execute_delta(
            &base_path,
            &fetcher,
            &dest_path,
            &"0".repeat(64),
            &PlannerConfig::default(),
            0.0,
        )
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256 mismatch"), "unexpected error: {err}");

        assert_eq!(
            std::fs::read(&base_path).unwrap(),
            base_content,
            "base_path must survive a failed reconstruction untouched"
        );
        assert_eq!(
            std::fs::read(&dest_path).unwrap(),
            base_content,
            "dest_path (same inode) must also still read back the original bytes"
        );

        let _ = std::fs::remove_file(&base_path);
        let _ = std::fs::remove_file(&dest_path);
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
