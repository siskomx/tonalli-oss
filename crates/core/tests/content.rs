// Managed-store contracts; initial execution observed the missing module before implementation.
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};
use tonalli_core::{
    catalog::Catalog,
    content::{ManagedStore, RemoveOutcome},
    download::pack_path,
    pack::PackError,
};

// This shared authored fixture also exposes corruption helpers used by the tile suite.
#[allow(dead_code)]
#[path = "fixtures/pmtiles/mod.rs"]
mod map_fixture;
#[path = "fixtures/models/mod.rs"]
mod model_fixture;

fn signer() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap()
}
fn signed(revision: u64, body: &[u8], kind: &str) -> Catalog {
    let key = signer();
    let raw = serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"test-en","title":"Test installed content","kind":kind,"language":"en","version":format!("r{revision}"),
        "bytes":body.len(),"sha256":format!("{:x}",Sha256::digest(body)),
        "url":"https://download.kiwix.org/zim/wikipedia/test-only.zim"
    }]})).unwrap();
    Catalog::verify(&raw, key.sign(&raw).as_ref(), key.public_key().as_ref(), 0).unwrap()
}
fn fixture() -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"))
        .unwrap()
}
fn stage(store: &ManagedStore, root: &Path, catalog: &Catalog, body: &[u8]) {
    // Native-only spec: tests emulate a matching completed cache, never make HTTP calls.
    let spec = store.cache_spec(catalog, "test-en").unwrap();
    let path = pack_path(root, &spec.path).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    if path.exists() {
        assert_eq!(fs::read(path).unwrap(), body);
    } else {
        fs::write(path, body).unwrap();
    }
}

#[test]
fn signed_model_install_reuses_verified_publication_and_preserves_prior_on_bad_update() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = model_fixture::structural_gguf();
    let catalog = signed(1, &body, "model");
    assert!(store.preflight(&catalog, "test-en").unwrap().allowed);
    stage(&store, root.path(), &catalog, &body);
    let installed = store.install_cached(&catalog, "test-en").unwrap();
    let retained = store.open_source("test-en").unwrap();
    assert_eq!(retained.len(), body.len() as u64);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::InUse);
    drop(retained);
    let invalid = signed(2, b"invalid model despite signed matching hash", "model");
    stage(
        &store,
        root.path(),
        &invalid,
        b"invalid model despite signed matching hash",
    );
    assert!(store.install_cached(&invalid, "test-en").is_err());
    assert_eq!(store.list().unwrap(), [installed]);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::Removed);
}

#[test]
fn verified_cached_install_survives_latest_catalog_change_using_historical_signed_authority() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let catalog = signed(1, &body, "zim");
    stage(&store, root.path(), &catalog, &body);
    let installed = store.install_cached(&catalog, "test-en").unwrap();
    assert_eq!(installed.sha256, format!("{:x}", Sha256::digest(&body)));
    // The store's reopen must authenticate the retained historical snapshot, not consult a newer download entry.
    let newer = signed(2, b"a different not-yet-installed version", "zim");
    let later_path = root.path().join("latest.catalog");
    newer.accept_revision(&later_path).unwrap();
    drop(store);
    let reopened = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
    let source = reopened.open_source("test-en").unwrap();
    let mut prefix = [0; 4];
    source.read_at(0, &mut prefix).unwrap();
    assert_eq!(&prefix, b"ZIM\x04");
    assert_eq!(source.len(), body.len() as u64);
}

#[test]
fn matching_sha_but_invalid_format_and_wrong_cache_sha_preserve_installed_record() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let prior = signed(1, &body, "zim");
    stage(&store, root.path(), &prior, &body);
    store.install_cached(&prior, "test-en").unwrap();
    let before = store.list().unwrap();
    let invalid = signed(2, b"not a ZIM despite its matching signed SHA", "zim");
    stage(
        &store,
        root.path(),
        &invalid,
        b"not a ZIM despite its matching signed SHA",
    );
    assert!(store.install_cached(&invalid, "test-en").is_err());
    assert_eq!(store.list().unwrap(), before);
    let changed = signed(3, b"expected archive bytes", "zim");
    stage(&store, root.path(), &changed, b"wrong archive bytes");
    assert!(store.install_cached(&changed, "test-en").is_err());
    assert_eq!(store.list().unwrap(), before);
    assert_eq!(
        store.open_source("test-en").unwrap().len(),
        body.len() as u64
    );
}

#[test]
fn absent_cache_unknown_id_and_foreign_signer_never_fall_through_to_network_or_catalog_activation()
{
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let catalog = signed(1, &body, "zim");
    assert!(store
        .install_cached(&catalog, "not-in-signed-catalog")
        .is_err());
    assert!(matches!(
        store.install_cached(&catalog, "test-en"),
        Err(PackError::NotFound(_))
    ));
    // Foreign-key case must be rejected before creating an authority file at the pinned key's revision.
    let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    let foreign_store = ManagedStore::open(root.path(), other.public_key().as_ref()).unwrap();
    assert!(foreign_store.cache_spec(&catalog, "test-en").is_err());
    assert!(foreign_store.list().unwrap().is_empty());
}

#[test]
fn removal_refuses_a_live_owned_reader_then_preserves_unmanaged_siblings() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let catalog = signed(1, &body, "zim");
    stage(&store, root.path(), &catalog, &body);
    store.install_cached(&catalog, "test-en").unwrap();
    let notes = root.path().join("notes.db");
    let settings = root.path().join("settings.json");
    let external = root.path().join("user-reference.zim");
    for path in [&notes, &settings, &external] {
        fs::write(path, b"preserve exact user bytes").unwrap();
    }
    let reader = store.open_source("test-en").unwrap();
    let alias = Arc::clone(&reader);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::InUse);
    drop(reader);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::InUse);
    drop(alias);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::Removed);
    assert!(store.list().unwrap().is_empty());
    for path in [&notes, &settings, &external] {
        assert_eq!(fs::read(path).unwrap(), b"preserve exact user bytes");
    }
}

