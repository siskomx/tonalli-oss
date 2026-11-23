//! Product boundary: committed native storage becomes readable Library content.
use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use tonalli_core::{
    catalog::Catalog,
    content::{ContentJobs, ManagedStore, RemoveOutcome},
    download::pack_path,
    pack::{PackError, PreadSource},
    server::{ContentDispatchError, ContentDispatcher, LoopbackServer, PackId, PackRegistry},
    zim::Zim,
};

fn key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[79; 32]).unwrap()
}
fn body(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles")
            .join(name),
    )
    .unwrap()
}
fn catalog(revision: u64, bytes: &[u8]) -> Arc<Catalog> {
    let signer = key();
    let payload=serde_json::to_vec(&json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"test-en","title":"Managed test archive","kind":"zim","language":"en","version":format!("r{revision}"),
        "bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes)),
        "url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
    }]})).unwrap();
    Arc::new(
        Catalog::verify(
            &payload,
            signer.sign(&payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap(),
    )
}
fn install(store: &ManagedStore, root: &std::path::Path, catalog: &Catalog, bytes: &[u8]) {
    let path = pack_path(root, &store.cache_spec(catalog, "test-en").unwrap().path).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    store.install_cached(catalog, "test-en").unwrap();
}
struct Library {
    server: LoopbackServer,
    client: Client,
    base: String,
    api: String,
    csrf: String,
    _web: tempfile::TempDir,
}
impl Library {
    fn open(registry: Arc<PackRegistry>, data: &std::path::Path) -> Self {
        let web = tempfile::tempdir().unwrap();
        fs::write(
            web.path().join("index.html"),
            "<title>Managed content test</title>",
        )
        .unwrap();
        let server =
            LoopbackServer::start(registry, data.to_path_buf(), web.path().to_path_buf()).unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let launch = client.get(server.info().launch_url).send().unwrap();
        let api = launch
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().split(';').next().unwrap())
            .find(|value| value.starts_with("tonalli_api="))
            .unwrap()
            .to_owned();
        let base = format!("http://127.0.0.1:{}", server.info().port);
        let session: Value = client
            .get(format!("{base}/api/session"))
            .header(COOKIE, &api)
            .header("sec-fetch-dest", "empty")
            .send()
            .unwrap()
            .json()
            .unwrap();
        let csrf = session["csrf"].as_str().unwrap().to_owned();
        Self {
            server,
            client,
            base,
            api,
            csrf,
            _web: web,
        }
    }
    fn get(&self, path: &str) -> Value {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .header(COOKIE, &self.api)
            .header("sec-fetch-dest", "empty")
            .header("x-tonalli-csrf", &self.csrf)
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().unwrap()
    }
    fn contains(&self, id: &str) -> bool {
        self.get("/api/packs")["packs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == id)
    }
}

#[test]
fn committed_content_syncs_across_reopened_store_owners_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let library = Library::open(Arc::clone(&registry), root.path());
    assert!(library.contains("test-en"));
    let reopened = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    registry.sync_managed(&reopened).unwrap(); // Store identity must survive a new Arc/process instance.
    assert!(library.contains("test-en"));
    let fresh = Arc::new(PackRegistry::default());
    let restart = Library::open(Arc::clone(&fresh), root.path());
    restart
        .server
        .attach_content(Arc::clone(&reopened), signed)
        .unwrap();
    assert!(
        restart.contains("test-en"),
        "native manager startup must publish installed content before ready"
    );
}

#[test]
fn catalog_refresh_publishes_committed_external_worker_store_before_ready_response() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let main = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    let registry = Arc::new(PackRegistry::default());
    let library = Library::open(registry, root.path());
    library
        .server
        .attach_content(Arc::clone(&main), Arc::clone(&signed))
        .unwrap();
    assert!(!library.contains("test-en"));
    let worker_store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    install(&worker_store, root.path(), &signed, &bytes);
    library.get("/api/content/catalog");
    assert!(
        library.contains("test-en"),
        "the main registry cannot depend on a worker-process Arc"
    );
    assert_eq!(
        library.get("/api/zim/article?pack=test-en&url=C/en/index.html")["url"],
        "C/en/index.html"
    );
}

struct NoDispatch;
impl ContentDispatcher for NoDispatch {
    fn dispatch(
        &self,
        _jobs: Arc<ContentJobs>,
        _ticket: tonalli_core::content::JobTicket,
    ) -> Result<(), ContentDispatchError> {
        panic!("read-only sync must never dispatch a native job")
    }
}

