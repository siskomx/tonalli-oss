use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc};
use tonalli_core::{
    pack::PreadSource,
    server::{PackId, PackRegistry, Server, ServerCfg},
    zim::Zim,
};

async fn stop_search_after_data(
    remote: &tonalli_core::remote::RemoteLibrary,
    root: &std::path::Path,
    action: tonalli_core::download::DownloadAction,
) -> tonalli_core::remote::CopyOutcome {
    let requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress = requested.clone();
    remote
        .copy_search(
            "selected",
            root.to_path_buf(),
            move || {
                if requested.load(std::sync::atomic::Ordering::SeqCst) {
                    action
                } else {
                    tonalli_core::download::DownloadAction::Continue
                }
            },
            move |p| {
                if p.phase == tonalli_core::download::DownloadPhase::Downloading
                    && p.downloaded_bytes > 0
                {
                    progress.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn selected_pack_transfer_is_exact_and_requires_existing_pairing() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("index.html"), "Owned transfer fixture").unwrap();
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    let expected = fs::read(&path).unwrap();
    let registry = Arc::new(PackRegistry::default());
    for id in ["selected", "private"] {
        registry
            .register_zim(
                PackId::new(id).unwrap(),
                Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
            )
            .unwrap();
    }
    let archive = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    let search = root.path().join("search");
    tonalli_core::index::build(&archive, &search, 1).unwrap();
    registry
        .attach_index(
            PackId::new("selected").unwrap(),
            Arc::new(tonalli_core::index::Index::open(&search, &archive).unwrap()),
        )
        .unwrap();
    let host = Server::start(
        ServerCfg {
            web_root: web,
            identity_dir: root.path().join("identity"),
            packs: vec![PackId::new("selected").unwrap()],
            ..ServerCfg::default()
        },
        Arc::clone(&registry),
    )
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(&host.certificate_der()).unwrap())
        .build()
        .unwrap();
    let origin = format!("https://{}", host.local_addr());
    assert_eq!(
        client
            .get(format!("{origin}/api/pack-transfer/selected"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let pair = client
        .post(format!("{origin}/pair"))
        .json(&serde_json::json!({
            "code":host.pairing_code().unwrap(),"label":"Owned transfer reader"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(pair.status(), 200);
    let cookies = pair
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let request = |path: &str| {
        client
            .get(format!("{origin}{path}"))
            .header(reqwest::header::COOKIE, &cookies)
            .header("sec-fetch-dest", "empty")
    };
    let session: serde_json::Value = request("/api/session")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let csrf = session["csrf"].as_str().unwrap();
    assert_eq!(
        request("/api/index-transfer/selected")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let search_manifest = request("/api/index-transfer/selected")
        .header("x-tonalli-csrf", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(search_manifest.status(), 200);
    let search_manifest: tonalli_core::index::snapshot::Manifest =
        search_manifest.json().await.unwrap();
    search_manifest.validate().unwrap();
    let incoming = tempfile::tempdir().unwrap();
    for file in &search_manifest.files {
        let url = format!(
            "/transfer/selected/search/{}/{}",
            search_manifest.generation, file.name
        );
        assert_eq!(
            client
                .get(format!("{origin}{url}"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let reply = request(&url).send().await.unwrap();
        assert_eq!(reply.status(), 200);
        let etag = reply
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = reply.bytes().await.unwrap();
        assert_eq!(bytes.len() as u64, file.bytes);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), file.sha256);
        if file.name == "meta.json" {
            let partial = request(&url)
                .header("range", "bytes=0-15")
                .header("if-range", &etag)
                .send()
                .await
                .unwrap();
            assert_eq!(partial.status(), 206);
            assert_eq!(partial.bytes().await.unwrap().as_ref(), &bytes[..16]);
            let stale = request(&url)
                .header("range", "bytes=0-15")
                .header("if-range", "\"other-generation\"")
                .send()
                .await
                .unwrap();
            assert_eq!(stale.status(), 200);
            assert_eq!(stale.bytes().await.unwrap(), bytes);
        }
        fs::write(incoming.path().join(&file.name), bytes).unwrap();
    }
    let restored = tempfile::tempdir().unwrap();
    let received = tonalli_core::index::snapshot::install(
        restored.path(),
        &archive,
        &search_manifest,
        |f| {
            Ok(Arc::new(
                PreadSource::open(&incoming.path().join(&f.name)).unwrap(),
            ))
        },
        || true,
    )
    .unwrap()
    .unwrap();
    let expected_hits = tonalli_core::index::Index::open(&search, &archive)
        .unwrap()
        .search("English", 5)
        .unwrap();
    assert!(!expected_hits.is_empty());
    assert_eq!(received.search("English", 5).unwrap(), expected_hits);
    for url in [
        "/api/index-transfer/private".to_string(),
        format!(
            "/transfer/private/search/{}/meta.json",
            search_manifest.generation
        ),
        format!(
            "/transfer/selected/search/{}/private.txt",
            search_manifest.generation
        ),
        format!(
            "/transfer/selected/search/{}/tonalli-building.json",
            search_manifest.generation
        ),
        format!("/transfer/selected/search/{}/meta.json", "0".repeat(64)),
    ] {
        assert_eq!(
            request(&url)
                .header("x-tonalli-csrf", csrf)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let second = root.path().join("new-search");
    tonalli_core::index::build(&archive, &second, 1).unwrap();
    registry
        .attach_index(
            PackId::new("selected").unwrap(),
            Arc::new(tonalli_core::index::Index::open(&second, &archive).unwrap()),
        )
        .unwrap();
    assert_eq!(
        request(&format!(
            "/transfer/selected/search/{}/meta.json",
            search_manifest.generation
        ))
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    for path in [
        "/api/zim/index-status?pack=selected",
        "/api/zim/index-start",
        "/api/zim/index-control",
    ] {
        assert_eq!(
            request(path)
                .header("x-tonalli-csrf", csrf)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert!(matches!(
            client
                .post(format!("{origin}{path}"))
                .header("cookie", &cookies)
                .header("sec-fetch-dest", "empty")
                .header("x-tonalli-csrf", csrf)
                .json(&serde_json::json!({"pack":"selected"}))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            404 | 405
        ));
    }
    assert_eq!(
        request("/api/copies")
            .header("x-tonalli-csrf", csrf)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let manifest = request("/api/pack-transfer/selected")
        .header("x-tonalli-csrf", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(manifest.status(), 200);
    let manifest: serde_json::Value = manifest.json().await.unwrap();
    assert_eq!(manifest["kind"], "zim");
    assert_eq!(manifest["bytes"], expected.len() as u64);
    assert_eq!(
        manifest["sha256"],
        format!("{:x}", Sha256::digest(&expected))
    );
    let blob = request("/transfer/selected/pack").send().await.unwrap();
    assert_eq!(blob.status(), 200);
    let etag = format!("\"{}\"", manifest["sha256"].as_str().unwrap());
    assert_eq!(
        blob.headers().get("etag").and_then(|v| v.to_str().ok()),
        Some(etag.as_str())
    );
    assert_eq!(blob.bytes().await.unwrap().as_ref(), expected);
    let partial = request("/transfer/selected/pack")
        .header("range", "bytes=17-31")
        .header("if-range", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(partial.status(), 206);
    assert_eq!(partial.bytes().await.unwrap().as_ref(), &expected[17..32]);
    assert_eq!(
        request("/api/pack-transfer/private")
            .header("x-tonalli-csrf", csrf)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        request("/transfer/private/pack")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        request("/api/pack-transfer/../notes")
            .header("x-tonalli-csrf", csrf)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let remote = tonalli_core::remote::RemoteLibrary::pair(
        tonalli_core::remote::RemoteHost::new(host.local_addr(), host.certificate_der()).unwrap(),
        &host.pairing_code().unwrap(),
        "Owned native copier",
    )
    .await
    .unwrap();
    let destination = tempfile::tempdir().unwrap();
    let paused = remote
        .copy_pack(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Pause,
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(matches!(paused, tonalli_core::remote::CopyOutcome::Paused));
    assert!(tonalli_core::sync::SyncStore::open(destination.path())
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
    let cancelled = remote
        .copy_pack(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Cancel,
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(matches!(
        cancelled,
        tonalli_core::remote::CopyOutcome::Cancelled
    ));
    assert!(tonalli_core::sync::SyncStore::open(destination.path())
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback = Arc::clone(&stop);
    let cancelled_after_data = remote
        .copy_pack(
            "selected",
            destination.path().to_path_buf(),
            move || {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    tonalli_core::download::DownloadAction::Cancel
                } else {
                    tonalli_core::download::DownloadAction::Continue
                }
            },
            move |p| {
                if p.phase == tonalli_core::download::DownloadPhase::Downloading
                    && p.downloaded_bytes > 0
                {
                    callback.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        cancelled_after_data,
        tonalli_core::remote::CopyOutcome::Cancelled
    ));
    let unfinished = fs::read_dir(destination.path().join("packs/nearby/versions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name().to_string_lossy().ends_with(".part")
                || e.file_name().to_string_lossy().ends_with(".part.meta")
        })
        .count();
    assert_eq!(
        unfinished, 0,
        "Cancelled must follow cleanup of this copy's unfinished files"
    );
    assert!(matches!(
        remote
            .copy_search(
                "selected",
                destination.path().to_path_buf(),
                || tonalli_core::download::DownloadAction::Continue,
                |_| Ok(())
            )
            .await,
        Err(tonalli_core::remote::CopyError::Pack(
            tonalli_core::pack::PackError::NotFound(_)
        ))
    ));
    let copied = remote
        .copy_pack(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Continue,
            |_| Ok(()),
        )
        .await
        .unwrap();
    let tonalli_core::remote::CopyOutcome::Complete(copied) = copied else {
        panic!("Expected verified copy")
    };
    let store = tonalli_core::sync::SyncStore::open(destination.path()).unwrap();
    assert_eq!(store.list().unwrap(), vec![copied.clone()]);
    let search_paused = remote
        .copy_search(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Pause,
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(matches!(
        search_paused,
        tonalli_core::remote::CopyOutcome::Paused
    ));
    let search_saved = remote
        .copy_search(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Continue,
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(matches!(
        search_saved,
        tonalli_core::remote::CopyOutcome::Complete(_)
    ));
    let local_archive = Arc::new(Zim::open(store.open_source(&copied).unwrap()).unwrap());
    let local_source = tonalli_core::index::jobs::IndexSource::registered(
        PackId::new(copied.local_id()).unwrap(),
        local_archive,
    )
    .unwrap();
    let local_jobs = tonalli_core::index::jobs::IndexJobs::open(destination.path()).unwrap();
    let local_search = local_jobs.safe_ready(&local_source).unwrap().unwrap();
    assert_eq!(local_search.search("English", 5).unwrap(), expected_hits);
    let cache = destination
        .path()
        .join("packs/nearby/search")
        .join(&copied.sha256);
    assert!(
        !cache.exists(),
        "completed transfer removes redundant cache bytes"
    );
    assert!(matches!(
        stop_search_after_data(
            &remote,
            destination.path(),
            tonalli_core::download::DownloadAction::Pause
        )
        .await,
        tonalli_core::remote::CopyOutcome::Paused
    ));
    assert!(fs::read_dir(&cache).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".part")));
    let unrelated = cache.join("unrelated.txt");
    fs::write(&unrelated, "preserve unrelated fixture").unwrap();
    assert!(remote
        .copy_search(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Cancel,
            |_| Ok(())
        )
        .await
        .is_err());
    assert_eq!(
        fs::read_to_string(&unrelated).unwrap(),
        "preserve unrelated fixture"
    );
    fs::remove_file(&unrelated).unwrap();
    assert!(matches!(
        remote
            .copy_search(
                "selected",
                destination.path().to_path_buf(),
                || tonalli_core::download::DownloadAction::Continue,
                |_| Ok(())
            )
            .await
            .unwrap(),
        tonalli_core::remote::CopyOutcome::Complete(_)
    ));
    assert!(matches!(
        stop_search_after_data(
            &remote,
            destination.path(),
            tonalli_core::download::DownloadAction::Cancel
        )
        .await,
        tonalli_core::remote::CopyOutcome::Cancelled
    ));
    assert!(!cache.exists());
    let search_cancelled = remote
        .copy_search(
            "selected",
            destination.path().to_path_buf(),
            || tonalli_core::download::DownloadAction::Cancel,
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(matches!(
        search_cancelled,
        tonalli_core::remote::CopyOutcome::Cancelled
    ));
    assert_eq!(
        local_jobs
            .safe_ready(&local_source)
            .unwrap()
            .unwrap()
            .search("English", 5)
            .unwrap(),
        expected_hits
    );
    assert_eq!(store.list().unwrap(), vec![copied.clone()]);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback = Arc::clone(&stop);
    let cancelled_existing = remote
        .copy_pack(
            "selected",
            destination.path().to_path_buf(),
            move || {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    tonalli_core::download::DownloadAction::Cancel
                } else {
                    tonalli_core::download::DownloadAction::Continue
                }
            },
            move |p| {
                if p.phase == tonalli_core::download::DownloadPhase::Verifying
                    && p.verified_bytes > 0
                {
                    callback.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        cancelled_existing,
        tonalli_core::remote::CopyOutcome::Cancelled
    ));
    assert_eq!(store.list().unwrap(), vec![copied.clone()]);
    assert_eq!(
        fs::read(
            destination
                .path()
                .join(store.cache_spec(&copied).unwrap().path)
        )
        .unwrap(),
        expected
    );
    let reader = host
        .devices()
        .unwrap()
        .into_iter()
        .find(|d| d.label == "Owned transfer reader")
        .unwrap();
    host.kick(&reader.id).unwrap();
    assert_eq!(
        request("/transfer/selected/pack")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}
