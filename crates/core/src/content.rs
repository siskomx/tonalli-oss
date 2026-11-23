//! Native-owned managed installations. User-selected references never enter this store.
mod jobs;
pub(crate) mod lease;
pub(crate) mod storage;

pub use jobs::{
    ContentJobs, JobAction, JobError, JobReply, JobRun, JobSnapshot, JobState, JobTicket,
};

use crate::{
    catalog::{Catalog, CatalogEntry, CatalogKind},
    download::{check_space, DownloadSpec},
    pack::{verify_sha256, PackError, PackSource, PreadSource},
    server::PackId,
    tiles::TileSource,
    zim::Zim,
};
use serde::Serialize;
use std::{
    fs::{self, File},
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};
use storage::{fingerprint, invalid, Ledger, Reference, PREFIX};

pub(crate) enum AdmittedInstall {
    Installed(InstalledContent),
    Stopped(crate::download::DownloadAction),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstalledContent {
    pub id: String,
    pub title: String,
    pub kind: CatalogKind,
    pub language: String,
    pub version: String,
    pub bytes: u64,
    pub sha256: String,
    pub authority_fingerprint: String,
    pub catalog_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed,
    NotInstalled,
    InUse,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPreflight {
    pub entry_id: String,
    pub available_bytes: u64,
    pub download_bytes: u64,
    pub staging_bytes: u64,
    pub required_bytes: u64,
    pub allowed: bool,
    pub reason: Option<String>,
}

pub struct ManagedStore {
    root: PathBuf,
    pin: Vec<u8>,
}

pub(crate) struct InstalledFile {
    #[cfg(feature = "ai")]
    pub path: PathBuf,
    pub source: Arc<dyn PackSource>,
    pub metadata: InstalledContent,
}

impl ManagedStore {
    pub(crate) fn publisher_key(&self) -> &[u8] {
        &self.pin
    }

    pub(crate) fn with_installed_reference<T>(
        &self,
        expected: &InstalledContent,
        operation: impl FnOnce() -> Result<T, PackError>,
    ) -> Result<T, PackError> {
        let _gate = self.gate()?;
        let ledger = self.ledger()?;
        let row = ledger
            .entries
            .iter()
            .find(|row| row.id == expected.id)
            .ok_or_else(|| PackError::NotFound("Installed content ID".into()))?;
        if self.resolve(row)?.1 != *expected {
            return Err(PackError::Mismatch(
                "Installed content changed during publication".into(),
            ));
        }
        operation()
    }
    /// Conservative local admission budget, not a promise about a remote validator.
    /// Existing partials are not deducted: a changed server validator may require a full restart.
    /// Installation renames on this same filesystem; actual worker/commit checks run again.
    pub fn preflight(&self, catalog: &Catalog, id: &str) -> Result<ContentPreflight, PackError> {
        catalog.snapshot_for_key(&self.pin)?;
        let entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| PackError::NotFound("Catalog content ID".into()))?;
        let _gate = self.gate()?;
        let installed = self
            .ledger()?
            .entries
            .iter()
            .any(|row| row.id == id && row.sha256.eq_ignore_ascii_case(&entry.sha256));
        let download_bytes = if installed && self.version_path(entry)?.try_exists()? {
            0
        } else {
            entry.bytes
        };
        let required_bytes = download_bytes
            .checked_add(1024 * 1024)
            .ok_or(PackError::NoSpace)?;
        let available_bytes = fs2::available_space(&self.root)?;
        let reason = if entry.kind == CatalogKind::Map {
            Some("incomplete_bundle")
        } else if available_bytes < required_bytes {
            Some("no_space")
        } else {
            None
        };
        Ok(ContentPreflight {
            entry_id: id.to_owned(),
            available_bytes,
            download_bytes,
            staging_bytes: 0,
            required_bytes,
            allowed: reason.is_none(),
            reason: reason.map(str::to_owned),
        })
    }

    /// The shell supplies an app-owned private root and pinned publisher key, never web input.
    pub fn open(root: &Path, pinned_key: &[u8]) -> Result<Self, PackError> {
        if pinned_key.len() != 32 {
            return Err(invalid());
        }
        fs::create_dir_all(root)?;
        let store = Self {
            root: root.canonicalize()?,
            pin: pinned_key.to_vec(),
        };
        let gate = store.path("installed.lock")?;
        let _lock = storage::lock(&gate)?;
        fs::create_dir_all(storage::parent(&store.path("leases/reserve.lock")?)?)?;
        Ok(store)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    fn path(&self, suffix: &str) -> Result<PathBuf, PackError> {
        storage::path(&self.root, &format!("{PREFIX}/{suffix}"))
    }
    fn gate(&self) -> Result<File, PackError> {
        storage::lock(&self.path("installed.lock")?)
    }
    fn ledger(&self) -> Result<Ledger, PackError> {
        Ledger::read(&self.path("installed.json")?)
    }
    fn id_key(id: &str) -> Result<String, PackError> {
        PackId::new(id.to_owned())?;
        Ok(fingerprint(id.as_bytes()))
    }
    fn lease_path(&self, id: &str) -> Result<PathBuf, PackError> {
        self.path(&format!("leases/{}.lock", Self::id_key(id)?))
    }
    fn version_path(&self, entry: &CatalogEntry) -> Result<PathBuf, PackError> {
        let extension = match entry.kind {
            CatalogKind::Zim => "zim",
            CatalogKind::Map => "pmtiles",
            CatalogKind::Model => "gguf",
        };
        self.path(&format!(
            "versions/{}/{}.{}",
            Self::id_key(&entry.id)?,
            entry.sha256.to_ascii_lowercase(),
            extension
        ))
    }

    pub(crate) fn accept_catalog(&self, catalog: &Catalog) -> Result<(), PackError> {
        catalog.snapshot_for_key(&self.pin)?;
        catalog.accept_revision(&self.path("content.catalog")?)
    }

    /// A native transfer worker uses this exact trusted spec; HTTP/FFI callers supply IDs only.
    pub fn cache_spec(&self, catalog: &Catalog, id: &str) -> Result<DownloadSpec, PackError> {
        catalog.snapshot_for_key(&self.pin)?;
        let entry = catalog
            .entries()
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| PackError::NotFound("Catalog content ID".into()))?;
        Ok(DownloadSpec {
            id: entry.id.clone(),
            url: Some(entry.url.clone()),
            bytes: entry.bytes,
            sha256: entry.sha256.to_ascii_lowercase(),
            path: format!(
                "packs/cache/{}/{}.blob",
                Self::id_key(id)?,
                entry.sha256.to_ascii_lowercase()
            ),
        })
    }

    pub fn list(&self) -> Result<Vec<InstalledContent>, PackError> {
        let _lock = self.gate()?;
        self.ledger()?
            .entries
            .iter()
            .map(|r| self.resolve(r).map(|(_, metadata)| metadata))
            .collect()
    }

    fn resolve(&self, row: &Reference) -> Result<(CatalogEntry, InstalledContent), PackError> {
        let path = self.path(&format!("authority/{}.tcat", row.authority_fingerprint))?;
        // TCAT permits at most 1 MiB of payload plus its fixed header and signature.
        if fs::metadata(&path)?.len() > 1024 * 1024 + 73 {
            return Err(invalid());
        }
        verify_sha256(&path, &row.authority_fingerprint)?;
        let catalog = Catalog::load_accepted(&path, &self.pin)?;
        let entry = catalog
            .entries()
            .iter()
            .find(|e| e.id == row.id && e.sha256.eq_ignore_ascii_case(&row.sha256))
            .ok_or_else(invalid)?;
        let metadata = InstalledContent {
            id: entry.id.clone(),
            title: entry.title.clone(),
            kind: entry.kind,
            language: entry.language.clone(),
            version: entry.version.clone(),
            bytes: entry.bytes,
            sha256: entry.sha256.to_ascii_lowercase(),
            authority_fingerprint: row.authority_fingerprint.clone(),
            catalog_revision: catalog.revision(),
        };
        Ok((entry.clone(), metadata))
    }

    pub fn open_source(&self, id: &str) -> Result<Arc<dyn PackSource>, PackError> {
        self.open_source_with_metadata(id).map(|(source, _)| source)
    }

    /// The retained reference and source handle describe one StoreMeta snapshot.
    pub(crate) fn open_source_with_metadata(
        &self,
        id: &str,
    ) -> Result<(Arc<dyn PackSource>, InstalledContent), PackError> {
        let file = self.open_installed_file(id)?;
        Ok((file.source, file.metadata))
    }

    pub(crate) fn open_installed_file(&self, id: &str) -> Result<InstalledFile, PackError> {
        let _lock = self.gate()?;
        let ledger = self.ledger()?;
        let row = ledger
            .entries
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| PackError::NotFound("Installed content ID".into()))?;
        let (entry, metadata) = self.resolve(row)?;
        // Global metadata lock precedes the content-ID lease; removal uses the same order.
        let path = self.version_path(&entry)?;
        let source: Arc<dyn PackSource> = Arc::new(lease::LeasedSource::open(
            &path,
            lease::file(&self.lease_path(id)?)?,
        )?);
        if source.len() != entry.bytes {
            return Err(PackError::Truncated);
        }
        Self::format(Arc::clone(&source), entry.kind, false)?;
        Ok(InstalledFile {
            #[cfg(feature = "ai")]
            path,
            source,
            metadata,
        })
    }

