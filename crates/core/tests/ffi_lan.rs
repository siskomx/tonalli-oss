//! Kotlin-facing LAN ownership, lifecycle and explicit pairing-entry contract.
#![cfg(feature = "ffi")]

use std::{path::Path, sync::Arc, time::Duration};

use reqwest::{blocking::Client, header, StatusCode};
use tonalli_core::{ffi::*, pack::PackError};

fn config(root: &Path) -> LanConfig {
    LanConfig {
        bind_address: "127.0.0.1".into(),
        port: 0,
        max_clients: 8,
        pack_ids: vec!["visible".into()],
        web_root: root.join("web").to_string_lossy().into_owned(),
        identity_dir: root.join("lan-identity").to_string_lossy().into_owned(),
    }
}

fn source() -> (tempfile::TempDir, Arc<LoopbackHandle>) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("web")).unwrap();
    std::fs::write(
        root.path().join("web/index.html"),
        "<!doctype html><title>LAN FFI fixture</title>",
    )
    .unwrap();
    let private = server_start_loopback(
        root.path().join("private").to_string_lossy().into_owned(),
        root.path().join("web").to_string_lossy().into_owned(),
    )
    .unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    for id in ["visible", "hidden"] {
        let pack = pack_open_internal(fixture.to_string_lossy().into_owned()).unwrap();
        server_register_zim(Arc::clone(&private), id.into(), pack).unwrap();
    }
    (root, private)
}

fn client(info: &LanInfo) -> Client {
    Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(&info.certificate_der).unwrap())
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[test]
fn advertised_hostname_has_matching_tls_and_http_authority() {
    let (root, private) = source();
    let lan = server_start_lan(private, config(root.path())).unwrap();
    let info = lan_info(Arc::clone(&lan)).unwrap();
    let hostname = "tonalli.local".to_string();
    let client = Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(&info.certificate_der).unwrap())
        .resolve(
            &hostname,
            format!("127.0.0.1:{}", info.port).parse().unwrap(),
        )
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("https://{hostname}:{}/pair", info.port);
    assert_eq!(client.get(&url).send().unwrap().status(), StatusCode::OK);
    assert_eq!(
        client
            .get(&url)
            .header(header::ORIGIN, "https://unrelated.local")
            .send()
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    server_stop_lan(lan).unwrap();
}

fn pair(client: &Client, lan: &Arc<LanHandle>, info: &LanInfo) -> String {
    let response = client.post(&info.url)
        .json(&serde_json::json!({"code":lan_pairing_code(Arc::clone(lan)).unwrap(),"label":"FFI laptop"}))
        .send().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    let content = cookies
        .iter()
        .find(|cookie| cookie.starts_with("tonalli_lan_content="))
        .unwrap();
    assert!(
        content.contains("Secure")
            && content.contains("HttpOnly")
            && content.contains("SameSite=Strict")
    );
    content.split(';').next().unwrap().to_owned()
}

