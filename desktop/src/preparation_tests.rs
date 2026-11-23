use super::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tonalli_core::{index::jobs::State, pack::PreadSource, zim::Zim};

fn fixture() -> (
    tempfile::TempDir,
    Arc<IndexJobs>,
    Arc<PackRegistry>,
    tonalli_core::index::jobs::IndexSource,
) {
    let root = tempfile::tempdir().unwrap();
    let jobs = Arc::new(IndexJobs::open(root.path()).unwrap());
    let registry = Arc::new(PackRegistry::default());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/core/tests/fixtures/articles/benign-v1.zim");
    registry
        .register_zim(
            PackId::new("book").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
        )
        .unwrap();
    let source = registry
        .preparation_source(&PackId::new("book").unwrap())
        .unwrap();
    (root, jobs, registry, source)
}

#[test]
fn actual_native_preparation_finishes_and_a_closed_owner_rejects_new_work() {
    let (_root, jobs, registry, source) = fixture();
    let owner = DesktopPreparation::new(Arc::clone(&registry));
    let first = jobs.start(&source).unwrap();
    owner
        .dispatch(Arc::clone(&jobs), first.ticket.clone())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while jobs
        .status(&PackId::new("book").unwrap())
        .unwrap()
        .unwrap()
        .state
        != State::Ready
    {
        assert!(
            Instant::now() < deadline,
            "actual native builder must finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    owner.shutdown();
    assert!(jobs.safe_ready(&source).unwrap().is_some());
    assert!(owner.dispatch(Arc::clone(&jobs), first.ticket).is_err());
}

#[test]
fn startup_recovers_only_after_the_actual_native_lease_is_released() {
    let (_root, jobs, registry, source) = fixture();
    let first = jobs.start(&source).unwrap();
    let run = jobs.reserve(&first.ticket).unwrap();
    let owner = DesktopPreparation::new(registry);
    assert!(matches!(
        owner.reconcile_startup(&jobs),
        Err(IndexJobError::Busy)
    ));
    drop(run);
    owner.reconcile_startup(&jobs).unwrap();
    assert_eq!(
        jobs.status(&PackId::new("book").unwrap())
            .unwrap()
            .unwrap()
            .state,
        State::Paused
    );
    assert!(jobs.active_ticket().unwrap().is_none());
    owner.shutdown();
}

#[test]
fn admitted_successor_waits_for_the_previous_thread_tail_and_shutdown_confirms_pause() {
    let (_root, jobs, registry, source) = fixture();
    let first = jobs.start(&source).unwrap();
    let mut run = jobs.reserve(&first.ticket).unwrap();
    jobs.request(&first.ticket, Action::Pause).unwrap();
    jobs.execute_unbound_control(&mut run).unwrap();
    jobs.finish_exited(&mut run).unwrap();
    let (release, held) = std::sync::mpsc::sync_channel(1);
    let predecessor =
        std::thread::spawn(move || held.recv_timeout(Duration::from_secs(5)).unwrap());
    let owner = DesktopPreparation::new(registry);
    *owner.worker.lock().unwrap() = Some(predecessor);
    let next = jobs.request(&first.ticket, Action::Resume).unwrap();
    jobs.request(&next.ticket, Action::Pause).unwrap();
    owner.dispatch(Arc::clone(&jobs), next.ticket).unwrap();
    release.send(()).unwrap();
    owner.shutdown();
    assert_eq!(
        jobs.status(&PackId::new("book").unwrap())
            .unwrap()
            .unwrap()
            .state,
        State::Paused
    );
    assert!(jobs.active_ticket().unwrap().is_none());
}
