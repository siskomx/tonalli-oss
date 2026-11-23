//! Shared resumable downloader, migrated from the spike CLI without network-policy changes.
mod control;
mod paths;
use control::{checkpoint, finish_stop, requested, Stop};
pub use control::{DownloadAction, DownloadOutcome, DownloadPhase, DownloadProgress};
pub use paths::{ensure_regular_or_absent, pack_path, write_atomic};

/// Expected bytes and a native-owned storage destination; this is not catalog-signature authority.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub id: String,
    pub url: Option<String>,
    pub bytes: u64,
    pub sha256: String,
    pub path: String,
}

impl DownloadSpec {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.id.is_empty()
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PackError::Mismatch(
                "pack needs an id and a 64-digit SHA-256".into(),
            ));
        }
        Ok(())
    }
}

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::pack::{verify_sha256_controlled, PackError};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, RANGE};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PartialMeta {
    url: String,
    sha256: String,
    bytes: u64,
    #[serde(default)]
    etag: Option<String>,
}

pub fn fetch_pack(pack: &DownloadSpec, base: &Path) -> Result<PathBuf, PackError> {
    fetch_pack_with_space(pack, base, |path| {
        fs2::available_space(path).map_err(Into::into)
    })
}

/// The space probe is injectable so low-storage behavior is testable without filling a disk.
pub fn fetch_pack_with_space(
    pack: &DownloadSpec,
    base: &Path,
    available_space: impl Fn(&Path) -> Result<u64, PackError>,
) -> Result<PathBuf, PackError> {
    match fetch_inner(
        pack,
        base,
        available_space,
        || DownloadAction::Continue,
        |_| Ok(()),
        None,
    )? {
        DownloadOutcome::Complete(path) => Ok(path),
        _ => Err(PackError::Corrupt("Uncontrolled download stopped".into())),
    }
}

/// Callbacks run synchronously on the caller's worker and must remain bounded/nonblocking.
/// Pause and cancel retain resumable staging; their outcomes require a successful stage flush.
pub fn fetch_pack_controlled(
    pack: &DownloadSpec,
    base: &Path,
    action: impl Fn() -> DownloadAction,
    progress: impl FnMut(DownloadProgress) -> Result<(), PackError>,
) -> Result<DownloadOutcome, PackError> {
    fetch_inner(
        pack,
        base,
        |path| fs2::available_space(path).map_err(Into::into),
        action,
        progress,
        None,
    )
}

/// Uses a trusted native-owned client (network binding, redirects and TLS remain the
/// native caller's responsibility). Catalog URLs and storage authority are unchanged.
pub fn fetch_pack_controlled_with_client(
    pack: &DownloadSpec,
    base: &Path,
    client: &Client,
    action: impl Fn() -> DownloadAction,
    progress: impl FnMut(DownloadProgress) -> Result<(), PackError>,
) -> Result<DownloadOutcome, PackError> {
    fetch_inner(
        pack,
        base,
        |path| fs2::available_space(path).map_err(Into::into),
        action,
        progress,
        Some(client),
    )
}

