//! Loopback media server: streams a theme's motion assets (mp4/webm) to the
//! Codex renderer's `<video>` element over HTTP with Range support, instead of
//! base64-inlining them into the injected payload. Data URLs don't scale for
//! video — a 7 MB clip becomes ~9.5 MB of base64 re-shipped over CDP on every
//! reload — so the bytes ride a real HTTP channel the browser streams natively.
//!
//! Hardening: bound to `127.0.0.1` on an OS-assigned port, gated by a per-start
//! random path token and a loopback `Host` check, and it only ever resolves
//! keys declared in the *currently applied* theme's `motionAssets` (paths come
//! from validated [`load_theme`]), so it cannot be coaxed into serving an
//! arbitrary file. It carries no CORS headers, so a foreign page cannot read a
//! response even if it guessed the URL.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::theme::load_theme;
use crate::{Result, ThemeEngineError};

/// Shared handle to the directory of the theme the daemon currently applies.
/// The daemon writes it on every rebuild; the server reads it per request so a
/// stale URL minted for a previous theme never resolves against the wrong
/// package.
pub type CurrentTheme = Arc<RwLock<Option<PathBuf>>>;

/// Read cap for the request head — we only ever parse a GET/HEAD line plus a
/// handful of headers; anything larger is not a media request we serve.
const MAX_HEAD_BYTES: usize = 8 * 1024;
const STREAM_CHUNK: usize = 64 * 1024;

/// The base a payload builder turns into per-asset motion URLs. Cheap to clone.
#[derive(Debug, Clone)]
pub struct MediaBase {
    origin: String,
    token: String,
}

impl MediaBase {
    /// `http://127.0.0.1:<port>/<token>/<theme-id>/<key>`. `id`/`key` are the
    /// package's `^[a-z0-9][a-z0-9-]{0,63}$` slugs, so no escaping is needed.
    pub fn url(&self, theme_id: &str, key: &str) -> String {
        format!("{}/{}/{}/{}", self.origin, self.token, theme_id, key)
    }

    #[cfg(test)]
    pub(crate) fn for_test(origin: &str, token: &str) -> Self {
        MediaBase {
            origin: origin.to_string(),
            token: token.to_string(),
        }
    }
}

/// A running loopback media server. Dropping it aborts the accept loop.
pub struct MediaServer {
    base: MediaBase,
    task: tokio::task::JoinHandle<()>,
}

impl MediaServer {
    /// Bind `127.0.0.1:0` and start accepting. Errors only if the loopback bind
    /// fails, in which case the caller degrades to the static intro.
    pub async fn start(current: CurrentTheme) -> Result<MediaServer> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let base = MediaBase {
            origin: format!("http://127.0.0.1:{port}"),
            token: token.clone(),
        };
        let task = tokio::spawn(accept_loop(listener, token, current));
        Ok(MediaServer { base, task })
    }

    pub fn base(&self) -> MediaBase {
        self.base.clone()
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Not a security boundary — the server only serves declared video art and
/// Host-checks loopback — just enough entropy to stop other local pages from
/// guessing the path. Derived from pid + a high-resolution timestamp (no `rand`
/// dependency).
fn random_token() -> String {
    use sha1::{Digest, Sha1};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha1::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn accept_loop(listener: TcpListener, token: String, current: CurrentTheme) {
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            continue;
        };
        let token = token.clone();
        let current = current.clone();
        tokio::spawn(async move {
            if let Err(error) = serve(stream, &token, &current).await {
                log::debug!("media request failed: {error}");
            }
        });
    }
}

struct RequestHead {
    method: String,
    target: String,
    host: Option<String>,
    range: Option<String>,
}

async fn serve(mut stream: TcpStream, token: &str, current: &CurrentTheme) -> Result<()> {
    let head = read_request_head(&mut stream).await?;

    // Reject Host-spoofed / proxied requests: media only ever loads from the
    // loopback origin we handed the renderer.
    if !head.host.as_deref().is_some_and(host_is_loopback) {
        return write_status(&mut stream, 421, "Misdirected Request").await;
    }
    if head.method != "GET" && head.method != "HEAD" {
        return write_status(&mut stream, 405, "Method Not Allowed").await;
    }
    let Some((path, mime)) = resolve(&head.target, token, current) else {
        return write_status(&mut stream, 404, "Not Found").await;
    };

    let mut file = tokio::fs::File::open(&path).await?;
    let total = file.metadata().await?.len();

    let range = head.range.as_deref();
    let (start, end) = match range.map(|r| parse_range(r, total)) {
        Some(Some(bounds)) => bounds,
        Some(None) => {
            // A Range header we cannot satisfy → 416 with the total size.
            let header = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\n\
                 Content-Range: bytes */{total}\r\n\
                 Content-Length: 0\r\n\
                 Connection: close\r\n\r\n"
            );
            stream.write_all(header.as_bytes()).await?;
            return Ok(());
        }
        None => (0, total.saturating_sub(1)),
    };
    let partial = range.is_some();
    // A full-file response streams `total` bytes — correct even for an empty
    // file, where `end + 1 - start` would wrongly claim 1 and hang the client.
    // A partial response only reaches here with total > 0 (parse_range rejects
    // any range against an empty file).
    let length = if partial { end + 1 - start } else { total };

    let status = if partial { "206 Partial Content" } else { "200 OK" };
    let mut header = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {mime}\r\n\
         Content-Length: {length}\r\n\
         Accept-Ranges: bytes\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n"
    );
    if partial {
        header.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    header.push_str("\r\n");
    stream.write_all(header.as_bytes()).await?;

    if head.method == "HEAD" {
        return Ok(());
    }

    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut remaining = length;
    let mut buf = vec![0u8; STREAM_CHUNK];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n]).await?;
        remaining -= n as u64;
    }
    stream.flush().await?;
    Ok(())
}

