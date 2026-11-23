use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;

#[test]
fn removal_retries_remaining_owned_versions_after_metadata_withdrawal() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let body = fixture("benign-v1.zim");
    let catalog = catalog(1, &body);
    stage(&store, &catalog, &body);
    store.install_cached(&catalog, "test-en").expect("install");
    let mut ledger = store.ledger().expect("ledger");
    ledger.entries.clear();
    storage::publish(
        &store.path("installed.json").expect("path"),
        &ledger.encode().expect("encode"),
    )
    .expect("withdrawn metadata");
    assert_eq!(
        store.remove("test-en").expect("retry owned cleanup"),
        RemoveOutcome::Removed
    );
    assert_eq!(
        store.remove("test-en").expect("idempotent"),
        RemoveOutcome::NotInstalled
    );
}

#[test]
fn oversized_removal_scan_preserves_metadata_before_mutation() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let body = fixture("benign-v1.zim");
    let catalog = catalog(1, &body);
    stage(&store, &catalog, &body);
    store.install_cached(&catalog, "test-en").expect("install");
    let before = fs::read(store.path("installed.json").expect("path")).expect("ledger");
    let entry = &catalog.entries()[0];
    let directory = store
        .version_path(entry)
        .expect("version")
        .parent()
        .expect("parent")
        .to_path_buf();
    for number in 0..256 {
        fs::write(directory.join(format!("{number:064x}.zim")), [])
            .expect("owned synthetic version");
    }
    assert!(matches!(
        store.remove("test-en"),
        Err(PackError::Corrupt(_))
    ));
    assert_eq!(
        fs::read(store.path("installed.json").expect("path")).expect("ledger"),
        before
    );
    assert_eq!(fs::read_dir(directory).expect("versions").count(), 257);
}

fn key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("test key")
}
fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles")
            .join(name),
    )
    .expect("existing tiny fixture")
}
fn catalog(revision: u64, body: &[u8]) -> Catalog {
    let key = key();
    let payload = serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"test-en","title":"Synthetic signed test","kind":"zim","language":"en","version":format!("r{revision}"),
        "bytes":body.len(),"sha256":fingerprint(body),"url":"https://download.kiwix.org/zim/wikipedia/test-only.zim"
    }]})).expect("test JSON");
    Catalog::verify(
        &payload,
        key.sign(&payload).as_ref(),
        key.public_key().as_ref(),
        0,
    )
    .expect("signed fixture")
}
fn stage(store: &ManagedStore, catalog: &Catalog, body: &[u8]) -> PathBuf {
    let spec = store.cache_spec(catalog, "test-en").expect("native spec");
    let path = storage::path(store.root(), &spec.path).expect("owned cache");
    fs::create_dir_all(path.parent().expect("parent")).expect("cache parent");
    fs::write(&path, body).expect("tiny fixture stage");
    path
}

#[test]
fn precommit_failure_keeps_old_record_and_retry_uses_moved_verified_blob_without_cache() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let old = fixture("benign-v1.zim");
    let prior = catalog(1, &old);
    stage(&store, &prior, &old);
    let installed = store
        .install_cached(&prior, "test-en")
        .expect("first install");
    let old_reader = store.open_source("test-en").expect("old owned reader");
    let ledger_before = fs::read(store.path("installed.json").expect("path")).expect("ledger");
    let next = fixture("hostile-v1.zim");
    let update = catalog(2, &next);
    let cache = stage(&store, &update, &next);
    let failed = store.install_with(
        &update,
        "test-en",
        |_| Ok(u64::MAX),
        |_, _| Err(PackError::Io("injected precommit failure".into())),
    );
    assert!(matches!(failed, Err(PackError::Io(_))));
    assert_eq!(
        fs::read(store.path("installed.json").expect("path")).expect("ledger"),
        ledger_before
    );
    assert_eq!(
        store.list().expect("authenticated old record"),
        vec![installed]
    );
    assert!(
        !cache.exists(),
        "cache moved to an immutable version, not downloaded again"
    );
    let result = store
        .install_cached(&update, "test-en")
        .expect("retry reuses owned version");
    assert_eq!(result.sha256, fingerprint(&next));
    assert_eq!(old_reader.len(), old.len() as u64);
    assert_eq!(
        store.remove("test-en").expect("remove"),
        RemoveOutcome::InUse
    );
    drop(old_reader);
    assert_eq!(
        store.remove("test-en").expect("remove all versions"),
        RemoveOutcome::Removed
    );
}