fn fetch_inner(
    pack: &DownloadSpec,
    base: &Path,
    available_space: impl Fn(&Path) -> Result<u64, PackError>,
    action: impl Fn() -> DownloadAction,
    mut progress: impl FnMut(DownloadProgress) -> Result<(), PackError>,
    supplied_client: Option<&Client>,
) -> Result<DownloadOutcome, PackError> {
    if let Some(stop) = requested(&action) {
        return finish_stop(stop, || Ok(()));
    }
    pack.validate()?;
    let target = pack_path(base, &pack.path)?;
    if target.is_file() && fs::metadata(&target)?.len() == pack.bytes {
        match verify_candidate(pack, &target, &action, &mut progress) {
            Ok(ControlFlow::Continue(())) => return Ok(DownloadOutcome::Complete(target)),
            Ok(ControlFlow::Break(stop)) => return finish_stop(stop, || Ok(())),
            // Preserve the existing cached-file fallback; progress failures travel as Stop::Failed.
            Err(_) => {}
        }
    }
    let url = pack.url.as_deref().ok_or_else(|| {
        PackError::NotFound(format!(
            "local pack {} is missing or fails verification; no download URL",
            pack.id
        ))
    })?;
    let parsed = reqwest::Url::parse(url).map_err(http_error)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(PackError::Http(
            "only HTTP(S) pack URLs are supported".into(),
        ));
    }
    let default_client;
    let client = if let Some(client) = supplied_client {
        client
    } else {
        default_client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(http_error)?;
        &default_client
    };
    let head = send_backing_off(client.head(url)).map_err(http_error)?;
    // Some Kiwix mirrors deny HEAD while supporting ordinary range requests.
    let (remote, total) =
        if head.status().is_success() && header_u64(head.headers(), CONTENT_LENGTH).is_ok() {
            let total = header_u64(head.headers(), CONTENT_LENGTH)?;
            (head, total)
        } else {
            let probe = send_backing_off(client.get(url).header(RANGE, "bytes=0-0"))
                .map_err(http_error)?;
            let total = if probe.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                let range = probe
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("bytes 0-0/"))
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| PackError::Http("invalid probe Content-Range".into()))?;
                if header_u64(probe.headers(), CONTENT_LENGTH)? != 1 {
                    return Err(PackError::Http("invalid probe Content-Length".into()));
                }
                range
            } else {
                probe.error_for_status_ref().map_err(http_error)?;
                header_u64(probe.headers(), CONTENT_LENGTH)?
            };
            (probe, total)
        };
    if total != pack.bytes {
        return Err(PackError::Mismatch(format!(
            "remote length {total} differs from manifest {}",
            pack.bytes
        )));
    }
    let meta = PartialMeta {
        url: url.to_owned(),
        sha256: pack.sha256.clone(),
        bytes: total,
        etag: remote
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with('"') && value.ends_with('"'))
            .map(str::to_owned),
    };
    drop(remote);
    let part = appended_path(&target, ".part");
    let metadata = appended_path(&target, ".part.meta");
    ensure_regular_or_absent(&part)?;
    ensure_regular_or_absent(&metadata)?;
    if let Some(stop) = requested(&action) {
        return finish_stop(stop, || sync_before_copy(&part, 0));
    }
    let saved: Option<PartialMeta> = fs::read_to_string(&metadata)
        .ok()
        .and_then(|text| toml::from_str(&text).ok());
    let part_len = fs::metadata(&part).map(|value| value.len()).unwrap_or(0);
    let mut offset = if saved.as_ref() == Some(&meta) && meta.etag.is_some() && part_len <= total {
        part_len
    } else {
        0
    };
    let parent = target
        .parent()
        .ok_or_else(|| PackError::Mismatch("pack has no parent".into()))?;
    fs::create_dir_all(parent)?;
    if let ControlFlow::Break(stop) = checkpoint(
        &action,
        &mut progress,
        DownloadProgress {
            phase: DownloadPhase::Downloading,
            downloaded_bytes: offset,
            total_bytes: total,
            verified_bytes: 0,
        },
    ) {
        return finish_stop(stop, || sync_before_copy(&part, offset));
    }
    check_space(available_space(parent)?, total - offset)?;
    if offset == 0 {
        discard_partial(&part, &metadata)?;
    }

    // A fully downloaded part may have survived a crash before checksum/activation.
    if offset == total && part.is_file() {
        return activate(pack, &part, &metadata, &target, &action, &mut progress);
    }

    for attempt in 0..2 {
        if let Some(stop) = requested(&action) {
            return finish_stop(stop, || sync_before_copy(&part, offset));
        }
        let mut request = client.get(url).header(RANGE, format!("bytes={offset}-"));
        if offset > 0 {
            if let Some(etag) = &meta.etag {
                request = request.header(IF_RANGE, etag);
            }
        }
        let mut response = send_backing_off(request).map_err(http_error)?;
        if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
            && offset > 0
            && attempt == 0
        {
            check_space(available_space(parent)?, total)?;
            discard_partial(&part, &metadata)?;
            offset = 0;
            continue;
        }
        if response.status() == reqwest::StatusCode::OK {
            offset = 0;
        } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let expected_range = format!("bytes {}-{}/{}", offset, total - 1, total);
            if response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                != Some(expected_range.as_str())
            {
                return Err(PackError::Http("unexpected Content-Range".into()));
            }
            if offset > 0
                && response
                    .headers()
                    .get(ETAG)
                    .and_then(|value| value.to_str().ok())
                    != meta.etag.as_deref()
            {
                check_space(available_space(parent)?, total)?;
                discard_partial(&part, &metadata)?;
                offset = 0;
                if attempt == 0 {
                    continue;
                }
                return Err(PackError::Http("ETag changed during resume".into()));
            }
        } else if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(PackError::RateLimited);
        } else {
            return Err(PackError::Http(format!(
                "unexpected download status {}",
                response.status()
            )));
        }
        let remaining = total - offset;
        if header_u64(response.headers(), CONTENT_LENGTH)? != remaining {
            return Err(PackError::Mismatch(
                "download Content-Length differs from expected bytes".into(),
            ));
        }
        check_space(available_space(parent)?, remaining)?;
        let current_meta = PartialMeta {
            etag: response
                .headers()
                .get(ETAG)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.starts_with('"') && value.ends_with('"'))
                .map(str::to_owned),
            ..meta
        };
        if let Some(stop) = requested(&action) {
            return finish_stop(stop, || sync_before_copy(&part, offset));
        }
        // Remove old metadata before truncating; a crash cannot associate old bytes with a new validator.
        if offset == 0 {
            discard_partial(&part, &metadata)?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(offset == 0)
            .append(offset > 0)
            .open(&part)?;
        let serialized = toml::to_string(&current_meta)
            .map_err(|error| PackError::Corrupt(error.to_string()))?;
        write_atomic(&metadata, serialized.as_bytes())?;
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if let Some(stop) = requested(&action) {
                return finish_stop(stop, || output.sync_all().map_err(Into::into));
            }
            let read = match response.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                value => value?,
            };
            if let Some(stop) = requested(&action) {
                return finish_stop(stop, || output.sync_all().map_err(Into::into));
            }
            if read == 0 {
                break;
            }
            copied += read as u64;
            if copied > remaining {
                drop(output);
                discard_partial(&part, &metadata)?;
                return Err(PackError::Mismatch(
                    "server sent more bytes than declared".into(),
                ));
            }
            output.write_all(&buffer[..read])?;
            if let ControlFlow::Break(stop) = checkpoint(
                &action,
                &mut progress,
                DownloadProgress {
                    phase: DownloadPhase::Downloading,
                    downloaded_bytes: offset + copied,
                    total_bytes: total,
                    verified_bytes: 0,
                },
            ) {
                return finish_stop(stop, || output.sync_all().map_err(Into::into));
            }
        }
        output.sync_all()?;
        drop(output);
        if copied != remaining {
            return Err(PackError::Truncated);
        }
        return activate(pack, &part, &metadata, &target, &action, &mut progress);
    }
    Err(PackError::Http("server refused a fresh download".into()))
}

