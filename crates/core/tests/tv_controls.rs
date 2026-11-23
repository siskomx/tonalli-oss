use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{COOKIE, SET_COOKIE},
    Method,
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tonalli_core::{
    media::{MediaFile, MediaStore},
    pack::{PackError, PreadSource},
    server::{LoopbackServer, PackRegistry, TvCatalog, TvControl, TvServer, TvStatus},
};
const WAV: &[u8] =
    b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0";
#[derive(Default)]
struct Owner {
    server: Mutex<Option<TvServer>>,
    starts: AtomicUsize,
    closes: AtomicUsize,
    closed: AtomicBool,
}
impl TvControl for Owner {
    fn status(&self) -> Result<TvStatus, PackError> {
        let server = self.server.lock().unwrap();
        Ok(TvStatus {
            running: server.is_some(),
            url: server
                .as_ref()
                .map(|s| format!("http://{}/description.xml", s.address())),
            ..TvStatus::default()
        })
    }
    fn start(&self, network: String, catalog: Arc<TvCatalog>) -> Result<(), PackError> {
        assert_eq!(network, "owned-tv-network");
        self.starts.fetch_add(1, Ordering::SeqCst);
        let mut server = self.server.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) || server.is_some() {
            return Err(PackError::Mismatch("already started or closed".into()));
        }
        *server = Some(TvServer::start("127.0.0.1".parse().unwrap(), 0, catalog)?);
        Ok(())
    }
    fn stop(&self) -> Result<(), PackError> {
        if let Some(mut server) = self.server.lock().unwrap().take() {
            server.shutdown()?;
        }
        Ok(())
    }
    fn close(&self) -> Result<(), PackError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        self.closed.store(true, Ordering::SeqCst);
        self.stop()
    }
}
struct App {
    server: LoopbackServer,
    client: Client,
    cookie: String,
    csrf: String,
    root: tempfile::TempDir,
}
impl App {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        fs::create_dir(&web).unwrap();
        fs::write(web.join("index.html"), "fixture").unwrap();
        let server = LoopbackServer::start(
            Arc::new(PackRegistry::default()),
            root.path().join("data"),
            web,
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let launch = client.get(server.info().launch_url).send().unwrap();
        let cookie = launch
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let session: Value = client
            .get(format!(
                "http://127.0.0.1:{}/api/session",
                server.info().port
            ))
            .header(COOKIE, &cookie)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .json()
            .unwrap();
        Self {
            server,
            client,
            cookie,
            csrf: session["csrf"].as_str().unwrap().into(),
            root,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.server.info().port)
    }
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, self.url(path))
            .header(COOKIE, &self.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &self.csrf)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> MediaFile {
        let source = self.root.path().join(name);
        fs::write(&source, bytes).unwrap();
        MediaStore::open(&self.root.path().join("data"))
            .unwrap()
            .import(Arc::new(PreadSource::open(&source).unwrap()), name, "")
            .unwrap()
    }
}
#[test]
fn tv_controls_require_private_session_csrf_and_an_attached_native_owner() {
    let app = App::new();
    let session: Value = app
        .request(Method::GET, "/api/session")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(session["private_tv"], false);
    assert_eq!(
        app.client.get(app.url("/api/tv")).send().unwrap().status(),
        401
    );
    assert_eq!(
        app.request(Method::GET, "/api/tv").send().unwrap().status(),
        503
    );
    let owner = Arc::new(Owner::default());
    app.server.attach_tv(owner.clone()).unwrap();
    let session: Value = app
        .request(Method::GET, "/api/session")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(session["private_tv"], true);
    let off: Value = app
        .request(Method::GET, "/api/tv")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(off["status"]["running"], false);
    assert_eq!(off["selected_media"], json!([]));
    assert_eq!(
        app.client
            .post(app.url("/api/tv/start"))
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .json(&json!({"network":"owned-tv-network","media":[]}))
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(owner.starts.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.request(Method::POST, "/api/tv/start")
            .json(&json!({"network":"x".repeat(20000),"media":[]}))
            .send()
            .unwrap()
            .status(),
        413
    );
    assert!(
        app.server.attach_tv(Arc::new(Owner::default())).is_err(),
        "cannot replace and orphan an attached owner"
    );
}
#[test]
fn tv_controls_validate_selection_before_dispatch_and_only_serve_selected_media() {
    let app = App::new();
    let selected = app.file("Selected.wav", WAV);
    let hidden = app.file("Hidden.wav", WAV);
    let text = app.file("Private.txt", b"PRIVATE SENTINEL");
    let owner = Arc::new(Owner::default());
    app.server.attach_tv(owner.clone()).unwrap();
    for media in [
        vec![],
        vec![text.id.clone()],
        vec!["../notes.db".into()],
        vec![selected.id.clone(); 129],
    ] {
        assert_eq!(
            app.request(Method::POST, "/api/tv/start")
                .json(&json!({"network":"owned-tv-network","media":media}))
                .send()
                .unwrap()
                .status(),
            409
        );
    }
    assert_eq!(owner.starts.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.request(Method::POST, "/api/tv/start")
            .json(&json!({"network":"owned-tv-network","media":[selected.id],"path":"../notes.db"}))
            .send()
            .unwrap()
            .status(),
        422
    );
    assert_eq!(
        app.request(Method::POST, "/api/tv/start")
            .json(&json!({"network":"owned-tv-network","media":[selected.id]}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(owner.starts.load(Ordering::SeqCst), 1);
    let status: Value = app
        .request(Method::GET, "/api/tv")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["selected_media"].as_array().unwrap().len(), 1);
    assert_eq!(status["selected_media"][0]["id"], selected.id);
    let base = status["status"]["url"]
        .as_str()
        .unwrap()
        .strip_suffix("/description.xml")
        .unwrap();
    let bytes = app
        .client
        .get(format!("{base}/media/{}", selected.id))
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    assert_eq!(bytes.as_ref(), WAV);
    for path in [
        format!("/media/{}", hidden.id),
        "/api/tv".into(),
        "/api/notes".into(),
        "/api/settings".into(),
        "/api/files".into(),
    ] {
        assert_eq!(
            app.client
                .get(format!("{base}{path}"))
                .send()
                .unwrap()
                .status(),
            404
        );
    }
    assert_eq!(
        app.request(Method::POST, "/api/tv/stop")
            .send()
            .unwrap()
            .status(),
        200
    );
    let off: Value = app
        .request(Method::GET, "/api/tv")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(off["status"]["running"], false);
    assert_eq!(off["selected_media"], json!([]));
    assert_eq!(fs::read(app.root.path().join("Selected.wav")).unwrap(), WAV);
}
#[test]
fn private_host_shutdown_closes_tv_owner_once() {
    let mut app = App::new();
    let file = app.file("Selected.wav", WAV);
    let owner = Arc::new(Owner::default());
    app.server.attach_tv(owner.clone()).unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/tv/start")
            .json(&json!({"network":"owned-tv-network","media":[file.id]}))
            .send()
            .unwrap()
            .status(),
        200
    );
    let address = owner.server.lock().unwrap().as_ref().unwrap().address();
    app.server.shutdown().unwrap();
    app.server.shutdown().unwrap();
    assert_eq!(owner.closes.load(Ordering::SeqCst), 1);
    assert!(!owner.status().unwrap().running);
    assert!(
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(200))
            .is_err()
    );
}

#[test]
fn tv_is_closed_even_when_an_independent_browser_share_owner_fails_cleanup() {
    use tonalli_core::server::{SharedMedia, SharingControl, SharingStatus};
    struct BrokenBrowser;
    impl SharingControl for BrokenBrowser {
        fn status(&self) -> Result<SharingStatus, PackError> {
            Err(PackError::Io("synthetic browser failure".into()))
        }
        fn start(&self, _: String, _: Vec<String>, _: Arc<SharedMedia>) -> Result<(), PackError> {
            self.stop()
        }
        fn stop(&self) -> Result<(), PackError> {
            Err(PackError::Io("synthetic browser failure".into()))
        }
        fn kick(&self, _: String) -> Result<(), PackError> {
            self.stop()
        }
    }
    let mut app = App::new();
    let file = app.file("Selected.wav", WAV);
    let owner = Arc::new(Owner::default());
    app.server.attach_tv(owner.clone()).unwrap();
    app.server.attach_sharing(Arc::new(BrokenBrowser)).unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/tv/start")
            .json(&json!({"network":"owned-tv-network","media":[file.id]}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert!(app.server.shutdown().is_err());
    assert_eq!(owner.closes.load(Ordering::SeqCst), 1);
    assert!(!owner.status().unwrap().running);
}
