use std::{fs, sync::Arc};
use tonalli_core::{media::MediaStore, pack::PreadSource};

#[test]
fn exports_preserve_private_ui_and_existing_destination_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("private");
    let web = dir.path().join("web");
    fs::create_dir(&data).unwrap();
    fs::create_dir(&web).unwrap();
    let original = dir.path().join("original.txt");
    fs::write(&original, b"Original").unwrap();
    let store = MediaStore::open(&data).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "copy.txt",
            "",
        )
        .unwrap();
    for target in [data.join("notes.db"), web.join("index.html")] {
        fs::write(&target, b"Protected").unwrap();
        assert!(tonalli_desktop::media::export_copy(&data, &web, &file.id, &target).is_err());
        assert_eq!(fs::read(target).unwrap(), b"Protected");
    }
    let target = dir.path().join("saved.txt");
    fs::write(&target, b"Older copy").unwrap();
    tonalli_desktop::media::export_copy(&data, &web, &file.id, &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"Original");
    fs::write(
        data.join(format!("packs/media/blobs/{}.blob", file.sha256)),
        b"Tampered",
    )
    .unwrap();
    assert!(tonalli_desktop::media::export_copy(&data, &web, &file.id, &target).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"Original");
    assert_eq!(fs::read(original).unwrap(), b"Original");
}
