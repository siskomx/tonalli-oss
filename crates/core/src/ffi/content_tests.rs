use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

fn fixture() -> (tempfile::TempDir, Arc<ContentHandle>, JobTicket) {
    let root = tempfile::tempdir().expect("private root");
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("test key");
    let raw = br#"{"schema_version":1,"revision":1,"entries":[{"id":"authored","title":"Authored","kind":"zim","language":"en","version":"1","bytes":100,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","url":"https://download.kiwix.org/test-only.zim"}]}"#;
    let catalog = Arc::new(
        Catalog::verify(raw, key.sign(raw).as_ref(), key.public_key().as_ref(), 0)
            .expect("signed test catalog"),
    );
    let store =
        Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).expect("store"));
    let jobs = Arc::new(ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).expect("jobs"));
    let ticket = jobs.start("authored", 1).expect("admit").ticket;
    (
        root,
        Arc::new(ContentHandle {
            store,
            catalog,
            jobs,
        }),
        ticket,
    )
}

#[test]
fn linear_ffi_run_rejects_networkless_continue_then_confirms_exact_stop_only_once() {
    let (_root, owner, ticket) = fixture();
    let run = content_claim(Arc::clone(&owner), ticket.clone().into()).expect("claim");
    assert!(matches!(
        content_execute_stop(Arc::clone(&run)),
        Err(ContentError::InvalidRequest)
    ));
    let stale = ContentTicket {
        id: ticket.id.clone(),
        epoch: ticket.epoch + 1,
    };
    assert!(matches!(
        content_request_stop(Arc::clone(&owner), stale, ContentControl::Pause),
        Err(ContentError::StaleAttempt)
    ));
    assert!(content_run_needs_network(Arc::clone(&run)).expect("continue"));
    content_request_stop(Arc::clone(&owner), ticket.into(), ContentControl::Pause).expect("pause");
    assert!(!content_run_needs_network(Arc::clone(&run)).expect("stopping"));
    content_execute_stop(Arc::clone(&run)).expect("no-network stop");
    content_finish(Arc::clone(&run)).expect("exit confirmed");
    assert!(matches!(
        content_finish(run),
        Err(ContentError::StaleAttempt)
    ));
    assert!(content_active_ticket(owner).expect("idle").is_none());
}

struct Scheduler;
impl ContentScheduler for Scheduler {
    fn dispatch(&self, _: Arc<ContentHandle>, _: ContentTicket) -> ContentDispatch {
        ContentDispatch::Accepted
    }
    fn control(
        &self,
        _: Arc<ContentHandle>,
        _: ContentTicket,
        _: ContentControl,
    ) -> ContentDispatch {
        ContentDispatch::Accepted
    }
}

#[test]
fn foreign_dispatch_owner_uses_the_exact_core_supplied_jobs_arc() {
    let (_root, owner, _) = fixture();
    let other = Arc::new(
        ContentJobs::open(Arc::clone(&owner.store), Arc::clone(&owner.catalog))
            .expect("same verified factory"),
    );
    let dispatcher = Dispatcher {
        owner,
        scheduler: Arc::new(Scheduler),
    };
    let received = dispatcher.owner(Arc::clone(&other));
    assert!(Arc::ptr_eq(&received.jobs, &other));
    assert!(!Arc::ptr_eq(&received.jobs, &dispatcher.owner.jobs));
}
