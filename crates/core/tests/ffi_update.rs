#![cfg(feature = "ffi")]

use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use tonalli_core::{
    ffi::update_ready,
    update::{current_platform, Store, MANIFEST_FILE, SIGNATURE_FILE},
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const PACKAGE: &[u8] = b"pretend package for this platform";
const BASE: &str = "https://github.com/example/tonalli/releases/download/update-feed/";

// Synthetic test-only key. Never a publisher key.
fn staged(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let platform = current_platform()?;
    let pair = Ed25519KeyPair::from_seed_unchecked(&[5; 32]).unwrap();
    let store = Store::new(root);
    store
        .save_settings(true, "example/tonalli", &hex(pair.public_key().as_ref()))
        .unwrap();
    let manifest = serde_json::to_vec(&json!({
        "schema_version": 1, "version": "99.0.0", "published": "2026-11-16", "notes": "", "page": "",
        "assets": [{"platform": platform, "url": "https://example.org/package",
            "bytes": PACKAGE.len(), "sha256": hex(&Sha256::digest(PACKAGE))}],
    }))
    .unwrap();
    let signature = pair.sign(&manifest).as_ref().to_vec();
    let installed = tonalli_core::update::app_version().unwrap();
    store
        .check(&installed, |url, _| {
            if url == format!("{BASE}{MANIFEST_FILE}") {
                Ok(manifest.clone())
            } else if url == format!("{BASE}{SIGNATURE_FILE}") {
                Ok(signature.clone())
            } else {
                panic!("unexpected {url}")
            }
        })
        .unwrap()
        .expect("newer release");
    let (offered, _) = store.offer(&installed, platform).unwrap().unwrap();
    let path = root.join(Store::staged_name(&offered, platform).unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    Some(path)
}

#[test]
fn nothing_is_ready_without_a_trusted_offer() {
    let root = tempfile::tempdir().unwrap();
    assert!(update_ready(root.path().to_str().unwrap().into()).unwrap().is_none());
}

#[test]
fn only_a_fully_verified_package_is_handed_to_the_installer() {
    let root = tempfile::tempdir().unwrap();
    let Some(path) = staged(root.path()) else { return };
    let data = || root.path().to_str().unwrap().to_owned();
    assert!(update_ready(data()).unwrap().is_none(), "nothing downloaded yet");

    std::fs::write(&path, b"tampered, and a different length").unwrap();
    assert!(update_ready(data()).is_err());

    std::fs::write(&path, PACKAGE).unwrap();
    let ready = update_ready(data()).unwrap().expect("verified package");
    assert_eq!(ready.version, "99.0.0");
    assert_eq!(ready.bytes, PACKAGE.len() as u64);
    assert_eq!(
        std::path::Path::new(&ready.path).canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
}