#[test]
fn no_space_preserves_prior_metadata_and_does_not_consume_cache() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let old = fixture("benign-v1.zim");
    let prior = catalog(1, &old);
    stage(&store, &prior, &old);
    store
        .install_cached(&prior, "test-en")
        .expect("first install");
    let before = fs::read(store.path("installed.json").expect("path")).expect("ledger");
    let next = fixture("hostile-v1.zim");
    let update = catalog(2, &next);
    let cache = stage(&store, &update, &next);
    assert_eq!(
        store.install_with(&update, "test-en", |_| Ok(0), storage::publish),
        Err(PackError::NoSpace)
    );
    assert_eq!(
        fs::read(store.path("installed.json").expect("path")).expect("ledger"),
        before
    );
    assert_eq!(fs::read(cache).expect("retained candidate"), next);
}

#[test]
fn final_admission_stop_keeps_verified_cache_and_old_install_untouched() {
    let root = tempfile::tempdir().unwrap();
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    let old = fixture("benign-v1.zim");
    let prior = catalog(1, &old);
    stage(&store, &prior, &old);
    store.install_cached(&prior, "test-en").unwrap();
    let before = store.list().unwrap();
    let next = fixture("hostile-v1.zim");
    let update = catalog(2, &next);
    let cache = stage(&store, &update, &next);
    let stopped = store
        .install_cached_admitted(&update, "test-en", || {
            Ok(ControlFlow::<crate::download::DownloadAction, ()>::Break(
                crate::download::DownloadAction::Cancel,
            ))
        })
        .unwrap();
    assert!(matches!(
        stopped,
        AdmittedInstall::Stopped(crate::download::DownloadAction::Cancel)
    ));
    assert_eq!(store.list().unwrap(), before);
    assert_eq!(fs::read(cache).unwrap(), next);
    assert!(!store.version_path(&update.entries()[0]).unwrap().exists());
}

#[test]
fn admission_guard_lives_through_publication_and_no_space_never_admits() {
    use std::cell::Cell;
    struct Guard<'a>(&'a Cell<bool>);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }
    let root = tempfile::tempdir().unwrap();
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    let bytes = fixture("benign-v1.zim");
    let input = catalog(1, &bytes);
    stage(&store, &input, &bytes);
    let admitted = Cell::new(false);
    let refused = store.install_admitted_with(
        &input,
        "test-en",
        |_| Ok(0),
        storage::publish,
        || {
            admitted.set(true);
            Ok(ControlFlow::Continue(Guard(&admitted)))
        },
    );
    assert!(matches!(refused, Err(PackError::NoSpace)));
    assert!(!admitted.get());
    assert!(store.list().unwrap().is_empty());
    let installed = store
        .install_admitted_with(
            &input,
            "test-en",
            |_| Ok(u64::MAX),
            |path, metadata| {
                assert!(
                    admitted.get(),
                    "final-admission fence must cover ledger publication"
                );
                storage::publish(path, metadata)
            },
            || {
                admitted.set(true);
                Ok(ControlFlow::Continue(Guard(&admitted)))
            },
        )
        .unwrap();
    assert!(matches!(installed, AdmittedInstall::Installed(_)));
    assert!(!admitted.get());
}

#[test]
fn uppercase_signed_sha_reconciles_the_canonical_installed_reference() {
    let root = tempfile::tempdir().unwrap();
    let signer = key();
    let store = ManagedStore::open(root.path(), signer.public_key().as_ref()).unwrap();
    let bytes = fixture("benign-v1.zim");
    let payload = serde_json::to_vec(&json!({"schema_version":1,"revision":1,"entries":[{
        "id":"test-en","title":"Uppercase digest","kind":"zim","language":"en","version":"r1",
        "bytes":bytes.len(),"sha256":fingerprint(&bytes).to_ascii_uppercase(),
        "url":"https://download.kiwix.org/zim/wikipedia/test-only.zim"
    }]}))
    .unwrap();
    let input = Catalog::verify(
        &payload,
        signer.sign(&payload).as_ref(),
        signer.public_key().as_ref(),
        0,
    )
    .unwrap();
    stage(&store, &input, &bytes);
    store.install_cached(&input, "test-en").unwrap();
    assert!(store
        .confirm_published(&input, "test-en")
        .unwrap()
        .is_some());
}