async fn read_request_head(stream: &mut TcpStream) -> Result<RequestHead> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !contains_head_end(&buf) {
        if buf.len() > MAX_HEAD_BYTES {
            return Err(ThemeEngineError::Theme("media request head too large".into()));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("").to_string();
    let target = request_line.next().unwrap_or("").to_string();

    let mut host = None;
    let mut range = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => host = Some(value.trim().to_string()),
                "range" => range = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }
    Ok(RequestHead {
        method,
        target,
        host,
        range,
    })
}

fn contains_head_end(buf: &[u8]) -> bool {
    buf.windows(4).any(|w| w == b"\r\n\r\n")
}

fn host_is_loopback(host: &str) -> bool {
    let host = host.trim();
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6: "[addr]" or "[addr]:port" → the part inside brackets.
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() == 1 {
        // "host:port" for IPv4 or a name → the part before the port.
        host.split(':').next().unwrap_or(host)
    } else {
        // Bare hostname, or an unbracketed IPv6 literal (e.g. "::1") with no port.
        host
    };
    hostname == "127.0.0.1" || hostname.eq_ignore_ascii_case("localhost") || hostname == "::1"
}

/// Resolve `/<token>/<id>/<key>` to a declared motion asset of the *current*
/// theme. Returns the on-disk path plus its mime, or `None` for any mismatch —
/// wrong token, no applied theme, id/key not the current package's motion set.
fn resolve(target: &str, token: &str, current: &CurrentTheme) -> Option<(PathBuf, &'static str)> {
    let path = target.split('?').next().unwrap_or(target);
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    if segments.next()? != token {
        return None;
    }
    let id = segments.next()?;
    let key = segments.next()?;
    if segments.next().is_some() {
        return None; // exactly three path segments
    }

    let dir = current.read().ok()?.clone()?;
    let theme = load_theme(&dir).ok()?;
    // Match on the manifest id, NOT the directory basename: the payload builds
    // URLs from `theme.config.id`, and an explicit dev directory may be named
    // differently from its packaged id.
    if theme.config.id != id {
        return None;
    }
    let asset = theme.motion_assets.get(key)?;
    Some((asset.path.clone(), asset.mime))
}

