use std::{fs, sync::Arc, time::Duration};

use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::Value;
use tonalli_core::{
    catalog::{Catalog, PUBLISHER_PUBLIC_KEY},
    content::ManagedStore,
    server::{LoopbackServer, PackRegistry},
};

#[test]
fn private_catalog_requires_native_attachment_and_full_api_credentials() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    let data = root.path().join("private");
    fs::create_dir(&web).unwrap();
    fs::write(
        web.join("index.html"),
        "<!doctype html><title>Tonalli</title>",
    )
    .unwrap();
    let server =
        LoopbackServer::start(Arc::new(PackRegistry::default()), data.clone(), web).unwrap();
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{}", server.info().port);
    let endpoint = format!("{base}/api/content/catalog");
    assert_eq!(client.get(&endpoint).send().unwrap().status(), 401);
    let launch = client.get(server.info().launch_url).send().unwrap();
    assert_eq!(launch.status(), 302);
    let cookies = launch
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
        .collect::<Vec<_>>();
    let api = cookies
        .iter()
        .find(|value| value.starts_with("tonalli_api="))
        .unwrap();
    let content = cookies
        .iter()
        .find(|value| value.starts_with("tonalli_content="))
        .unwrap();
    let session = || {
        client
            .get(format!("{base}/api/session"))
            .header(COOKIE, api)
            .header("sec-fetch-dest", "empty")
            .send()
            .unwrap()
            .json::<Value>()
            .unwrap()
    };
    let initial = session();
    assert_eq!(initial["private_content"], false);
    let csrf = initial["csrf"].as_str().unwrap();
    assert_eq!(
        client
            .get(&endpoint)
            .header(COOKIE, content)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(&endpoint)
            .header(COOKIE, api)
            .header("sec-fetch-dest", "empty")
            .send()
            .unwrap()
            .status(),
        403
    );
    let request = || {
        client
            .get(&endpoint)
            .header(COOKIE, api)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", csrf)
    };
    assert_eq!(request().send().unwrap().status(), 503);

    let store = Arc::new(ManagedStore::open(&data, PUBLISHER_PUBLIC_KEY).unwrap());
    fs::create_dir(root.path().join("other-private")).unwrap();
    let other_store = Arc::new(
        ManagedStore::open(&root.path().join("other-private"), PUBLISHER_PUBLIC_KEY).unwrap(),
    );
    assert!(server
        .attach_content(other_store, Arc::new(Catalog::bundled().unwrap()))
        .is_err());
    let foreign_key = Ed25519KeyPair::from_seed_unchecked(&[91; 32]).unwrap();
    let empty_payload = br#"{"schema_version":1,"revision":1,"entries":[]}"#;
    let foreign_empty = Catalog::verify(
        empty_payload,
        foreign_key.sign(empty_payload).as_ref(),
        foreign_key.public_key().as_ref(),
        0,
    )
    .unwrap();
    assert!(server
        .attach_content(Arc::clone(&store), Arc::new(foreign_empty))
        .is_err());
    assert_eq!(session()["private_content"], false);
    server
        .attach_content(store, Arc::new(Catalog::bundled().unwrap()))
        .unwrap();
    assert_eq!(session()["private_content"], true);
    assert_eq!(
        client
            .get(format!("{base}/content"))
            .header(COOKIE, content)
            .send()
            .unwrap()
            .status(),
        200,
    );
    let response = request().send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value = response.json().unwrap();
    assert_eq!(body["downloadAvailable"], false);
    assert_eq!(body["installed"], serde_json::json!([]));
    assert_eq!(body["revision"], Catalog::bundled().unwrap().revision());
    let rows = body["entries"].as_array().unwrap();
    let signed = Catalog::bundled().unwrap();
    assert_eq!(rows.len(), signed.entries().len());
    for (row, entry) in rows.iter().zip(signed.entries()) {
        assert_eq!(row["id"], entry.id);
        assert_eq!(row["title"], entry.title);
        assert_eq!(row["version"], entry.version);
        assert_eq!(row["bytes"], entry.bytes);
        assert!(row["installedVersion"].is_null());
        for forbidden in ["url", "path", "sha256", "authority_fingerprint"] {
            assert!(
                row.get(forbidden).is_none(),
                "unexpected native field {forbidden}"
            );
        }
    }
    assert_eq!(
        client
            .get(&endpoint)
            .header(COOKIE, api)
            .header("x-tonalli-csrf", csrf)
            .header("sec-fetch-dest", "iframe")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        request()
            .header("origin", "https://foreign.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    server.rotate_token().unwrap();
    assert_eq!(request().send().unwrap().status(), 401);
}
