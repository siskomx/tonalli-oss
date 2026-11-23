use spike_cli::{add::add_path, manifest::Manifest};

#[test]
fn manifest_extra_metadata_roundtrips_toml_values_without_semantic_loss() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("packs.toml");
    let text = "manifest_version=1\nobserved=2026-11-12T10:20:30-08:00\ntags=['en','es']\n[extra]\nretained=true\n[[packs]]\nid='metadata'\nkind='zim'\nversion='test'\nbytes=1\nsha256='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\npath='packs/metadata.zim'\nobserved=2026-11-12T18:20:30Z\n[ packs.details ]\nlabels=['English','Español']\n";
    std::fs::write(&path, text).unwrap();
    let before: toml::Value = toml::from_str(text).unwrap();
    let manifest = Manifest::load(&path).unwrap();
    manifest.save(&path).unwrap();
    let after: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before, after);
    let reopened = Manifest::load(&path).unwrap();
    assert_eq!(manifest.extra, reopened.extra);
    assert_eq!(manifest.packs[0].extra, reopened.packs[0].extra);
}
use std::fs;

#[test]
fn adds_supported_files_without_mutating_source_and_preserves_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::write(
        input.path().join("sample.zim"),
        b"local user supplied bytes",
    )
    .unwrap();
    fs::write(input.path().join("skip.txt"), b"not a pack").unwrap();
    let path = dir.path().join("packs.toml");
    fs::write(
        &path,
        "manifest_version=1\npacks=[]\n[pending]\niso='later'\n",
    )
    .unwrap();
    let added = add_path(&path, input.path()).unwrap();
    assert_eq!(added.len(), 1);
    let manifest = Manifest::load(&path).unwrap();
    assert_eq!(manifest.packs.len(), 1);
    assert!(fs::read_to_string(&path).unwrap().contains("later"));
    assert_eq!(
        fs::read(input.path().join("sample.zim")).unwrap(),
        b"local user supplied bytes"
    );
    assert_eq!(
        fs::read(dir.path().join(&manifest.packs[0].path)).unwrap(),
        b"local user supplied bytes"
    );
    assert_eq!(add_path(&path, input.path()).unwrap().len(), 0);
}

#[test]
fn manifest_rejects_paths_outside_pack_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("packs.toml");
    fs::write(&path, "manifest_version=1\n[[packs]]\nid='bad'\nkind='zim'\nversion='test'\nbytes=0\nsha256='e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'\npath='../outside.zim'\n").unwrap();
    assert!(Manifest::load(&path).is_err());
}

#[test]
fn fetch_requires_explicit_selection() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spike-cli"))
        .arg("fetch")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--id"));
}

#[test]
fn same_bytes_with_different_pack_extensions_keep_unique_ids() {
    let dir = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("sample.zim"), b"same bytes").unwrap();
    fs::write(input.path().join("sample.pmtiles"), b"same bytes").unwrap();
    let path = dir.path().join("packs.toml");
    fs::write(&path, "manifest_version=1\npacks=[]\n").unwrap();
    assert_eq!(add_path(&path, input.path()).unwrap().len(), 2);
    assert_eq!(Manifest::load(&path).unwrap().packs.len(), 2);
}

#[test]
fn reimport_repairs_missing_or_corrupt_local_pack() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("original.zim");
    fs::write(&source, b"verified copy").unwrap();
    let path = dir.path().join("packs.toml");
    fs::write(&path, "manifest_version=1\npacks=[]\n").unwrap();
    add_path(&path, &source).unwrap();
    let target = dir
        .path()
        .join(&Manifest::load(&path).unwrap().packs[0].path);
    fs::remove_file(&target).unwrap();
    add_path(&path, &source).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"verified copy");
    fs::write(&target, b"corrupt").unwrap();
    add_path(&path, &source).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"verified copy");
    assert_eq!(Manifest::load(&path).unwrap().packs.len(), 1);
}
