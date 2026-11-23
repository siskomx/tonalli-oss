//! Signed catalog metadata. Callers supply a pinned publisher key and app-owned state path.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use fs2::FileExt;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};

use crate::{pack::PackError, server::PackId};

const MAX_PAYLOAD: usize = 1024 * 1024;
const MAGIC: &[u8; 5] = b"TCAT\x01";
const HEADER: usize = MAGIC.len() + 4;
const SIGNATURE_BYTES: usize = 64;

pub const PUBLISHER_PUBLIC_KEY: &[u8; 32] =
    include_bytes!("../assets/catalog/publisher-public-key.bin");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKind {
    Zim,
    Map,
    Model,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub title: String,
    pub kind: CatalogKind,
    pub language: String,
    pub version: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    revision: u64,
    entries: Vec<CatalogEntry>,
}

pub struct Catalog {
    document: Document,
    payload: Vec<u8>,
    signature: Vec<u8>,
    public_key: Vec<u8>,
}

fn invalid() -> PackError {
    PackError::Mismatch("Invalid or outdated signed catalog".into())
}

fn corrupt() -> PackError {
    PackError::Corrupt("Invalid catalog snapshot".into())
}

impl Catalog {
    pub fn bundled() -> Result<Self, PackError> {
        Self::verify(
            include_bytes!("../assets/catalog/catalog-v1.json"),
            include_bytes!("../assets/catalog/catalog-v1.sig"),
            PUBLISHER_PUBLIC_KEY,
            0,
        )
    }

