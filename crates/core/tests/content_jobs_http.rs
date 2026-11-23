use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{COOKIE, SET_COOKIE},
    Method,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tonalli_core::{
    catalog::Catalog,
    content::{ContentJobs, JobAction, JobState, JobTicket, ManagedStore},
    download::pack_path,
    server::{ContentDispatchError, ContentDispatcher, LoopbackServer, PackRegistry},
};

#[derive(Clone, Copy)]
enum Dispatch {
    Accept,
    Reject,
    Uncertain,
}
struct Native {
    mode: Dispatch,
    calls: Mutex<Vec<(Arc<ContentJobs>, JobTicket)>>,
    reenter: Mutex<Option<(Client, String, String, String)>>,
    controls: Mutex<Vec<(String, u64, JobAction)>>,
}
impl ContentDispatcher for Native {
    fn control(
        &self,
        _jobs: Arc<ContentJobs>,
        ticket: JobTicket,
        action: JobAction,
    ) -> Result<(), ContentDispatchError> {
        self.controls
            .lock()
            .unwrap()
            .push((ticket.id, ticket.epoch, action));
        Ok(())
    }
    fn dispatch(
        &self,
        jobs: Arc<ContentJobs>,
        ticket: JobTicket,
    ) -> Result<(), ContentDispatchError> {
        if let Some((client, url, api, csrf)) = self.reenter.lock().unwrap().as_ref() {
            assert_eq!(
                client
                    .get(url)
                    .header(COOKIE, api)
                    .header("sec-fetch-dest", "empty")
                    .header("x-tonalli-csrf", csrf)
                    .send()
                    .unwrap()
                    .status(),
                200
            );
        }
        // This represents native admission only; HTTP never runs the downloader for us.
        self.calls.lock().unwrap().push((jobs, ticket));
        match self.mode {
            Dispatch::Accept => Ok(()),
            Dispatch::Reject => Err(ContentDispatchError::Rejected),
            Dispatch::Uncertain => Err(ContentDispatchError::Uncertain),
        }
    }
}
struct Fixture {
    root: tempfile::TempDir,
    server: LoopbackServer,
    store: Arc<ManagedStore>,
    catalog: Arc<Catalog>,
    client: Client,
    api: String,
    content: String,
    csrf: String,
    base: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        fs::create_dir(&web).unwrap();
        fs::write(web.join("index.html"), "<title>Content test</title>").unwrap();
        let data = root.path().join("private");
        let key = Ed25519KeyPair::from_seed_unchecked(&[71; 32]).unwrap();
        let bytes = fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
        )
        .unwrap();
        let payload=serde_json::to_vec(&json!({"schema_version":1,"revision":1,"entries":[{
            "id":"test-en","title":"Synthetic archive","kind":"zim","language":"en","version":"r1",
            "bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),"url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
        }]})).unwrap();
        let catalog = Arc::new(
            Catalog::verify(
                &payload,
                key.sign(&payload).as_ref(),
                key.public_key().as_ref(),
                0,
            )
            .unwrap(),
        );
        let server =
            LoopbackServer::start(Arc::new(PackRegistry::default()), data.clone(), web).unwrap();
        let store = Arc::new(ManagedStore::open(&data, key.public_key().as_ref()).unwrap());
        let cache = pack_path(&data, &store.cache_spec(&catalog, "test-en").unwrap().path).unwrap();
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(cache, bytes).unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{}", server.info().port);
        let launch = client.get(server.info().launch_url).send().unwrap();
        let cookies = launch
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
            .collect::<Vec<_>>();
        let api = cookies
            .iter()
            .find(|c| c.starts_with("tonalli_api="))
            .unwrap()
            .clone();
        let content = cookies
            .iter()
            .find(|c| c.starts_with("tonalli_content="))
            .unwrap()
            .clone();
        let session: Value = client
            .get(format!("{base}/api/session"))
            .header(COOKIE, &api)
            .header("sec-fetch-dest", "empty")
            .send()
            .unwrap()
            .json()
            .unwrap();
        let csrf = session["csrf"].as_str().unwrap().to_owned();
        Self {
            root,
            server,
            store,
            catalog,
            client,
            api,
            content,
            csrf,
            base,
        }
    }
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .header(COOKIE, &self.api)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", &self.csrf)
    }
    fn attach(&self, mode: Dispatch) -> Arc<Native> {
        let native = Arc::new(Native {
            mode,
            calls: Mutex::new(Vec::new()),
            reenter: Mutex::new(None),
            controls: Mutex::new(Vec::new()),
        });
        self.server
            .attach_content_jobs(
                Arc::clone(&self.store),
                Arc::clone(&self.catalog),
                native.clone(),
            )
            .unwrap();
        native
    }
    fn start(&self) -> reqwest::blocking::Response {
        self.request(Method::POST, "/api/content/jobs")
            .json(&json!({"entryId":"test-en","catalogRevision":1}))
            .send()
            .unwrap()
    }
}

