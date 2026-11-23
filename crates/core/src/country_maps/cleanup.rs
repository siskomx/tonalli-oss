use super::CountryMaps;
use crate::pack::PackError;
use fs2::FileExt;
use std::fs;

impl CountryMaps {
    /// Remove only generated files inside the native-owned store, after publication.
    pub fn cleanup(&self) -> Result<(), PackError> {
        let worker = super::store::lock(&self.root.join("worker.lock"))?;
        if worker.try_lock_exclusive().is_err() {
            return Ok(());
        }
        for row in self.list()? {
            if !matches!(row.state.as_str(), "completed" | "cancelled") {
                continue;
            }
            let directory = self.root.join(&row.id);
            if !directory.try_exists()? {
                continue;
            }
            if !directory.canonicalize()?.starts_with(&self.root) {
                return Err(PackError::Corrupt("Map cleanup path".into()));
            }
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    return Err(PackError::Corrupt("Linked map staging".into()));
                }
                if kind.is_dir()
                    && name
                        .strip_prefix("cache-")
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    let path = entry.path();
                    if !path.canonicalize()?.starts_with(&self.root) {
                        return Err(PackError::Corrupt("Map cache path".into()));
                    }
                    fs::remove_dir_all(path)?;
                } else if kind.is_file() {
                    let staged = name.starts_with("stage-") && name.ends_with(".pmtiles");
                    let generated = name
                        .strip_suffix(".pmtiles")
                        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
                    if staged
                        || generated.is_some_and(|sha| {
                            row.installed.as_ref().is_none_or(|map| map.sha256 != sha)
                        })
                    {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
        }
        Ok(())
    }
}
