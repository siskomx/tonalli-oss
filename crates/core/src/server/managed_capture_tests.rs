use super::*;
use crate::catalog::Catalog;
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
#[path = "../../tests/fixtures/models/mod.rs"]
mod model_fixture;

fn catalog(revision: u64, body: &[u8]) -> Catalog {
    catalog_kind(revision, body, "zim")
}
fn catalog_kind(revision: u64, body: &[u8], kind: &str) -> Catalog {
    let key = Ed25519KeyPair::from_seed_unchecked(&[97; 32]).unwrap();
    let payload=serde_json::to_vec(&serde_json::json!({"schema_version":1,"revision":revision,"entries":[{
        "id":"test-en","title":"Capture test","kind":kind,"language":"en","version":format!("r{revision}"),
        "bytes":body.len(),"sha256":format!("{:x}",Sha256::digest(body)),
        "url":"https://download.kiwix.org/zim/wikipedia/cached-test.zim"
    }]})).unwrap();
    Catalog::verify(
        &payload,
        key.sign(&payload).as_ref(),
        key.public_key().as_ref(),
        0,
    )
    .unwrap()
}

#[test]
fn a_leased_model_is_in_use_without_restoring_it_as_a_book() {
    let root = tempfile::tempdir().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[97; 32]).unwrap();
    let store = Arc::new(ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap());
    let body = model_fixture::structural_gguf();
    let catalog = Arc::new(catalog_kind(1, &body, "model"));
    install(&store, &catalog, &body);
    let jobs = ContentJobs::open(store.clone(), catalog).unwrap();
    let registry = PackRegistry::default();
    registry.sync_managed(&store).unwrap();
    let source = store.open_source("test-en").unwrap();
    let id = PackId::new("test-en").unwrap();
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::InUse
    );
    assert!(registry.zims.read().unwrap().is_empty());
    drop(source);
    assert_eq!(
        registry.remove_managed(&jobs, &id).unwrap(),
        RemoveOutcome::Removed
    );
}
fn install(store: &ManagedStore, catalog: &Catalog, body: &[u8]) {
    let spec = store.cache_spec(catalog, "test-en").unwrap();
    let path = crate::download::pack_path(store.root(), &spec.path).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    store.install_cached(catalog, "test-en").unwrap();
}

#[test]
fn source_and_reference_are_captured_together_before_aba_publication_check() {
    let root = tempfile::tempdir().unwrap();
    let key = Ed25519KeyPair::from_seed_unchecked(&[97; 32]).unwrap();
    let store = ManagedStore::open(root.path(), key.public_key().as_ref()).unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles");
    let a = std::fs::read(fixtures.join("benign-v1.zim")).unwrap();
    let b = std::fs::read(fixtures.join("hostile-v1.zim")).unwrap();
    let first = catalog(1, &a);
    let second = catalog(2, &b);
    install(&store, &first, &a);
    let listed = store.list().unwrap().remove(0);
    assert_eq!(listed.sha256, format!("{:x}", Sha256::digest(&a)));
    install(&store, &second, &b);
    let (source, captured) = store.open_source_with_metadata("test-en").unwrap();
    assert_eq!(captured.sha256, format!("{:x}", Sha256::digest(&b)));
    assert_eq!(source.len(), captured.bytes);
    let zim = Arc::new(Zim::open(source).unwrap());
    let expected_b = Zim::open(Arc::new(
        crate::pack::PreadSource::open(&fixtures.join("hostile-v1.zim")).unwrap(),
    ))
    .unwrap();
    assert_eq!(zim.uuid(), expected_b.uuid());
    assert_eq!(zim.stored_md5().unwrap(), expected_b.stored_md5().unwrap());
    install(&store, &first, &a); // ABA: the first reference matches again, but the retained source is B.
    let registry = PackRegistry::default();
    let id = PackId::new("test-en").unwrap();
    let mut origins = registry.managed.lock().unwrap();
    let result = registry.publish_managed(&store, &captured, &id, zim, None, &mut origins);
    assert!(
        matches!(result, Err(PackError::Mismatch(_))),
        "B must not publish tagged with A after A→B→A"
    );
    assert!(registry.zims.read().unwrap().is_empty());
}
