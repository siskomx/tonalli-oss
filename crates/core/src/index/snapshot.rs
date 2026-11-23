//! Portable immutable search files; the caller supplies only authenticated, verified bytes.
use super::{guarded, identity, index_error, storage, Index, IndexMeta};
use crate::{
    pack::{PackError, PackSource, PreadSource},
    zim::Zim,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
    sync::Arc,
};

const MAX_FILES: usize = 4096;
const MAX_MANIFEST: usize = 1024 * 1024;
const REQUIRED: [&str; 3] = [".managed.json", "meta.json", "tonalli-meta.json"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecord {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub generation: String,
    pub meta: IndexMeta,
    pub files: Vec<FileRecord>,
}
pub struct Snapshot {
    manifest: Manifest,
    sources: BTreeMap<String, Arc<dyn PackSource>>,
}
fn invalid() -> PackError {
    PackError::Corrupt("Invalid portable article search".into())
}
fn filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !name.is_empty()
        && name.len() <= 160
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && ![
            "CURRENT",
            "BUILDING",
            "build.lock",
            "tonalli-building.json",
            ".tantivy-meta.lock",
            ".tantivy-writer.lock",
        ]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
        && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}
fn ordinary(path: &Path, directory: bool) -> Result<(), PackError> {
    let meta = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = meta.file_type().is_symlink();
    if linked
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file()
        }
    {
        return Err(invalid());
    }
    Ok(())
}
impl Manifest {
    /// Bounds and flat paths apply before any destination or source I/O.
    pub fn validate(&self) -> Result<u64, PackError> {
        if self.version != 1
            || !crate::content::storage::hex(&self.generation)
            || !(3..=MAX_FILES).contains(&self.files.len())
            || serde_json::to_vec(self).map_err(index_error)?.len() > MAX_MANIFEST
        {
            return Err(invalid());
        }
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            if !filename(&file.name)
                || !names.insert(file.name.to_ascii_lowercase())
                || !crate::content::storage::hex(&file.sha256)
            {
                return Err(invalid());
            }
            total = total
                .checked_add(file.bytes)
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(invalid)?;
        }
        if !REQUIRED
            .iter()
            .all(|name| self.files.iter().any(|f| f.name == *name))
        {
            return Err(invalid());
        }
        Ok(total)
    }
    pub fn from_slice(bytes: &[u8]) -> Result<Self, PackError> {
        if bytes.len() > MAX_MANIFEST {
            return Err(invalid());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(index_error)?;
        value.validate()?;
        Ok(value)
    }
    pub fn check_source(&self, zim: &Zim) -> Result<u64, PackError> {
        let bytes = self.validate()?;
        if self.meta != identity(zim)? {
            return Err(PackError::Mismatch(
                "Search source or analysis profile differs".into(),
            ));
        }
        Ok(bytes)
    }
}
impl Snapshot {
    pub fn export(index: &Index, zim: &Zim) -> Result<Self, PackError> {
        Self::export_controlled(index, zim, || true)?
            .ok_or_else(|| PackError::Io("Search export stopped".into()))
    }
    pub fn export_controlled(
        index: &Index,
        zim: &Zim,
        mut admitted: impl FnMut() -> bool,
    ) -> Result<Option<Self>, PackError> {
        guarded(|| {
            index.check_zim(zim)?;
            let root = &index.generation.path;
            ordinary(root, true)?;
            if storage::read_meta(root)? != index.generation.meta {
                return Err(invalid());
            }
            // Tantivy's live file inventory excludes unrelated files and runtime locks.
            let mut names = index
                .index
                .directory()
                .list_managed_files()
                .into_iter()
                .map(|p| p.to_str().map(str::to_owned).ok_or_else(invalid))
                .collect::<Result<BTreeSet<_>, _>>()?;
            names.extend([".managed.json".into(), "tonalli-meta.json".into()]);
            if names.len() > MAX_FILES {
                return Err(invalid());
            }
            let mut sources = BTreeMap::new();
            let mut files = Vec::new();
            for name in names {
                if !filename(&name) {
                    return Err(invalid());
                }
                let path = root.join(&name);
                ordinary(&path, false)?;
                let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&path)?);
                let Some(digest) = read_source(source.as_ref(), None, &mut admitted)? else {
                    return Ok(None);
                };
                files.push(FileRecord {
                    name: name.clone(),
                    bytes: source.len(),
                    sha256: digest,
                });
                sources.insert(name, source);
            }
            let manifest = Manifest {
                version: 1,
                generation: generation_id(index)?,
                meta: index.generation.meta.clone(),
                files,
            };
            manifest.validate()?;
            check_managed(root, &manifest)?;
            index.check_zim(zim)?;
            if storage::read_meta(root)? != manifest.meta {
                return Err(invalid());
            }
            Ok(Some(Self { manifest, sources }))
        })
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn source(&self, name: &str) -> Result<Arc<dyn PackSource>, PackError> {
        self.sources
            .get(name)
            .cloned()
            .ok_or_else(|| PackError::NotFound("Search file".into()))
    }
    /// Only files owned by the selected live generation can be requested over LAN.
    pub fn open_file(
        index: &Index,
        zim: &Zim,
        generation: &str,
        name: &str,
    ) -> Result<Arc<dyn PackSource>, PackError> {
        guarded(|| {
            if !filename(name) || generation_id(index)? != generation {
                return Err(PackError::NotFound("Search generation".into()));
            }
            index.check_zim(zim)?;
            let root = &index.generation.path;
            ordinary(root, true)?;
            if storage::read_meta(root)? != index.generation.meta {
                return Err(invalid());
            }
            if !matches!(name, ".managed.json" | "tonalli-meta.json")
                && !index
                    .index
                    .directory()
                    .list_managed_files()
                    .contains(Path::new(name))
            {
                return Err(PackError::NotFound("Search file".into()));
            }
            let path = root.join(name);
            ordinary(&path, false)?;
            Ok(Arc::new(PreadSource::open(&path)?) as Arc<dyn PackSource>)
        })
    }
}

