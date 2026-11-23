use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

fn archive(name: &str) -> Vec<u8> {
    fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles")
            .join(name),
    )
    .unwrap()
}
fn signed(id: &str, revision: u64, body: &[u8]) -> Arc<Catalog> {
    let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
    let payload=serde_json::to_vec(&serde_json::json!({"schema_version":1,"revision":revision,"entries":[{
        "id":id,"title":"Synthetic archive","kind":"zim","language":"en","version":format!("r{revision}"),
        "bytes":body.len(),"sha256":managed::fingerprint(body),"url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
    }]})).unwrap();
    Arc::new(
        Catalog::verify(
            &payload,
            key.sign(&payload).as_ref(),
            key.public_key().as_ref(),
            0,
        )
        .unwrap(),
    )
}
fn stage(store: &ManagedStore, catalog: &Catalog, body: &[u8]) -> PathBuf {
    let id = &catalog.entries()[0].id;
    let path = pack_path(store.root(), &store.cache_spec(catalog, id).unwrap().path).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn cancel_after_real_precommit_failure_removes_only_its_unpublished_candidate() {
    for late_cancel in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
        let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
        let old_body = archive("benign-v1.zim");
        let next_body = archive("hostile-v1.zim");
        let old = signed("test-en", 1, &old_body);
        stage(&store, &old, &old_body);
        store.install_cached(&old, "test-en").unwrap();
        let old_reader = store.open_source("test-en").unwrap();
        let other = signed("other-en", 1, &next_body);
        stage(&store, &other, &next_body);
        store.install_cached(&other, "other-en").unwrap();
        let installed_before = store.list().unwrap();
        let update = signed("test-en", 2, &next_body);
        let jobs = ContentJobs::open(Arc::clone(&store), Arc::clone(&update)).unwrap();
        let first = jobs.start("test-en", 2).unwrap();
        let mut failed_run = jobs.claim(&first.ticket).unwrap();
        let cache = stage(&store, &update, &next_body);
        let error = store
            .install_admitted_with(
                &update,
                "test-en",
                |_| Ok(u64::MAX),
                |_, _| Err(PackError::Io("injected precommit failure".into())),
                || Ok(ControlFlow::Continue(())),
            )
            .err()
            .unwrap();
        failed_run.proof = Some(ExitProof::Failed(JobError::Pack(error)));
        assert_eq!(
            jobs.finish_exited(&mut failed_run).unwrap().state,
            JobState::Failed
        );
        let candidate = store.version_path(&update.entries()[0]).unwrap();
        assert!(
            !cache.exists() && candidate.exists(),
            "real Store path moved the verified cache before ledger failure"
        );
        let mut unrelated = update.entries()[0].clone();
        unrelated.sha256 = "b".repeat(64);
        let unrelated_path = store.version_path(&unrelated).unwrap();
        fs::write(&unrelated_path, b"unrelated history sentinel").unwrap();
        let request = jobs
            .request(
                &first.job.id,
                if late_cancel {
                    JobAction::Retry
                } else {
                    JobAction::Cancel
                },
            )
            .unwrap();
        let mut run = jobs.claim(&request.ticket).unwrap();
        if !late_cancel {
            assert!(
                jobs.finish_exited(&mut run).is_err(),
                "unexecuted policy shortcut cannot confirm cleanup with an unpublished candidate"
            );
            assert_eq!(
                jobs.list()
                    .unwrap()
                    .iter()
                    .find(|row| row.id == first.job.id)
                    .unwrap()
                    .state,
                JobState::Cancelling
            );
        }
        if late_cancel {
            jobs.request(&first.job.id, JobAction::Pause).unwrap();
        }
        jobs.execute(&mut run, |_| Ok(())).unwrap();
        if late_cancel {
            jobs.request(&first.job.id, JobAction::Cancel).unwrap();
        }
        assert!(
            candidate.exists(),
            "native worker exit still precedes candidate cleanup"
        );
        assert_eq!(
            jobs.finish_exited(&mut run).unwrap().state,
            JobState::Cancelled
        );
        assert!(
            !candidate.exists(),
            "cancel must remove this exact unpublished immutable candidate"
        );
        assert_eq!(store.list().unwrap(), installed_before);
        assert_eq!(
            fs::read(store.version_path(&old.entries()[0]).unwrap()).unwrap(),
            old_body
        );
        assert_eq!(
            fs::read(store.version_path(&other.entries()[0]).unwrap()).unwrap(),
            next_body
        );
        assert_eq!(
            fs::read(unrelated_path).unwrap(),
            b"unrelated history sentinel"
        );
        let mut prefix = [0; 4];
        old_reader.read_at(0, &mut prefix).unwrap();
        assert_eq!(&prefix, b"ZIM\x04");
    }
}