#[test]
fn durable_offline_queued_stops_notify_native_without_redispatching_network_work() {
    let f = Fixture::new();
    let native = f.attach(Dispatch::Accept);
    let started: Value = f.start().json().unwrap();
    let id = started["job"]["id"].as_str().unwrap();
    for (action, expected) in [("pause", "pausing"), ("cancel", "cancelling")] {
        let reply: Value = f
            .request(Method::POST, &format!("/api/content/jobs/{id}/{action}"))
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(reply["job"]["state"], expected);
    }
    assert_eq!(
        native.calls.lock().unwrap().len(),
        1,
        "stop notifications cannot create a fresh download dispatch"
    );
    let controls = native.controls.lock().unwrap();
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].0, id);
    assert_eq!(controls[0].1, controls[1].1);
    assert_eq!(controls[0].2, JobAction::Pause);
    assert_eq!(controls[1].2, JobAction::Cancel);
}

#[test]
fn remove_refuses_the_same_installed_id_while_queued_or_paused_job_owns_it() {
    let f = Fixture::new();
    f.store.install_cached(&f.catalog, "test-en").unwrap();
    let before = f.store.list().unwrap();
    let native = f.attach(Dispatch::Accept);
    let started: Value = f.start().json().unwrap();
    let id = started["job"]["id"].as_str().unwrap();
    let refused: Value = f
        .request(Method::DELETE, "/api/content/installed/test-en")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(refused, json!({"removed":false,"reason":"in_use"}));
    f.request(Method::POST, &format!("/api/content/jobs/{id}/pause"))
        .send()
        .unwrap();
    let (jobs, ticket) = native.calls.lock().unwrap().pop().unwrap();
    let mut run = jobs.claim(&ticket).unwrap();
    jobs.execute(&mut run, |_| Ok(())).unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let refused: Value = f
        .request(Method::DELETE, "/api/content/installed/test-en")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(refused, json!({"removed":false,"reason":"in_use"}));
    assert_eq!(f.store.list().unwrap(), before);
}

#[test]
fn native_dispatch_can_reenter_private_catalog_without_an_attachment_lock_deadlock() {
    let f = Fixture::new();
    let native = f.attach(Dispatch::Accept);
    *native.reenter.lock().unwrap() = Some((
        f.client.clone(),
        format!("{}/api/content/catalog", f.base),
        f.api.clone(),
        f.csrf.clone(),
    ));
    assert_eq!(f.start().status(), 200);
    assert_eq!(native.calls.lock().unwrap().len(), 1);
}

#[test]
fn writes_and_jobs_stay_unavailable_without_explicit_native_dispatcher() {
    let f = Fixture::new();
    f.server
        .attach_content(Arc::clone(&f.store), Arc::clone(&f.catalog))
        .unwrap();
    let catalog: Value = f
        .request(Method::GET, "/api/content/catalog")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(catalog["downloadAvailable"], false);
    for (method, path) in [
        (Method::GET, "/api/content/jobs"),
        (Method::POST, "/api/content/jobs"),
        (Method::POST, "/api/content/preflight"),
        (Method::DELETE, "/api/content/installed/test-en"),
    ] {
        assert_eq!(
            f.request(method, path)
                .json(&json!({"entryId":"test-en","catalogRevision":1}))
                .send()
                .unwrap()
                .status(),
            503
        );
    }
}

#[test]
fn native_tickets_are_only_admitted_by_http_then_real_core_execution_is_observed() {
    let f = Fixture::new();
    let native = f.attach(Dispatch::Accept);
    let catalog: Value = f
        .request(Method::GET, "/api/content/catalog")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(catalog["downloadAvailable"], true);
    let response = f.start();
    assert_eq!(response.status(), 200);
    let job: Value = response.json().unwrap();
    assert_eq!(job["job"]["state"], "queued");
    assert!(
        f.store.list().unwrap().is_empty(),
        "HTTP must not execute the worker"
    );
    assert_eq!(f.start().status(), 409);
    let (jobs, ticket) = native.calls.lock().unwrap().pop().unwrap();
    assert_eq!(ticket.id, job["job"]["id"]);
    let mut run = jobs.claim(&ticket).unwrap();
    jobs.execute(&mut run, |_| Ok(())).unwrap();
    assert_eq!(
        jobs.finish_exited(&mut run).unwrap().state,
        JobState::Completed
    );
    let listed: Value = f
        .request(Method::GET, "/api/content/jobs")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(listed["jobs"][0]["state"], "completed");
    assert!(listed["jobs"][0].get("ticket").is_none() && listed["jobs"][0].get("path").is_none());
    let id = job["job"]["id"].as_str().unwrap();
    assert_eq!(
        f.request(Method::POST, &format!("/api/content/jobs/{id}/continue"))
            .send()
            .unwrap()
            .status(),
        400
    );
    let lease = f.store.open_source("test-en").unwrap();
    let refused: Value = f
        .request(Method::DELETE, "/api/content/installed/test-en")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(refused, json!({"removed":false,"reason":"in_use"}));
    drop(lease);
    let removed: Value = f
        .request(Method::DELETE, "/api/content/installed/test-en")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(removed, json!({"removed":true,"reason":null}));
}

