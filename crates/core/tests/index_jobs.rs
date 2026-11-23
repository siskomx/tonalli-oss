//! Native preparation ownership contracts. Authored tiny ZIMs; no HTTP or full corpus.
//! Real source/build/lease contracts for the native lifecycle adapter.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use tonalli_core::{
    catalog::Catalog,
    content::ManagedStore,
    index::jobs::{Action, DispatchFailure, IndexJobError, IndexJobs, IndexSource, State},
    index::{BuildControl, BuildPhase, Index},
    pack::{PackError, PackSource, PreadSource},
    server::{PackId, PackRegistry},
    zim::Zim,
};

fn archive(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles")
        .join(name)
}

fn registered(id: &str, name: &str) -> IndexSource {
    let zim = Arc::new(Zim::open(Arc::new(PreadSource::open(&archive(name)).unwrap())).unwrap());
    let registry = PackRegistry::default();
    let id = PackId::new(id).unwrap();
    registry.register_zim(id.clone(), Arc::clone(&zim)).unwrap();
    IndexSource::registered(id, zim).unwrap()
}

fn hits(index: Index) -> Vec<(String, String)> {
    let mut found = index
        .search("fixture", 20)
        .unwrap()
        .into_iter()
        .map(|hit| {
            assert!(hit.score.is_finite());
            (hit.title, hit.url)
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

fn only_current(root: &Path) -> PathBuf {
    fn visit(root: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), found);
            } else if entry.file_name() == "CURRENT" {
                found.push(entry.path());
            }
        }
    }
    let mut found = Vec::new();
    visit(root, &mut found);
    assert_eq!(
        found.len(),
        1,
        "one source fixture must own one activated namespace"
    );
    found.remove(0)
}

#[test]
fn real_pause_resume_and_restart_reopen_the_verified_ready_generation() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&first.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |progress| {
        Ok(
            if progress.phase == BuildPhase::Indexing && progress.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let paused = jobs.finish_exited(&mut run).unwrap();
    assert_eq!(paused.state, State::Paused);
    assert!(paused.progress.unwrap().committed_articles >= 1);
    assert!(jobs.safe_ready(&source).unwrap().is_none());
    let resume = jobs.request(&first.ticket, Action::Resume).unwrap();
    assert!(resume.ticket.epoch > first.ticket.epoch);
    let mut run = jobs.claim(&resume.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Ready);
    let expected = hits(jobs.safe_ready(&source).unwrap().unwrap());
    assert!(!expected.is_empty());
    drop(jobs);
    let reopened = IndexJobs::open(root.path()).unwrap();
    let reopened_source = registered("reader-fixture", "benign-v1.zim");
    assert_eq!(
        hits(reopened.safe_ready(&reopened_source).unwrap().unwrap()),
        expected
    );
}

#[test]
fn live_native_owner_blocks_duplicate_claim_rejection_and_orphan_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let admitted = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&admitted.ticket, &source).unwrap();
    assert!(matches!(
        jobs.start(&registered("other-book", "benign-v1.zim")),
        Err(IndexJobError::Busy)
    ));
    assert!(matches!(
        jobs.claim(&admitted.ticket, &source),
        Err(IndexJobError::Busy)
    ));
    assert!(matches!(
        jobs.dispatch_rejected(&admitted.ticket),
        Err(IndexJobError::Busy)
    ));
    assert!(matches!(
        jobs.reconcile_interrupted(&admitted.ticket),
        Err(IndexJobError::Busy)
    ));
    assert!(
        matches!(
            jobs.finish_exited(&mut run),
            Err(IndexJobError::InvalidRequest)
        ),
        "no caller-chosen completion proof"
    );
    jobs.execute(&mut run, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    drop(run); // Models an actually exited native owner, not a cancellation acknowledgement.
    let paused = jobs.reconcile_interrupted(&admitted.ticket).unwrap();
    assert_eq!(paused.state, State::Paused);
    let resumed = jobs.request(&admitted.ticket, Action::Resume).unwrap();
    assert!(matches!(
        jobs.request(&admitted.ticket, Action::Cancel),
        Err(IndexJobError::StaleTicket)
    ));
    let mut next = jobs.claim(&resumed.ticket, &source).unwrap();
    let mut restored_cursor = None;
    jobs.execute(&mut next, 1, |p| {
        if p.phase == BuildPhase::Indexing {
            restored_cursor = Some(p.committed_articles);
            Ok(BuildControl::Cancel)
        } else {
            Ok(BuildControl::Continue)
        }
    })
    .unwrap();
    assert!(
        restored_cursor.unwrap() >= 1,
        "orphan reconciliation must preserve the genuine committed checkpoint"
    );
    assert_eq!(
        jobs.finish_exited(&mut next).unwrap().state,
        State::Cancelled
    );
}

