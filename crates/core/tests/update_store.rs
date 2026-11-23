use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use tonalli_core::pack::PackError;
use tonalli_core::update::{Store, Version, MANIFEST_FILE, SIGNATURE_FILE};

// Synthetic test-only keys. Never a publisher key.
fn pair(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn key_hex(pair: &Ed25519KeyPair) -> String {
    hex(pair.public_key().as_ref())
}

fn v(text: &str) -> Version {
    text.parse().unwrap()
}

const SOURCE: &str = "example/tonalli";
const BASE: &str = "https://github.com/example/tonalli/releases/download/update-feed/";
const PACKAGE: &[u8] = b"pretend this is the new application package";

fn manifest(version: &str, package: &[u8]) -> serde_json::Value {
    json!({
        "schema_version": 1, "version": version, "published": "2026-11-16",
        "notes": "notes", "page": "https://example.org/release",
        "assets": [{
            "platform": "windows-x64", "url": "https://example.org/Tonalli-Windows-x64.zip",
            "bytes": package.len(), "sha256": hex(&Sha256::digest(package)),
        }],
    })
}

struct Feed(HashMap<String, Vec<u8>>, AtomicUsize);

impl Feed {
    fn serving(value: &serde_json::Value, signer: &Ed25519KeyPair) -> Self {
        let bytes = serde_json::to_vec(value).unwrap();
        let signature = signer.sign(&bytes).as_ref().to_vec();
        Self(
            HashMap::from([
                (format!("{BASE}{MANIFEST_FILE}"), bytes),
                (format!("{BASE}{SIGNATURE_FILE}"), signature),
            ]),
            AtomicUsize::new(0),
        )
    }

    fn fetch(&self, url: &str, _limit: usize) -> Result<Vec<u8>, PackError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| PackError::NotFound(url.into()))
    }
}

fn configured(root: &std::path::Path, trusted: &Ed25519KeyPair) -> Store {
    let store = Store::new(root);
    store.save_settings(true, SOURCE, &key_hex(trusted)).unwrap();
    store
}

#[test]
fn updates_are_off_and_unconfigured_by_default() {
    let root = tempfile::tempdir().unwrap();
    let settings = Store::new(root.path()).settings().unwrap();
    assert!(!settings.enabled);
    assert!(settings.source.is_empty() && settings.public_key.is_empty());
}

#[test]
fn settings_are_validated_normalized_and_persisted() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let store = Store::new(root.path());
    let saved = store
        .save_settings(true, " example/tonalli ", &key_hex(&trusted).to_uppercase())
        .unwrap();
    assert_eq!(saved.source, BASE);
    assert_eq!(saved.public_key, key_hex(&trusted));
    let reread = Store::new(root.path()).settings().unwrap();
    assert!(reread.enabled);
    assert_eq!(reread.source, BASE);

    assert!(store.save_settings(true, "http://nope.example.org/", &key_hex(&trusted)).is_err());
    assert!(store.save_settings(true, SOURCE, "not a key").is_err());
    assert!(store.save_settings(true, "", &key_hex(&trusted)).is_err());
    assert!(store.save_settings(true, SOURCE, "").is_err());
    // A failed save leaves the previous settings untouched.
    assert_eq!(store.settings().unwrap().source, BASE);

    // Turning updates off needs no valid source, and keeps what was configured.
    let off = store.save_settings(false, SOURCE, &key_hex(&trusted)).unwrap();
    assert!(!off.enabled);
    assert_eq!(off.source, BASE);
}

#[test]
fn nothing_is_fetched_while_updates_are_off() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = Store::new(root.path());
    assert!(store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).is_err());
    store.save_settings(false, SOURCE, &key_hex(&trusted)).unwrap();
    assert!(store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).is_err());
    assert_eq!(feed.1.load(Ordering::SeqCst), 0);
}

#[test]
fn a_newer_signed_release_is_offered_and_remembered() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    let offered = store
        .check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l))
        .unwrap()
        .expect("newer release is offered");
    assert_eq!(offered.version_text(), "0.1.0-preview.3");
    let (remembered, asset) = store
        .offer(&v("0.1.0-preview.2"), "windows-x64")
        .unwrap()
        .expect("offer survives");
    assert_eq!(remembered.version_text(), "0.1.0-preview.3");
    assert_eq!(asset.bytes, PACKAGE.len() as u64);
}

