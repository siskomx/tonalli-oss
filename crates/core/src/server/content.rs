//! Private catalog presentation. Native code supplies signed authority and storage.
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::ServerState;
use crate::{
    catalog::{Catalog, CatalogKind},
    content::{
        ContentJobs, InstalledContent, JobAction, JobError, JobReply, JobTicket, ManagedStore,
        RemoveOutcome,
    },
    pack::PackError,
};

/// Native scheduling acceptance. Rejected guarantees no matching pending/running task;
/// Uncertain preserves admission and must not cause an automatic duplicate dispatch.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum ContentDispatchError {
    #[error("Native content dispatch rejected")]
    Rejected,
    #[error("Native content dispatch acceptance is uncertain")]
    Uncertain,
}

pub trait ContentDispatcher: Send + Sync {
    /// Schedule this exact ticket promptly on the allowed native worker/network.
    /// Retain jobs for claim/execute/finish; never infer execution from the HTTP response.
    fn dispatch(
        &self,
        jobs: Arc<ContentJobs>,
        ticket: JobTicket,
    ) -> Result<(), ContentDispatchError>;
    /// Wake/cancel an already admitted native task after durable Pause/Cancel.
    /// A queued offline OS job may have no service; ordinary active monitors can keep the no-op.
    fn control(
        &self,
        _jobs: Arc<ContentJobs>,
        _ticket: JobTicket,
        _action: JobAction,
    ) -> Result<(), ContentDispatchError> {
        Ok(())
    }
}

struct Worker {
    jobs: Arc<ContentJobs>,
    dispatcher: Arc<dyn ContentDispatcher>,
}

pub(super) struct Manager {
    store: Arc<ManagedStore>,
    catalog: Arc<Catalog>,
    worker: Option<Worker>,
    registry: Arc<super::PackRegistry>,
}

impl Manager {
    pub(super) fn new(
        store: Arc<ManagedStore>,
        catalog: Arc<Catalog>,
        registry: Arc<super::PackRegistry>,
    ) -> Result<Self, PackError> {
        store.accept_catalog(&catalog)?;
        registry.sync_managed(&store)?;
        Ok(Self {
            store,
            catalog,
            worker: None,
            registry,
        })
    }

    pub(super) fn with_jobs(
        store: Arc<ManagedStore>,
        catalog: Arc<Catalog>,
        dispatcher: Arc<dyn ContentDispatcher>,
        registry: Arc<super::PackRegistry>,
    ) -> Result<Self, PackError> {
        let jobs = Arc::new(
            ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).map_err(
                |error| match error {
                    JobError::Pack(error) => error,
                    _ => PackError::Corrupt("Content jobs unavailable".into()),
                },
            )?,
        );
        registry.sync_managed(&store)?;
        Ok(Self {
            store,
            catalog,
            worker: Some(Worker { jobs, dispatcher }),
            registry,
        })
    }

    fn catalog(&self) -> Result<Value, PackError> {
        self.registry.sync_managed(&self.store)?;
        let installed = self
            .store
            .list()?
            .into_iter()
            .map(|entry| {
                let readable = self.store.open_source(&entry.id).is_ok();
                (entry, readable)
            })
            .collect::<Vec<_>>();
        let entries = self
            .catalog
            .entries()
            .iter()
            .map(|entry| {
                let current = installed
                    .iter()
                    .find(|(installed, _)| installed.id == entry.id);
                json!({
                    "id":entry.id, "title":entry.title, "kind":kind(entry.kind),
                    "language":entry.language, "version":entry.version, "bytes":entry.bytes,
                    "installedVersion":current.map(|(installed, _)| installed.version.as_str()),
                    "installedReadable":current.map(|(_, readable)| *readable),
                })
            })
            .collect::<Vec<_>>();
        let installed = installed
            .iter()
            .map(|(entry, readable)| installed_row(entry, *readable))
            .collect::<Vec<_>>();
        Ok(json!({
            "revision":self.catalog.revision(), "entries":entries, "installed":installed,
            "downloadAvailable":self.worker.is_some(),
        }))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EntryRequest {
    entry_id: String,
    catalog_revision: u64,
}

fn manager(state: &ServerState) -> Option<Arc<Manager>> {
    state
        .content
        .lock()
        .ok()
        .and_then(|manager| manager.clone())
}

#[cfg(feature = "ai")]
pub(super) fn model_content(
    state: &ServerState,
) -> Option<(Arc<ManagedStore>, Arc<Catalog>, bool)> {
    manager(state).map(|manager| {
        (
            manager.store.clone(),
            manager.catalog.clone(),
            manager.worker.is_some(),
        )
    })
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}

fn job_error(error_value: JobError) -> Response {
    match error_value {
        JobError::Busy => error(StatusCode::CONFLICT, "busy"),
        JobError::CatalogChanged => error(StatusCode::CONFLICT, "catalog_changed"),
        JobError::NotFound => error(StatusCode::NOT_FOUND, "not_found"),
        JobError::StaleAttempt => error(StatusCode::CONFLICT, "busy"),
        JobError::InvalidRequest => error(StatusCode::BAD_REQUEST, "invalid_request"),
        JobError::Pack(error_value) => pack_error(error_value),
    }
}

fn pack_error(error_value: PackError) -> Response {
    match error_value {
        PackError::NoSpace => error(StatusCode::INSUFFICIENT_STORAGE, "no_space"),
        PackError::NotFound(_) => error(StatusCode::NOT_FOUND, "not_found"),
        PackError::PublicationUncertain(_) => {
            error(StatusCode::SERVICE_UNAVAILABLE, "publication_uncertain")
        }
        _ => error(StatusCode::SERVICE_UNAVAILABLE, "content_unavailable"),
    }
}

fn worker(manager: &Manager) -> Option<&Worker> {
    manager.worker.as_ref()
}

fn download_unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "download_unavailable")
}

