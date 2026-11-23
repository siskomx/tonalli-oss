use super::identity;

#[test]
fn restarting_or_changing_bind_keeps_the_private_key_and_spki_pin() {
    let root = tempfile::tempdir().unwrap();
    let first = identity::load(root.path(), "127.0.0.1".parse().unwrap()).unwrap();
    let key = std::fs::read(root.path().join("server-key.pk8")).unwrap();
    let again = identity::load(root.path(), "127.0.0.1".parse().unwrap()).unwrap();
    assert_eq!(first.spki_sha256, again.spki_sha256);
    let moved = identity::load(root.path(), "192.168.1.2".parse().unwrap()).unwrap();
    assert_eq!(first.spki_sha256, moved.spki_sha256);
    assert_eq!(
        std::fs::read(root.path().join("server-key.pk8")).unwrap(),
        key
    );
    assert!(!first.certificate_der.is_empty());
    assert!(!moved.certificate_der.is_empty());
}

#[test]
fn corrupt_stored_key_fails_and_never_rekeys() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("server-key.pk8");
    let original = b"invalid key must remain unchanged";
    std::fs::write(&path, original).unwrap();
    assert!(identity::load(root.path(), "127.0.0.1".parse().unwrap()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}