/// Parse a single-range `bytes=` header against `total`. Supports `a-b`, `a-`
/// and `-suffix`; multi-range and unsatisfiable specs return `None`.
fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let (start_s, end_s) = (start_s.trim(), end_s.trim());

    let (start, end) = if start_s.is_empty() {
        let suffix: u64 = end_s.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        (total - suffix.min(total), total - 1)
    } else {
        let start: u64 = start_s.parse().ok()?;
        let end = if end_s.is_empty() {
            total - 1
        } else {
            end_s.parse::<u64>().ok()?.min(total - 1)
        };
        (start, end)
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

async fn write_status(stream: &mut TcpStream, code: u16, reason: &str) -> Result<()> {
    let response =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_theme_with_motion(dir: &std::path::Path, id: &str, video: &[u8]) {
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("theme.json"),
            format!(
                r##"{{"schemaVersion":2,"id":"{id}","name":"T",
                    "motionAssets":{{"intro-video":"assets/intro-video.mp4"}}}}"##
            ),
        )
        .unwrap();
        std::fs::write(dir.join("theme.css"), "html.codex-theme-studio {}\n").unwrap();
        std::fs::write(dir.join("assets/intro-video.mp4"), video).unwrap();
    }

    async fn raw_request(port: u16, request: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    }

    /// Split a URL `http://127.0.0.1:PORT/rest` into `(port, "/rest")`.
    fn port_and_path(url: &str) -> (u16, String) {
        let after = url.strip_prefix("http://127.0.0.1:").unwrap();
        let (port_s, rest) = after.split_once('/').unwrap();
        (port_s.parse().unwrap(), format!("/{rest}"))
    }

    fn split_head_body(response: &[u8]) -> (String, Vec<u8>) {
        let idx = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("head terminator");
        (
            String::from_utf8_lossy(&response[..idx]).to_string(),
            response[idx + 4..].to_vec(),
        )
    }

    #[tokio::test]
    async fn streams_full_and_ranged_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ning-hongye");
        let body: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        write_theme_with_motion(&dir, "ning-hongye", &body);

        let current: CurrentTheme = Arc::new(RwLock::new(Some(dir.clone())));
        let server = MediaServer::start(current).await.unwrap();
        let (port, path) = port_and_path(&server.base().url("ning-hongye", "intro-video"));

        // Full GET → 200 with the whole file.
        let full = raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
        .await;
        let (head, got) = split_head_body(&full);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(head.contains("Content-Type: video/mp4"), "{head}");
        assert!(head.contains("Accept-Ranges: bytes"), "{head}");
        assert_eq!(got, body);

        // Ranged GET → 206 with exactly the requested slice.
        let ranged = raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=10-19\r\n\r\n"),
        )
        .await;
        let (head, got) = split_head_body(&ranged);
        assert!(head.starts_with("HTTP/1.1 206 Partial Content"), "{head}");
        assert!(head.contains("Content-Range: bytes 10-19/4096"), "{head}");
        assert_eq!(got, body[10..=19]);
    }

    #[tokio::test]
    async fn rejects_bad_token_and_foreign_host() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skin");
        write_theme_with_motion(&dir, "skin", b"video-bytes");
        let current: CurrentTheme = Arc::new(RwLock::new(Some(dir)));
        let server = MediaServer::start(current).await.unwrap();
        let (port, path) = port_and_path(&server.base().url("skin", "intro-video"));

        // Wrong token → 404.
        let bad = raw_request(
            port,
            &format!("GET /deadbeef/skin/intro-video HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
        .await;
        assert!(
            String::from_utf8_lossy(&bad).starts_with("HTTP/1.1 404"),
            "{:?}",
            String::from_utf8_lossy(&bad)
        );

        // Foreign Host → 421, even with a valid path.
        let spoofed = raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: evil.example.com\r\n\r\n"),
        )
        .await;
        assert!(
            String::from_utf8_lossy(&spoofed).starts_with("HTTP/1.1 421"),
            "{:?}",
            String::from_utf8_lossy(&spoofed)
        );
    }

    #[tokio::test]
    async fn unknown_key_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skin");
        write_theme_with_motion(&dir, "skin", b"bytes");
        let current: CurrentTheme = Arc::new(RwLock::new(Some(dir)));
        let server = MediaServer::start(current).await.unwrap();
        let (port, path) = port_and_path(&server.base().url("skin", "intro-video"));
        let token = path.split('/').nth(1).unwrap().to_string();

        let missing = raw_request(
            port,
            &format!("GET /{token}/skin/nope HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"),
        )
        .await;
        assert!(
            String::from_utf8_lossy(&missing).starts_with("HTTP/1.1 404"),
            "{:?}",
            String::from_utf8_lossy(&missing)
        );
    }

    #[tokio::test]
    async fn resolves_by_manifest_id_not_dir_name() {
        let tmp = tempfile::tempdir().unwrap();
        // The directory basename differs from the packaged manifest id — an
        // explicit dev checkout. Resolution must key off the manifest id.
        let dir = tmp.path().join("dev-checkout");
        write_theme_with_motion(&dir, "packaged-id", b"video-bytes");
        let current: CurrentTheme = Arc::new(RwLock::new(Some(dir)));
        let server = MediaServer::start(current).await.unwrap();
        let (port, path) = port_and_path(&server.base().url("packaged-id", "intro-video"));
        let ok = raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
        .await;
        assert!(
            String::from_utf8_lossy(&ok).starts_with("HTTP/1.1 200"),
            "{:?}",
            String::from_utf8_lossy(&ok)
        );
    }

    #[tokio::test]
    async fn no_applied_theme_is_not_found() {
        // With no directive set, every path resolves to 404 (static fallback).
        let current: CurrentTheme = Arc::new(RwLock::new(None));
        let server = MediaServer::start(current).await.unwrap();
        let (port, path) = port_and_path(&server.base().url("skin", "intro-video"));
        let response = raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
        .await;
        assert!(
            String::from_utf8_lossy(&response).starts_with("HTTP/1.1 404"),
            "{:?}",
            String::from_utf8_lossy(&response)
        );
    }

    #[test]
    fn parse_range_forms() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-", 1000), Some((10, 999)));
        assert_eq!(parse_range("bytes=-50", 1000), Some((950, 999)));
        assert_eq!(parse_range("bytes=990-100000", 1000), Some((990, 999)));
        assert_eq!(parse_range("bytes=1000-1001", 1000), None); // start past end
        assert_eq!(parse_range("bytes=0-0,5-6", 1000), None); // multi-range
        assert_eq!(parse_range("bytes=abc", 1000), None);
    }

    #[test]
    fn host_loopback_matching() {
        assert!(host_is_loopback("127.0.0.1:54213"));
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("localhost:8080"));
        assert!(host_is_loopback("LocalHost"));
        assert!(host_is_loopback("[::1]:9000"));
        assert!(host_is_loopback("::1"));
        assert!(!host_is_loopback("evil.example.com"));
        assert!(!host_is_loopback("10.0.0.5:80"));
    }
}