fn dispatch(worker: &Worker, reply: JobReply) -> Response {
    if reply.dispatch {
        // No attachment/Jobs/Store lock is held across native code, including a native exception.
        let result = catch_unwind(AssertUnwindSafe(|| {
            worker
                .dispatcher
                .dispatch(Arc::clone(&worker.jobs), reply.ticket.clone())
        }))
        .unwrap_or(Err(ContentDispatchError::Uncertain));
        match result {
            Ok(()) => {}
            Err(ContentDispatchError::Rejected) => {
                return match worker.jobs.dispatch_rejected(&reply.ticket) {
                    Ok(_) => error(StatusCode::SERVICE_UNAVAILABLE, "download_unavailable"),
                    Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "request_unconfirmed"),
                };
            }
            Err(ContentDispatchError::Uncertain) => {
                return error(StatusCode::SERVICE_UNAVAILABLE, "request_unconfirmed")
            }
        }
    }
    Json(json!({"job":reply.job})).into_response()
}

pub(super) async fn jobs(State(state): State<Arc<ServerState>>) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return unavailable(),
    };
    match tokio::task::spawn_blocking(move || {
        let worker = match worker(&manager) {
            Some(value) => value,
            None => return download_unavailable(),
        };
        match worker.jobs.list() {
            Ok(jobs) => match manager.registry.sync_managed(&manager.store) {
                Ok(()) => Json(json!({"jobs":jobs})).into_response(),
                Err(error) => pack_error(error),
            },
            Err(value) => job_error(value),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => unavailable(),
    }
}

pub(super) async fn preflight(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<EntryRequest>, JsonRejection>,
) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return unavailable(),
    };
    if worker(&manager).is_none() {
        return download_unavailable();
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    match tokio::task::spawn_blocking(move || {
        if body.catalog_revision != manager.catalog.revision() {
            return error(StatusCode::CONFLICT, "catalog_changed");
        }
        match manager.store.preflight(&manager.catalog, &body.entry_id) {
            Ok(budget) => Json(budget).into_response(),
            Err(value) => pack_error(value),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => unavailable(),
    }
}

pub(super) async fn start(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<EntryRequest>, JsonRejection>,
) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return unavailable(),
    };
    if worker(&manager).is_none() {
        return download_unavailable();
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    match tokio::task::spawn_blocking(move || {
        let worker = match worker(&manager) {
            Some(value) => value,
            None => return download_unavailable(),
        };
        if body.catalog_revision != manager.catalog.revision() {
            return error(StatusCode::CONFLICT, "catalog_changed");
        }
        let budget = match manager.store.preflight(&manager.catalog, &body.entry_id) {
            Ok(value) => value,
            Err(value) => return pack_error(value),
        };
        if !budget.allowed {
            return error(
                StatusCode::CONFLICT,
                if budget.reason.as_deref() == Some("no_space") {
                    "no_space"
                } else {
                    "incomplete_bundle"
                },
            );
        }
        match worker.jobs.start(&body.entry_id, body.catalog_revision) {
            Ok(reply) => dispatch(worker, reply),
            Err(value) => job_error(value),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => unavailable(),
    }
}

pub(super) async fn control(
    State(state): State<Arc<ServerState>>,
    Path((id, action)): Path<(String, String)>,
) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return unavailable(),
    };
    if worker(&manager).is_none() {
        return download_unavailable();
    }
    let action = match action.as_str() {
        "pause" => JobAction::Pause,
        "resume" => JobAction::Resume,
        "cancel" => JobAction::Cancel,
        "retry" => JobAction::Retry,
        _ => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    match tokio::task::spawn_blocking(move || {
        let worker = match worker(&manager) {
            Some(value) => value,
            None => return download_unavailable(),
        };
        match worker.jobs.request(&id, action) {
            Ok(reply) => {
                if !reply.dispatch && matches!(action, JobAction::Pause | JobAction::Cancel) {
                    let notification = catch_unwind(AssertUnwindSafe(|| {
                        worker.dispatcher.control(
                            Arc::clone(&worker.jobs),
                            reply.ticket.clone(),
                            action,
                        )
                    }));
                    if !matches!(notification, Ok(Ok(()))) {
                        return error(StatusCode::SERVICE_UNAVAILABLE, "request_unconfirmed");
                    }
                }
                dispatch(worker, reply)
            }
            Err(value) => job_error(value),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => unavailable(),
    }
}

pub(super) async fn remove(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return unavailable(),
    };
    if worker(&manager).is_none() {
        return download_unavailable();
    }
    match tokio::task::spawn_blocking(move || {
        match manager
            .worker
            .as_ref()
            .ok_or(JobError::InvalidRequest)
            .and_then(|worker| {
                super::PackId::new(id)
                    .map_err(JobError::from)
                    .and_then(|id| manager.registry.remove_managed(&worker.jobs, &id))
            }) {
            Ok(RemoveOutcome::Removed) => {
                Json(json!({"removed":true,"reason":null})).into_response()
            }
            Ok(RemoveOutcome::NotInstalled) => {
                Json(json!({"removed":false,"reason":null})).into_response()
            }
            Ok(RemoveOutcome::InUse) => {
                Json(json!({"removed":false,"reason":"in_use"})).into_response()
            }
            Err(value) => job_error(value),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => unavailable(),
    }
}

fn kind(kind: CatalogKind) -> &'static str {
    match kind {
        CatalogKind::Zim => "zim",
        CatalogKind::Map => "map",
        CatalogKind::Model => "model",
    }
}

fn installed_row(entry: &InstalledContent, readable: bool) -> Value {
    json!({
        "id":entry.id, "title":entry.title, "kind":kind(entry.kind),
        "language":entry.language, "version":entry.version, "bytes":entry.bytes,
        "readable":readable,
    })
}

pub(super) fn available(state: &ServerState) -> bool {
    state.content.lock().is_ok_and(|manager| manager.is_some())
}

pub(super) async fn catalog(State(state): State<Arc<ServerState>>) -> Response {
    let manager = match state.content.lock() {
        Ok(manager) => manager.clone(),
        Err(_) => return unavailable(),
    };
    let Some(manager) = manager else {
        return unavailable();
    };
    match tokio::task::spawn_blocking(move || manager.catalog()).await {
        Ok(Ok(catalog)) => Json(catalog).into_response(),
        _ => unavailable(),
    }
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"content_unavailable"})),
    )
        .into_response()
}
