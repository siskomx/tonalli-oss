use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{COOKIE, SET_COOKIE};
use tempfile::TempDir;
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::server::{LoopbackServer, PackId, PackRegistry};

const PACK_LEN: usize = 200_003;
#[allow(dead_code)]
#[path = "fixtures/pmtiles/mod.rs"]
mod pmtiles_fixture;
const ASSETS: &[&str] = &[
    "/assets/map/style.json",
    "/assets/map/fonts/Noto%20Sans%20Regular/0-255.pbf",
    "/assets/map/fonts/Noto%20Sans%20Regular/256-511.pbf",
    "/assets/map/sprites/sprite.json",
    "/assets/map/sprites/sprite.png",
    "/assets/map/sprites/sprite@2x.json",
    "/assets/map/sprites/sprite@2x.png",
    "/assets/map/attribution",
];

struct Fixture {
    server: LoopbackServer,
    client: Client,
    registry: Arc<PackRegistry>,
    _root: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        write_assets(&web);
        let pack = root.path().join("installed.pmtiles");
        fs::write(&pack, bytes()).unwrap();
        let registry = Arc::new(PackRegistry::default());
        registry
            .register_tiles(
                PackId::new("test").unwrap(),
                Arc::new(PreadSource::open(&pack).unwrap()),
            )
            .unwrap();
        let server =
            LoopbackServer::start(Arc::clone(&registry), root.path().join("private"), web).unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            server,
            client,
            registry,
            _root: root,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.server.info().port)
    }
    fn cookies(&self) -> (String, String) {
        let response = self
            .client
            .get(self.server.info().launch_url)
            .send()
            .unwrap();
        assert_eq!(response.status(), 302);
        let values: Vec<_> = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        (
            values
                .iter()
                .find(|value| value.starts_with("tonalli_content="))
                .unwrap()
                .clone(),
            values
                .iter()
                .find(|value| value.starts_with("tonalli_api="))
                .unwrap()
                .clone(),
        )
    }
    fn get(&self, path: &str, cookie: &str) -> reqwest::blocking::RequestBuilder {
        self.client.get(self.url(path)).header(COOKIE, cookie)
    }
    fn range(&self, range: &str, cookie: &str) -> Response {
        self.get("/packs/test.pmtiles", cookie)
            .header("Range", range)
            .send()
            .unwrap()
    }
    fn replace(&self, source: Arc<dyn PackSource>) {
        let id = PackId::new("test").unwrap();
        self.registry.remove(&id).unwrap();
        self.registry.register_tiles(id, source).unwrap();
    }
}

fn bytes() -> Vec<u8> {
    let mut payload: Vec<u8> = (0..PACK_LEN).map(|i| (i % 251) as u8).collect();
    let archive = |payload: &[u8]| {
        pmtiles_fixture::archive(
            &pmtiles_fixture::directory(&[(0, 1, payload.len() as u64, 0)]),
            &[],
            payload,
            1,
            1,
        )
    };
    let overhead = archive(&payload).len() - payload.len();
    payload.truncate(PACK_LEN - overhead);
    let bytes = archive(&payload);
    assert_eq!(bytes.len(), PACK_LEN);
    bytes
}

fn write_assets(web: &Path) {
    fs::create_dir_all(web.join("assets/map/fonts/Noto Sans Regular")).unwrap();
    fs::create_dir_all(web.join("assets/map/sprites")).unwrap();
    fs::write(web.join("index.html"), "<!doctype html><title>test</title>").unwrap();
    fs::write(web.join("assets/map/style.json"), br#"{"version":8,"glyphs":"/assets/map/fonts/{fontstack}/{range}.pbf","sprite":"/assets/map/sprites/sprite","sources":{"test":{"type":"vector","url":"pmtiles:///packs/test.pmtiles"}},"layers":[]}"#).unwrap();
    for range in ["0-255", "256-511"] {
        fs::write(
            web.join(format!("assets/map/fonts/Noto Sans Regular/{range}.pbf")),
            format!("fixture glyph range {range}"),
        )
        .unwrap();
    }
    for suffix in ["", "@2x"] {
        fs::write(
            web.join(format!("assets/map/sprites/sprite{suffix}.json")),
            br#"{"icon":{"x":0,"y":0,"width":1,"height":1,"pixelRatio":1}}"#,
        )
        .unwrap();
        fs::write(
            web.join(format!("assets/map/sprites/sprite{suffix}.png")),
            b"\x89PNG\r\n\x1a\nfixture",
        )
        .unwrap();
    }
    fs::write(
        web.join("assets/map/attribution"),
        "<!doctype html><title>Map credits</title><p>Map data © OpenStreetMap contributors; basemap © Protomaps</p>",
    )
    .unwrap();
}

#[test]
fn range_request_returns_206_and_exact_bytes() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    let response = fixture.range("bytes=127-4098", &cookie);
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 127-4098/{PACK_LEN}")
    );
    assert_eq!(response.headers()["content-length"], "3972");
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert!(!response.headers().contains_key("content-encoding"));
    assert_eq!(&response.bytes().unwrap()[..], &bytes()[127..=4098]);
}

