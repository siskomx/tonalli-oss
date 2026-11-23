use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use tempfile::TempDir;
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::server::{PackId, PackRegistry, Server, ServerCfg, ServerHandle};
use tonalli_core::zim::Zim;

#[allow(dead_code)]
#[path = "fixtures/pmtiles/mod.rs"]
mod pmtiles;

struct Fixture {
    server: ServerHandle,
    registry: Arc<PackRegistry>,
    _root: TempDir,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct Credentials {
    content: String,
    api: String,
    csrf: String,
}

#[test]
fn paired_room_is_explicit_bounded_plain_text_and_revoked_without_private_state() {
    use serde_json::{json, Value};
    let f = Fixture::with_packs(8, &[]);
    let first = f.pair("Host");
    let second = f.pair("Owned room reader");
    let view1 = "1".repeat(32);
    let view2 = "2".repeat(32);
    let reply = f.api(&format!("/api/room?view={view1}"), &first);
    assert_eq!(reply.status, 200);
    let initial: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(initial["joined"], false);
    assert!(initial["messages"].as_array().unwrap().is_empty());
    let epoch = initial["epoch"].as_str().unwrap();
    let post = |c: &Credentials, body: Value| {
        f.request(
            "POST",
            "/api/room",
            &[
                ("Cookie", &c.api),
                ("Sec-Fetch-Dest", "empty"),
                ("X-Tonalli-Csrf", &c.csrf),
                ("Content-Type", "application/json"),
            ],
            &serde_json::to_vec(&body).unwrap(),
        )
    };
    assert_eq!(
        post(
            &first,
            json!({"action":"send","view":view1,"epoch":epoch,"text":"before join"})
        )
        .status,
        403
    );
    assert_eq!(
        post(&first, json!({"action":"join","view":view1,"epoch":epoch})).status,
        200
    );
    assert_eq!(
        post(&second, json!({"action":"join","view":view2,"epoch":epoch})).status,
        200
    );
    let unicode = "🦊".repeat(1000);
    assert_eq!(
        post(
            &first,
            json!({"action":"send","view":view1,"epoch":epoch,"text":unicode})
        )
        .status,
        200
    );
    let reply = f.api(&format!("/api/room?view={view2}&epoch={epoch}"), &second);
    assert_eq!(reply.status, 200);
    let received: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(received["messages"][0]["text"], unicode);
    assert_eq!(received["messages"][0]["sender"]["is_host"], false);
    assert_eq!(post(&first,json!({"action":"send","view":view1,"epoch":epoch,"text":"<img src='https://example.invalid/never'>"})).status,200);
    assert_eq!(post(&first,json!({"action":"send","view":view1,"epoch":epoch,"text":"forged","sender":{"id":"host","is_host":true}})).status,422);
    assert_eq!(
        f.request("GET", &format!("/api/room?view={view1}"), &[], &[])
            .status,
        401
    );
    assert_eq!(
        f.request(
            "GET",
            &format!("/api/room?view={view1}"),
            &[("Cookie", &first.api), ("Sec-Fetch-Dest", "empty")],
            &[]
        )
        .status,
        403
    );
    assert_eq!(
        f.request(
            "GET",
            &format!("/api/room?view={view1}"),
            &[
                ("Cookie", &first.content),
                ("Sec-Fetch-Dest", "empty"),
                ("X-Tonalli-Csrf", &first.csrf)
            ],
            &[]
        )
        .status,
        401
    );
    for path in [
        "/api/notes",
        "/api/files",
        "/api/sharing",
        "/api/tv",
        "/api/tv/start",
        "/api/tv/stop",
        "/chat",
        "/notes",
    ] {
        assert_eq!(f.api(path, &first).status, 404);
    }
    assert_eq!(
        post(&first, json!({"action":"leave","view":view1,"epoch":epoch})).status,
        200
    );
    assert_eq!(
        post(
            &first,
            json!({"action":"send","view":view1,"epoch":epoch,"text":"left"})
        )
        .status,
        403
    );
    let id = f
        .server
        .devices()
        .unwrap()
        .into_iter()
        .find(|d| d.label == "Owned room reader")
        .unwrap()
        .id;
    f.server.kick(&id).unwrap();
    assert_eq!(
        f.api(&format!("/api/room?view={view2}"), &second).status,
        401
    );
    assert_eq!(
        post(&first, json!({"action":"join","view":view1,"epoch":epoch})).status,
        200
    );
    let final_view: Value =
        serde_json::from_slice(&f.api(&format!("/api/room?view={view1}"), &first).body).unwrap();
    assert_eq!(final_view["members"].as_array().unwrap().len(), 1);
}

#[test]
fn selected_media_share_is_snapshot_paired_read_only_and_revocable() {
    use tonalli_core::{media::MediaStore, server::SharedMedia};
    let mut f = Fixture::with_packs(8, &[]);
    let data = f._root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let store = MediaStore::open(&data).unwrap();
    let original = f._root.path().join("selected.txt");
    fs::write(&original, "<script>private selection</script>").unwrap();
    let selected = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Chosen.txt",
            "Chosen folder",
        )
        .unwrap();
    fs::write(&original, "HIDDEN SENTINEL").unwrap();
    let hidden = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Hidden.txt",
            "Private folder",
        )
        .unwrap();
    let image_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0";
    fs::write(&original, image_bytes).unwrap();
    let image = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Chosen.png",
            "",
        )
        .unwrap();
    let html_bytes = b"<html><script>alert('must never run')</script></html>";
    fs::write(&original, html_bytes).unwrap();
    let html = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Chosen.html",
            "",
        )
        .unwrap();
    fs::write(&original, vec![b'A'; 65537]).unwrap();
    let large = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Large.txt",
            "",
        )
        .unwrap();
    assert!(SharedMedia::capture(&data, &["../private/notes.db".into()]).is_err());
    assert!(SharedMedia::capture(&data, &vec![selected.id.clone(); 129]).is_err());
    let many: Vec<String> = (0..1025).map(|n| format!("missing-{n}")).collect();
    assert!(SharedMedia::capture(&data, &many).is_err());
    let media = Arc::new(
        SharedMedia::capture(
            &data,
            &[
                selected.id.clone(),
                image.id.clone(),
                html.id.clone(),
                large.id.clone(),
            ],
        )
        .unwrap(),
    );
    store
        .edit_text(&selected.id, selected.version, "A later private edit")
        .unwrap();
    f.server.shutdown().unwrap();
    f.server = Server::start(
        ServerCfg {
            web_root: f._root.path().join("web"),
            identity_dir: f._root.path().join("identity"),
            media,
            ..ServerCfg::default()
        },
        Arc::clone(&f.registry),
    )
    .unwrap();
    let c = f.pair("selected media reader");
    assert_eq!(f.request("GET", "/api/shared-media", &[], &[]).status, 401);
    let listing = f.api("/api/shared-media", &c);
    assert_eq!(listing.status, 200);
    let rows: serde_json::Value = serde_json::from_slice(&listing.body).unwrap();
    assert_eq!(rows["files"].as_array().unwrap().len(), 4);
    assert_eq!(rows["files"][0]["file"]["id"], selected.id);
    assert!(!String::from_utf8_lossy(&listing.body).contains("Hidden"));
    let text = f.api(&format!("/api/shared-media/{}/text", selected.id), &c);
    assert_eq!(text.status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&text.body).unwrap()["text"],
        "<script>private selection</script>"
    );
    let path = format!("/shared-media/{}/download", selected.id);
    let reply = f.request(
        "GET",
        &path,
        &[("Cookie", &c.content), ("Range", "bytes=0-7")],
        &[],
    );
    assert_eq!(reply.status, 206);
    assert_eq!(reply.body, b"<script>");
    assert!(reply
        .headers
        .iter()
        .any(|(key, value)| key == "content-disposition" && value.starts_with("attachment;")));
    assert!(reply
        .headers
        .iter()
        .any(|(key, value)| key == "content-type" && value == "application/octet-stream"));
    let image_path = format!("/shared-media/{}/content", image.id);
    assert_eq!(
        f.request("GET", &image_path, &[("Sec-Fetch-Dest", "image")], &[])
            .status,
        401
    );
    assert_eq!(
        f.request(
            "GET",
            &image_path,
            &[("Cookie", &c.content), ("Sec-Fetch-Dest", "document")],
            &[]
        )
        .status,
        403
    );
    let picture = f.request(
        "GET",
        &image_path,
        &[("Cookie", &c.content), ("Sec-Fetch-Dest", "image")],
        &[],
    );
    assert_eq!(picture.status, 200);
    assert_eq!(picture.body, image_bytes);
    assert!(picture
        .headers
        .iter()
        .any(|(key, value)| key == "content-type" && value == "image/png"));
    let attachment = f.request(
        "GET",
        &format!("/shared-media/{}/download", html.id),
        &[("Cookie", &c.content)],
        &[],
    );
    assert_eq!(attachment.status, 200);
    assert_eq!(attachment.body, html_bytes);
    assert!(attachment
        .headers
        .iter()
        .any(|(key, value)| key == "content-disposition" && value.starts_with("attachment;")));
    assert_eq!(
        f.api(&format!("/api/shared-media/{}/text", large.id), &c)
            .status,
        404
    );
    assert_eq!(
        f.request(
            "GET",
            &format!("/shared-media/{}/content", html.id),
            &[("Cookie", &c.content), ("Sec-Fetch-Dest", "document")],
            &[]
        )
        .status,
        404
    );
    for path in [
        format!("/shared-media/{}/download", hidden.id),
        format!("/api/shared-media/{}/text", hidden.id),
        format!("/shared-media/{}/content", selected.id),
    ] {
        let response = if path.starts_with("/api/") {
            f.api(&path, &c)
        } else {
            f.request("GET", &path, &[("Cookie", &c.content)], &[])
        };
        assert_eq!(response.status, 404, "{path}");
    }
    assert_eq!(
        f.request("POST", &path, &[("Cookie", &c.content)], &[])
            .status,
        405
    );
    let device = f
        .server
        .devices()
        .unwrap()
        .into_iter()
        .find(|device| device.label == "selected media reader")
        .unwrap();
    f.server.kick(&device.id).unwrap();
    assert_eq!(
        f.request("GET", &path, &[("Cookie", &c.content)], &[])
            .status,
        401
    );
}

