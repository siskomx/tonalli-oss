use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
    thread,
    time::Duration,
};
use tonalli_core::{
    catalog::Catalog,
    content::{ContentJobs, JobAction, JobError, JobState, ManagedStore},
    download::pack_path,
    pack::PackError,
};

fn fixture() -> (
    tempfile::TempDir,
    Arc<ManagedStore>,
    ContentJobs,
    Arc<Catalog>,
    Vec<u8>,
) {
    let root = tempfile::tempdir().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[83; 32]).unwrap();
    let body = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap();
    let payload=serde_json::to_vec(&json!({"schema_version":1,"revision":1,"entries":[{
        "id":"test-en","title":"Transport test","kind":"zim","language":"en","version":"r1","bytes":body.len(),
        "sha256":format!("{:x}",Sha256::digest(&body)),"url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
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
    (root, store, jobs, catalog, body)
}

#[test]
fn native_transport_failure_honors_late_stop_but_pause_still_requires_stage_flush() {
    for (action, bad_stage) in [
        (JobAction::Pause, false),
        (JobAction::Cancel, false),
        (JobAction::Pause, true),
    ] {
        let (root, store, jobs, catalog, body) = fixture();
        let spec = store.cache_spec(&catalog, "test-en").unwrap();
        let cache = pack_path(root.path(), &spec.path).unwrap();
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let part = PathBuf::from(format!("{}.part", cache.display()));
        if bad_stage {
            fs::create_dir(&part).unwrap();
        } else {
            fs::write(&part, &body[..64]).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let proxy = thread::spawn(move || {
            let began = std::time::Instant::now();
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && began.elapsed() < Duration::from_secs(4) =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("owned proxy admission failed: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 8192 {
                if stream.read(&mut byte).unwrap() == 0 {
                    break;
                }
                bytes.push(byte[0]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let started = jobs.start("test-en", 1).unwrap();
        let mut run = jobs.claim(&started.ticket).unwrap();
        assert!(matches!(
            jobs.execute_with_client(&mut run, &client, |_| Ok(())),
            Err(JobError::Pack(PackError::Http(_)))
        ));
        proxy.join().unwrap();
        jobs.request(&started.job.id, action).unwrap(); // Native detects network loss/cancel before its final cleanup boundary.
        let stopped = jobs.finish_exited(&mut run).unwrap();
        assert_eq!(
            stopped.state,
            if bad_stage {
                JobState::Failed
            } else if action == JobAction::Pause {
                JobState::Paused
            } else {
                JobState::Cancelled
            }
        );
        if action == JobAction::Cancel {
            assert!(!part.exists());
        } else if !bad_stage {
            assert_eq!(fs::read(&part).unwrap(), body[..64]);
            assert!(
                jobs.request(&started.job.id, JobAction::Resume)
                    .unwrap()
                    .dispatch
            );
        }
        assert!(store.list().unwrap().is_empty());
    }
}

#[test]
fn callback_http_error_retains_genuine_failure_precedence_over_late_pause() {
    let (root, store, jobs, catalog, body) = fixture();
    let spec = store.cache_spec(&catalog, "test-en").unwrap();
    let cache = pack_path(root.path(), &spec.path).unwrap();
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(cache, body).unwrap();
    let started = jobs.start("test-en", 1).unwrap();
    let mut run = jobs.claim(&started.ticket).unwrap();
    assert!(jobs
        .execute(&mut run, |_| Err(PackError::Http(
            "native checkpoint failed".into()
        )))
        .is_err());
    jobs.request(&started.job.id, JobAction::Pause).unwrap();
    assert_eq!(
        jobs.finish_exited(&mut run).unwrap().state,
        JobState::Failed
    );
}
