//! One explicitly requested local copy; terminal status follows actual worker completion.
#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
use super::super::{PackRegistry, ServerState};
use crate::{
    download::{DownloadAction, DownloadPhase},
    pack::PackError,
    remote::{CopyError, CopyOutcome, RemoteError},
    sync::{PackManifest, SyncStore},
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

/// Native notification/service admission; no reader or private-server handle crosses it.
pub trait CopyDispatcher: Send + Sync {
    fn dispatch(&self, epoch: u64, ipv6: bool) -> Result<std::net::IpAddr, PackError>;
}

/// Independent, native-owned copy status. Credentials exist only in accepted copy tasks.
pub struct CopyOwner {
    pub(in super::super) root: PathBuf,
    pub(in super::super) copies: Arc<Copies>,
}
#[derive(Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CopyRunState {
    pub epoch: u64,
    pub running: bool,
}
impl CopyOwner {
    pub fn open(root: &Path, dispatcher: Arc<dyn CopyDispatcher>) -> Result<Self, PackError> {
        let store = SyncStore::open(root)?;
        Ok(Self {
            root: store.root().to_owned(),
            copies: Arc::new(Copies {
                dispatcher: Some(dispatcher),
                ..Copies::default()
            }),
        })
    }
    pub fn is_running(&self) -> Result<bool, PackError> {
        self.copies.busy()
    }
    pub fn status(&self) -> Result<CopyRunState, PackError> {
        let slot = self
            .copies
            .slot
            .lock()
            .map_err(|_| super::super::registry_poisoned())?;
        Ok(CopyRunState {
            epoch: slot.epoch,
            running: slot.running,
        })
    }
    pub fn pause(&self, epoch: u64) -> Result<(), PackError> {
        let mut slot = self
            .copies
            .slot
            .lock()
            .map_err(|_| super::super::registry_poisoned())?;
        if epoch != slot.epoch || epoch == 0 {
            return Err(PackError::Mismatch("Stale nearby copy attempt".into()));
        }
        if slot.running {
            slot.action = DownloadAction::Pause;
            slot.snapshot.state = Phase::Pausing;
        }
        Ok(())
    }
    pub fn close(&self) {
        self.copies.stop(true);
    }
}
impl Drop for CopyOwner {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    #[default]
    Idle,
    Preparing,
    Downloading,
    Verifying,
    Pausing,
    Cancelling,
    Paused,
    Cancelled,
    Complete,
    Failed,
}
#[derive(Clone, Default, Serialize)]
pub(in super::super) struct Snapshot {
    state: Phase,
    id: Option<String>,
    title: Option<String>,
    kind: Option<crate::catalog::CatalogKind>,
    local_id: Option<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
    verified_bytes: u64,
    error: Option<&'static str>,
    /// Present only when article search was requested: requested, copying, copied, unavailable, cancelled or failed.
    search: Option<&'static str>,
}
struct Slot {
    epoch: u64,
    running: bool,
    closed: bool,
    action: DownloadAction,
    snapshot: Snapshot,
    staged: Option<PackManifest>,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            epoch: 0,
            running: false,
            closed: false,
            action: DownloadAction::Continue,
            snapshot: Snapshot::default(),
            staged: None,
        }
    }
}
#[derive(Default)]
pub(in super::super) struct Copies {
    slot: Mutex<Slot>,
    dispatcher: Option<Arc<dyn CopyDispatcher>>,
    registry: Mutex<Weak<PackRegistry>>,
}
impl Copies {
    pub(super) fn is_native_owned(&self) -> bool {
        self.dispatcher.is_some()
    }
    fn failed(&self) {
        if let Ok(mut slot) = self.slot.lock() {
            slot.running = false;
            slot.snapshot.state = Phase::Failed;
            slot.snapshot.error = Some("copy_failed");
        }
    }
    fn spawn(
        self: &Arc<Self>,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), StatusCode> {
        let owner = Arc::clone(self);
        std::thread::Builder::new()
            .name("tonalli-nearby-copy".into())
            .spawn(move || {
                let completed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .max_blocking_threads(2)
                        .build()?;
                    runtime.block_on(work);
                    Ok::<_, std::io::Error>(())
                }));
                if !matches!(completed, Ok(Ok(()))) {
                    owner.failed();
                }
            })
            .map(|_| ())
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    pub(in super::super) fn bind(&self, registry: &Arc<PackRegistry>) -> Result<(), PackError> {
        *self
            .registry
            .lock()
            .map_err(|_| super::super::registry_poisoned())? = Arc::downgrade(registry);
        Ok(())
    }
    pub(super) fn busy(&self) -> Result<bool, PackError> {
        self.slot
            .lock()
            .map(|slot| slot.running)
            .map_err(|_| super::super::registry_poisoned())
    }
    pub(super) fn is_closed(&self) -> Result<bool, PackError> {
        self.slot
            .lock()
            .map(|slot| slot.closed)
            .map_err(|_| super::super::registry_poisoned())
    }
    pub(in super::super) fn snapshot(&self) -> Result<Snapshot, StatusCode> {
        self.slot
            .lock()
            .map(|s| s.snapshot.clone())
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    pub(in super::super) fn stop(&self, closed: bool) {
        if let Ok(mut slot) = self.slot.lock() {
            slot.closed |= closed;
            slot.action = DownloadAction::Cancel;
            if slot.running {
                slot.snapshot.state = Phase::Cancelling;
            }
        }
    }
    pub(in super::super) fn restore(&self, registry: &PackRegistry, root: &Path) {
        if !root.join("packs/nearby/installed").exists() {
            return;
        }
        let result = (|| {
            let store = SyncStore::open(root)?;
            for row in store.list()? {
                registry.register_copy(&store, &row)?;
            }
            Ok::<_, PackError>(())
        })();
        if result.is_err() {
            if let Ok(mut slot) = self.slot.lock() {
                if !slot.running {
                    slot.snapshot.state = Phase::Failed;
                    slot.snapshot.error = Some("copies_unavailable");
                }
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Start {
    id: String,
    #[serde(default)]
    search: bool,
}

fn observe(
    copies: Arc<Copies>,
) -> impl FnMut(crate::download::DownloadProgress) -> Result<(), PackError> + Send + 'static {
    move |value| {
        let mut slot = copies
            .slot
            .lock()
            .map_err(|_| PackError::Corrupt("Copy state unavailable".into()))?;
        slot.snapshot.downloaded_bytes = value.downloaded_bytes;
        slot.snapshot.total_bytes = value.total_bytes;
        slot.snapshot.verified_bytes = value.verified_bytes;
        if slot.action == DownloadAction::Continue {
            slot.snapshot.state = match value.phase {
                DownloadPhase::Downloading => Phase::Downloading,
                DownloadPhase::Verifying => Phase::Verifying,
            };
        }
        Ok(())
    }
}
pub(in super::super) async fn start(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Start>,
) -> Response {
    if super::super::PackId::new(&input.id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let client = match state.remote.client() {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::CONFLICT.into_response(),
        Err(s) => return s.into_response(),
    };
    let copies = match state.remote.copies() {
        Ok(copies) => copies,
        Err(status) => return status.into_response(),
    };
    let epoch = {
        let mut slot = match copies.slot.lock() {
            Ok(s) => s,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        if slot.closed
            || slot.running
            || (slot.staged.is_some() && slot.snapshot.id.as_deref() != Some(input.id.as_str()))
        {
            return StatusCode::CONFLICT.into_response();
        }
        let Some(epoch) = slot.epoch.checked_add(1) else {
            return StatusCode::CONFLICT.into_response();
        };
        slot.epoch = epoch;
        slot.running = true;
        slot.action = DownloadAction::Continue;
        slot.snapshot = Snapshot {
            state: Phase::Preparing,
            id: Some(input.id.clone()),
            search: input.search.then_some("requested"),
            ..Snapshot::default()
        };
        epoch
    };
    let root = state.data_dir.clone();
    let source = if let Some(dispatcher) = &copies.dispatcher {
        match dispatcher.dispatch(epoch, client.uses_ipv6()) {
            Ok(source) => Some(source),
            Err(_) => {
                copies.failed();
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        }
    } else {
        None
    };
    let search_client = if input.search {
        match client.copy_session_on(&input.id, source) {
            Ok(session) => Some(session),
            Err(_) => {
                copies.failed();
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        }
    } else {
        None
    };
    let client = match client.copy_session_on(&input.id, source) {
        Ok(client) => client,
        Err(_) => {
            copies.failed();
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let task = Arc::clone(&copies);
    let spawned = task.spawn(async move {
        let action = Arc::clone(&copies);
        let progress = Arc::clone(&copies);
        let prepared = Arc::clone(&copies);
        let result = client
            .copy_pack_observed(
                root.clone(),
                move || {
                    action
                        .slot
                        .lock()
                        .map(|s| s.action)
                        .unwrap_or(DownloadAction::Cancel)
                },
                observe(progress),
                move |lease, manifest| {
                    let previous = prepared
                        .slot
                        .lock()
                        .map_err(|_| PackError::Corrupt("Copy state unavailable".into()))?
                        .staged
                        .clone();
                    if let Some(previous) = previous {
                        if previous.sha256 != manifest.sha256 {
                            lease.discard(&previous)?;
                        }
                    }
                    let mut slot = prepared
                        .slot
                        .lock()
                        .map_err(|_| PackError::Corrupt("Copy state unavailable".into()))?;
                    slot.snapshot.kind = Some(manifest.kind);
                    slot.snapshot.title = Some(manifest.title.clone());
                    slot.staged = Some(manifest.clone());
                    Ok(())
                },
            )
            .await;
        let result = match result {
            Ok(CopyOutcome::Complete(row)) => {
                let publish_root = root.clone();
                let registry = copies
                    .registry
                    .lock()
                    .ok()
                    .and_then(|registry| registry.upgrade());
                let publish = tokio::task::spawn_blocking(move || {
                    let store = SyncStore::open(&publish_root)?;
                    if let Some(registry) = registry {
                        registry.register_copy(&store, &row)?;
                    }
                    Ok::<_, PackError>(row)
                })
                .await;
                match publish {
                    Ok(Ok(row)) => Ok(CopyOutcome::Complete(row)),
                    Ok(Err(e)) => Err(CopyError::Pack(e)),
                    Err(_) => Err(CopyError::Pack(PackError::Corrupt(
                        "Copy publication failed".into(),
                    ))),
                }
            }
            other => other,
        };
        let result = if result.is_err()
            && copies
                .slot
                .lock()
                .is_ok_and(|s| s.action == DownloadAction::Cancel)
        {
            match cleanup(&copies, &root).await {
                Ok(()) => Ok(CopyOutcome::Cancelled),
                Err(error) => Err(CopyError::Pack(error)),
            }
        } else {
            result
        };
        // The saved book is already published; its article search is an optional second phase
        // whose failure never undoes or fails the book.
        let mut search_note = None;
        let mut result = result;
        if let (Ok(CopyOutcome::Complete(_)), Some(session)) = (&result, search_client) {
            if let Ok(mut slot) = copies.slot.lock() {
                slot.staged = None;
                slot.snapshot.state = Phase::Downloading;
                slot.snapshot.search = Some("copying");
                slot.snapshot.downloaded_bytes = 0;
                slot.snapshot.total_bytes = 0;
                slot.snapshot.verified_bytes = 0;
            }
            let action = Arc::clone(&copies);
            let outcome = session
                .copy_search_observed(
                    root.clone(),
                    move || {
                        action
                            .slot
                            .lock()
                            .map(|s| s.action)
                            .unwrap_or(DownloadAction::Cancel)
                    },
                    observe(Arc::clone(&copies)),
                    |_, _| Ok(()),
                )
                .await;
            search_note = Some(match outcome {
                Ok(CopyOutcome::Complete(_)) => "copied",
                Ok(CopyOutcome::Paused) => {
                    result = Ok(CopyOutcome::Paused);
                    "copying"
                }
                Ok(CopyOutcome::Cancelled) => "cancelled",
                Err(CopyError::Remote(RemoteError::Rejected(404)))
                | Err(CopyError::Pack(PackError::NotFound(_))) => "unavailable",
                Err(_) => "failed",
            });
        }
        if let Ok(mut slot) = copies.slot.lock() {
            slot.running = false;
            if search_note.is_some() {
                slot.snapshot.search = search_note;
            }
            match result {
                Ok(CopyOutcome::Complete(row)) => {
                    slot.snapshot.state = Phase::Complete;
                    slot.snapshot.title = Some(row.title.clone());
                    slot.snapshot.local_id = Some(row.local_id());
                    slot.staged = None;
                }
                Ok(CopyOutcome::Paused) => slot.snapshot.state = Phase::Paused,
                Ok(CopyOutcome::Cancelled) => {
                    slot.snapshot.state = Phase::Cancelled;
                    slot.staged = None;
                }
                Err(error) => {
                    slot.snapshot.state = Phase::Failed;
                    slot.snapshot.error =
                        Some(if matches!(error, CopyError::Pack(PackError::NoSpace)) {
                            "no_space"
                        } else {
                            "copy_failed"
                        });
                }
            }
        }
    });
    if let Err(status) = spawned {
        task.failed();
        return status.into_response();
    }
    StatusCode::ACCEPTED.into_response()
}
async fn cleanup(copies: &Copies, root: &Path) -> Result<(), PackError> {
    let staged = copies
        .slot
        .lock()
        .map_err(|_| PackError::Corrupt("Copy state unavailable".into()))?
        .staged
        .clone();
    if let Some(manifest) = staged {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let store = SyncStore::open(&root)?;
            let lease = store.copy_lease()?;
            lease.discard(&manifest)
        })
        .await
        .map_err(|_| PackError::Corrupt("Copy cleanup failed".into()))??;
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Pause,
    Cancel,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Control {
    action: Action,
}
pub(in super::super) async fn control(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Control>,
) -> Response {
    let copies = match state.remote.copies() {
        Ok(copies) => copies,
        Err(status) => return status.into_response(),
    };
    let mut slot = match copies.slot.lock() {
        Ok(s) => s,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if slot.running {
        match input.action {
            Action::Pause => {
                slot.action = DownloadAction::Pause;
                slot.snapshot.state = Phase::Pausing;
            }
            Action::Cancel => {
                slot.action = DownloadAction::Cancel;
                slot.snapshot.state = Phase::Cancelling;
            }
        }
    } else if matches!(input.action, Action::Cancel)
        && matches!(slot.snapshot.state, Phase::Paused | Phase::Failed)
    {
        slot.action = DownloadAction::Cancel;
        slot.snapshot.state = Phase::Cancelling;
        slot.running = true;
        let copies = Arc::clone(&copies);
        let root = state.data_dir.clone();
        let task = Arc::clone(&copies);
        let spawned = task.spawn(async move {
            let result = cleanup(&copies, &root).await;
            if let Ok(mut slot) = copies.slot.lock() {
                slot.running = false;
                match result {
                    Ok(()) => {
                        slot.staged = None;
                        slot.snapshot.state = Phase::Cancelled;
                    }
                    Err(_) => {
                        slot.snapshot.state = Phase::Failed;
                        slot.snapshot.error = Some("copy_failed");
                    }
                }
            }
        });
        if spawned.is_err() {
            slot.running = false;
            slot.snapshot.state = Phase::Failed;
            slot.snapshot.error = Some("copy_failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    Json(slot.snapshot.clone()).into_response()
}
