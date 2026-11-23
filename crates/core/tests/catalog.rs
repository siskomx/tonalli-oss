use std::path::Path;

use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use tonalli_core::{catalog::Catalog, pack::PackError};

#[test]
fn bundled_catalog_authenticates_both_launch_languages() {
    let catalog = Catalog::bundled().unwrap();
    assert!(catalog.entries().iter().any(|entry| entry.language == "en"));
    assert!(catalog.entries().iter().any(|entry| entry.language == "es"));
    assert!(catalog.revision() > 0);
}

#[test]
fn bundled_catalog_offers_a_model_small_enough_for_a_4gb_phone() {
    let catalog = Catalog::bundled().unwrap();
    assert!(catalog.revision() >= 3);
    let entry = catalog
        .entries()
        .iter()
        .find(|entry| entry.id == "qwen3-0_6b-q4-0")
        .expect("smallest model is listed");
    assert_eq!(entry.kind, tonalli_core::catalog::CatalogKind::Model);
    assert_eq!(entry.bytes, 428_970_080);
    assert_eq!(
        entry.sha256,
        "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4"
    );
    assert!(entry.url.starts_with("https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/afe8ad3577aa22500b3cd1c880f153b3b0e1cae8/"));
}

// Synthetic test-only key. This is never a publisher key or trusted application pin.
fn key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap()
}

fn payload(revision: u64) -> Value {
    json!({
        "schema_version": 1,
        "revision": revision,
        "entries": [{
            "id": "wikipedia-en",
            "title": "Wikipedia (English)",
            "kind": "zim",
            "language": "en",
            "version": "2026-10",
            "bytes": 1024,
            "sha256": "a".repeat(64),
            "url": "https://download.kiwix.org/zim/wikipedia/example.zim"
        }]
    })
}

fn verify(value: &Value, highest_revision: u64) -> Result<Catalog, PackError> {
    let bytes = serde_json::to_vec(value).unwrap();
    let signer = key();
    Catalog::verify(
        &bytes,
        signer.sign(&bytes).as_ref(),
        signer.public_key().as_ref(),
        highest_revision,
    )
}

#[test]
fn valid_signed_catalog_exposes_owned_metadata_without_local_paths() {
    let mut raw = payload(12);
    let catalog = verify(&raw, 11).unwrap();
    raw["entries"][0]["title"] = "Changed after verification".into();
    assert_eq!(catalog.revision(), 12);
    assert_eq!(catalog.entries().len(), 1);
    let entry = &catalog.entries()[0];
    assert_eq!(entry.id, "wikipedia-en");
    assert_eq!(entry.title, "Wikipedia (English)");
    assert_eq!(entry.bytes, 1024);
    assert_eq!(entry.sha256, "a".repeat(64));
    assert_eq!(
        entry.url,
        "https://download.kiwix.org/zim/wikipedia/example.zim"
    );
}

#[test]
fn authentication_precedes_parsing_and_binds_the_exact_payload_bytes() {
    let signer = key();
    let bytes = serde_json::to_vec(&payload(1)).unwrap();
    let signature = signer.sign(&bytes);
    let mut changed = bytes.clone();
    changed.push(b' '); // Still valid JSON; the detached signature binds these bytes too.
    assert!(matches!(
        Catalog::verify(
            &changed,
            signature.as_ref(),
            signer.public_key().as_ref(),
            0
        ),
        Err(PackError::Checksum)
    ));
    let invalid_json = b"not JSON";
    assert!(matches!(
        Catalog::verify(
            invalid_json,
            signature.as_ref(),
            signer.public_key().as_ref(),
            0
        ),
        Err(PackError::Checksum)
    ));
    assert!(matches!(
        Catalog::verify(
            invalid_json,
            signer.sign(invalid_json).as_ref(),
            signer.public_key().as_ref(),
            0
        ),
        Err(PackError::Corrupt(_))
    ));
}

#[test]
fn wrong_key_or_malformed_signature_is_refused() {
    let bytes = serde_json::to_vec(&payload(1)).unwrap();
    let signer = key();
    let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    for signature in [
        vec![],
        vec![0; 63],
        vec![0; 65],
        other.sign(&bytes).as_ref().to_vec(),
    ] {
        assert!(matches!(
            Catalog::verify(&bytes, &signature, signer.public_key().as_ref(), 0),
            Err(PackError::Checksum)
        ));
    }
    assert!(matches!(
        Catalog::verify(
            &bytes,
            signer.sign(&bytes).as_ref(),
            other.public_key().as_ref(),
            0
        ),
        Err(PackError::Checksum)
    ));
}

