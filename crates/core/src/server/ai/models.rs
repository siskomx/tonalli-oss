//! Explicit private model controls. Existing content jobs acquire files; Core owns loading.
use super::{ChatState, ServerState, failure, poisoned};
use crate::{
    ai::{self, AiHandle, ModelKind},
    catalog::{Catalog, CatalogEntry, CatalogKind},
    content::ManagedStore,
    pack::PackError,
};
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Loading {
    id: String,
    cancelled: AtomicBool,
}
impl Loading {
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct LoadLease {
    owner: Arc<ChatState>,
    operation: Arc<Loading>,
    model: Option<Arc<AiHandle>>,
}
impl LoadLease {
    pub(super) fn cancelled(&self) -> bool {
        self.operation.cancelled.load(Ordering::Acquire)
    }
    fn finish(mut self, result: Result<Arc<AiHandle>, PackError>) {
        match result {
            Ok(handle) => {
                self.model = Some(handle.clone());
                if let Ok(mut slot) = self.owner.slot.lock() {
                    if !slot.stopped
                        && !self.cancelled()
                        && slot.model.is_none()
                        && slot.active.is_none()
                        && slot
                            .loading
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &self.operation))
                    {
                        slot.model = Some((self.operation.id.clone(), handle));
                        slot.owned_model = true;
                        self.model.take(); // Slot owns this model; do not close its shared handle.
                    }
                }
            }
            Err(error) => {
                if let Ok(mut slot) = self.owner.slot.lock() {
                    if !slot.stopped && !self.cancelled() {
                        slot.model_error = Some(match error {
                            PackError::NotFound(_) => "model_missing",
                            PackError::Mismatch(_) => "model_refused",
                            _ => "model_failed",
                        });
                    }
                }
            }
        }
    }
}
impl Drop for LoadLease {
    fn drop(&mut self) {
        // Discarded native resources retire before another load can be admitted.
        if let Some(handle) = self.model.take() {
            let _ = handle.close();
            drop(handle);
        }
        if let Ok(mut slot) = self.owner.slot.lock() {
            if slot
                .loading
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.operation))
            {
                slot.loading.take();
            }
        }
    }
}

impl ChatState {
    pub(super) fn begin_load(self: &Arc<Self>, id: String) -> Result<LoadLease, PackError> {
        let mut slot = self.slot.lock().map_err(|_| poisoned())?;
        if slot.stopped {
            return Err(PackError::NotFound("Reader closed".into()));
        }
        if slot.model.is_some() || slot.active.is_some() || slot.loading.is_some() {
            return Err(PackError::Mismatch("AI is busy".into()));
        }
        let operation = Arc::new(Loading {
            id,
            cancelled: AtomicBool::new(false),
        });
        slot.loading = Some(operation.clone());
        slot.model_error = None;
        Ok(LoadLease {
            owner: self.clone(),
            operation,
            model: None,
        })
    }

    pub(in crate::server) fn configure_model_root(&self, root: &Path) -> Result<(), PackError> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(PackError::Mismatch("Invalid native model root".into()));
        }
        let mut slot = self.slot.lock().map_err(|_| poisoned())?;
        if slot.stopped
            || slot
                .legacy_root
                .as_ref()
                .is_some_and(|current| current != &root)
        {
            return Err(PackError::Mismatch(
                "Native model root already configured or closed".into(),
            ));
        }
        slot.legacy_root = Some(root);
        Ok(())
    }
}

fn legacy_path(root: &Path, entry: &CatalogEntry) -> Result<PathBuf, PackError> {
    // The native-only legacy layout is outside the pack downloader's packs/ policy.
    let hash = entry.sha256.to_ascii_lowercase();
    let mut path = root.to_path_buf();
    for (index, child) in ["ai", "models", &hash, "model.gguf"].iter().enumerate() {
        path.push(child);
        let metadata = std::fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked
            || (index < 3 && !metadata.is_dir())
            || (index == 3 && (!metadata.is_file() || metadata.len() != entry.bytes))
        {
            return Err(PackError::Mismatch("Invalid installed model file".into()));
        }
    }
    Ok(path)
}

