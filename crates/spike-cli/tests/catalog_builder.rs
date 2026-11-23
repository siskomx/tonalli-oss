use std::{fs, path::Path};

use serde_json::json;
use spike_cli::catalog::{generate_key, sign_catalog};
use tonalli_core::catalog::Catalog;

fn input(path: &Path) -> Vec<u8> {
    let payload = serde_json::to_vec(&json!({
        "schema_version": 1, "revision": 1,
        "entries": [{
            "id":"wikipedia-en-knots", "title":"Knots by Wikipedia", "kind":"zim",
            "language":"en", "version":"2026-10", "bytes":1024,
            "sha256":"a".repeat(64),
            "url":"https://download.kiwix.org/zim/wikipedia/example.zim"
        }]
    }))
    .unwrap();
    fs::write(path, &payload).unwrap();
    payload
}

#[test]
fn generated_publisher_key_signs_exact_catalog_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("test-only-private.pk8");
    let public = generate_key(&key).unwrap();
    assert_eq!(public.len(), 32);
    let document = temp.path().join("catalog.json");
    let payload = input(&document);
    let signature = temp.path().join("catalog.sig");
    sign_catalog(&document, &key, &signature).unwrap();
    let signature = fs::read(signature).unwrap();
    assert_eq!(signature.len(), 64);
    assert_eq!(
        Catalog::verify(&payload, &signature, &public, 0)
            .unwrap()
            .revision(),
        1
    );
    assert_eq!(fs::read(document).unwrap(), payload);
}

#[test]
fn builder_never_overwrites_a_key_or_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("test-only-private.pk8");
    generate_key(&key).unwrap();
    let original_key = fs::read(&key).unwrap();
    assert!(generate_key(&key).is_err());
    assert!(fs::read(&key).unwrap() == original_key);
    let document = temp.path().join("catalog.json");
    input(&document);
    let signature = temp.path().join("catalog.sig");
    fs::write(&signature, b"existing publisher output").unwrap();
    assert!(sign_catalog(&document, &key, &signature).is_err());
    assert_eq!(fs::read(signature).unwrap(), b"existing publisher output");
    assert!(fs::read(key).unwrap() == original_key);
}

#[test]
fn malformed_catalog_or_key_never_publishes_a_signature() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("test-only-private.pk8");
    generate_key(&key).unwrap();
    let document = temp.path().join("catalog.json");
    let signature = temp.path().join("catalog.sig");
    for bytes in [b"not json".to_vec(), vec![b' '; 1024 * 1024 + 1]] {
        fs::write(&document, &bytes).unwrap();
        assert!(sign_catalog(&document, &key, &signature).is_err());
        assert!(!signature.exists());
        assert_eq!(fs::read(&document).unwrap(), bytes);
    }
    input(&document);
    fs::write(&key, b"invalid synthetic key").unwrap();
    assert!(sign_catalog(&document, &key, &signature).is_err());
    assert!(!signature.exists());
}

#[test]
fn builder_commands_reach_the_shared_verifier() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("test-only-private.pk8");
    let document = temp.path().join("catalog.json");
    let signature = temp.path().join("catalog.sig");
    input(&document);
    spike_cli::run([
        "catalog-keygen".into(),
        "--out".into(),
        key.to_str().unwrap().into(),
    ])
    .unwrap();
    spike_cli::run([
        "catalog-sign".into(),
        "--catalog".into(),
        document.to_str().unwrap().into(),
        "--key".into(),
        key.to_str().unwrap().into(),
        "--out".into(),
        signature.to_str().unwrap().into(),
    ])
    .unwrap();
    assert_eq!(fs::read(signature).unwrap().len(), 64);
    assert!(spike_cli::run(["catalog-keygen".into(), "--out".into()]).is_err());
}