#[test]
fn installed_packs_listing_obeys_shared_allowlist_and_empty_share() {
    for shared in [vec!["visible"], vec![]] {
        let f = Fixture::with_packs(8, &shared);
        let c = f.pair("library reader");
        assert_eq!(f.request("GET", "/api/packs", &[], &[]).status, 401);
        let response = f.api("/api/packs", &c);
        assert_eq!(response.status, 200);
        let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        let packs = value["packs"].as_array().unwrap();
        assert_eq!(packs.len(), shared.len());
        if !shared.is_empty() {
            assert_eq!(packs[0]["id"], "visible");
            assert!(packs[0]["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()));
            assert!(packs[0]["total"].as_u64().is_some_and(|total| total > 0));
            assert_eq!(packs[0].as_object().unwrap().len(), 3);
        }
    }
}

#[test]
fn session_private_notes_capability_is_false_on_lan() {
    let f = Fixture::new(8);
    let credentials = f.pair("capability check");
    let response = f.api("/api/session", &credentials);
    assert_eq!(response.status, 200);
    let session: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(session["private_notes"], false);
    assert_eq!(session["private_ai"], false);
    assert_eq!(session["private_ai_management"], false);
    assert_eq!(session["private_content"], false);
    assert_eq!(session["private_tv"], false);
    assert_eq!(session["csrf"], credentials.csrf);
}

#[test]
#[ignore = "requires TONALLI_TEST_LAN_BIND naming a real private interface"]
fn real_selected_interface_keeps_https_alive_after_discovery_startup() {
    let bind = std::env::var("TONALLI_TEST_LAN_BIND")
        .expect("set TONALLI_TEST_LAN_BIND to a real private interface address")
        .parse::<std::net::IpAddr>()
        .expect("TONALLI_TEST_LAN_BIND must be a literal IP address");
    assert!(!bind.is_loopback(), "this regression requires actual mDNS");
    let mut f = Fixture::with_packs_on_bind(8, &[], bind);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(f.request("GET", "/pair", &[], &[]).status, 200);
    let credentials = f.pair("real interface regression");
    assert_eq!(f.api("/api/session", &credentials).status, 200);
    f.server.shutdown().unwrap();
}

impl Fixture {
    fn new(cap: usize) -> Self {
        Self::with_packs(cap, &["visible", "map"])
    }

    fn with_packs(cap: usize, shared: &[&str]) -> Self {
        Self::with_packs_on_bind(cap, shared, "127.0.0.1".parse().unwrap())
    }

    fn with_packs_on_bind(cap: usize, shared: &[&str], bind: std::net::IpAddr) -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        fs::create_dir_all(web.join("assets/map")).unwrap();
        fs::write(
            web.join("index.html"),
            "<!doctype html><title>Tonalli</title>",
        )
        .unwrap();
        fs::write(web.join("assets/map/style.json"), "{}").unwrap();
        fs::write(web.join("notes.json"), "PRIVATE SENTINEL").unwrap();
        fs::write(web.join("settings.json"), "PRIVATE SENTINEL").unwrap();
        fs::create_dir_all(web.join("private")).unwrap();
        fs::write(web.join("private/secret.json"), "PRIVATE SENTINEL").unwrap();
        fs::write(web.join("pair"), "UNTRUSTED WEB ROOT PAIR SENTINEL").unwrap();
        let registry = Arc::new(PackRegistry::default());
        let zim =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
        for id in ["visible", "hidden"] {
            registry
                .register_zim(
                    PackId::new(id).unwrap(),
                    Arc::new(Zim::open(Arc::new(PreadSource::open(&zim).unwrap())).unwrap()),
                )
                .unwrap();
        }
        let tiles = root.path().join("map.pmtiles");
        fs::write(
            &tiles,
            pmtiles::archive(
                &pmtiles::directory(&[(0, 1, pmtiles::MVT.len() as u64, 0)]),
                &[],
                pmtiles::MVT,
                1,
                1,
            ),
        )
        .unwrap();
        for id in ["map", "hidden-map"] {
            registry
                .register_tiles(
                    PackId::new(id).unwrap(),
                    Arc::new(PreadSource::open(&tiles).unwrap()),
                )
                .unwrap();
        }
        let cfg = ServerCfg {
            bind,
            max_clients: cap,
            packs: shared.iter().map(|id| PackId::new(*id).unwrap()).collect(),
            web_root: web,
            identity_dir: root.path().join("identity"),
            ..ServerCfg::default()
        };
        let server = Server::start(cfg, Arc::clone(&registry)).unwrap();
        Self {
            server,
            registry,
            _root: root,
        }
    }

    fn connect(&self) -> StreamOwned<ClientConnection, TcpStream> {
        self.tls(TcpStream::connect(self.server.local_addr()).unwrap())
    }

    fn tls(&self, socket: TcpStream) -> StreamOwned<ClientConnection, TcpStream> {
        let mut roots = RootCertStore::empty();
        roots.add(self.server.certificate_der().into()).unwrap();
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let conn = ClientConnection::new(
            Arc::new(config),
            ServerName::IpAddress(self.server.local_addr().ip().into()),
        )
        .unwrap();
        socket.set_nodelay(true).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        StreamOwned::new(conn, socket)
    }

    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut stream = self.connect();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.server.local_addr(),
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
        read_reply(&mut stream)
    }

    fn pair(&self, label: &str) -> Credentials {
        let body = serde_json::to_vec(
            &serde_json::json!({"code": self.server.pairing_code().unwrap(), "label": label}),
        )
        .unwrap();
        let reply = self.request(
            "POST",
            "/pair",
            &[("Content-Type", "application/json")],
            &body,
        );
        assert_eq!(reply.status, 200);
        let cookies: Vec<_> = reply
            .headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(cookies.len(), 2);
        for cookie in &cookies {
            assert!(
                cookie.contains("Secure")
                    && cookie.contains("HttpOnly")
                    && cookie.contains("SameSite=Strict"),
                "{cookie}"
            );
        }
        let content = cookies
            .iter()
            .find(|cookie| cookie.starts_with("tonalli_lan_content="))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let api = cookies
            .iter()
            .find(|cookie| cookie.starts_with("tonalli_lan_api="))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert_ne!(
            content.split_once('=').unwrap().1,
            api.split_once('=').unwrap().1
        );
        let session = self.request(
            "GET",
            "/api/session",
            &[("Cookie", &api), ("Sec-Fetch-Dest", "empty")],
            &[],
        );
        assert_eq!(session.status, 200);
        let csrf = serde_json::from_slice::<serde_json::Value>(&session.body).unwrap()["csrf"]
            .as_str()
            .unwrap()
            .to_owned();
        Credentials { content, api, csrf }
    }

    fn api(&self, path: &str, c: &Credentials) -> Reply {
        self.request(
            "GET",
            path,
            &[
                ("Cookie", &c.api),
                ("Sec-Fetch-Dest", "empty"),
                ("X-Tonalli-Csrf", &c.csrf),
            ],
            &[],
        )
    }
}