#[test]
fn completed_worker_response_never_precedes_main_library_publication() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let main = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    let library = Library::open(Arc::new(PackRegistry::default()), root.path());
    library
        .server
        .attach_content_jobs(Arc::clone(&main), Arc::clone(&signed), Arc::new(NoDispatch))
        .unwrap();
    // A distinct Store/Jobs context models UIDT's independently reopened durable state.
    // Actual Android cross-process execution remains a separate runnable-app check.
    let worker_store =
        Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    let jobs = ContentJobs::open(Arc::clone(&worker_store), Arc::clone(&signed)).unwrap();
    let request = jobs.start("test-en", 1).unwrap();
    let mut run = jobs.claim(&request.ticket).unwrap();
    let path = pack_path(
        root.path(),
        &worker_store.cache_spec(&signed, "test-en").unwrap().path,
    )
    .unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    jobs.execute(&mut run, |_| Ok(())).unwrap();
    jobs.finish_exited(&mut run).unwrap();
    assert_eq!(
        library.get("/api/content/jobs")["jobs"][0]["state"],
        "completed"
    );
    assert!(library.contains("test-en"));
}

#[test]
fn managed_update_switches_new_readers_while_retaining_the_old_positioned_reader() {
    let root = tempfile::tempdir().unwrap();
    let old = body("benign-v1.zim");
    let next = body("hostile-v1.zim");
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    install(&store, root.path(), &catalog(1, &old), &old);
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let library = Library::open(Arc::clone(&registry), root.path());
    let reader = store.open_source("test-en").unwrap();
    install(&store, root.path(), &catalog(2, &next), &next);
    registry.sync_managed(&store).unwrap();
    let expected = Zim::open(Arc::new(
        PreadSource::open(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/articles/hostile-v1.zim"),
        )
        .unwrap(),
    ))
    .unwrap()
    .title_count()
    .unwrap();
    assert_eq!(
        library.get("/api/zim/titles?pack=test-en&limit=1")["total"],
        expected
    );
    assert_eq!(reader.len(), old.len() as u64);
    let mut retained = vec![0; old.len()];
    let mut offset = 0;
    while offset < retained.len() {
        let count = reader
            .read_at(offset as u64, &mut retained[offset..])
            .unwrap();
        assert!(count > 0);
        offset += count;
    }
    assert_eq!(retained, old);
}

#[test]
fn managed_sync_never_takes_over_a_reader_or_another_store_owner() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap();
    install(&store, root.path(), &signed, &bytes);
    let registry = Arc::new(PackRegistry::default());
    let source = Arc::new(
        Zim::open(Arc::new(
            PreadSource::open(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/articles/benign-v1.zim"),
            )
            .unwrap(),
        ))
        .unwrap(),
    );
    registry
        .register_zim(PackId::new("test-en").unwrap(), source)
        .unwrap();
    assert!(matches!(
        registry.sync_managed(&store),
        Err(PackError::Mismatch(_))
    ));
    let library = Library::open(Arc::clone(&registry), root.path());
    assert!(library.contains("test-en"));
    let owned = Arc::new(PackRegistry::default());
    owned.sync_managed(&store).unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let other = ManagedStore::open(other_root.path(), key().public_key().as_ref()).unwrap();
    install(&other, other_root.path(), &signed, &bytes);
    assert!(matches!(
        owned.sync_managed(&other),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn managed_remove_releases_idle_registry_lease_and_restores_visibility_for_real_in_use() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let jobs = ContentJobs::open(Arc::clone(&store), signed).unwrap();
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let library = Library::open(Arc::clone(&registry), root.path());
    let reader = store.open_source("test-en").unwrap();
    assert_eq!(
        registry
            .remove_managed(&jobs, &PackId::new("test-en").unwrap())
            .unwrap(),
        RemoveOutcome::InUse
    );
    assert!(library.contains("test-en"));
    let mut prefix = [0; 4];
    reader.read_at(0, &mut prefix).unwrap();
    assert_eq!(&prefix, b"ZIM\x04");
    drop(reader);
    assert_eq!(
        registry
            .remove_managed(&jobs, &PackId::new("test-en").unwrap())
            .unwrap(),
        RemoveOutcome::Removed
    );
    assert!(!library.contains("test-en"));
    assert!(store.list().unwrap().is_empty());
}

fn prepared_managed_index(
    store: &ManagedStore,
    registry: &PackRegistry,
    directory: &std::path::Path,
) -> Arc<tonalli_core::index::Index> {
    let zim = Zim::open(store.open_source("test-en").unwrap()).unwrap();
    tonalli_core::index::build(&zim, directory, 1).unwrap();
    let index = Arc::new(tonalli_core::index::Index::open(directory, &zim).unwrap());
    assert!(!index.search("fixture", 5).unwrap().is_empty());
    registry
        .attach_index(PackId::new("test-en").unwrap(), Arc::clone(&index))
        .unwrap();
    index
}

#[test]
fn managed_remove_releases_an_idle_body_index_and_its_pack_leases() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let jobs = ContentJobs::open(Arc::clone(&store), signed).unwrap();
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let directory = tempfile::tempdir().unwrap();
    drop(prepared_managed_index(&store, &registry, directory.path()));
    assert_eq!(
        registry
            .remove_managed(&jobs, &PackId::new("test-en").unwrap())
            .unwrap(),
        RemoveOutcome::Removed
    );
    assert!(store.list().unwrap().is_empty());
    let library = Library::open(registry, root.path());
    assert!(!library.contains("test-en"));
}

