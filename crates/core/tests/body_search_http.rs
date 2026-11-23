//! Product body-search boundary; authored tiny ZIMs only, never downloaded corpora.
use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{COOKIE, SET_COOKIE},
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;
use tonalli_core::{
    index::{self, Index},
    pack::{PackError, PackSource, PreadSource},
    server::{LoopbackServer, PackId, PackRegistry, Server, ServerCfg, ServerHandle},
    zim::Zim,
};

fn archive(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles")
        .join(name)
}
struct Source {
    inner: PreadSource,
    removed: AtomicBool,
}
impl PackSource for Source {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        self.inner.read_at(offset, buffer)
    }
}
struct Fixture {
    registry: Arc<PackRegistry>,
    zim: Arc<Zim>,
    source: Arc<Source>,
    index: Arc<Index>,
    root: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = Arc::new(Source {
            inner: PreadSource::open(&archive("benign-v1.zim")).unwrap(),
            removed: AtomicBool::new(false),
        });
        let zim = Arc::new(Zim::open(source.clone()).unwrap());
        let directory = root.path().join("index");
        index::build(&zim, &directory, 1).unwrap();
        // A distinct handle with identical archive identity ensures the HTTP
        // source-removal check probes the registered source, not only Index's lease.
        let independent = Zim::open(Arc::new(
            PreadSource::open(&archive("benign-v1.zim")).unwrap(),
        ))
        .unwrap();
        let index = Arc::new(Index::open(&directory, &independent).unwrap());
        let registry = Arc::new(PackRegistry::default());
        registry
            .register_zim(PackId::new("visible").unwrap(), zim.clone())
            .unwrap();
        registry
            .register_zim(PackId::new("hidden").unwrap(), zim.clone())
            .unwrap();
        fs::create_dir(root.path().join("web")).unwrap();
        fs::write(
            root.path().join("web/index.html"),
            "<!doctype html><title>Reader</title>",
        )
        .unwrap();
        Self {
            root,
            registry,
            zim,
            source,
            index,
        }
    }
    fn attach(&self, id: &str) {
        self.registry
            .attach_index(PackId::new(id).unwrap(), self.index.clone())
            .unwrap();
    }
    fn local(&self) -> (LoopbackServer, Client, String, Credentials) {
        let server = LoopbackServer::start(
            self.registry.clone(),
            self.root.path().join("private"),
            self.root.path().join("web"),
        )
        .unwrap();
        let client = client();
        let info = server.info();
        let origin = format!("http://127.0.0.1:{}", info.port);
        let response = client.get(&info.launch_url).send().unwrap();
        let credentials = login(&client, &origin, response);
        (server, client, origin, credentials)
    }
    fn lan(&self, shared: Vec<&str>) -> (ServerHandle, Client, String, Credentials) {
        let server = Server::start(
            ServerCfg {
                bind: "127.0.0.1".parse().unwrap(),
                packs: shared
                    .into_iter()
                    .map(|id| PackId::new(id).unwrap())
                    .collect(),
                web_root: self.root.path().join("web"),
                identity_dir: self.root.path().join("identity"),
                ..ServerCfg::default()
            },
            self.registry.clone(),
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .add_root_certificate(
                reqwest::Certificate::from_der(&server.certificate_der()).unwrap(),
            )
            .build()
            .unwrap();
        let origin = format!("https://{}", server.local_addr());
        let response = client.post(format!("{origin}/pair")).json(&serde_json::json!({"code":server.pairing_code().unwrap(),"label":"body search fixture"})).send().unwrap();
        assert_eq!(response.status(), 200);
        let credentials = login(&client, &origin, response);
        (server, client, origin, credentials)
    }
}
fn client() -> Client {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
struct Credentials {
    cookies: String,
    content: String,
    csrf: String,
}
fn login(client: &Client, origin: &str, response: reqwest::blocking::Response) -> Credentials {
    let cookies: Vec<_> = response
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
    let content = cookies
        .iter()
        .find(|cookie| cookie.contains("content="))
        .unwrap()
        .clone();
    let cookies = cookies.join("; ");
    let session: Value = client
        .get(format!("{origin}/api/session"))
        .header(COOKIE, &cookies)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .unwrap()
        .json()
        .unwrap();
    Credentials {
        cookies,
        content,
        csrf: session["csrf"].as_str().unwrap().to_owned(),
    }
}
fn api(
    client: &Client,
    origin: &str,
    credentials: &Credentials,
    pack: &str,
    query: &str,
    limit: usize,
) -> RequestBuilder {
    client
        .get(format!("{origin}/api/zim/search"))
        .query(&[("pack", pack), ("q", query), ("limit", &limit.to_string())])
        .header(COOKIE, &credentials.cookies)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", &credentials.csrf)
}
fn assert_phrase(response: reqwest::blocking::Response) {
    assert_eq!(response.status(), 200);
    let body: Value = response.json().unwrap();
    assert_eq!(body["pack"], "visible");
    let hits = body["hits"].as_array().unwrap();
    assert!(hits.iter().any(|hit| hit["url"] == "C/en/index.html"));
    assert!(hits.len() <= 20);
    for hit in hits {
        assert!(hit["score"].as_f64().unwrap().is_finite());
        assert_eq!(hit.as_object().unwrap().len(), 3);
        assert!(
            !hit["title"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("stylesheet"),
            "phrase must exercise body, not title"
        );
    }
    let text = body.to_string();
    assert!(
        !text.contains("file:") && !text.contains("generation-") && !text.contains("index_meta")
    );
}

#[test]
fn authenticated_installed_index_search_finds_body_phrase_with_canonical_article_links() {
    let f = Fixture::new();
    f.attach("visible");
    let (mut server, client, origin, credentials) = f.local();
    assert_phrase(
        api(
            &client,
            &origin,
            &credentials,
            "visible",
            "\"External stylesheet\"",
            20,
        )
        .send()
        .unwrap(),
    );
    let response: Value = api(&client, &origin, &credentials, "visible", "applies", 1)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(response["hits"].as_array().unwrap().len() <= 1);
    server.shutdown().unwrap();
}

#[test]
fn wrong_index_identity_does_not_replace_a_working_attachment() {
    let f = Fixture::new();
    f.attach("visible");
    let other = Zim::open(Arc::new(
        PreadSource::open(&archive("hostile-v1.zim")).unwrap(),
    ))
    .unwrap();
    let directory = f.root.path().join("wrong-index");
    index::build(&other, &directory, 1).unwrap();
    let wrong = Arc::new(Index::open(&directory, &other).unwrap());
    assert!(matches!(
        f.registry
            .attach_index(PackId::new("visible").unwrap(), wrong),
        Err(PackError::Mismatch(_))
    ));
    let (mut server, client, origin, c) = f.local();
    assert_phrase(
        api(
            &client,
            &origin,
            &c,
            "visible",
            "\"External stylesheet\"",
            20,
        )
        .send()
        .unwrap(),
    );
    server.shutdown().unwrap();
}

#[test]
fn source_and_registry_removal_prevent_stale_index_results_or_id_reuse() {
    let f = Fixture::new();
    f.attach("visible");
    let (mut server, client, origin, c) = f.local();
    f.source.removed.store(true, Ordering::SeqCst);
    let removed = api(&client, &origin, &c, "visible", "stylesheet", 20)
        .send()
        .unwrap();
    assert_eq!(removed.status(), 503);
    assert_eq!(removed.json::<Value>().unwrap()["error"], "media_removed");
    server.shutdown().unwrap();
    let f = Fixture::new();
    f.attach("visible");
    let (mut server, client, origin, c) = f.local();
    f.registry.remove(&PackId::new("visible").unwrap()).unwrap();
    assert_eq!(
        api(&client, &origin, &c, "visible", "stylesheet", 20)
            .send()
            .unwrap()
            .status(),
        404
    );
    f.registry
        .register_zim(PackId::new("visible").unwrap(), f.zim.clone())
        .unwrap();
    let unavailable = api(&client, &origin, &c, "visible", "stylesheet", 20)
        .send()
        .unwrap();
    assert_eq!(unavailable.status(), 503);
    assert_eq!(
        unavailable.json::<Value>().unwrap()["error"],
        "index_unavailable"
    );
    server.shutdown().unwrap();
}

#[test]
fn search_guards_and_bounds_precede_content_and_unavailable_is_not_empty_success() {
    let f = Fixture::new();
    let (mut server, client, origin, c) = f.local();
    assert_eq!(
        client
            .get(format!("{origin}/api/zim/search?pack=visible&q=x"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/zim/search?pack=visible&q=x"))
            .header(COOKIE, &c.content)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &c.csrf)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/zim/search?pack=visible&q=x"))
            .header(COOKIE, &c.cookies)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        403
    );
    let unavailable = api(&client, &origin, &c, "visible", "x", 20)
        .send()
        .unwrap();
    assert_eq!(unavailable.status(), 503);
    assert_eq!(
        unavailable.json::<Value>().unwrap()["error"],
        "index_unavailable"
    );
    f.attach("visible");
    for (query, limit) in [
        ("x".repeat(4097), 20),
        ("\"unfinished".to_owned(), 20),
        ("x".to_owned(), 1001),
    ] {
        assert_eq!(
            api(&client, &origin, &c, "visible", &query, limit)
                .send()
                .unwrap()
                .status(),
            400
        );
    }
    assert_eq!(
        api(&client, &origin, &c, "../private/index", "x", 20)
            .send()
            .unwrap()
            .status(),
        400
    );
    server.shutdown().unwrap();
}

#[test]
fn lan_body_search_is_authenticated_and_limited_to_explicit_installed_allowlist() {
    let f = Fixture::new();
    f.attach("visible");
    f.attach("hidden");
    let (mut server, client, origin, c) = f.lan(vec!["visible"]);
    assert_eq!(
        client
            .get(format!("{origin}/api/zim/search?pack=visible&q=x"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_phrase(
        api(
            &client,
            &origin,
            &c,
            "visible",
            "\"External stylesheet\"",
            20,
        )
        .send()
        .unwrap(),
    );
    assert_eq!(
        api(&client, &origin, &c, "hidden", "stylesheet", 20)
            .send()
            .unwrap()
            .status(),
        404
    );
    server.shutdown().unwrap();
    let (mut server, client, origin, c) = f.lan(vec![]);
    assert_eq!(
        api(&client, &origin, &c, "visible", "stylesheet", 20)
            .send()
            .unwrap()
            .status(),
        404
    );
    server.shutdown().unwrap();
}