#[test]
fn same_blob_new_signed_revision_does_not_overwrite_old_authority_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let original = signed(1, &body, "zim");
    stage(&store, root.path(), &original, &body);
    let first = store.install_cached(&original, "test-en").unwrap();
    let authority = root
        .path()
        .join("packs/managed/authority")
        .join(format!("{}.tcat", first.authority_fingerprint));
    let authority_before = fs::read(&authority).unwrap();
    let old_reader = store.open_source("test-en").unwrap();
    let updated = signed(2, &body, "zim");
    stage(&store, root.path(), &updated, &body);
    let second = store.install_cached(&updated, "test-en").unwrap();
    assert_eq!(first.sha256, second.sha256);
    assert_ne!(first.authority_fingerprint, second.authority_fingerprint);
    assert_eq!(fs::read(authority).unwrap(), authority_before);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::InUse);
    drop(old_reader);
    assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::Removed);
}

#[test]
fn matching_signed_sha_cannot_install_zim_bytes_as_a_map() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let wrong_kind = signed(1, &body, "map");
    stage(&store, root.path(), &wrong_kind, &body);
    assert!(store.install_cached(&wrong_kind, "test-en").is_err());
    assert!(store.list().unwrap().is_empty());
    let map = map_fixture::simple();
    let valid = signed(2, &map, "map");
    stage(&store, root.path(), &valid, &map);
    store.install_cached(&valid, "test-en").unwrap();
    let tiles =
        tonalli_core::tiles::TileSource::open(store.open_source("test-en").unwrap()).unwrap();
    assert_eq!(
        tiles.tile(0, 0, 0).unwrap().as_deref(),
        Some(map_fixture::MVT)
    );
}

#[test]
fn case_distinct_signed_ids_never_alias_artifact_cache_or_read_lease_paths() {
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let sha = format!("{:x}", Sha256::digest(&body));
    let entries:Vec<_>=["A","a"].into_iter().map(|id|json!({"id":id,"title":id,"kind":"zim","language":"en","version":"r1","bytes":body.len(),"sha256":sha,"url":"https://download.kiwix.org/zim/wikipedia/test-only.zim"})).collect();
    let payload =
        serde_json::to_vec(&json!({"schema_version":1,"revision":1,"entries":entries})).unwrap();
    let catalog = Catalog::verify(
        &payload,
        key.sign(&payload).as_ref(),
        key.public_key().as_ref(),
        0,
    )
    .unwrap();
    let upper = store.cache_spec(&catalog, "A").unwrap();
    let lower = store.cache_spec(&catalog, "a").unwrap();
    assert_ne!(
        upper.path.to_ascii_lowercase(),
        lower.path.to_ascii_lowercase()
    );
    for (id, spec) in [("A", upper), ("a", lower)] {
        let path = pack_path(root.path(), &spec.path).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &body).unwrap();
        store.install_cached(&catalog, id).unwrap();
    }
    // A reader of a different signed ID must not prevent removal or lose its identical-SHA blob.
    let lower_reader = store.open_source("a").unwrap();
    assert_eq!(store.remove("A").unwrap(), RemoveOutcome::Removed);
    let mut magic = [0; 4];
    lower_reader.read_at(0, &mut magic).unwrap();
    assert_eq!(&magic, b"ZIM\x04");
    assert_eq!(
        store
            .list()
            .unwrap()
            .iter()
            .map(|x| x.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a"]
    );
    assert_eq!(store.open_source("a").unwrap().len(), body.len() as u64);
}

#[test]
fn content_reader_child() {
    let Some(root) = std::env::var_os("TONALLI_TEST_MANAGED_READER_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let key = signer();
    let store = ManagedStore::open(&root, key.public_key().as_ref()).unwrap();
    let source = store.open_source("test-en").unwrap();
    fs::write(root.join("child-reader-ready"), b"ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !root.join("release-child-reader").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "parent did not release owned child reader"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut prefix = [0; 4];
    source.read_at(0, &mut prefix).unwrap();
    assert_eq!(&prefix, b"ZIM\x04");
    drop(source);
}

#[test]
fn removal_refuses_a_reader_owned_by_another_process() {
    use std::process::{Command, Stdio};
    let root = tempfile::tempdir().unwrap();
    let key = signer();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let body = fixture();
    let catalog = signed(1, &body, "zim");
    stage(&store, root.path(), &catalog, &body);
    store.install_cached(&catalog, "test-en").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "content_reader_child", "--nocapture"])
        .env("TONALLI_TEST_MANAGED_READER_ROOT", root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !root.path().join("child-reader-ready").exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("owned reader child exited before admission: {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "owned child readiness timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::InUse);
        fs::write(root.path().join("release-child-reader"), b"release").unwrap();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "owned child release timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(store.remove("test-en").unwrap(), RemoveOutcome::Removed);
    }));
    // Test cleanup affects only the retained child we created, never a process found by name.
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    finished.unwrap();
}

// First implementation lease also needs tests next to content's private publication helper:
// - Inject NoSpace before version publication; previous installed metadata/blob remain byte-identical.
// - Inject metadata temp-write/persist failure; no old version removed and no partial installed row.
// - Inject post-rename flush failure => PublicationUncertain, then reopen reads one self-consistent row.
// - Corrupt retained signed authority or alter installed hash/path fields => typed rejection before open.
// - Competing install/remove processes serialize on the retained metadata lock; no orphan row points
//   to a deleted version. Lock files must never be unlinked as ordinary garbage.
// No network/process/build execution has occurred during this source-only preparation.
