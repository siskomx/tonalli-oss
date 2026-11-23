use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tonalli_core::{
    catalog::Catalog,
    content::{ContentJobs, JobAction, JobState, ManagedStore},
    download::pack_path,
    pack::PackError,
};

struct Fixture {
    root: tempfile::TempDir,
    store: Arc<ManagedStore>,
    jobs: Arc<ContentJobs>,
    cache: PathBuf,
    bytes: Vec<u8>,
}

fn body(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles")
            .join(name),
    )
    .unwrap()
}

fn signed(bytes: &[u8], revision: u64) -> Arc<Catalog> {
    let key = Ed25519KeyPair::from_seed_unchecked(&[43; 32]).unwrap();
    let payload = serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"test-en","title":"Test archive","kind":"zim","language":"en","version":format!("r{revision}"),
        "bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes)),
        "url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
    }]})).unwrap();
    Arc::new(
        Catalog::verify(
            &payload,
            key.sign(&payload).as_ref(),
            key.public_key().as_ref(),
            0,
        )
        .unwrap(),
    )
}

fn fixture(bytes: Vec<u8>, install_prior: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[43; 32]).unwrap();
    let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
    if install_prior {
        let old = body("benign-v1.zim");
        let catalog = signed(&old, 1);
        let cache = pack_path(
            root.path(),
            &store.cache_spec(&catalog, "test-en").unwrap().path,
        )
        .unwrap();
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(cache, old).unwrap();
        store.install_cached(&catalog, "test-en").unwrap();
    }
    let catalog = signed(&bytes, 2);
    let cache = pack_path(
        root.path(),
        &store.cache_spec(&catalog, "test-en").unwrap().path,
    )
    .unwrap();
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, &bytes).unwrap();
    let jobs = Arc::new(ContentJobs::open(Arc::clone(&store), catalog).unwrap());
    Fixture {
        root,
        store,
        jobs,
        cache,
        bytes,
    }
}

#[test]
fn preexecution_pause_cannot_hide_an_unflushable_owned_stage() {
    let f = fixture(body("benign-v1.zim"), false);
    let part = PathBuf::from(format!("{}.part", f.cache.display()));
    fs::create_dir(&part).unwrap(); // A real invalid stage shape cannot pass a required file flush.
    let job = f.jobs.start("test-en", 2).unwrap();
    f.jobs.request(&job.job.id, JobAction::Pause).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    assert!(f.jobs.execute(&mut run, |_| Ok(())).is_err());
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Failed
    );
    assert!(f.store.list().unwrap().is_empty());
    assert_eq!(fs::read(&f.cache).unwrap(), f.bytes);
}

#[test]
fn native_supplied_client_uses_same_cached_execution_and_terminal_proof() {
    let f = fixture(body("benign-v1.zim"), false);
    let job = f.jobs.start("test-en", 2).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .build()
        .unwrap();
    f.jobs
        .execute_with_client(&mut run, &client, |_| Ok(()))
        .unwrap();
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Completed
    );
}

#[test]
fn actual_cached_zim_installs_with_observed_progress_and_core_owned_completion() {
    let f = fixture(body("benign-v1.zim"), false);
    let job = f.jobs.start("test-en", 2).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    let mut samples = Vec::new();
    f.jobs
        .execute(&mut run, |sample| {
            samples.push(sample);
            Ok(())
        })
        .unwrap();
    assert!(samples
        .iter()
        .any(|s| s.state == JobState::Verifying && s.verified_bytes > 0));
    assert!(samples.iter().all(|s| s.verified_bytes <= s.total_bytes));
    assert!(!f.cache.exists());
    assert_eq!(
        f.store.open_source("test-en").unwrap().len(),
        f.bytes.len() as u64
    );
    assert_ne!(f.jobs.list().unwrap()[0].state, JobState::Completed);
    let done = f.jobs.finish_exited(&mut run).unwrap();
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.verified_bytes, done.total_bytes);
}

