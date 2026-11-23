#![cfg(feature = "ffi")]
use tonalli_core::ffi::*;
use tonalli_core::pack::PackError;

#[test]
fn reader_identity_verification_is_owned_and_checksum_failures_are_typed() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles/benign-v1.zim");
    let handle = pack_open_internal(source.to_string_lossy().into_owned()).unwrap();
    let identity = zim_verify_identity(handle.clone()).expect("verified reader source");
    assert_eq!(identity.uuid.len(), 32);
    assert_eq!(identity.stored_md5.len(), 32);
    assert_eq!(zim_identity(handle.clone()).unwrap(), identity);
    pack_close(handle.clone()).unwrap();
    assert!(matches!(zim_identity(handle), Err(PackError::NotFound(_))));
    let temp = tempfile::tempdir().unwrap();
    let corrupt = temp.path().join("corrupt.zim");
    let mut bytes = std::fs::read(source).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&corrupt, bytes).unwrap();
    let handle = pack_open_internal(corrupt.to_string_lossy().into_owned()).unwrap();
    assert!(matches!(
        zim_verify_identity(handle),
        Err(PackError::Checksum)
    ));
}

#[test]
fn zim_and_index_calls_preserve_core_results() {
    use std::sync::Arc;
    use tonalli_core::{index, pack::PreadSource, zim::Zim};
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles/benign-v1.zim");
    let zim = Zim::open(Arc::new(PreadSource::open(&fixture).expect("source"))).expect("ZIM");
    let handle = pack_open_internal(fixture.to_string_lossy().into_owned()).expect("FFI open");
    assert_eq!(
        zim_suggest(handle.clone(), "English".into(), 10).expect("suggest"),
        zim.suggest("English", 10).expect("core suggest")
    );
    assert_eq!(
        zim_article(handle.clone(), "C/en/index.html".into()).expect("article"),
        zim.article("C/en/index.html").expect("core article")
    );
    let dir = tempfile::tempdir().expect("index directory");
    index::build(&zim, dir.path(), 1).expect("build");
    let expected = index::Index::open(dir.path(), &zim)
        .expect("core index")
        .search("fixture", 10)
        .expect("core search");
    let index = index_open(dir.path().to_string_lossy().into_owned(), handle).expect("FFI index");
    assert_eq!(
        index_search(index, "fixture".into(), 10).expect("FFI search"),
        expected
    );
}

#[test]
fn empty_fd_list_is_typed_error() {
    assert!(matches!(pack_open_fds(vec![]), Err(PackError::Mismatch(_))));
}

#[test]
fn folded_title_attachment_is_explicit_and_failed_replacement_preserves_reader() {
    use std::sync::Arc;
    use tonalli_core::{pack::PreadSource, zim::Zim};
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles/benign-v1.zim");
    let zim = Zim::open(Arc::new(PreadSource::open(&fixture).expect("source"))).expect("ZIM");
    let dir = tempfile::tempdir().expect("directory");
    let table = dir.path().join("titles");
    zim.build_title_index(&table).expect("build table");
    let handle = pack_open_internal(fixture.to_string_lossy().into_owned()).expect("open");
    assert!(zim_suggest(handle.clone(), "english".into(), 10)
        .expect("raw")
        .is_empty());
    zim_attach_title_index(handle.clone(), table.to_string_lossy().into_owned()).expect("attach");
    let expected = zim_suggest(handle.clone(), "english".into(), 10).expect("folded");
    assert!(!expected.is_empty());
    assert!(zim_attach_title_index(
        handle.clone(),
        dir.path().join("missing").to_string_lossy().into_owned()
    )
    .is_err());
    assert_eq!(
        zim_suggest(handle, "english".into(), 10).expect("retained"),
        expected
    );
}

#[test]
fn loopback_owner_stops_and_rejects_info_after_close() {
    let dir = tempfile::tempdir().expect("fixture");
    let web = dir.path().join("web");
    std::fs::create_dir(&web).expect("web");
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>fixture</title>",
    )
    .expect("index");
    let handle = server_start_loopback(
        dir.path().join("private").to_string_lossy().into_owned(),
        web.to_string_lossy().into_owned(),
    )
    .expect("start");
    let info = server_info(handle.clone()).expect("info");
    assert!(info
        .launch_url
        .starts_with(&format!("http://127.0.0.1:{}/", info.port)));
    server_stop_loopback(handle.clone()).expect("stop");
    assert!(matches!(
        server_info(handle.clone()),
        Err(PackError::NotFound(_))
    ));
    server_stop_loopback(handle).expect("idempotent stop");
}

#[test]
fn close_invalidates_every_alias() {
    let dir = tempfile::tempdir().expect("fixture");
    let path = dir.path().join("pack");
    std::fs::write(&path, b"fixture").expect("write");
    let handle = pack_open_internal(path.to_string_lossy().into_owned()).expect("open");
    let alias = handle.clone();
    pack_close(handle).expect("close");
    assert!(matches!(
        zim_suggest(alias, "a".into(), 1),
        Err(PackError::NotFound(_))
    ));
    std::fs::remove_file(path).expect("mapping and file released");
}

#[cfg(unix)]
#[test]
fn invalid_names_close_all_transferred_descriptors() {
    use std::os::fd::IntoRawFd;
    let a = tempfile::tempfile().expect("a").into_raw_fd();
    let b = tempfile::tempfile().expect("b").into_raw_fd();
    assert!(pack_open_fds(vec![
        PackPart {
            name: String::new(),
            fd: a
        },
        PackPart {
            name: "b.zimab".into(),
            fd: b
        }
    ])
    .is_err());
    // /proc/self/fd observes closure without reconstructing ownership of closed fds.
    for fd in [a, b] {
        assert!(!std::path::Path::new(&format!("/proc/self/fd/{fd}")).exists());
    }
}

#[test]
fn notes_wrong_passphrase_preserves_encrypted_files_and_recovery_works() {
    let dir = tempfile::tempdir().expect("fixture");
    let path = dir.path().join("notes.db").to_string_lossy().into_owned();
    let recovery = notes_create(path.clone(), "correct".into()).expect("create");
    let handle = notes_open_passphrase(path.clone(), "correct".into()).expect("unlock");
    let id = notes_add(
        handle.clone(),
        "secret title".into(),
        "private marker".into(),
    )
    .expect("add");
    drop(handle);
    let snapshot = || {
        let mut files: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").path())
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|p| std::fs::read(p).expect("read"))
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    assert!(notes_open_passphrase(path.clone(), "wrong".into()).is_err());
    assert_eq!(before, snapshot());
    assert!(before
        .iter()
        .all(|b| !b.windows(14).any(|w| w == b"private marker")));
    let recovered = notes_open_recovery(path, recovery).expect("recover");
    assert_eq!(
        notes_get(recovered, id).expect("get").body,
        "private marker"
    );
}
