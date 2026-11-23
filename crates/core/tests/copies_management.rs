use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc};
use tonalli_core::{
    catalog::CatalogKind,
    server::{LoopbackServer, PackRegistry},
    sync::{PackManifest, SyncStore},
};

#[tokio::test]
async fn private_copy_management_requires_admin_preserves_readers_and_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("index.html"), "Owned copy-management fixture").unwrap();
    let data = root.path().join("private");
    let store = SyncStore::open(&data).unwrap();
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap();
    let manifest = PackManifest {
        version: 1,
        id: "shared-book".into(),
        title: "Owned nearby book".into(),
        kind: CatalogKind::Zim,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    fs::write(data.join(store.cache_spec(&manifest).unwrap().path), &bytes).unwrap();
    store.install_cached(&manifest).unwrap();
    let registry = Arc::new(PackRegistry::default());
    let mut app = LoopbackServer::start(registry, data.clone(), web.clone()).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let launch = client.get(&app.info().launch_url).send().await.unwrap();
    let cookie = launch
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let origin = format!("http://127.0.0.1:{}", app.info().port);
    let session: serde_json::Value = client
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
    let request = |method, path: &str| {
        client
            .request(method, format!("{origin}{path}"))
            .header("cookie", &cookie)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", csrf)
    };
    assert_eq!(
        client
            .get(format!("{origin}/api/copies"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/copies"))
            .header("cookie", &cookie)
            .header("sec-fetch-dest", "empty")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let listing = request(reqwest::Method::GET, "/api/copies")
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status(), 200);
    let listing: serde_json::Value = listing.json().await.unwrap();
    let row = &listing["copies"][0];
    assert_eq!(row["id"], manifest.local_id());
    assert_eq!(row["title"], manifest.title);
    assert_eq!(row["kind"], "zim");
    assert_eq!(row["bytes"], manifest.bytes);
    assert!(row.get("path").is_none());
    assert!(row.get("root").is_none());
    let route = format!("/api/copies/{}", manifest.local_id());
    assert_eq!(
        request(reqwest::Method::DELETE, &route)
            .send()
            .await
            .unwrap()
            .status(),
        423
    );
    let setup = request(reqwest::Method::POST, "/api/admin/setup")
        .json(&serde_json::json!({"passphrase":"Test-only nearby management passphrase"}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), 200);
    let setup: serde_json::Value = setup.json().await.unwrap();
    let mut token = setup["token"].as_str().unwrap().to_owned();
    assert_eq!(
        request(reqwest::Method::DELETE, "/api/copies/shared-book")
            .header("x-tonalli-admin", &token)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let reader = store.open_source(&manifest).unwrap();
    let blocked = request(reqwest::Method::DELETE, &route)
        .header("x-tonalli-admin", &token)
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 200);
    let blocked: serde_json::Value = blocked.json().await.unwrap();
    assert_eq!(blocked["removed"], false);
    assert_eq!(blocked["reason"], "in_use");
    let restored: serde_json::Value = request(reqwest::Method::GET, "/api/packs")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["packs"][0]["id"], manifest.local_id());
    drop(reader);
    // Force a real storage wait, then revoke Admin after the registry source was withdrawn.
    let gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.join("packs/nearby/installed.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&gate).unwrap();
    let pending = request(reqwest::Method::DELETE, &route).header("x-tonalli-admin", &token);
    let pending = tokio::spawn(async move { pending.send().await.unwrap() });
    let mut waiting = false;
    for _ in 0..40 {
        let packs: serde_json::Value = request(reqwest::Method::GET, "/api/packs")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if packs["packs"] == serde_json::json!([]) {
            waiting = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        waiting,
        "Removal must reach its storage wait before revocation"
    );
    assert_eq!(
        request(reqwest::Method::POST, "/api/admin/lock")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    drop(gate);
    assert_eq!(
        pending.await.unwrap().status(),
        423,
        "Revocation while waiting must prevent deletion"
    );
    assert_eq!(store.list().unwrap(), vec![manifest.clone()]);
    let login: serde_json::Value = request(reqwest::Method::POST, "/api/admin/login")
        .json(&serde_json::json!({"passphrase":"Test-only nearby management passphrase"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    token = login["token"].as_str().unwrap().to_owned();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(data.join(store.cache_spec(&manifest).unwrap().path))
            .unwrap();
        assert_eq!(
            request(reqwest::Method::DELETE, &route)
                .header("x-tonalli-admin", &token)
                .send()
                .await
                .unwrap()
                .status(),
            503
        );
        let incomplete: serde_json::Value = request(reqwest::Method::GET, "/api/copies")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            incomplete["copies"][0]["removalPending"], true,
            "Incomplete cleanup must remain visible for retry"
        );
        drop(held);
    }
    let removed = request(reqwest::Method::DELETE, &route)
        .header("x-tonalli-admin", &token)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 200);
    let removed: serde_json::Value = removed.json().await.unwrap();
    assert_eq!(removed["removed"], true);
    assert!(store.list().unwrap().is_empty());
    let packs: serde_json::Value = request(reqwest::Method::GET, "/api/packs")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs["packs"], serde_json::json!([]));
    assert_eq!(
        request(reqwest::Method::POST, "/api/admin/lock")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        request(reqwest::Method::DELETE, &route)
            .header("x-tonalli-admin", &token)
            .send()
            .await
            .unwrap()
            .status(),
        423
    );
    app.shutdown().unwrap();
    let mut reopened = LoopbackServer::start(Arc::new(PackRegistry::default()), data, web).unwrap();
    assert!(store.list().unwrap().is_empty());
    reopened.shutdown().unwrap();
}