pub fn check_space(available: u64, required: u64) -> Result<(), PackError> {
    const MARGIN: u64 = 1024 * 1024;
    if available < required.saturating_add(MARGIN) {
        Err(PackError::NoSpace)
    } else {
        Ok(())
    }
}

fn activate(
    pack: &DownloadSpec,
    part: &Path,
    meta: &Path,
    target: &Path,
    action: &impl Fn() -> DownloadAction,
    progress: &mut impl FnMut(DownloadProgress) -> Result<(), PackError>,
) -> Result<DownloadOutcome, PackError> {
    match verify_candidate(pack, part, action, progress) {
        Ok(ControlFlow::Continue(())) => {}
        Ok(ControlFlow::Break(stop)) => return finish_stop(stop, || sync_partial(part)),
        Err(error) => {
            discard_partial(part, meta)?;
            return Err(error);
        }
    }
    if let Some(stop) = requested(action) {
        return finish_stop(stop, || sync_partial(part));
    }
    fs::rename(part, target)?;
    remove_if_exists(meta)?;
    Ok(DownloadOutcome::Complete(target.to_owned()))
}

fn verify_candidate(
    pack: &DownloadSpec,
    path: &Path,
    action: &impl Fn() -> DownloadAction,
    progress: &mut impl FnMut(DownloadProgress) -> Result<(), PackError>,
) -> Result<ControlFlow<Stop>, PackError> {
    verify_sha256_controlled(path, &pack.sha256, |verified_bytes| {
        Ok(checkpoint(
            action,
            progress,
            DownloadProgress {
                phase: DownloadPhase::Verifying,
                downloaded_bytes: pack.bytes,
                total_bytes: pack.bytes,
                verified_bytes,
            },
        ))
    })
}

