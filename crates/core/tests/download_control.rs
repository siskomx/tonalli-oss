use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tonalli_core::download::{
    fetch_pack, fetch_pack_controlled, fetch_pack_controlled_with_client, DownloadAction,
    DownloadOutcome, DownloadPhase, DownloadSpec,
};
use tonalli_core::pack::PackError;

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    ranges: Arc<Mutex<Vec<usize>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(body: Arc<Vec<u8>>, truncate: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/fixture.zim", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::clone(&stop);
        let requests = Arc::clone(&ranges);
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Closing a paused/cancelled client is expected, not a fixture panic.
                        let _ = reply(&mut stream, &body, truncate, &requests);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            ranges,
            worker: Some(worker),
        }
    }
}

fn reply(
    stream: &mut TcpStream,
    body: &[u8],
    truncate: bool,
    ranges: &Mutex<Vec<usize>>,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        if request.len() >= 8192 || stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).map_err(|_| io::ErrorKind::InvalidData)?;
    if request.starts_with("HEAD ") {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
    } else {
        let start = request
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("range: bytes=")
                    .and_then(|value| value.split_once('-'))
                    .and_then(|(value, _)| value.parse::<usize>().ok())
            })
            .ok_or(io::ErrorKind::InvalidData)?;
        if start >= body.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        ranges.lock().unwrap().push(start);
        write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", body.len() - start, start, body.len() - 1, body.len())?;
        let end = if truncate {
            (start + 32768).min(body.len())
        } else {
            body.len()
        };
        for chunk in body[start..end].chunks(16384) {
            stream.write_all(chunk)?;
        }
    }
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    let _ = io::copy(stream, &mut io::sink());
    Ok(())
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn fixture(truncate: bool) -> (tempfile::TempDir, Server, DownloadSpec, Arc<Vec<u8>>) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("packs")).unwrap();
    let body = Arc::new((0..512 * 1024).map(|i| (i % 251) as u8).collect::<Vec<_>>());
    let server = Server::new(Arc::clone(&body), truncate);
    let spec = DownloadSpec {
        id: "fixture".into(),
        url: Some(server.url.clone()),
        bytes: body.len() as u64,
        sha256: format!("{:x}", Sha256::digest(body.as_slice())),
        path: "packs/test.zim".into(),
    };
    (root, server, spec, body)
}

fn action(state: &AtomicU8) -> DownloadAction {
    match state.load(Ordering::Relaxed) {
        1 => DownloadAction::Pause,
        2 => DownloadAction::Cancel,
        _ => DownloadAction::Continue,
    }
}

#[test]
fn supplied_native_client_policy_is_not_replaced_by_legacy_default_transport() {
    let (root, server, spec, bytes) = fixture(false);
    let strict = reqwest::blocking::Client::builder()
        .https_only(true)
        .build()
        .unwrap();
    let refused = fetch_pack_controlled_with_client(
        &spec,
        root.path(),
        &strict,
        || DownloadAction::Continue,
        |_| Ok(()),
    );
    assert!(matches!(refused, Err(PackError::Http(_))));
    assert!(server.ranges.lock().unwrap().is_empty());
    // The same real HTTP fixture still works through the unchanged legacy default.
    let DownloadOutcome::Complete(path) =
        fetch_pack_controlled(&spec, root.path(), || DownloadAction::Continue, |_| Ok(())).unwrap()
    else {
        panic!("expected verified completion")
    };
    assert_eq!(fs::read(path).unwrap(), *bytes);
}