#[test]
fn byte_range_unit_is_case_insensitive() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    let response = fixture.range("BYTES=0-126", &cookie);
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().unwrap().as_ref(), &bytes()[..127]);
}

#[test]
fn open_ended_suffix_and_clamped_ranges_are_exact() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for (range, start, end) in [
        ("bytes=199990-", 199990, PACK_LEN - 1),
        ("bytes=-13", PACK_LEN - 13, PACK_LEN - 1),
        ("bytes=200000-999999", 200000, PACK_LEN - 1),
        ("bytes=-999999", 0, PACK_LEN - 1),
        ("bytes=0-0", 0, 0),
    ] {
        let response = fixture.range(range, &cookie);
        assert_eq!(response.status(), 206, "{range}");
        assert_eq!(
            response.headers()["content-range"],
            format!("bytes {start}-{end}/{PACK_LEN}")
        );
        assert_eq!(response.bytes().unwrap().as_ref(), &bytes()[start..=end]);
    }
}

#[test]
fn unsatisfiable_and_invalid_byte_ranges_return_416() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for range in [
        "bytes=200003-",
        "bytes=9-1",
        "bytes=-0",
        "bytes=abc-def",
        "bytes=18446744073709551616-",
    ] {
        let response = fixture.range(range, &cookie);
        assert_eq!(response.status(), 416, "{range}");
        assert_eq!(
            response.headers()["content-range"],
            format!("bytes */{PACK_LEN}")
        );
    }
}

#[test]
fn full_get_and_unsupported_range_units_return_original_archive() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for response in [
        fixture.get("/packs/test.pmtiles", &cookie).send().unwrap(),
        fixture.range("items=0-3", &cookie),
    ] {
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-length"], PACK_LEN.to_string());
        assert!(!response.headers().contains_key("content-range"));
        assert!(!response.headers().contains_key("content-encoding"));
        assert_eq!(response.bytes().unwrap().as_ref(), bytes());
    }
}

#[test]
fn unsupported_multipart_and_unmatched_if_range_return_whole_archive() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    let requests = [
        fixture
            .get("/packs/test.pmtiles", &cookie)
            .header("Range", "bytes=0-2,6-8"),
        fixture
            .get("/packs/test.pmtiles", &cookie)
            .header("Range", "bytes=0-2")
            .header("If-Range", "\"unmatched-validator\""),
    ];
    for request in requests {
        let response = request.send().unwrap();
        assert_eq!(response.status(), 200);
        assert!(!response.headers().contains_key("content-range"));
        assert_eq!(response.bytes().unwrap().as_ref(), bytes());
    }
}

#[test]
fn head_ignores_range_and_does_not_read_the_source() {
    let fixture = Fixture::new();
    let source = Arc::new(TestSource::new(Failure::Always(PackError::MediaRemoved)));
    fixture.replace(source.clone());
    source.enable();
    let (cookie, _) = fixture.cookies();
    let response = fixture
        .client
        .head(fixture.url("/packs/test.pmtiles"))
        .header(COOKIE, cookie)
        .header("Range", "bytes=0-127")
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-length"], PACK_LEN.to_string());
    assert!(!response.headers().contains_key("content-range"));
    assert!(response.bytes().unwrap().is_empty());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn source_failure_before_headers_is_an_error_status() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for (error, status) in [(PackError::MediaRemoved, 503), (PackError::Truncated, 500)] {
        let source = Arc::new(TestSource::new(Failure::Always(error)));
        fixture.replace(source.clone());
        source.enable();
        assert_eq!(fixture.range("bytes=0-127", &cookie).status(), status);
    }
}

#[test]
fn source_failure_mid_stream_never_becomes_a_successful_short_body() {
    let fixture = Fixture::new();
    let source = Arc::new(TestSource::new(Failure::AfterFirstRead));
    fixture.replace(source.clone());
    source.enable();
    let (cookie, _) = fixture.cookies();
    if let Ok(response) = fixture
        .get("/packs/test.pmtiles", &cookie)
        .header("Range", "bytes=0-200002")
        .send()
    {
        assert_eq!(response.status(), 206);
        assert!(response.bytes().is_err());
    }
}

