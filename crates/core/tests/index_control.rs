use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{fs, path::Path, sync::Arc};

use tonalli_core::{
    index::{self, BuildControl, BuildOutcome, BuildPhase, Index},
    pack::{PackError, PackSource, PreadSource},
    zim::Zim,
};

fn archive() -> Zim {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()
}

fn documents(root: &Path) -> u64 {
    let name = fs::read_to_string(root.join("CURRENT")).unwrap();
    tantivy::Index::open_in_dir(root.join(name.trim()))
        .unwrap()
        .reader()
        .unwrap()
        .searcher()
        .num_docs()
}

#[test]
fn paused_build_reopens_at_committed_cursor_without_duplicate_or_missing_documents() {
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    let paused = index::build_controlled(&zim, root.path(), 1, |progress| {
        Ok(
            if progress.phase == BuildPhase::Indexing && progress.processed_articles >= 3 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let BuildOutcome::Paused(paused) = paused else {
        panic!("expected persisted pause")
    };
    assert!(paused.committed_articles >= 3);
    assert!(!root.path().join("CURRENT").exists());
    let mut first_cursor = None;
    let result = index::build_controlled(&zim, root.path(), 2, |progress| {
        if progress.phase == BuildPhase::Indexing {
            first_cursor.get_or_insert(progress.processed_articles);
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    assert!(matches!(result, BuildOutcome::Ready(_)));
    assert_eq!(first_cursor, Some(paused.committed_articles));
    let baseline = tempfile::tempdir().unwrap();
    index::build(&zim, baseline.path(), 2).unwrap();
    assert_eq!(documents(root.path()), documents(baseline.path()));
    let hits = |path: &Path| {
        let mut hits = Index::open(path, &zim)
            .unwrap()
            .search("\"nontrivial knot\"", 20)
            .unwrap()
            .into_iter()
            .map(|hit| {
                assert!(hit.score.is_finite());
                (hit.title, hit.url)
            })
            .collect::<Vec<_>>();
        hits.sort();
        hits
    };
    assert_eq!(hits(root.path()), hits(baseline.path()));
}

#[test]
fn cancel_after_reopen_discards_only_staging_and_preserves_current() {
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    index::build(&zim, root.path(), 1).unwrap();
    let previous = fs::read(root.path().join("CURRENT")).unwrap();
    index::build_controlled(&zim, root.path(), 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let result =
        index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Cancel)).unwrap();
    assert!(matches!(result, BuildOutcome::Cancelled));
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), previous);
    assert!(!root.path().join("BUILDING").exists());
    assert!(!Index::open(root.path(), &zim)
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
}

#[test]
fn callback_failure_keeps_the_last_committed_resume_point_and_old_generation() {
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    index::build(&zim, root.path(), 1).unwrap();
    let previous = fs::read(root.path().join("CURRENT")).unwrap();
    index::build_controlled(&zim, root.path(), 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let error = index::build_controlled(&zim, root.path(), 1, |p| {
        if p.phase == BuildPhase::Indexing && p.processed_articles > 2 {
            Err(PackError::Io("checkpoint observer failed".into()))
        } else {
            Ok(BuildControl::Continue)
        }
    })
    .unwrap_err();
    assert!(matches!(error, PackError::Io(_)));
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), previous);
    assert!(matches!(
        index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Continue)).unwrap(),
        BuildOutcome::Ready(_)
    ));
    let baseline = tempfile::tempdir().unwrap();
    index::build(&zim, baseline.path(), 1).unwrap();
    assert_eq!(documents(root.path()), documents(baseline.path()));
}

#[test]
fn exact_descriptor_survives_current_change_and_does_not_retain_the_pack() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    let source = Arc::new(PreadSource::open(&path).unwrap());
    let weak = Arc::downgrade(&source);
    let zim = Zim::open(source).unwrap();
    let root = tempfile::tempdir().unwrap();
    index::build(&zim, root.path(), 1).unwrap();
    let opened = Index::open(root.path(), &zim).unwrap();
    let descriptor = opened.generation_descriptor();
    fs::write(root.path().join("CURRENT"), b"generation-does-not-exist").unwrap();
    drop(opened);
    drop(zim);
    assert!(
        weak.upgrade().is_none(),
        "descriptor must not keep a source lease"
    );
    assert!(!descriptor
        .open(&archive())
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
}

#[test]
fn resumed_stage_rejects_foreign_profile_without_replacing_its_checkpoint() {
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    index::build_controlled(&zim, root.path(), 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let pointer = fs::read(root.path().join("BUILDING")).unwrap();
    let stage = root
        .path()
        .join(std::str::from_utf8(&pointer).unwrap().trim());
    let meta_path = stage.join("tonalli-meta.json");
    let mut meta: serde_json::Value =
        serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
    meta["analyzer_revision"] = serde_json::json!(999);
    fs::write(&meta_path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(matches!(
        index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Continue)),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(fs::read(root.path().join("BUILDING")).unwrap(), pointer);
    assert!(!root.path().join("CURRENT").exists());
}

#[test]
fn forged_complete_or_cursor_cannot_activate_partial_documents() {
    for field in ["complete", "articles"] {
        let zim = archive();
        let root = tempfile::tempdir().unwrap();
        index::build_controlled(&zim, root.path(), 1, |p| {
            Ok(
                if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                    BuildControl::Pause
                } else {
                    BuildControl::Continue
                },
            )
        })
        .unwrap();
        let pointer = fs::read(root.path().join("BUILDING")).unwrap();
        let stage = root
            .path()
            .join(std::str::from_utf8(&pointer).unwrap().trim());
        let path = stage.join("meta.json");
        let mut meta: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut payload: serde_json::Value =
            serde_json::from_str(meta["payload"].as_str().unwrap()).unwrap();
        payload[field] = if field == "complete" {
            serde_json::json!(true)
        } else {
            serde_json::json!(u64::MAX)
        };
        meta["payload"] = serde_json::json!(serde_json::to_string(&payload).unwrap());
        fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(
            matches!(
                index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Continue)),
                Err(PackError::Corrupt(_))
            ),
            "forged {field} was accepted"
        );
        assert!(!root.path().join("CURRENT").exists());
        assert_eq!(fs::read(root.path().join("BUILDING")).unwrap(), pointer);
    }
}

