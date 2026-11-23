//! Attach verified local copies without replacing another registry owner's content.
use super::{registry_poisoned, PackId, PackRegistry, RegisteredTiles};
use crate::{
    catalog::CatalogKind,
    pack::PackError,
    sync::{PackManifest, SyncStore},
    tiles::TileSource,
    zim::Zim,
};
use std::sync::Arc;

type CopySources = (Option<Arc<Zim>>, Option<RegisteredTiles>);
fn sources(store: &SyncStore, manifest: &PackManifest) -> Result<CopySources, PackError> {
    manifest.validate()?;
    let source = store.open_source(manifest)?;
    Ok(match manifest.kind {
        CatalogKind::Zim => (Some(Arc::new(Zim::open(source)?)), None),
        CatalogKind::Map => {
            let metadata = TileSource::open(Arc::clone(&source))?.metadata().clone();
            (None, Some(RegisteredTiles { source, metadata }))
        }
        CatalogKind::Model => {
            return Err(PackError::Mismatch(
                "Models are not nearby-library packs".into(),
            ))
        }
    })
}

impl PackRegistry {
    pub(super) fn register_copy(
        &self,
        store: &SyncStore,
        manifest: &PackManifest,
    ) -> Result<(), PackError> {
        let id = PackId::new(manifest.local_id())?;
        let (zim, tiles) = sources(store, manifest)?;
        let mut copied = self.copied.lock().map_err(|_| registry_poisoned())?;
        if let Some(owner) = copied.get(&id) {
            if owner != store.root() {
                return Err(PackError::Mismatch(
                    "Nearby copy belongs to another store".into(),
                ));
            }
            return Ok(());
        }
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
        let mut maps = self.tiles.write().map_err(|_| registry_poisoned())?;
        if managed.contains_key(&id) || zims.contains_key(&id) || maps.contains_key(&id) {
            return Err(PackError::Mismatch(
                "Nearby copy ID belongs to another content owner".into(),
            ));
        }
        if let Some(zim) = zim {
            zims.insert(id.clone(), zim);
        }
        if let Some(tiles) = tiles {
            maps.insert(id.clone(), tiles);
        }
        copied.insert(id, store.root().to_path_buf());
        Ok(())
    }

    pub(super) fn remove_copy(
        &self,
        store: &SyncStore,
        id: &PackId,
        admitted: impl FnOnce() -> bool,
    ) -> Result<Option<crate::content::RemoveOutcome>, PackError> {
        let mut copied = self.copied.lock().map_err(|_| registry_poisoned())?;
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        if managed.contains_key(id) || copied.get(id).is_some_and(|root| root != store.root()) {
            return Err(PackError::Mismatch("Copy belongs to another owner".into()));
        }
        let descriptor = {
            let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
            let mut maps = self.tiles.write().map_err(|_| registry_poisoned())?;
            let mut indexes = self.indexes.write().map_err(|_| registry_poisoned())?;
            if !copied.contains_key(id)
                && (zims.contains_key(id) || maps.contains_key(id) || indexes.contains_key(id))
            {
                return Err(PackError::Mismatch("Content is not an owned copy".into()));
            }
            if zims.get(id).is_some_and(|row| Arc::strong_count(row) > 1)
                || maps
                    .get(id)
                    .is_some_and(|row| Arc::strong_count(&row.source) > 1)
                || indexes
                    .get(id)
                    .is_some_and(|row| Arc::strong_count(row) > 1)
            {
                return Ok(Some(crate::content::RemoveOutcome::InUse));
            }
            let descriptor = indexes.get(id).map(|index| index.generation_descriptor());
            drop((zims.remove(id), maps.remove(id), indexes.remove(id)));
            descriptor
        };
        // Mutation fences remain held while idle registry leases are dropped. An
        // independent process/reader still blocks the store's exclusive file lease.
        let result = store.remove_admitted(id.as_str(), admitted);
        match store.manifest(id.as_str())? {
            Some(row) => {
                let (zim, tiles) = sources(store, &row)
                    .map_err(|error| PackError::PublicationUncertain(error.to_string()))?;
                let index = zim
                    .as_ref()
                    .and_then(|zim| descriptor.and_then(|saved| saved.open(zim).ok()))
                    .map(Arc::new);
                if let Some(zim) = zim {
                    self.zims
                        .write()
                        .map_err(|_| registry_poisoned())?
                        .insert(id.clone(), zim);
                }
                if let Some(tiles) = tiles {
                    self.tiles
                        .write()
                        .map_err(|_| registry_poisoned())?
                        .insert(id.clone(), tiles);
                }
                if let Some(index) = index {
                    self.indexes
                        .write()
                        .map_err(|_| registry_poisoned())?
                        .insert(id.clone(), index);
                }
                copied.insert(id.clone(), store.root().to_path_buf());
            }
            None => {
                copied.remove(id);
            }
        }
        drop(managed);
        result
    }
}
