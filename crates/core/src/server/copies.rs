//! Private management of verified nearby copies; no catalog authority or disk paths in requests.
use super::{PackId, ServerState};
use crate::{content::RemoveOutcome, pack::PackError, sync::SyncStore};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

fn problem(error: PackError) -> Response {
    let (status, code) = match error {
        PackError::Mismatch(_) => (StatusCode::CONFLICT, "in_use"),
        PackError::PublicationUncertain(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, "cleanup_uncertain")
        }
        _ => (StatusCode::SERVICE_UNAVAILABLE, "copies_unavailable"),
    };
    (status, Json(json!({"error":code}))).into_response()
}
pub(super) async fn listing(State(state): State<Arc<ServerState>>) -> Response {
    match tokio::task::spawn_blocking(move || {
        let store = SyncStore::open(&state.data_dir)?;
        let rows = store.inventory()?;
        for row in &rows { if !row.removal_pending { state.registry.register_copy(&store, &row.manifest)?; } }
        Ok::<_,PackError>(json!({"copies":rows.into_iter().map(|entry| {let row=entry.manifest;json!({"id":row.local_id(),"title":row.title,"kind":row.kind,"bytes":row.bytes,"removalPending":entry.removal_pending})}).collect::<Vec<_>>()}))
    }).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => problem(error),
        Err(_) => problem(PackError::Io("Copy listing stopped".into())),
    }
}
pub(super) async fn remove(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut tokens = headers.get_all("x-tonalli-admin").iter();
    let token = tokens
        .next()
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if tokens.next().is_some() || !state.admin.permits(&token) {
        return (StatusCode::LOCKED, Json(json!({"error":"admin_locked"}))).into_response();
    }
    if crate::sync::hash_for_id(&id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match tokio::task::spawn_blocking(move || {
        if !state.admin.permits(&token) { return Ok(None); }
        let store = SyncStore::open(&state.data_dir)?;
        state.registry.remove_copy(&store, &PackId::new(id)?, || state.admin.permits(&token))
    }).await {
        Ok(Ok(Some(outcome))) => Json(json!({"removed":outcome == RemoveOutcome::Removed,"reason":if outcome == RemoveOutcome::InUse { Some("in_use") } else { None }})).into_response(),
        Ok(Ok(None)) => (StatusCode::LOCKED, Json(json!({"error":"admin_locked"}))).into_response(),
        Ok(Err(error)) => problem(error),
        Err(_) => problem(PackError::Io("Copy removal stopped".into())),
    }
}
