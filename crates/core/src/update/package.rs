//! Safe unpacking of a downloaded Windows update package into a fresh staging folder.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::pack::PackError;

const MAX_ENTRIES: usize = 20_000;

fn refused(reason: &str) -> PackError {
    PackError::Mismatch(format!("update package: {reason}"))
}

fn corrupt(_: zip::result::ZipError) -> PackError {
    PackError::Corrupt("update package is not a valid archive".into())
}

/// The path of an entry relative to the package folder, or an error if it could point anywhere else.
fn relative(name: &str, prefix: &str) -> Result<PathBuf, PackError> {
    // Windows tools write `\` between folders; it is a separator there, so read it as one. Every
    // part is still checked below, so `..\` and drive tricks stay refused.
    let name = name.replace('\\', "/");
    let name = name.as_str();
    let tail = name.strip_prefix(prefix).ok_or_else(|| {
        refused(&format!("entry outside the package folder: {}", name.chars().take(120).collect::<String>()))
    })?;
    let mut path = PathBuf::new();
    for part in tail.trim_end_matches('/').split('/') {
        let valid = !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with(['.', ' '])
            && part.len() <= 255
            && !part.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
            && !part.chars().any(char::is_control);
        if !valid {
            return Err(refused("unsafe path in package"));
        }
        path.push(part);
    }
    Ok(path)
}

fn partial_for(destination: &Path) -> Result<PathBuf, PackError> {
    let mut name = destination
        .file_name()
        .ok_or_else(|| refused("invalid destination"))?
        .to_os_string();
    name.push(".partial");
    Ok(destination.with_file_name(name))
}

fn extract(
    archive: &Path,
    partial: &Path,
    prefix: &str,
    required: &str,
    max_bytes: u64,
) -> Result<(), PackError> {
    let mut zip = ZipArchive::new(File::open(archive)?).map_err(corrupt)?;
    if zip.len() > MAX_ENTRIES {
        return Err(refused("too many entries"));
    }
    let mut planned = Vec::with_capacity(zip.len());
    let (mut total, mut has_required) = (0u64, false);
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(corrupt)?;
        let name = entry.name().map_err(corrupt)?.into_owned();
        if entry.is_symlink() {
            return Err(refused("links are not allowed"));
        }
        let path = relative(&name, prefix)?;
        if entry.is_dir() {
            planned.push((index, path, None));
            continue;
        }
        total = total
            .checked_add(entry.size())
            .filter(|total| *total <= max_bytes)
            .ok_or_else(|| refused("package is too large"))?;
        has_required |= path == Path::new(required);
        planned.push((index, path, Some(entry.size())));
    }
    if !has_required {
        return Err(refused("the application is missing"));
    }
    fs::create_dir_all(partial)?;
    for (index, path, size) in planned {
        let target = partial.join(&path);
        let Some(size) = size else {
            fs::create_dir_all(&target)?;
            continue;
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut entry = zip.by_index(index).map_err(corrupt)?;
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
        let written = io::copy(&mut (&mut entry).take(size), &mut output)?;
        if written != size {
            return Err(PackError::Truncated);
        }
        output.sync_all()?;
    }
    Ok(())
}

/// Unpacks every entry under `prefix` (for example `Windows-x64/`) into the new folder `destination`,
/// without that wrapper folder. The folder appears only if everything was valid, and an existing
/// folder is never merged into.
pub fn unpack(
    archive: &Path,
    destination: &Path,
    prefix: &str,
    required: &str,
    max_bytes: u64,
) -> Result<(), PackError> {
    if destination.exists() {
        return Err(refused("destination already exists"));
    }
    let partial = partial_for(destination)?;
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    match extract(archive, &partial, prefix, required, max_bytes) {
        Ok(()) => fs::rename(&partial, destination).map_err(|error| {
            let _ = fs::remove_dir_all(&partial);
            error.into()
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(&partial);
            Err(error)
        }
    }
}