fn generation_id(index: &Index) -> Result<String, PackError> {
    // An opaque identity binds requests to the selected immutable directory, not CURRENT.
    let bytes = serde_json::to_vec(&(&index.generation.path, &index.generation.meta))
        .map_err(index_error)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
pub fn install(
    root: &Path,
    zim: &Zim,
    manifest: &Manifest,
    mut source: impl FnMut(&FileRecord) -> Result<Arc<dyn PackSource>, PackError>,
    mut admitted: impl FnMut() -> bool,
) -> Result<Option<Index>, PackError> {
    guarded(|| {
        let bytes = manifest.check_source(zim)?;
        if !admitted() {
            return Ok(None);
        }
        fs::create_dir_all(root)?;
        ordinary(root, true)?;
        let _owner = super::build::build_lock(root)?;
        if root.join("BUILDING").try_exists()? {
            return Err(PackError::Mismatch(
                "Search preparation must finish before importing".into(),
            ));
        }
        if fs2::available_space(root)? < bytes.checked_add(32 * 1024 * 1024).ok_or_else(invalid)? {
            return Err(PackError::NoSpace);
        }
        let staged = tempfile::Builder::new()
            .prefix("generation-")
            .tempdir_in(root)?;
        for row in &manifest.files {
            if !admitted() {
                return Ok(None);
            }
            let input = source(row)?;
            if input.len() != row.bytes {
                return Err(PackError::Truncated);
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(staged.path().join(&row.name))?;
            let Some(digest) = read_source(input.as_ref(), Some(&mut output), &mut admitted)?
            else {
                return Ok(None);
            };
            if digest != row.sha256 {
                return Err(PackError::Checksum);
            }
            output.sync_all()?;
        }
        check_managed(staged.path(), manifest)?;
        let ready = Index::open_generation(staged.path(), zim)?;
        if !ready
            .index
            .validate_checksum()
            .map_err(index_error)?
            .is_empty()
        {
            return Err(PackError::Checksum);
        }
        if !admitted() {
            return Ok(None);
        }
        ready.check_zim(zim)?;
        // Retain a validated generation even if pointer publication is uncertain.
        let path = staged.keep();
        storage::point(root, "CURRENT", &path)?;
        Ok(Some(ready))
    })
}

fn read_source(
    source: &dyn PackSource,
    mut output: Option<&mut fs::File>,
    admitted: &mut impl FnMut() -> bool,
) -> Result<Option<String>, PackError> {
    let mut digest = Sha256::new();
    let mut offset = 0;
    let mut buffer = [0u8; 64 * 1024];
    while offset < source.len() {
        if !admitted() {
            return Ok(None);
        }
        let count = (source.len() - offset).min(buffer.len() as u64) as usize;
        let read = source.read_at(offset, &mut buffer[..count])?;
        if read == 0 {
            return Err(PackError::Truncated);
        }
        if read > count {
            return Err(invalid());
        }
        digest.update(&buffer[..read]);
        if let Some(file) = output.as_deref_mut() {
            file.write_all(&buffer[..read])?;
        }
        offset += read as u64;
    }
    Ok(Some(format!("{:x}", digest.finalize())))
}
fn check_managed(root: &Path, manifest: &Manifest) -> Result<(), PackError> {
    let raw = storage::read_bounded(&root.join(".managed.json"), MAX_MANIFEST as u64)?;
    let files: Vec<String> = serde_json::from_slice(&raw).map_err(index_error)?;
    let actual = files.iter().collect::<BTreeSet<_>>();
    let expected = manifest
        .files
        .iter()
        .map(|f| &f.name)
        .filter(|n| !matches!(n.as_str(), ".managed.json" | "tonalli-meta.json"))
        .collect::<BTreeSet<_>>();
    if actual.len() != files.len() || actual != expected {
        return Err(invalid());
    }
    Ok(())
}
