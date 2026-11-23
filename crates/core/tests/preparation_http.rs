use reqwest::{Client, Method, RequestBuilder};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tonalli_core::{
    index::{
        jobs::{Action, IndexJobs, Ticket},
        BuildControl,
    },
    pack::PreadSource,
    remote,
    server::{
        ContentDispatchError, LoopbackServer, PackId, PackRegistry, PreparationDispatcher, Server,
        ServerCfg,
    },
    zim::Zim,
};

// This owner runs the actual builder/cleanup on native threads; no HTTP fixture responses.
struct NativeThreads {
    registry: Arc<PackRegistry>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}
impl PreparationDispatcher for NativeThreads {
    fn dispatch(&self, jobs: Arc<IndexJobs>, ticket: Ticket) -> Result<(), ContentDispatchError> {
        let registry = Arc::clone(&self.registry);
        let task = std::thread::Builder::new()
            .name("owned-preparation".into())
            .spawn(move || {
                let id = PackId::new(&ticket.pack).unwrap();
                let mut run = jobs.reserve(&ticket).unwrap();
                if !jobs.execute_unbound_control(&mut run).unwrap() {
                    let source = registry.preparation_source(&id);
                    if let Err(error) = source {
                        jobs.fail_unbound(&mut run, error).unwrap();
                        jobs.finish_exited(&mut run).unwrap();
                        return;
                    }
                    let source = source.unwrap();
                    if jobs.bind_source(&mut run, &source).is_err() {
                        drop(source);
                        jobs.finish_exited(&mut run).unwrap();
                        return;
                    }
                    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
                        .unwrap();
                    if let Some(index) = jobs.safe_ready(&source).unwrap() {
                        registry.attach_index(id, Arc::new(index)).unwrap();
                    }
                    drop(source);
                }
                jobs.finish_exited(&mut run).unwrap();
            })
            .map_err(|_| ContentDispatchError::Rejected)?;
        self.tasks.lock().unwrap().push(task);
        Ok(())
    }
    fn control(
        &self,
        _jobs: Arc<IndexJobs>,
        _ticket: Ticket,
        _action: Action,
    ) -> Result<(), ContentDispatchError> {
        // The real worker is already running/accepted and polls durable controls.
        Ok(())
    }
}
impl Drop for NativeThreads {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.join().unwrap();
        }
    }
}

fn book_registry() -> Arc<PackRegistry> {
    let registry = Arc::new(PackRegistry::default());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    registry
        .register_zim(
            PackId::new("book").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
        )
        .unwrap();
    registry
}

struct LocalClient {
    http: Client,
    origin: String,
    cookie: String,
    csrf: String,
}
impl LocalClient {
    async fn new(app: &LoopbackServer) -> Self {
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let launch = http.get(app.info().launch_url).send().await.unwrap();
        let cookie = launch
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let origin = format!("http://127.0.0.1:{}", app.info().port);
        let session: Value = http
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
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.origin))
            .header("cookie", &self.cookie)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", &self.csrf)
    }
    async fn status(&self) -> Value {
        let response = self
            .request(Method::GET, "/api/zim/index-status?pack=book")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: Value = response.json().await.unwrap();
        assert_eq!(value.as_object().unwrap().len(), 8);
        for key in [
            "pack",
            "jobId",
            "epoch",
            "state",
            "phase",
            "processedArticles",
            "committedArticles",
            "reason",
        ] {
            assert!(value.get(key).is_some(), "{key}");
        }
        assert_eq!(value["pack"], "book");
        value
    }
    async fn wait(&self, state: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let value = self.status().await;
            if value["state"] == state {
                return value;
            }
            assert!(Instant::now() < deadline, "expected {state}: {value}");
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    async fn control(&self, snapshot: &Value, action: &str) -> reqwest::Response {
        self.request(Method::POST, "/api/zim/index-control")
            .json(&json!({"jobId":snapshot["jobId"],"epoch":snapshot["epoch"],"action":action}))
            .send()
            .await
            .unwrap()
    }
}

fn web(root: &Path) -> PathBuf {
    let path = root.join("web");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("index.html"), "Owned native preparation fixture").unwrap();
    path
}