#[test]
fn catalog_size_count_and_revision_are_bounded() {
    let signer = key();
    let oversized = vec![b' '; 1024 * 1024 + 1];
    assert!(matches!(
        Catalog::verify(&oversized, &[], signer.public_key().as_ref(), 0),
        Err(PackError::Mismatch(_))
    ));
    assert!(matches!(
        verify(&payload(0), 0),
        Err(PackError::Mismatch(_))
    ));
    assert!(matches!(
        verify(&payload(4), 5),
        Err(PackError::Mismatch(_))
    ));
    assert!(verify(&payload(5), 5).is_ok());
    let overflow = serde_json::to_string(&payload(1))
        .unwrap()
        .replace("\"revision\":1", "\"revision\":18446744073709551616");
    assert!(Catalog::verify(
        overflow.as_bytes(),
        signer.sign(overflow.as_bytes()).as_ref(),
        signer.public_key().as_ref(),
        0
    )
    .is_err());
    let mut many = payload(1);
    let entry = many["entries"][0].clone();
    many["entries"] = Value::Array(
        (0..257)
            .map(|i| {
                let mut next = entry.clone();
                next["id"] = format!("book-{i}").into();
                next
            })
            .collect(),
    );
    assert!(matches!(verify(&many, 0), Err(PackError::Mismatch(_))));
}

#[test]
fn unknown_fields_duplicates_and_unsafe_entry_values_are_refused() {
    let mut unknown = payload(1);
    unknown["unrecognized"] = true.into();
    assert!(verify(&unknown, 0).is_err());
    let mut unknown = payload(1);
    unknown["entries"][0]["path"] = "../../private/notes.db".into();
    assert!(verify(&unknown, 0).is_err());
    let mut duplicate = payload(1);
    duplicate["entries"]
        .as_array_mut()
        .unwrap()
        .push(payload(1)["entries"][0].clone());
    assert!(matches!(verify(&duplicate, 0), Err(PackError::Mismatch(_))));
    for (field, bad) in [
        ("id", json!("../private")),
        ("id", json!("bad id")),
        ("id", json!("x".repeat(129))),
        ("title", json!("")),
        ("title", json!("bad\nlabel")),
        ("title", json!("x".repeat(513))),
        ("kind", json!("arbitrary-file")),
        ("language", json!("../../en")),
        ("version", json!("")),
        ("version", json!("x".repeat(129))),
        ("bytes", json!(0)),
        ("bytes", json!(-1)),
        ("sha256", json!("g".repeat(64))),
        ("sha256", json!("a".repeat(63))),
        ("url", json!("http://download.kiwix.org/a.zim")),
        ("url", json!("https://user:password@example.com/a.zim")),
        ("url", json!("https://example.com/a.zim?token=secret")),
        ("url", json!("https://example.com/a.zim#fragment")),
        ("url", json!("https:///missing-host")),
        ("url", json!("https://example.com/\nfile")),
    ] {
        let mut bad_entry = payload(1);
        bad_entry["entries"][0][field] = bad;
        assert!(verify(&bad_entry, 0).is_err(), "accepted invalid {field}");
    }
}

fn accept(path: &Path, revision: u64) -> Result<(), PackError> {
    verify(&payload(revision), 0)?.accept_revision(path)
}

#[test]
fn persisted_revision_is_rechecked_before_accepting_a_stale_verified_catalog() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("accepted-revision.json");
    let stale = verify(&payload(1), 0).unwrap();
    accept(&path, 2).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        stale.accept_revision(&path),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    accept(&path, 2).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut conflicting = payload(2);
    conflicting["entries"][0]["sha256"] = "b".repeat(64).into();
    assert!(matches!(
        verify(&conflicting, 0).unwrap().accept_revision(&path),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    accept(&path, 3).unwrap();
    // A fresh caller with stale in-memory state still cannot downgrade the disk record.
    assert!(matches!(accept(&path, 2), Err(PackError::Mismatch(_))));
}

#[test]
fn invalid_revision_state_is_preserved_instead_of_resetting_trust() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("accepted-revision.json");
    std::fs::write(&path, b"broken prior state").unwrap();
    assert!(accept(&path, 1).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken prior state");
}

#[test]
fn accepted_catalog_and_revision_reload_as_one_authenticated_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("accepted-catalog.json");
    accept(&path, 4).unwrap();
    let signer = key();
    let loaded = Catalog::load_accepted(&path, signer.public_key().as_ref()).unwrap();
    assert_eq!(loaded.revision(), 4);
    assert_eq!(loaded.entries()[0].title, "Wikipedia (English)");
    let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    assert!(Catalog::load_accepted(&path, other.public_key().as_ref()).is_err());
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(Catalog::load_accepted(&path, signer.public_key().as_ref()).is_err());
    assert!(accept(&path, 5).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn concurrent_acceptance_cannot_replace_a_higher_published_revision() {
    use std::sync::{Arc, Barrier};
    for _ in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("accepted-catalog.json");
        let barrier = Arc::new(Barrier::new(12));
        let writers = (1..=12)
            .map(|revision| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                let catalog = verify(&payload(revision), 0).unwrap();
                std::thread::spawn(move || {
                    barrier.wait();
                    match catalog.accept_revision(&path) {
                        Ok(()) | Err(PackError::Mismatch(_)) => {}
                        Err(error) => panic!("unexpected publication error: {error}"),
                    }
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        let signer = key();
        assert_eq!(
            Catalog::load_accepted(&path, signer.public_key().as_ref())
                .unwrap()
                .revision(),
            12
        );
    }
}
