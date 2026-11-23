use std::{
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tonalli_core::{
    index::{
        self,
        snapshot::{self, Snapshot},
        Index,
    },
    pack::{PackError, PackSource, PreadSource},
    zim::Zim,
};

fn zim() -> Zim {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()
}

fn snapshot() -> Snapshot {
    let archive = zim();
    Snapshot::export(&Index::open(built().path(), &archive).unwrap(), &archive).unwrap()
}

#[test]
fn native_namespace_adoption_restores_without_scheduling_or_overlapping_a_build() {
    use tonalli_core::{
        index::{
            jobs::{IndexJobError, IndexJobs, IndexSource},
            BuildControl,
        },
        server::PackId,
    };
    let snapshot = snapshot();
    let root = tempfile::tempdir().unwrap();
    let jobs = IndexJobs::open(root.path()).unwrap();
    let source =
        IndexSource::registered(PackId::new("copied-book").unwrap(), Arc::new(zim())).unwrap();
    assert!(jobs.safe_ready(&source).unwrap().is_none());
    let installed = jobs
        .install_snapshot(
            &source,
            snapshot.manifest(),
            |r| snapshot.source(&r.name),
            || true,
        )
        .unwrap()
        .unwrap();
    assert!(jobs.active_ticket().unwrap().is_none());
    let reopened = IndexJobs::open(root.path())
        .unwrap()
        .safe_ready(&source)
        .unwrap()
        .unwrap();
    assert_eq!(
        installed.search("knot", 5).unwrap(),
        reopened.search("knot", 5).unwrap()
    );
    let ticket = jobs.start(&source).unwrap().ticket;
    let mut run = jobs.claim(&ticket, &source).unwrap();
    assert!(matches!(
        jobs.install_snapshot(
            &source,
            snapshot.manifest(),
            |_| panic!("busy worker must refuse before opening transfer files"),
            || true
        ),
        Err(IndexJobError::Busy)
    ));
    jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))
        .unwrap();
    jobs.finish_exited(&mut run).unwrap();
    assert!(jobs
        .install_snapshot(
            &source,
            snapshot.manifest(),
            |r| snapshot.source(&r.name),
            || false
        )
        .unwrap()
        .is_none());
    assert!(!jobs
        .safe_ready(&source)
        .unwrap()
        .unwrap()
        .search("knot", 1)
        .unwrap()
        .is_empty());
}

#[test]
fn malformed_manifest_and_wrong_book_never_open_inputs_or_publish() {
    let snapshot = snapshot();
    let archive = zim();
    let parent = tempfile::tempdir().unwrap();
    let out = parent.path().join("not-created");
    let original = snapshot.manifest();
    assert!(snapshot::Manifest::from_slice(&vec![b' '; 1024 * 1024 + 1]).is_err());
    assert!(snapshot::Manifest::from_slice(b"{\"version\":1,\"version\":1}").is_err());
    snapshot::Manifest::from_slice(&serde_json::to_vec(original).unwrap()).unwrap();
    let mut cases = Vec::new();
    for name in [
        "../notes.db",
        "/absolute",
        "C:stream",
        "folder/file",
        "folder\\file",
        "CON.txt",
        "LPT1.idx",
        "file.",
        "CURRENT",
        ".tantivy-meta.lock",
    ] {
        let mut bad = original.clone();
        bad.files[0].name = name.into();
        cases.push(bad);
    }
    let mut duplicate = original.clone();
    duplicate.files.push(duplicate.files[0].clone());
    duplicate.files.last_mut().unwrap().name = duplicate.files[0].name.to_ascii_uppercase();
    cases.push(duplicate);
    let mut missing = original.clone();
    missing.files.retain(|f| f.name != "tonalli-meta.json");
    cases.push(missing);
    let mut overflow = original.clone();
    overflow.files[0].bytes = u64::MAX;
    cases.push(overflow);
    let mut wrong = original.clone();
    wrong.meta.zim_uuid[0] ^= 1;
    cases.push(wrong);
    let mut version = original.clone();
    version.version = 2;
    cases.push(version);
    for bad in cases {
        assert!(snapshot::install(
            &out,
            &archive,
            &bad,
            |_| panic!("invalid manifest must precede input I/O"),
            || true
        )
        .is_err());
        assert!(!out.exists());
    }
}

