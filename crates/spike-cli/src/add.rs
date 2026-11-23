use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tonalli_core::pack::{verify_sha256, PackError};

use crate::fetch::check_space;
use crate::manifest::{ensure_regular_or_absent, manifest_dir, pack_path, Manifest, Pack};

/// Copies local files into pack storage and records fingerprints, not publisher trust or format validity.
pub fn add_path(manifest_path: &Path, source: &Path) -> Result<Vec<String>, PackError> {
    let mut manifest = Manifest::load(manifest_path)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(PackError::Mismatch("imports must not be symlinks".into()));
    }
    let mut sources: Vec<PathBuf> = if metadata.is_dir() {
        let mut entries = Vec::new();
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                return Err(PackError::Mismatch(
                    "import folder contains a symlink".into(),
                ));
            }
            if entry.file_type()?.is_file() && kind(&entry.path()).is_some() {
                entries.push(entry.path());
            }
        }
        entries
    } else {
        ensure_regular_or_absent(source)?;
        if kind(source).is_none() {
            return Err(PackError::Mismatch(
                "supported imports: .zim, .pmtiles, .iso, .gguf".into(),
            ));
        }
        vec![source.to_owned()]
    };
    sources.sort();
    let base = manifest_dir(manifest_path);
    let storage = pack_path(base, "packs/import.tmp")?
        .parent()
        .ok_or_else(|| PackError::Mismatch("missing storage directory".into()))?
        .to_owned();
    fs::create_dir_all(&storage)?;
    let mut added = Vec::new();
    for source in sources {
        let (pack_kind, extension) = kind(&source)
            .ok_or_else(|| PackError::Mismatch("unsupported pack extension".into()))?;
        let bytes = fs::metadata(&source)?.len();
        check_space(fs2::available_space(&storage)?, bytes)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&storage)?;
        let copied = std::io::copy(&mut File::open(&source)?, &mut temporary)?;
        if copied != bytes {
            return Err(PackError::Truncated);
        }
        temporary.as_file().sync_all()?;
        temporary.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = temporary.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let hash = format!("{:x}", digest.finalize());
        if let Some(existing) = manifest
            .packs
            .iter()
            .find(|pack| pack.sha256 == hash && pack.kind == pack_kind)
        {
            let destination = pack_path(base, &existing.path)?;
            if verify_sha256(&destination, &hash).is_err() {
                temporary
                    .persist(&destination)
                    .map_err(|error| PackError::Io(error.error.to_string()))?;
            }
            continue;
        }
        let id = format!("local-{pack_kind}-{hash}");
        let relative = format!("packs/{id}.{extension}");
        let destination = pack_path(base, &relative)?;
        if destination.exists() {
            verify_sha256(&destination, &hash)?;
        } else {
            temporary
                .persist_noclobber(&destination)
                .map_err(|error| PackError::Io(error.error.to_string()))?;
        }
        let mut extra = toml::Table::new();
        extra.insert("provenance".into(), toml::Value::String(format!(
            "Local import from {}; SHA-256 is a locally computed fingerprint, not publisher-verified. Extension-only classification; format not validated.", source.display()
        )));
        manifest.packs.push(Pack {
            id: id.clone(),
            kind: pack_kind.into(),
            version: "local".into(),
            url: None,
            bytes,
            sha256: hash,
            path: relative,
            deferred: false,
            extra,
        });
        // Commit each successful import; a later bad file does not lose earlier registrations.
        manifest.save(manifest_path)?;
        added.push(id);
    }
    Ok(added)
}

fn kind(path: &Path) -> Option<(&'static str, &'static str)> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "zim" => Some(("zim", "zim")),
        "pmtiles" => Some(("pmtiles", "pmtiles")),
        "iso" => Some(("iso", "iso")),
        "gguf" => Some(("model", "gguf")),
        _ => None,
    }
}