#[test]
fn definite_queued_pause_confirmation_needs_neither_source_nor_os_work() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let admitted = jobs.start(&source).unwrap();
    jobs.request(&admitted.ticket, Action::Pause).unwrap();
    drop(source);
    assert!(jobs
        .dispatch_failed(&admitted.ticket, DispatchFailure::Uncertain)
        .unwrap()
        .is_none());
    let uncertain = jobs
        .status(&PackId::new("reader-fixture").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        uncertain.state,
        State::Pausing,
        "an uncertain OS admission must not fabricate native exit"
    );
    // Trusted native caller has confirmed that no matching OS task was accepted.
    let stopped = jobs
        .dispatch_failed(&admitted.ticket, DispatchFailure::Rejected)
        .unwrap()
        .unwrap();
    assert_eq!(
        stopped.state,
        State::Paused,
        "must not leave a source-free queued Pause stuck Pausing"
    );
    let ticket = stopped.ticket;
    let rebound = registered("reader-fixture", "benign-v1.zim");
    let resume = jobs.request(&ticket, Action::Resume).unwrap();
    let mut run = jobs.claim(&resume.ticket, &rebound).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Ready);
}

#[test]
fn failed_checkpoint_has_explicit_resume_instead_of_a_permanently_failed_slot() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let admitted = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&admitted.ticket, &source).unwrap();
    assert!(jobs
        .execute(&mut run, 1, |progress| {
            if progress.phase == BuildPhase::Indexing && progress.processed_articles >= 1 {
                Err(PackError::Io("controlled observer failure".into()))
            } else {
                Ok(BuildControl::Continue)
            }
        })
        .is_err());
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Failed);
    let resume = jobs.request(&admitted.ticket, Action::Resume).unwrap();
    let mut next = jobs.claim(&resume.ticket, &source).unwrap();
    jobs.execute(&mut next, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    assert_eq!(jobs.finish_exited(&mut next).unwrap().state, State::Ready);
    assert!(!hits(jobs.safe_ready(&source).unwrap().unwrap()).is_empty());
}

#[test]
fn changed_source_is_refused_and_old_ready_is_not_reported_for_the_replacement() {
    let root = tempfile::tempdir().unwrap();
    let old = registered("reader-fixture", "benign-v1.zim");
    let replacement = registered("reader-fixture", "hostile-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let baseline = jobs.start(&old).unwrap();
    let mut run = jobs.claim(&baseline.ticket, &old).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    assert!(jobs.safe_ready(&old).unwrap().is_some());
    let admitted = jobs.start(&old).unwrap();
    assert!(matches!(
        jobs.claim(&admitted.ticket, &replacement),
        Err(IndexJobError::SourceChanged)
    ));
    // This trusted native claim definitively failed before any builder started.
    jobs.dispatch_rejected(&admitted.ticket).unwrap();
    assert!(jobs.safe_ready(&replacement).unwrap().is_none());
    let resumed = jobs
        .request(
            &jobs
                .status(&PackId::new("reader-fixture").unwrap())
                .unwrap()
                .unwrap()
                .ticket,
            Action::Resume,
        )
        .unwrap();
    assert!(matches!(
        jobs.claim(&resumed.ticket, &replacement),
        Err(IndexJobError::SourceChanged)
    ));
}

struct Removable {
    inner: PreadSource,
    removed: AtomicBool,
    reads: AtomicUsize,
}
impl PackSource for Removable {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        self.inner.read_at(offset, buffer)
    }
}