#[test]
fn abrupt_process_exit_resumes_only_committed_documents() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "abrupt_exit_helper",
            "--ignored",
            "--test-threads=1",
        ])
        .env("TONALLI_INDEX_CRASH_ROOT", root.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    assert!(!root.path().join("CURRENT").exists());
    let mut resumed = None;
    index::build_controlled(&archive(), root.path(), 1, |p| {
        if p.phase == BuildPhase::Indexing {
            resumed.get_or_insert(p.processed_articles);
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    assert_eq!(resumed, Some(2));
    let baseline = tempfile::tempdir().unwrap();
    index::build(&archive(), baseline.path(), 1).unwrap();
    assert_eq!(documents(root.path()), documents(baseline.path()));
}

#[test]
#[ignore = "subprocess-only abrupt exit in an owned temporary build"]
fn abrupt_exit_helper() {
    let root = std::env::var_os("TONALLI_INDEX_CRASH_ROOT").expect("owned subprocess directory");
    let root = Path::new(&root);
    index::build_controlled(&archive(), root, 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    index::build_controlled(&archive(), root, 1, |p| {
        if p.phase == BuildPhase::Indexing && p.processed_articles > 2 {
            std::process::exit(73);
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    panic!("abrupt exit boundary was not reached");
}

#[test]
fn cleanup_refuses_pointer_to_a_retained_immutable_generation() {
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    index::build(&zim, root.path(), 1).unwrap();
    let old = fs::read(root.path().join("CURRENT")).unwrap();
    let descriptor = Index::open(root.path(), &zim)
        .unwrap()
        .generation_descriptor();
    index::build(&zim, root.path(), 1).unwrap();
    let current = fs::read(root.path().join("CURRENT")).unwrap();
    fs::write(root.path().join("BUILDING"), &old).unwrap();
    assert!(matches!(
        index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Cancel)),
        Err(PackError::Corrupt(_))
    ));
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), current);
    assert!(!descriptor
        .open(&zim)
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
}

#[test]
fn cleanup_recovers_a_partially_or_fully_deleted_owned_stage() {
    for partial in [true, false] {
        let zim = archive();
        let root = tempfile::tempdir().unwrap();
        index::build_controlled(&zim, root.path(), 1, |p| {
            Ok(
                if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                    BuildControl::Pause
                } else {
                    BuildControl::Continue
                },
            )
        })
        .unwrap();
        let name = fs::read_to_string(root.path().join("BUILDING")).unwrap();
        let stage = root.path().join(name.trim());
        if partial {
            fs::remove_file(stage.join("meta.json")).unwrap();
        } else {
            fs::remove_dir_all(&stage).unwrap();
        }
        assert!(matches!(
            index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Cancel)).unwrap(),
            BuildOutcome::Cancelled
        ));
        assert!(!root.path().join("BUILDING").exists());
        assert!(!stage.exists());
    }
}

#[test]
#[cfg(windows)]
fn cleanup_failure_after_current_publication_is_uncertain_and_retryable() {
    use std::os::windows::fs::OpenOptionsExt;
    let zim = archive();
    let root = tempfile::tempdir().unwrap();
    let mut held = None;
    let result = index::build_controlled(&zim, root.path(), 1, |p| {
        if p.phase == BuildPhase::Activating {
            held = Some(
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .open(root.path().join("BUILDING"))
                    .unwrap(),
            );
        }
        Ok(BuildControl::Continue)
    });
    assert!(
        matches!(result, Err(PackError::PublicationUncertain(_))),
        "post-publication cleanup failure was reported as {result:?}"
    );
    assert!(!Index::open(root.path(), &zim)
        .unwrap()
        .search("knot", 20)
        .unwrap()
        .is_empty());
    drop(held);
    assert!(matches!(
        index::build_controlled(&zim, root.path(), 1, |_| Ok(BuildControl::Continue)).unwrap(),
        BuildOutcome::Ready(_)
    ));
    assert!(!root.path().join("BUILDING").exists());
}

struct CountedSource {
    source: PreadSource,
    counting: AtomicBool,
    bytes: AtomicU64,
}
impl PackSource for CountedSource {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize, PackError> {
        let count = self.source.read_at(offset, bytes)?;
        if self.counting.load(Ordering::SeqCst) {
            self.bytes.fetch_add(count as u64, Ordering::SeqCst);
        }
        Ok(count)
    }
}

fn counted_archive() -> (Zim, Arc<CountedSource>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    let source = Arc::new(CountedSource {
        source: PreadSource::open(&path).unwrap(),
        counting: AtomicBool::new(false),
        bytes: AtomicU64::new(0),
    });
    (Zim::open(source.clone()).unwrap(), source)
}

#[test]
fn checking_pause_stops_checksum_at_a_chunk_and_retains_committed_cursor() {
    let (zim, source) = counted_archive();
    let root = tempfile::tempdir().unwrap();
    index::build_controlled(&zim, root.path(), 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let pointer = fs::read(root.path().join("BUILDING")).unwrap();
    let result = index::build_controlled(&zim, root.path(), 1, |p| {
        if p.phase == BuildPhase::Checking {
            source.counting.store(true, Ordering::SeqCst);
            if source.bytes.load(Ordering::SeqCst) >= 65536 {
                return Ok(BuildControl::Pause);
            }
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    assert!(
        matches!(result, BuildOutcome::Paused(p) if p.phase == BuildPhase::Checking && p.committed_articles == 2),
        "whole-checksum behavior ignored Pause: {result:?}"
    );
    assert!(source.bytes.load(Ordering::SeqCst) <= 65536);
    assert_eq!(fs::read(root.path().join("BUILDING")).unwrap(), pointer);
    source.counting.store(false, Ordering::SeqCst);
    let mut resumed = None;
    index::build_controlled(&zim, root.path(), 1, |p| {
        if p.phase == BuildPhase::Indexing {
            resumed = Some(p.processed_articles);
            return Ok(BuildControl::Pause);
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    assert_eq!(resumed, Some(2));
}

#[test]
fn checking_cancel_stops_checksum_at_a_chunk_and_preserves_current() {
    let (zim, source) = counted_archive();
    let root = tempfile::tempdir().unwrap();
    index::build(&zim, root.path(), 1).unwrap();
    let current = fs::read(root.path().join("CURRENT")).unwrap();
    index::build_controlled(&zim, root.path(), 1, |p| {
        Ok(
            if p.phase == BuildPhase::Indexing && p.processed_articles >= 2 {
                BuildControl::Pause
            } else {
                BuildControl::Continue
            },
        )
    })
    .unwrap();
    let result = index::build_controlled(&zim, root.path(), 1, |p| {
        if p.phase == BuildPhase::Checking {
            source.counting.store(true, Ordering::SeqCst);
            if source.bytes.load(Ordering::SeqCst) >= 65536 {
                return Ok(BuildControl::Cancel);
            }
        }
        Ok(BuildControl::Continue)
    })
    .unwrap();
    assert!(
        matches!(result, BuildOutcome::Cancelled),
        "whole-checksum behavior ignored Cancel: {result:?}"
    );
    assert!(source.bytes.load(Ordering::SeqCst) <= 65536);
    assert_eq!(fs::read(root.path().join("CURRENT")).unwrap(), current);
    assert!(!root.path().join("BUILDING").exists());
}