#[test]
fn corrupt_truncated_or_cancelled_files_keep_current_and_remove_staging() {
    let archive = zim();
    let snapshot = snapshot();
    let out = tempfile::tempdir().unwrap();
    let mut observations = 0;
    let installed = snapshot::install(
        out.path(),
        &archive,
        snapshot.manifest(),
        |r| snapshot.source(&r.name),
        || {
            observations += 1;
            true
        },
    )
    .unwrap()
    .unwrap();
    let current = fs::read(out.path().join("CURRENT")).unwrap();
    let entries = fs::read_dir(out.path()).unwrap().count();
    let first = &snapshot.manifest().files[0];
    let input = snapshot.source(&first.name).unwrap();
    let damaged = tempfile::tempdir().unwrap();
    let path = damaged.path().join("changed");
    let mut bytes = vec![0; input.len() as usize];
    input.read_at(0, &mut bytes).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, &bytes).unwrap();
    let corrupt: Arc<dyn PackSource> = Arc::new(PreadSource::open(&path).unwrap());
    assert_eq!(
        snapshot::install(
            out.path(),
            &archive,
            snapshot.manifest(),
            |r| if r.name == first.name {
                Ok(corrupt.clone())
            } else {
                snapshot.source(&r.name)
            },
            || true
        )
        .err(),
        Some(PackError::Checksum)
    );
    fs::write(&path, []).unwrap();
    let truncated: Arc<dyn PackSource> = Arc::new(PreadSource::open(&path).unwrap());
    assert_eq!(
        snapshot::install(
            out.path(),
            &archive,
            snapshot.manifest(),
            |_| Ok(truncated.clone()),
            || true
        )
        .err(),
        Some(PackError::Truncated)
    );
    let mut calls = 0;
    assert!(snapshot::install(
        out.path(),
        &archive,
        snapshot.manifest(),
        |r| snapshot.source(&r.name),
        || {
            calls += 1;
            calls < observations
        }
    )
    .unwrap()
    .is_none());
    assert_eq!(
        calls, observations,
        "cancel at final activation admission, after validation"
    );
    assert_eq!(fs::read(out.path().join("CURRENT")).unwrap(), current);
    assert_eq!(fs::read_dir(out.path()).unwrap().count(), entries);
    assert!(!installed
        .search("\"nontrivial knot\"", 5)
        .unwrap()
        .is_empty());
}

#[test]
fn changed_managed_inventory_and_active_build_are_refused() {
    use fs2::FileExt;
    use sha2::{Digest, Sha256};
    let archive = zim();
    let snapshot = snapshot();
    let out = tempfile::tempdir().unwrap();
    let installed = snapshot::install(
        out.path(),
        &archive,
        snapshot.manifest(),
        |r| snapshot.source(&r.name),
        || true,
    )
    .unwrap()
    .unwrap();
    let current = fs::read(out.path().join("CURRENT")).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(out.path().join("build.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert!(matches!(
        snapshot::install(
            out.path(),
            &archive,
            snapshot.manifest(),
            |r| snapshot.source(&r.name),
            || true
        ),
        Err(PackError::Mismatch(_))
    ));
    drop(lock);
    let bad = tempfile::tempdir().unwrap();
    let file = bad.path().join("managed");
    let bytes = b"[\"../../notes.db\"]";
    fs::write(&file, bytes).unwrap();
    let mut manifest = snapshot.manifest().clone();
    let row = manifest
        .files
        .iter_mut()
        .find(|f| f.name == ".managed.json")
        .unwrap();
    row.bytes = bytes.len() as u64;
    row.sha256 = format!("{:x}", Sha256::digest(bytes));
    assert!(matches!(
        snapshot::install(
            out.path(),
            &archive,
            &manifest,
            |r| if r.name == ".managed.json" {
                Ok(Arc::new(PreadSource::open(&file).unwrap()))
            } else {
                snapshot.source(&r.name)
            },
            || true
        ),
        Err(PackError::Corrupt(_))
    ));
    assert_eq!(fs::read(out.path().join("CURRENT")).unwrap(), current);
    assert!(!installed.search("knot", 1).unwrap().is_empty());
}
fn built() -> &'static tempfile::TempDir {
    static BUILT: OnceLock<tempfile::TempDir> = OnceLock::new();
    BUILT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        index::build(&zim(), root.path(), 1).unwrap();
        root
    })
}

#[test]
fn portable_generation_opens_queries_and_preserves_the_previous_reader() {
    let archive = zim();
    let index = Index::open(built().path(), &archive).unwrap();
    let generation = index.generation_descriptor();
    // An unrelated file beside real segment files must not become a public export.
    let selected = fs::read_to_string(built().path().join("CURRENT")).unwrap();
    fs::write(
        built().path().join(selected.trim()).join("private.txt"),
        "not index content",
    )
    .unwrap();
    let snapshot = Snapshot::export(&index, &archive).unwrap();
    assert!(!snapshot
        .manifest()
        .files
        .iter()
        .any(|f| f.name == "private.txt" || f.name.contains("lock")));
    let installed = tempfile::tempdir().unwrap();
    let first = snapshot::install(
        installed.path(),
        &archive,
        snapshot.manifest(),
        |row| snapshot.source(&row.name),
        || true,
    )
    .unwrap()
    .unwrap();
    let expected = index.search("\"nontrivial knot\"", 5).unwrap();
    assert!(!expected.is_empty());
    assert_eq!(first.search("\"nontrivial knot\"", 5).unwrap(), expected);
    let second = snapshot::install(
        installed.path(),
        &archive,
        snapshot.manifest(),
        |row| snapshot.source(&row.name),
        || true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.search("\"nontrivial knot\"", 5).unwrap(), expected);
    assert_eq!(first.search("\"nontrivial knot\"", 5).unwrap(), expected);
    assert!(generation.open(&archive).is_ok());
    assert_eq!(
        Index::open(installed.path(), &archive)
            .unwrap()
            .search("\"nontrivial knot\"", 5)
            .unwrap(),
        expected
    );
}