struct ModelCatalog {
    entries: Vec<CatalogEntry>,
    store: Option<Arc<ManagedStore>>,
    download: bool,
}

fn entries(state: &ServerState) -> Result<ModelCatalog, PackError> {
    let (catalog, store, download) = match super::super::content::model_content(state) {
        Some((store, catalog, download)) => (catalog, Some(store), download),
        None => (Arc::new(Catalog::bundled()?), None, false),
    };
    Ok(ModelCatalog {
        entries: catalog
            .entries()
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Model)
            .cloned()
            .collect(),
        store,
        download,
    })
}

fn status(state: &ServerState) -> Result<Value, PackError> {
    let ModelCatalog {
        entries,
        store,
        download,
    } = entries(state)?;
    let (phase, model, error, root) = {
        let slot = state.ai.slot.lock().map_err(|_| poisoned())?;
        let phase = if let Some(loading) = &slot.loading {
            if loading.cancelled.load(Ordering::Acquire) {
                "stopping"
            } else {
                "loading"
            }
        } else if slot.model.is_some() {
            "ready"
        } else {
            "off"
        };
        (
            phase,
            slot.model
                .as_ref()
                .map(|(id, _)| id.clone())
                .or_else(|| slot.loading.as_ref().map(|loading| loading.id.clone())),
            slot.model_error,
            slot.legacy_root.clone(),
        )
    };
    let installed = store
        .as_ref()
        .map(|store| store.list())
        .transpose()?
        .unwrap_or_default();
    let models = entries.iter().map(|entry| {
        let managed = installed.iter().any(|item| item.id == entry.id && item.kind == CatalogKind::Model);
        let legacy = root.as_ref().is_some_and(|root| legacy_path(root, entry).is_ok());
        json!({"id":entry.id,"title":entry.title,"version":entry.version,"bytes":entry.bytes,"installed":managed || legacy})
    }).collect::<Vec<_>>();
    Ok(
        json!({"state":phase,"model":model,"error":error,"models":models,"downloadAvailable":download}),
    )
}

