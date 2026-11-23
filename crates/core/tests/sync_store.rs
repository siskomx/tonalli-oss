use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use tonalli_core::{
    catalog::CatalogKind,
    pack::PackError,
    sync::{PackManifest, SyncStore},
};

#[test]
fn model_is_not_admitted_as_a_nearby_library_copy() {
    let root = tempfile::tempdir().unwrap();
    let store = SyncStore::open(root.path()).unwrap();
    let model = PackManifest {
        version: 1,
        id: "model".into(),
        title: "Local model".into(),
        kind: CatalogKind::Model,
        bytes: 1024,
        sha256: "a".repeat(64),
    };
    assert!(model.validate().is_err());
    assert!(store.cache_spec(&model).is_err());
    assert!(store.install_cached(&model).is_err());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn nearby_copy_removal_respects_readers_and_only_deletes_its_owned_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = SyncStore::open(root.path()).unwrap();
    let original =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    let bytes = fs::read(&original).unwrap();
    let manifest = PackManifest {
        version: 1,
        id: "shared-book".into(),
        title: "Owned copy".into(),
        kind: CatalogKind::Zim,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let spec = store.cache_spec(&manifest).unwrap();
    fs::write(root.path().join(&spec.path), &bytes).unwrap();
    store.install_cached(&manifest).unwrap();
    fs::write(root.path().join("notes.db"), b"private sentinel").unwrap();
    let reader = store.open_source(&manifest).unwrap();
    let other_process_store = SyncStore::open(root.path()).unwrap();
    assert_eq!(
        other_process_store.remove(&manifest.local_id()).unwrap(),
        tonalli_core::content::RemoveOutcome::InUse
    );
    assert_eq!(store.list().unwrap(), vec![manifest.clone()]);
    drop(reader);
    assert_eq!(
        store.remove(&manifest.local_id()).unwrap(),
        tonalli_core::content::RemoveOutcome::Removed
    );
    assert!(store.list().unwrap().is_empty());
    assert!(!root.path().join(&spec.path).exists());
    assert!(store.open_source(&manifest).is_err());
    assert_eq!(
        store.remove(&manifest.local_id()).unwrap(),
        tonalli_core::content::RemoveOutcome::NotInstalled
    );
    assert!(store.remove("../notes.db").is_err());
    assert!(store.remove("shared-book").is_err());
    assert_eq!(
        fs::read(root.path().join("notes.db")).unwrap(),
        b"private sentinel"
    );
    assert_eq!(fs::read(original).unwrap(), bytes);
}

#[test]
fn interrupted_copy_removal_stays_withdrawn_and_can_finish_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let store = SyncStore::open(root.path()).unwrap();
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap();
    let manifest = PackManifest {
        version: 1,
        id: "shared-book".into(),
        title: "Owned copy".into(),
        kind: CatalogKind::Zim,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let spec = store.cache_spec(&manifest).unwrap();
    fs::write(root.path().join(&spec.path), bytes).unwrap();
    store.install_cached(&manifest).unwrap();
    // Simulate interruption after durable reference withdrawal, before blob cleanup.
    fs::write(
        root.path()
            .join(format!("packs/nearby/installed/{}.json", manifest.sha256)),
        serde_json::to_vec(&serde_json::json!({"removed":manifest})).unwrap(),
    )
    .unwrap();
    let reopened = SyncStore::open(root.path()).unwrap();
    assert!(reopened.list().unwrap().is_empty());
    assert!(reopened.open_source(&manifest).is_err());
    assert_eq!(
        reopened.remove(&manifest.local_id()).unwrap(),
        tonalli_core::content::RemoveOutcome::Removed
    );
    assert!(!root.path().join(&spec.path).exists());
    // A later explicit save can install the exact same archive again.
    let original =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    fs::copy(original, root.path().join(&spec.path)).unwrap();
    reopened.install_cached(&manifest).unwrap();
    assert_eq!(reopened.list().unwrap(), vec![manifest.clone()]);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // A real non-delete-sharing Windows handle blocks cleanup after withdrawal.
        let obstruction = fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(root.path().join(&spec.path))
            .unwrap();
        assert!(matches!(
            reopened.remove(&manifest.local_id()),
            Err(PackError::PublicationUncertain(_))
        ));
        assert!(reopened.list().unwrap().is_empty());
        assert!(reopened.open_source(&manifest).is_err());
        drop(obstruction);
        assert_eq!(
            reopened.remove(&manifest.local_id()).unwrap(),
            tonalli_core::content::RemoveOutcome::Removed
        );
    }
}

#[test]
fn nearby_copies_publish_only_complete_verified_archives_and_preserve_prior_copies() {
    let root = tempfile::tempdir().unwrap();
    let store = SyncStore::open(root.path()).unwrap();
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap();
    let good = PackManifest {
        version: 1,
        id: "selected".into(),
        title: "Owned copy".into(),
        kind: CatalogKind::Zim,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let cache = |m: &PackManifest, content: &[u8]| {
        let spec = store.cache_spec(m).unwrap();
        let path = root.path().join(spec.path);
        fs::write(path, content).unwrap();
    };
    cache(&good, &bytes);
    assert_eq!(store.install_cached(&good).unwrap(), good);
    let before = store.list().unwrap();
    assert_eq!(before, vec![good.clone()]);
    let mut changed = good.clone();
    changed.sha256 = "0".repeat(64);
    cache(&changed, &bytes);
    assert_eq!(
        store.install_cached(&changed).unwrap_err(),
        PackError::Checksum
    );
    let mut short = good.clone();
    short.sha256 = format!("{:x}", Sha256::digest(&bytes[..32]));
    cache(&short, &bytes[..32]);
    assert!(store.install_cached(&short).is_err());
    let mut invalid = good.clone();
    invalid.id = "../notes".into();
    assert!(store.cache_spec(&invalid).is_err());
    let mut not_archive = good.clone();
    not_archive.bytes = 12;
    not_archive.sha256 = format!("{:x}", Sha256::digest(b"not an index"));
    cache(&not_archive, b"not an index");
    assert!(store.install_cached(&not_archive).is_err());
    assert_eq!(
        SyncStore::open(root.path()).unwrap().list().unwrap(),
        before
    );
}
