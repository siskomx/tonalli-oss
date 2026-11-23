//! Durable reference withdrawal precedes cleanup of exactly one app-owned copy.
use super::{hash_for_id, invalid, storage, CopyRecord, PackManifest, RemovedCopy, SyncStore};
use crate::{
    content::{lease, RemoveOutcome},
    pack::PackError,
};
use std::{fs, path::PathBuf};

impl SyncStore {
    fn id_metadata(&self, id: &str) -> Result<PathBuf, PackError> {
        let hash = hash_for_id(id)?;
        storage::path(&self.root, &format!("packs/nearby/installed/{hash}.json"))
    }
    pub(super) fn lease_path(&self, manifest: &PackManifest) -> Result<PathBuf, PackError> {
        manifest.validate()?;
        storage::path(
            &self.root,
            &format!("packs/nearby/leases/{}.lock", manifest.sha256),
        )
    }
    pub fn manifest(&self, id: &str) -> Result<Option<PackManifest>, PackError> {
        let metadata = self.id_metadata(id)?;
        let _gate = storage::lock(&storage::path(&self.root, "packs/nearby/installed.lock")?)?;
        if !metadata.exists() {
            return Ok(None);
        }
        let record = Self::read_record(&metadata)?;
        if record.manifest().local_id() != id {
            return Err(invalid());
        }
        Ok(match record {
            CopyRecord::Live(row) => Some(row),
            CopyRecord::Removed(_) => None,
        })
    }
    pub fn remove(&self, id: &str) -> Result<RemoveOutcome, PackError> {
        self.remove_admitted(id, || true)?.ok_or_else(invalid)
    }
    pub(crate) fn remove_admitted(
        &self,
        id: &str,
        admitted: impl FnOnce() -> bool,
    ) -> Result<Option<RemoveOutcome>, PackError> {
        let metadata = self.id_metadata(id)?;
        let _copy = self.copy_lease()?;
        let _gate = storage::lock(&storage::path(&self.root, "packs/nearby/installed.lock")?)?;
        let hash = hash_for_id(id)?;
        let reader_lease = lease::file(&storage::path(
            &self.root,
            &format!("packs/nearby/leases/{hash}.lock"),
        )?)?;
        if !lease::exclusive(&reader_lease)? {
            return Ok(Some(RemoveOutcome::InUse));
        }
        let mut files = Vec::new();
        for suffix in [".blob", ".blob.part", ".blob.part.meta"] {
            files.push(storage::path(
                &self.root,
                &format!("packs/nearby/versions/{hash}{suffix}"),
            )?);
        }
        let existed = metadata.exists();
        if !admitted() {
            return Ok(None);
        }
        if existed {
            let record = Self::read_record(&metadata)?;
            if record.manifest().local_id() != id {
                return Err(invalid());
            }
            if let CopyRecord::Live(row) = record {
                crate::download::write_atomic(
                    &metadata,
                    &serde_json::to_vec(&RemovedCopy { removed: row }).map_err(|_| invalid())?,
                )?;
            }
            // A failed durability barrier leaves the bytes intact and never reports success.
            storage::durable(&metadata)?;
        } else if !files.iter().any(|path| path.exists()) {
            return Ok(Some(RemoveOutcome::NotInstalled));
        }
        let cleanup = (|| -> Result<(), PackError> {
            for path in &files {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            #[cfg(unix)]
            fs::File::open(files[0].parent().ok_or_else(invalid)?)?.sync_all()?;
            if existed {
                fs::remove_file(&metadata)?;
            }
            #[cfg(unix)]
            fs::File::open(metadata.parent().ok_or_else(invalid)?)?.sync_all()?;
            Ok(())
        })();
        cleanup.map_err(|e| PackError::PublicationUncertain(e.to_string()))?;
        Ok(Some(RemoveOutcome::Removed))
    }
}