#[test]
fn missing_candidate_is_idempotent_but_failed_candidate_cleanup_is_never_cancelled() {
    for invalid_shape in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
        let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
        let body = archive("benign-v1.zim");
        let input = signed("test-en", 1, &body);
        let jobs = ContentJobs::open(Arc::clone(&store), Arc::clone(&input)).unwrap();
        let job = jobs.start("test-en", 1).unwrap();
        let mut failed = jobs.claim(&job.ticket).unwrap();
        stage(&store, &input, &body);
        let error = store
            .install_admitted_with(
                &input,
                "test-en",
                |_| Ok(u64::MAX),
                |_, _| Err(PackError::Io("injected precommit failure".into())),
                || Ok(ControlFlow::Continue(())),
            )
            .err()
            .unwrap();
        failed.proof = Some(ExitProof::Failed(JobError::Pack(error)));
        jobs.finish_exited(&mut failed).unwrap();
        let candidate = store.version_path(&input.entries()[0]).unwrap();
        fs::remove_file(&candidate).unwrap();
        if invalid_shape {
            fs::create_dir(&candidate).unwrap();
        }
        let request = jobs.request(&job.job.id, JobAction::Cancel).unwrap();
        let mut cleanup = jobs.claim(&request.ticket).unwrap();
        jobs.execute(&mut cleanup, |_| Ok(())).unwrap();
        if invalid_shape {
            assert!(
                jobs.finish_exited(&mut cleanup).is_err(),
                "invalid candidate deletion must remain unconfirmed"
            );
            assert_eq!(jobs.list().unwrap()[0].state, JobState::Cancelling);
            assert!(candidate.is_dir());
            fs::remove_dir(&candidate).unwrap();
            fs::write(&candidate, &body).unwrap(); // Restore only this test-owned exact candidate.
        }
        assert_eq!(
            jobs.finish_exited(&mut cleanup).unwrap().state,
            JobState::Cancelled
        );
        assert!(!candidate.exists());
        assert!(store.list().unwrap().is_empty());
    }
}

#[test]
fn cancel_preserves_installed_same_sha_even_when_its_signed_authority_changed() {
    let root = tempfile::tempdir().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
    let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
    let body = archive("benign-v1.zim");
    let original = signed("test-en", 1, &body);
    let jobs = ContentJobs::open(Arc::clone(&store), Arc::clone(&original)).unwrap();
    let job = jobs.start("test-en", 1).unwrap();
    jobs.request(&job.job.id, JobAction::Pause).unwrap();
    let mut paused = jobs.claim(&job.ticket).unwrap();
    jobs.execute(&mut paused, |_| Ok(())).unwrap();
    jobs.finish_exited(&mut paused).unwrap();
    let newer = signed("test-en", 2, &body);
    stage(&store, &newer, &body);
    store.install_cached(&newer, "test-en").unwrap();
    let before = store.list().unwrap();
    assert_ne!(
        before[0].authority_fingerprint,
        managed::fingerprint(&original.snapshot_for_key(&store.pin).unwrap())
    );
    let cancel = jobs.request(&job.job.id, JobAction::Cancel).unwrap();
    let mut cleanup = jobs.claim(&cancel.ticket).unwrap();
    jobs.execute(&mut cleanup, |_| Ok(())).unwrap();
    assert_eq!(
        jobs.finish_exited(&mut cleanup).unwrap().state,
        JobState::Cancelled
    );
    assert_eq!(store.list().unwrap(), before);
    assert_eq!(
        fs::read(store.version_path(&newer.entries()[0]).unwrap()).unwrap(),
        body
    );
}

#[test]
fn uncertain_store_exit_reconciles_committed_metadata_or_fails_without_fabricating_install() {
    for committed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
        let body = fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/articles/benign-v1.zim"),
        )
        .unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({"schema_version":1,"revision":1,"entries":[{
            "id":"test-en","title":"Synthetic archive","kind":"zim","language":"en","version":"r1",
            "bytes":body.len(),"sha256":managed::fingerprint(&body),
            "url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
        }]})).unwrap();
        let catalog = Arc::new(
            Catalog::verify(
                &payload,
                key.sign(&payload).as_ref(),
                key.public_key().as_ref(),
                0,
            )
            .unwrap(),
        );
        let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
        let jobs = ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).unwrap();
        let job = jobs.start("test-en", 1).unwrap();
        let mut run = jobs.claim(&job.ticket).unwrap();
        let path = pack_path(store.root(), &run.spec.path).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &body).unwrap();
        let error = store
            .install_admitted_with(
                &catalog,
                "test-en",
                |_| Ok(u64::MAX),
                |path, bytes| {
                    if committed {
                        managed::publish_with_flush(path, bytes, |_, _| {
                            Err(std::io::Error::other("injected postrename flush failure"))
                        })
                    } else {
                        Err(PackError::PublicationUncertain(
                            "injected pre-ledger uncertainty".into(),
                        ))
                    }
                },
                || Ok(ControlFlow::Continue(())),
            )
            .err()
            .unwrap();
        assert!(matches!(error, PackError::PublicationUncertain(_)));
        run.proof = Some(ExitProof::Uncertain(error)); // The real controlled Store result, not a native completion flag.
        let finished = jobs.finish_exited(&mut run).unwrap();
        assert_eq!(
            finished.state,
            if committed {
                JobState::Completed
            } else {
                JobState::Failed
            }
        );
        assert_eq!(store.list().unwrap().len(), usize::from(committed));
        if !committed {
            assert!(
                !path.exists(),
                "verified immutable version already owns the moved cache"
            );
            let retry = jobs.request(&job.job.id, JobAction::Retry).unwrap();
            let mut retry_run = jobs.claim(&retry.ticket).unwrap();
            jobs.execute(&mut retry_run, |_| Ok(())).unwrap();
            assert_eq!(
                jobs.finish_exited(&mut retry_run).unwrap().state,
                JobState::Completed
            );
        }
    }
}
