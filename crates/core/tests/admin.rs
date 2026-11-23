use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
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

#[test]
fn host_room_stays_off_until_explicit_sharing_and_keeps_host_authority_private() {
    use reqwest::Method;
    use std::sync::Mutex;
    use tonalli_core::{
        pack::PackError,
        room::{Room, RoomAction, RoomError, RoomMember, RoomRequest, RoomSnapshot},
        server::{SharedMedia, SharingControl, SharingStatus},
    };
    struct Owner(Mutex<Option<Room>>);
    impl SharingControl for Owner {
        fn status(&self) -> Result<SharingStatus, PackError> {
            Ok(SharingStatus {
                running: self.0.lock().unwrap().is_some(),
                busy: false,
                url: None,
                pairing_code: None,
                devices: vec![],
                networks: vec![],
                error: None,
                certificate_sha256: None,
                friendly_url: None,
                native_invitation: None,
                hotspot: None,
            })
        }
        fn start(
            &self,
            network: String,
            packs: Vec<String>,
            media: Arc<SharedMedia>,
        ) -> Result<(), PackError> {
            assert_eq!(network, "owned-room");
            assert!(packs.is_empty() && media.is_empty());
            *self.0.lock().unwrap() = Some(Room::new().unwrap());
            Ok(())
        }
        fn stop(&self) -> Result<(), PackError> {
            if let Some(mut room) = self.0.lock().unwrap().take() {
                room.end();
            }
            Ok(())
        }
        fn kick(&self, _: String) -> Result<(), PackError> {
            Ok(())
        }
        fn room(&self, request: RoomRequest) -> Result<RoomSnapshot, RoomError> {
            match self.0.lock().unwrap().as_mut() {
                Some(room) => room.request(
                    RoomMember {
                        id: "host".into(),
                        label: "Host".into(),
                        is_host: true,
                    },
                    request,
                    std::time::Instant::now(),
                ),
                None if matches!(request.action, RoomAction::Get | RoomAction::Leave) => {
                    Ok(RoomSnapshot::default())
                }
                None => Err(RoomError::Unavailable),
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let view = "a".repeat(32);
    let response = app
        .request(Method::GET, &format!("/api/room?view={view}"))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().unwrap()["active"], false);
    app.server
        .attach_sharing(Arc::new(Owner(Mutex::new(None))))
        .unwrap();
    let start = app
        .request(Method::POST, "/api/sharing/start")
        .json(&json!({"network":"owned-room","packs":[],"media":[]}))
        .send()
        .unwrap();
    assert_eq!(start.status(), 200);
    let initial: Value = app
        .request(Method::GET, &format!("/api/room?view={view}"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let epoch = initial["epoch"].as_str().unwrap();
    for (action, text) in [("join", ""), ("send", "Owned host room message")] {
        let reply = app
            .request(Method::POST, "/api/room")
            .json(&json!({"action":action,"view":view,"epoch":epoch,"text":text}))
            .send()
            .unwrap();
        assert_eq!(reply.status(), 200);
        if action == "send" {
            assert_eq!(
                reply.json::<Value>().unwrap()["messages"][0]["sender"]["is_host"],
                true
            );
        }
    }
    assert_eq!(
        app.request(Method::POST, "/api/sharing/stop")
            .send()
            .unwrap()
            .status(),
        200
    );
    let stopped: Value = app
        .request(Method::GET, &format!("/api/room?view={view}"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(stopped["active"], false);
    assert!(stopped["messages"].as_array().unwrap().is_empty());
}

#[test]
fn workshop_previews_require_private_auth_and_keep_managed_sources_unchanged() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let path = root.path().join("fixture");
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    fs::write(&path, bytes).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&path).unwrap()),
            "part.OBJ",
            "Workshop",
        )
        .unwrap();
    let response = app
        .request(Method::POST, "/api/workshop/model")
        .json(&json!({"id":file.id}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: Value = response.json().unwrap();
    assert_eq!(value["model"]["positions"].as_array().unwrap().len(), 9);
    assert_eq!(
        serde_json::to_value(store.get(&file.id).unwrap()).unwrap(),
        serde_json::to_value(&file).unwrap()
    );
    assert_eq!(store.open_file(&file.id).unwrap().0.sha256, file.sha256);
    let url = format!(
        "http://127.0.0.1:{}/api/workshop/model",
        app.server.info().port
    );
    assert_eq!(
        app.client
            .post(&url)
            .json(&json!({"id":file.id}))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        app.client
            .post(&url)
            .header(COOKIE, &app.cookie)
            .json(&json!({"id":file.id}))
            .send()
            .unwrap()
            .status(),
        403
    );
    for id in ["../notes.db", "C:/private.obj", "not-a-managed-id"] {
        assert_eq!(
            app.request(Method::POST, "/api/workshop/model")
                .json(&json!({"id":id}))
                .send()
                .unwrap()
                .status(),
            400
        );
    }
    assert_eq!(
        app.request(Method::POST, "/api/workshop/model")
            .json(&json!({"id":"a".repeat(32)}))
            .send()
            .unwrap()
            .status(),
        404
    );
    fs::write(&path, b"NaN invalid geometry").unwrap();
    let broken = store
        .import(
            Arc::new(PreadSource::open(&path).unwrap()),
            "bad.stl",
            "Workshop",
        )
        .unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/workshop/model")
            .json(&json!({"id":broken.id}))
            .send()
            .unwrap()
            .status(),
        415
    );
    let text = store
        .import(
            Arc::new(PreadSource::open(&path).unwrap()),
            "not-a-model.txt",
            "Workshop",
        )
        .unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/workshop/model")
            .json(&json!({"id":text.id}))
            .send()
            .unwrap()
            .status(),
        415
    );
}

#[test]
fn pdf_grants_require_private_fetch_auth_and_valid_managed_sources() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let source = root.path().join("source");
    let import = |name: &str, bytes: &[u8]| {
        fs::write(&source, bytes).unwrap();
        store
            .import(Arc::new(PreadSource::open(&source).unwrap()), name, "Books")
            .unwrap()
    };
    let bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
    let pdf = import("Book.PDF", bytes);
    let forged = import("Forged.pdf", b"<html>active content</html>");
    let renamed = import("Not-a-document.txt", bytes);
    let grant = app
        .request(Method::POST, "/api/documents/stream")
        .json(&json!({"id":pdf.id}))
        .send()
        .unwrap();
    assert_eq!(grant.status(), 200);
    let grant: Value = grant.json().unwrap();
    let url = grant["url"].as_str().unwrap();
    assert!(url.starts_with("/api/documents/content/"));
    let token = url.strip_prefix("/api/documents/content/").unwrap();
    assert_eq!(
        app.request(Method::GET, &format!("/media/{token}"))
            .send()
            .unwrap()
            .status(),
        404
    );
    let reply = app
        .request(Method::GET, url)
        .header("Range", "bytes=0-7")
        .send()
        .unwrap();
    assert_eq!(reply.status(), 206);
    assert_eq!(reply.headers()["content-type"], "application/pdf");
    assert_eq!(reply.headers()["cache-control"], "no-store");
    assert_eq!(
        reply.headers()["content-security-policy"],
        "default-src 'none'; sandbox"
    );
    assert_eq!(reply.bytes().unwrap().as_ref(), &bytes[..8]);
    let absolute = format!("http://127.0.0.1:{}{url}", app.server.info().port);
    assert_eq!(
        app.client
            .get(&absolute)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        app.client
            .get(&absolute)
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        403
    );
    for destination in ["document", "iframe", "object", "image"] {
        assert_eq!(
            app.client
                .get(&absolute)
                .header(COOKIE, &app.cookie)
                .header("X-Tonalli-Csrf", &app.csrf)
                .header("Sec-Fetch-Dest", destination)
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    for (id, expected) in [
        (&forged.id, 415),
        (&renamed.id, 415),
        (&"../notes.db".to_string(), 400),
        (&"f".repeat(32), 404),
    ] {
        assert_eq!(
            app.request(Method::POST, "/api/documents/stream")
                .json(&json!({"id":id}))
                .send()
                .unwrap()
                .status(),
            expected
        );
    }
    let root_before = fs::read(&source).unwrap();
    assert_eq!(
        app.request(Method::GET, "/api/documents/content/not-a-token")
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(fs::read(source).unwrap(), root_before);
}

#[test]
fn book_archive_grants_serve_only_captured_members_with_inert_frame_headers() {
    use reqwest::Method;
    use std::io::{Cursor, Write};
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    use zip::{write::SimpleFileOptions, ZipWriter};
    let rows:&[(&str,&[u8])]=&[
        ("mimetype",b"application/epub+zip"),
        ("META-INF/container.xml",br#"<container><rootfiles><rootfile full-path="package.opf"/></rootfiles></container>"#),
        ("package.opf",br#"<package><manifest><item id="one" href="text/one.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/></spine></package>"#),
        ("text/one.xhtml",b"<html><body><h1>Book page</h1><script>parent.bookUnsafe=1</script></body></html>"),
        ("cover.svg",br#"<svg xmlns="http://www.w3.org/2000/svg"><text>Cover</text></svg>"#),
    ];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in rows {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let source = root.path().join("owned.epub");
    fs::write(&source, &bytes).unwrap();
    let book = MediaStore::open(&root.path().join("data"))
        .unwrap()
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "Owned.epub",
            "Books",
        )
        .unwrap();
    let reply = app
        .request(Method::POST, "/api/documents/archive")
        .json(&json!({"id":book.id}))
        .send()
        .unwrap();
    assert_eq!(reply.status(), 200);
    let opened: Value = reply.json().unwrap();
    let base = opened["base_url"].as_str().unwrap();
    assert!(base.starts_with("/book/") && base.ends_with('/'));
    assert_eq!(opened["book"]["kind"], "epub");
    assert_eq!(opened["book"]["chapters"][0]["path"], "text/one.xhtml");
    let url = format!(
        "http://127.0.0.1:{}{base}text/one.xhtml",
        app.server.info().port
    );
    let get = |destination: &str, cookie: bool| {
        let request = app.client.get(&url).header("Sec-Fetch-Dest", destination);
        if cookie {
            request.header(COOKIE, &app.cookie)
        } else {
            request
        }
        .send()
        .unwrap()
    };
    assert_eq!(get("iframe", false).status(), 401);
    assert_eq!(get("document", true).status(), 403);
    assert_eq!(get("empty", true).status(), 403);
    let response = get("iframe", true);
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("script-src 'none'"));
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(response.text().unwrap().contains("Book page"));
    let svg_url = format!("http://127.0.0.1:{}{base}cover.svg", app.server.info().port);
    for destination in ["iframe", "image", "document", "empty"] {
        let response = app
            .client
            .get(&svg_url)
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", destination)
            .send()
            .unwrap();
        assert_eq!(
            response.status(),
            if matches!(destination, "iframe" | "image") {
                200
            } else {
                403
            }
        );
        if response.status().is_success() {
            assert_eq!(response.headers()["content-type"], "image/svg+xml");
            assert!(response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("script-src 'none'"));
        }
    }
    let missing = format!(
        "http://127.0.0.1:{}{base}missing.xhtml",
        app.server.info().port
    );
    assert_eq!(
        app.client
            .get(missing)
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", "iframe")
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(fs::read(source).unwrap(), bytes);
}

#[test]
fn sharing_selected_summary_survives_an_old_off_status_reply() {
    use reqwest::Method;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Barrier, Condvar, Mutex,
    };
    use tonalli_core::{
        media::MediaStore,
        pack::{PackError, PreadSource},
        server::{SharedMedia, SharingControl, SharingHotspot, SharingStatus},
    };
    struct Owner {
        running: AtomicBool,
        block_once: AtomicBool,
        entered: Barrier,
        release: (Mutex<bool>, Condvar),
    }
    impl SharingControl for Owner {
        fn status(&self) -> Result<SharingStatus, PackError> {
            let running = self.running.load(Ordering::SeqCst);
            if self.block_once.swap(false, Ordering::SeqCst) {
                self.entered.wait();
                let mut release = self.release.0.lock().unwrap();
                while !*release {
                    release = self.release.1.wait(release).unwrap();
                }
            }
            Ok(SharingStatus {
                running,
                busy: false,
                url: None,
                pairing_code: None,
                devices: vec![],
                networks: vec![],
                error: None,
                certificate_sha256: None,
                friendly_url: None,
                native_invitation: None,
                hotspot: running.then(|| SharingHotspot {
                    ssid: "DIRECT-fixture".into(),
                    password: "fixture-pass".into(),
                }),
            })
        }
        fn start(
            &self,
            _: String,
            _: Vec<String>,
            media: Arc<SharedMedia>,
        ) -> Result<(), PackError> {
            assert!(!media.is_empty());
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn stop(&self) -> Result<(), PackError> {
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn kick(&self, _: String) -> Result<(), PackError> {
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let source = root.path().join("fixture.txt");
    fs::write(&source, b"Chosen public text").unwrap();
    let file = MediaStore::open(&root.path().join("data"))
        .unwrap()
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "Chosen.txt",
            "",
        )
        .unwrap();
    let owner = Arc::new(Owner {
        running: AtomicBool::new(false),
        block_once: AtomicBool::new(true),
        entered: Barrier::new(2),
        release: (Mutex::new(false), Condvar::new()),
    });
    app.server.attach_sharing(owner.clone()).unwrap();
    let request = app.request(Method::GET, "/api/sharing");
    let pending = std::thread::spawn(move || request.send().unwrap());
    owner.entered.wait();
    assert_eq!(
        app.request(Method::POST, "/api/sharing/start")
            .json(&json!({"network":"fixture","packs":[],"media":[file.id]}))
            .send()
            .unwrap()
            .status(),
        200
    );
    *owner.release.0.lock().unwrap() = true;
    owner.release.1.notify_all();
    assert_eq!(pending.join().unwrap().status(), 200);
    let status: Value = app
        .request(Method::GET, "/api/sharing")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["status"]["running"], true);
    assert_eq!(status["status"]["hotspot"]["ssid"], "DIRECT-fixture");
    assert_eq!(status["status"]["hotspot"]["password"], "fixture-pass");
    assert_eq!(status["selected_media"].as_array().unwrap().len(), 1);
    assert_eq!(status["selected_media"][0]["id"], file.id);
    assert_eq!(status["shared_packs"], json!([]));

    // Items can be taken off the running share; nothing can be added back.
    let hide = |body: Value| {
        app.request(Method::POST, "/api/sharing/hide")
            .json(&body)
            .send()
            .unwrap()
            .status()
    };
    assert_eq!(hide(json!({"media":[file.id]})), 200);
    let after: Value = app
        .request(Method::GET, "/api/sharing")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(after["selected_media"], json!([]));
    assert!(hide(json!({"add":["x"]})).is_client_error());
    assert_eq!(hide(json!({"media":vec!["x"; 1025]})), 409);
    app.request(Method::POST, "/api/sharing/stop")
        .send()
        .unwrap();
    assert_eq!(
        hide(json!({"media":[file.id]})),
        409,
        "nothing to remove from when not sharing"
    );
}

#[test]
fn native_music_control_uses_current_queue_and_outlives_reader_attachment() {
    use reqwest::Method;
    use std::sync::Mutex;
    use tonalli_core::{
        media::MediaStore,
        music::{MusicStore, PlaybackAction, PlaybackStatus, PlaybackTrack},
        pack::{PackError, PreadSource},
        server::MusicControl,
    };
    #[derive(Default)]
    struct Engine(Mutex<Vec<(PlaybackAction, PlaybackTrack)>>);
    impl MusicControl for Engine {
        fn status(&self) -> Result<PlaybackStatus, PackError> {
            Ok(PlaybackStatus {
                available: true,
                ..Default::default()
            })
        }
        fn control(&self, action: PlaybackAction, track: PlaybackTrack) -> Result<(), PackError> {
            self.0.lock().unwrap().push((action, track));
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let data = root.path().join("data");
    let source = root.path().join("fixture.wav");
    fs::write(&source,b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0").unwrap();
    let file = MediaStore::open(&data)
        .unwrap()
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "Track.wav",
            "",
        )
        .unwrap();
    MusicStore::open(&data)
        .unwrap()
        .set_queue(0, vec![file.id.clone()], Some(0))
        .unwrap();
    let engine = Arc::new(Engine::default());
    app.server.attach_music(engine.clone()).unwrap();
    let status: Value = app
        .request(Method::GET, "/api/music/playback")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["available"], true);
    assert_eq!(
        app.request(Method::POST, "/api/music/playback")
            .json(&json!({"action":"play"}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        engine.0.lock().unwrap()[0].1.id.as_deref(),
        Some(file.id.as_str())
    );
    fs::write(
        data.join("packs/private/music.json"),
        b"corrupt while playing",
    )
    .unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/music/playback")
            .json(&json!({"action":"stop"}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(engine.0.lock().unwrap().len(), 2);
    drop(app);
    assert_eq!(
        engine.0.lock().unwrap().len(),
        2,
        "Closing a reader must not stop its independent music service"
    );
}

#[test]
fn music_controls_and_stream_capabilities_remain_private() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    assert_eq!(
        app.request(Method::GET, "/music").send().unwrap().status(),
        200
    );
    assert_eq!(
        app.request(Method::GET, "/api/music")
            .send()
            .unwrap()
            .status(),
        200
    );
    let source = root.path().join("test.wav");
    fs::write(&source,b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0").unwrap();
    let file = MediaStore::open(&root.path().join("data"))
        .unwrap()
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "Track.wav",
            "",
        )
        .unwrap();
    let reply = app
        .request(Method::POST, "/api/music")
        .json(&json!({"action":"queue","revision":0,"tracks":[file.id],"current":0}))
        .send()
        .unwrap();
    assert_eq!(reply.status(), 200);
    let reply = app
        .request(Method::POST, "/api/music/stream")
        .json(&json!({"id":file.id}))
        .send()
        .unwrap();
    assert_eq!(reply.status(), 200);
    let stream: Value = reply.json().unwrap();
    let path = stream["url"].as_str().unwrap();
    assert!(path.starts_with("/media/"));
    let url = format!("http://127.0.0.1:{}{path}", app.server.info().port);
    assert_eq!(
        app.client
            .get(&url)
            .header("Sec-Fetch-Dest", "audio")
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        app.client
            .get(&url)
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", "document")
            .send()
            .unwrap()
            .status(),
        403
    );
    let audio = app
        .client
        .get(&url)
        .header(COOKIE, &app.cookie)
        .header("Sec-Fetch-Dest", "audio")
        .header("Range", "bytes=0-3")
        .send()
        .unwrap();
    assert_eq!(audio.status(), 206);
    assert_eq!(audio.headers()["content-type"], "audio/wav");
    assert_eq!(audio.bytes().unwrap().as_ref(), b"RIFF");
    fs::write(&source, b"<html>not audio</html>").unwrap();
    let bad = MediaStore::open(&root.path().join("data"))
        .unwrap()
        .import(Arc::new(PreadSource::open(&source).unwrap()), "Bad.wav", "")
        .unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/music/stream")
            .json(&json!({"id":bad.id}))
            .send()
            .unwrap()
            .status(),
        415
    );
    assert_eq!(
        app.request(Method::POST, "/api/music/stream")
            .json(&json!({"id":"../../notes.db"}))
            .send()
            .unwrap()
            .status(),
        400
    );
}

#[test]
fn video_and_cover_grants_are_private_typed_and_do_not_evict_music() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let source = root.path().join("source");
    let import = |name: &str, bytes: &[u8]| {
        fs::write(&source, bytes).unwrap();
        store
            .import(
                Arc::new(PreadSource::open(&source).unwrap()),
                name,
                "Shows/Example/Season 01",
            )
            .unwrap()
    };
    let video = import("Episode.mp4", b"\0\0\0\x18ftypisom\0\0\0\0isommp42");
    let image = import(
        "poster.png",
        b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0",
    );
    let audio = import("music.wav", b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0");
    let grant = |endpoint: &str, id: &str| {
        let reply = app
            .request(Method::POST, endpoint)
            .json(&json!({"id":id}))
            .send()
            .unwrap();
        assert_eq!(reply.status(), 200, "{endpoint}");
        reply.json::<Value>().unwrap()["url"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        app.request(Method::GET, "/videos").send().unwrap().status(),
        200
    );
    let audio_url = grant("/api/music/stream", &audio.id);
    let video_url = grant("/api/video/stream", &video.id);
    let image_url = grant("/api/video/poster", &image.id);
    let get = |path: &str, destination: &str, authenticated: bool| {
        let request = app
            .client
            .get(format!("http://127.0.0.1:{}{path}", app.server.info().port))
            .header("Sec-Fetch-Dest", destination)
            .header("Range", "bytes=4-7");
        if authenticated {
            request.header(COOKIE, &app.cookie)
        } else {
            request
        }
        .send()
        .unwrap()
    };
    assert_eq!(get(&video_url, "video", false).status(), 401);
    assert_eq!(get(&video_url, "document", true).status(), 403);
    assert_eq!(get(&video_url, "image", true).status(), 403);
    assert_eq!(get(&image_url, "video", true).status(), 403);
    let response = get(&video_url, "video", true);
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.bytes().unwrap().as_ref(), b"ftyp");
    let response = get(&image_url, "image", true);
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "image/png");
    for _ in 0..40 {
        grant("/api/video/poster", &image.id);
    }
    assert_eq!(get(&audio_url, "audio", true).status(), 206);
    let position = |url: &str, sequence: u64, seconds: f64| {
        let reply=app.request(Method::POST,"/api/video/progress").json(&json!({"token":url.strip_prefix("/media/").unwrap(),"sequence":sequence,"seconds":seconds})).send().unwrap();
        assert_eq!(reply.status(), 200);
        reply.json::<Value>().unwrap()["accepted"]
            .as_bool()
            .unwrap()
    };
    assert!(position(&video_url, 2, 12.0));
    assert!(!position(&video_url, 1, 3.0));
    let newer = grant("/api/video/stream", &video.id);
    assert!(!position(&video_url, 3, 5.0));
    assert!(position(&newer, 0, 13.0));
    let progress = tonalli_core::reading::ProgressStore::open(&root.path().join("data"))
        .unwrap()
        .list()
        .unwrap();
    assert_eq!(
        progress[0].destination,
        tonalli_core::reading::Destination::Media {
            id: video.id.clone()
        }
    );
    assert_eq!(
        progress[0].position,
        tonalli_core::reading::Position::Time { seconds: 13.0 }
    );
    let bad = import("Fake.mp4", b"<html>not a movie</html>");
    for (endpoint, id, status) in [
        ("/api/video/stream", bad.id.as_str(), 415),
        ("/api/video/poster", video.id.as_str(), 415),
        ("/api/video/stream", "../../notes.db", 400),
    ] {
        assert_eq!(
            app.request(Method::POST, endpoint)
                .json(&json!({"id":id}))
                .send()
                .unwrap()
                .status(),
            status
        );
    }
}

#[test]
fn gallery_raster_admission_never_serves_svg_or_html_inline() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let source = root.path().join("source");
    let cases:[(&str,&[u8],u16,&str);5]=[
        ("pixel.gif",b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;",200,"image/gif"),
        ("pixel.bmp",b"BM\x1a\0\0\0\0\0\0\0\x1a\0\0\0\x0c\0\0\0\x01\0\x01\0\x01\0\x18\0",200,"image/bmp"),
        ("pixel.avif",b"\0\0\0\x18ftypavif\0\0\0\0avifmif1",200,"image/avif"),
        ("page.svg",b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>",415,""),
        ("page.gif",b"<html>not an image</html>",415,"")
    ];
    for (name, bytes, status, mime) in cases {
        fs::write(&source, bytes).unwrap();
        let file = store
            .import(Arc::new(PreadSource::open(&source).unwrap()), name, "")
            .unwrap();
        let reply = app
            .request(Method::POST, "/api/video/poster")
            .json(&json!({"id":file.id}))
            .send()
            .unwrap();
        assert_eq!(reply.status(), status, "{name}");
        if status == 200 {
            let url: Value = reply.json().unwrap();
            let reply = app
                .client
                .get(format!(
                    "http://127.0.0.1:{}{}",
                    app.server.info().port,
                    url["url"].as_str().unwrap()
                ))
                .header(COOKIE, &app.cookie)
                .header("Sec-Fetch-Dest", "image")
                .send()
                .unwrap();
            assert_eq!(reply.status(), 200);
            assert_eq!(reply.headers()["content-type"], mime);
            assert_eq!(reply.bytes().unwrap().as_ref(), bytes);
        }
    }
}

#[test]
fn private_reading_progress_requires_session_and_survives_restart() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource};
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let list = || app.request(Method::GET, "/api/reading").send().unwrap();
    assert_eq!(list().status(), 200);
    let source = root.path().join("fixture.txt");
    fs::write(&source, "owned test text").unwrap();
    let file = MediaStore::open(&root.path().join("data"))
        .unwrap()
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "fixture.txt",
            "",
        )
        .unwrap();
    let entry = json!({"destination":{"kind":"media","id":file.id},"position":{"kind":"scroll","y":120.0},"title":"Fixture","bookmarked":false});
    assert_eq!(
        app.request(Method::POST, "/api/reading")
            .json(&entry)
            .send()
            .unwrap()
            .status(),
        200
    );
    let url = format!("http://127.0.0.1:{}/api/reading", app.server.info().port);
    assert_eq!(app.client.get(url).send().unwrap().status(), 401);
    assert_eq!(app.request(Method::POST,"/api/reading").json(&json!({"destination":{"kind":"article","pack":"missing","url":"A/Missing"},"position":{"kind":"start"},"title":"Missing","bookmarked":false})).send().unwrap().status(),404);
    drop(app);
    let app = App::open(root.path());
    let rows: Value = app
        .request(Method::GET, "/api/reading")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(rows["entries"][0], entry);
    assert_eq!(
        app.request(Method::POST, "/api/reading/bookmark")
            .json(&json!({"destination":entry["destination"],"bookmarked":true}))
            .send()
            .unwrap()
            .status(),
        200
    );
    MediaStore::open(&root.path().join("data"))
        .unwrap()
        .remove(&file.id, file.version)
        .unwrap();
    let missing: Value = app
        .request(Method::GET, "/api/reading")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(missing["unavailable"][0], entry["destination"]);
    assert_eq!(missing["entries"][0]["bookmarked"], true);
    assert_eq!(fs::read_to_string(source).unwrap(), "owned test text");
    assert_eq!(
        app.request(Method::DELETE, "/api/reading")
            .json(&entry["destination"])
            .send()
            .unwrap()
            .status(),
        200
    );
}

#[test]
fn managed_file_routes_require_admin_and_confirmed_versioned_mutations() {
    use reqwest::Method;
    let root = tempfile::tempdir().unwrap();
    let mut app = App::open(root.path());
    let upload = || {
        app.request(
            Method::POST,
            "/api/files/upload?name=checklist.txt&folder=Trips",
        )
        .header("Content-Type", "application/octet-stream")
        .body("Water, map, book")
    };
    assert_eq!(upload().send().unwrap().status(), 423);
    assert_eq!(
        app.request(Method::POST, "/api/files/native-import")
            .json(&json!({"folder":"Trips"}))
            .send()
            .unwrap()
            .status(),
        423
    );
    let login: Value = app
        .request(Method::POST, "/api/admin/setup")
        .json(&json!({"passphrase":"synthetic file password"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token = login["token"].as_str().unwrap();
    let response = upload().header("X-Tonalli-Admin", token).send().unwrap();
    assert_eq!(response.status(), 200);
    let file: Value = response.json().unwrap();
    let id = file["id"].as_str().unwrap();
    assert_eq!(
        app.request(Method::GET, &format!("/api/files/{id}/download"))
            .send()
            .unwrap()
            .text()
            .unwrap(),
        "Water, map, book"
    );
    assert_eq!(
        app.request(Method::POST, &format!("/api/files/{id}/rename"))
            .header("X-Tonalli-Admin", token)
            .json(&json!({"version":1,"name":"packing.txt"}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.request(Method::PUT, &format!("/api/files/{id}/text"))
            .header("X-Tonalli-Admin", token)
            .json(&json!({"version":1,"text":"stale"}))
            .send()
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        app.request(Method::PUT, &format!("/api/files/{id}/text"))
            .header("X-Tonalli-Admin", token)
            .json(&json!({"version":2,"text":"Updated checklist"}))
            .send()
            .unwrap()
            .status(),
        200
    );
    let text: Value = app
        .request(Method::GET, &format!("/api/files/{id}/text"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(text["text"], "Updated checklist");
    assert_eq!(
        app.request(Method::DELETE, &format!("/api/files/{id}"))
            .header("X-Tonalli-Admin", token)
            .json(&json!({"version":3,"confirmed":false}))
            .send()
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        app.request(Method::POST, "/api/admin/lock")
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.request(Method::DELETE, &format!("/api/files/{id}"))
            .header("X-Tonalli-Admin", token)
            .json(&json!({"version":3,"confirmed":true}))
            .send()
            .unwrap()
            .status(),
        423
    );
    let login: Value = app
        .request(Method::POST, "/api/admin/login")
        .json(&json!({"passphrase":"synthetic file password"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(
        app.request(Method::DELETE, &format!("/api/files/{id}"))
            .header("X-Tonalli-Admin", login["token"].as_str().unwrap())
            .json(&json!({"version":3,"confirmed":true}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.request(Method::GET, &format!("/api/files/{id}/download"))
            .send()
            .unwrap()
            .status(),
        404
    );
    let grant: Value = app
        .request(Method::POST, "/api/files/native-import")
        .header("X-Tonalli-Admin", login["token"].as_str().unwrap())
        .json(&json!({"folder":"Trips"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    app.server.shutdown().unwrap();
    let store = tonalli_core::media::MediaStore::open(&root.path().join("data")).unwrap();
    assert_eq!(
        store
            .claim_native_import(grant["ticket"].as_str().unwrap())
            .unwrap(),
        "Trips"
    );
}
impl App {
    fn open(root: &std::path::Path) -> Self {
        Self::with_registry(root, Arc::new(PackRegistry::default()))
    }
    fn with_registry(root: &std::path::Path, registry: Arc<PackRegistry>) -> Self {
        let web = root.join("web");
        fs::create_dir_all(&web).unwrap();
        fs::write(web.join("index.html"), "fixture").unwrap();
        let server = LoopbackServer::start(registry, root.join("data"), web).unwrap();
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
        Self {
            server,
            client,
            cookie,
            csrf: session["csrf"].as_str().unwrap().into(),
        }
    }
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(
                method,
                format!("http://127.0.0.1:{}{path}", self.server.info().port),
            )
            .header(COOKIE, &self.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &self.csrf)
    }
}

#[test]
fn embedded_pack_copy_requires_admin_and_keeps_source_and_library_on_failure() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource, server::PackId, zim::Zim};
    let root = tempfile::tempdir().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles/benign-v1.zim");
    let original = fs::read(&path).unwrap();
    let zim = Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap());
    let bytes = zim.article("C/assets/body-probe.bin").unwrap().bytes;
    let registry = Arc::new(PackRegistry::default());
    registry
        .register_zim(PackId::new("embedded").unwrap(), zim)
        .unwrap();
    let app = App::with_registry(root.path(), registry.clone());
    let body = json!({"pack":"embedded","url":"C/assets/body-probe.bin"});
    assert_eq!(
        app.request(Method::POST, "/api/documents/import")
            .json(&body)
            .send()
            .unwrap()
            .status(),
        423
    );
    let setup: Value = app
        .request(Method::POST, "/api/admin/setup")
        .json(&json!({"passphrase":"Owned embedded document test"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token = setup["token"].as_str().unwrap();
    let copied = app
        .request(Method::POST, "/api/documents/import")
        .header("X-Tonalli-Admin", token)
        .json(&body)
        .send()
        .unwrap();
    assert_eq!(copied.status(), 200);
    let copied: Value = copied.json().unwrap();
    assert_eq!(copied["name"], "body-probe.bin");
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let rows = store.list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].bytes, bytes.len() as u64);
    assert_eq!(rows[0].id, copied["id"].as_str().unwrap());
    let (_, source) = store.open_file(&rows[0].id).unwrap();
    let mut actual = vec![0; bytes.len()];
    let mut offset = 0;
    while offset < actual.len() {
        let count = source
            .read_at(offset as u64, &mut actual[offset..])
            .unwrap();
        assert!(count > 0);
        offset += count;
    }
    assert_eq!(actual, bytes);
    for url in ["C/missing.pdf", "../notes.db", "C/../../notes.db"] {
        assert!(!app
            .request(Method::POST, "/api/documents/import")
            .header("X-Tonalli-Admin", token)
            .json(&json!({"pack":"embedded","url":url}))
            .send()
            .unwrap()
            .status()
            .is_success());
    }
    app.request(Method::POST, "/api/admin/lock").send().unwrap();
    assert_eq!(
        app.request(Method::POST, "/api/documents/import")
            .header("X-Tonalli-Admin", token)
            .json(&body)
            .send()
            .unwrap()
            .status(),
        423
    );
    assert_eq!(store.list().unwrap().len(), 1);
    registry.remove(&PackId::new("embedded").unwrap()).unwrap();
    assert!(store.open_file(&rows[0].id).is_ok());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn embedded_archives_open_privately_without_a_managed_import() {
    use reqwest::Method;
    use tonalli_core::{media::MediaStore, pack::PreadSource, server::PackId, zim::Zim};
    let root = tempfile::tempdir().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/documents/documents-v1.zim");
    let registry = Arc::new(PackRegistry::default());
    registry
        .register_zim(
            PackId::new("embedded").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
        )
        .unwrap();
    let app = App::with_registry(root.path(), registry);
    for (url, kind, first) in [
        ("C/docs/book.epub", "epub", "OPS/one.xhtml"),
        ("C/docs/comic.cbz", "comic", "pages/page01.png"),
    ] {
        let response = app
            .request(Method::POST, "/api/documents/archive")
            .json(&json!({"pack":"embedded","url":url}))
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: Value = response.json().unwrap();
        assert_eq!(value["book"]["kind"], kind);
        assert_eq!(value["book"]["chapters"][0]["path"], first);
        assert!(value["base_url"].as_str().unwrap().starts_with("/book/"));
    }
    for body in [
        json!({"pack":"embedded","url":"C/spoof.pdf"}),
        json!({"pack":"embedded","url":"../notes.db"}),
        json!({"pack":"embedded","url":"C/docs/book.epub","id":"a".repeat(32)}),
    ] {
        assert!(!app
            .request(Method::POST, "/api/documents/archive")
            .json(&body)
            .send()
            .unwrap()
            .status()
            .is_success());
    }
    assert!(MediaStore::open(&root.path().join("data"))
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
}

#[test]
fn admin_login_configuration_and_restart_preserve_private_state() {
    use reqwest::Method;
    let root = tempfile::tempdir().unwrap();
    let mut app = App::open(root.path());
    let status: Value = app
        .request(Method::GET, "/api/admin")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["configured"], false);
    let url = format!("http://127.0.0.1:{}/api/admin", app.server.info().port);
    assert_eq!(app.client.get(&url).send().unwrap().status(), 401);
    assert_eq!(
        app.client
            .get(&url)
            .header(COOKIE, &app.cookie)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .json(&json!({"revision":0,"theme":"dark","show_maps":false,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        423
    );
    let setup = app
        .request(Method::POST, "/api/admin/setup")
        .json(&json!({"passphrase":"synthetic admin password"}))
        .send()
        .unwrap();
    assert_eq!(setup.status(), 200);
    let setup: Value = setup.json().unwrap();
    let token = setup["token"].as_str().unwrap();
    let stored = root.path().join("data/packs/private/admin.json");
    let initial = fs::read(&stored).unwrap();
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .header("X-Tonalli-Admin", "unrelated token")
            .json(&json!({"revision":0,"theme":"dark","show_maps":false,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        423
    );
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .header("X-Tonalli-Admin", token)
            .json(&json!({"revision":0,"theme":"unknown","show_maps":false,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        422
    );
    assert_eq!(
        app.request(Method::POST, "/api/admin/login")
            .json(&json!({"passphrase":"x".repeat(10000)}))
            .send()
            .unwrap()
            .status(),
        413
    );
    assert_eq!(fs::read(&stored).unwrap(), initial);
    assert_eq!(
        app.request(Method::POST, "/api/admin/setup")
            .json(&json!({"passphrase":"replacement password"}))
            .send()
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .header("X-Tonalli-Admin", token)
            .json(&json!({"revision":0,"theme":"dark","show_maps":false,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .header("X-Tonalli-Admin", token)
            .json(&json!({"revision":0,"theme":"light","show_maps":true,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        app.request(Method::POST, "/api/admin/lock")
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.request(Method::PUT, "/api/preferences")
            .header("X-Tonalli-Admin", token)
            .json(&json!({"revision":1,"theme":"light","show_maps":true,"show_notes":true}))
            .send()
            .unwrap()
            .status(),
        423
    );
    let stored = root.path().join("data/packs/private/admin.json");
    let before = fs::read(&stored).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains("synthetic admin password"));
    assert_eq!(
        app.request(Method::POST, "/api/admin/login")
            .json(&json!({"passphrase":"wrong password"}))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(fs::read(&stored).unwrap(), before);
    assert_eq!(
        app.request(Method::POST, "/api/admin/login")
            .json(&json!({"passphrase":"synthetic admin password"}))
            .send()
            .unwrap()
            .status(),
        429
    );
    app.server.shutdown().unwrap();
    drop(app);
    let app = App::open(root.path());
    let status: Value = app
        .request(Method::GET, "/api/admin")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["configured"], true);
    assert_eq!(status["unlocked"], false);
    let preferences: Value = app
        .request(Method::GET, "/api/preferences")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(preferences["theme"], "dark");
    assert_eq!(preferences["show_maps"], false);
    assert_eq!(preferences["revision"], 1);
    let login = app
        .request(Method::POST, "/api/admin/login")
        .json(&json!({"passphrase":"synthetic admin password"}))
        .send()
        .unwrap();
    assert_eq!(login.status(), 200);
}

#[test]
fn malformed_admin_metadata_never_becomes_a_new_setup() {
    use reqwest::Method;
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path());
    let file = root.path().join("data/packs/private/admin.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"broken").unwrap();
    assert_eq!(
        app.request(Method::GET, "/api/admin")
            .send()
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        app.request(Method::POST, "/api/admin/setup")
            .json(&json!({"passphrase":"replacement password"}))
            .send()
            .unwrap()
            .status(),
        503
    );
    assert_eq!(fs::read(file).unwrap(), b"broken");
}
