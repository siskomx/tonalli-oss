use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Barrier, OnceLock,
};

use tempfile::TempDir;
use tonalli_core::index::{self, Index, IndexMeta};
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::zim::Zim;

fn pack_path(es: bool) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs")
        .join(if es {
            "wikipedia_es_climate-change_maxi_2026-10.zim"
        } else {
            "wikipedia_en_knots_maxi_2026-10.zim"
        })
}

fn zim(es: bool) -> Zim {
    Zim::open(Arc::new(PreadSource::open(&pack_path(es)).unwrap())).unwrap()
}

fn built_index() -> &'static TempDir {
    static BUILT: OnceLock<TempDir> = OnceLock::new();
    BUILT.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap();
        index::build(&zim(false), directory.path(), 2).unwrap();
        directory
    })
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        // Windows CopyFile can deny write sharing while it reads a source file.
        // These runtime locks are recreated by Tantivy, not part of a snapshot.
        if name == ".tantivy-meta.lock" || name == ".tantivy-writer.lock" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

fn installed_copy() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    copy_tree(built_index().path(), directory.path());
    directory
}

fn generation(root: &Path) -> PathBuf {
    root.join(fs::read_to_string(root.join("CURRENT")).unwrap().trim())
}

#[test]
fn search_finds_known_phrase() {
    let archive = zim(false);
    let article = archive.article("C/Trefoil_knot").unwrap();
    assert!(String::from_utf8(article.bytes)
        .unwrap()
        .contains("nontrivial"));
    let search = Index::open(built_index().path(), &archive).unwrap();
    let hits = search.search("\"nontrivial knot\"", 20).unwrap();
    assert!(hits.iter().any(|hit| hit.url == "C/Trefoil_knot"));
    assert!(hits
        .iter()
        .all(|hit| !hit.title.is_empty() && hit.score.is_finite()));
    assert!(!hits.iter().any(|hit| hit.url == "C/(2,3)-torus_knot"));
}