#[test]
fn lan_keeps_only_registry_and_reads_after_private_stop_and_source_drop() {
    let (root, private) = source();
    let source_weak = Arc::downgrade(&private);
    let lan = server_start_lan(Arc::clone(&private), config(root.path())).unwrap();
    let info = lan_info(Arc::clone(&lan)).unwrap();
    let client = client(&info);
    let cookie = pair(&client, &lan, &info);

    server_stop_loopback(Arc::clone(&private)).unwrap();
    assert!(matches!(
        server_info(Arc::clone(&private)),
        Err(PackError::NotFound(_))
    ));
    drop(private);
    assert!(
        source_weak.upgrade().is_none(),
        "LAN must not retain private UI/vault ownership"
    );

    let origin = info.url.strip_suffix("/pair").unwrap();
    let article = client
        .get(format!("{origin}/zim/visible/C/en/index.html"))
        .header(header::COOKIE, &cookie)
        .send()
        .unwrap();
    assert_eq!(article.status(), StatusCode::OK);
    assert!(!article.bytes().unwrap().is_empty());
    assert_eq!(
        client
            .get(format!("{origin}/zim/hidden/C/en/index.html"))
            .header(header::COOKIE, &cookie)
            .send()
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    for path in ["/api/notes", "/api/settings", "/notes", "/settings"] {
        assert_eq!(
            client
                .get(format!("{origin}{path}"))
                .header(header::COOKIE, &cookie)
                .send()
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
            "private route {path}"
        );
    }
    server_stop_lan(lan).unwrap();
}

#[test]
fn lan_can_start_from_an_already_stopped_private_host_registry() {
    let (root, private) = source();
    server_stop_loopback(Arc::clone(&private)).unwrap();
    let source_weak = Arc::downgrade(&private);
    let lan = server_start_lan(Arc::clone(&private), config(root.path())).unwrap();
    drop(private);
    assert!(source_weak.upgrade().is_none());
    assert!(lan_info(Arc::clone(&lan))
        .unwrap()
        .url
        .starts_with("https://127.0.0.1:"));
    server_stop_lan(lan).unwrap();
}

#[test]
fn lan_info_is_public_only_and_aliases_stop_idempotently() {
    let (root, private) = source();
    let lan = server_start_lan(private, config(root.path())).unwrap();
    let alias = Arc::clone(&lan);
    // Exhaustive destructuring intentionally rejects accidental private fields.
    let LanInfo {
        address,
        port,
        url,
        certificate_der,
        spki_sha256,
        certificate_sha256,
        friendly_url,
    } = lan_info(Arc::clone(&lan)).unwrap();
    assert!(friendly_url.starts_with("https://tonalli-"));
    use sha2::Digest;
    assert_eq!(
        certificate_sha256,
        sha2::Sha256::digest(&certificate_der)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    );
    assert_eq!(address, "127.0.0.1");
    assert_ne!(port, 0);
    assert_eq!(url, format!("https://127.0.0.1:{port}/pair"));
    assert!(!certificate_der.is_empty());
    assert_eq!(spki_sha256.len(), 64);
    assert!(spki_sha256
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    assert!(!lan_pairing_code(Arc::clone(&lan)).unwrap().is_empty());
    assert!(lan_devices(Arc::clone(&lan)).unwrap().is_empty());
    server_stop_lan(lan).unwrap();
    server_stop_lan(Arc::clone(&alias)).unwrap();
    assert!(matches!(
        lan_info(Arc::clone(&alias)),
        Err(PackError::NotFound(_))
    ));
    assert!(matches!(
        lan_pairing_code(Arc::clone(&alias)),
        Err(PackError::NotFound(_))
    ));
    assert!(matches!(
        lan_devices(Arc::clone(&alias)),
        Err(PackError::NotFound(_))
    ));
    assert!(matches!(
        lan_kick(alias, "missing".into()),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn lan_devices_and_kick_use_the_same_registry_backed_server() {
    let (root, private) = source();
    let lan = server_start_lan(private, config(root.path())).unwrap();
    let info = lan_info(Arc::clone(&lan)).unwrap();
    let client = client(&info);
    let cookie = pair(&client, &lan, &info);
    let devices = lan_devices(Arc::clone(&lan)).unwrap();
    assert_eq!(devices.len(), 1);
    let LanDevice {
        id,
        label,
        peer_address,
    } = &devices[0];
    assert_eq!(label, "FFI laptop");
    assert_eq!(peer_address, "127.0.0.1");
    assert!(!id.is_empty());
    assert!(matches!(
        lan_kick(Arc::clone(&lan), "missing".into()),
        Err(PackError::NotFound(_))
    ));
    lan_kick(Arc::clone(&lan), id.clone()).unwrap();
    assert!(lan_devices(Arc::clone(&lan)).unwrap().is_empty());
    assert_eq!(
        client
            .get(format!(
                "{}/zim/visible/C/en/index.html",
                info.url.strip_suffix("/pair").unwrap()
            ))
            .header(header::COOKIE, cookie)
            .send()
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    server_stop_lan(lan).unwrap();
}

#[test]
fn lan_config_rejects_unsafe_binds_bad_caps_ids_and_resource_paths() {
    let (root, private) = source();
    for bind in [
        "invalid",
        "0.0.0.0",
        "::",
        "8.8.8.8",
        "100.64.0.1",
        "::ffff:127.0.0.1",
    ] {
        let mut cfg = config(root.path());
        cfg.bind_address = bind.into();
        assert!(
            matches!(
                server_start_lan(Arc::clone(&private), cfg),
                Err(PackError::Mismatch(_))
            ),
            "bind {bind}"
        );
    }
    for cap in [0, 129, u32::MAX] {
        let mut cfg = config(root.path());
        cfg.max_clients = cap;
        assert!(matches!(
            server_start_lan(Arc::clone(&private), cfg),
            Err(PackError::Mismatch(_))
        ));
    }
    for id in ["../escape", "not-installed"] {
        let mut cfg = config(root.path());
        cfg.pack_ids = vec![id.into()];
        assert!(matches!(
            server_start_lan(Arc::clone(&private), cfg),
            Err(PackError::Mismatch(_))
        ));
    }
    let mut cfg = config(root.path());
    cfg.web_root.clear();
    assert!(matches!(
        server_start_lan(Arc::clone(&private), cfg),
        Err(PackError::Mismatch(_))
    ));
    let mut cfg = config(root.path());
    cfg.identity_dir = cfg.web_root.clone();
    assert!(matches!(
        server_start_lan(private, cfg),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn lan_exports_are_panic_guarded_and_do_not_consult_ai_ram_policy() {
    let source = include_str!("../src/ffi/lan.rs");
    for name in [
        "server_start_lan",
        "lan_info",
        "lan_pairing_code",
        "lan_devices",
        "lan_kick",
        "server_stop_lan",
    ] {
        let signature = format!("pub fn {name}(");
        let tail = source
            .split_once(&signature)
            .unwrap_or_else(|| panic!("missing export {name}"))
            .1;
        let function = tail.split("#[uniffi::export]").next().unwrap();
        assert!(
            function.contains("pack_boundary(||"),
            "missing panic boundary: {name}"
        );
        for forbidden in ["tier_for", "ram_tier", "RamTier"] {
            assert!(
                !function.contains(forbidden),
                "{name} must not read {forbidden}"
            );
        }
    }
}