#[test]
fn source_free_cancel_removes_owned_stage_without_reading_unavailable_book_or_old_current() {
    let root = tempfile::tempdir().unwrap();
    let removable = Arc::new(Removable {
        inner: PreadSource::open(&archive("benign-v1.zim")).unwrap(),
        removed: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let zim = Arc::new(Zim::open(removable.clone()).unwrap());
    let source = IndexSource::registered(PackId::new("reader-fixture").unwrap(), zim).unwrap();
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&first.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let current = only_current(root.path());
    let prior = fs::read(&current).unwrap();
    let second = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&second.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    removable.removed.store(true, Ordering::SeqCst);
    let before_reads = removable.reads.load(Ordering::SeqCst);
    let cancel = jobs.request(&second.ticket, Action::Cancel).unwrap();
    let mut stop = jobs.claim_stop(&cancel.ticket).unwrap();
    jobs.execute_stop(&mut stop).unwrap();
    assert_eq!(
        jobs.finish_exited(&mut stop).unwrap().state,
        State::Cancelled
    );
    assert_eq!(
        removable.reads.load(Ordering::SeqCst),
        before_reads,
        "stop-only Cancel must not create or read a fake Zim"
    );
    assert_eq!(fs::read(&current).unwrap(), prior);
    removable.removed.store(false, Ordering::SeqCst);
    assert!(!hits(jobs.safe_ready(&source).unwrap().unwrap()).is_empty());
}

#[test]
fn crash_after_ready_before_finish_never_becomes_false_cancelled_or_permanently_busy() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let admitted = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&admitted.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    let current = only_current(root.path());
    let ready = fs::read(&current).unwrap();
    drop(run); // Activated CURRENT is real, but volatile coordinator proof was lost.
    let cancel = jobs.request(&admitted.ticket, Action::Cancel).unwrap();
    let mut stop = jobs.claim_stop(&cancel.ticket).unwrap();
    assert!(matches!(
        jobs.execute_stop(&mut stop),
        Err(IndexJobError::Pack(PackError::PublicationUncertain(_)))
    ));
    let state = jobs.finish_exited(&mut stop).unwrap();
    assert_eq!(state.state, State::Failed);
    assert_eq!(state.error_code.as_deref(), Some("publication_uncertain"));
    assert_eq!(fs::read(current).unwrap(), ready);
    assert!(!hits(jobs.safe_ready(&source).unwrap().unwrap()).is_empty());
    jobs.start(&registered("another-book", "benign-v1.zim"))
        .expect("actual stop exit must release the single active slot");
}

fn signed(revision: u64, body: &[u8], signer: &Ed25519KeyPair) -> Catalog {
    let payload = serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"managed-book","title":"Tiny managed book","kind":"zim","language":"en","version":format!("r{revision}"),
        "bytes":body.len(),"sha256":format!("{:x}",Sha256::digest(body)),"url":"https://download.kiwix.org/zim/wikipedia/test.zim"
    }]})).unwrap();
    Catalog::verify(
        &payload,
        signer.sign(&payload).as_ref(),
        signer.public_key().as_ref(),
        0,
    )
    .unwrap()
}

#[test]
fn managed_source_factory_uses_actual_installed_metadata_and_rejects_a_reopened_replacement() {
    let root = tempfile::tempdir().unwrap();
    let signer = Ed25519KeyPair::from_seed_unchecked(&[51; 32]).unwrap();
    let store = ManagedStore::open(root.path(), signer.public_key().as_ref()).unwrap();
    let first = fs::read(archive("benign-v1.zim")).unwrap();
    let first_catalog = signed(1, &first, &signer);
    let install = |catalog: &Catalog, body: &[u8]| {
        let spec = store.cache_spec(catalog, "managed-book").unwrap();
        let path = tonalli_core::download::pack_path(root.path(), &spec.path).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        store.install_cached(catalog, "managed-book").unwrap();
    };
    install(&first_catalog, &first);
    let first_source = IndexSource::managed(&store, "managed-book").unwrap(); // No expected-SHA argument exists.
    let jobs = IndexJobs::open(root.path()).unwrap();
    let admitted = jobs.start(&first_source).unwrap();
    let replacement = fs::read(archive("hostile-v1.zim")).unwrap();
    install(&signed(2, &replacement, &signer), &replacement);
    let reopened = IndexSource::managed(&store, "managed-book").unwrap();
    assert!(matches!(
        jobs.claim(&admitted.ticket, &reopened),
        Err(IndexJobError::SourceChanged)
    ));
    jobs.dispatch_rejected(&admitted.ticket).unwrap();
    assert!(jobs.safe_ready(&reopened).unwrap().is_none());
}

#[test]
fn ownership_resume_after_publication_uncertainty_can_cancel_only_its_new_stage() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&first.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    let current = only_current(root.path());
    let prior = fs::read(&current).unwrap();
    drop(run);
    let stop = jobs.request(&first.ticket, Action::Cancel).unwrap();
    let mut run = jobs.claim_stop(&stop.ticket).unwrap();
    assert!(jobs.execute_stop(&mut run).is_err());
    jobs.finish_exited(&mut run).unwrap();
    let resumed = jobs.request(&stop.ticket, Action::Resume).unwrap();
    let mut run = jobs.claim(&resumed.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let stop = jobs.request(&resumed.ticket, Action::Cancel).unwrap();
    let mut run = jobs.claim_stop(&stop.ticket).unwrap();
    jobs.execute_stop(&mut run)
        .expect("a new attempt must witness the already-active generation");
    assert_eq!(
        jobs.finish_exited(&mut run).unwrap().state,
        State::Cancelled
    );
    assert_eq!(fs::read(current).unwrap(), prior);
    assert!(!hits(jobs.safe_ready(&source).unwrap().unwrap()).is_empty());
}

