use std::io::Write;

use tempfile::NamedTempFile;
use tonalli_core::pack::{verify_sha256, PackError};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn verify_sha256_accepts_good_hash() -> Result<(), Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"abc")?;
    verify_sha256(file.path(), ABC_SHA256)?;
    Ok(())
}

#[test]
fn verify_sha256_rejects_bad_hash() -> Result<(), Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"abc")?;
    assert!(matches!(
        verify_sha256(file.path(), &"0".repeat(64)),
        Err(PackError::Checksum)
    ));
    Ok(())
}

#[test]
fn verify_sha256_handles_empty_files() -> Result<(), Box<dyn std::error::Error>> {
    let file = NamedTempFile::new()?;
    verify_sha256(
        file.path(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    )?;
    Ok(())
}

#[test]
fn verify_sha256_reads_past_one_buffer() -> Result<(), Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::new()?;
    for _ in 0..1000 {
        file.write_all(&[b'a'; 1000])?;
    }
    verify_sha256(
        file.path(),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
    )?;
    Ok(())
}

#[test]
fn verify_sha256_accepts_uppercase_hex_and_rejects_malformed_hashes(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"abc")?;
    verify_sha256(file.path(), &ABC_SHA256.to_uppercase())?;
    for expected in ["".to_owned(), "f".repeat(63), "g".repeat(64)] {
        assert!(matches!(
            verify_sha256(file.path(), &expected),
            Err(PackError::Checksum)
        ));
    }
    Ok(())
}

#[test]
fn verify_sha256_reports_missing_file_as_io() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    assert!(matches!(
        verify_sha256(&dir.path().join("absent.zim"), ABC_SHA256),
        Err(PackError::Io(_))
    ));
    Ok(())
}
