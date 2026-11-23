//! Private adapter for the existing NOMAD-derived search-preparation UI contract.
use super::{ContentDispatchError, PackId, PackRegistry, ServerState};
use crate::{
    index::jobs::{
        Action, DispatchFailure, IndexJobError, IndexJobs, Snapshot, State as JobState, Ticket,
    },
    pack::PackError,
};
use axum::{
    extract::{rejection::JsonRejection, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub trait PreparationDispatcher: Send + Sync {
    fn dispatch(&self, jobs: Arc<IndexJobs>, ticket: Ticket) -> Result<(), ContentDispatchError>;
    fn control(
        &self,
        jobs: Arc<IndexJobs>,
        ticket: Ticket,
        action: Action,
    ) -> Result<(), ContentDispatchError>;
}

pub(super) struct Manager {
    jobs: Arc<IndexJobs>,
    dispatcher: Arc<dyn PreparationDispatcher>,
    registry: Arc<PackRegistry>,
}
impl Manager {
    pub fn new(
        jobs: Arc<IndexJobs>,
        dispatcher: Arc<dyn PreparationDispatcher>,
        registry: Arc<PackRegistry>,
    ) -> Self {
        Self {
            jobs,
            dispatcher,
            registry,
        }
    }
    pub fn restore(&self) -> Result<(), PackError> {
        let ids = self
            .registry
            .zims
            .read()
            .map_err(|_| super::registry_poisoned())?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            if self
                .registry
                .indexes
                .read()
                .map_err(|_| super::registry_poisoned())?
                .contains_key(&id)
            {
                continue;
            }
            let source = self.registry.preparation_source(&id).map_err(as_pack)?;
            if let Some(index) = self.jobs.safe_ready(&source).map_err(as_pack)? {
                self.registry.attach_index(id, Arc::new(index))?;
            }
        }
        Ok(())
    }

    fn status(&self, id: &PackId) -> Result<Value, IndexJobError> {
        let row = self.jobs.status(id)?;
        let source = self.registry.preparation_source(id);
        match (row, source) {
            (Some(row), Ok(source)) => {
                if !self.jobs.source_matches(&row.ticket, &source)? {
                    if matches!(row.state, JobState::Ready | JobState::Cancelled) {
                        return Ok(view(
                            id,
                            None,
                            Some(if self.attach_ready(id, &source)? {
                                "ready"
                            } else {
                                "unprepared"
                            }),
                            None,
                        ));
                    }
                    return Ok(view(
                        id,
                        Some(&row),
                        Some("stale_source"),
                        Some("stale_source"),
                    ));
                }
                if row.state == JobState::Ready && !self.attach_ready(id, &source)? {
                    return Err(PackError::NotFound("Prepared search generation".into()).into());
                }
                Ok(view(id, Some(&row), None, None))
            }
            (Some(row), Err(error))
                if !matches!(row.state, JobState::Ready | JobState::Cancelled) =>
            {
                Ok(view(
                    id,
                    Some(&row),
                    Some("stale_source"),
                    Some(reason(&error)),
                ))
            }
            (None, Ok(source)) => {
                let ready = self.attach_ready(id, &source)?;
                Ok(view(
                    id,
                    None,
                    Some(if ready { "ready" } else { "unprepared" }),
                    None,
                ))
            }
            (_, Err(error)) => Err(error),
        }
    }

    fn attach_ready(
        &self,
        id: &PackId,
        source: &crate::index::jobs::IndexSource,
    ) -> Result<bool, IndexJobError> {
        if self
            .registry
            .indexes
            .read()
            .map_err(|_| super::registry_poisoned())?
            .contains_key(id)
        {
            return Ok(true);
        }
        if let Some(index) = self.jobs.safe_ready(source)? {
            self.registry.attach_index(id.clone(), Arc::new(index))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn send(
        &self,
        snapshot: &Snapshot,
        action: Option<Action>,
    ) -> Result<(), ContentDispatchError> {
        if snapshot.dispatch {
            self.dispatcher
                .dispatch(Arc::clone(&self.jobs), snapshot.ticket.clone())
        } else if let Some(action) = action {
            self.dispatcher
                .control(Arc::clone(&self.jobs), snapshot.ticket.clone(), action)
        } else {
            Ok(())
        }
    }
}

fn view(id: &PackId, row: Option<&Snapshot>, state: Option<&str>, reason: Option<&str>) -> Value {
    let progress = row.and_then(|row| row.progress);
    let default = row.map_or("unprepared", |row| match row.state {
        JobState::Queued => "queued",
        JobState::Checking | JobState::Indexing | JobState::Validating | JobState::Activating => {
            "preparing"
        }
        JobState::Pausing => "pausing",
        JobState::Paused => "paused",
        JobState::Cancelling => "cancelling",
        JobState::Cancelled => "cancelled",
        JobState::Ready => "ready",
        JobState::Failed => "failed",
    });
    json!({"pack":id.as_str(),"jobId":row.map(|row| &row.job_id),"epoch":row.map(|row| row.ticket.epoch.to_string()),"state":state.unwrap_or(default),"phase":progress.map(|p|p.phase),"processedArticles":progress.map(|p|p.processed_articles),"committedArticles":progress.map(|p|p.committed_articles),"reason":reason.or_else(|| row.and_then(|row|row.error_code.as_deref()))})
}
fn manager(state: &ServerState) -> Option<Arc<Manager>> {
    state.preparation.lock().ok().and_then(|slot| slot.clone())
}
fn problem(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}
fn bad_body(error: JsonRejection) -> Response {
    let status = if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    problem(status, "bad_request")
}
fn as_pack(error: IndexJobError) -> PackError {
    match error {
        IndexJobError::Pack(error) => error,
        _ => PackError::Mismatch("Preparation source unavailable".into()),
    }
}
fn reason(error: &IndexJobError) -> &'static str {
    match error {
        IndexJobError::Busy => "busy",
        IndexJobError::StaleTicket | IndexJobError::SourceChanged => "stale_source",
        IndexJobError::Pack(PackError::MediaRemoved) => "media_removed",
        IndexJobError::Pack(PackError::NoSpace) => "no_space",
        IndexJobError::Pack(PackError::PublicationUncertain(_)) => "publication_uncertain",
        IndexJobError::NotFound | IndexJobError::Pack(PackError::NotFound(_)) => {
            "source_unavailable"
        }
        _ => "failed",
    }
}
fn failure(error: IndexJobError) -> Response {
    let status = match error {
        IndexJobError::Busy | IndexJobError::StaleTicket | IndexJobError::SourceChanged => {
            StatusCode::CONFLICT
        }
        IndexJobError::InvalidRequest => StatusCode::BAD_REQUEST,
        IndexJobError::NotFound | IndexJobError::Pack(PackError::NotFound(_)) => {
            StatusCode::NOT_FOUND
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    problem(status, reason(&error))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Book {
    pack: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Control {
    job_id: String,
    epoch: String,
    action: Action,
}

pub(super) async fn status(
    State(state): State<Arc<ServerState>>,
    Query(book): Query<Book>,
) -> Response {
    let manager = match manager(&state) {
        Some(value) => value,
        None => return problem(StatusCode::SERVICE_UNAVAILABLE, "preparation_unavailable"),
    };
    let id = match PackId::new(book.pack) {
        Ok(id) => id,
        Err(_) => return problem(StatusCode::BAD_REQUEST, "bad_request"),
    };
    match tokio::task::spawn_blocking(move || manager.status(&id)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "failed"),
    }
}

pub(super) async fn start(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<Book>, JsonRejection>,
) -> Response {
    let Json(book) = match body {
        Ok(value) => value,
        Err(error) => return bad_body(error),
    };
    let id = match PackId::new(book.pack) {
        Ok(id) => id,
        Err(_) => return problem(StatusCode::BAD_REQUEST, "bad_request"),
    };
    let manager = match manager(&state) {
        Some(value) => value,
        None => return problem(StatusCode::SERVICE_UNAVAILABLE, "preparation_unavailable"),
    };
    command(manager, id, None).await
}

pub(super) async fn control(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<Control>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(value) => value,
        Err(error) => return bad_body(error),
    };
    if body.epoch.starts_with('0') || !body.epoch.bytes().all(|b| b.is_ascii_digit()) {
        return problem(StatusCode::BAD_REQUEST, "bad_request");
    }
    let epoch = match body.epoch.parse::<u64>() {
        Ok(value) if value > 0 => value,
        _ => return problem(StatusCode::BAD_REQUEST, "bad_request"),
    };
    let manager = match manager(&state) {
        Some(value) => value,
        None => return problem(StatusCode::SERVICE_UNAVAILABLE, "preparation_unavailable"),
    };
    let task = Arc::clone(&manager);
    let ticket =
        match tokio::task::spawn_blocking(move || task.jobs.ticket_for_job(&body.job_id, epoch))
            .await
        {
            Ok(Ok(ticket)) => ticket,
            Ok(Err(error)) => return failure(error),
            Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "failed"),
        };
    let id = match PackId::new(&ticket.pack) {
        Ok(id) => id,
        Err(_) => return problem(StatusCode::BAD_REQUEST, "bad_request"),
    };
    command(manager, id, Some((ticket, body.action))).await
}

async fn command(manager: Arc<Manager>, id: PackId, control: Option<(Ticket, Action)>) -> Response {
    match tokio::task::spawn_blocking(move || {
        let action = control.as_ref().map(|(_, action)| *action);
        let snapshot = if let Some((ticket, action)) = control {
            match manager.jobs.request(&ticket, action) {
                Ok(snapshot) => snapshot,
                Err(IndexJobError::InvalidRequest) if action != Action::Resume => {
                    // A poll's stop button can outlive the work; return source-checked truth.
                    if manager.jobs.status(&id)?.is_some_and(|current| {
                        current.ticket == ticket
                            && matches!(
                                current.state,
                                JobState::Ready
                                    | JobState::Paused
                                    | JobState::Cancelled
                                    | JobState::Failed
                            )
                    }) {
                        return manager.status(&id);
                    }
                    return Err(IndexJobError::InvalidRequest);
                }
                Err(error) => return Err(error),
            }
        } else {
            let source = manager.registry.preparation_source(&id)?;
            manager.jobs.start(&source)?
        };
        if let Err(dispatch) = manager.send(&snapshot, action) {
            if matches!(dispatch, ContentDispatchError::Rejected) {
                manager
                    .jobs
                    .dispatch_failed(&snapshot.ticket, DispatchFailure::Rejected)?;
            }
            return Err(IndexJobError::Pack(PackError::Io(
                "Native preparation admission unconfirmed".into(),
            )));
        }
        Ok::<_, IndexJobError>(view(&id, Some(&snapshot), None, None))
    })
    .await
    {
        Ok(Ok(value)) => (StatusCode::ACCEPTED, Json(value)).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "request_unconfirmed"),
    }
}
