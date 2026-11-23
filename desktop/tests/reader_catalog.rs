#![cfg(feature = "shell")]

use std::path::{Path, PathBuf};
use tonalli_desktop::content::ReaderCatalog;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/core/tests/fixtures/articles/benign-v1.zim")
}

#[test]
fn verified_sources_survive_restart_and_duplicate_open_keeps_identity() {
    let data = tempfile::tempdir().unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    let (id, zim) = catalog.add_paths(vec![fixture()]).unwrap();
    assert!(zim.title_count().unwrap() > 0);
    let before = std::fs::read(data.path().join("reader/catalog.json")).unwrap();
    let (same, _) = catalog.add_paths(vec![fixture()]).unwrap();
    assert_eq!(same, id);
    assert_eq!(catalog.entries().len(), 1);
    assert_eq!(
        std::fs::read(data.path().join("reader/catalog.json")).unwrap(),
        before
    );
    let reopened = ReaderCatalog::load(data.path()).unwrap();
    assert_eq!(reopened.entries()[0].id, id);
    assert!(
        ReaderCatalog::open_entry(&reopened.entries()[0])
            .unwrap()
            .title_count()
            .unwrap()
            > 0
    );
}

#[test]
fn corrupt_candidate_preserves_catalog_and_verified_source() {
    let data = tempfile::tempdir().unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    catalog.add_paths(vec![fixture()]).unwrap();
    let before = std::fs::read(data.path().join("reader/catalog.json")).unwrap();
    let corrupt = data.path().join("corrupt.zim");
    let mut bytes = std::fs::read(fixture()).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&corrupt, bytes).unwrap();
    assert!(catalog.add_paths(vec![corrupt]).is_err());
    assert_eq!(
        std::fs::read(data.path().join("reader/catalog.json")).unwrap(),
        before
    );
    assert_eq!(catalog.entries().len(), 1);
    assert!(ReaderCatalog::open_entry(&catalog.entries()[0]).is_ok());
}

#[test]
fn multiple_sources_remain_and_replaced_identity_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let second = data.path().join("second.zim");
    std::fs::copy(fixture(), &second).unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    catalog.add_paths(vec![fixture()]).unwrap();
    catalog.add_paths(vec![second.clone()]).unwrap();
    assert_eq!(ReaderCatalog::load(data.path()).unwrap().entries().len(), 2);
    let mut bytes = std::fs::read(&second).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(second, bytes).unwrap();
    assert!(ReaderCatalog::open_entry(&catalog.entries()[1]).is_err());
    assert!(ReaderCatalog::open_entry(&catalog.entries()[0]).is_ok());
}

#[test]
fn malformed_private_catalog_is_not_silently_reset() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join("reader")).unwrap();
    let path = data.path().join("reader/catalog.json");
    std::fs::write(&path, b"not a catalog").unwrap();
    assert!(ReaderCatalog::load(data.path()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"not a catalog");
}

fn split_fixture(folder: &Path, stem: &str) {
    let bytes = std::fs::read(fixture()).unwrap();
    let middle = bytes.len() / 2;
    std::fs::write(folder.join(format!("{stem}.zimaa")), &bytes[..middle]).unwrap();
    std::fs::write(folder.join(format!("{stem}.zimab")), &bytes[middle..]).unwrap();
}

#[test]
fn folder_opens_plain_and_split_groups_without_recursing_or_opening_iso() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::copy(fixture(), folder.path().join("plain.zim")).unwrap();
    split_fixture(folder.path(), "split");
    std::fs::copy(fixture(), folder.path().join("not-an-archive.iso")).unwrap();
    std::fs::create_dir(folder.path().join("nested")).unwrap();
    std::fs::copy(fixture(), folder.path().join("nested/hidden.zim")).unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    let opened = catalog.add_folder(folder.path()).unwrap();
    assert_eq!(opened.opened.len(), 2);
    assert_eq!(opened.failed, 0);
    assert_eq!(opened.ignored, 2);
    assert!(opened
        .opened
        .iter()
        .all(|(_, zim)| zim.title_count().unwrap() > 0));
    let ids: Vec<_> = opened.opened.iter().map(|(id, _)| id.clone()).collect();
    let before = std::fs::read(data.path().join("reader/catalog.json")).unwrap();
    let mut restored = ReaderCatalog::load(data.path()).unwrap();
    let repeated = restored.add_folder(folder.path()).unwrap();
    assert_eq!(
        repeated
            .opened
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(restored.entries().len(), 2);
    assert_eq!(
        std::fs::read(data.path().join("reader/catalog.json")).unwrap(),
        before
    );
}

#[test]
fn bad_folder_groups_do_not_rollback_good_groups_or_existing_sources() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    let (kept, _) = catalog.add_paths(vec![fixture()]).unwrap();
    std::fs::copy(fixture(), folder.path().join("a-good.zim")).unwrap();
    let mut corrupt = std::fs::read(fixture()).unwrap();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(folder.path().join("b-bad.zim"), corrupt).unwrap();
    split_fixture(folder.path(), "c-incomplete");
    std::fs::rename(
        folder.path().join("c-incomplete.zimab"),
        folder.path().join("c-incomplete.zimac"),
    )
    .unwrap();
    let result = catalog.add_folder(folder.path()).unwrap();
    assert_eq!(result.opened.len(), 1);
    assert_eq!(result.failed, 2);
    assert_eq!(catalog.entries().len(), 2);
    assert_eq!(catalog.entries()[0].id, kept);
    assert!(ReaderCatalog::open_entry(&catalog.entries()[0]).is_ok());
    assert_eq!(ReaderCatalog::load(data.path()).unwrap().entries().len(), 2);
}

#[test]
fn oversized_or_unreadable_folder_scan_preserves_catalog_before_any_group_is_added() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let mut catalog = ReaderCatalog::load(data.path()).unwrap();
    catalog.add_paths(vec![fixture()]).unwrap();
    let before = std::fs::read(data.path().join("reader/catalog.json")).unwrap();
    std::fs::copy(fixture(), folder.path().join("a-valid.zim")).unwrap();
    for index in 0..256 {
        std::fs::write(
            folder.path().join(format!("ignored-{index}.txt")),
            b"fixture",
        )
        .unwrap();
    }
    assert!(catalog.add_folder(folder.path()).is_err());
    assert!(catalog.add_folder(&folder.path().join("missing")).is_err());
    assert_eq!(
        std::fs::read(data.path().join("reader/catalog.json")).unwrap(),
        before
    );
    assert_eq!(catalog.entries().len(), 1);
}
