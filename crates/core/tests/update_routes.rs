use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
    Method,
};
use serde_json::{json, Value};
use std::{fs, sync::Arc};
use tonalli_core::server::{LoopbackServer, PackRegistry};

struct App {
    server: LoopbackServer,
    client: Client,
    cookie: String,
    csrf: String,
}

impl App {
    fn open(root: &std::path::Path) -> Self {
        let web = root.join("web");
        fs::create_dir_all(&web).unwrap();
        fs::write(web.join("index.html"), "fixture").unwrap();
        let server =
            LoopbackServer::start(Arc::new(PackRegistry::default()), root.join("data"), web).unwrap();
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let launch = client.get(server.info().launch_url).send().unwrap();
        let cookie = launch
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|x| x.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let base = format!("http://127.0.0.1:{}", server.info().port);
        let session: Value = client
            .get(format!("{base}/api/session"))
            .header(COOKIE, &cookie)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .json()
            .unwrap();
        Self { server, client, cookie, csrf: session["csrf"].as_str().unwrap().into() }
    }

    fn request(&self, method: Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, format!("http://127.0.0.1:{}{path}", self.server.info().port))
            .header(COOKIE, &self.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &self.csrf)
    }
}

const KEY: &str = "0102030405060708091011121314151617181920212223242526272829303132";

#[test]
fn updates_start_off_with_nothing_configured_and_nothing_available() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let status: Value = app.request(Method::GET, "/api/update").send().unwrap().json().unwrap();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["source"], "");
    assert_eq!(status["check"], "never");
    assert_eq!(status["available"], Value::Null);
    assert_eq!(status["download"]["state"], "idle");
    assert!(status["current"].as_str().unwrap().split('.').count() >= 3);
    assert_eq!(status["supported"], cfg!(any(windows, target_os = "android", target_os = "ios")));
}

#[test]
fn update_routes_require_the_app_session() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let base = format!("http://127.0.0.1:{}", app.server.info().port);
    assert_eq!(app.client.get(format!("{base}/api/update")).send().unwrap().status(), 401);
    let no_csrf = app
        .client
        .put(format!("{base}/api/update/settings"))
        .header(COOKIE, &app.cookie)
        .header("Sec-Fetch-Dest", "empty")
        .json(&json!({"enabled":true,"source":"example/tonalli","public_key":KEY}))
        .send()
        .unwrap();
    assert!(no_csrf.status() == 403 || no_csrf.status() == 401);
    assert!(!root.path().join("data/packs/private/update.json").exists());
}

#[test]
fn saving_settings_validates_normalizes_and_persists() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    for bad in [
        json!({"enabled":true,"source":"http://updates.example.org/","public_key":KEY}),
        json!({"enabled":true,"source":"example/tonalli","public_key":"nope"}),
        json!({"enabled":true,"source":"","public_key":KEY}),
        json!({"enabled":true,"source":"https://localhost/x","public_key":KEY}),
    ] {
        let reply = app.request(Method::PUT, "/api/update/settings").json(&bad).send().unwrap();
        assert_eq!(reply.status(), 400, "{bad}");
        assert_eq!(reply.json::<Value>().unwrap()["error"], "invalid_update_settings");
    }
    assert!(!root.path().join("data/packs/private/update.json").exists());

    let reply = app
        .request(Method::PUT, "/api/update/settings")
        .json(&json!({"enabled":true,"source":" example/tonalli ","public_key":KEY.to_uppercase()}))
        .send()
        .unwrap();
    assert_eq!(reply.status(), 200);
    let status: Value = reply.json().unwrap();
    assert_eq!(status["enabled"], true);
    assert_eq!(status["source"], "https://github.com/example/tonalli/releases/download/update-feed/");
    assert_eq!(status["public_key"], KEY);
    let again: Value = app.request(Method::GET, "/api/update").send().unwrap().json().unwrap();
    assert_eq!(again["enabled"], true);
    assert!(root.path().join("data/packs/private/update.json").is_file());

    let unknown = app
        .request(Method::PUT, "/api/update/settings")
        .json(&json!({"enabled":true,"source":"example/tonalli","public_key":KEY,"command":"x"}))
        .send()
        .unwrap();
    assert!(unknown.status().is_client_error());
    let unchanged: Value = app.request(Method::GET, "/api/update").send().unwrap().json().unwrap();
    assert_eq!(unchanged["source"], "https://github.com/example/tonalli/releases/download/update-feed/");
}

#[test]
fn checking_or_downloading_without_permission_or_an_offer_does_nothing() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let check = app.request(Method::POST, "/api/update/check").send().unwrap();
    assert_eq!(check.status(), 409);
    assert_eq!(check.json::<Value>().unwrap()["error"], "updates_off");

    app.request(Method::PUT, "/api/update/settings")
        .json(&json!({"enabled":true,"source":"example/tonalli","public_key":KEY}))
        .send()
        .unwrap();
    let download = app.request(Method::POST, "/api/update/download").send().unwrap();
    assert_eq!(download.status(), 409);
    assert_eq!(download.json::<Value>().unwrap()["error"], "no_update");
    assert!(!root.path().join("data/packs/updates").exists());
}