#[test]
fn the_same_or_an_older_release_is_not_an_update() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    for installed in ["0.1.0-preview.3", "0.1.0-preview.4", "0.1.0"] {
        assert!(store.check(&v(installed), |u, l| feed.fetch(u, l)).unwrap().is_none());
    }
    assert!(store.offer(&v("0.1.0"), "windows-x64").unwrap().is_none());
}

#[test]
fn a_release_signed_by_anyone_else_is_refused_and_never_remembered() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let attacker = pair(2);
    let feed = Feed::serving(&manifest("9.9.9", PACKAGE), &attacker);
    let store = configured(root.path(), &trusted);
    assert!(store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).is_err());
    assert!(store.offer(&v("0.1.0-preview.2"), "windows-x64").unwrap().is_none());
}

#[test]
fn a_remembered_offer_stops_counting_if_the_trusted_key_changes() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).unwrap();
    store.save_settings(true, SOURCE, &key_hex(&pair(2))).unwrap();
    assert!(store.offer(&v("0.1.0-preview.2"), "windows-x64").unwrap().is_none());
}

#[test]
fn a_platform_without_an_asset_has_no_offer() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).unwrap();
    assert!(store.offer(&v("0.1.0-preview.2"), "android-arm64").unwrap().is_none());
}

#[test]
fn the_staged_file_path_is_inside_the_private_updates_folder() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    store.check(&v("0.1.0-preview.2"), |u, l| feed.fetch(u, l)).unwrap();
    let (offered, _) = store.offer(&v("0.1.0-preview.2"), "windows-x64").unwrap().unwrap();
    let relative = Store::staged_name(&offered, "windows-x64").unwrap();
    assert!(relative.starts_with("packs/updates/") && relative.ends_with(".zip"));
    assert!(!relative.contains(".."));
    assert_eq!(Store::staged_name(&offered, "android-arm64").unwrap().rsplit('.').next(), Some("apk"));
    assert!(Store::staged_name(&offered, "plan9").is_err());
}

#[test]
fn a_quick_look_reports_a_staged_file_only_by_exact_size_without_hashing_it() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    let installed = v("0.1.0-preview.2");
    assert!(!store.staged(&installed, "windows-x64").unwrap(), "no offer yet");
    store.check(&installed, |u, l| feed.fetch(u, l)).unwrap();
    assert!(!store.staged(&installed, "windows-x64").unwrap(), "nothing staged yet");
    let (offered, _) = store.offer(&installed, "windows-x64").unwrap().unwrap();
    let path = root.path().join(Store::staged_name(&offered, "windows-x64").unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"short").unwrap();
    assert!(!store.staged(&installed, "windows-x64").unwrap(), "wrong size");
    std::fs::write(&path, PACKAGE).unwrap();
    assert!(store.staged(&installed, "windows-x64").unwrap());
}

#[test]
fn a_staged_file_must_match_the_signed_size_and_hash_to_be_ready() {
    let root = tempfile::tempdir().unwrap();
    let trusted = pair(1);
    let feed = Feed::serving(&manifest("0.1.0-preview.3", PACKAGE), &trusted);
    let store = configured(root.path(), &trusted);
    let installed = v("0.1.0-preview.2");
    store.check(&installed, |u, l| feed.fetch(u, l)).unwrap();
    let (offered, _) = store.offer(&installed, "windows-x64").unwrap().unwrap();
    let relative = Store::staged_name(&offered, "windows-x64").unwrap();
    let path = root.path().join(&relative);

    assert!(store.ready(&installed, "windows-x64").unwrap().is_none(), "nothing staged yet");

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"tampered bytes of another length").unwrap();
    assert!(store.ready(&installed, "windows-x64").is_err());

    let mut same_length = PACKAGE.to_vec();
    same_length[0] ^= 1;
    std::fs::write(&path, &same_length).unwrap();
    assert!(matches!(store.ready(&installed, "windows-x64"), Err(PackError::Checksum)));

    std::fs::write(&path, PACKAGE).unwrap();
    let ready = store.ready(&installed, "windows-x64").unwrap().expect("verified file is ready");
    assert_eq!(ready.path.canonicalize().unwrap(), path.canonicalize().unwrap());
    assert_eq!(ready.version, "0.1.0-preview.3");
}