#[test]
fn resolve_failure_after_real_publication_is_uncertain_and_can_be_confirmed() {
    let root = tempfile::tempdir().unwrap();
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    let bytes = fixture("benign-v1.zim");
    let input = catalog(1, &bytes);
    stage(&store, &input, &bytes);
    let authority = store
        .path(&format!(
            "authority/{}.tcat",
            fingerprint(&input.snapshot_for_key(&store.pin).unwrap())
        ))
        .unwrap();
    let saved = root.path().join("held-test-authority.tcat");
    let result = store.install_admitted_with(
        &input,
        "test-en",
        |_| Ok(u64::MAX),
        |path, metadata| {
            storage::publish(path, metadata)?;
            fs::rename(&authority, &saved)?;
            Ok(())
        },
        || Ok(ControlFlow::Continue(())),
    );
    assert!(matches!(result, Err(PackError::PublicationUncertain(_))));
    fs::rename(saved, authority).unwrap();
    assert!(store
        .confirm_published(&input, "test-en")
        .unwrap()
        .is_some());
}

#[test]
fn uncertain_admitted_publication_reconciles_only_an_already_committed_exact_authority() {
    let root = tempfile::tempdir().unwrap();
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    let bytes = fixture("benign-v1.zim");
    let input = catalog(1, &bytes);
    stage(&store, &input, &bytes);
    assert!(store
        .confirm_published(&input, "test-en")
        .unwrap()
        .is_none());
    let result = store.install_admitted_with(
        &input,
        "test-en",
        |_| Ok(u64::MAX),
        |path, bytes| {
            storage::publish_with_flush(path, bytes, |_, _| {
                Err(std::io::Error::other("injected publication flush failure"))
            })
        },
        || Ok(ControlFlow::Continue(())),
    );
    assert!(matches!(result, Err(PackError::PublicationUncertain(_))));
    assert!(store
        .confirm_published(&input, "test-en")
        .unwrap()
        .is_some());
    // A newer signature over the same artifact is not evidence that that authority was installed.
    assert!(store
        .confirm_published(&catalog(2, &bytes), "test-en")
        .unwrap()
        .is_none());
}

#[test]
fn postcommit_flush_failure_is_uncertain_but_reopens_one_authenticated_complete_record() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let bytes = fixture("benign-v1.zim");
    let input = catalog(1, &bytes);
    stage(&store, &input, &bytes);
    let result = store.install_with(
        &input,
        "test-en",
        |_| Ok(u64::MAX),
        |path, bytes| {
            storage::publish_with_flush(path, bytes, |_, _| {
                Err(std::io::Error::other("injected final flush failure"))
            })
        },
    );
    assert!(matches!(result, Err(PackError::PublicationUncertain(_))));
    let reopened = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("reopen");
    assert_eq!(reopened.list().expect("authenticated publication").len(), 1);
    assert_eq!(
        reopened
            .open_source("test-en")
            .expect("complete blob")
            .len(),
        bytes.len() as u64
    );
}

#[test]
fn corrupt_authority_or_ledger_reference_is_rejected_before_source_open() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let bytes = fixture("benign-v1.zim");
    let input = catalog(1, &bytes);
    stage(&store, &input, &bytes);
    let installed = store.install_cached(&input, "test-en").expect("install");
    let authority = store
        .path(&format!(
            "authority/{}.tcat",
            installed.authority_fingerprint
        ))
        .expect("authority");
    let original = fs::read(&authority).expect("snapshot");
    fs::write(&authority, b"truncated authority").expect("corrupt own test copy");
    assert!(store.open_source("test-en").is_err());
    assert!(store.list().is_err());
    fs::write(authority, original).expect("restore fixture");
    let mut ledger = store.ledger().expect("ledger");
    ledger.entries[0].sha256 = "a".repeat(64);
    storage::publish(
        &store.path("installed.json").expect("path"),
        &ledger.encode().expect("encode"),
    )
    .expect("write malformed binding");
    assert!(store.open_source("test-en").is_err());
}

#[test]
fn accepted_catalog_authenticates_empty_foreign_input_and_preserves_revision_floor() {
    let root = tempfile::tempdir().expect("temp");
    let key = key();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store");
    let body = fixture("benign-v1.zim");
    let current = catalog(2, &body);
    store.accept_catalog(&current).expect("accept current");
    let path = store.path("content.catalog").expect("latest");
    let before = fs::read(&path).expect("accepted");
    let foreign = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).expect("foreign test key");
    let payload = br#"{"schema_version":1,"revision":3,"entries":[]}"#;
    let empty = Catalog::verify(
        payload,
        foreign.sign(payload).as_ref(),
        foreign.public_key().as_ref(),
        0,
    )
    .expect("valid under wrong pin");
    assert_eq!(store.accept_catalog(&empty), Err(PackError::Checksum));
    assert_eq!(fs::read(&path).expect("unchanged"), before);
    assert!(store.accept_catalog(&catalog(1, &body)).is_err());
    assert_eq!(fs::read(path).expect("unchanged"), before);
}
