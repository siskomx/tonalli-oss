use super::*;
use crate::{index, pack::PreadSource};
use std::sync::Arc;

fn archive() -> (Zim, std::sync::Weak<PreadSource>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    let source = Arc::new(PreadSource::open(&path).unwrap());
    let weak = Arc::downgrade(&source);
    (Zim::open(source).unwrap(), weak)
}

fn pause(zim: &Zim, root: &Path) {
    assert!(matches!(
        build_controlled(zim, root, 1, |p| Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            }
        ))
        .unwrap(),
        BuildOutcome::Paused(_)
    ));
}

#[test]
fn cancelled_owned_preparation_needs_no_live_zim_or_source_lease() {
    let (zim, weak) = archive();
    let root = tempfile::tempdir().unwrap();
    let meta = index::build(&zim, root.path(), 1).unwrap();
    let current = fs::read(root.path().join("CURRENT")).unwrap();
    let before = capture_preparation_current(root.path()).unwrap();
    pause(&zim, root.path());
    let stage = storage::pointed_path(root.path(), "BUILDING").unwrap();
    drop(zim);
    assert!(weak.upgrade().is_none());
    cancel_owned_preparation(root.path(), &meta, &before).unwrap();
    assert!(!root.path().join("BUILDING").exists());
    assert!(!stage.exists());
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), current);
    assert!(!Index::open(root.path(), &archive().0)
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
}

#[test]
fn cancelled_preparation_rejects_a_foreign_trusted_profile() {
    let (zim, _) = archive();
    let root = tempfile::tempdir().unwrap();
    let before = capture_preparation_current(root.path()).unwrap();
    pause(&zim, root.path());
    let pointer = fs::read(root.path().join("BUILDING")).unwrap();
    let meta = identity(&zim).unwrap();
    let mut foreign = meta.clone();
    foreign.zim_uuid[0] ^= 1;
    assert!(matches!(
        cancel_owned_preparation(root.path(), &foreign, &before),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(fs::read(root.path().join("BUILDING")).unwrap(), pointer);
    cancel_owned_preparation(root.path(), &meta, &before).unwrap();
}

#[test]
fn cancelled_preparation_never_claims_cancelled_after_publication() {
    let (zim, _) = archive();
    let root = tempfile::tempdir().unwrap();
    let before = capture_preparation_current(root.path()).unwrap();
    let meta = index::build(&zim, root.path(), 1).unwrap();
    let current = fs::read(root.path().join("CURRENT")).unwrap();
    assert!(!root.path().join("BUILDING").exists());
    assert!(matches!(
        cancel_owned_preparation(root.path(), &meta, &before),
        Err(PackError::PublicationUncertain(_))
    ));
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), current);
    assert!(!Index::open(root.path(), &zim)
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
}

#[test]
fn cancelled_preparation_refuses_an_already_active_stage_even_with_current_token() {
    let (zim, _) = archive();
    let root = tempfile::tempdir().unwrap();
    let meta = index::build(&zim, root.path(), 1).unwrap();
    let current = fs::read(root.path().join("CURRENT")).unwrap();
    let before = capture_preparation_current(root.path()).unwrap();
    fs::write(root.path().join("BUILDING"), &current).unwrap();
    assert!(matches!(
        cancel_owned_preparation(root.path(), &meta, &before),
        Err(PackError::PublicationUncertain(_))
    ));
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), current);
}
