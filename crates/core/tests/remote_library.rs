use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc};
#[path = "remote_library/copy_owner.rs"]
mod copy_owner;
use tonalli_core::{
    pack::{PackSource, PreadSource},
    remote::{RemoteError, RemoteHost, RemoteLibrary},
    server::{LoopbackServer, PackId, PackRegistry, Server, ServerCfg, ServerHandle},
    zim::Zim,
};

struct Host {
    server: ServerHandle,
    _root: tempfile::TempDir,
}
impl Host {
    fn new() -> Self {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
        Self::with_source(Arc::new(PreadSource::open(&path).unwrap()))
    }
    /// A host whose shared book also has a prepared article-search index.
    fn indexed() -> Self {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
        Self::build(Arc::new(PreadSource::open(&path).unwrap()), true)
    }
    fn with_source(source: Arc<dyn PackSource>) -> Self {
        Self::build(source, false)
    }
    fn build(source: Arc<dyn PackSource>, indexed: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        fs::create_dir(&web).unwrap();
        fs::write(
            web.join("index.html"),
            "<!doctype html><title>Owned remote fixture</title>",
        )
        .unwrap();
        let registry = Arc::new(PackRegistry::default());
        for name in ["visible", "hidden"] {
            registry
                .register_zim(
                    PackId::new(name).unwrap(),
                    Arc::new(Zim::open(Arc::clone(&source)).unwrap()),
                )
                .unwrap();
        }
        if indexed {
            let archive = Zim::open(Arc::clone(&source)).unwrap();
            let search = root.path().join("search");
            tonalli_core::index::build(&archive, &search, 1).unwrap();
            registry
                .attach_index(
                    PackId::new("visible").unwrap(),
                    Arc::new(tonalli_core::index::Index::open(&search, &archive).unwrap()),
                )
                .unwrap();
        }
        let server = Server::start(
            ServerCfg {
                web_root: web,
                identity_dir: root.path().join("identity"),
                packs: vec![PackId::new("visible").unwrap()],
                ..ServerCfg::default()
            },
            registry,
        )
        .unwrap();
        Self {
            server,
            _root: root,
        }
    }
    fn invitation(&self) -> RemoteHost {
        RemoteHost::new(self.server.local_addr(), self.server.certificate_der()).unwrap()
    }
}

#[tokio::test]
async fn native_client_pairs_browses_resolves_and_obeys_host_revocation() {
    let host = Host::new();
    let client = RemoteLibrary::pair(
        host.invitation(),
        &host.server.pairing_code().unwrap(),
        "Owned native reader",
    )
    .await
    .unwrap();
    let packs = client.packs().await.unwrap();
    assert_eq!(packs["packs"].as_array().unwrap().len(), 1);
    assert_eq!(packs["packs"][0]["id"], "visible");
    let titles = client.titles("visible", 0, 2).await.unwrap();
    assert_eq!(titles["pack"], "visible");
    assert_eq!(titles["rows"].as_array().unwrap().len(), 2);
    let article = client.article("visible", "C/en/index.html").await.unwrap();
    assert_eq!(article["url"], "C/en/index.html");
    assert_eq!(article["iframe_url"], "/zim/visible/C/en/index.html");
    let content = client.content("visible", "C/en/index.html").await.unwrap();
    assert!(content.mime_type.starts_with("text/html"));
    assert_eq!(
        format!("{:x}", Sha256::digest(&content.bytes)),
        "1fc347b9b46d6ae93a85ecbef650b292e6a260c9bf07ff60a84d3cddd0bd52fc"
    );
    assert_eq!(
        client.titles("../notes", 0, 1).await.unwrap_err(),
        RemoteError::InvalidInput
    );
    assert_eq!(
        client.article("visible", "../api/notes").await.unwrap_err(),
        RemoteError::InvalidInput
    );
    assert_eq!(
        client.titles("hidden", 0, 1).await.unwrap_err(),
        RemoteError::Rejected(404)
    );
    let devices = host.server.devices().unwrap();
    assert_eq!(devices.len(), 1);
    host.server.kick(&devices[0].id).unwrap();
    assert_eq!(
        client.packs().await.unwrap_err(),
        RemoteError::Rejected(401)
    );
    client.close();
    assert_eq!(client.packs().await.unwrap_err(), RemoteError::Closed);
}

