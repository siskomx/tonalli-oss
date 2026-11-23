use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::IndexMeta;
use crate::pack::PackError;

pub(super) fn generation(root: &Path) -> Result<PathBuf, PackError> {
    pointed_generation(root, "CURRENT")
}

pub(super) fn pointed_generation(root: &Path, pointer: &str) -> Result<PathBuf, PackError> {
    let path = pointed_path(root, pointer)?;
    check_generation(&path)?;
    Ok(path)
}

pub(super) fn pointed_path(root: &Path, pointer: &str) -> Result<PathBuf, PackError> {
    let pointer = root.join(pointer);
    let name = read_bounded(&pointer, 256)?;
    let name = std::str::from_utf8(&name)
        .map_err(|_| corrupt("invalid generation name"))?
        .trim();
    let mut components = Path::new(name).components();
    if !name.starts_with("generation-")
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(corrupt("generation must name an internal index directory"));
    }
    Ok(root.join(name))
}

pub(super) fn check_generation(path: &Path) -> Result<(), PackError> {
    let info = fs::symlink_metadata(path)?;
    if !info.is_dir() || info.file_type().is_symlink() {
        return Err(corrupt("index generation is not an ordinary directory"));
    }
    for entry in fs::read_dir(path)? {
        if !entry?.file_type()?.is_file() {
            return Err(corrupt("index generation contains a symlink or non-file"));
        }
    }
    Ok(())
}

pub(super) fn read_meta(root: &Path) -> Result<IndexMeta, PackError> {
    serde_json::from_slice(&read_bounded(&root.join("tonalli-meta.json"), 64 * 1024)?)
        .map_err(|error| PackError::Corrupt(error.to_string()))
}

pub(super) fn write_meta(root: &Path, meta: &IndexMeta) -> Result<(), PackError> {
    write_identity(&root.join("tonalli-meta.json"), meta)
}

pub(super) const OWNER: &str = "tonalli-building.json";

pub(super) fn write_owner(root: &Path, meta: &IndexMeta) -> Result<(), PackError> {
    write_identity(&root.join(OWNER), meta)
}

pub(super) fn read_owner(root: &Path) -> Result<IndexMeta, PackError> {
    serde_json::from_slice(&read_bounded(&root.join(OWNER), 64 * 1024)?)
        .map_err(|error| PackError::Corrupt(error.to_string()))
}

fn write_identity(path: &Path, meta: &IndexMeta) -> Result<(), PackError> {
    let bytes =
        serde_json::to_vec_pretty(meta).map_err(|error| PackError::Corrupt(error.to_string()))?;
    let mut file = fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn point(root: &Path, pointer_name: &str, staged: &Path) -> Result<(), PackError> {
    let name = staged
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| corrupt("invalid staged generation name"))?
        .to_owned();
    let mut pointer = tempfile::NamedTempFile::new_in(root)?;
    pointer.write_all(name.as_bytes())?;
    pointer.as_file().sync_all()?;
    let published = pointer
        .persist(root.join(pointer_name))
        .map_err(|error| PackError::Io(error.error.to_string()))?;
    published.sync_all().map_err(|error| {
        PackError::PublicationUncertain(format!("index pointer flush failed: {error}"))
    })?;
    #[cfg(unix)]
    fs::File::open(root)?.sync_all().map_err(|error| {
        PackError::PublicationUncertain(format!("index pointer directory flush failed: {error}"))
    })?;
    Ok(())
}

pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, PackError> {
    let info = fs::symlink_metadata(path)?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() > limit {
        return Err(corrupt("invalid index metadata file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(corrupt("index metadata grew beyond its size limit"));
    }
    Ok(bytes)
}
fn corrupt(message: &str) -> PackError {
    PackError::Corrupt(message.into())
}