fn read_line(stream: &mut impl Read) -> String {
    let mut line = Vec::new();
    while !line.ends_with(b"\r\n") {
        assert!(line.len() < 8192);
        let mut byte = [0_u8];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
    }
    String::from_utf8(line[..line.len() - 2].to_vec()).unwrap()
}

fn read_reply(stream: &mut impl Read) -> Reply {
    let status = read_line(stream)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = Vec::new();
    loop {
        assert!(headers.len() < 128);
        let line = read_line(stream);
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let mut body = Vec::new();
    if headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value == "chunked")
    {
        loop {
            let size =
                usize::from_str_radix(read_line(stream).split(';').next().unwrap(), 16).unwrap();
            if size == 0 {
                while !read_line(stream).is_empty() {}
                break;
            }
            assert!(body.len() + size <= 8 * 1024 * 1024);
            let start = body.len();
            body.resize(start + size, 0);
            stream.read_exact(&mut body[start..]).unwrap();
            assert_eq!(read_line(stream), "");
        }
    } else {
        let size = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map(|(_, value)| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(size <= 8 * 1024 * 1024);
        body.resize(size, 0);
        stream.read_exact(&mut body).unwrap();
    }
    Reply {
        status,
        headers,
        body,
    }
}

#[test]
fn request_without_token_is_401_and_tls_handshake_uses_pinned_certificate() {
    let f = Fixture::new(8);
    assert_eq!(f.request("GET", "/api/maps", &[], &[]).status, 401);
    assert_eq!(f.server.spki_sha256().len(), 64);
    for path in [
        "/zim/visible/C/Home",
        "/packs/map.pmtiles",
        "/assets/map/style.json",
    ] {
        assert_eq!(f.request("GET", path, &[], &[]).status, 401, "{path}");
    }
}

#[test]
fn unpaired_navigation_and_probe_paths_reach_pairing_without_weakening_denials() {
    let f = Fixture::new(8);
    let location = |reply: &Reply| {
        reply
            .headers
            .iter()
            .find(|(name, _)| name == "location")
            .map(|(_, value)| value.clone())
    };
    for path in [
        "/",
        "/generate_204",
        "/gen_204",
        "/hotspot-detect.html",
        "/library/test/success.html",
        "/success.txt",
        "/ncsi.txt",
        "/connecttest.txt",
        "/canonical.html",
    ] {
        let reply = f.request("GET", path, &[], &[]);
        assert_eq!(reply.status, 302, "{path}");
        assert_eq!(location(&reply).as_deref(), Some("/pair"), "{path}");
        assert!(reply.body.is_empty(), "{path}");
    }
    let html = [("Accept", "text/html,application/xhtml+xml")];
    for path in ["/library", "/article", "/map"] {
        let reply = f.request("GET", path, &html, &[]);
        assert_eq!((reply.status, location(&reply).as_deref()), (302, Some("/pair")), "{path}");
    }
    let stale = [
        ("Accept", "text/html"),
        ("Cookie", "tonalli_lan_content=forged"),
    ];
    assert_eq!(f.request("GET", "/library", &stale, &[]).status, 302);
    assert_eq!(f.request("HEAD", "/", &[], &[]).status, 302);

    assert_eq!(f.request("GET", "/api/maps", &html, &[]).status, 401);
    assert_eq!(f.request("POST", "/", &html, &[]).status, 401);
    assert_eq!(f.request("GET", "/zim/visible/C/Home", &[], &[]).status, 401);
    assert_eq!(f.request("GET", "/notes", &html, &[]).status, 404);
    assert_eq!(f.request("GET", "/generate_204?x=1", &[("Cookie", "tonalli_lan_api=forged")], &[]).status, 302);
    assert_eq!(f.request("GET", "/", &[("Host", "connectivitycheck.example")], &[]).status, 403);
    assert_eq!(f.request("GET", "/generate_204", &[("Origin", "https://external.invalid")], &[]).status, 403);

    let paired = f.pair("laptop");
    assert_eq!(f.api("/api/maps", &paired).status, 200);
    let after = f.request("GET", "/generate_204", &[("Cookie", &paired.content)], &[]);
    assert_ne!(after.status, 302);
}

#[test]
fn paired_listing_and_every_lookup_obey_explicit_pack_allowlist() {
    let f = Fixture::new(8);
    let c = f.pair("laptop");
    let maps = f.api("/api/maps", &c);
    assert_eq!(maps.status, 200);
    let value: serde_json::Value = serde_json::from_slice(&maps.body).unwrap();
    assert_eq!(value["packs"].as_array().unwrap().len(), 1);
    assert_eq!(value["packs"][0]["id"], "map");
    assert_eq!(
        f.api("/api/zim/titles?limit=1", &c).status,
        200,
        "sole ZIM selection must exclude unshared registered packs"
    );
    assert_eq!(f.api("/api/zim/titles?pack=hidden&limit=1", &c).status, 404);
    for path in ["/zim/hidden/C/Home", "/packs/hidden-map.pmtiles"] {
        assert_eq!(
            f.request("GET", path, &[("Cookie", &c.content)], &[])
                .status,
            404
        );
    }
}

#[test]
fn hiding_an_item_on_a_running_share_removes_it_for_paired_readers_at_once() {
    use tonalli_core::{media::MediaStore, server::SharedMedia};
    let mut f = Fixture::with_packs(8, &[]);
    let data = f._root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let store = MediaStore::open(&data).unwrap();
    let original = f._root.path().join("live.txt");
    let mut ids = Vec::new();
    for (name, text) in [("Keep.txt", "keep me"), ("Drop.txt", "drop me")] {
        fs::write(&original, text).unwrap();
        let file = store
            .import(Arc::new(PreadSource::open(&original).unwrap()), name, "")
            .unwrap();
        ids.push(file.id);
    }
    let media = Arc::new(SharedMedia::capture(&data, &ids).unwrap());
    f.server.shutdown().unwrap();
    f.server = Server::start(
        ServerCfg {
            web_root: f._root.path().join("web"),
            identity_dir: f._root.path().join("identity"),
            packs: ["visible", "map"]
                .iter()
                .map(|id| PackId::new(*id).unwrap())
                .collect(),
            media: Arc::clone(&media),
            ..ServerCfg::default()
        },
        Arc::clone(&f.registry),
    )
    .unwrap();
    let c = f.pair("live reader");
    let listed = |f: &Fixture| -> Vec<String> {
        let reply = f.api("/api/shared-media", &c);
        assert_eq!(reply.status, 200);
        let rows: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        rows["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["file"]["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let download = |f: &Fixture, id: &str| {
        f.request(
            "GET",
            &format!("/shared-media/{id}/download"),
            &[("Cookie", &c.content)],
            &[],
        )
        .status
    };
    assert_eq!(listed(&f), ids);
    assert_eq!(download(&f, &ids[1]), 200);

    media.hide(&[], &[ids[1].clone()]).unwrap();
    assert_eq!(listed(&f), vec![ids[0].clone()]);
    assert_eq!(download(&f, &ids[1]), 404);
    assert_eq!(
        f.api(&format!("/api/shared-media/{}/text", ids[1]), &c)
            .status,
        404
    );
    assert_eq!(download(&f, &ids[0]), 200, "other files stay shared");

    let maps = |f: &Fixture| {
        let reply = f.api("/api/maps", &c);
        let value: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        value["packs"].as_array().unwrap().len()
    };
    assert_eq!(maps(&f), 1);
    assert_eq!(
        f.api("/api/zim/titles?pack=visible&limit=1", &c).status,
        200
    );
    media.hide(&["map".into(), "visible".into()], &[]).unwrap();
    assert_eq!(maps(&f), 0);
    assert_eq!(
        f.api("/api/zim/titles?pack=visible&limit=1", &c).status,
        404
    );
    assert_eq!(
        f.request("GET", "/zim/visible/C/Home", &[("Cookie", &c.content)], &[])
            .status,
        404
    );

    // Hiding is removal only: repeating it or naming unknown items changes nothing else.
    media.hide(&["nothing".into()], &["nothing".into()]).unwrap();
    assert_eq!(listed(&f), vec![ids[0].clone()]);
    assert!(media.hide(&[], &vec!["x".to_owned(); 1025]).is_err());
}

#[test]
fn app_update_controls_stay_private_to_the_device_even_after_pairing() {
    let f = Fixture::new(8);
    let c = f.pair("laptop");
    for (method, path) in [
        ("GET", "/api/update"),
        ("PUT", "/api/update/settings"),
        ("POST", "/api/update/check"),
        ("POST", "/api/update/download"),
        ("POST", "/api/update/cancel"),
        ("GET", "/%61pi/update"),
    ] {
        assert_eq!(
            f.request(
                method,
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "{method} {path}"
        );
    }
}

#[test]
fn notes_settings_and_arbitrary_paths_stay_404_even_after_pairing() {
    let f = Fixture::new(8);
    let c = f.pair("laptop");
    for (method, path) in [
        ("POST", "/api/content/jobs"),
        ("POST", "/api/content/preflight"),
        (
            "POST",
            "/api/content/jobs/00000000000000000000000000000000/pause",
        ),
        (
            "POST",
            "/api/content/jobs/00000000000000000000000000000000/resume",
        ),
        (
            "POST",
            "/api/content/jobs/00000000000000000000000000000000/cancel",
        ),
        (
            "POST",
            "/api/content/jobs/00000000000000000000000000000000/retry",
        ),
        ("DELETE", "/api/content/installed/test-en"),
        ("POST", "/%61pi/content/jobs"),
        ("POST", "/api/%63ontent/preflight"),
        ("DELETE", "/api/content/%69nstalled/test-en"),
    ] {
        assert_eq!(
            f.request(
                method,
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "{method} {path}"
        );
    }
    for path in [
        "/api/notes",
        "/api/notes/unlock",
        "/api/notes/create",
        "/api/notes/recovery",
        "/api/map-management",
        "/api/sharing",
        "/api/sharing/start",
        "/api/sharing/stop",
        "/api/sharing/devices/anything/disconnect",
        "/api/tv",
        "/api/tv/start",
        "/api/tv/stop",
        "/%61pi/tv",
        "/%61pi/sharing",
        "/api/map-management/mx/pause",
        "/%61pi/map-management",
        "/api/notes/passphrase",
        "/api/settings",
        "/api/admin",
        "/api/admin/login",
        "/api/preferences",
        "/%61pi/%61dmin",
        "/api/index",
        "/api/ai",
        "/api/ai/chat",
        "/api/ai/models",
        "/api/ai/models/load",
        "/api/ai/models/off",
        "/api/content/catalog",
        "/api/content/jobs",
        "/%61pi/content/catalog",
        "/%61pi/%61i/cancel",
        "/API/NOTES",
        "/%61pi/%6eotes",
    ] {
        assert_eq!(f.api(path, &c).status, 404, "{path}");
    }
    for path in [
        "/notes",
        "/content",
        "/%63ontent",
        "/chat",
        "/CHAT",
        "/%63hat",
        "/chat.json",
        "/notes.json",
        "/NoTeS",
        "/NOTES.JSON",
        "/%6eotes",
        "/%6eotes.json",
        "/notes/unlock",
        "/settings",
        "/admin",
        "/preferences",
        "/settings.json",
        "/SeTtInGs.JsOn",
        "/%73ettings.json",
        "/private/secret.json",
        "/PRIVATE/secret.json",
        "/%70rivate/secret.json",
        "/private%2fsecret.json",
        "/identity/server-key.der",
        "/%2e%2e/identity/server-key.der",
        "/C:/Windows/win.ini",
    ] {
        assert_eq!(
            f.request("GET", path, &[("Cookie", &c.content)], &[])
                .status,
            404,
            "{path}"
        );
    }
    for path in ["/api/notes/unlock", "/api/notes"] {
        assert_eq!(
            f.request(
                "POST",
                path,
                &[
                    ("Cookie", &c.api),
                    ("Sec-Fetch-Dest", "empty"),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Content-Type", "application/json")
                ],
                br#"{"passphrase":"private","title":"must not be written","body":"private"}"#
            )
            .status,
            404
        );
    }
}

#[cfg(feature = "ai")]
#[test]
fn private_ai_history_never_enters_the_paired_lan_router() {
    let f = Fixture::new(8);
    let c = f.pair("reader");
    for (method, path) in [
        ("GET", "/api/ai/history"),
        ("GET", "/api/ai/history/0123456789abcdef0123456789abcdef"),
        ("PUT", "/api/ai/history/0123456789abcdef0123456789abcdef"),
        ("DELETE", "/api/ai/history/0123456789abcdef0123456789abcdef"),
    ] {
        assert_eq!(
            f.request(
                method,
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "{method} {path}"
        );
    }
}

#[test]
fn managed_media_controls_never_enter_the_paired_reader_router() {
    let f = Fixture::new(8);
    let c = f.pair("media reader");
    for path in [
        "/api/files",
        "/api/files/upload",
        "/api/files/native-import",
        "/api/files/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/download",
        "/%61pi/%66iles",
    ] {
        assert_eq!(f.api(path, &c).status, 404, "{path}");
        assert_eq!(
            f.request(
                "POST",
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "POST {path}"
        );
    }
    for path in [
        "/files",
        "/%66iles",
        "/native/export-media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "/native/import-media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert_eq!(
            f.request("GET", path, &[("Cookie", &c.content)], &[])
                .status,
            404,
            "{path}"
        );
    }
}

#[test]
fn music_and_video_playback_is_not_a_lan_library_endpoint() {
    let f = Fixture::new(8);
    let c = f.pair("music isolation");
    for path in [
        "/api/music",
        "/api/music/stream",
        "/api/video/stream",
        "/api/video/poster",
        "/api/video/progress",
        "/api/documents/stream",
        "/api/documents/import",
        "/api/documents/content/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "/reader",
        "/cookbook",
        "/workshop",
        "/%77orkshop",
        "/api/workshop/model",
        "/Cookbook",
        "/%63ookbook",
        "/book/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/one.xhtml",
        "/%72eader",
        "/api/%64ocuments/stream",
        "/%61pi/%76ideo/stream",
        "/videos",
        "/%61pi/%6dusic",
        "/media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "/music",
    ] {
        assert_eq!(f.api(path, &c).status, 404, "{path}");
        assert_eq!(
            f.request(
                "POST",
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "POST {path}"
        );
    }
}

#[test]
fn reading_history_and_bookmarks_never_enter_lan_router() {
    let f = Fixture::new(8);
    let c = f.pair("reading privacy");
    for path in [
        "/api/reading",
        "/api/reading/bookmark",
        "/%61pi/%72eading",
        "/reading",
    ] {
        assert_eq!(f.api(path, &c).status, 404, "{path}");
        assert_eq!(
            f.request(
                "POST",
                path,
                &[
                    ("Cookie", &c.api),
                    ("X-Tonalli-Csrf", &c.csrf),
                    ("Sec-Fetch-Dest", "empty")
                ],
                &[]
            )
            .status,
            404,
            "POST {path}"
        );
    }
}

#[test]
fn lan_map_assets_ranges_and_api_guards_need_separate_paired_credentials() {
    let f = Fixture::new(8);
    let c = f.pair("laptop");
    assert_eq!(
        f.request(
            "GET",
            "/assets/map/style.json",
            &[("Cookie", &c.content)],
            &[]
        )
        .status,
        200
    );
    let range = f.request(
        "GET",
        "/packs/map.pmtiles",
        &[("Cookie", &c.content), ("Range", "bytes=0-7")],
        &[],
    );
    assert_eq!(range.status, 206);
    assert_eq!(range.body, b"PMTiles\x03");
    assert_eq!(
        f.request(
            "GET",
            "/api/maps",
            &[
                ("Cookie", &c.content),
                ("Sec-Fetch-Dest", "empty"),
                ("X-Tonalli-Csrf", &c.csrf)
            ],
            &[]
        )
        .status,
        401
    );
    assert_eq!(
        f.request(
            "GET",
            "/api/maps",
            &[
                ("Cookie", &c.api),
                ("Sec-Fetch-Dest", "image"),
                ("X-Tonalli-Csrf", &c.csrf)
            ],
            &[]
        )
        .status,
        403
    );
    assert_eq!(
        f.request(
            "GET",
            "/api/maps",
            &[("Cookie", &c.api), ("Sec-Fetch-Dest", "empty")],
            &[]
        )
        .status,
        403
    );
}

#[test]
fn client_cap_counts_devices_and_kick_revokes_the_next_range_and_api_request() {
    let f = Fixture::new(1);
    let c = f.pair("first laptop");
    let body = serde_json::to_vec(
        &serde_json::json!({"code":f.server.pairing_code().unwrap(),"label":"second laptop"}),
    )
    .unwrap();
    assert_eq!(
        f.request(
            "POST",
            "/pair",
            &[("Content-Type", "application/json")],
            &body
        )
        .status,
        409
    );
    let devices = f.server.devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].label, "first laptop");
    assert_eq!(devices[0].peer, f.server.local_addr().ip());
    f.server.kick(&devices[0].id).unwrap();
    assert!(f.server.devices().unwrap().is_empty());
    assert_eq!(f.api("/api/maps", &c).status, 401);
    assert_eq!(
        f.request(
            "GET",
            "/packs/map.pmtiles",
            &[("Cookie", &c.content), ("Range", "bytes=0-7")],
            &[]
        )
        .status,
        401
    );
    assert_eq!(
        f.request(
            "GET",
            "/assets/map/style.json",
            &[("Cookie", &c.content)],
            &[]
        )
        .status,
        401
    );
    let _replacement = f.pair("replacement laptop");
}

#[test]
fn overlapping_identity_and_served_roots_are_rejected_without_starting() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir_all(&web).unwrap();
    fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let cfg = ServerCfg {
        web_root: web.clone(),
        identity_dir: web.join("identity"),
        ..ServerCfg::default()
    };
    assert!(matches!(
        Server::start(cfg, Arc::new(PackRegistry::default())),
        Err(PackError::Mismatch(_))
    ));
    let cfg = ServerCfg {
        web_root: web,
        identity_dir: root.path().to_owned(),
        ..ServerCfg::default()
    };
    assert!(matches!(
        Server::start(cfg, Arc::new(PackRegistry::default())),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn pairing_code_brute_force_locks_out_the_actual_peer_despite_forwarded_headers() {
    let f = Fixture::new(8);
    let code = f.server.pairing_code().unwrap();
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for attempt in 0..5 {
        let body =
            serde_json::to_vec(&serde_json::json!({"code":wrong,"label":"incorrect"})).unwrap();
        let fake = format!("10.0.0.{}", attempt + 1);
        let reply = f.request(
            "POST",
            "/pair",
            &[
                ("Content-Type", "application/json"),
                ("X-Forwarded-For", &fake),
            ],
            &body,
        );
        assert!(
            matches!(reply.status, 401 | 429),
            "unexpected status {}",
            reply.status
        );
    }
    let body =
        serde_json::to_vec(&serde_json::json!({"code":code,"label":"correct but locked"})).unwrap();
    assert_eq!(
        f.request(
            "POST",
            "/pair",
            &[
                ("Content-Type", "application/json"),
                ("X-Forwarded-For", "10.255.255.254")
            ],
            &body
        )
        .status,
        429
    );
    assert!(f.server.devices().unwrap().is_empty());
}

#[test]
fn untrusted_certificate_is_rejected_instead_of_accepting_any_server() {
    let f = Fixture::new(8);
    let config = ClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let conn = ClientConnection::new(
        Arc::new(config),
        ServerName::IpAddress(f.server.local_addr().ip().into()),
    )
    .unwrap();
    let socket = TcpStream::connect(f.server.local_addr()).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut stream = StreamOwned::new(conn, socket);
    let result = stream.write_all(b"GET /pair HTTP/1.1\r\nConnection: close\r\n\r\n");
    if result.is_ok() {
        let mut buffer = [0_u8; 1];
        assert!(stream.read(&mut buffer).is_err());
    }
}

#[test]
fn pair_page_is_trusted_server_content_and_rejects_cross_site_pairing() {
    let f = Fixture::new(8);
    let page = f.request("GET", "/pair", &[], &[]);
    assert_eq!(page.status, 200);
    assert!(!String::from_utf8_lossy(&page.body).contains("UNTRUSTED WEB ROOT PAIR SENTINEL"));
    assert!(page
        .headers
        .iter()
        .any(|(name, value)| name == "content-security-policy"
            && value.contains("frame-ancestors 'none'")));
    let body = serde_json::to_vec(
        &serde_json::json!({"code":f.server.pairing_code().unwrap(),"label":"cross-site"}),
    )
    .unwrap();
    assert_eq!(
        f.request(
            "POST",
            "/pair",
            &[
                ("Content-Type", "application/json"),
                ("Origin", "https://external.invalid"),
                ("Sec-Fetch-Site", "cross-site")
            ],
            &body
        )
        .status,
        403
    );
    assert!(f.server.devices().unwrap().is_empty());
}

#[test]
fn loopback_cookie_namespace_is_never_accepted_by_the_lan_listener() {
    let f = Fixture::new(8);
    let c = f.pair("namespace check");
    let content = c
        .content
        .replacen("tonalli_lan_content=", "tonalli_content=", 1);
    let api = c.api.replacen("tonalli_lan_api=", "tonalli_api=", 1);
    assert_ne!(content, c.content);
    assert_ne!(api, c.api);
    assert_eq!(
        f.request(
            "GET",
            "/assets/map/style.json",
            &[("Cookie", &content)],
            &[]
        )
        .status,
        401
    );
    assert_eq!(
        f.request(
            "GET",
            "/api/maps",
            &[
                ("Cookie", &api),
                ("Sec-Fetch-Dest", "empty"),
                ("X-Tonalli-Csrf", &c.csrf)
            ],
            &[]
        )
        .status,
        401
    );
}

struct ReadGate {
    entered: mpsc::Sender<()>,
    released: Mutex<bool>,
    changed: Condvar,
}

struct GatedSource {
    source: PreadSource,
    gate: Arc<ReadGate>,
}

impl PackSource for GatedSource {
    fn len(&self) -> u64 {
        self.source.len()
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if offset >= 64 * 1024 {
            let _ = self.gate.entered.send(());
            let mut released = self.gate.released.lock().unwrap();
            while !*released {
                released = self.gate.changed.wait(released).unwrap();
            }
        }
        self.source.read_at(offset, output)
    }
}

struct ReleaseGate(Arc<ReadGate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.changed.notify_all();
    }
}

#[test]
fn kick_cancels_an_active_range_without_waiting_for_the_next_disk_read() {
    active_range_cancellation(StopAction::Kick);
}

#[test]
fn shutdown_cancels_an_active_range_without_waiting_for_the_next_disk_read() {
    active_range_cancellation(StopAction::Shutdown);
}

#[test]
fn drop_cancels_an_active_range_without_waiting_for_the_next_disk_read() {
    active_range_cancellation(StopAction::Drop);
}

enum StopAction {
    Kick,
    Shutdown,
    Drop,
}

fn active_range_cancellation(action: StopAction) {
    let mut f = Fixture::new(8);
    let c = f.pair("active reader");
    let path = f._root.path().join("map.pmtiles");
    let mut bytes = fs::read(&path).unwrap();
    bytes.resize(200_003, 0);
    fs::write(&path, bytes).unwrap();
    let (entered, observed) = mpsc::channel();
    let gate = Arc::new(ReadGate {
        entered,
        released: Mutex::new(false),
        changed: Condvar::new(),
    });
    let _release = ReleaseGate(Arc::clone(&gate));
    f.registry.remove(&PackId::new("map").unwrap()).unwrap();
    f.registry
        .register_tiles(
            PackId::new("map").unwrap(),
            Arc::new(GatedSource {
                source: PreadSource::open(&path).unwrap(),
                gate,
            }),
        )
        .unwrap();
    let mut stream = f.connect();
    write!(stream, "GET /packs/map.pmtiles HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nRange: bytes=0-200002\r\nConnection: close\r\n\r\n", f.server.local_addr(), c.content).unwrap();
    stream.flush().unwrap();
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        assert!(headers.len() < 65536);
        let mut byte = [0_u8];
        stream.read_exact(&mut byte).unwrap();
        headers.push(byte[0]);
    }
    assert!(headers.starts_with(b"HTTP/1.1 206"));
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    let devices = f.server.devices().unwrap();
    let address = f.server.local_addr();
    let started = std::time::Instant::now();
    let closes_listener = !matches!(&action, StopAction::Kick);
    match action {
        StopAction::Kick => f.server.kick(&devices[0].id).unwrap(),
        StopAction::Shutdown => {
            f.server.shutdown().unwrap();
            f.server.shutdown().unwrap();
        }
        StopAction::Drop => {
            drop(f.server);
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "revocation/shutdown waited for blocked source I/O"
    );
    stream
        .sock
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut body = Vec::new();
    if let Err(error) = stream.read_to_end(&mut body) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof,
            "kick failed to cancel blocked range: {error}"
        );
    }
    assert!(
        body.len() <= 64 * 1024,
        "revoked stream continued delivering chunks"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "revocation did not close the active response within its bound"
    );
    // A refused loopback connect costs about two seconds on this Windows host.
    if closes_listener {
        assert!(TcpStream::connect(address).is_err());
    }
}

#[test]
fn slow_unpaired_handshakes_do_not_serialize_later_tls_connections() {
    let f = Fixture::new(8);
    let stalled: Vec<_> = (0..16)
        .map(|_| TcpStream::connect(f.server.local_addr()).unwrap())
        .collect();
    let started = std::time::Instant::now();
    assert_eq!(f.request("GET", "/api/maps", &[], &[]).status, 401);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "valid TLS connection waited behind an unpaired stalled handshake"
    );
    drop(stalled);
}

#[test]
fn startup_requires_explicit_resource_paths_and_bounded_client_cap() {
    assert!(matches!(
        Server::start(ServerCfg::default(), Arc::new(PackRegistry::default())),
        Err(PackError::Mismatch(_))
    ));
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    fs::create_dir_all(&web).unwrap();
    fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    for cap in [0, 129] {
        let cfg = ServerCfg {
            max_clients: cap,
            web_root: web.clone(),
            identity_dir: root.path().join("identity"),
            ..ServerCfg::default()
        };
        assert!(matches!(
            Server::start(cfg, Arc::new(PackRegistry::default())),
            Err(PackError::Mismatch(_))
        ));
    }
}

#[test]
fn paired_articles_images_and_redirects_keep_the_no_script_content_policy() {
    let f = Fixture::new(8);
    let c = f.pair("article reader");
    for path in [
        "/zim/visible/C/en/index.html",
        "/zim/visible/C/assets/pixel.png",
        "/zim/visible/C/redirect/start.html",
        "/zim/visible/C/missing",
    ] {
        let reply = f.request("GET", path, &[("Cookie", &c.content)], &[]);
        assert!(
            reply
                .headers
                .iter()
                .any(|(name, value)| name == "content-security-policy"
                    && value.contains("script-src 'none'")
                    && value.contains("frame-ancestors 'self'")),
            "{path}"
        );
        assert!(reply
            .headers
            .iter()
            .any(|(name, value)| name == "x-content-type-options" && value == "nosniff"));
        assert_eq!(
            reply.status,
            if path.ends_with("/missing") {
                404
            } else if path.contains("/redirect/") {
                302
            } else {
                200
            }
        );
    }
    let id = f.server.devices().unwrap()[0].id.clone();
    f.server.kick(&id).unwrap();
    assert_eq!(
        f.request(
            "GET",
            "/zim/visible/C/assets/pixel.png",
            &[("Cookie", &c.content)],
            &[]
        )
        .status,
        401
    );
}

#[test]
fn empty_allowlist_shares_nothing_even_with_a_populated_registry() {
    let f = Fixture::with_packs(8, &[]);
    let c = f.pair("empty share");
    let maps = f.api("/api/maps", &c);
    assert_eq!(maps.status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&maps.body).unwrap()["packs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        f.api("/api/zim/titles?pack=visible&limit=1", &c).status,
        404
    );
    assert_ne!(f.api("/api/zim/titles?limit=1", &c).status, 200);
    for path in ["/zim/visible/C/en/index.html", "/packs/map.pmtiles"] {
        assert_eq!(
            f.request("GET", path, &[("Cookie", &c.content)], &[])
                .status,
            404
        );
    }
}

#[test]
fn kick_is_rechecked_on_the_same_tls_keepalive_connection() {
    let f = Fixture::new(8);
    let c = f.pair("persistent connection");
    let mut stream = f.connect();
    let request = format!(
        "GET /packs/map.pmtiles HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nRange: bytes=0-7\r\n\r\n",
        f.server.local_addr(),
        c.content
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();
    assert_eq!(read_reply(&mut stream).status, 206);
    let id = f.server.devices().unwrap()[0].id.clone();
    f.server.kick(&id).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();
    assert_eq!(read_reply(&mut stream).status, 401);
}

#[test]
fn http_request_burst_hits_the_real_peer_limiter() {
    let f = Fixture::new(8);
    let c = f.pair("burst reader");
    let mut stream = f.connect();
    let request = format!(
        "GET /api/session HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nSec-Fetch-Dest: empty\r\n\r\n",
        f.server.local_addr(),
        c.api
    );
    let batch = request.repeat(100);
    stream.write_all(batch.as_bytes()).unwrap();
    stream.flush().unwrap();
    let replies: Vec<_> = (0..100).map(|_| read_reply(&mut stream).status).collect();
    assert!(
        replies.contains(&429),
        "HTTP route was not wired through the bounded peer limiter"
    );
    assert!(replies.iter().all(|status| matches!(status, 200 | 429)));
}

#[test]
fn shutdown_is_idempotent_and_drop_closes_the_owned_listener() {
    let mut f = Fixture::new(8);
    let address = f.server.local_addr();
    f.server.shutdown().unwrap();
    f.server.shutdown().unwrap();
    assert!(TcpStream::connect(address).is_err());
    let f = Fixture::new(8);
    let address = f.server.local_addr();
    drop(f.server);
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn reported_pin_hashes_certificate_spki_and_another_valid_identity_is_rejected() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new(8);
    let certificate: rustls::pki_types::CertificateDer<'static> = f.server.certificate_der().into();
    let parsed = rustls::server::ParsedCertificate::try_from(&certificate).unwrap();
    assert_eq!(
        f.server.spki_sha256(),
        format!(
            "{:x}",
            Sha256::digest(parsed.subject_public_key_info().as_ref())
        )
    );
    let other = Fixture::new(8);
    assert_ne!(f.server.spki_sha256(), other.server.spki_sha256());
    let mut roots = RootCertStore::empty();
    roots.add(other.server.certificate_der().into()).unwrap();
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let conn = ClientConnection::new(
        Arc::new(config),
        ServerName::IpAddress(f.server.local_addr().ip().into()),
    )
    .unwrap();
    let socket = TcpStream::connect(f.server.local_addr()).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut stream = StreamOwned::new(conn, socket);
    if stream.write_all(b"GET /pair HTTP/1.1\r\n\r\n").is_ok() {
        assert!(
            stream.read(&mut [0_u8; 1]).is_err(),
            "TLS accepted a different valid pinned identity"
        );
    }
}

struct BackpressureSource {
    header: Vec<u8>,
    first: std::sync::atomic::AtomicUsize,
    bytes: std::sync::atomic::AtomicUsize,
}

impl PackSource for BackpressureSource {
    fn len(&self) -> u64 {
        8 * 1024 * 1024
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        use std::sync::atomic::Ordering;
        if offset > self.len() {
            return Err(PackError::Truncated);
        }
        let count = output.len().min((self.len() - offset) as usize);
        output[..count].fill(0);
        let start = offset as usize;
        if start < self.header.len() {
            let available = count.min(self.header.len() - start);
            output[..available].copy_from_slice(&self.header[start..start + available]);
        }
        if offset == 0 && count == 64 * 1024 {
            self.first.fetch_add(1, Ordering::SeqCst);
        }
        self.bytes.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

#[test]
fn kick_releases_payload_slots_when_all_old_clients_stop_reading_socket_data() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new(8);
    let old = f.pair("stalled receiver");
    let source = Arc::new(BackpressureSource {
        header: pmtiles::archive(
            &pmtiles::directory(&[(0, 1, pmtiles::MVT.len() as u64, 0)]),
            &[],
            pmtiles::MVT,
            1,
            1,
        ),
        first: AtomicUsize::new(0),
        bytes: AtomicUsize::new(0),
    });
    f.registry.remove(&PackId::new("map").unwrap()).unwrap();
    f.registry
        .register_tiles(PackId::new("map").unwrap(), source.clone())
        .unwrap();
    source.bytes.store(0, Ordering::SeqCst);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut stalled = Vec::new();
    for _ in 0..4 {
        let socket = runtime.block_on(async {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.set_recv_buffer_size(4096).unwrap();
            socket
                .connect(f.server.local_addr())
                .await
                .unwrap()
                .into_std()
                .unwrap()
        });
        socket.set_nonblocking(false).unwrap();
        let mut tls = f.tls(socket);
        write!(
            tls,
            "GET /packs/map.pmtiles HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\n\r\n",
            f.server.local_addr(),
            old.content
        )
        .unwrap();
        tls.flush().unwrap();
        stalled.push(tls);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while source.first.load(Ordering::SeqCst) < 4 {
        assert!(
            std::time::Instant::now() < deadline,
            "four payload slots did not become active"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    let mut previous = source.bytes.load(Ordering::SeqCst);
    let mut unchanged = std::time::Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(10));
        let current = source.bytes.load(Ordering::SeqCst);
        if current != previous {
            previous = current;
            unchanged = std::time::Instant::now();
        }
        if unchanged.elapsed() >= Duration::from_millis(100) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "small receive windows did not create bounded socket backpressure"
        );
    }
    assert!(
        previous < 4 * source.len() as usize,
        "responses completed instead of stalling"
    );
    let id = f.server.devices().unwrap()[0].id.clone();
    f.server.kick(&id).unwrap();
    let fresh = f.pair("fresh reader");
    let started = std::time::Instant::now();
    assert_eq!(f.api("/api/maps", &fresh).status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "kicked socket writes kept all payload permits"
    );
    assert_eq!(f.api("/api/maps", &old).status, 401);
    drop(stalled);
}