    /// No network fallback: only a matching completed cache or already verified version can activate.
    pub fn install_cached(
        &self,
        catalog: &Catalog,
        id: &str,
    ) -> Result<InstalledContent, PackError> {
        self.install_with(
            catalog,
            id,
            |p| fs2::available_space(p).map_err(Into::into),
            storage::publish,
        )
    }

    fn install_with(
        &self,
        catalog: &Catalog,
        id: &str,
        available: impl FnOnce(&Path) -> Result<u64, PackError>,
        publish: impl FnOnce(&Path, &[u8]) -> Result<(), PackError>,
    ) -> Result<InstalledContent, PackError> {
        match self.install_admitted_with(catalog, id, available, publish, || {
            Ok(ControlFlow::Continue(()))
        })? {
            AdmittedInstall::Installed(installed) => Ok(installed),
            AdmittedInstall::Stopped(_) => Err(invalid()),
        }
    }

    pub(crate) fn install_cached_admitted<G>(
        &self,
        catalog: &Catalog,
        id: &str,
        admit: impl FnOnce() -> Result<ControlFlow<crate::download::DownloadAction, G>, PackError>,
    ) -> Result<AdmittedInstall, PackError> {
        self.install_admitted_with(
            catalog,
            id,
            |p| fs2::available_space(p).map_err(Into::into),
            storage::publish,
            admit,
        )
    }