fn small_app(root: &Path) -> (LoopbackServer, Arc<PackRegistry>, Arc<IndexJobs>) {
    let registry = Arc::new(PackRegistry::default());
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    registry
        .register_zim(
            PackId::new("book").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
        )
        .unwrap();
    let data = root.join("private");
    let app = LoopbackServer::start(Arc::clone(&registry), data.clone(), web(root)).unwrap();
    let jobs = Arc::new(IndexJobs::open(&data).unwrap());
    app.attach_preparation(
        Arc::clone(&jobs),
        Arc::new(NativeThreads {
            registry: Arc::clone(&registry),
            tasks: Mutex::new(Vec::new()),
        }),
    )
    .unwrap();
    (app, registry, jobs)
}

#[tokio::test]
async fn late_stop_after_completion_returns_current_source_state_without_false_error() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, registry, _jobs) = small_app(root.path());
    let local = LocalClient::new(&app).await;
    let started = local
        .request(Method::POST, "/api/zim/index-start")
        .json(&json!({"pack":"book"}))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);
    let ticket: Value = started.json().await.unwrap();
    local.wait("ready").await;
    for action in ["pause", "cancel"] {
        let response = local.control(&ticket, action).await;
        assert_eq!(
            response.status(),
            202,
            "a completed generation wins a late stop"
        );
        assert_eq!(response.json::<Value>().await.unwrap()["state"], "ready");
    }
    registry.remove(&PackId::new("book").unwrap()).unwrap();
    let replacement =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/hostile-v1.zim");
    registry
        .register_zim(
            PackId::new("book").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&replacement).unwrap())).unwrap()),
        )
        .unwrap();
    let response = local.control(&ticket, "pause").await;
    assert_eq!(response.status(), 202);
    let actual: Value = response.json().await.unwrap();
    assert_eq!(
        actual["state"], "unprepared",
        "never report an old source's Ready for its replacement"
    );
    assert!(actual["jobId"].is_null());
    let mut stale = ticket;
    let epoch = stale["epoch"].as_str().unwrap().parse::<u64>().unwrap();
    stale["epoch"] = Value::String((epoch + 1).to_string());
    assert_eq!(local.control(&stale, "pause").await.status(), 409);
    app.shutdown().unwrap();
}