#[test]
fn positioned_short_reads_are_retried_with_bounded_buffers() {
    let fixture = Fixture::new();
    let source = Arc::new(TestSource::new(Failure::ShortReads));
    fixture.replace(source.clone());
    source.enable();
    let (cookie, _) = fixture.cookies();
    let response = fixture.range("bytes=100-130000", &cookie);
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().unwrap().as_ref(), &bytes()[100..=130000]);
    assert!(source.max_buffer.load(Ordering::SeqCst) <= 64 * 1024);
    assert!(source.calls.load(Ordering::SeqCst) > 1);
}

#[test]
fn map_routes_without_credential_are_401() {
    let fixture = Fixture::new();
    let (_, api) = fixture.cookies();
    for path in std::iter::once("/packs/test.pmtiles").chain(ASSETS.iter().copied()) {
        for cookie in [None, Some(api.as_str())] {
            let mut request = fixture
                .client
                .get(fixture.url(path))
                .header("Range", "bytes=0-126");
            if let Some(cookie) = cookie {
                request = request.header(COOKIE, cookie);
            }
            assert_eq!(request.send().unwrap().status(), 401, "{path}");
        }
    }
}

#[test]
fn unregistered_removed_or_pathlike_pack_ids_are_not_served() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    fixture
        .registry
        .remove(&PackId::new("test").unwrap())
        .unwrap();
    for path in [
        "/packs/test.pmtiles",
        "/packs/missing.pmtiles",
        "/packs/%2e%2e%2fprivate%2fnotes.db.pmtiles",
    ] {
        assert_eq!(
            fixture
                .get(path, &cookie)
                .header("Range", "bytes=0-2")
                .send()
                .unwrap()
                .status(),
            404
        );
    }
}

#[test]
fn style_json_has_no_external_urls() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    let response = fixture
        .get("/assets/map/style.json", &cookie)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let style = response.json::<serde_json::Value>().unwrap();
    check_urls(&style, &fixture.url(""));
    assert_eq!(style["glyphs"], "/assets/map/fonts/{fontstack}/{range}.pbf");
    assert_eq!(style["sprite"], "/assets/map/sprites/sprite");
}
fn check_urls(value: &serde_json::Value, origin: &str) {
    match value {
        serde_json::Value::String(text) => {
            let text = text.strip_prefix("pmtiles://").unwrap_or(text);
            if text.starts_with("http://") || text.starts_with("https://") {
                assert!(
                    text.starts_with(&format!("{origin}/")),
                    "external style URL: {text}"
                );
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                check_urls(value, origin);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                check_urls(value, origin);
            }
        }
        _ => {}
    }
}

#[test]
fn glyph_range_for_latin_and_accents_returns_200() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for (path, range) in [(ASSETS[1], "0-255"), (ASSETS[2], "256-511")] {
        let response = fixture.get(path, &cookie).send().unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "application/x-protobuf");
        assert_eq!(
            response.text().unwrap(),
            format!("fixture glyph range {range}")
        );
    }
}