    /// Authenticate exact bytes before parsing. A zero floor permits the first valid revision.
    pub fn verify(
        payload: &[u8],
        signature: &[u8],
        public_key: &[u8],
        highest_revision: u64,
    ) -> Result<Self, PackError> {
        if payload.is_empty() || payload.len() > MAX_PAYLOAD {
            return Err(invalid());
        }
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(payload, signature)
            .map_err(|_| PackError::Checksum)?;
        let document: Document = serde_json::from_slice(payload).map_err(|_| corrupt())?;
        if document.schema_version != 1
            || document.revision == 0
            || document.revision < highest_revision
            || document.entries.len() > 256
        {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        for entry in &document.entries {
            validate_entry(entry)?;
            if !ids.insert(&entry.id) {
                return Err(invalid());
            }
        }
        Ok(Self {
            document,
            payload: payload.to_vec(),
            signature: signature.to_vec(),
            public_key: public_key.to_vec(),
        })
    }

    pub fn revision(&self) -> u64 {
        self.document.revision
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.document.entries
    }

    /// Read one authenticated snapshot; no separate revision counter can diverge from its catalog.
    pub fn load_accepted(path: &Path, public_key: &[u8]) -> Result<Self, PackError> {
        ordinary_or_missing(path)?;
        let mut bytes = Vec::new();
        File::open(path)?
            .take((HEADER + MAX_PAYLOAD + SIGNATURE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() < HEADER || &bytes[..MAGIC.len()] != MAGIC {
            return Err(corrupt());
        }
        let length = u32::from_be_bytes(
            bytes[MAGIC.len()..HEADER]
                .try_into()
                .map_err(|_| corrupt())?,
        ) as usize;
        if length > MAX_PAYLOAD || bytes.len() != HEADER + length + SIGNATURE_BYTES {
            return Err(corrupt());
        }
        Self::verify(
            &bytes[HEADER..HEADER + length],
            &bytes[HEADER + length..],
            public_key,
            0,
        )
    }

    /// Publish only beneath a native-owned private directory. The persistent sibling lock
    /// serializes the disk recheck and replacement across processes as well as threads.
    /// PublicationUncertain means new bytes may be visible; retry the same signed catalog.
    pub fn accept_revision(&self, path: &Path) -> Result<(), PackError> {
        self.accept_with_flush(path, durable_publication)
    }

    pub(crate) fn snapshot_for_key(&self, pin: &[u8]) -> Result<Vec<u8>, PackError> {
        Self::verify(&self.payload, &self.signature, pin, 0)?;
        let mut bytes = Vec::with_capacity(HEADER + self.payload.len() + SIGNATURE_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes.extend_from_slice(&self.signature);
        Ok(bytes)
    }

    fn accept_with_flush(
        &self,
        path: &Path,
        flush: impl FnOnce(&File, &Path) -> std::io::Result<()>,
    ) -> Result<(), PackError> {
        let parent = path.parent().ok_or_else(invalid)?;
        let mut lock_name = path.file_name().ok_or_else(invalid)?.to_os_string();
        lock_name.push(".lock");
        let lock_path = parent.join(lock_name);
        ordinary_or_missing(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        ordinary_or_missing(path)?;
        match fs::symlink_metadata(path) {
            Ok(_) => {
                let prior = Self::load_accepted(path, &self.public_key)?;
                if prior.revision() > self.revision()
                    || (prior.revision() == self.revision() && prior.payload != self.payload)
                {
                    return Err(invalid());
                }
                if prior.revision() == self.revision() {
                    let published = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path)
                        .map_err(uncertain)?;
                    return flush(&published, parent).map_err(uncertain);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(&self.snapshot_for_key(&self.public_key)?)?;
        staged.as_file().sync_all()?;
        let published = staged
            .persist(path)
            .map_err(|error| PackError::from(error.error))?;
        flush(&published, parent).map_err(uncertain)?;
        // The lock file is deliberately retained: deleting it could create two independent locks.
        Ok(())
    }
}

fn uncertain(error: std::io::Error) -> PackError {
    PackError::PublicationUncertain(error.to_string())
}

fn durable_publication(published: &File, parent: &Path) -> std::io::Result<()> {
    // On Windows sync_all uses FlushFileBuffers, including the renamed file's metadata.
    published.sync_all()?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

fn validate_entry(entry: &CatalogEntry) -> Result<(), PackError> {
    PackId::new(entry.id.clone()).map_err(|_| invalid())?;
    let text = |value: &str, maximum: usize| {
        !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
    };
    if !text(&entry.title, 512)
        || !text(&entry.version, 128)
        || entry.language.len() > 35
        || entry
            .language
            .split('-')
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_alphabetic()))
        || entry.bytes == 0
        || entry.sha256.len() != 64
        || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || entry.url.len() > 4096
        || entry
            .url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    let url = url::Url::parse(&entry.url).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str() != entry.url
    {
        return Err(invalid());
    }
    Ok(())
}

fn ordinary_or_missing(path: &Path) -> Result<(), PackError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let linked = metadata.file_type().is_symlink();
            if linked || !metadata.is_file() {
                return Err(invalid());
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn signed(revision: u64) -> Catalog {
        let signer = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "revision": revision, "entries": []
        }))
        .unwrap();
        Catalog::verify(
            &payload,
            signer.sign(&payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap()
    }

    #[test]
    fn durability_barrier_runs_after_publication_while_exclusive_lock_is_held() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("accepted.bin");
        let catalog = signed(1);
        let calls = AtomicUsize::new(0);
        catalog
            .accept_with_flush(&path, |published, parent| {
                assert_eq!(parent, root.path());
                assert_eq!(
                    Catalog::load_accepted(&path, &catalog.public_key)
                        .unwrap()
                        .revision(),
                    1
                );
                assert_eq!(published.metadata()?.len(), fs::metadata(&path)?.len());
                let competing = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(root.path().join("accepted.bin.lock"))?;
                assert!(competing.try_lock_exclusive().is_err());
                calls.fetch_add(1, Ordering::SeqCst);
                published.sync_all()
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn post_rename_flush_failure_is_uncertain_and_same_revision_retries_the_barrier() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("accepted.bin");
        signed(1).accept_revision(&path).unwrap();
        let catalog = signed(2);
        let failure = catalog.accept_with_flush(&path, |_, _| {
            assert_eq!(
                Catalog::load_accepted(&path, &catalog.public_key)
                    .unwrap()
                    .revision(),
                2
            );
            Err(std::io::Error::other("injected durability failure"))
        });
        assert!(matches!(failure, Err(PackError::PublicationUncertain(_))));
        // New bytes are visible; the failed barrier cannot promise the previous snapshot survived.
        let visible = fs::read(&path).unwrap();
        assert!(signed(1).accept_revision(&path).is_err());
        let calls = AtomicUsize::new(0);
        catalog
            .accept_with_flush(&path, |published, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                published.sync_all()
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read(&path).unwrap(), visible);
    }
}