#[test]
fn managed_remove_keeps_a_live_body_search_reader_until_it_exits() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let jobs = ContentJobs::open(Arc::clone(&store), signed).unwrap();
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let active = prepared_managed_index(&store, &registry, directory.path());
    let id = PackId::new("test-en").unwrap();
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::InUse
    );
    assert!(!active.search("fixture", 5).unwrap().is_empty());
    drop(active);
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::Removed
    );
}

#[test]
fn refused_managed_removal_restores_the_captured_index_without_following_current() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let jobs = ContentJobs::open(Arc::clone(&store), signed).unwrap();
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let directory = tempfile::tempdir().unwrap();
    drop(prepared_managed_index(&store, &registry, directory.path()));
    let independent_reader = store.open_source("test-en").unwrap();
    fs::write(
        directory.path().join("CURRENT"),
        b"generation-not-the-opened-generation",
    )
    .unwrap();
    let id = PackId::new("test-en").unwrap();
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::InUse
    );
    let library = Library::open(Arc::clone(&registry), root.path());
    assert!(
        !library.get("/api/zim/search?pack=test-en&q=fixture")["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(independent_reader);
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::Removed
    );
}

#[test]
fn refused_removal_keeps_titles_readable_when_the_saved_body_index_is_corrupt() {
    let root = tempfile::tempdir().unwrap();
    let bytes = body("benign-v1.zim");
    let signed = catalog(1, &bytes);
    let store = Arc::new(ManagedStore::open(root.path(), key().public_key().as_ref()).unwrap());
    install(&store, root.path(), &signed, &bytes);
    let jobs = ContentJobs::open(Arc::clone(&store), signed).unwrap();
    let registry = Arc::new(PackRegistry::default());
    registry.sync_managed(&store).unwrap();
    let directory = tempfile::tempdir().unwrap();
    drop(prepared_managed_index(&store, &registry, directory.path()));
    let independent_reader = store.open_source("test-en").unwrap();
    let generation = fs::read_to_string(directory.path().join("CURRENT")).unwrap();
    fs::write(
        directory
            .path()
            .join(generation.trim())
            .join("tonalli-meta.json"),
        b"{",
    )
    .unwrap();
    assert_eq!(
        registry
            .remove_managed(&jobs, &PackId::new("test-en").unwrap())
            .unwrap(),
        RemoveOutcome::InUse
    );
    let library = Library::open(Arc::clone(&registry), root.path());
    assert!(library.contains("test-en"));
    assert!(
        library.get("/api/zim/titles?pack=test-en&limit=1")["total"]
            .as_u64()
            .unwrap()
            > 0
    );
    let unavailable = library
        .client
        .get(format!(
            "{}/api/zim/search?pack=test-en&q=fixture",
            library.base
        ))
        .header(COOKIE, &library.api)
        .header("sec-fetch-dest", "empty")
        .header("x-tonalli-csrf", &library.csrf)
        .send()
        .unwrap();
    assert_eq!(unavailable.status(), 503);
    assert_eq!(
        unavailable.json::<Value>().unwrap()["error"],
        "index_unavailable"
    );
    drop(independent_reader);
}