pub(in crate::server) async fn listing(State(state): State<Arc<ServerState>>) -> Response {
    match tokio::task::spawn_blocking(move || status(&state)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => failure(StatusCode::SERVICE_UNAVAILABLE, "ai_models_unavailable"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct LoadRequest {
    model: String,
}

pub(in crate::server) async fn load(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<LoadRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    };
    if super::super::PackId::new(&request.model).is_err() {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    }
    let ModelCatalog {
        entries: catalog,
        store,
        ..
    } = match entries(&state) {
        Ok(value) => value,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_models_unavailable"),
    };
    let Some(entry) = catalog.into_iter().find(|entry| entry.id == request.model) else {
        return failure(StatusCode::BAD_REQUEST, "ai_unknown_model");
    };
    let root = match state.ai.slot.lock() {
        Ok(slot) => slot.legacy_root.clone(),
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_models_unavailable"),
    };
    let lease = match state.ai.begin_load(entry.id.clone()) {
        Ok(lease) => lease,
        Err(_) => return failure(StatusCode::CONFLICT, "ai_busy"),
    };
    tokio::task::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if lease.cancelled() {
                return Err(PackError::NotFound("Load stopped".into()));
            }
            if let Some(store) = store.as_ref() {
                if store
                    .list()?
                    .iter()
                    .any(|item| item.id == entry.id && item.kind == CatalogKind::Model)
                {
                    return ai::load_managed(store, &entry.id, ModelKind::Generation);
                }
            }
            let root = root.ok_or_else(|| PackError::NotFound("Installed model".into()))?;
            let path = legacy_path(&root, &entry)
                .map_err(|_| PackError::NotFound("Installed model".into()))?;
            ai::require_internal_storage(&path)?;
            crate::pack::verify_sha256(&path, &entry.sha256)?;
            if lease.cancelled() {
                return Err(PackError::NotFound("Load stopped".into()));
            }
            ai::load_internal(&path, ModelKind::Generation)
        }))
        .unwrap_or_else(|_| Err(PackError::Corrupt("Model load failed".into())));
        lease.finish(result);
    });
    let mut response = listing(State(state)).await;
    if response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::ACCEPTED;
    }
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct OffRequest {}
pub(in crate::server) async fn off(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<OffRequest>, JsonRejection>,
) -> Response {
    if body.is_err() {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    }
    let owner = state.ai.clone();
    match tokio::task::spawn_blocking(move || owner.detach_model(false, true)).await {
        Ok(Ok(())) => listing(State(state)).await,
        _ => failure(StatusCode::SERVICE_UNAVAILABLE, "ai_off_failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_native_file_lookup_uses_existing_model_directory_and_exact_length() {
        let root = tempfile::tempdir().unwrap();
        let mut entry = Catalog::bundled()
            .unwrap()
            .entries()
            .iter()
            .find(|entry| entry.kind == CatalogKind::Model)
            .unwrap()
            .clone();
        // Only the file lookup is under test; this is not an inference model.
        entry.bytes = 4;
        let path = root
            .path()
            .join("ai/models")
            .join(&entry.sha256)
            .join("model.gguf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"data").unwrap();
        assert_eq!(legacy_path(root.path(), &entry).unwrap(), path);
        std::fs::write(&path, b"wrong length").unwrap();
        assert!(legacy_path(root.path(), &entry).is_err());
    }

    #[test]
    fn legacy_model_lookup_refuses_linked_directories() {
        let root = tempfile::tempdir().unwrap();
        let alias = tempfile::tempdir().unwrap();
        let mut entry = Catalog::bundled()
            .unwrap()
            .entries()
            .iter()
            .find(|entry| entry.kind == CatalogKind::Model)
            .unwrap()
            .clone();
        entry.bytes = 4;
        let directory = root.path().join("ai/models").join(&entry.sha256);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("model.gguf"), b"data").unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(alias.path().join("ai"))
                .arg(root.path().join("ai"))
                .output()
                .unwrap();
            assert!(result.status.success(), "Owned junction fixture failed");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.path().join("ai"), alias.path().join("ai")).unwrap();
        assert!(legacy_path(alias.path(), &entry).is_err());
        drop(alias);
        assert_eq!(
            std::fs::read(directory.join("model.gguf")).unwrap(),
            b"data"
        );
    }

    #[test]
    #[ignore = "requires the already verified optional Qwen model; no network fallback"]
    fn real_model_controls_do_not_publish_a_load_completed_after_off_or_shutdown() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spike/packs/ai/Qwen3-1.7B-Q4_K_M.gguf")
            .canonicalize()
            .unwrap();
        let sha = "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5";
        crate::pack::verify_sha256(&path, sha).unwrap();
        for shutdown in [false, true] {
            let owner = Arc::new(ChatState::default());
            let pending = owner.begin_load("model".into()).unwrap();
            let model = ai::load_internal(&path, ModelKind::Generation).unwrap();
            assert!(model.info().is_ok());
            owner.detach(shutdown).unwrap();
            pending.finish(Ok(model.clone()));
            assert!(!owner.available());
            assert!(model.info().is_err());
            assert!(owner.slot.lock().unwrap().loading.is_none());
            assert_eq!(owner.begin_load("next".into()).is_err(), shutdown);
        }
        crate::pack::verify_sha256(&path, sha).unwrap();
    }

    #[test]
    fn native_model_directory_is_explicit_and_cannot_be_retargeted_or_reopened() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let owner = ChatState::default();
        owner.configure_model_root(root.path()).unwrap();
        owner.configure_model_root(root.path()).unwrap();
        assert!(owner.configure_model_root(other.path()).is_err());
        owner.detach(true).unwrap();
        assert!(owner.configure_model_root(root.path()).is_err());
    }
}
