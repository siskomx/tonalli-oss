use sha2::{Digest, Sha256};
use std::fs;
use tonalli_core::{
    download::{fetch_pack_with_space, pack_path, DownloadSpec},
    pack::PackError,
};

fn spec(body: &[u8]) -> DownloadSpec {
    DownloadSpec {
        id: "test".into(),
        url: None,
        bytes: body.len() as u64,
        sha256: format!("{:x}", Sha256::digest(body)),
        path: "packs/test.zim".into(),
    }
}

#[test]
fn core_download_spec_reuses_verified_local_content_without_network_or_new_space() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("packs")).unwrap();
    let body = b"verified existing bytes";
    let target = root.path().join("packs/test.zim");
    fs::write(&target, body).unwrap();
    let path = fetch_pack_with_space(&spec(body), root.path(), |_| {
        panic!("existing verified content must not need more storage")
    })
    .unwrap();
    assert_eq!(path, target.canonicalize().unwrap());
    assert_eq!(fs::read(path).unwrap(), body);
}

#[test]
fn core_download_spec_without_a_url_preserves_unverified_existing_content() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("packs")).unwrap();
    let target = root.path().join("packs/test.zim");
    fs::write(&target, b"previous content").unwrap();
    assert!(matches!(
        fetch_pack_with_space(&spec(b"expected content"), root.path(), |_| Ok(u64::MAX)),
        Err(PackError::NotFound(_))
    ));
    assert_eq!(fs::read(target).unwrap(), b"previous content");
    assert!(!root.path().join("packs/test.zim.part").exists());
}

#[test]
fn shared_download_paths_remain_inside_the_owned_packs_directory() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        pack_path(root.path(), "packs/test.zim").unwrap(),
        root.path().canonicalize().unwrap().join("packs/test.zim")
    );
    for bad in ["notes.db", "packs/../notes.db", "packs", "/packs/test.zim"] {
        assert!(matches!(
            pack_path(root.path(), bad),
            Err(PackError::Mismatch(_))
        ));
    }
    let absolute = root.path().join("outside.zim");
    assert!(pack_path(root.path(), absolute.to_str().unwrap()).is_err());
}

#[cfg(windows)]
#[test]
fn windows_junction_is_rejected_even_when_its_target_stays_under_the_owned_root() {
    use std::{os::windows::fs::MetadataExt, process::Command};

    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("packs");
    let actual = root.path().join("actual");
    fs::create_dir(&packs).unwrap();
    fs::create_dir(&actual).unwrap();
    fs::write(actual.join("test.zim"), b"owned target").unwrap();
    let junction = packs.join("linked");
    let made = Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&actual)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "could not create owned junction fixture"
    );
    let metadata = fs::symlink_metadata(&junction).unwrap();
    assert_ne!(metadata.file_attributes() & 0x400, 0);
    // Containment alone would accept this alias, so link rejection is independently exercised.
    assert!(junction
        .canonicalize()
        .unwrap()
        .starts_with(root.path().canonicalize().unwrap()));
    assert!(matches!(
        pack_path(root.path(), "packs/linked/test.zim"),
        Err(PackError::Mismatch(_))
    ));
    fs::remove_dir(&junction).unwrap(); // Remove only the test-owned link, never its target.
    assert_eq!(fs::read(actual.join("test.zim")).unwrap(), b"owned target");
}