#[test]
fn pause_during_real_cached_hash_retains_candidate_until_explicit_resume() {
    let f = fixture(body("benign-v1.zim"), false);
    let job = f.jobs.start("test-en", 2).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    let mut requested = false;
    f.jobs
        .execute(&mut run, |sample| {
            if sample.state == JobState::Verifying && sample.verified_bytes > 0 && !requested {
                requested = true;
                f.jobs.request(&job.job.id, JobAction::Pause).unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert!(requested);
    assert!(f.store.list().unwrap().is_empty());
    assert_eq!(fs::read(&f.cache).unwrap(), f.bytes);
    assert_eq!(f.jobs.list().unwrap()[0].state, JobState::Pausing);
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Paused
    );
    let resumed = f.jobs.request(&job.job.id, JobAction::Resume).unwrap();
    let mut second = f.jobs.claim(&resumed.ticket).unwrap();
    f.jobs.execute(&mut second, |_| Ok(())).unwrap();
    assert_eq!(
        f.jobs.finish_exited(&mut second).unwrap().state,
        JobState::Completed
    );
}

#[test]
fn cancel_before_execution_cleans_exact_owned_staging_only_after_worker_exit() {
    let f = fixture(body("benign-v1.zim"), false);
    let part = PathBuf::from(format!("{}.part", f.cache.display()));
    let meta = PathBuf::from(format!("{}.part.meta", f.cache.display()));
    fs::write(&part, b"owned partial").unwrap();
    fs::write(&meta, b"owned partial validator").unwrap();
    let unrelated = f.root.path().join("notes.db");
    fs::write(&unrelated, b"unrelated private bytes").unwrap();
    let job = f.jobs.start("test-en", 2).unwrap();
    f.jobs.request(&job.job.id, JobAction::Cancel).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    f.jobs
        .execute(&mut run, |_| {
            panic!("cancel-before-start cannot begin content work")
        })
        .unwrap();
    assert!(f.cache.exists() && part.exists() && meta.exists());
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Cancelled
    );
    assert!(!f.cache.exists() && !part.exists() && !meta.exists());
    assert_eq!(fs::read(unrelated).unwrap(), b"unrelated private bytes");
    assert!(f.store.list().unwrap().is_empty());
}

#[test]
fn invalid_format_failure_preserves_old_install_and_remains_cancelable() {
    let f = fixture(
        b"signed checksum matches but this is not a ZIM".to_vec(),
        true,
    );
    let prior = f.store.list().unwrap();
    let job = f.jobs.start("test-en", 2).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    assert!(f.jobs.execute(&mut run, |_| Ok(())).is_err());
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Failed
    );
    assert_eq!(f.store.list().unwrap(), prior);
    assert_eq!(fs::read(&f.cache).unwrap(), f.bytes);
    let cancel = f.jobs.request(&job.job.id, JobAction::Cancel).unwrap();
    assert!(cancel.dispatch);
    let mut cleanup = f.jobs.claim(&cancel.ticket).unwrap();
    f.jobs.execute(&mut cleanup, |_| Ok(())).unwrap();
    assert_eq!(
        f.jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
    assert!(!f.cache.exists());
    assert_eq!(f.store.list().unwrap(), prior);
}

#[test]
fn fallible_checkpoint_failure_cannot_be_reported_as_requested_pause() {
    let f = fixture(body("hostile-v1.zim"), true);
    let prior = f.store.list().unwrap();
    let job = f.jobs.start("test-en", 2).unwrap();
    let mut run = f.jobs.claim(&job.ticket).unwrap();
    let result = f.jobs.execute(&mut run, |_| {
        f.jobs.request(&job.job.id, JobAction::Pause).unwrap();
        Err(PackError::Io("test checkpoint failed".into()))
    });
    assert!(result.is_err());
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Failed
    );
    assert_eq!(f.store.list().unwrap(), prior);
    let retry = f.jobs.request(&job.job.id, JobAction::Retry).unwrap();
    assert!(retry.dispatch && retry.ticket.epoch > job.ticket.epoch);
}
