use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};
use tonalli_core::{
    catalog::Catalog,
    content::{ContentJobs, JobAction, JobError, JobState, ManagedStore},
    download::DownloadAction,
};

struct Fixture {
    root: tempfile::TempDir,
    store: Arc<ManagedStore>,
    catalog: Arc<Catalog>,
    jobs: Arc<ContentJobs>,
}

fn key(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn catalog(seed: u8, revision: u64, prefix: &str, count: usize) -> Arc<Catalog> {
    let signer = key(seed);
    let entries = (0..count)
        .map(|index| {
            json!({
                "id":format!("{prefix}-{index}"), "title":format!("Test library {index}"),
                "kind":"zim", "language":"en", "version":format!("r{revision}"),
                "bytes":1024, "sha256":"a".repeat(64),
                "url":format!("https://download.kiwix.org/zim/wikipedia/test-{index}.zim")
            })
        })
        .collect::<Vec<_>>();
    let payload =
        serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":entries}))
            .unwrap();
    Arc::new(
        Catalog::verify(
            &payload,
            signer.sign(&payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap(),
    )
}

fn fixture(count: usize) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(ManagedStore::open(root.path(), key(17).public_key().as_ref()).unwrap());
    let catalog = catalog(17, 1, "pack", count);
    let jobs = Arc::new(ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).unwrap());
    Fixture {
        root,
        store,
        catalog,
        jobs,
    }
}

fn ledger(root: &Path) -> PathBuf {
    root.join("packs/managed/jobs.json")
}

#[test]
fn model_download_uses_existing_ticket_pause_and_resume_admission() {
    let root = tempfile::tempdir().unwrap();
    let signer = key(17);
    let payload = serde_json::to_vec(&json!({"schema_version":1,"revision":1,"entries":[{
        "id":"model","title":"Local model","kind":"model","language":"en","version":"1",
        "bytes":1024,"sha256":"a".repeat(64),"url":"https://example.invalid/model.gguf"
    }]}))
    .unwrap();
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
    let jobs = ContentJobs::open(store, catalog).unwrap();
    let first = jobs.start("model", 1).unwrap();
    assert_eq!(first.job.state, JobState::Queued);
    jobs.request_ticket(&first.ticket, JobAction::Pause)
        .unwrap();
    let mut run = jobs.claim(&first.ticket).unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let paused = jobs.list().unwrap().remove(0);
    assert_eq!(paused.state, JobState::Paused);
    let resumed = jobs.request(&paused.id, JobAction::Resume).unwrap();
    assert!(resumed.ticket.epoch > first.ticket.epoch);
}

#[test]
fn native_ticket_stop_cannot_pause_a_replacement_epoch() {
    let f = fixture(1);
    assert!(f.jobs.active_ticket().unwrap().is_none());
    let first = f.jobs.start("pack-0", 1).unwrap();
    assert_eq!(f.jobs.active_ticket().unwrap(), Some(first.ticket.clone()));
    f.jobs
        .request_ticket(&first.ticket, JobAction::Pause)
        .unwrap();
    let mut run = f.jobs.claim(&first.ticket).unwrap();
    f.jobs.finish_exited(&mut run).unwrap();
    assert!(f.jobs.active_ticket().unwrap().is_none());
    let resumed = f.jobs.request(&first.job.id, JobAction::Resume).unwrap();
    assert_eq!(
        f.jobs.active_ticket().unwrap(),
        Some(resumed.ticket.clone())
    );
    assert!(matches!(
        f.jobs.request_ticket(&first.ticket, JobAction::Pause),
        Err(JobError::StaleAttempt)
    ));
    assert_eq!(f.jobs.list().unwrap()[0].state, JobState::Queued);
    f.jobs
        .request_ticket(&resumed.ticket, JobAction::Cancel)
        .unwrap();
    assert_eq!(f.jobs.list().unwrap()[0].state, JobState::Cancelling);
}

#[test]
fn explicit_orphan_reconciliation_cannot_claim_paused_when_stage_flush_fails() {
    let f = fixture(1);
    let spec = f.store.cache_spec(&f.catalog, "pack-0").unwrap();
    let part = f.root.path().join(format!("{}.part", spec.path));
    fs::create_dir_all(&part).unwrap();
    let first = f.jobs.start("pack-0", 1).unwrap();
    let failed = f.jobs.reconcile_interrupted(&first.ticket).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("io"));
    assert!(part.is_dir());
    assert!(
        f.jobs
            .request(&first.job.id, JobAction::Cancel)
            .unwrap()
            .dispatch
    );
}

