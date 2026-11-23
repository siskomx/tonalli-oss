//! Shared immutable, checksum-verified CLI model ownership; Core owns admission.

use crate::manifest::pack_path;
use crate::manifest::Pack;
use std::{fs::File, path::Path, sync::Arc};
use tonalli_core::{ai, pack::verify_sha256};
use tonalli_core::{
    ai::{AiHandle, ModelKind},
    pack::PackError,
};

pub(crate) struct VerifiedModel {
    pub(crate) handle: Arc<AiHandle>,
    pub(crate) pin: serde_json::Value,
    _lease: File,
}

pub(crate) fn validate_pack(pack: &Pack, kind: ModelKind) -> Result<(), PackError> {
    let role = match kind {
        ModelKind::Generation => "generation",
        ModelKind::Embedding => "embedding",
    };
    if pack.kind != "gguf"
        || pack.extra.get("role").and_then(toml::Value::as_str) != Some(role)
        || !Path::new(&pack.path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        let command = match kind {
            ModelKind::Generation => "ai-bench",
            ModelKind::Embedding => "ai-rerank",
        };
        return Err(PackError::Mismatch(format!(
            "{command} requires an installed {role}-role gguf pack"
        )));
    }
    Ok(())
}

impl VerifiedModel {
    pub(crate) fn load(
        base: &Path,
        pack: &Pack,
        kind: ModelKind,
        on_verified: impl FnOnce(&serde_json::Value) -> Result<(), PackError>,
    ) -> Result<Self, PackError> {
        validate_pack(pack, kind)?;
        let path = pack_path(base, &pack.path)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let lease = options.open(&path)?;
        if lease.metadata()?.len() != pack.bytes {
            return Err(PackError::Mismatch(
                "model bytes differ from manifest".into(),
            ));
        }
        verify_sha256(&path, &pack.sha256)?;
        let pin = serde_json::json!({"id":pack.id,"path":path,"bytes":pack.bytes,"sha256":pack.sha256,"version":pack.version,
            "verification":"full SHA256 once under read lease before Core load"});
        // Publish provenance synchronously before native load can fail or allocate.
        on_verified(&pin)?;
        let handle = ai::load_internal(&path, kind)?;
        Ok(Self {
            handle,
            pin,
            _lease: lease,
        })
    }

    pub(crate) fn close(&self) -> Result<(), PackError> {
        self.handle.close()
    }
}

impl Drop for VerifiedModel {
    fn drop(&mut self) {
        let _ = self.handle.close();
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
