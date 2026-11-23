use std::{fs, sync::Arc};
use tonalli_core::{
    media::{MediaError, MediaStore},
    pack::{PackError, PackSource, PreadSource},
};

#[test]
fn managed_files_keep_originals_and_reject_stale_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.txt");
    fs::write(&original, b"Original checklist").unwrap();
    let root = dir.path().join("app");
    fs::create_dir(&root).unwrap();
    let store = MediaStore::open(&root).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "checklist.txt",
            "Trips/Weekend",
        )
        .unwrap();
    assert_eq!(file.bytes, 18);
    assert_eq!(file.version, 1);
    assert_eq!(store.read_text(&file.id).unwrap(), "Original checklist");
    assert!(matches!(
        store.rename(&file.id, 0, "different.txt"),
        Err(MediaError::Conflict)
    ));
    let renamed = store.rename(&file.id, 1, "packing.txt").unwrap();
    assert_eq!(renamed.id, file.id);
    assert_eq!(renamed.version, 2);
    assert!(matches!(
        store.edit_text(&file.id, 1, "stale"),
        Err(MediaError::Conflict)
    ));
    let edited = store.edit_text(&file.id, 2, "Water, map, book").unwrap();
    assert_eq!(edited.version, 3);
    assert!(matches!(
        store.edit_text(&file.id, 3, &"x".repeat(65537)),
        Err(MediaError::Invalid)
    ));
    assert_eq!(fs::read(&original).unwrap(), b"Original checklist");
    drop(store);
    let store = MediaStore::open(&root).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.read_text(&file.id).unwrap(), "Water, map, book");
    assert!(matches!(
        store.remove(&file.id, 2),
        Err(MediaError::Conflict)
    ));
    store.remove(&file.id, 3).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(matches!(store.source(&file.id), Err(MediaError::NotFound)));
    assert_eq!(fs::read(&original).unwrap(), b"Original checklist");
}

#[test]
fn invalid_names_truncation_and_abandoned_upload_do_not_publish() {
    struct Truncated;
    impl PackSource for Truncated {
        fn len(&self) -> u64 {
            10
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> Result<usize, PackError> {
            Ok(0)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = MediaStore::open(dir.path()).unwrap();
    for name in [
        "../notes.db",
        "/absolute.txt",
        "a\\b.txt",
        "a/b.txt",
        ".",
        "..",
        "NUL",
        "CON.txt",
        "a\u{0000}.txt",
        " trailing.txt",
        "trailing. ",
    ] {
        assert!(
            matches!(store.begin(name, ""), Err(MediaError::Invalid)),
            "{name}"
        );
    }
    for parent in ["../private", "/absolute", "a//b", "a/../b", "a\\b"] {
        assert!(
            matches!(store.begin("file.txt", parent), Err(MediaError::Invalid)),
            "{parent}"
        );
    }
    assert!(store.import(Arc::new(Truncated), "file.txt", "").is_err());
    {
        let _abandoned = store.begin("file.txt", "").unwrap();
    }
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(dir.path().join("packs/media/staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn binary_files_are_downloadable_but_never_inline_text_edits() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("image.png");
    fs::write(&original, b"not really an image").unwrap();
    let store = MediaStore::open(dir.path()).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "image.png",
            "",
        )
        .unwrap();
    assert_eq!(store.source(&file.id).unwrap().len(), 19);
    assert!(matches!(
        store.read_text(&file.id),
        Err(MediaError::Invalid)
    ));
    assert!(matches!(
        store.edit_text(&file.id, 1, "overwrite"),
        Err(MediaError::Invalid)
    ));
    assert_eq!(fs::read(&original).unwrap(), b"not really an image");
}

#[test]
fn shared_blobs_and_open_readers_survive_other_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.txt");
    fs::write(&original, b"Original").unwrap();
    let store = MediaStore::open(dir.path()).unwrap();
    let import = || {
        store
            .import(
                Arc::new(PreadSource::open(&original).unwrap()),
                "copy.txt",
                "",
            )
            .unwrap()
    };
    let first = import();
    let second = import();
    assert_ne!(first.id, second.id);
    let old = store.source(&second.id).unwrap();
    store.remove(&first.id, 1).unwrap();
    let mut exported = Vec::new();
    store.export_to(&second.id, &mut exported).unwrap();
    assert_eq!(exported, b"Original");
    store.edit_text(&second.id, 1, "Updated").unwrap();
    let mut bytes = [0; 8];
    assert_eq!(old.read_at(0, &mut bytes).unwrap(), 8);
    assert_eq!(&bytes, b"Original");
    exported.clear();
    store.export_to(&second.id, &mut exported).unwrap();
    assert_eq!(exported, b"Updated");
    assert_eq!(fs::read(original).unwrap(), b"Original");
}

#[test]
fn corrupt_catalog_is_not_reset_or_replaced() {
    let dir = tempfile::tempdir().unwrap();
    drop(MediaStore::open(dir.path()).unwrap());
    let path = dir.path().join("packs/media/catalog.sqlite");
    fs::write(&path, b"not a catalog").unwrap();
    assert!(MediaStore::open(dir.path()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"not a catalog");
}

#[test]
fn native_import_grant_is_one_use_and_wrong_claim_does_not_change_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = MediaStore::open(dir.path()).unwrap();
    let ticket = store.authorize_native_import("Trips/Weekend").unwrap();
    let path = dir.path().join("packs/media/native-import.json");
    let before = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains(&ticket));
    assert!(store.claim_native_import(&"0".repeat(64)).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(store.claim_native_import(&ticket).unwrap(), "Trips/Weekend");
    assert!(store.claim_native_import(&ticket).is_err());
    assert!(store.authorize_native_import("../private").is_err());
}

#[test]
fn stopping_native_copy_before_publication_leaves_no_installed_file() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Source(Arc<AtomicBool>);
    impl PackSource for Source {
        fn len(&self) -> u64 {
            131072
        }
        fn read_at(&self, _: u64, buf: &mut [u8]) -> Result<usize, PackError> {
            buf.fill(b'x');
            self.0.store(true, Ordering::Release);
            Ok(buf.len())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = MediaStore::open(dir.path()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    assert!(matches!(
        store.import_controlled(Arc::new(Source(stop.clone())), "copy.txt", "", || !stop
            .load(Ordering::Acquire)),
        Err(MediaError::Cancelled)
    ));
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(dir.path().join("packs/media/staging"))
            .unwrap()
            .count(),
        0
    );
}