#[tokio::test]
async fn selected_copy_handoff_survives_reader_close_and_obeys_host_revocation() {
    use tonalli_core::{
        download::DownloadAction,
        remote::{CopyError, CopyOutcome},
        sync::SyncStore,
    };
    let host = Host::new();
    let invitation = host.invitation();
    let code = host.server.pairing_code().unwrap();
    let reader = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(RemoteLibrary::pair(invitation, &code, "Owned copy handoff"))
    })
    .await
    .unwrap()
    .unwrap(); // Pairing/view runtime is already gone.
    assert!(matches!(
        reader.copy_session("../notes"),
        Err(RemoteError::InvalidInput)
    ));
    let copy = reader.copy_session("visible").unwrap();
    let hidden = reader.copy_session("hidden").unwrap();
    let revoked = reader.copy_session("visible").unwrap();
    reader.close();
    assert_eq!(reader.packs().await.unwrap_err(), RemoteError::Closed);
    assert!(matches!(
        reader.copy_session("visible"),
        Err(RemoteError::Closed)
    ));
    let destination = tempfile::tempdir().unwrap();
    let target = destination.path().to_path_buf();
    // A real, separate runtime owns the transfer after the view's connection closes.
    let outcome = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(copy.copy_pack(target, || DownloadAction::Continue, |_| Ok(())))
    })
    .await
    .unwrap()
    .expect("reader close must not cancel the accepted copy");
    let CopyOutcome::Complete(row) = outcome else {
        panic!("copy did not complete")
    };
    let store = SyncStore::open(destination.path()).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(row.id, "visible");
    assert!(!destination.path().join("notes.db").exists());
    assert!(matches!(
        hidden
            .copy_pack(
                destination.path().to_path_buf(),
                || DownloadAction::Continue,
                |_| Ok(())
            )
            .await,
        Err(CopyError::Remote(RemoteError::Rejected(404)))
    ));
    let devices = host.server.devices().unwrap();
    assert_eq!(devices.len(), 1);
    host.server.kick(&devices[0].id).unwrap();
    assert!(matches!(
        revoked
            .copy_pack(
                destination.path().to_path_buf(),
                || DownloadAction::Continue,
                |_| Ok(())
            )
            .await,
        Err(CopyError::Remote(RemoteError::Rejected(401)))
    ));
    assert_eq!(
        store.list().unwrap().len(),
        1,
        "revocation must preserve the verified copy"
    );
}

