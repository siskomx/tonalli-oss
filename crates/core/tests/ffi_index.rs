#![cfg(feature = "ffi")]
//! Thin index-registration ownership boundary, using authored tiny archives only.
use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use tonalli_core::{
    ffi::*,
    index,
    pack::{PackError, PreadSource},
    zim::Zim,
};

fn indexed(root: &Path, name: &str) -> (Arc<PackHandle>, Arc<IndexHandle>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles")
        .join(name);
    let zim = Zim::open(Arc::new(PreadSource::open(&fixture).expect("source"))).expect("ZIM");
    index::build(&zim, root, 1).expect("tiny authored index");
    let pack = pack_open_internal(fixture.to_string_lossy().into_owned()).expect("owned pack");
    let index = index_open(root.to_string_lossy().into_owned(), pack.clone()).expect("owned index");
    (pack, index)
}

fn start(root: &Path) -> Arc<LoopbackHandle> {
    let web = root.join("web");
    std::fs::create_dir_all(&web).expect("web");
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>fixture</title>",
    )
    .expect("UI");
    server_start_loopback(
        root.join("private").to_string_lossy().into_owned(),
        web.to_string_lossy().into_owned(),
    )
    .expect("server")
}

struct Browser {
    client: Client,
    origin: String,
    cookies: String,
    csrf: String,
}

fn login(server: Arc<LoopbackHandle>) -> Browser {
    let info = server_info(server).expect("live server");
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    let launch = client.get(&info.launch_url).send().expect("launch");
    let cookies = launch
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("cookie")
                .split(';')
                .next()
                .expect("pair")
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("; ");
    let origin = format!("http://127.0.0.1:{}", info.port);
    let response = client
        .get(format!("{origin}/api/session"))
        .header(COOKIE, &cookies)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .expect("session");
    assert_eq!(response.status(), 200);
    let session: Value = response.json().expect("JSON");
    Browser {
        client,
        origin,
        cookies,
        csrf: session["csrf"].as_str().expect("CSRF").to_owned(),
    }
}

fn search(browser: &Browser) -> Value {
    let response = browser
        .client
        .get(format!("{}/api/zim/search", browser.origin))
        .query(&[("pack", "reader"), ("q", "fixture"), ("limit", "20")])
        .header(COOKIE, &browser.cookies)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", &browser.csrf)
        .send()
        .expect("search");
    assert_eq!(response.status(), 200);
    response.json().expect("body results")
}

#[test]
fn registered_index_survives_caller_alias_drop_and_wrong_replacement() {
    let root = tempfile::tempdir().expect("fixture");
    let (pack, index) = indexed(&root.path().join("index"), "benign-v1.zim");
    let server = start(root.path());
    server_register_zim(server.clone(), "reader".into(), pack.clone()).expect("register source");
    server_register_index(server.clone(), "reader".into(), index.clone()).expect("register index");
    let browser = login(server.clone());
    let before = search(&browser);
    assert_eq!(before["pack"], "reader");
    assert!(before["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .any(|hit| hit["url"] == "C/en/index.html"));
    let (wrong_pack, wrong_index) = indexed(&root.path().join("wrong-index"), "hostile-v1.zim");
    assert!(matches!(
        server_register_index(server.clone(), "reader".into(), wrong_index),
        Err(PackError::Mismatch(_))
    ));
    pack_close(wrong_pack).expect("close wrong source");
    drop(index);
    pack_close(pack).expect("close caller pack aliases");
    assert_eq!(
        search(&browser),
        before,
        "registry owns independent index/source references"
    );
    server_unregister_pack(server.clone(), "reader".into()).expect("remove source and index");
    server_stop_loopback(server).expect("stop");
}

#[test]
fn index_registration_rejects_missing_removed_and_stopped_server_without_panic() {
    let root = tempfile::tempdir().expect("fixture");
    let (pack, index) = indexed(&root.path().join("index"), "benign-v1.zim");
    let server = start(root.path());
    assert!(matches!(
        server_register_index(server.clone(), "reader".into(), index.clone()),
        Err(PackError::NotFound(_))
    ));
    server_register_zim(server.clone(), "reader".into(), pack.clone()).expect("register");
    server_unregister_pack(server.clone(), "reader".into()).expect("unregister");
    assert!(matches!(
        server_register_index(server.clone(), "reader".into(), index.clone()),
        Err(PackError::NotFound(_))
    ));
    server_register_zim(server.clone(), "reader".into(), pack).expect("register again");
    server_stop_loopback(server.clone()).expect("stop");
    assert!(matches!(
        server_register_index(server, "reader".into(), index),
        Err(PackError::NotFound(_))
    ));
}
