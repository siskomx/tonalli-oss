//! Update settings, the last verified offer and the staged package, all under the private pack
//! root. Everything read back from disk is verified again before it is trusted.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    normalize_source, parse_public_key, Asset, Manifest, Version, MANIFEST_FILE, SIGNATURE_FILE,
};
use crate::pack::{verify_sha256, PackError};

const SETTINGS_FILE: &str = "update.json";
const OFFER_FILE: &str = "update-offer.json";
const OFFER_SIGNATURE_FILE: &str = "update-offer.sig";
const MAX_SETTINGS_BYTES: u64 = 8 * 1024;
const MAX_MANIFEST_FETCH: usize = 256 * 1024;
const SIGNATURE_BYTES: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub source: String,
    pub public_key: String,
}

/// A downloaded package whose signature, size and hash all verified just now.
#[derive(Debug)]
pub struct Ready {
    pub path: PathBuf,
    pub version: String,
    pub asset: Asset,
}

pub struct Store {
    root: PathBuf,
}

fn io(error: std::io::Error) -> PackError {
    PackError::from(error)
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    /// A path below the pack root with no symlink or Windows junction at any existing step.
    fn owned(&self, relative: &str) -> Result<PathBuf, PackError> {
        let path = crate::download::pack_path(&self.root, relative)?;
        // `pack_path` resolves from the canonical root (the `\\?\` form on Windows).
        let root = self.root.canonicalize().map_err(io)?;
        let mut current = root.clone();
        let tail = path
            .strip_prefix(&root)
            .map_err(|_| PackError::Mismatch("update storage is unavailable".into()))?;
        for part in tail.components() {
            current.push(part);
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = metadata.file_type().is_symlink();
                if linked {
                    return Err(PackError::Mismatch("update storage is unavailable".into()));
                }
            }
        }
        Ok(path)
    }

    fn private(&self, name: &str) -> Result<PathBuf, PackError> {
        self.owned(&format!("packs/private/{name}"))
    }

    fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, PackError> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
        if bytes.len() as u64 > limit {
            return Err(PackError::Corrupt("update file is too large".into()));
        }
        Ok(Some(bytes))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), PackError> {
        let path = self.private(name)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        crate::download::write_atomic(&path, bytes)
    }

    fn forget_offer(&self) {
        for name in [OFFER_FILE, OFFER_SIGNATURE_FILE] {
            if let Ok(path) = self.private(name) {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub fn settings(&self) -> Result<Settings, PackError> {
        match Self::read_bounded(&self.private(SETTINGS_FILE)?, MAX_SETTINGS_BYTES)? {
            None => Ok(Settings::default()),
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| PackError::Corrupt("update settings are not valid".into())),
        }
    }

    pub fn save_settings(
        &self,
        enabled: bool,
        source: &str,
        public_key: &str,
    ) -> Result<Settings, PackError> {
        let missing = |what: &str| PackError::Mismatch(format!("update {what} is required"));
        let source = source.trim();
        let public_key = public_key.trim();
        if enabled && source.is_empty() {
            return Err(missing("source"));
        }
        if enabled && public_key.is_empty() {
            return Err(missing("trusted key"));
        }
        let source = if source.is_empty() { String::new() } else { normalize_source(source)? };
        let public_key = if public_key.is_empty() {
            String::new()
        } else {
            parse_public_key(public_key)?
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        let settings = Settings { enabled, source, public_key };
        let bytes = serde_json::to_vec(&settings)
            .map_err(|_| PackError::Corrupt("update settings are not valid".into()))?;
        self.write(SETTINGS_FILE, &bytes)?;
        Ok(settings)
    }

    fn trusted(&self) -> Result<Option<(Settings, [u8; 32])>, PackError> {
        let settings = self.settings()?;
        if !settings.enabled || settings.source.is_empty() {
            return Ok(None);
        }
        let key = parse_public_key(&settings.public_key)?;
        Ok(Some((settings, key)))
    }

    /// Asks the configured source for its newest signed release. `fetch` receives the address and a
    /// byte limit. A release is remembered only after its signature verified.
    pub fn check(
        &self,
        installed: &Version,
        fetch: impl Fn(&str, usize) -> Result<Vec<u8>, PackError>,
    ) -> Result<Option<Manifest>, PackError> {
        let (settings, key) = self
            .trusted()?
            .ok_or_else(|| PackError::Mismatch("updates are off".into()))?;
        let payload = fetch(&format!("{}{MANIFEST_FILE}", settings.source), MAX_MANIFEST_FETCH)?;
        let signature = fetch(&format!("{}{SIGNATURE_FILE}", settings.source), SIGNATURE_BYTES)?;
        let manifest = Manifest::verify(&payload, &signature, &key)?;
        if !manifest.is_newer_than(installed) {
            self.forget_offer();
            return Ok(None);
        }
        self.write(OFFER_FILE, &payload)?;
        self.write(OFFER_SIGNATURE_FILE, &signature)?;
        Ok(Some(manifest))
    }

    /// The remembered release for this platform, if it still verifies against the trusted key.
    pub fn offer(
        &self,
        installed: &Version,
        platform: &str,
    ) -> Result<Option<(Manifest, Asset)>, PackError> {
        let Some((_, key)) = self.trusted()? else {
            return Ok(None);
        };
        let (Some(payload), Some(signature)) = (
            Self::read_bounded(&self.private(OFFER_FILE)?, MAX_MANIFEST_FETCH as u64)?,
            Self::read_bounded(&self.private(OFFER_SIGNATURE_FILE)?, SIGNATURE_BYTES as u64)?,
        ) else {
            return Ok(None);
        };
        let Ok(manifest) = Manifest::verify(&payload, &signature, &key) else {
            return Ok(None);
        };
        if !manifest.is_newer_than(installed) {
            return Ok(None);
        }
        Ok(manifest
            .asset_for(platform)
            .cloned()
            .map(|asset| (manifest, asset)))
    }

    /// Where the package for this release is staged, relative to the pack root.
    pub fn staged_name(manifest: &Manifest, platform: &str) -> Result<String, PackError> {
        let extension = match platform {
            "android-arm64" => "apk",
            "windows-x64" => "zip",
            "ios-arm64" => "ipa",
            _ => return Err(PackError::Mismatch("this platform has no update package".into())),
        };
        Ok(format!(
            "packs/updates/{platform}-{}.{extension}",
            manifest.version_text()
        ))
    }

    /// A cheap status check: a staged file of exactly the signed size exists. Not proof of integrity;
    /// `ready` hashes it before anything is installed.
    pub fn staged(&self, installed: &Version, platform: &str) -> Result<bool, PackError> {
        let Some((manifest, asset)) = self.offer(installed, platform)? else {
            return Ok(false);
        };
        let path = self.owned(&Self::staged_name(&manifest, platform)?)?;
        Ok(fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == asset.bytes))
    }

    /// The staged package, only if it matches the signed size and hash exactly.
    pub fn ready(&self, installed: &Version, platform: &str) -> Result<Option<Ready>, PackError> {
        let Some((manifest, asset)) = self.offer(installed, platform)? else {
            return Ok(None);
        };
        let path = self.owned(&Self::staged_name(&manifest, platform)?)?;
        let length = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => return Err(PackError::Corrupt("staged update is not a file".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        if length != asset.bytes {
            return Err(PackError::Mismatch("staged update has the wrong size".into()));
        }
        verify_sha256(&path, &asset.sha256)?;
        Ok(Some(Ready { path, version: manifest.version_text().to_owned(), asset }))
    }
}