#[tokio::test]
async fn native_connection_uses_existing_reader_routes_and_keeps_local_state_private() {
    let host = Host::new();
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(
        web.join("index.html"),
        "<!doctype html><title>Owned native reader</title>",
    )
    .unwrap();
    let mut app = LoopbackServer::start(
        Arc::new(PackRegistry::default()),
        root.path().join("private"),
        web,
    )
    .unwrap();
    let info = app.info();
    let origin = format!("http://127.0.0.1:{}", info.port);
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let launch = http.get(info.launch_url).send().await.unwrap();
    assert_eq!(launch.status(), 302);
    let cookie = launch
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .map(|h| h.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
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
    let csrf = session["csrf"].as_str().unwrap();
    let request = |method: reqwest::Method, path: &str| {
        http.request(method, format!("{origin}{path}"))
            .header("cookie", &cookie)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", csrf)
    };
    let certificate = host
        .server
        .certificate_der()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let invitation = format!(
        "tonalli://library?address={}&certificate={certificate}",
        host.server.local_addr()
    );
    let connect=request(reqwest::Method::POST,"/api/remote/connect").json(&serde_json::json!({"invitation":invitation,"code":host.server.pairing_code().unwrap(),"label":"Owned app client"})).send().await.unwrap();
    assert_eq!(connect.status(), 200);
    let session: serde_json::Value = request(reqwest::Method::GET, "/api/session")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["private_notes"], false);
    assert_eq!(session["private_remote"], true);
    let packs: serde_json::Value = request(reqwest::Method::GET, "/api/packs")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs["packs"][0]["id"], "visible");
    let article = request(reqwest::Method::GET, "/zim/visible/C/en/index.html")
        .send()
        .await
        .unwrap();
    assert_eq!(article.status(), 200);
    assert!(article.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("script-src 'none'"));
    assert_eq!(
        format!("{:x}", Sha256::digest(article.bytes().await.unwrap())),
        "1fc347b9b46d6ae93a85ecbef650b292e6a260c9bf07ff60a84d3cddd0bd52fc"
    );
    for path in [
        "/api/notes",
        "/api/files",
        "/api/copies",
        "/api/preferences",
        "/api/sharing",
        "/api/remote/connect/extra",
    ] {
        assert_eq!(
            request(reqwest::Method::GET, path)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let copy = request(reqwest::Method::POST, "/api/remote/copy")
        .json(&serde_json::json!({"id":"visible"}))
        .send()
        .await
        .unwrap();
    assert_eq!(copy.status(), 202);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let copied = loop {
        let status: serde_json::Value = request(reqwest::Method::GET, "/api/remote")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if status["copy"]["state"] == "complete" {
            break status["copy"]["local_id"].as_str().unwrap().to_owned();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "copy did not complete: {status}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let disconnect = request(reqwest::Method::POST, "/api/remote/disconnect")
        .send()
        .await
        .unwrap();
    assert_eq!(disconnect.status(), 200);
    let session: serde_json::Value = request(reqwest::Method::GET, "/api/session")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["private_notes"], true);
    let packs: serde_json::Value = request(reqwest::Method::GET, "/api/packs")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs["packs"][0]["id"], copied);
    assert!(!root.path().join("private/notes.db").exists());
    app.shutdown().unwrap();
}

/// Connects a fresh private app to `host`, saves "visible" and returns its final copy state.
async fn copy_through_app(host: &Host, body: serde_json::Value) -> (serde_json::Value, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("index.html"), "<!doctype html><title>Copy app</title>").unwrap();
    let mut app = LoopbackServer::start(
        Arc::new(PackRegistry::default()),
        root.path().join("private"),
        web,
    )
    .unwrap();
    let info = app.info();
    let origin = format!("http://127.0.0.1:{}", info.port);
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let launch = http.get(info.launch_url).send().await.unwrap();
    let cookie = launch
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .map(|h| h.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
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
    let csrf = session["csrf"].as_str().unwrap().to_owned();
    let request = |method: reqwest::Method, path: &str| {
        http.request(method, format!("{origin}{path}"))
            .header("cookie", &cookie)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", &csrf)
    };
    let certificate = host
        .server
        .certificate_der()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let invitation = format!(
        "tonalli://library?address={}&certificate={certificate}",
        host.server.local_addr()
    );
    let connect = request(reqwest::Method::POST, "/api/remote/connect")
        .json(&serde_json::json!({"invitation":invitation,"code":host.server.pairing_code().unwrap(),"label":"Search copy app"}))
        .send()
        .await
        .unwrap();
    assert_eq!(connect.status(), 200);
    let copy = request(reqwest::Method::POST, "/api/remote/copy")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(copy.status(), 202);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let finished = loop {
        let status: serde_json::Value = request(reqwest::Method::GET, "/api/remote")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if matches!(status["copy"]["state"].as_str(), Some("complete" | "failed")) {
            break status["copy"].clone();
        }
        assert!(std::time::Instant::now() < deadline, "copy did not finish: {status}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    app.shutdown().unwrap();
    (finished, root)
}

#[tokio::test]
async fn native_copy_can_also_bring_the_hosts_prepared_article_search() {
    use tonalli_core::{
        index::jobs::{IndexJobs, IndexSource},
        sync::SyncStore,
    };
    let host = Host::indexed();
    let (copy, root) = copy_through_app(&host, serde_json::json!({"id":"visible","search":true})).await;
    assert_eq!(copy["state"], "complete", "{copy}");
    assert_eq!(copy["search"], "copied", "{copy}");
    let private = root.path().join("private");
    let store = SyncStore::open(&private).unwrap();
    let row = store.list().unwrap().remove(0);
    let archive = Arc::new(Zim::open(store.open_source(&row).unwrap()).unwrap());
    let source = IndexSource::registered(PackId::new(row.local_id()).unwrap(), archive).unwrap();
    let ready = IndexJobs::open(&private).unwrap().safe_ready(&source).unwrap();
    assert!(!ready.expect("copied search index is ready").search("English", 5).unwrap().is_empty());

    let (book_only, root) = copy_through_app(&host, serde_json::json!({"id":"visible"})).await;
    assert_eq!(book_only["state"], "complete");
    assert!(book_only["search"].is_null(), "{book_only}");
    let store = SyncStore::open(&root.path().join("private")).unwrap();
    let row = store.list().unwrap().remove(0);
    let archive = Arc::new(Zim::open(store.open_source(&row).unwrap()).unwrap());
    let source = IndexSource::registered(PackId::new(row.local_id()).unwrap(), archive).unwrap();
    assert!(IndexJobs::open(&root.path().join("private")).unwrap().safe_ready(&source).unwrap().is_none());
}

#[tokio::test]
async fn a_host_without_prepared_search_still_delivers_the_book() {
    use tonalli_core::sync::SyncStore;
    let host = Host::new();
    let (copy, root) = copy_through_app(&host, serde_json::json!({"id":"visible","search":true})).await;
    assert_eq!(copy["state"], "complete", "{copy}");
    assert_eq!(copy["search"], "unavailable", "{copy}");
    assert_eq!(SyncStore::open(&root.path().join("private")).unwrap().list().unwrap().len(), 1);
}

#[tokio::test]
async fn native_client_does_not_pair_with_a_different_host_certificate() {
    let host = Host::new();
    let other = Host::new();
    let wrong = RemoteHost::new(host.server.local_addr(), other.server.certificate_der()).unwrap();
    assert!(
        RemoteLibrary::pair(wrong, &host.server.pairing_code().unwrap(), "Must not pair")
            .await
            .is_err()
    );
    assert!(host.server.devices().unwrap().is_empty());
    let direct = reqwest::Client::builder()
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_der(&host.server.certificate_der()).unwrap(),
        )
        .build()
        .unwrap();
    for path in [
        "/api/remote",
        "/api/remote/connect",
        "/api/remote/disconnect",
    ] {
        assert_eq!(
            direct
                .get(format!("https://{}{path}", host.server.local_addr()))
                .send()
                .await
                .unwrap()
                .status(),
            404,
            "native connection controls must be private before LAN authentication"
        );
    }
    for address in [
        "8.8.8.8:443",
        "0.0.0.0:443",
        "[::ffff:127.0.0.1]:443",
        "127.0.0.1:0",
    ] {
        assert!(RemoteHost::new(address.parse().unwrap(), host.server.certificate_der()).is_err());
    }
}
