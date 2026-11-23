use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;

use sha2::{Digest, Sha256};
use spike_cli::{fetch::fetch_pack_with_space, manifest::Pack};
use tempfile::TempDir;
use tonalli_core::pack::PackError;

const BODY: &[u8] = b"abcdefghij";

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(mode: &'static str, target: std::path::PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/fixture.zim", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            let mut get_count = 0;
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap() == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                if request.is_empty() {
                    continue;
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                log.lock().unwrap().push(request.clone());
                if request.starts_with("head ") {
                    if mode == "head403" {
                        write!(stream, "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        finish(&mut stream);
                        continue;
                    }
                    let etag = if mode == "weak" { "W/\"v2\"" } else { "\"v2\"" };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: {etag}\r\nConnection: close\r\n\r\n").unwrap();
                    finish(&mut stream);
                    continue;
                }
                get_count += 1;
                let range_value = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="));
                let range = range_value
                    .map(|value| value.split('-').next().unwrap().parse::<usize>().unwrap());
                if range_value == Some("0-0") {
                    write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/10\r\nETag: \"v2\"\r\nConnection: close\r\n\r\na").unwrap();
                    finish(&mut stream);
                    continue;
                }
                if mode == "416" && get_count == 1 && range.is_some() {
                    write!(stream, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    finish(&mut stream);
                    continue;
                }
                let start = if mode == "ignore" {
                    0
                } else {
                    range.unwrap_or(0)
                };
                let payload = if mode == "corrupt" {
                    b"XXXXXXXXXX"
                } else {
                    BODY
                };
                let status = if range.is_some() && mode != "ignore" {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let etag = if mode == "changed206" && get_count == 1 {
                    "\"v3\""
                } else if mode == "weak" {
                    "W/\"v2\""
                } else {
                    "\"v2\""
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n", payload.len() - start).unwrap();
                if status.starts_with("206") {
                    let reported_start = if mode == "bad206" { 0 } else { start };
                    write!(stream, "Content-Range: bytes {reported_start}-9/10\r\n").unwrap();
                }
                write!(stream, "\r\n").unwrap();
                if mode == "truncate" && get_count == 1 {
                    stream.write_all(&payload[..3]).unwrap();
                } else if mode == "absent" {
                    stream.write_all(&payload[start..5]).unwrap();
                    stream.flush().unwrap();
                    assert!(!target.exists(), "final file appeared before complete body");
                    stream.write_all(&payload[5..]).unwrap();
                } else {
                    stream.write_all(&payload[start..]).unwrap();
                }
                finish(&mut stream);
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn gets(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("get "))
            .cloned()
            .collect()
    }
}

fn finish(stream: &mut TcpStream) {
    stream.flush().unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    // Drain the peer's close so Windows does not abort a response during teardown.
    let _ = stream.read_to_end(&mut Vec::new());
}

impl Drop for Server {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!("requests: {:?}", self.requests.lock().unwrap());
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn fixture(mode: &'static str) -> (TempDir, Server, Pack) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("packs")).unwrap();
    let server = Server::new(mode, dir.path().join("packs/test.zim"));
    let pack: Pack = toml::from_str(&format!("id='test'\nkind='zim'\nversion='test'\nurl='{}'\nbytes=10\nsha256='{:x}'\npath='packs/test.zim'\n", server.url, Sha256::digest(BODY))).unwrap();
    (dir, server, pack)
}

fn partial(dir: &TempDir, pack: &Pack, body: &[u8], etag: &str, total: u64) {
    fs::write(dir.path().join("packs/test.zim.part"), body).unwrap();
    fs::write(
        dir.path().join("packs/test.zim.part.meta"),
        format!(
            "url='{}'\nsha256='{}'\nbytes={}\netag='{}'\n",
            pack.url.as_deref().unwrap(),
            pack.sha256,
            total,
            etag
        ),
    )
    .unwrap();
}

fn fetch(dir: &TempDir, pack: &Pack) -> Result<std::path::PathBuf, PackError> {
    fetch_pack_with_space(pack, dir.path(), |_| Ok(u64::MAX))
}

#[test]
fn resume_uses_range_and_completes() {
    let (dir, server, pack) = fixture("resume");
    partial(&dir, &pack, b"abc", "\"v2\"", 10);
    let path = fetch(&dir, &pack).unwrap();
    assert_eq!(fs::read(path).unwrap(), BODY);
    assert!(server.gets()[0].contains("range: bytes=3-"));
    assert!(server.gets()[0].contains("if-range: \"v2\""));
    assert!(!dir.path().join("packs/test.zim.part.meta").exists());
}

#[test]
fn server_ignoring_range_returns_200_full_body_restarts_from_zero() {
    let (dir, _server, pack) = fixture("ignore");
    partial(&dir, &pack, b"abc", "\"v2\"", 10);
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
}

#[test]
fn server_returns_416_for_range_restarts_from_zero() {
    let (dir, server, pack) = fixture("416");
    partial(&dir, &pack, b"abc", "\"v2\"", 10);
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert_eq!(server.gets().len(), 2);
    assert!(server.gets()[1].contains("range: bytes=0-"));
}

#[test]
fn stale_partial_with_changed_etag_or_length_is_discarded() {
    for (etag, len) in [("\"v1\"", 10), ("\"v2\"", 12)] {
        let (dir, server, pack) = fixture("resume");
        partial(&dir, &pack, b"BAD", etag, len);
        assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
        assert!(server.gets()[0].contains("range: bytes=0-"));
    }
}

#[test]
fn stale_partial_larger_than_remote_is_discarded() {
    let (dir, server, pack) = fixture("resume");
    partial(&dir, &pack, b"far too much stale data", "\"v2\"", 10);
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.gets()[0].contains("range: bytes=0-"));
}

#[test]
fn leftover_part_file_from_crashed_run_is_reused_only_if_meta_matches() {
    let (dir, server, pack) = fixture("resume");
    partial(&dir, &pack, b"BAD", "\"v2\"", 10);
    fs::write(dir.path().join("packs/test.zim.part.meta"), "broken = [").unwrap();
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.gets()[0].contains("range: bytes=0-"));
}

#[test]
fn corrupt_download_never_replaces_installed_file() {
    let (dir, _server, pack) = fixture("corrupt");
    let target = dir.path().join("packs/test.zim");
    fs::write(&target, b"previous installed version").unwrap();
    assert!(matches!(fetch(&dir, &pack), Err(PackError::Checksum)));
    assert_eq!(fs::read(target).unwrap(), b"previous installed version");
    assert!(!dir.path().join("packs/test.zim.part").exists());
}

#[test]
fn insufficient_disk_space_fails_before_writing() {
    let (dir, _server, pack) = fixture("resume");
    assert!(matches!(
        fetch_pack_with_space(&pack, dir.path(), |_| Ok(0)),
        Err(PackError::NoSpace)
    ));
    assert!(!dir.path().join("packs/test.zim").exists());
    assert!(!dir.path().join("packs/test.zim.part").exists());
    assert!(!dir.path().join("packs/test.zim.part.meta").exists());
}

#[test]
fn final_path_absent_until_verified() {
    let (dir, _server, pack) = fixture("absent");
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
}

#[test]
fn verified_existing_pack_needs_no_network() {
    let (dir, server, mut pack) = fixture("resume");
    fs::write(dir.path().join("packs/test.zim"), BODY).unwrap();
    pack.url = None;
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn head_forbidden_falls_back_to_range_probe_and_download() {
    let (dir, server, pack) = fixture("head403");
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.gets()[0].contains("range: bytes=0-0"));
}

#[test]
fn malformed_resume_range_preserves_installed_file() {
    let (dir, _server, pack) = fixture("bad206");
    partial(&dir, &pack, b"abc", "\"v2\"", 10);
    fs::write(dir.path().join("packs/test.zim"), b"installed").unwrap();
    assert!(matches!(fetch(&dir, &pack), Err(PackError::Http(_))));
    assert_eq!(
        fs::read(dir.path().join("packs/test.zim")).unwrap(),
        b"installed"
    );
}

#[test]
fn changed_resume_etag_restarts_without_appending() {
    let (dir, server, pack) = fixture("changed206");
    partial(&dir, &pack, b"abc", "\"v2\"", 10);
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert_eq!(server.gets().len(), 2);
    assert!(server.gets()[1].contains("range: bytes=0-"));
}

#[test]
fn interrupted_body_keeps_partial_for_next_run() {
    let (dir, server, pack) = fixture("truncate");
    assert!(fetch(&dir, &pack).is_err());
    assert!(!dir.path().join("packs/test.zim").exists());
    assert_eq!(
        fs::read(dir.path().join("packs/test.zim.part")).unwrap(),
        b"abc"
    );
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.gets()[1].contains("range: bytes=3-"));
}

#[test]
fn verified_download_replaces_previous_version() {
    let (dir, _server, pack) = fixture("resume");
    fs::write(dir.path().join("packs/test.zim"), b"old").unwrap();
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
}

#[test]
fn fresh_range_download_accepts_weak_etag_without_resuming_stale_bytes() {
    let (dir, server, pack) = fixture("weak");
    partial(&dir, &pack, b"BAD", "W/\"v2\"", 10);
    assert_eq!(fs::read(fetch(&dir, &pack).unwrap()).unwrap(), BODY);
    assert!(server.gets()[0].contains("range: bytes=0-"));
    assert!(!server.gets()[0].contains("if-range:"));
}