#[test]
fn ownership_corrupt_namespace_cannot_redirect_source_free_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.claim(&first.ticket, &source).unwrap();
    jobs.execute(&mut run, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let other = registered("another-book", "benign-v1.zim");
    let next = jobs.start(&other).unwrap();
    let mut run = jobs.claim(&next.ticket, &other).unwrap();
    jobs.execute(&mut run, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 1 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let path = root.path().join("packs/index-jobs/jobs.json");
    let mut ledger: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    ledger["rows"][0]["namespace"] = ledger["rows"][1]["namespace"].clone();
    fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    assert!(
        jobs.request(&first.ticket, Action::Cancel).is_err(),
        "a valid-looking hash must still bind the exact source and ID"
    );
}

#[test]
fn ownership_missing_worker_witness_is_not_silently_recreated_or_ignored() {
    let root = tempfile::tempdir().unwrap();
    let jobs = IndexJobs::open(root.path()).unwrap();
    let path = root.path().join("packs/index-jobs/worker.lock");
    fs::remove_file(&path).unwrap();
    assert!(jobs
        .status(&PackId::new("reader-fixture").unwrap())
        .is_err());
    assert!(!path.exists());
}

#[test]
fn queued_cancel_controls_the_existing_native_admission_instead_of_dispatching_twice() {
    let root = tempfile::tempdir().unwrap();
    let jobs = IndexJobs::open(root.path()).unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let first = jobs.start(&source).unwrap();
    assert!(first.dispatch);
    let stopping = jobs.request(&first.ticket, Action::Cancel).unwrap();
    assert!(
        !stopping.dispatch,
        "a queued admitted native task must be controlled, not duplicated"
    );
    assert_eq!(stopping.state, State::Cancelling);
}

#[test]
fn native_reservation_owns_the_slot_before_reopening_a_book_and_pauses_without_a_source() {
    let root = tempfile::tempdir().unwrap();
    let bytes = Arc::new(PreadSource::open(&archive("benign-v1.zim")).unwrap());
    let weak = Arc::downgrade(&bytes);
    let source = IndexSource::registered(
        PackId::new("reader-fixture").unwrap(),
        Arc::new(Zim::open(bytes).unwrap()),
    )
    .unwrap();
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.reserve(&first.ticket).unwrap();
    drop(source);
    assert_eq!(jobs.active_ticket().unwrap(), Some(first.ticket.clone()));
    assert!(
        weak.upgrade().is_none(),
        "unbound reservation must retain no book handle"
    );
    assert!(matches!(
        jobs.reconcile_interrupted(&first.ticket),
        Err(IndexJobError::Busy)
    ));
    jobs.request(&first.ticket, Action::Pause).unwrap();
    assert!(jobs.execute_unbound_control(&mut run).unwrap());
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Paused);
    assert_eq!(jobs.active_ticket().unwrap(), None);
}

#[test]
fn rejected_native_binding_retains_ownership_until_cleanup_finishes() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.reserve(&first.ticket).unwrap();
    let replacement = registered("reader-fixture", "hostile-v1.zim");
    assert!(matches!(
        jobs.bind_source(&mut run, &replacement),
        Err(IndexJobError::SourceChanged)
    ));
    assert!(matches!(
        jobs.reconcile_interrupted(&first.ticket),
        Err(IndexJobError::Busy)
    ));
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Failed);
}

#[test]
fn unavailable_native_source_is_a_failure_after_actual_owner_cleanup_not_fake_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.reserve(&first.ticket).unwrap();
    jobs.fail_unbound(&mut run, IndexJobError::Pack(PackError::MediaRemoved))
        .unwrap();
    assert!(matches!(
        jobs.reconcile_interrupted(&first.ticket),
        Err(IndexJobError::Busy)
    ));
    let failed = jobs.finish_exited(&mut run).unwrap();
    assert_eq!(failed.state, State::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("media_removed"));
}

#[test]
fn bound_native_source_cannot_be_replaced_or_confirmed_stopped_without_running_the_engine() {
    let root = tempfile::tempdir().unwrap();
    let source = registered("reader-fixture", "benign-v1.zim");
    let jobs = IndexJobs::open(root.path()).unwrap();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.reserve(&first.ticket).unwrap();
    jobs.bind_source(&mut run, &source).unwrap();
    assert!(matches!(
        jobs.bind_source(&mut run, &source),
        Err(IndexJobError::InvalidRequest)
    ));
    jobs.request(&first.ticket, Action::Pause).unwrap();
    assert!(matches!(
        jobs.execute_unbound_control(&mut run),
        Err(IndexJobError::InvalidRequest)
    ));
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    assert_eq!(jobs.finish_exited(&mut run).unwrap().state, State::Paused);
}
