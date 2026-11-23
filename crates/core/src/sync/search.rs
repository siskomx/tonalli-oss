//! Temporary verified search bytes for one owned book, never an installed generation.
use super::*;
use crate::index::snapshot::{FileRecord, Manifest};
use std::collections::BTreeSet;

impl SyncStore {
    fn search_directory(&self, book: &PackManifest) -> Result<PathBuf, PackError> {
        book.validate()?;
        storage::path(
            &self.root,
            &format!("packs/nearby/search/{}/.path-check", book.sha256),
        )?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(invalid)
    }
    pub(crate) fn search_spec(
        &self,
        book: &PackManifest,
        file: &FileRecord,
    ) -> Result<DownloadSpec, PackError> {
        book.validate()?;
        if !storage::hex(&file.sha256) {
            return Err(invalid());
        }
        let path = format!("packs/nearby/search/{}/{}.blob", book.sha256, file.sha256);
        storage::path(&self.root, &path)?;
        Ok(DownloadSpec {
            id: format!("search-{}", file.sha256),
            url: None,
            bytes: file.bytes,
            sha256: file.sha256.clone(),
            path,
        })
    }
}
impl CopyLease<'_> {
    fn search_paths(&self, book: &PackManifest) -> Result<Vec<PathBuf>, PackError> {
        let directory = self.store.search_directory(book)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut paths = Vec::new();
        for entry in entries {
            if paths.len() >= 3 * 4096 {
                return Err(invalid());
            }
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_| invalid())?;
            let hash = name
                .strip_suffix(".blob.part.meta")
                .or_else(|| name.strip_suffix(".blob.part"))
                .or_else(|| name.strip_suffix(".blob"))
                .ok_or_else(invalid)?;
            if !storage::hex(hash) {
                return Err(invalid());
            }
            let path = storage::path(
                &self.store.root,
                &format!("packs/nearby/search/{}/{name}", book.sha256),
            )?;
            if !fs::symlink_metadata(&path)?.is_file() {
                return Err(invalid());
            }
            paths.push(path);
        }
        Ok(paths)
    }
    pub(crate) fn reserve_search(
        &self,
        book: &PackManifest,
        offer: &Manifest,
    ) -> Result<(), PackError> {
        let total = offer.validate()?;
        let mut keep = BTreeSet::new();
        for file in &offer.files {
            keep.insert(file.sha256.as_str());
        }
        // Retire only validated cache files that cannot belong to this offer.
        for path in self.search_paths(book)? {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(invalid)?;
            if !keep.contains(name.get(..64).ok_or_else(invalid)?) {
                fs::remove_file(path)?;
            }
        }
        let mut missing = 0u64;
        let mut seen = BTreeSet::new();
        for file in &offer.files {
            if !seen.insert(&file.sha256) {
                continue;
            }
            let spec = self.store.search_spec(book, file)?;
            let mut present = 0;
            for relative in [&spec.path, &format!("{}.part", spec.path)] {
                let path = storage::path(self.store.root(), relative)?;
                match fs::metadata(path) {
                    Ok(meta) => {
                        present = meta.len().min(file.bytes);
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            missing = missing
                .checked_add(file.bytes - present)
                .ok_or(PackError::NoSpace)?;
        }
        let required = total
            .checked_add(missing)
            .and_then(|n| n.checked_add(32 * 1024 * 1024))
            .ok_or(PackError::NoSpace)?;
        if fs2::available_space(self.store.root())? < required {
            return Err(PackError::NoSpace);
        }
        Ok(())
    }
    pub(crate) fn clear_search(&self, book: &PackManifest) -> Result<(), PackError> {
        let directory = self.store.search_directory(book)?;
        let paths = self.search_paths(book)?;
        if !directory.try_exists()? {
            return Ok(());
        }
        // Validate the whole inventory before deleting only these exact cache files.
        for path in paths {
            fs::remove_file(path)?;
        }
        #[cfg(unix)]
        fs::File::open(&directory)?.sync_all()?;
        fs::remove_dir(&directory)?;
        #[cfg(unix)]
        fs::File::open(directory.parent().ok_or_else(invalid)?)?.sync_all()?;
        Ok(())
    }
}