#[tokio::test]
async fn finished_preparation_of_replaced_book_does_not_trap_current_ui_in_cancel() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, registry, jobs) = small_app(root.path());
    let id = PackId::new("book").unwrap();
    let source = registry.preparation_source(&id).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&first.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let local = LocalClient::new(&app).await;
    assert_eq!(local.status().await["state"], "ready");
    registry.remove(&id).unwrap();
    let replacement =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/hostile-v1.zim");
    registry
        .register_zim(
            id,
            Arc::new(Zim::open(Arc::new(PreadSource::open(&replacement).unwrap())).unwrap()),
        )
        .unwrap();
    let current = local.status().await;
    assert_eq!(current["state"], "unprepared");
    assert!(current["jobId"].is_null());
    assert_eq!(
        local
            .request(Method::POST, "/api/zim/index-start")
            .json(&json!({"pack":"book"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    local.wait("ready").await;
    assert!(
        jobs.safe_ready(&source).unwrap().is_some(),
        "the old immutable generation must remain intact"
    );
    app.shutdown().unwrap();
}

#[tokio::test]
async fn preparation_inputs_are_bounded_and_cannot_supply_native_authority() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _registry, jobs) = small_app(root.path());
    let local = LocalClient::new(&app).await;
    for epoch in [
        json!(1),
        json!("0"),
        json!("01"),
        json!("+1"),
        json!("1e3"),
        json!("18446744073709551616"),
    ] {
        assert_eq!(
            local
                .request(Method::POST, "/api/zim/index-control")
                .json(&json!({"jobId":"a".repeat(32),"epoch":epoch,"action":"cancel"}))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    assert_eq!(
        local
            .request(Method::POST, "/api/zim/index-start")
            .json(&json!({"pack":"book","padding":"x".repeat(2048)}))
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    assert_eq!(
        local
            .request(Method::GET, "/api/zim/index-status?pack=book&path=outside")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert!(jobs
        .status(&PackId::new("book").unwrap())
        .unwrap()
        .is_none());
    app.shutdown().unwrap();
}

#[tokio::test]
async fn private_preparation_uses_existing_contract_real_native_work_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let web = web(root.path());
    let data = root.path().join("private");
    let registry = book_registry();
    let mut app = LoopbackServer::start(Arc::clone(&registry), data.clone(), web.clone()).unwrap();
    let local = LocalClient::new(&app).await;
    let path = "/api/zim/index-status?pack=book";
    assert_eq!(
        local
            .http
            .get(format!("{}{path}", local.origin))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        local
            .http
            .get(format!("{}{path}", local.origin))
            .header("cookie", &local.cookie)
            .header("sec-fetch-dest", "empty")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        local
            .request(Method::GET, path)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    let jobs = Arc::new(IndexJobs::open(&data).unwrap());
    let owner = Arc::new(NativeThreads {
        registry: Arc::clone(&registry),
        tasks: Mutex::new(Vec::new()),
    });
    let other = Arc::new(IndexJobs::open(&root.path().join("other")).unwrap());
    assert!(app.attach_preparation(other, owner.clone()).is_err());
    app.attach_preparation(Arc::clone(&jobs), owner.clone())
        .unwrap();
    assert!(app.attach_preparation(Arc::clone(&jobs), owner).is_err());
    let session: Value = local
        .request(Method::GET, "/api/session")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["private_index_preparation"], true);
    assert_eq!(
        local.status().await,
        json!({"pack":"book","jobId":null,"epoch":null,"state":"unprepared","phase":null,"processedArticles":null,"committedArticles":null,"reason":null})
    );
    for body in [
        json!({"pack":"../book"}),
        json!({"pack":"book","path":"outside"}),
        json!({"pack":"book","sha256":"a".repeat(64)}),
    ] {
        assert_eq!(
            local
                .request(Method::POST, "/api/zim/index-start")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    assert!(jobs
        .status(&PackId::new("book").unwrap())
        .unwrap()
        .is_none());
    let first = local
        .request(Method::POST, "/api/zim/index-start")
        .json(&json!({"pack":"book"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 202);
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["jobId"].as_str().unwrap().len(), 32);
    assert!(first["epoch"].is_string());
    assert_eq!(local.control(&first, "pause").await.status(), 202);
    let paused = local.wait("paused").await;
    assert_eq!(local.control(&paused, "cancel").await.status(), 202);
    local.wait("cancelled").await;
    let next = local
        .request(Method::POST, "/api/zim/index-start")
        .json(&json!({"pack":"book"}))
        .send()
        .await
        .unwrap();
    assert_eq!(next.status(), 202);
    assert_eq!(local.control(&first, "cancel").await.status(), 409);
    local.wait("ready").await;
    let search = local
        .request(Method::GET, "/api/zim/search?pack=book&q=knot&limit=20")
        .send()
        .await
        .unwrap();
    assert_eq!(search.status(), 200);
    let search: Value = search.json().await.unwrap();
    assert!(!search["hits"].as_array().unwrap().is_empty());
    app.shutdown().unwrap();
    drop(app);
    drop(registry);
    let registry = book_registry();
    let mut reopened =
        LoopbackServer::start(Arc::clone(&registry), data.clone(), web.clone()).unwrap();
    reopened
        .attach_preparation(
            Arc::new(IndexJobs::open(&data).unwrap()),
            Arc::new(NativeThreads {
                registry: Arc::clone(&registry),
                tasks: Mutex::new(Vec::new()),
            }),
        )
        .unwrap();
    let local = LocalClient::new(&reopened).await;
    // Restoration must serve the index before a preparation-status request repairs anything.
    assert_eq!(
        local
            .request(Method::GET, "/api/zim/search?pack=book&q=knot&limit=20")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(local.status().await["state"], "ready");
    let host = Server::start(
        ServerCfg {
            web_root: web,
            identity_dir: root.path().join("identity"),
            packs: vec![PackId::new("book").unwrap()],
            ..ServerCfg::default()
        },
        registry,
    )
    .unwrap();
    let invitation = remote::invitation(host.local_addr(), &host.certificate_der());
    assert_eq!(local.request(Method::POST,"/api/remote/connect").json(&json!({"invitation":invitation,"code":host.pairing_code().unwrap(),"label":"Owned preparation reader"})).send().await.unwrap().status(),200);
    let session: Value = local
        .request(Method::GET, "/api/session")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["private_index_preparation"], false);
    for route in [
        "/api/zim/index-status?pack=book",
        "/api/zim/index-start",
        "/api/zim/index-control",
    ] {
        assert_eq!(
            local
                .request(Method::GET, route)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            local
                .request(Method::POST, route)
                .json(&json!({"pack":"book"}))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    reopened.shutdown().unwrap();
}