fn sync_partial(path: &Path) -> Result<(), PackError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()
        .map_err(Into::into)
}

fn sync_before_copy(path: &Path, staged_bytes: u64) -> Result<(), PackError> {
    if staged_bytes > 0 {
        return sync_partial(path);
    }
    match fs::symlink_metadata(path) {
        Ok(_) => sync_partial(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn appended_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_if_exists(path: &Path) -> Result<(), PackError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn discard_partial(part: &Path, meta: &Path) -> Result<(), PackError> {
    remove_if_exists(meta)?;
    remove_if_exists(part)
}

fn header_u64(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Result<u64, PackError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| PackError::Http("missing or invalid Content-Length".into()))
}

fn http_error(error: impl std::fmt::Debug) -> PackError {
    PackError::Http(format!("{error:?}"))
}

/// A paired host limits how fast one device may ask, so a quick run of requests (a book with many
/// search parts) can be told to wait. Retry a few times with a growing pause, then report the limit.
const RATE_LIMIT_RETRIES: u32 = 4;

fn send_backing_off(
    request: reqwest::blocking::RequestBuilder,
) -> reqwest::Result<reqwest::blocking::Response> {
    let mut pause = Duration::from_millis(150);
    for _ in 0..RATE_LIMIT_RETRIES {
        let Some(attempt) = request.try_clone() else {
            break;
        };
        let response = attempt.send()?;
        if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }
        std::thread::sleep(pause);
        pause *= 2;
    }
    request.send()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_partial_flush_cannot_report_pause_or_cancel_success() {
        for stop in [Stop::Pause, Stop::Cancel] {
            let outcome = finish_stop(stop, || Err(PackError::Io("partial flush failed".into())));
            assert!(
                matches!(outcome, Err(PackError::Io(message)) if message == "partial flush failed")
            );
        }
    }

    #[test]
    fn a_required_stage_that_disappeared_is_a_flush_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            sync_partial(&root.path().join("missing.part")),
            Err(PackError::Io(_))
        ));
    }

    /// Serves `connections` requests, one connection each: the first `limited` get a 429, the rest 200.
    fn rate_limited_server(
        limited: usize,
        connections: usize,
    ) -> (String, std::thread::JoinHandle<usize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pack", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for stream in listener.incoming().take(connections) {
                let mut stream = stream.unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let reply = if served < limited {
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                stream.write_all(reply.as_bytes()).unwrap();
                served += 1;
            }
            served
        });
        (url, handle)
    }

    #[test]
    fn a_rate_limited_request_is_retried_after_a_short_pause() {
        let (url, server) = rate_limited_server(2, 3);
        let client = Client::new();
        let started = std::time::Instant::now();
        let response = send_backing_off(client.get(&url)).unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(server.join().unwrap(), 3);
        assert!(started.elapsed() >= Duration::from_millis(400));
    }

    #[test]
    fn retrying_is_bounded_and_still_reports_the_limit() {
        let attempts = RATE_LIMIT_RETRIES as usize + 1;
        let (url, server) = rate_limited_server(attempts, attempts);
        let client = Client::new();
        let response = send_backing_off(client.get(&url)).unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(server.join().unwrap(), attempts);
    }
}
