use std::fs;
use tonalli_core::reading::{Destination, Position, ProgressStore, ReadingEntry};

fn article(url: &str) -> ReadingEntry {
    ReadingEntry {
        destination: Destination::Article {
            pack: "fixture".into(),
            url: url.into(),
        },
        position: Position::Scroll { y: 450.0 },
        title: "Test article".into(),
        bookmarked: false,
    }
}
#[test]
fn progress_and_bookmarks_survive_reopen_without_touching_notes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("notes.db"),
        b"untouched encrypted vault fixture",
    )
    .unwrap();
    let store = ProgressStore::open(root.path()).unwrap();
    assert!(store.list().unwrap().is_empty());
    let mut entry = article("A/First");
    store.save(entry.clone()).unwrap();
    entry.bookmarked = true;
    store.save(entry.clone()).unwrap();
    store.save(article("A/Second")).unwrap();
    drop(store);
    let store = ProgressStore::open(root.path()).unwrap();
    let rows = store.list().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], entry);
    store.remove(&entry.destination).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(
        fs::read(root.path().join("notes.db")).unwrap(),
        b"untouched encrypted vault fixture"
    );
}
#[test]
fn invalid_locations_and_corrupt_metadata_never_replace_saved_progress() {
    let root = tempfile::tempdir().unwrap();
    let store = ProgressStore::open(root.path()).unwrap();
    store.save(article("A/First")).unwrap();
    let path = root.path().join("packs/private/reading.json");
    let before = fs::read(&path).unwrap();
    for url in [
        "https://example.org/a",
        "../notes.db",
        "/absolute",
        "A/../private",
        "A\\private",
    ] {
        assert!(store.save(article(url)).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let mut bad = article("A/First");
    bad.position = Position::Scroll { y: f64::NAN };
    assert!(store.save(bad).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::write(&path, b"not valid progress").unwrap();
    assert!(store.list().is_err());
    assert!(store.save(article("A/Second")).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not valid progress");
}

#[test]
fn embedded_document_pages_persist_and_invalid_pages_do_not_replace_progress() {
    let root = tempfile::tempdir().unwrap();
    let store = ProgressStore::open(root.path()).unwrap();
    let mut entry = article("C/books/guide.pdf");
    entry.position = Position::Page { number: 2 };
    store.save(entry.clone()).unwrap();
    assert_eq!(
        ProgressStore::open(root.path()).unwrap().list().unwrap(),
        vec![entry.clone()]
    );
    let path = root.path().join("packs/private/reading.json");
    let original = fs::read(&path).unwrap();
    for number in [0, 1_000_001] {
        entry.position = Position::Page { number };
        assert!(store.save(entry.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn embedded_book_locator_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let store = ProgressStore::open(root.path()).unwrap();
    let mut entry = article("C/books/guide.epub");
    entry.position = Position::Epub {
        location: r#"{"chapter":1,"y":120}"#.into(),
    };
    store.save(entry.clone()).unwrap();
    assert_eq!(
        ProgressStore::open(root.path()).unwrap().list().unwrap(),
        vec![entry]
    );
}

#[test]
fn history_bound_keeps_bookmarks_and_rejects_full_bookmark_loss() {
    let root = tempfile::tempdir().unwrap();
    let store = ProgressStore::open(root.path()).unwrap();
    let mut first = article("A/Bookmarked");
    first.bookmarked = true;
    store.save(first.clone()).unwrap();
    first.bookmarked = false;
    store.save(first).unwrap();
    assert!(store.list().unwrap()[0].bookmarked);
    for i in 0..128 {
        store.save(article(&format!("A/Entry{i}"))).unwrap();
    }
    let rows = store.list().unwrap();
    assert_eq!(rows.len(), 128);
    assert!(rows
        .iter()
        .any(|entry| entry.destination == article("A/Bookmarked").destination));
    assert!(!rows
        .iter()
        .any(|entry| entry.destination == article("A/Entry0").destination));
    for entry in rows {
        store.bookmark(&entry.destination, true).unwrap();
    }
    let path = root.path().join("packs/private/reading.json");
    let before = fs::read(&path).unwrap();
    assert!(store.save(article("A/Overflow")).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}
