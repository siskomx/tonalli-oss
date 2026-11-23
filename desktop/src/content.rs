//! Native-owned references to user-selected files; no filesystem paths enter HTTP.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tonalli_core::{
    pack::{ConcatSource, PackError, PackSource, PreadSource},
    server::PackId,
    zim::Zim,
};

const MAX_BYTES: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    path: PathBuf,
    name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderEntry {
    pub id: String,
    parts: Vec<Part>,
    uuid: [u8; 16],
    stored_md5: [u8; 16],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    entries: Vec<ReaderEntry>,
}

pub struct ReaderCatalog {
    path: PathBuf,
    entries: Vec<ReaderEntry>,
}

pub struct FolderOpen {
    pub opened: Vec<(String, Arc<Zim>)>,
    pub failed: usize,
    pub ignored: usize,
}

fn invalid() -> PackError {
    PackError::Mismatch("Invalid private reader catalog".into())
}

fn not_linked(path: &Path) -> Result<(), PackError> {
    match fs::symlink_metadata(path) {
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
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

impl ReaderCatalog {
    pub fn load(data: &Path) -> Result<Self, PackError> {
        let parent = data.canonicalize()?.join("reader");
        let path = parent.join("catalog.json");
        not_linked(&parent)?;
        not_linked(&path)?;
        let entries = match fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_BYTES {
                    return Err(invalid());
                }
                let stored: Stored = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if stored.version != 1 {
                    return Err(invalid());
                }
                Self::check(&stored.entries)?;
                stored.entries
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, entries })
    }

    pub fn entries(&self) -> &[ReaderEntry] {
        &self.entries
    }

    /// Scan only immediate children before publishing any group. Each group then
    /// uses the same checksum-verified atomic catalog update as the file picker.
    pub fn add_folder(&mut self, folder: &Path) -> Result<FolderOpen, PackError> {
        not_linked(folder)?;
        let mut groups = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
        let mut ignored = 0;
        for (index, child) in fs::read_dir(folder)?.enumerate() {
            if index >= 256 {
                return Err(PackError::Mismatch(
                    "Choose a folder with at most 256 immediate entries".into(),
                ));
            }
            let child = child?;
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() || not_linked(&path).is_err() {
                ignored += 1;
                continue;
            }
            let Some(name) = child.file_name().to_str().map(str::to_owned) else {
                ignored += 1;
                continue;
            };
            let group = if name.ends_with(".zim") {
                Some(name.clone())
            } else {
                name.rsplit_once(".zim").and_then(|(stem, suffix)| {
                    (suffix.len() == 2 && suffix.bytes().all(|byte| byte.is_ascii_lowercase()))
                        .then(|| format!("{stem}.zim"))
                })
            };
            if let Some(group) = group {
                groups.entry(group).or_default().push(path);
            } else {
                ignored += 1;
            }
        }
        let mut result = FolderOpen {
            opened: Vec::new(),
            failed: 0,
            ignored,
        };
        for mut paths in groups.into_values() {
            paths.sort();
            match self.add_paths(paths) {
                Ok(opened) => result.opened.push(opened),
                Err(_) => result.failed += 1,
            }
        }
        Ok(result)
    }

    fn check(entries: &[ReaderEntry]) -> Result<(), PackError> {
        let mut ids = std::collections::HashSet::new();
        if entries.len() > MAX_ENTRIES {
            return Err(invalid());
        }
        for entry in entries {
            PackId::new(entry.id.clone())?;
            if !entry.id.starts_with("reader-")
                || !ids.insert(&entry.id)
                || entry.parts.is_empty()
                || entry.parts.len() > 128
            {
                return Err(invalid());
            }
            for part in &entry.parts {
                if !part.path.is_absolute()
                    || part.name.is_empty()
                    || part.name.len() > 4096
                    || part.name.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    fn open_parts(parts: &[Part]) -> Result<Arc<Zim>, PackError> {
        let sources = parts
            .iter()
            .map(|part| {
                let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&part.path)?);
                Ok((part.name.clone(), source))
            })
            .collect::<Result<Vec<_>, PackError>>()?;
        Ok(Arc::new(Zim::open(Arc::new(ConcatSource::from_parts(
            sources,
        )?))?))
    }

    pub fn open_entry(entry: &ReaderEntry) -> Result<Arc<Zim>, PackError> {
        Self::check(std::slice::from_ref(entry))?;
        let zim = Self::open_parts(&entry.parts)?;
        if zim.uuid() != entry.uuid || zim.stored_md5()? != entry.stored_md5 {
            return Err(PackError::Mismatch(
                "Saved source identity changed; reopen it explicitly".into(),
            ));
        }
        Ok(zim)
    }

    pub fn add_paths(&mut self, paths: Vec<PathBuf>) -> Result<(String, Arc<Zim>), PackError> {
        if paths.is_empty() || paths.len() > 128 {
            return Err(invalid());
        }
        let parts = paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(invalid)?
                    .to_owned();
                Ok(Part {
                    path: path.canonicalize()?,
                    name,
                })
            })
            .collect::<Result<Vec<_>, PackError>>()?;
        let zim = Self::open_parts(&parts)?;
        zim.verify_md5()?;
        let uuid = zim.uuid();
        let stored_md5 = zim.stored_md5()?;
        if let Some(entry) = self.entries.iter().find(|entry| {
            entry.uuid == uuid
                && entry.stored_md5 == stored_md5
                && entry.parts.len() == parts.len()
                && entry
                    .parts
                    .iter()
                    .zip(&parts)
                    .all(|(a, b)| a.path == b.path && a.name == b.name)
        }) {
            return Ok((entry.id.clone(), zim));
        }
        let id = (1..=MAX_ENTRIES)
            .map(|number| format!("reader-{number}"))
            .find(|id| self.entries.iter().all(|entry| &entry.id != id))
            .ok_or_else(invalid)?;
        let mut next = self.entries.clone();
        next.push(ReaderEntry {
            id: id.clone(),
            parts,
            uuid,
            stored_md5,
        });
        Self::check(&next)?;
        let bytes = serde_json::to_vec(&Stored {
            version: 1,
            entries: next.clone(),
        })
        .map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let parent = self.path.parent().ok_or_else(invalid)?;
        not_linked(parent)?;
        not_linked(&self.path)?;
        fs::create_dir_all(parent)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged
            .persist(&self.path)
            .map_err(|error| PackError::from(error.error))?;
        self.entries = next;
        Ok((id, zim))
    }
}