#[test]
fn preflight_is_conservative_local_budget_and_native_controls_confirm_real_stops() {
    let f = Fixture::new();
    let native = f.attach(Dispatch::Accept);
    let budget: Value = f
        .request(Method::POST, "/api/content/preflight")
        .json(&json!({"entryId":"test-en","catalogRevision":1}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(budget["entryId"], "test-en");
    assert_eq!(budget["downloadBytes"], f.catalog.entries()[0].bytes);
    assert_eq!(budget["stagingBytes"], 0);
    assert_eq!(
        budget["requiredBytes"],
        f.catalog.entries()[0].bytes + 1024 * 1024
    );
    assert_eq!(
        budget["allowed"],
        budget["availableBytes"].as_u64().unwrap() >= budget["requiredBytes"].as_u64().unwrap()
    );
    assert!(budget.get("path").is_none() && budget.get("url").is_none());
    let started: Value = f.start().json().unwrap();
    let id = started["job"]["id"].as_str().unwrap();
    let pause: Value = f
        .request(Method::POST, &format!("/api/content/jobs/{id}/pause"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(pause["job"]["state"], "pausing");
    assert_eq!(native.calls.lock().unwrap().len(), 1);
    let (jobs, ticket) = native.calls.lock().unwrap().pop().unwrap();
    let mut run = jobs.claim(&ticket).unwrap();
    jobs.execute(&mut run, |_| Ok(())).unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let resumed: Value = f
        .request(Method::POST, &format!("/api/content/jobs/{id}/resume"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(resumed["job"]["state"], "queued");
    let cancel: Value = f
        .request(Method::POST, &format!("/api/content/jobs/{id}/cancel"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(cancel["job"]["state"], "cancelling");
    let (jobs, ticket) = native.calls.lock().unwrap().pop().unwrap();
    let mut cleanup = jobs.claim(&ticket).unwrap();
    jobs.execute(&mut cleanup, |_| Ok(())).unwrap();
    assert_eq!(
        jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn definite_schedule_rejection_is_recoverable_but_uncertain_admission_never_redispatches() {
    for (mode, expected) in [
        (Dispatch::Reject, "paused"),
        (Dispatch::Uncertain, "queued"),
    ] {
        let f = Fixture::new();
        let native = f.attach(mode);
        let response = f.start();
        assert_eq!(response.status(), 503);
        let error: Value = response.json().unwrap();
        assert_eq!(
            error["error"],
            if expected == "paused" {
                "download_unavailable"
            } else {
                "request_unconfirmed"
            }
        );
        let listed: Value = f
            .request(Method::GET, "/api/content/jobs")
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(listed["jobs"][0]["state"], expected);
        if expected == "queued" {
            assert_eq!(f.start().status(), 409);
        }
        assert_eq!(native.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn native_jobs_routes_reject_unsigned_authority_and_wrong_session_context() {
    let f = Fixture::new();
    let native = f.attach(Dispatch::Accept);
    assert_eq!(f.request(Method::POST,"/api/content/jobs").json(&json!({"entryId":"test-en","catalogRevision":1,"url":"https://foreign.invalid/file"})).send().unwrap().status(),400);
    assert_eq!(
        f.request(Method::POST, "/api/content/jobs")
            .json(&json!({"entryId":"test-en","catalogRevision":2}))
            .send()
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        f.request(Method::POST, "/api/content/jobs")
            .json(&json!({"entryId":"missing","catalogRevision":1}))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        f.client
            .get(format!("{}/api/content/jobs", f.base))
            .header(COOKIE, &f.content)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        f.request(Method::GET, "/api/content/jobs")
            .header("origin", "https://foreign.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert!(native.calls.lock().unwrap().is_empty());
    assert!(!f.root.path().join("private/notes.db").exists());
}
