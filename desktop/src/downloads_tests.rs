use super::*;
use std::{sync::mpsc, time::Duration};
use tonalli_core::{
    catalog::{Catalog, PUBLISHER_PUBLIC_KEY},
    content::ManagedStore,
};

#[test]
fn startup_reconciliation_preserves_partial_bytes_and_allows_explicit_resume() {
    let root = tempfile::tempdir().expect("private fixture");
    let catalog = Arc::new(Catalog::bundled().expect("signed catalog"));
    let store = Arc::new(ManagedStore::open(root.path(), PUBLISHER_PUBLIC_KEY).expect("store"));
    let jobs = ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).expect("jobs");
    let first = jobs
        .start("wikipedia-en-knots", catalog.revision())
        .expect("admit");
    let run = jobs.claim(&first.ticket).expect("owned worker");
    let spec = store
        .cache_spec(&catalog, "wikipedia-en-knots")
        .expect("trusted cache spec");
    let part = root.path().join(format!("{}.part", spec.path));
    std::fs::create_dir_all(part.parent().expect("cache parent")).expect("cache directory");
    std::fs::write(&part, b"retained incomplete transfer bytes").expect("staging fixture");
    drop(run); // Actual native owner has exited; no completion is fabricated.
    let dispatcher = DesktopContentDispatcher::default();
    assert!(dispatcher
        .reconcile_startup(&jobs)
        .expect("recover exact orphan")
        .is_none());
    assert_eq!(
        std::fs::read(&part).expect("preserved staging"),
        b"retained incomplete transfer bytes"
    );
    assert_eq!(jobs.list().expect("ledger")[0].state, JobState::Paused);
    let next = jobs
        .request(&first.ticket.id, JobAction::Resume)
        .expect("explicit resume admitted");
    assert!(next.dispatch);
    assert_eq!(next.ticket.epoch, first.ticket.epoch + 1);
}

#[test]
fn startup_reconciliation_refuses_a_live_core_worker_and_preserves_ledger() {
    let root = tempfile::tempdir().expect("private fixture");
    let catalog = Arc::new(Catalog::bundled().expect("signed catalog"));
    let store = Arc::new(ManagedStore::open(root.path(), PUBLISHER_PUBLIC_KEY).expect("store"));
    let jobs = ContentJobs::open(store, Arc::clone(&catalog)).expect("jobs");
    let first = jobs
        .start("wikipedia-en-knots", catalog.revision())
        .expect("admit");
    let run = jobs.claim(&first.ticket).expect("live worker lease");
    let ledger = root.path().join("packs/managed/jobs.json");
    let before = std::fs::read(&ledger).expect("ledger bytes");
    assert!(matches!(
        DesktopContentDispatcher::default().reconcile_startup(&jobs),
        Err(JobError::Busy)
    ));
    assert_eq!(std::fs::read(&ledger).expect("unchanged ledger"), before);
    drop(run);
}

#[test]
fn successor_waits_for_terminal_predecessor_thread_without_rejecting_admitted_ticket() {
    let root = tempfile::tempdir().expect("private fixture");
    let catalog = Arc::new(Catalog::bundled().expect("signed bundled catalog"));
    let store =
        Arc::new(ManagedStore::open(root.path(), PUBLISHER_PUBLIC_KEY).expect("private store"));
    let jobs = Arc::new(ContentJobs::open(store, Arc::clone(&catalog)).expect("jobs"));
    let first = jobs
        .start("wikipedia-en-knots", catalog.revision())
        .expect("admit first");
    jobs.request(&first.ticket.id, JobAction::Pause)
        .expect("stop before any HTTP");
    let first_jobs = Arc::clone(&jobs);
    let first_ticket = first.ticket.clone();
    let (terminal, observed) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let predecessor = std::thread::spawn(move || {
        let mut run = first_jobs.claim(&first_ticket).expect("first claim");
        first_jobs
            .execute(&mut run, |_| Ok(()))
            .expect("early pause, no HTTP");
        let finished = first_jobs
            .finish_exited(&mut run)
            .expect("actual first terminal");
        assert_eq!(finished.state, JobState::Paused);
        terminal.send(()).expect("publish observed");
        held.recv_timeout(Duration::from_secs(5))
            .expect("bounded predecessor return fence");
    });
    observed
        .recv_timeout(Duration::from_secs(5))
        .expect("first terminal published");
    let dispatcher = DesktopContentDispatcher::default();
    *dispatcher.worker.lock().expect("worker slot") = Some(predecessor);
    let next = jobs
        .request(&first.ticket.id, JobAction::Resume)
        .expect("Core admits new epoch");
    jobs.request(&next.ticket.id, JobAction::Pause)
        .expect("second attempt also does no HTTP");
    let accepted = dispatcher.dispatch(Arc::clone(&jobs), next.ticket);
    release.send(()).expect("release actual predecessor return");
    dispatcher.shutdown();
    assert!(
        accepted.is_ok(),
        "an already admitted successor must not be rejected while predecessor returns"
    );
    let rows = jobs.list().expect("terminal ledger");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].state,
        JobState::Paused,
        "no orphaned Pausing admission"
    );
}
