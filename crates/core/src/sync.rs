//! Verified nearby-library copies in app-owned storage, separate from publisher catalogs.
use crate::{
    catalog::CatalogKind,
    content::storage,
    download::DownloadSpec,
    pack::{PackError, PackSource},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
mod removal;
mod search;

#[derive(Deserialize)]
#[serde(untagged)]
enum CopyRecord {
    Live(PackManifest),
    Removed(RemovedCopy),
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RemovedCopy {
    removed: PackManifest,
}
impl CopyRecord {
    fn manifest(&self) -> &PackManifest {
        match self {
            Self::Live(row) => row,
            Self::Removed(row) => &row.removed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub kind: crate::catalog::CatalogKind,
    pub bytes: u64,
    pub sha256: String,
}
pub struct SyncStore {
    root: PathBuf,
}
pub(crate) struct CopyEntry {
    pub manifest: PackManifest,
    pub removal_pending: bool,
}
pub(crate) struct CopyLease<'a> {
    store: &'a SyncStore,
    _file: fs::File,
}
impl CopyLease<'_> {
    pub(crate) fn discard(&self, manifest: &PackManifest) -> Result<(), PackError> {
        let store = self.store;
        let metadata = store.metadata(manifest)?;
        let _gate = storage::lock(&storage::path(&store.root, "packs/nearby/installed.lock")?)?;
        let spec = store.cache_spec(manifest)?;
        let mut paths = vec![
            format!("{}.part.meta", spec.path),
            format!("{}.part", spec.path),
        ];
        if !metadata.exists() {
            paths.push(spec.path);
        }
        for relative in paths {
            let path = storage::path(&store.root, &relative)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            }
            #[cfg(unix)]
            fs::File::open(path.parent().ok_or_else(invalid)?)?.sync_all()?;
        }
        self.clear_search(manifest)
    }
}
fn invalid() -> PackError {
    PackError::Corrupt("Invalid nearby-library copy".into())
}
pub(crate) fn hash_for_id(id: &str) -> Result<&str, PackError> {
    id.strip_prefix("copy-")
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(invalid)
}
impl PackManifest {
    pub fn local_id(&self) -> String {
        format!("copy-{}", self.sha256)
    }
    pub fn validate(&self) -> Result<(), PackError> {
        crate::server::PackId::new(&self.id)?;
        if self.version != 1
            || self.kind == CatalogKind::Model
            || self.bytes == 0
            || self.title.is_empty()
            || self.title.len() > 512
            || self.title.chars().any(char::is_control)
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl SyncStore {
    pub(crate) fn copy_lease(&self) -> Result<CopyLease<'_>, PackError> {
        let file =
            crate::content::lease::file(&storage::path(&self.root, "packs/nearby/copy.lock")?)?;
        if !crate::content::lease::exclusive(&file)? {
            return Err(PackError::Mismatch(
                "Another nearby copy is already running".into(),
            ));
        }
        Ok(CopyLease {
            store: self,
            _file: file,
        })
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub fn open(root: &Path) -> Result<Self, PackError> {
        fs::create_dir_all(root)?;
        let store = Self {
            root: root.canonicalize()?,
        };
        for dir in ["versions", "installed", "leases"] {
            fs::create_dir_all(store.directory(dir)?)?;
        }
        Ok(store)
    }
    fn directory(&self, name: &str) -> Result<PathBuf, PackError> {
        storage::path(&self.root, &format!("packs/nearby/{name}/.path-check"))?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(invalid)
    }
    fn metadata(&self, manifest: &PackManifest) -> Result<PathBuf, PackError> {
        manifest.validate()?;
        storage::path(
            &self.root,
            &format!("packs/nearby/installed/{}.json", manifest.sha256),
        )
    }
    pub fn cache_spec(&self, manifest: &PackManifest) -> Result<DownloadSpec, PackError> {
        manifest.validate()?;
        Ok(DownloadSpec {
            id: manifest.local_id(),
            url: None,
            bytes: manifest.bytes,
            sha256: manifest.sha256.clone(),
            path: format!("packs/nearby/versions/{}.blob", manifest.sha256),
        })
    }
    pub fn open_source(&self, manifest: &PackManifest) -> Result<Arc<dyn PackSource>, PackError> {
        let _gate = storage::lock(&storage::path(&self.root, "packs/nearby/installed.lock")?)?;
        match Self::read_record(&self.metadata(manifest)?)? {
            CopyRecord::Live(row) if &row == manifest => self.open_leased(manifest),
            _ => Err(PackError::NotFound("Nearby copy".into())),
        }
    }
    fn open_leased(&self, manifest: &PackManifest) -> Result<Arc<dyn PackSource>, PackError> {
        let spec = self.cache_spec(manifest)?;
        let source: Arc<dyn PackSource> = Arc::new(crate::content::lease::LeasedSource::open(
            &storage::path(&self.root, &spec.path)?,
            crate::content::lease::file(&self.lease_path(manifest)?)?,
        )?);
        if source.len() != manifest.bytes {
            return Err(PackError::Truncated);
        }
        Ok(source)
    }
    /// The existing downloader stages, resumes and verifies bytes. Activation also
    /// validates archive structure before publishing any local-library reference.
    pub fn install_cached(&self, manifest: &PackManifest) -> Result<PackManifest, PackError> {
        self.install_cached_if(manifest, || true)?
            .ok_or_else(invalid)
    }
    pub(crate) fn install_cached_if(
        &self,
        manifest: &PackManifest,
        admitted: impl FnOnce() -> bool,
    ) -> Result<Option<PackManifest>, PackError> {
        let metadata = self.metadata(manifest)?;
        let _gate = storage::lock(&storage::path(&self.root, "packs/nearby/installed.lock")?)?;
        let spec = self.cache_spec(manifest)?;
        let path = storage::path(&self.root, &spec.path)?;
        crate::pack::verify_sha256(&path, &manifest.sha256)?;
        let source = self.open_leased(manifest)?;
        match manifest.kind {
            CatalogKind::Zim => crate::zim::Zim::open(source)?.verify_md5()?,
            CatalogKind::Map => {
                crate::tiles::TileSource::open(source)?;
            }
            CatalogKind::Model => return Err(invalid()),
        }
        if !admitted() {
            return Ok(None);
        }
        if metadata.exists() {
            let record = Self::read_record(&metadata)?;
            let saved = record.manifest();
            if saved.sha256 != manifest.sha256
                || saved.bytes != manifest.bytes
                || saved.kind != manifest.kind
            {
                return Err(invalid());
            }
            if matches!(record, CopyRecord::Live(_)) {
                return Ok(Some(saved.clone()));
            }
        }
        if fs::read_dir(self.directory("installed")?)?.count() >= 256 {
            return Err(invalid());
        }
        storage::durable(&path)?;
        crate::download::write_atomic(
            &metadata,
            &serde_json::to_vec(manifest).map_err(|_| invalid())?,
        )?;
        storage::durable(&metadata)?;
        Ok(Some(manifest.clone()))
    }
    fn read_record(path: &Path) -> Result<CopyRecord, PackError> {
        if fs::metadata(path)?.len() > 8192 {
            return Err(invalid());
        }
        let record: CopyRecord = serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid())?;
        record.manifest().validate()?;
        Ok(record)
    }
    pub fn list(&self) -> Result<Vec<PackManifest>, PackError> {
        Ok(self
            .inventory()?
            .into_iter()
            .filter(|row| !row.removal_pending)
            .map(|row| row.manifest)
            .collect())
    }
    pub(crate) fn inventory(&self) -> Result<Vec<CopyEntry>, PackError> {
        let _gate = storage::lock(&storage::path(&self.root, "packs/nearby/installed.lock")?)?;
        let mut rows = Vec::new();
        for entry in fs::read_dir(self.directory("installed")?)? {
            if rows.len() >= 256 {
                return Err(invalid());
            }
            let entry = entry?;
            let name = entry.file_name().to_str().ok_or_else(invalid)?.to_owned();
            let Some(hash) = name.strip_suffix(".json") else {
                return Err(invalid());
            };
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid());
            }
            let path = storage::path(&self.root, &format!("packs/nearby/installed/{name}"))?;
            let record = Self::read_record(&path)?;
            let manifest = record.manifest();
            if manifest.sha256 != hash {
                return Err(invalid());
            }
            if let CopyRecord::Live(manifest) = record {
                self.open_leased(&manifest)?;
                rows.push(CopyEntry {
                    manifest,
                    removal_pending: false,
                });
            } else if let CopyRecord::Removed(retired) = record {
                rows.push(CopyEntry {
                    manifest: retired.removed,
                    removal_pending: true,
                });
            }
        }
        rows.sort_by(|a, b| a.manifest.sha256.cmp(&b.manifest.sha256));
        Ok(rows)
    }
}
