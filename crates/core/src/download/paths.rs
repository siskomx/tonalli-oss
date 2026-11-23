//! App-owned pack paths and the existing resumable-download metadata write.
use crate::pack::PackError;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Resolves only ordinary paths under the manifest's packs directory.
pub fn pack_path(base: &Path, relative: &str) -> Result<PathBuf, PackError> {
    let components: Vec<_> = Path::new(relative).components().collect();
    if components.len() < 2
        || components[0] != Component::Normal("packs".as_ref())
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(PackError::Mismatch(
            "pack destination must be relative under packs/".into(),
        ));
    }
    let root = base.canonicalize()?;
    let mut path = root.clone();
    for component in components {
        path.push(component.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !path.canonicalize()?.starts_with(&root) {
                    return Err(PackError::Mismatch(
                        "pack path traverses a symlink or escapes its root".into(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    ensure_regular_or_absent(&path)?;
    Ok(path)
}

pub fn ensure_regular_or_absent(path: &Path) -> Result<(), PackError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            PackError::Mismatch(format!("not an ordinary file: {}", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PackError> {
    ensure_regular_or_absent(path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent_dir(path))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| PackError::Io(error.error.to_string()))?;
    Ok(())
}
