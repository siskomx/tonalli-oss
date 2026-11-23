use crate::{download::pack_path, pack::PackError, server::PackId};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const PREFIX: &str = "packs/managed";
const MAX_LEDGER: u64 = 512 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub id: String,
    pub sha256: String,
    pub authority_fingerprint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    version: u32,
    pub entries: Vec<Reference>,
}

pub(super) fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
}
pub(super) fn invalid() -> PackError {
    PackError::Corrupt("Invalid managed content metadata".into())
}

pub(crate) fn path(root: &Path, relative: &str) -> Result<PathBuf, PackError> {
    let path = pack_path(root, relative)?;
    // pack_path supplies the root/path policy; also reject in-root Windows junction aliases.
    let mut current = root.to_path_buf();
    let relative = path.strip_prefix(root).map_err(|_| invalid())?;
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = metadata.file_type().is_symlink();
                if linked {
                    return Err(invalid());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

pub(super) fn parent(path: &Path) -> Result<&Path, PackError> {
    path.parent().ok_or_else(invalid)
}

pub(crate) fn lock(path: &Path) -> Result<File, PackError> {
    fs::create_dir_all(parent(path)?)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    FileExt::lock_exclusive(&file)?;
    Ok(file)
}

impl Ledger {
    pub fn read(path: &Path) -> Result<Self, PackError> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    version: 1,
                    entries: Vec::new(),
                })
            }
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_LEDGER + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_LEDGER {
            return Err(invalid());
        }
        let ledger: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if ledger.version != 1 || ledger.entries.len() > 256 {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        for row in &ledger.entries {
            PackId::new(row.id.clone()).map_err(|_| invalid())?;
            if !ids.insert(&row.id) || !hex(&row.sha256) || !hex(&row.authority_fingerprint) {
                return Err(invalid());
            }
        }
        Ok(ledger)
    }

    pub fn encode(&self) -> Result<Vec<u8>, PackError> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if self.entries.len() > 256 || bytes.len() as u64 > MAX_LEDGER {
            return Err(invalid());
        }
        Ok(bytes)
    }
}

pub(crate) fn flush(file: &File, parent: &Path) -> std::io::Result<()> {
    file.sync_all()?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

pub(crate) fn publish(path: &Path, bytes: &[u8]) -> Result<(), PackError> {
    publish_with_flush(path, bytes, flush)
}

pub(crate) fn durable(path: &Path) -> Result<(), PackError> {
    durable_with(path, flush)
}

fn durable_with(
    path: &Path,
    barrier: impl FnOnce(&File, &Path) -> std::io::Result<()>,
) -> Result<(), PackError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| PackError::PublicationUncertain(e.to_string()))?;
    barrier(&file, parent(path)?).map_err(|e| PackError::PublicationUncertain(e.to_string()))
}

pub(super) fn publish_with_flush(
    path: &Path,
    bytes: &[u8],
    final_flush: impl FnOnce(&File, &Path) -> std::io::Result<()>,
) -> Result<(), PackError> {
    crate::download::write_atomic(path, bytes)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| PackError::PublicationUncertain(e.to_string()))?;
    final_flush(&file, parent(path)?).map_err(|e| PackError::PublicationUncertain(e.to_string()))
}

pub(super) fn immutable(path: &Path, bytes: &[u8]) -> Result<(), PackError> {
    immutable_with(path, bytes, durable)
}

fn immutable_with(
    path: &Path,
    bytes: &[u8],
    retry_barrier: impl FnOnce(&Path) -> Result<(), PackError>,
) -> Result<(), PackError> {
    if path.exists() {
        if fs::metadata(path)?.len() != bytes.len() as u64 || fs::read(path)? != bytes {
            return Err(invalid());
        }
        return retry_barrier(path);
    }
    fs::create_dir_all(parent(path)?)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent(path)?)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    let published = staged
        .persist_noclobber(path)
        .map_err(|e| PackError::from(e.error))?;
    flush(&published, parent(path)?).map_err(|e| PackError::PublicationUncertain(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_authority_retry_cannot_skip_durability_failure() {
        let root = tempfile::tempdir().expect("temp");
        let path = root.path().join("authority.tcat");
        fs::write(&path, b"signed snapshot").expect("fixture");
        let result = immutable_with(&path, b"signed snapshot", |p| {
            durable_with(p, |_, _| Err(std::io::Error::other("retry barrier failed")))
        });
        assert!(matches!(result, Err(PackError::PublicationUncertain(_))));
        immutable(&path, b"signed snapshot").expect("retry repeats real barrier");
    }

    #[test]
    fn missing_postrename_file_is_uncertain_not_ordinary_io() {
        let root = tempfile::tempdir().expect("temp");
        assert!(matches!(
            durable(&root.path().join("missing")),
            Err(PackError::PublicationUncertain(_))
        ));
    }
}