#[test]
fn sprite_json_and_png_return_200() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    for path in &ASSETS[3..7] {
        let response = fixture.get(path, &cookie).send().unwrap();
        assert_eq!(response.status(), 200);
        if path.ends_with(".json") {
            assert_eq!(response.headers()["content-type"], "application/json");
            assert!(response.json::<serde_json::Value>().unwrap()["icon"].is_object());
        } else {
            assert_eq!(response.headers()["content-type"], "image/png");
            assert!(response.bytes().unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }
}

#[test]
fn attribution_text_contains_openstreetmap() {
    let fixture = Fixture::new();
    let (cookie, _) = fixture.cookies();
    let response = fixture
        .get("/assets/map/attribution", &cookie)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(response.text().unwrap().contains("OpenStreetMap"));
}

#[test]
fn installed_map_inventory_requires_full_api_credentials() {
    let fixture = Fixture::new();
    let (content, api) = fixture.cookies();
    for cookie in [None, Some(content.as_str())] {
        let mut request = fixture.client.get(fixture.url("/api/maps"));
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        assert_eq!(request.send().unwrap().status(), 401);
    }
    let session = fixture
        .get("/api/session", &api)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .unwrap()
        .json::<serde_json::Value>()
        .unwrap();
    let csrf = session["csrf"].as_str().unwrap();
    for destination in [None, Some("image"), Some("empty")] {
        let mut request = fixture.get("/api/maps", &api);
        if let Some(destination) = destination {
            request = request.header("Sec-Fetch-Dest", destination);
        }
        assert_eq!(request.send().unwrap().status(), 403);
    }
    let response = fixture
        .get("/api/maps", &api)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", csrf)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let json = response.json::<serde_json::Value>().unwrap();
    assert_eq!(json["packs"].as_array().unwrap().len(), 1);
    assert_eq!(json["packs"][0]["id"], "test");
    assert_eq!(
        json["packs"][0]["bounds"],
        serde_json::json!([-100.0, 10.0, -99.0, 20.0])
    );
    assert_eq!(json["packs"][0]["center"], serde_json::json!([-99.5, 15.0]));
    assert_eq!(json["packs"][0]["min_zoom"], 0);
    assert_eq!(json["packs"][0]["max_zoom"], 8);
    assert_eq!(
        json["packs"][0]["attribution"],
        "OpenStreetMap contributors"
    );
    fixture
        .registry
        .remove(&PackId::new("test").unwrap())
        .unwrap();
    let json = fixture
        .get("/api/maps", &api)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", csrf)
        .send()
        .unwrap()
        .json::<serde_json::Value>()
        .unwrap();
    assert_eq!(json["packs"], serde_json::json!([]));
}

#[test]
fn invalid_archive_cannot_enter_the_installed_map_registry() {
    let registry = PackRegistry::default();
    let source = Arc::new(pmtiles_fixture::Source::new(
        b"not a PMTiles archive".to_vec(),
    ));
    assert!(registry
        .register_tiles(PackId::new("invalid").unwrap(), source)
        .is_err());
    assert!(matches!(
        registry.remove(&PackId::new("invalid").unwrap()),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn blocked_positioned_read_does_not_block_the_http_reactor() {
    let fixture = Fixture::new();
    let (content, api) = fixture.cookies();
    let (entered, waiting) = mpsc::sync_channel(1);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let source = Arc::new(BlockingSource {
        inner: TestSource::new(Failure::ShortReads),
        armed: AtomicBool::new(false),
        entered,
        release: Arc::clone(&release),
    });
    fixture.replace(source.clone());
    source.armed.store(true, Ordering::SeqCst);
    let request = fixture
        .get("/packs/test.pmtiles", &content)
        .header("Range", "bytes=0-127");
    let download = std::thread::spawn(move || request.send().and_then(|response| response.bytes()));
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let session = fixture
        .get("/api/session", &api)
        .header("Sec-Fetch-Dest", "empty")
        .timeout(Duration::from_secs(2))
        .send();
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    let downloaded = download.join().unwrap().unwrap();
    assert_eq!(session.unwrap().status(), 200);
    assert_eq!(downloaded.as_ref(), &bytes()[..128]);
}

struct BlockingSource {
    inner: TestSource,
    armed: AtomicBool,
    entered: mpsc::SyncSender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}
impl PackSource for BlockingSource {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered
                .send(())
                .map_err(|error| PackError::Io(error.to_string()))?;
            let released = self.release.0.lock().unwrap();
            let (released, _) = self
                .release
                .1
                .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
                .unwrap();
            if !*released {
                return Err(PackError::Io("test read gate timed out".into()));
            }
        }
        self.inner.read_at(offset, buffer)
    }
}

enum Failure {
    Always(PackError),
    AfterFirstRead,
    ShortReads,
}
struct TestSource {
    failure: Failure,
    enabled: AtomicBool,
    bytes: Vec<u8>,
    calls: AtomicUsize,
    max_buffer: AtomicUsize,
}
impl TestSource {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            enabled: AtomicBool::new(false),
            bytes: bytes(),
            calls: AtomicUsize::new(0),
            max_buffer: AtomicUsize::new(0),
        }
    }
    fn enable(&self) {
        self.calls.store(0, Ordering::SeqCst);
        self.max_buffer.store(0, Ordering::SeqCst);
        self.enabled.store(true, Ordering::SeqCst);
    }
}
impl PackSource for TestSource {
    fn len(&self) -> u64 {
        PACK_LEN as u64
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.max_buffer.fetch_max(buffer.len(), Ordering::SeqCst);
        match &self.failure {
            _ if !self.enabled.load(Ordering::SeqCst) => {}
            Failure::Always(error) => return Err(error.clone()),
            Failure::AfterFirstRead if call > 0 => return Err(PackError::MediaRemoved),
            _ => {}
        }
        let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
        if start >= PACK_LEN {
            return Err(PackError::Truncated);
        }
        let count = buffer.len().min(PACK_LEN - start).min(
            if self.enabled.load(Ordering::SeqCst) && matches!(self.failure, Failure::ShortReads) {
                997
            } else {
                usize::MAX
            },
        );
        buffer[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}