#[test]
fn definite_dispatch_rejection_cannot_claim_paused_with_an_unflushable_stage() {
    let f = fixture(1);
    let spec = f.store.cache_spec(&f.catalog, "pack-0").unwrap();
    let part = f.root.path().join(format!("{}.part", spec.path));
    fs::create_dir_all(&part).unwrap();
    let first = f.jobs.start("pack-0", 1).unwrap();
    let failed = f.jobs.dispatch_rejected(&first.ticket).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert!(
        part.is_dir(),
        "failed barrier must preserve the owned invalid stage"
    );
    assert!(
        f.jobs
            .request(&first.job.id, JobAction::Cancel)
            .unwrap()
            .dispatch
    );
}

#[test]
fn definite_dispatch_rejection_releases_admission_without_starting_or_touching_staging() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let rejected = f.jobs.dispatch_rejected(&first.ticket).unwrap();
    assert_eq!(rejected.state, JobState::Paused);
    assert_eq!(rejected.error_code.as_deref(), Some("dispatch_rejected"));
    assert!(matches!(
        f.jobs.claim(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    let resumed = f.jobs.request(&first.job.id, JobAction::Resume).unwrap();
    assert!(matches!(
        f.jobs.dispatch_rejected(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    let live = f.jobs.claim(&resumed.ticket).unwrap();
    assert!(matches!(
        f.jobs.dispatch_rejected(&resumed.ticket),
        Err(JobError::Busy)
    ));
    drop(live);
}

#[test]
fn rejected_cleanup_cancel_is_retryable_but_cannot_reject_its_replacement_dispatch() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    f.jobs.request(&first.job.id, JobAction::Cancel).unwrap();
    let rejected = f.jobs.dispatch_rejected(&first.ticket).unwrap();
    assert_eq!(rejected.state, JobState::Failed);
    let next = f.jobs.request(&first.job.id, JobAction::Cancel).unwrap();
    assert!(next.dispatch);
    assert!(next.ticket.epoch > first.ticket.epoch);
    assert!(matches!(
        f.jobs.dispatch_rejected(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    let mut cleanup = f.jobs.claim(&next.ticket).unwrap();
    assert_eq!(f.jobs.requested(&cleanup).unwrap(), DownloadAction::Cancel);
    assert_eq!(
        f.jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn newer_signed_catalog_cannot_duplicate_a_paused_artifact_owner() {
    let f = fixture(2);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let mut run = f.jobs.claim(&first.ticket).unwrap();
    f.jobs.request(&first.job.id, JobAction::Pause).unwrap();
    f.jobs.finish_exited(&mut run).unwrap();
    let newer = catalog(17, 2, "pack", 2); // Same ID and artifact SHA, changed signed revision/version.
    let jobs = ContentJobs::open(Arc::clone(&f.store), newer).unwrap();
    assert!(matches!(jobs.start("pack-0", 2), Err(JobError::Busy)));
    assert_eq!(jobs.list().unwrap().len(), 1);
}

#[test]
fn refreshed_catalog_resume_retains_historical_signed_authority_and_rollback_floor() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let mut run = f.jobs.claim(&first.ticket).unwrap();
    f.jobs.request(&first.job.id, JobAction::Pause).unwrap();
    f.jobs.finish_exited(&mut run).unwrap();
    let before: Value = serde_json::from_slice(&fs::read(ledger(f.root.path())).unwrap()).unwrap();
    let jobs = ContentJobs::open(Arc::clone(&f.store), catalog(17, 2, "pack", 1)).unwrap();
    let floor_path = f.root.path().join("packs/managed/content.catalog");
    let floor = fs::read(&floor_path).unwrap();
    let resumed = jobs.request(&first.job.id, JobAction::Resume).unwrap();
    assert!(resumed.dispatch);
    assert_eq!(resumed.ticket.epoch, first.ticket.epoch + 1);
    let after: Value = serde_json::from_slice(&fs::read(ledger(f.root.path())).unwrap()).unwrap();
    assert_eq!(
        after["jobs"][0]["authority_fingerprint"],
        before["jobs"][0]["authority_fingerprint"]
    );
    assert_eq!(fs::read(&floor_path).unwrap(), floor);
    assert!(ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).is_err());
    assert!(matches!(
        jobs.start("pack-0", 1),
        Err(JobError::CatalogChanged)
    ));
    assert!(jobs.claim(&resumed.ticket).is_ok());
}

#[test]
fn cancelling_idle_paused_job_dispatches_a_cleanup_only_run() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let mut run = f.jobs.claim(&first.ticket).unwrap();
    f.jobs.request(&first.job.id, JobAction::Pause).unwrap();
    f.jobs.finish_exited(&mut run).unwrap();
    let cancel = f.jobs.request(&first.job.id, JobAction::Cancel).unwrap();
    assert!(cancel.dispatch);
    let mut cleanup = f.jobs.claim(&cancel.ticket).unwrap();
    assert_eq!(f.jobs.requested(&cleanup).unwrap(), DownloadAction::Cancel);
    assert_eq!(
        f.jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn cancelling_paused_history_cannot_steal_another_active_slot() {
    let f = fixture(2);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let mut run = f.jobs.claim(&first.ticket).unwrap();
    f.jobs.request(&first.job.id, JobAction::Pause).unwrap();
    f.jobs.finish_exited(&mut run).unwrap();
    let second = f.jobs.start("pack-1", 1).unwrap();
    let before = fs::read(ledger(f.root.path())).unwrap();
    assert!(matches!(
        f.jobs.request(&first.job.id, JobAction::Cancel),
        Err(JobError::Busy)
    ));
    assert_eq!(fs::read(ledger(f.root.path())).unwrap(), before);
    assert!(f.jobs.claim(&second.ticket).is_ok());
}

#[test]
fn signed_catalog_identity_is_required_and_public_snapshot_has_no_path_authority() {
    let f = fixture(2);
    let foreign = catalog(18, 1, "foreign", 1);
    assert!(ContentJobs::open(Arc::clone(&f.store), foreign).is_err());
    assert!(matches!(
        f.jobs.start("missing", 1),
        Err(JobError::NotFound)
    ));
    assert!(matches!(
        f.jobs.start("pack-0", 2),
        Err(JobError::CatalogChanged)
    ));
    let reply = f.jobs.start("pack-0", 1).unwrap();
    assert!(reply.dispatch);
    assert_eq!(reply.job.state, JobState::Queued);
    assert_eq!(reply.job.downloaded_bytes, 0);
    assert_eq!(reply.job.verified_bytes, 0);
    assert_eq!(reply.job.total_bytes, 1024);
    let value = serde_json::to_value(reply.job).unwrap();
    assert!(
        value.get("path").is_none() && value.get("url").is_none() && value.get("sha256").is_none()
    );
}

#[test]
fn concurrent_start_reserves_exactly_one_durable_slot() {
    let f = fixture(2);
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|index| {
            let jobs = Arc::clone(&f.jobs);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                jobs.start(&format!("pack-{index}"), 1)
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    let rows = f.jobs.list().unwrap();
    assert_eq!(rows.len(), 1);
    let reopened = ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).unwrap();
    assert_eq!(reopened.list().unwrap()[0].id, rows[0].id);
}

#[test]
fn pause_acknowledgment_cannot_finish_or_resume_a_worker_that_still_owns_its_run() {
    let f = fixture(2);
    let reply = f.jobs.start("pack-0", 1).unwrap();
    let run = f.jobs.claim(&reply.ticket).unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        run
    });
    ready_rx.recv().unwrap();
    let ack = f.jobs.request(&reply.job.id, JobAction::Pause).unwrap();
    assert_eq!(ack.job.state, JobState::Pausing);
    assert!(!ack.dispatch);
    assert!(matches!(
        f.jobs.request(&reply.job.id, JobAction::Resume),
        Err(JobError::Busy)
    ));
    assert!(matches!(f.jobs.start("pack-1", 1), Err(JobError::Busy)));
    let before = fs::read(ledger(f.root.path())).unwrap();
    assert!(f.jobs.reconcile_interrupted(&reply.ticket).is_err());
    let reopened = ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).unwrap();
    assert_eq!(reopened.list().unwrap()[0].state, JobState::Pausing);
    assert_eq!(fs::read(ledger(f.root.path())).unwrap(), before);
    release_tx.send(()).unwrap();
    let mut run = worker.join().unwrap(); // Native completion must return the unique run first.
    assert_eq!(
        f.jobs.finish_exited(&mut run).unwrap().state,
        JobState::Paused
    );
}

#[test]
fn cancellation_before_claim_is_cleanup_only_and_never_starts_network_work() {
    let f = fixture(2);
    let reply = f.jobs.start("pack-0", 1).unwrap();
    let ack = f.jobs.request(&reply.job.id, JobAction::Cancel).unwrap();
    assert_eq!(ack.job.state, JobState::Cancelling);
    let mut run = f.jobs.claim(&reply.ticket).unwrap();
    assert_eq!(f.jobs.requested(&run).unwrap(), DownloadAction::Cancel);
    let finished = f.jobs.finish_exited(&mut run).unwrap();
    assert_eq!(finished.state, JobState::Cancelled);
    assert_eq!(finished.downloaded_bytes, 0);
    assert_eq!(finished.verified_bytes, 0);
    assert!(f.jobs.claim(&reply.ticket).is_err());
    assert!(f.jobs.start("pack-1", 1).is_ok());
}

#[test]
fn false_finish_and_stale_epoch_or_attempt_cannot_release_a_replacement() {
    let f = fixture(2);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let mut old = f.jobs.claim(&first.ticket).unwrap();
    assert!(f.jobs.finish_exited(&mut old).is_err()); // No durable stop intent.
    assert!(f.jobs.claim(&first.ticket).is_err()); // The first run still owns its lease.
    f.jobs.request(&first.job.id, JobAction::Pause).unwrap();
    f.jobs.finish_exited(&mut old).unwrap();
    let resumed = f.jobs.request(&first.job.id, JobAction::Resume).unwrap();
    assert_eq!(resumed.job.id, first.job.id);
    assert!(resumed.ticket.epoch > first.ticket.epoch);
    assert!(matches!(
        f.jobs.claim(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    let mut current = f.jobs.claim(&resumed.ticket).unwrap();
    f.jobs.request(&first.job.id, JobAction::Cancel).unwrap();
    assert!(f.jobs.finish_exited(&mut old).is_err());
    assert_eq!(f.jobs.list().unwrap()[0].state, JobState::Cancelling);
    assert!(f.jobs.start("pack-1", 1).is_err());
    assert_eq!(
        f.jobs.finish_exited(&mut current).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn losing_an_initialized_ledger_cannot_be_mistaken_for_a_fresh_store() {
    let f = fixture(1);
    f.jobs.start("pack-0", 1).unwrap();
    let path = ledger(f.root.path());
    fs::remove_file(&path).unwrap();
    assert!(ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).is_err());
    assert!(f.jobs.list().is_err());
    assert!(!path.exists());
}

#[test]
fn cancellation_cannot_claim_cleanup_while_owned_staging_remains() {
    let f = fixture(1);
    let reply = f.jobs.start("pack-0", 1).unwrap();
    let mut run = f.jobs.claim(&reply.ticket).unwrap();
    let spec = f.store.cache_spec(&f.catalog, "pack-0").unwrap();
    let partial = f.root.path().join(format!("{}.part", spec.path));
    fs::create_dir_all(partial.parent().unwrap()).unwrap();
    fs::write(&partial, b"owned resumable bytes").unwrap();
    f.jobs.request(&reply.job.id, JobAction::Cancel).unwrap();
    assert!(f.jobs.finish_exited(&mut run).is_err());
    assert_eq!(f.jobs.list().unwrap()[0].state, JobState::Cancelling);
    assert_eq!(fs::read(partial).unwrap(), b"owned resumable bytes");
}

#[test]
fn finished_empty_cancelled_history_is_bounded_and_reusable() {
    let f = fixture(1);
    let mut first_id = None;
    for _ in 0..64 {
        let reply = f.jobs.start("pack-0", 1).unwrap();
        first_id.get_or_insert(reply.job.id.clone());
        let mut run = f.jobs.claim(&reply.ticket).unwrap();
        f.jobs.request(&reply.job.id, JobAction::Cancel).unwrap();
        f.jobs.finish_exited(&mut run).unwrap();
    }
    assert_eq!(f.jobs.list().unwrap().len(), 64);
    let next = f.jobs.start("pack-0", 1).unwrap();
    let rows = f.jobs.list().unwrap();
    assert_eq!(rows.len(), 64);
    assert!(!rows.iter().any(|row| Some(&row.id) == first_id.as_ref()));
    assert!(f
        .jobs
        .request(first_id.as_ref().unwrap(), JobAction::Cancel)
        .is_err());
    assert_eq!(rows.last().unwrap().id, next.job.id);
}

#[test]
fn resumable_history_cannot_be_silently_evicted_to_make_room() {
    let f = fixture(65);
    for index in 0..64 {
        let reply = f.jobs.start(&format!("pack-{index}"), 1).unwrap();
        let mut run = f.jobs.claim(&reply.ticket).unwrap();
        f.jobs.request(&reply.job.id, JobAction::Pause).unwrap();
        f.jobs.finish_exited(&mut run).unwrap();
    }
    let before = fs::read(ledger(f.root.path())).unwrap();
    assert!(f.jobs.start("pack-64", 1).is_err());
    assert_eq!(fs::read(ledger(f.root.path())).unwrap(), before);
}

#[test]
fn lost_unclaimed_dispatch_requires_explicit_reconciliation_then_a_new_epoch() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    let before = fs::read(ledger(f.root.path())).unwrap();
    let reopened = ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).unwrap();
    assert_eq!(reopened.list().unwrap()[0].state, JobState::Queued);
    assert_eq!(fs::read(ledger(f.root.path())).unwrap(), before);
    assert_eq!(
        reopened.reconcile_interrupted(&first.ticket).unwrap().state,
        JobState::Paused
    );
    let resumed = reopened.request(&first.job.id, JobAction::Resume).unwrap();
    assert!(resumed.dispatch);
    assert_eq!(resumed.ticket.epoch, first.ticket.epoch + 1);
    assert!(matches!(
        reopened.claim(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    assert!(matches!(
        reopened.reconcile_interrupted(&first.ticket),
        Err(JobError::StaleAttempt)
    ));
    let live = reopened.claim(&resumed.ticket).unwrap();
    assert!(matches!(
        reopened.reconcile_interrupted(&resumed.ticket),
        Err(JobError::Busy)
    ));
    drop(live);
}

#[test]
fn reconciling_unclaimed_pending_cancel_does_not_invent_cleanup_completion() {
    let f = fixture(1);
    let first = f.jobs.start("pack-0", 1).unwrap();
    f.jobs.request(&first.job.id, JobAction::Cancel).unwrap();
    assert_eq!(
        f.jobs.reconcile_interrupted(&first.ticket).unwrap().state,
        JobState::Cancelling
    );
    let mut cleanup = f.jobs.claim(&first.ticket).unwrap();
    assert_eq!(f.jobs.requested(&cleanup).unwrap(), DownloadAction::Cancel);
    assert_eq!(
        f.jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn reopening_never_recovers_or_autostarts_an_unfinished_run() {
    let f = fixture(1);
    let reply = f.jobs.start("pack-0", 1).unwrap();
    let run = f.jobs.claim(&reply.ticket).unwrap();
    drop(run); // Models process death/abandoned ownership, not an observed stopped terminal.
    let before = fs::read(ledger(f.root.path())).unwrap();
    let reopened = ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).unwrap();
    assert_eq!(reopened.list().unwrap()[0].state, JobState::Queued);
    assert_eq!(fs::read(ledger(f.root.path())).unwrap(), before);
    assert!(reopened.claim(&reply.ticket).is_err());
    assert_eq!(
        reopened.reconcile_interrupted(&reply.ticket).unwrap().state,
        JobState::Paused
    );
    let resumed = reopened.request(&reply.job.id, JobAction::Resume).unwrap();
    assert!(resumed.dispatch);
    assert!(resumed.ticket.epoch > reply.ticket.epoch);
}

#[test]
fn malformed_oversized_or_fabricated_completed_metadata_is_preserved_and_refused() {
    let f = fixture(1);
    f.jobs.start("pack-0", 1).unwrap();
    let path = ledger(f.root.path());
    let original = fs::read(&path).unwrap();
    let mut completed: Value = serde_json::from_slice(&original).unwrap();
    completed["jobs"][0]["state"] = "completed".into();
    let mut authority = completed.clone();
    authority["jobs"][0]["state"] = "queued".into();
    authority["jobs"][0]["authority_fingerprint"] = "0".repeat(64).into();
    for bytes in [
        b"not JSON".to_vec(),
        vec![b' '; 512 * 1024 + 1],
        serde_json::to_vec(&completed).unwrap(),
        serde_json::to_vec(&authority).unwrap(),
    ] {
        fs::write(&path, &bytes).unwrap();
        assert!(ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn referenced_signed_snapshot_must_match_its_fingerprint_pin_and_entry() {
    let f = fixture(1);
    f.jobs.start("pack-0", 1).unwrap();
    let path = ledger(f.root.path());
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for different in [catalog(17, 2, "other", 1), catalog(18, 2, "pack", 1)] {
        let temporary = f.root.path().join("signed-test.tcat");
        // Each test gets a separate trusted snapshot destination; no prior revision is overwritten.
        let temporary = temporary.with_extension(format!("{}.tcat", different.entries()[0].id));
        different.accept_revision(&temporary).unwrap();
        let signed = fs::read(temporary).unwrap();
        let fingerprint = format!("{:x}", Sha256::digest(&signed));
        fs::write(
            f.root
                .path()
                .join(format!("packs/managed/authority/{fingerprint}.tcat")),
            signed,
        )
        .unwrap();
        let mut changed = original.clone();
        changed["jobs"][0]["authority_fingerprint"] = fingerprint.into();
        let bytes = serde_json::to_vec(&changed).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(ContentJobs::open(Arc::clone(&f.store), Arc::clone(&f.catalog)).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
