use super::CountryMaps;
use crate::{
    download::{ensure_regular_or_absent, write_atomic},
    pack::{verify_sha256, PackError, PackSource, PreadSource},
    tiles::TileSource,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, sync::Arc};

#[derive(Clone, Serialize, Deserialize)]
pub struct ImportedMap {
    pub id: String,
    pub title: String,
    pub bytes: u64,
    pub sha256: String,
}
impl CountryMaps {
    pub fn import(
        &self,
        source: Arc<dyn PackSource>,
        title: String,
    ) -> Result<ImportedMap, PackError> {
        TileSource::open(Arc::clone(&source))?;
        let gate = super::store::lock(&self.root.join("worker.lock"))?;
        gate.try_lock_exclusive().map_err(|_| super::busy())?;
        if self.list()?.iter().any(|row| super::active(&row.state)) {
            return Err(super::busy());
        }
        if source.len() > fs2::available_space(&self.root)?.saturating_sub(64 * 1024 * 1024) {
            return Err(PackError::NoSpace);
        }
        let directory = self.root.join("imports");
        fs::create_dir_all(&directory)?;
        let mut stage = tempfile::NamedTempFile::new_in(&directory)?;
        let mut offset = 0;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        while offset < source.len() {
            let take = (source.len() - offset).min(buffer.len() as u64) as usize;
            let count = source.read_at(offset, &mut buffer[..take])?;
            if count == 0 {
                return Err(PackError::Truncated);
            }
            stage.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            offset += count as u64;
        }
        stage.as_file().sync_all()?;
        TileSource::open(Arc::new(PreadSource::open(stage.path())?))?;
        let sha256 = format!("{:x}", digest.finalize());
        let path = directory.join(format!("{sha256}.pmtiles"));
        ensure_regular_or_absent(&path)?;
        if !path.try_exists()? {
            stage
                .persist(&path)
                .map_err(|e| PackError::Io(e.error.to_string()))?;
        }
        verify_sha256(&path, &sha256)?;
        let title: String = title
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        let row = ImportedMap {
            id: format!("mapfile-{sha256}"),
            title: if title.is_empty() {
                "Map file".into()
            } else {
                title
            },
            bytes: offset,
            sha256,
        };
        let mut entries = self.imports()?;
        entries.retain(|entry| entry.id != row.id);
        if entries.len() >= 32 {
            return Err(PackError::Mismatch("Too many imported maps".into()));
        }
        entries.push(row.clone());
        write_atomic(
            &self.root.join("imports.json"),
            &serde_json::to_vec(&entries)
                .map_err(|_| PackError::Corrupt("Imported maps".into()))?,
        )?;
        Ok(row)
    }

    pub fn imports(&self) -> Result<Vec<ImportedMap>, PackError> {
        let path = self.root.join("imports.json");
        ensure_regular_or_absent(&path)?;
        if !path.try_exists()? {
            return Ok(Vec::new());
        }
        if fs::metadata(&path)?.len() > 65536 {
            return Err(PackError::Corrupt("Imported maps too large".into()));
        }
        let rows: Vec<ImportedMap> = serde_json::from_slice(&fs::read(path)?)
            .map_err(|_| PackError::Corrupt("Imported maps".into()))?;
        if rows.len() > 32
            || rows.iter().any(|r| {
                r.sha256.len() != 64
                    || !r.sha256.bytes().all(|c| c.is_ascii_hexdigit())
                    || r.id != format!("mapfile-{}", r.sha256)
            })
        {
            return Err(PackError::Corrupt("Imported map reference".into()));
        }
        Ok(rows)
    }

    pub fn imported_path(&self, row: &ImportedMap) -> Result<PathBuf, PackError> {
        if row.sha256.len() != 64 || !row.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(PackError::Corrupt("Imported map reference".into()));
        }
        let path = self
            .root
            .join("imports")
            .join(format!("{}.pmtiles", row.sha256));
        ensure_regular_or_absent(&path)?;
        verify_sha256(&path, &row.sha256)?;
        Ok(path)
    }

    pub fn remove_import(&self, id: &str) -> Result<(), PackError> {
        let gate = super::store::lock(&self.root.join("worker.lock"))?;
        gate.try_lock_exclusive().map_err(|_| super::busy())?;
        let mut entries = self.imports()?;
        let index = entries
            .iter()
            .position(|row| row.id == id)
            .ok_or_else(|| PackError::NotFound("Imported map".into()))?;
        let removed = entries.remove(index);
        let path = self.imported_path(&removed)?;
        write_atomic(
            &self.root.join("imports.json"),
            &serde_json::to_vec(&entries)
                .map_err(|_| PackError::Corrupt("Imported maps".into()))?,
        )?;
        fs::remove_file(path)?;
        Ok(())
    }
}
