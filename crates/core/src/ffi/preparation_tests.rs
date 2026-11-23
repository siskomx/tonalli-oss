use super::*;
use crate::{
    ffi::{pack_close, pack_open_internal},
    index::jobs::{IndexSource, State},
    pack::PreadSource,
    server::PackId,
    zim::Zim,
};
use std::{path::PathBuf, sync::Arc};

fn archive(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles")
        .join(name)
}
fn fixture() -> (
    tempfile::TempDir,
    Arc<PreparationHandle>,
    IndexSource,
    PreparationTicket,
) {
    let root = tempfile::tempdir().unwrap();
    let owner = preparation_open(root.path().to_string_lossy().into_owned()).unwrap();
    let source = IndexSource::registered(
        PackId::new("reader-fixture").unwrap(),
        Arc::new(
            Zim::open(Arc::new(
                PreadSource::open(&archive("benign-v1.zim")).unwrap(),
            ))
            .unwrap(),
        ),
    )
    .unwrap();
    let ticket = owner.jobs.start(&source).unwrap().ticket.into();
    (root, owner, source, ticket)
}

#[test]
fn preparation_ffi_keeps_exact_epoch_ownership_and_invalidates_finished_aliases() {
    let (_root, owner, _source, ticket) = fixture();
    let run = preparation_reserve(Arc::clone(&owner), ticket.clone()).unwrap();
    assert!(preparation_reconcile(Arc::clone(&owner), ticket.clone()).is_err());
    let mut stale = ticket.clone();
    stale.epoch += 1;
    assert!(
        preparation_request_stop(Arc::clone(&owner), stale, PreparationControl::Pause).is_err()
    );
    preparation_request_stop(
        Arc::clone(&owner),
        ticket.clone(),
        PreparationControl::Pause,
    )
    .unwrap();
    assert!(preparation_run_control(Arc::clone(&run)).unwrap());
    assert!(!preparation_finish(Arc::clone(&run)).unwrap());
    assert!(preparation_finish(run).is_err());
    assert!(preparation_active_ticket(owner).unwrap().is_none());
}

#[test]
fn preparation_ffi_runs_the_real_engine_and_reopens_its_generation() {
    let (root, owner, source, ticket) = fixture();
    let run = preparation_reserve(Arc::clone(&owner), ticket).unwrap();
    assert!(!preparation_run_control(Arc::clone(&run)).unwrap());
    assert!(!preparation_bind_stored(Arc::clone(&run)).unwrap());
    let pack = pack_open_internal(archive("benign-v1.zim").to_string_lossy().into_owned()).unwrap();
    preparation_bind_pack(Arc::clone(&run), Arc::clone(&pack)).unwrap();
    assert!(preparation_bind_pack(Arc::clone(&run), Arc::clone(&pack)).is_err());
    preparation_execute(Arc::clone(&run)).unwrap();
    pack_close(pack).unwrap();
    assert!(!preparation_finish(run).unwrap());
    let reopened = preparation_open(root.path().to_string_lossy().into_owned()).unwrap();
    assert!(reopened.jobs.safe_ready(&source).unwrap().is_some());
    assert_eq!(
        reopened
            .jobs
            .status(&PackId::new("reader-fixture").unwrap())
            .unwrap()
            .unwrap()
            .state,
        State::Ready
    );
}

#[test]
fn preparation_ffi_failed_binding_and_provider_failure_cannot_publish_success() {
    let (_root, owner, _source, ticket) = fixture();
    let run = preparation_reserve(Arc::clone(&owner), ticket.clone()).unwrap();
    let pack =
        pack_open_internal(archive("hostile-v1.zim").to_string_lossy().into_owned()).unwrap();
    assert!(preparation_bind_pack(Arc::clone(&run), Arc::clone(&pack)).is_err());
    assert!(preparation_reconcile(Arc::clone(&owner), ticket.clone()).is_err());
    pack_close(pack).unwrap();
    preparation_finish(run).unwrap();
    let resumed = owner
        .jobs
        .request(&ticket.into(), crate::index::jobs::Action::Resume)
        .unwrap();
    let run = preparation_reserve(Arc::clone(&owner), resumed.ticket.into()).unwrap();
    preparation_source_failed(Arc::clone(&run), PreparationSourceFailure::MediaRemoved).unwrap();
    preparation_finish(run).unwrap();
    let state = owner
        .jobs
        .status(&PackId::new("reader-fixture").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(state.state, State::Failed);
    assert_eq!(state.error_code.as_deref(), Some("media_removed"));
}
