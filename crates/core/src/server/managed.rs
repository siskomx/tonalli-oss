//! Installed-store ownership and publication; no paths or transport options enter HTTP.
use super::*;
#[cfg(test)]
#[path = "managed_capture_tests.rs"]
mod capture_tests;
use crate::{
    catalog::CatalogKind,
    content::{ContentJobs, InstalledContent, JobError, ManagedStore, RemoveOutcome},
};

type PriorOwners = (Option<Arc<Zim>>, Option<Arc<crate::index::Index>>);

pub(super) struct Origin {
    root: PathBuf,
    publisher: Vec<u8>,
    sha256: String,
}
impl Origin {
    fn belongs(&self, store: &ManagedStore) -> bool {
        self.root == store.root() && self.publisher == store.publisher_key()
    }
}

impl PackRegistry {
    /// Resolve native registry/store authority before scheduling; no web checksum or path is trusted.
    pub fn preparation_source(
        &self,
        id: &PackId,
    ) -> Result<crate::index::jobs::IndexSource, crate::index::jobs::IndexJobError> {
        let (origin, zim) = {
            let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
            let origin = managed
                .get(id)
                .map(|origin| (origin.root.clone(), origin.publisher.clone()));
            let zims = self.zims.read().map_err(|_| registry_poisoned())?;
            let zim = zims
                .get(id)
                .cloned()
                .ok_or_else(|| PackError::NotFound("Selected book".into()))?;
            (origin, zim)
        };
        if let Some((root, pin)) = origin {
            let store = ManagedStore::open(&root, &pin)?;
            self.sync_managed(&store)?;
            crate::index::jobs::IndexSource::managed(&store, id.as_str())
        } else {
            crate::index::jobs::IndexSource::registered(id.clone(), zim)
        }
    }
    /// Reopen committed installed content, including commits made by another process.
    /// Preparation is outside map locks; only this store's entries may replace prior owners.
    pub fn sync_managed(&self, store: &ManagedStore) -> Result<(), PackError> {
        for installed in store.list()? {
            if installed.kind != CatalogKind::Zim {
                continue;
            } // Partial map bundles remain unavailable to product downloads.
            let id = PackId::new(installed.id.clone())?;
            {
                let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
                if let Some(origin) = managed.get(&id) {
                    if !origin.belongs(store) {
                        return Err(owner_conflict());
                    }
                    if origin.sha256 == installed.sha256
                        && self
                            .zims
                            .read()
                            .map_err(|_| registry_poisoned())?
                            .contains_key(&id)
                    {
                        continue;
                    }
                }
            }
            let (source, captured) = store.open_source_with_metadata(&installed.id)?;
            let zim = Arc::new(Zim::open(source)?);
            let mut managed = self.managed.lock().map_err(|_| registry_poisoned())?;
            let previous = self.publish_managed(store, &captured, &id, zim, None, &mut managed)?;
            drop(managed);
            drop(previous);
        }
        Ok(())
    }

    // The origin/mutation fence precedes StoreMeta; map locks never cross source validation.
    fn publish_managed(
        &self,
        store: &ManagedStore,
        installed: &InstalledContent,
        id: &PackId,
        zim: Arc<Zim>,
        prepared_index: Option<Arc<crate::index::Index>>,
        managed: &mut HashMap<PackId, Origin>,
    ) -> Result<PriorOwners, PackError> {
        store.with_installed_reference(installed, || {
            let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
            let tiles = self.tiles.read().map_err(|_| registry_poisoned())?;
            let mut indexes = self.indexes.write().map_err(|_| registry_poisoned())?;
            if let Some(origin) = managed.get(id) {
                if !origin.belongs(store) {
                    return Err(owner_conflict());
                }
            } else if zims.contains_key(id) || tiles.contains_key(id) {
                return Err(owner_conflict());
            }
            let old = zims.insert(id.clone(), zim);
            let index = match prepared_index {
                Some(index) => indexes.insert(id.clone(), index),
                None => indexes.remove(id),
            };
            managed.insert(
                id.clone(),
                Origin {
                    root: store.root().to_path_buf(),
                    publisher: store.publisher_key().to_vec(),
                    sha256: installed.sha256.clone(),
                },
            );
            Ok((old, index))
        })
    }

    /// Withdraw only managed ownership. Idle registry leases are dropped before the
    /// job-aware StoreMeta -> Jobs remove; an independent reader still yields InUse.
    pub fn remove_managed(
        &self,
        jobs: &ContentJobs,
        id: &PackId,
    ) -> Result<RemoveOutcome, JobError> {
        let store = jobs.managed_store();
        let mut managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        let prior_sha = managed.get(id).map(|origin| origin.sha256.clone());
        let (removed_zim, removed_index, descriptor) = {
            let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
            let tiles = self.tiles.read().map_err(|_| registry_poisoned())?;
            let mut indexes = self.indexes.write().map_err(|_| registry_poisoned())?;
            if let Some(origin) = managed.get(id) {
                if !origin.belongs(store) {
                    return Err(owner_conflict().into());
                }
            } else if zims.contains_key(id) || tiles.contains_key(id) {
                return Err(owner_conflict().into());
            }
            if indexes
                .get(id)
                .is_some_and(|index| Arc::strong_count(index) > 1)
                || zims.get(id).is_some_and(|zim| Arc::strong_count(zim) > 1)
            {
                return Ok(RemoveOutcome::InUse);
            }
            let descriptor = indexes.get(id).map(|index| index.generation_descriptor());
            (zims.remove(id), indexes.remove(id), descriptor)
        };
        // A descriptor keeps the exact generation without retaining its pack lease.
        drop((removed_zim, removed_index));
        let result = jobs.remove_installed(id.as_str());
        let installed = store
            .list()?
            .into_iter()
            .find(|entry| entry.id == id.as_str());
        if let Some(installed) = installed {
            if installed.kind != CatalogKind::Zim {
                return result;
            }
            let restored = (|| {
                let (source, captured) = store.open_source_with_metadata(id.as_str())?;
                let zim = Arc::new(Zim::open(source)?);
                let index = if prior_sha.as_deref() == Some(captured.sha256.as_str()) {
                    // Optional search data must not hide a valid restored book.
                    descriptor
                        .as_ref()
                        .and_then(|saved| saved.open(&zim).ok())
                        .map(Arc::new)
                } else {
                    None // A concurrent committed update must not inherit the old index.
                };
                self.publish_managed(store, &captured, id, zim, index, &mut managed)
            })();
            if restored.is_err() {
                return Err(PackError::PublicationUncertain(
                    "Installed content could not be restored to the Library".into(),
                )
                .into());
            }
        } else {
            managed.remove(id);
        }
        result
    }
}

fn owner_conflict() -> PackError {
    PackError::Mismatch("Pack ID belongs to another content owner".into())
}
