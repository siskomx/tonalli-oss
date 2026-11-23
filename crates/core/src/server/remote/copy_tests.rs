use super::*;
use crate::server::LoopbackServer;

struct NotificationAdmission;
impl CopyDispatcher for NotificationAdmission {
    fn dispatch(&self, _epoch: u64, _ipv6: bool) -> Result<std::net::IpAddr, PackError> {
        Ok(std::net::Ipv4Addr::LOCALHOST.into())
    }
}
fn view(root: &Path) -> LoopbackServer {
    let web = root.join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "Owned copy view").unwrap();
    LoopbackServer::start(Arc::new(PackRegistry::default()), root.join("data"), web).unwrap()
}

#[test]
fn copy_owner_view_detach_releases_private_root_and_preserves_accepted_action() {
    let directory = tempfile::tempdir().unwrap();
    let mut first = view(directory.path());
    let owner = CopyOwner::open(
        &directory.path().join("data"),
        Arc::new(NotificationAdmission),
    )
    .unwrap();
    first.attach_copy_owner(&owner).unwrap();
    let private = Arc::downgrade(&first.state);
    let registry = Arc::downgrade(&first.state.registry);
    {
        let mut slot = owner.copies.slot.lock().unwrap();
        slot.running = true;
        slot.epoch = 1;
        slot.snapshot.state = Phase::Preparing;
    }
    first.shutdown().unwrap();
    drop(first);
    assert!(
        private.upgrade().is_none(),
        "The copy owner must not retain private Root state"
    );
    assert!(
        registry.upgrade().is_none(),
        "Only a weak public registry binding may remain"
    );
    assert!(owner.is_running().unwrap());
    assert_eq!(
        owner.copies.slot.lock().unwrap().action,
        DownloadAction::Continue,
        "View destruction must not cancel accepted native-owned work"
    );
    assert!(!owner.copies.is_closed().unwrap());
    owner.pause(owner.status().unwrap().epoch).unwrap();
    assert_eq!(
        owner.copies.slot.lock().unwrap().action,
        DownloadAction::Pause
    );
    let mut second = view(directory.path());
    second.attach_copy_owner(&owner).unwrap();
    second.state.remote.close(false); // The user's explicit disconnect still cancels.
    assert_eq!(
        owner.copies.slot.lock().unwrap().action,
        DownloadAction::Cancel
    );
    owner.close();
    assert!(second.attach_copy_owner(&owner).is_err());
    second.shutdown().unwrap();
}

#[test]
fn copy_owner_rejects_other_library_and_default_view_still_cancels() {
    let directory = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut first = view(directory.path());
    let owner = CopyOwner::open(other.path(), Arc::new(NotificationAdmission)).unwrap();
    assert!(first.attach_copy_owner(&owner).is_err());
    let copies = first.state.remote.copies().unwrap();
    first.shutdown().unwrap();
    assert!(copies.is_closed().unwrap());
    assert_eq!(copies.slot.lock().unwrap().action, DownloadAction::Cancel);
}

#[test]
fn copy_owner_stale_notification_cannot_pause_a_later_attempt() {
    let directory = tempfile::tempdir().unwrap();
    let owner = CopyOwner::open(directory.path(), Arc::new(NotificationAdmission)).unwrap();
    {
        let mut slot = owner.copies.slot.lock().unwrap();
        slot.epoch = 2;
        slot.running = true;
    }
    assert!(
        owner.pause(1).is_err(),
        "an older notification must be fenced"
    );
    assert_eq!(
        owner.copies.slot.lock().unwrap().action,
        DownloadAction::Continue
    );
    owner.pause(2).unwrap();
    assert_eq!(
        owner.copies.slot.lock().unwrap().action,
        DownloadAction::Pause
    );
}

#[test]
fn copy_owner_restore_failure_does_not_report_active_work_as_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let owner = CopyOwner::open(directory.path(), Arc::new(NotificationAdmission)).unwrap();
    std::fs::write(
        directory.path().join("packs/nearby/installed/bad.json"),
        b"invalid",
    )
    .unwrap();
    {
        let mut slot = owner.copies.slot.lock().unwrap();
        slot.running = true;
        slot.snapshot.state = Phase::Preparing;
    }
    owner
        .copies
        .restore(&PackRegistry::default(), directory.path());
    assert!(matches!(
        owner.copies.slot.lock().unwrap().snapshot.state,
        Phase::Preparing
    ));
    assert!(owner.is_running().unwrap());
}
