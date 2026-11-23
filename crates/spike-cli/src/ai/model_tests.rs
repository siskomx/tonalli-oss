use super::*;

fn pack() -> Pack {
    Pack {
        id: "test".into(),
        kind: "gguf".into(),
        version: "fixture".into(),
        url: None,
        bytes: 17,
        sha256: "0".repeat(64),
        path: "packs/test.gguf".into(),
        deferred: false,
        extra: [("role".into(), "embedding".into())].into_iter().collect(),
    }
}

#[test]
fn verification_role_and_activated_filename_are_shared_without_guessing_model_names() {
    let mut value = pack();
    validate_pack(&value, ModelKind::Embedding).unwrap();
    assert!(matches!(
        validate_pack(&value, ModelKind::Generation),
        Err(PackError::Mismatch(_))
    ));
    value.extra.clear();
    assert!(validate_pack(&value, ModelKind::Embedding).is_err());
    value.extra.insert("role".into(), "generation".into());
    validate_pack(&value, ModelKind::Generation).unwrap();
    value.path.push_str(".part");
    assert!(validate_pack(&value, ModelKind::Generation).is_err());
    value.path = "packs/test.gguf".into();
    value.kind = "zim".into();
    assert!(validate_pack(&value, ModelKind::Generation).is_err());
}

#[test]
fn verification_checks_length_and_sha_before_core_load_and_preserves_source() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir(temporary.path().join("packs")).unwrap();
    let path = temporary.path().join("packs/test.gguf");
    let raw = b"not model weights";
    std::fs::write(&path, raw).unwrap();
    let mut value = pack();
    value.bytes = raw.len() as u64 + 1;
    assert!(matches!(
        VerifiedModel::load(temporary.path(), &value, ModelKind::Embedding, |_| Ok(())),
        Err(PackError::Mismatch(_))
    ));
    value.bytes = raw.len() as u64;
    assert!(matches!(
        VerifiedModel::load(temporary.path(), &value, ModelKind::Embedding, |_| Ok(())),
        Err(PackError::Checksum)
    ));
    assert_eq!(std::fs::read(path).unwrap(), raw);
}

#[test]
fn verified_receipt_is_published_before_native_load_and_can_refuse_it() {
    use sha2::{Digest, Sha256};
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir(temporary.path().join("packs")).unwrap();
    let raw = b"not a GGUF";
    std::fs::write(temporary.path().join("packs/test.gguf"), raw).unwrap();
    let mut value = pack();
    value.bytes = raw.len() as u64;
    value.sha256 = format!("{:x}", Sha256::digest(raw));
    let mut published = false;
    let result = VerifiedModel::load(temporary.path(), &value, ModelKind::Embedding, |pin| {
        assert_eq!(pin["sha256"], value.sha256);
        published = true;
        Err(PackError::NoSpace)
    });
    assert!(published);
    assert!(matches!(result, Err(PackError::NoSpace)));
}
