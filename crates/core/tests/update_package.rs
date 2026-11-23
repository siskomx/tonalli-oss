use std::io::Write;
use std::path::Path;

use tonalli_core::update::unpack;
use zip::{write::SimpleFileOptions, ZipWriter};

const MAX: u64 = 1 << 20;

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        writer.start_file(*name, SimpleFileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, found);
            } else {
                found.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk(dir, dir, &mut found);
    found.sort();
    found
}

#[test]
fn the_package_folder_is_unpacked_without_its_wrapper_directory() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    archive(
        &zip,
        &[
            ("Windows-x64/tonalli-desktop.exe", b"exe"),
            ("Windows-x64/web/index.html", b"<html>"),
            ("Windows-x64/licenses/NOTICE.txt", b"notice"),
        ],
    );
    let destination = temp.path().join("staged");
    unpack(&zip, &destination, "Windows-x64/", "tonalli-desktop.exe", MAX).unwrap();
    assert_eq!(names(&destination), ["licenses/NOTICE.txt", "tonalli-desktop.exe", "web/index.html"]);
    assert_eq!(std::fs::read(destination.join("tonalli-desktop.exe")).unwrap(), b"exe");
    assert!(!temp.path().join("staged.partial").exists());
}

#[test]
fn a_package_written_with_windows_backslash_separators_is_unpacked_the_same_way() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    archive(
        &zip,
        &[
            ("Windows-x64\\tonalli-desktop.exe", b"exe"),
            ("Windows-x64\\web\\assets\\index.html", b"<html>"),
        ],
    );
    let destination = temp.path().join("staged");
    unpack(&zip, &destination, "Windows-x64/", "tonalli-desktop.exe", MAX).unwrap();
    assert_eq!(names(&destination), ["tonalli-desktop.exe", "web/assets/index.html"]);
}

#[test]
fn backslash_tricks_still_cannot_leave_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    for (index, name) in [
        "Windows-x64\\..\\..\\evil.txt",
        "Windows-x64\\a\\..\\..\\evil.txt",
        "\\evil.txt",
        "..\\evil.txt",
        "Windows-x64\\C:\\evil.txt",
        "Windows-x64\\\\evil.txt",
    ]
    .iter()
    .enumerate()
    {
        let zip = temp.path().join(format!("update-{index}.zip"));
        archive(&zip, &[("Windows-x64/tonalli-desktop.exe", b"exe"), (name, b"bad")]);
        let destination = temp.path().join(format!("staged-{index}"));
        assert!(unpack(&zip, &destination, "Windows-x64/", "tonalli-desktop.exe", MAX).is_err(), "{name}");
        assert!(!destination.exists(), "{name}");
    }
    assert!(!temp.path().join("evil.txt").exists());
}

#[test]
fn paths_that_escape_the_folder_are_refused_and_nothing_is_written() {
    let temp = tempfile::tempdir().unwrap();
    for (index, name) in [
        "Windows-x64/../../evil.txt",
        "Windows-x64/..\\..\\evil.txt",
        "/evil.txt",
        "C:/evil.txt",
        "Windows-x64/C:/evil.txt",
        "Windows-x64//evil.txt",
        "Windows-x64/a/../../../evil.txt",
    ]
    .iter()
    .enumerate()
    {
        let zip = temp.path().join(format!("update-{index}.zip"));
        archive(&zip, &[("Windows-x64/tonalli-desktop.exe", b"exe"), (name, b"bad")]);
        let destination = temp.path().join(format!("staged-{index}"));
        assert!(unpack(&zip, &destination, "Windows-x64/", "tonalli-desktop.exe", MAX).is_err(), "{name}");
        assert!(!destination.exists(), "{name}");
        assert!(!temp.path().join(format!("staged-{index}.partial")).exists(), "{name}");
    }
    assert!(!temp.path().join("evil.txt").exists());
    assert!(!temp.path().parent().unwrap().join("evil.txt").exists());
}

#[test]
fn anything_outside_the_package_folder_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    archive(&zip, &[("Windows-x64/tonalli-desktop.exe", b"exe"), ("Other/extra.txt", b"x")]);
    assert!(unpack(&zip, &temp.path().join("staged"), "Windows-x64/", "tonalli-desktop.exe", MAX).is_err());
    assert!(!temp.path().join("staged").exists());
}

#[test]
fn a_package_without_the_application_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    archive(&zip, &[("Windows-x64/readme.txt", b"no exe here")]);
    assert!(unpack(&zip, &temp.path().join("staged"), "Windows-x64/", "tonalli-desktop.exe", MAX).is_err());
    assert!(!temp.path().join("staged").exists());
}

#[test]
fn an_oversized_package_is_refused_by_what_it_would_unpack_to() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    let big = vec![0u8; 4096];
    archive(&zip, &[("Windows-x64/tonalli-desktop.exe", &big), ("Windows-x64/data.bin", &big)]);
    assert!(unpack(&zip, &temp.path().join("staged"), "Windows-x64/", "tonalli-desktop.exe", 6000).is_err());
    assert!(!temp.path().join("staged").exists());
    unpack(&zip, &temp.path().join("fits"), "Windows-x64/", "tonalli-desktop.exe", 9000).unwrap();
}

#[test]
fn it_never_merges_into_an_existing_folder() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    archive(&zip, &[("Windows-x64/tonalli-desktop.exe", b"exe")]);
    let destination = temp.path().join("staged");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep.txt"), b"keep").unwrap();
    assert!(unpack(&zip, &destination, "Windows-x64/", "tonalli-desktop.exe", MAX).is_err());
    assert_eq!(names(&destination), ["keep.txt"]);
}

#[test]
fn symbolic_links_inside_a_package_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    let mut writer = ZipWriter::new(std::fs::File::create(&zip).unwrap());
    writer
        .start_file("Windows-x64/tonalli-desktop.exe", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"exe").unwrap();
    writer
        .add_symlink("Windows-x64/link", "C:/Windows", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();
    assert!(unpack(&zip, &temp.path().join("staged"), "Windows-x64/", "tonalli-desktop.exe", MAX).is_err());
    assert!(!temp.path().join("staged").exists());
}

#[test]
fn a_corrupt_archive_is_refused_without_leaving_files() {
    let temp = tempfile::tempdir().unwrap();
    let zip = temp.path().join("update.zip");
    std::fs::write(&zip, b"PK this is not really a zip archive").unwrap();
    assert!(unpack(&zip, &temp.path().join("staged"), "Windows-x64/", "tonalli-desktop.exe", MAX).is_err());
    assert!(!temp.path().join("staged").exists());
    assert!(!temp.path().join("staged.partial").exists());
}