#[test]
fn mid_body_pause_and_cancel_report_observed_bytes_and_resume_with_sha_verification() {
    for requested in [1, 2] {
        let (root, server, spec, body) = fixture(false);
        let state = AtomicU8::new(0);
        let mut progress = Vec::new();
        let outcome = fetch_pack_controlled(
            &spec,
            root.path(),
            || action(&state),
            |value| {
                if value.downloaded_bytes > 0 {
                    state.store(requested, Ordering::Relaxed);
                }
                progress.push(value);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(
            (requested, outcome),
            (1, DownloadOutcome::Paused) | (2, DownloadOutcome::Cancelled)
        ));
        assert!(!root.path().join("packs/test.zim").exists());
        let part = fs::read(root.path().join("packs/test.zim.part")).unwrap();
        assert!(!part.is_empty() && part.len() < body.len());
        assert_eq!(part, body[..part.len()]);
        assert!(root.path().join("packs/test.zim.part.meta").exists());
        assert!(progress
            .windows(2)
            .all(|pair| pair[0].downloaded_bytes <= pair[1].downloaded_bytes));
        assert!(progress
            .iter()
            .all(|p| p.total_bytes == spec.bytes && p.downloaded_bytes <= part.len() as u64));
        assert_eq!(progress.last().unwrap().downloaded_bytes, part.len() as u64);
        let complete = fetch_pack(&spec, root.path()).unwrap();
        assert_eq!(fs::read(complete).unwrap(), body.as_slice());
        assert_eq!(
            server.ranges.lock().unwrap().last().copied(),
            Some(part.len())
        );
    }
}

#[test]
fn cancellation_takes_precedence_over_the_already_verified_installed_fast_path() {
    let (root, _server, mut spec, body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, body.as_slice()).unwrap();
    spec.url = None;
    let outcome = fetch_pack_controlled(
        &spec,
        root.path(),
        || DownloadAction::Cancel,
        |_| panic!("prelatched cancellation must not report download progress"),
    )
    .unwrap();
    assert!(matches!(outcome, DownloadOutcome::Cancelled));
    assert_eq!(fs::read(target).unwrap(), body.as_slice());
    assert!(!root.path().join("packs/test.zim.part").exists());
}

#[test]
fn modified_paused_stage_fails_sha_verification_without_replacing_installed_content() {
    let (root, _server, spec, _body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous installed content").unwrap();
    let state = AtomicU8::new(0);
    assert!(matches!(
        fetch_pack_controlled(
            &spec,
            root.path(),
            || action(&state),
            |value| {
                if value.downloaded_bytes > 0 {
                    state.store(1, Ordering::Relaxed);
                }
                Ok(())
            }
        )
        .unwrap(),
        DownloadOutcome::Paused
    ));
    let part = root.path().join("packs/test.zim.part");
    let mut bytes = fs::read(&part).unwrap();
    bytes[0] ^= 1;
    fs::write(&part, bytes).unwrap();
    assert!(matches!(
        fetch_pack(&spec, root.path()),
        Err(PackError::Checksum)
    ));
    assert_eq!(fs::read(target).unwrap(), b"previous installed content");
    // Existing checksum-failure policy discards only this invocation's derived partial paths.
    assert!(!part.exists());
    assert!(!root.path().join("packs/test.zim.part.meta").exists());
}

#[test]
fn cancellation_at_the_last_progress_event_preserves_installed_content_and_the_complete_stage() {
    let (root, server, spec, body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous installed content").unwrap();
    let state = AtomicU8::new(0);
    let outcome = fetch_pack_controlled(
        &spec,
        root.path(),
        || action(&state),
        |value| {
            if value.phase == DownloadPhase::Downloading && value.downloaded_bytes == spec.bytes {
                state.store(2, Ordering::Relaxed);
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(matches!(outcome, DownloadOutcome::Cancelled));
    assert_eq!(fs::read(&target).unwrap(), b"previous installed content");
    assert_eq!(
        fs::read(root.path().join("packs/test.zim.part")).unwrap(),
        body.as_slice()
    );
    let prior_gets = server.ranges.lock().unwrap().len();
    assert_eq!(
        fs::read(fetch_pack(&spec, root.path()).unwrap()).unwrap(),
        body.as_slice()
    );
    assert_eq!(
        server.ranges.lock().unwrap().len(),
        prior_gets,
        "complete stage should be verified and activated without another GET"
    );
}

#[test]
fn truncated_body_returns_an_error_without_claiming_completion_or_overwriting_installed_content() {
    let (root, _server, spec, _body) = fixture(true);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous installed content").unwrap();
    let mut progress = Vec::new();
    assert!(fetch_pack_controlled(
        &spec,
        root.path(),
        || DownloadAction::Continue,
        |value| {
            progress.push(value);
            Ok(())
        }
    )
    .is_err());
    assert_eq!(fs::read(target).unwrap(), b"previous installed content");
    assert_eq!(
        fs::metadata(root.path().join("packs/test.zim.part"))
            .unwrap()
            .len(),
        32768
    );
    assert_eq!(progress.last().unwrap().downloaded_bytes, 32768);
    assert!(progress
        .iter()
        .all(|value| value.downloaded_bytes < spec.bytes));
}

#[test]
fn fallible_verifying_progress_prevents_activation_and_preserves_the_complete_stage() {
    let (root, _server, spec, body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous installed content").unwrap();
    let mut phases = Vec::new();
    let error = fetch_pack_controlled(
        &spec,
        root.path(),
        || DownloadAction::Continue,
        |value| {
            phases.push(value.phase);
            if value.phase == DownloadPhase::Verifying {
                return Err(PackError::Io("checkpoint could not be saved".into()));
            }
            Ok(())
        },
    );
    assert!(
        matches!(error, Err(PackError::Io(message)) if message == "checkpoint could not be saved")
    );
    assert_eq!(phases.last(), Some(&DownloadPhase::Verifying));
    assert!(phases[..phases.len() - 1]
        .iter()
        .all(|phase| *phase == DownloadPhase::Downloading));
    assert_eq!(fs::read(&target).unwrap(), b"previous installed content");
    assert_eq!(
        fs::read(root.path().join("packs/test.zim.part")).unwrap(),
        body.as_slice()
    );
    assert_eq!(
        fs::read(fetch_pack(&spec, root.path()).unwrap()).unwrap(),
        body.as_slice()
    );
}

#[test]
fn cached_target_checksum_can_pause_between_hash_chunks_without_creating_a_stage() {
    let (root, _server, mut spec, body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, body.as_slice()).unwrap();
    spec.url = None;
    let state = AtomicU8::new(0);
    let mut progress = Vec::new();
    let outcome = fetch_pack_controlled(
        &spec,
        root.path(),
        || action(&state),
        |value| {
            assert_eq!(value.phase, DownloadPhase::Verifying);
            if value.verified_bytes > 0 {
                state.store(1, Ordering::Relaxed);
            }
            progress.push(value);
            Ok(())
        },
    )
    .unwrap();
    assert!(matches!(outcome, DownloadOutcome::Paused));
    let hashed = progress.last().unwrap().verified_bytes;
    assert!(hashed > 0 && hashed < spec.bytes);
    assert_eq!(fs::read(target).unwrap(), body.as_slice());
    assert!(!root.path().join("packs/test.zim.part").exists());
}

#[test]
fn complete_stage_checksum_can_cancel_without_a_new_get_or_replacing_installed_content() {
    let (root, server, spec, body) = fixture(false);
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous installed content").unwrap();
    let state = AtomicU8::new(0);
    assert!(matches!(
        fetch_pack_controlled(
            &spec,
            root.path(),
            || action(&state),
            |value| {
                if value.phase == DownloadPhase::Downloading && value.downloaded_bytes == spec.bytes
                {
                    state.store(1, Ordering::Relaxed);
                }
                Ok(())
            }
        )
        .unwrap(),
        DownloadOutcome::Paused
    ));
    let previous_gets = server.ranges.lock().unwrap().len();
    let state = AtomicU8::new(0);
    let mut last_hashed = 0;
    let outcome = fetch_pack_controlled(
        &spec,
        root.path(),
        || action(&state),
        |value| {
            if value.phase == DownloadPhase::Verifying && value.verified_bytes > 0 {
                last_hashed = value.verified_bytes;
                state.store(2, Ordering::Relaxed);
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(matches!(outcome, DownloadOutcome::Cancelled));
    assert!(last_hashed > 0 && last_hashed < spec.bytes);
    assert_eq!(server.ranges.lock().unwrap().len(), previous_gets);
    assert_eq!(fs::read(target).unwrap(), b"previous installed content");
    assert_eq!(
        fs::read(root.path().join("packs/test.zim.part")).unwrap(),
        body.as_slice()
    );
}
