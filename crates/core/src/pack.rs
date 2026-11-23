//! Pack verification, byte sources and errors shared by the core.

mod segments;
mod source;

pub use segments::{ConcatSource, WindowSource};
pub use source::{MmapSource, PackSource, PreadSource};

use std::fs::File;
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::path::Path;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("pack checksum does not match")]
    Checksum,
    #[error("pack is truncated")]
    Truncated,
    #[error("pack mismatch: {0}")]
    Mismatch(String),
    #[error("pack media was removed")]
    MediaRemoved,
    #[error("pack not found: {0}")]
    NotFound(String),
    #[error("corrupt pack: {0}")]
    Corrupt(String),
    #[error("insufficient storage space")]
    NoSpace,
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("publication durability is uncertain: {0}")]
    PublicationUncertain(String),
    #[error("download was limited by the server")]
    RateLimited,
}

impl From<io::Error> for PackError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Checks a hexadecimal SHA-256 digest using bounded memory and read-only I/O.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), PackError> {
    match verify_sha256_controlled(path, expected, |_| {
        Ok(ControlFlow::<std::convert::Infallible>::Continue(()))
    })? {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(never) => match never {},
    }
}

pub(crate) fn verify_sha256_controlled<B>(
    path: &Path,
    expected: &str,
    mut checkpoint: impl FnMut(u64) -> Result<ControlFlow<B>, PackError>,
) -> Result<ControlFlow<B>, PackError> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackError::Checksum);
    }
    if let ControlFlow::Break(stop) = checkpoint(0)? {
        return Ok(ControlFlow::Break(stop));
    }

    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut processed = 0;
    loop {
        let read = match file.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        processed += read as u64;
        if let ControlFlow::Break(stop) = checkpoint(processed)? {
            return Ok(ControlFlow::Break(stop));
        }
    }

    let actual = format!("{:x}", digest.finalize());
    if let ControlFlow::Break(stop) = checkpoint(processed)? {
        return Ok(ControlFlow::Break(stop));
    }
    if actual.eq_ignore_ascii_case(expected) {
        Ok(ControlFlow::Continue(()))
    } else {
        Err(PackError::Checksum)
    }
}

#[cfg(test)]
mod checksum_controls_tests {
    use super::*;
    use std::ops::ControlFlow;

    #[test]
    fn controlled_checksum_stops_after_one_chunk_without_comparing_a_partial_digest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pack.bin");
        std::fs::write(&path, vec![7; 3 * 64 * 1024]).unwrap();
        let mut observed = Vec::new();
        let outcome = verify_sha256_controlled(&path, &"0".repeat(64), |processed| {
            observed.push(processed);
            Ok(if processed > 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            })
        })
        .unwrap();
        assert_eq!(outcome, ControlFlow::Break(()));
        assert_eq!(observed, [0, 64 * 1024]);
    }

    #[test]
    fn controlled_checksum_propagates_checkpoint_failure_before_opening_missing_source() {
        let root = tempfile::tempdir().unwrap();
        let outcome =
            verify_sha256_controlled(&root.path().join("missing.bin"), &"0".repeat(64), |_| {
                Err::<ControlFlow<()>, _>(PackError::Io("checkpoint refused".into()))
            });
        assert_eq!(outcome, Err(PackError::Io("checkpoint refused".into())));
    }
}
