use super::Host;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tonalli_core::{
    pack::{PackError, PackSource, PreadSource},
    server::{CopyDispatcher, CopyOwner, LoopbackServer, PackRegistry},
};

// Delay a real pack read during the TLS manifest request; no transport or bytes are simulated.
struct ReadGate {
    source: PreadSource,
    armed: AtomicBool,
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl ReadGate {
    fn new() -> Self {
        Self {
            source: PreadSource::open(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/articles/benign-v1.zim")
                    .as_path(),
            )
            .unwrap(),
            armed: AtomicBool::new(false),
            state: Mutex::new((false, false)),
            changed: Condvar::new(),
        }
    }
    fn wait_entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |s| !s.0)
            .unwrap();
        assert!(state.0, "real TLS manifest must reach its selected pack");
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
impl PackSource for ReadGate {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        if self.armed.load(Ordering::SeqCst) {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.changed.notify_all();
            let (state, _) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(10), |s| !s.1)
                .unwrap();
            if !state.1 {
                return Err(PackError::Io("owned read gate expired".into()));
            }
        }
        self.source.read_at(offset, buffer)
    }
}
struct Admission(AtomicUsize);
impl CopyDispatcher for Admission {
    fn dispatch(&self, epoch: u64, ipv6: bool) -> Result<std::net::IpAddr, PackError> {
        assert!(epoch > 0);
        assert!(!ipv6);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(std::net::Ipv4Addr::LOCALHOST.into())
    }
}

struct FixedSource(std::net::IpAddr);
impl CopyDispatcher for FixedSource {
    fn dispatch(&self, _epoch: u64, _ipv6: bool) -> Result<std::net::IpAddr, PackError> {
        Ok(self.0)
    }
}
#[tokio::test]
async fn copy_owner_rejects_wildcard_public_and_wrong_family_source_before_network_work() {
    let host = Host::new();
    for source in ["0.0.0.0", "203.0.113.1", "::1"] {
        let directory = tempfile::tempdir().unwrap();
        let owner = CopyOwner::open(
            &directory.path().join("data"),
            Arc::new(FixedSource(source.parse().unwrap())),
        )
        .unwrap();
        let mut app = view(directory.path());
        app.attach_copy_owner(&owner).unwrap();
        let http = HttpView::open(&app).await;
        let invitation = tonalli_core::remote::invitation(
            host.server.local_addr(),
            &host.server.certificate_der(),
        );
        assert_eq!(http.request(reqwest::Method::POST, "/api/remote/connect").json(&serde_json::json!({"invitation":invitation,"code":host.server.pairing_code().unwrap(),"label":"Owned binding refusal"})).send().await.unwrap().status(), 200);
        assert_eq!(
            http.request(reqwest::Method::POST, "/api/remote/copy")
                .json(&serde_json::json!({"id":"visible"}))
                .send()
                .await
                .unwrap()
                .status(),
            503,
            "source {source}"
        );
        assert!(!owner.is_running().unwrap());
        assert!(
            tonalli_core::sync::SyncStore::open(&directory.path().join("data"))
                .unwrap()
                .list()
                .unwrap()
                .is_empty()
        );
        owner.close();
        app.shutdown().unwrap();
    }
}
fn view(root: &Path) -> LoopbackServer {
    let web = root.join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "Owned background copy view").unwrap();
    LoopbackServer::start(Arc::new(PackRegistry::default()), root.join("data"), web).unwrap()
}
struct HttpView {
    http: reqwest::Client,
    origin: String,
    cookie: String,
    csrf: String,
}
impl HttpView {
    async fn open(server: &LoopbackServer) -> Self {
        let info = server.info();
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let launch = http.get(info.launch_url).send().await.unwrap();
        assert_eq!(launch.status(), 302);
        let cookie = launch
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let origin = format!("http://127.0.0.1:{}", info.port);
        let session: serde_json::Value = http
            .get(format!("{origin}/api/session"))
            .header("cookie", &cookie)
            .header("sec-fetch-dest", "empty")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            http,
            origin,
            cookie,
            csrf: session["csrf"].as_str().unwrap().to_owned(),
        }
    }
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.origin))
            .header("cookie", &self.cookie)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", &self.csrf)
    }
}

#[tokio::test]
async fn copy_owner_real_transfer_outlives_private_view_and_publishes_in_reopened_library() {
    let source = Arc::new(ReadGate::new());
    let host = Host::with_source(source.clone());
    let directory = tempfile::tempdir().unwrap();
    let admission = Arc::new(Admission(AtomicUsize::new(0)));
    let owner = CopyOwner::open(&directory.path().join("data"), admission.clone()).unwrap();
    let mut first = view(directory.path());
    first.attach_copy_owner(&owner).unwrap();
    let old = HttpView::open(&first).await;
    let invitation =
        tonalli_core::remote::invitation(host.server.local_addr(), &host.server.certificate_der());
    assert_eq!(old.request(reqwest::Method::POST, "/api/remote/connect").json(&serde_json::json!({"invitation":invitation,"code":host.server.pairing_code().unwrap(),"label":"Owned background reader"})).send().await.unwrap().status(), 200);
    source.armed.store(true, Ordering::SeqCst);
    assert_eq!(
        old.request(reqwest::Method::POST, "/api/remote/copy")
            .json(&serde_json::json!({"id":"visible"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    let gate = source.clone();
    tokio::task::spawn_blocking(move || gate.wait_entered())
        .await
        .unwrap();
    assert!(owner.is_running().unwrap());
    assert_eq!(admission.0.load(Ordering::SeqCst), 1);
    first.shutdown().unwrap();
    drop(first);
    assert!(
        old.request(reqwest::Method::GET, "/api/session")
            .send()
            .await
            .is_err(),
        "old private listener must close while copy is active"
    );
    let mut second = view(directory.path());
    second.attach_copy_owner(&owner).unwrap();
    let current = HttpView::open(&second).await;
    let preparing: serde_json::Value = current
        .request(reqwest::Method::GET, "/api/remote")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preparing["connected"], false);
    assert_eq!(preparing["copy"]["state"], "preparing");
    source.release();
    let deadline = Instant::now() + Duration::from_secs(10);
    while owner.is_running().unwrap() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let status: serde_json::Value = current
        .request(reqwest::Method::GET, "/api/remote")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["copy"]["state"], "complete", "{status}");
    let local = status["copy"]["local_id"].as_str().unwrap();
    let packs: serde_json::Value = current
        .request(reqwest::Method::GET, "/api/packs")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs["packs"][0]["id"], local);
    assert_eq!(
        current
            .request(
                reqwest::Method::GET,
                &format!("/zim/{local}/C/en/index.html")
            )
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(!directory.path().join("data/notes.db").exists());
    assert_eq!(
        admission.0.load(Ordering::SeqCst),
        1,
        "reopening must not dispatch a new copy"
    );
    owner.close();
    second.shutdown().unwrap();
}