    pub(crate) fn confirm_published(
        &self,
        catalog: &Catalog,
        id: &str,
    ) -> Result<Option<InstalledContent>, PackError> {
        let authority = fingerprint(&catalog.snapshot_for_key(&self.pin)?);
        let entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(invalid)?;
        let _gate = self.gate()?;
        let ledger = self.ledger()?;
        let Some(row) = ledger.entries.iter().find(|row| {
            row.id == id
                && row.sha256.eq_ignore_ascii_case(&entry.sha256)
                && row.authority_fingerprint == authority
        }) else {
            return Ok(None);
        };
        Self::version_barrier(&self.version_path(entry)?)?;
        storage::durable(&self.path("installed.json")?)?;
        self.resolve(row).map(|(_, installed)| Some(installed))
    }

    /// One retained signed candidate only. StoreMeta remains held through the operation,
    /// which may acquire Jobs and publish its terminal after exact cleanup. An installed
    /// ID/SHA is protected regardless of the catalog authority that installed it.
    pub(crate) fn with_unpublished_candidate<T, E: From<PackError>>(
        &self,
        catalog: &Catalog,
        id: &str,
        operation: impl FnOnce(Option<&Path>) -> Result<T, E>,
    ) -> Result<T, E> {
        catalog.snapshot_for_key(&self.pin)?;
        let entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| PackError::NotFound("Catalog content ID".into()))?;
        let candidate = self.version_path(entry)?;
        let _gate = self.gate()?;
        let protected = self
            .ledger()?
            .entries
            .iter()
            .any(|row| row.id == id && row.sha256.eq_ignore_ascii_case(&entry.sha256));
        operation(if protected {
            None
        } else {
            Some(candidate.as_path())
        })
    }

    fn install_admitted_with<G>(
        &self,
        catalog: &Catalog,
        id: &str,
        available: impl FnOnce(&Path) -> Result<u64, PackError>,
        publish: impl FnOnce(&Path, &[u8]) -> Result<(), PackError>,
        admit: impl FnOnce() -> Result<ControlFlow<crate::download::DownloadAction, G>, PackError>,
    ) -> Result<AdmittedInstall, PackError> {
        let snapshot = catalog.snapshot_for_key(&self.pin)?;
        let spec = self.cache_spec(catalog, id)?;
        let entry = catalog
            .entries()
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(invalid)?;
        let authority = fingerprint(&snapshot);
        let _lock = self.gate()?;
        let mut ledger = self.ledger()?;
        let row = Reference {
            id: id.to_owned(),
            sha256: spec.sha256.clone(),
            authority_fingerprint: authority.clone(),
        };
        if let Some(prior) = ledger.entries.iter_mut().find(|r| r.id == id) {
            *prior = row.clone();
        } else {
            ledger.entries.push(row.clone());
        }
        let metadata = ledger.encode()?;
        check_space(
            available(&self.root)?,
            (snapshot.len() + metadata.len()) as u64,
        )?;
        let version = self.version_path(entry)?;
        let cache = storage::path(&self.root, &spec.path)?;
        if version.exists() {
            self.verify_blob(&version, entry)?;
        } else {
            if !cache.exists() {
                return Err(PackError::NotFound("Completed managed cache".into()));
            }
            self.verify_blob(&cache, entry)?;
        }
        // StoreMeta -> Jobs: the admitted guard fences cancellation through final publication.
        // Expensive validation above remains Verifying and does not consume the candidate.
        let _admission = match admit()? {
            ControlFlow::Continue(guard) => guard,
            ControlFlow::Break(action) => return Ok(AdmittedInstall::Stopped(action)),
        };
        if !version.exists() {
            fs::create_dir_all(storage::parent(&version)?)?;
            // Both paths are under this same native root, with junction/symlink traversal rejected.
            fs::rename(&cache, &version)?;
        }
        Self::version_barrier(&version)?;
        storage::immutable(
            &self.path(&format!("authority/{authority}.tcat"))?,
            &snapshot,
        )?;
        publish(&self.path("installed.json")?, &metadata)?;
        self.resolve(&row)
            .map(|(_, record)| AdmittedInstall::Installed(record))
            .map_err(|error| PackError::PublicationUncertain(error.to_string()))
    }

    fn verify_blob(&self, path: &Path, entry: &CatalogEntry) -> Result<(), PackError> {
        if fs::metadata(path)?.len() != entry.bytes {
            return Err(PackError::Truncated);
        }
        verify_sha256(path, &entry.sha256)?;
        Self::format(Arc::new(PreadSource::open(path)?), entry.kind, true)
    }

    fn version_barrier(path: &Path) -> Result<(), PackError> {
        storage::durable(path)
    }

    fn format(
        source: Arc<dyn PackSource>,
        kind: CatalogKind,
        checksum: bool,
    ) -> Result<(), PackError> {
        match kind {
            CatalogKind::Zim => {
                let zim = Zim::open(source)?;
                if checksum {
                    zim.verify_md5()?;
                }
            }
            CatalogKind::Map => {
                TileSource::open(source)?;
            }
            CatalogKind::Model => crate::gguf::preflight(source.as_ref())?,
        }
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<RemoveOutcome, PackError> {
        self.remove_admitted(id, || Ok(ControlFlow::<(), ()>::Continue(())))
    }

    pub(crate) fn remove_admitted<G>(
        &self,
        id: &str,
        admit: impl FnOnce() -> Result<ControlFlow<(), G>, PackError>,
    ) -> Result<RemoveOutcome, PackError> {
        self.remove_admitted_with(id, admit, storage::publish)
    }

    fn remove_admitted_with<G>(
        &self,
        id: &str,
        admit: impl FnOnce() -> Result<ControlFlow<(), G>, PackError>,
        publish: impl FnOnce(&Path, &[u8]) -> Result<(), PackError>,
    ) -> Result<RemoveOutcome, PackError> {
        Self::id_key(id)?;
        let _lock = self.gate()?;
        // StoreMeta -> Jobs; the guard covers metadata withdrawal and owned deletion.
        let _admission = match admit()? {
            ControlFlow::Continue(guard) => guard,
            ControlFlow::Break(()) => return Ok(RemoveOutcome::InUse),
        };
        let mut ledger = self.ledger()?;
        let installed = ledger.entries.iter().any(|r| r.id == id);
        let lease_file = lease::file(&self.lease_path(id)?)?;
        if !lease::exclusive(&lease_file)? {
            return Ok(RemoveOutcome::InUse);
        }
        let directory =
            storage::parent(&self.path(&format!("versions/{}/reserve.zim", Self::id_key(id)?))?)?
                .to_path_buf();
        let mut files = Vec::new();
        match fs::read_dir(&directory) {
            Ok(items) => {
                for item in items {
                    if files.len() >= 256 {
                        return Err(invalid());
                    }
                    let item = item?;
                    let name = item.file_name().into_string().map_err(|_| invalid())?;
                    let stem = name
                        .strip_suffix(".zim")
                        .or_else(|| name.strip_suffix(".pmtiles"))
                        .or_else(|| name.strip_suffix(".gguf"))
                        .ok_or_else(invalid)?;
                    if !storage::hex(stem) {
                        return Err(invalid());
                    }
                    files.push(self.path(&format!("versions/{}/{name}", Self::id_key(id)?))?);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if !installed && files.is_empty() {
            return Ok(RemoveOutcome::NotInstalled);
        }
        ledger.entries.retain(|r| r.id != id);
        if installed {
            publish(&self.path("installed.json")?, &ledger.encode()?)?;
        }
        // Withdraw first: a failed deletion may leave owned blobs, never a row pointing at deleted bytes.
        let cleanup = (|| -> Result<(), PackError> {
            for file in files {
                fs::remove_file(file)?;
            }
            if directory.exists() {
                fs::remove_dir(&directory)?;
            }
            #[cfg(unix)]
            File::open(storage::parent(&directory)?)?.sync_all()?;
            Ok(())
        })();
        cleanup.map_err(|e| PackError::PublicationUncertain(e.to_string()))?;
        // Authority and lock files are intentionally retained; never delete another ID's authority.
        Ok(RemoveOutcome::Removed)
    }
}

#[cfg(test)]
mod tests;
