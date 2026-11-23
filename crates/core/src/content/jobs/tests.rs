use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::{sync::mpsc, thread, time::Duration};

#[test]
fn removal_admission_guard_fences_concurrent_job_start_through_actual_deletion() {
    let root = tempfile::tempdir().unwrap();
    let signer = Ed25519KeyPair::from_seed_unchecked(&[37; 32]).unwrap();
    let bytes = fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap();
    let payload=serde_json::to_vec(&serde_json::json!({"schema_version":1,"revision":1,"entries":[{
        "id":"test-en","title":"Removal test","kind":"zim","language":"en","version":"r1","bytes":bytes.len(),
        "sha256":managed::fingerprint(&bytes),"url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
    }]})).unwrap();
    let catalog = Arc::new(
        Catalog::verify(
            &payload,
            signer.sign(&payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap(),
    );
    let store = Arc::new(ManagedStore::open(root.path(), signer.public_key().as_ref()).unwrap());
    let jobs = Arc::new(ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).unwrap());
    let spec = store.cache_spec(&catalog, "test-en").unwrap();
    let cache = pack_path(root.path(), &spec.path).unwrap();
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(cache, bytes).unwrap();
    store.install_cached(&catalog, "test-en").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let remove_store = Arc::clone(&store);
    let remove_jobs = Arc::clone(&jobs);
    let removal = thread::spawn(move || {
        remove_store.remove_admitted_with(
            "test-en",
            || {
                let gate = remove_jobs
                    .paths
                    .lock()
                    .map_err(|_| PackError::Io("test admission failed".into()))?;
                Ok(std::ops::ControlFlow::<(), _>::Continue(gate))
            },
            |path, metadata| {
                let probe = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(remove_store.path("jobs.lock")?)?;
                let blocked = fs2::FileExt::try_lock_exclusive(&probe).unwrap_err();
                assert_eq!(
                    blocked.raw_os_error(),
                    fs2::lock_contended_error().raw_os_error(),
                    "Jobs must still be held at real metadata publication"
                );
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                managed::publish(path, metadata)
            },
        )
    });
    entered_rx.recv().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let starter = thread::spawn(move || {
        started_tx.send(jobs.start("test-en", 1)).unwrap();
    });
    let pending = started_rx.recv_timeout(Duration::from_millis(100));
    release_tx.send(()).unwrap();
    assert_eq!(
        removal.join().unwrap().unwrap(),
        crate::content::RemoveOutcome::Removed
    );
    starter.join().unwrap();
    assert!(
        matches!(pending, Err(mpsc::RecvTimeoutError::Timeout)),
        "start cannot pass the removal admission fence before deletion"
    );
    assert!(started_rx.recv().unwrap().is_ok());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn first_open_waits_for_store_gate_before_publishing_authority_or_jobs() {
    let root = tempfile::tempdir().unwrap();
    let signer = Ed25519KeyPair::from_seed_unchecked(&[31; 32]).unwrap();
    let payload = br#"{"schema_version":1,"revision":1,"entries":[]}"#;
    let catalog = Arc::new(
        Catalog::verify(
            payload,
            signer.sign(payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap(),
    );
    let store = Arc::new(ManagedStore::open(root.path(), signer.public_key().as_ref()).unwrap());
    let gate = store.gate().unwrap();
    let worker_store = Arc::clone(&store);
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(ContentJobs::open(worker_store, catalog))
            .unwrap();
    });
    started_rx.recv().unwrap();
    let pending = done_rx.recv_timeout(Duration::from_secs(1));
    let unpublished = !store.path("jobs.lock").unwrap().exists()
        && !store.path("jobs.json").unwrap().exists()
        && !store.path("authority").unwrap().exists();
    drop(gate);
    worker.join().unwrap();
    assert!(
        matches!(pending, Err(mpsc::RecvTimeoutError::Timeout)),
        "initialization must wait for the store gate"
    );
    assert!(
        unpublished,
        "authority and jobs must remain unpublished while the gate is held"
    );
    assert!(done_rx.recv().unwrap().is_ok());
}
