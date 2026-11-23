use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use spike_cli::catalog::generate_key;
use spike_cli::update::{build_manifest, Release};
use tonalli_core::update::Manifest;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Fixture {
    temp: tempfile::TempDir,
    key: std::path::PathBuf,
    public: [u8; 32],
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("test-only-private.pk8");
    let public = generate_key(&key).unwrap();
    fs::write(temp.path().join("Tonalli-Windows-x64.zip"), b"windows package bytes").unwrap();
    fs::write(temp.path().join("Tonalli-Android-arm64.apk"), b"android package bytes!").unwrap();
    fs::create_dir(temp.path().join("out")).unwrap();
    Fixture { temp, key, public }
}

fn release(f: &Fixture) -> Release {
    Release {
        version: "0.1.0-preview.3".into(),
        published: "2026-11-16".into(),
        notes: "Smaller AI model for older phones.".into(),
        page: "https://github.com/example/tonalli/releases/tag/v0.1.0-preview.3".into(),
        base_url: "https://github.com/example/tonalli/releases/download/v0.1.0-preview.3/".into(),
        assets: vec![
            ("windows-x64".into(), f.temp.path().join("Tonalli-Windows-x64.zip")),
            ("android-arm64".into(), f.temp.path().join("Tonalli-Android-arm64.apk")),
        ],
    }
}

fn out(f: &Fixture) -> std::path::PathBuf {
    f.temp.path().join("out")
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_release_becomes_a_manifest_the_apps_accept() {
    let f = fixture();
    build_manifest(&release(&f), &f.key, &out(&f)).unwrap();
    assert_eq!(listing(&out(&f)), ["update-manifest.json", "update-manifest.sig"]);
    let payload = fs::read(out(&f).join("update-manifest.json")).unwrap();
    let signature = fs::read(out(&f).join("update-manifest.sig")).unwrap();
    assert_eq!(signature.len(), 64);
    let manifest = Manifest::verify(&payload, &signature, &f.public).unwrap();
    assert_eq!(manifest.version_text(), "0.1.0-preview.3");
    assert_eq!(manifest.notes, "Smaller AI model for older phones.");
    let windows = manifest.asset_for("windows-x64").unwrap();
    assert_eq!(windows.bytes, 21);
    assert_eq!(windows.sha256, hex(&Sha256::digest(b"windows package bytes")));
    assert_eq!(
        windows.url,
        "https://github.com/example/tonalli/releases/download/v0.1.0-preview.3/Tonalli-Windows-x64.zip"
    );
    assert_eq!(manifest.asset_for("android-arm64").unwrap().bytes, 22);
}

#[test]
fn existing_outputs_are_never_overwritten_and_nothing_partial_is_left() {
    let f = fixture();
    fs::write(out(&f).join("update-manifest.sig"), b"previous signature").unwrap();
    assert!(build_manifest(&release(&f), &f.key, &out(&f)).is_err());
    assert_eq!(listing(&out(&f)), ["update-manifest.sig"]);
    assert_eq!(fs::read(out(&f).join("update-manifest.sig")).unwrap(), b"previous signature");
}

#[test]
fn unsafe_or_incomplete_releases_publish_nothing() {
    let f = fixture();
    let key_before = fs::read(&f.key).unwrap();
    let mut cases: Vec<Release> = Vec::new();
    let mut http = release(&f);
    http.base_url = "http://example.org/download/".into();
    cases.push(http);
    let mut version = release(&f);
    version.version = "latest".into();
    cases.push(version);
    let mut platform = release(&f);
    platform.assets[0].0 = "plan9-mips".into();
    cases.push(platform);
    let mut missing = release(&f);
    missing.assets[0].1 = f.temp.path().join("does-not-exist.zip");
    cases.push(missing);
    let mut empty = release(&f);
    empty.assets.clear();
    cases.push(empty);
    let mut duplicate = release(&f);
    duplicate.assets[1].0 = "windows-x64".into();
    cases.push(duplicate);
    let mut odd_name = release(&f);
    fs::write(f.temp.path().join("bad name.zip"), b"x").unwrap();
    odd_name.assets[0].1 = f.temp.path().join("bad name.zip");
    cases.push(odd_name);
    for case in &cases {
        assert!(build_manifest(case, &f.key, &out(&f)).is_err());
        assert!(listing(&out(&f)).is_empty());
    }
    assert_eq!(fs::read(&f.key).unwrap(), key_before);
}

#[test]
fn the_command_line_reaches_the_builder() {
    let f = fixture();
    let notes = f.temp.path().join("notes.txt");
    fs::write(&notes, "From a file.").unwrap();
    let r = release(&f);
    let mut args: Vec<String> = vec![
        "update-manifest".into(),
        "--version".into(), r.version.clone(),
        "--published".into(), r.published.clone(),
        "--notes-file".into(), notes.to_str().unwrap().into(),
        "--page".into(), r.page.clone(),
        "--base-url".into(), r.base_url.clone(),
        "--key".into(), f.key.to_str().unwrap().into(),
        "--out-dir".into(), out(&f).to_str().unwrap().into(),
    ];
    for (platform, path) in &r.assets {
        args.push("--asset".into());
        args.push(format!("{platform}={}", path.to_str().unwrap()));
    }
    spike_cli::run(args).unwrap();
    let payload = fs::read(out(&f).join("update-manifest.json")).unwrap();
    let signature = fs::read(out(&f).join("update-manifest.sig")).unwrap();
    let manifest = Manifest::verify(&payload, &signature, &f.public).unwrap();
    assert_eq!(manifest.notes, "From a file.");
    assert!(spike_cli::run(["update-manifest".into(), "--version".into()]).is_err());
}

#[test]
fn update_keygen_makes_the_same_kind_of_key_as_the_catalog_one() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("update.pk8");
    spike_cli::run(["update-keygen".into(), "--out".into(), key.to_str().unwrap().into()]).unwrap();
    assert!(key.is_file());
    assert!(spike_cli::run(["update-keygen".into(), "--out".into(), key.to_str().unwrap().into()]).is_err());
}