#[test]
fn blank_query_and_zero_limit_return_no_hits() {
    let search = Index::open(built_index().path(), &zim(false)).unwrap();
    assert!(search.search(" \n", 10).unwrap().is_empty());
    assert!(search.search("knot", 0).unwrap().is_empty());
    assert!(matches!(
        search.search("\"unfinished", 10),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn unsafe_query_sizes_and_recursive_syntax_are_rejected_before_tantivy() {
    let search = Index::open(built_index().path(), &zim(false)).unwrap();
    // The first RED case is intentionally small; it must fail before this test
    // reaches inputs that would be dangerous in an unbounded implementation.
    assert!(matches!(
        search.search("knot", 1001),
        Err(PackError::Mismatch(_))
    ));
    assert!(matches!(
        search.search("knot", usize::MAX),
        Err(PackError::Mismatch(_))
    ));
    for query in [
        "knot ".repeat(1000),
        format!("{}knot{}", "(".repeat(1000), ")".repeat(1000)),
        format!("{}knot", "NOT ".repeat(500)),
        format!(
            "{}{}knot{}",
            "NOT ".repeat(20),
            "(".repeat(20),
            ")".repeat(20)
        ),
    ] {
        assert!(matches!(
            search.search(&query, 10),
            Err(PackError::Mismatch(_))
        ));
    }
    // Ordinary short quoted phrases remain supported under the textual budget.
    assert!(search
        .search(&format!("\"{}\"", "(NOT) ".repeat(4)), 10)
        .is_ok());
    assert!(search
        .search(&format!("\"{}\"", "\\(NOT\\) ".repeat(4)), 10)
        .is_ok());
}

#[test]
fn open_with_other_zim_is_mismatch() {
    assert!(matches!(
        Index::open(built_index().path(), &zim(true)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn open_with_same_uuid_different_stored_md5_is_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("changed.zim");
    fs::copy(pack_path(false), &path).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 1]).unwrap();
    drop(file);
    let changed = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    assert_eq!(changed.uuid(), zim(false).uuid());
    assert!(matches!(
        Index::open(built_index().path(), &changed),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn verify_md5_rejects_zim_whose_content_does_not_match_stored_md5() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("changed.zim");
    fs::copy(pack_path(false), &path).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.seek(SeekFrom::Start(1000)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::Start(1000)).unwrap();
    file.write_all(&[byte[0] ^ 1]).unwrap();
    drop(file);
    let changed = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    assert_eq!(changed.verify_md5(), Err(PackError::Checksum));
}

#[test]
fn open_with_different_format_version_is_mismatch() {
    let directory = installed_copy();
    let path = generation(directory.path()).join("tonalli-meta.json");
    let mut meta: IndexMeta = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta.format_version += 1;
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

fn mutate_metadata(root: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let path = generation(root).join("tonalli-meta.json");
    let mut meta = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut meta);
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
}

#[test]
fn metadata_analyzer_revision_mismatch_is_refused() {
    let directory = installed_copy();
    mutate_metadata(directory.path(), |meta| {
        meta["analyzer_revision"] = serde_json::json!(999)
    });
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn metadata_std_unicode_mismatch_is_refused() {
    let directory = installed_copy();
    mutate_metadata(directory.path(), |meta| {
        meta["std_unicode_version"] = serde_json::json!([16, 0, 0])
    });
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn metadata_legacy_profile_is_refused() {
    let directory = installed_copy();
    mutate_metadata(directory.path(), |meta| {
        meta["format_version"] = serde_json::json!(1);
        meta.as_object_mut().unwrap().remove("analyzer_revision");
        meta.as_object_mut().unwrap().remove("std_unicode_version");
    });
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn metadata_v2_missing_analysis_fields_is_mismatch_but_missing_original_fields_is_corrupt() {
    for field in ["analyzer_revision", "std_unicode_version", "zim_uuid"] {
        let directory = installed_copy();
        mutate_metadata(directory.path(), |meta| {
            meta.as_object_mut().unwrap().remove(field);
        });
        let result = Index::open(directory.path(), &zim(false));
        if field == "zim_uuid" {
            assert!(matches!(result, Err(PackError::Corrupt(_))));
        } else {
            assert!(matches!(result, Err(PackError::Mismatch(_))));
        }
    }
}

#[test]
fn metadata_analysis_identity_is_written_and_checked_before_tantivy_files() {
    let directory = installed_copy();
    let metadata = generation(directory.path()).join("tonalli-meta.json");
    let meta: IndexMeta = serde_json::from_slice(&fs::read(metadata).unwrap()).unwrap();
    let (major, minor, patch) = char::UNICODE_VERSION;
    assert_eq!(meta.format_version, 2);
    assert_eq!(meta.analyzer_revision, 1);
    assert_eq!(meta.std_unicode_version, [major, minor, patch]);
    mutate_metadata(directory.path(), |meta| {
        meta["analyzer_revision"] = serde_json::json!(999)
    });
    fs::remove_file(generation(directory.path()).join("meta.json")).unwrap();
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn open_with_different_tantivy_version_is_mismatch() {
    let directory = installed_copy();
    let path = generation(directory.path()).join("tonalli-meta.json");
    let mut meta: IndexMeta = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta.tantivy_version = "different-build".into();
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn corrupt_index_segment_returns_error_not_empty_results() {
    let directory = installed_copy();
    let segment = fs::read_dir(generation(directory.path()))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "term")
        })
        .unwrap();
    fs::write(segment, b"corrupt segment").unwrap();
    let result =
        Index::open(directory.path(), &zim(false)).and_then(|search| search.search("knot", 10));
    assert!(matches!(result, Err(PackError::Corrupt(_))));
}

struct Removed {
    inner: Arc<dyn PackSource>,
    removed: AtomicBool,
}
impl PackSource for Removed {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            Err(PackError::MediaRemoved)
        } else {
            self.inner.read_at(off, buf)
        }
    }
}

#[test]
fn search_on_removed_source_stays_poisoned_until_reopened() {
    let source = Arc::new(Removed {
        inner: Arc::new(PreadSource::open(&pack_path(false)).unwrap()),
        removed: AtomicBool::new(false),
    });
    let archive = Zim::open(source.clone()).unwrap();
    let search = Index::open(built_index().path(), &archive).unwrap();
    source.removed.store(true, Ordering::SeqCst);
    assert!(matches!(
        search.search("knot", 10),
        Err(PackError::MediaRemoved)
    ));
    source.removed.store(false, Ordering::SeqCst);
    assert!(matches!(
        search.search("knot", 10),
        Err(PackError::MediaRemoved)
    ));
    assert!(!Index::open(built_index().path(), &archive)
        .unwrap()
        .search("knot", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn failed_build_keeps_installed_generation() {
    let directory = installed_copy();
    let before = fs::read(directory.path().join("CURRENT")).unwrap();
    let source = Arc::new(Removed {
        inner: Arc::new(PreadSource::open(&pack_path(false)).unwrap()),
        removed: AtomicBool::new(false),
    });
    let archive = Zim::open(source.clone()).unwrap();
    source.removed.store(true, Ordering::SeqCst);
    assert!(matches!(
        index::build(&archive, directory.path(), 2),
        Err(PackError::MediaRemoved)
    ));
    assert_eq!(fs::read(directory.path().join("CURRENT")).unwrap(), before);
    assert!(!Index::open(directory.path(), &zim(false))
        .unwrap()
        .search("knot", 10)
        .unwrap()
        .is_empty());
}

struct StagingFailure {
    inner: Arc<dyn PackSource>,
    root: PathBuf,
    generations_before: usize,
    reads_left: AtomicUsize,
    failed: AtomicBool,
}
impl PackSource for StagingFailure {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        let generations = fs::read_dir(&self.root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .count();
        if generations > self.generations_before
            && self
                .reads_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_err()
        {
            self.failed.store(true, Ordering::SeqCst);
            return Err(PackError::Io(
                "injected failure during staged article iteration".into(),
            ));
        }
        self.inner.read_at(off, buf)
    }
}

#[test]
fn failure_during_staged_iteration_preserves_pointer_and_old_reader() {
    let directory = installed_copy();
    let archive = zim(false);
    let old = Index::open(directory.path(), &archive).unwrap();
    let before = fs::read(directory.path().join("CURRENT")).unwrap();
    let source = Arc::new(StagingFailure {
        inner: Arc::new(PreadSource::open(&pack_path(false)).unwrap()),
        root: directory.path().to_owned(),
        generations_before: fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .count(),
        reads_left: AtomicUsize::new(20),
        failed: AtomicBool::new(false),
    });
    let failing = Zim::open(source.clone()).unwrap();
    assert!(matches!(
        index::build(&failing, directory.path(), 2),
        Err(PackError::Io(_))
    ));
    assert!(
        source.failed.load(Ordering::SeqCst),
        "failure must occur after staging begins"
    );
    assert_eq!(fs::read(directory.path().join("CURRENT")).unwrap(), before);
    assert!(!old.search("knot", 10).unwrap().is_empty());
    assert!(!Index::open(directory.path(), &archive)
        .unwrap()
        .search("knot", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn successful_rebuild_keeps_old_index_handle_usable() {
    let directory = installed_copy();
    let archive = zim(false);
    let old = Index::open(directory.path(), &archive).unwrap();
    let before = fs::read(directory.path().join("CURRENT")).unwrap();
    index::build(&archive, directory.path(), 2).unwrap();
    assert_ne!(fs::read(directory.path().join("CURRENT")).unwrap(), before);
    assert!(!old.search("trefoil", 5).unwrap().is_empty());
    assert!(!Index::open(directory.path(), &archive)
        .unwrap()
        .search("trefoil", 5)
        .unwrap()
        .is_empty());
}

fn concurrent_readers(
    root: &Path,
    barrier: Arc<Barrier>,
    passes: usize,
) -> Vec<std::thread::JoinHandle<Result<(), PackError>>> {
    let archive = Arc::new(zim(false));
    (0..8)
        .map(|_| {
            let root = root.to_owned();
            let archive = Arc::clone(&archive);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..passes {
                    let index = Index::open(&root, &archive)?;
                    assert!(!index.search("trefoil", 1)?.is_empty());
                }
                Ok(())
            })
        })
        .collect()
}

#[test]
fn concurrent_opens_of_an_immutable_generation_succeed() {
    let root = built_index().path();
    let barrier = Arc::new(Barrier::new(9));
    let readers = concurrent_readers(root, Arc::clone(&barrier), 100);
    barrier.wait();
    for reader in readers {
        reader.join().unwrap().unwrap();
    }
}

#[test]
fn snapshot_copy_does_not_interfere_with_concurrent_opens() {
    let root = built_index().path();
    let barrier = Arc::new(Barrier::new(9));
    let readers = concurrent_readers(root, Arc::clone(&barrier), 100);
    barrier.wait();
    for _ in 0..100 {
        let snapshot = installed_copy();
        assert!(!generation(snapshot.path())
            .join(".tantivy-meta.lock")
            .exists());
        assert!(!generation(snapshot.path())
            .join(".tantivy-writer.lock")
            .exists());
    }
    for reader in readers {
        reader.join().unwrap().unwrap();
    }
}

#[test]
fn zero_threads_and_escaping_generation_paths_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        index::build(&zim(false), directory.path(), 0),
        Err(PackError::Mismatch(_))
    ));
    fs::write(directory.path().join("CURRENT"), "../outside").unwrap();
    assert!(matches!(
        Index::open(directory.path(), &zim(false)),
        Err(PackError::Corrupt(_))
    ));
}

#[test]
#[ignore = "explicit persistent small-index artifact for Task 4 portability checks"]
fn build_small_fixture_artifact_and_record_queries() {
    let root = std::env::var_os("TONALLI_INDEX_ARTIFACT")
        .map(PathBuf::from)
        .expect("set TONALLI_INDEX_ARTIFACT to an explicit new v2 output directory");
    assert!(
        !root.exists(),
        "artifact output must be new; never replace historical generations"
    );
    let archive = zim(false);
    let started = std::time::Instant::now();
    let meta = index::build(&archive, &root, 2).unwrap();
    println!(
        "index_build_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let search = Index::open(&root, &archive).unwrap();
    println!("index_path={} identity={meta:?}", root.display());
    for query in ["\"nontrivial knot\"", "trefoil", "\"overhand knot\""] {
        println!(
            "query={query:?} hits={:?}",
            search.search(query, 5).unwrap()
        );
    }
}
