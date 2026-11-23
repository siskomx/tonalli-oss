use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
pub use tonalli_core::download::pack_path;
pub(crate) use tonalli_core::download::{ensure_regular_or_absent, write_atomic};
use tonalli_core::{download::DownloadSpec, pack::PackError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub id: String,
    pub kind: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub bytes: u64,
    pub sha256: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deferred: bool,
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_version: u32,
    #[serde(default)]
    pub packs: Vec<Pack>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, PackError> {
        let manifest: Self = toml::from_str(&fs::read_to_string(path)?)
            .map_err(|error| PackError::Corrupt(format!("manifest: {error}")))?;
        if manifest.manifest_version != 1 {
            return Err(PackError::Mismatch("unsupported manifest version".into()));
        }
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for pack in &manifest.packs {
            validate_pack(pack)?;
            pack_path(manifest_dir(path), &pack.path)?;
            if !ids.insert(&pack.id) || !paths.insert(pack.path.to_lowercase()) {
                return Err(PackError::Mismatch(
                    "duplicate pack id or destination".into(),
                ));
            }
        }
        Ok(manifest)
    }

    pub fn save(&self, path: &Path) -> Result<(), PackError> {
        let text = toml::to_string_pretty(self)
            .map_err(|error| PackError::Corrupt(format!("manifest: {error}")))?;
        write_atomic(path, text.as_bytes())
    }
}

pub(crate) fn manifest_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

impl From<&Pack> for DownloadSpec {
    fn from(pack: &Pack) -> Self {
        Self {
            id: pack.id.clone(),
            url: pack.url.clone(),
            bytes: pack.bytes,
            sha256: pack.sha256.clone(),
            path: pack.path.clone(),
        }
    }
}

pub(crate) fn validate_pack(pack: &Pack) -> Result<(), PackError> {
    DownloadSpec::from(pack).validate()
}
