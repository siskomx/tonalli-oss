use super::{PackId, ServerState};
use crate::{
    media::MediaStore,
    pack::PackError,
    reading::{Destination, ProgressStore, ReadingEntry},
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

fn available(state: &ServerState, destination: &Destination) -> bool {
    match destination {
        Destination::Media { id } => MediaStore::open(&state.data_dir)
            .and_then(|store| store.source(id))
            .is_ok(),
        Destination::Map { pack } => PackId::new(pack.clone()).ok().is_some_and(|id| {
            state
                .registry
                .tiles
                .read()
                .is_ok_and(|tiles| tiles.contains_key(&id))
        }),
        Destination::Article { pack, url } => {
            let zim = PackId::new(pack.clone()).ok().and_then(|id| {
                state
                    .registry
                    .zims
                    .read()
                    .ok()
                    .and_then(|zims| zims.get(&id).cloned())
            });
            zim.is_some_and(|zim| zim.entry_info(url).is_ok_and(|entry| entry.url == *url))
        }
    }
}
async fn work(
    state: Arc<ServerState>,
    operation: impl FnOnce(&ServerState, ProgressStore) -> Result<Value, PackError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        operation(&state, ProgressStore::open(&state.data_dir)?)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"reading_unavailable"})),
        )
            .into_response(),
    }
}
pub(super) async fn listing(State(state): State<Arc<ServerState>>) -> Response {
    work(state, |state, store| {
        let entries = store.list()?;
        let unavailable: Vec<_> = entries
            .iter()
            .filter(|entry| !available(state, &entry.destination))
            .map(|entry| &entry.destination)
            .collect();
        Ok(json!({"entries":entries,"unavailable":unavailable}))
    })
    .await
}
pub(super) async fn save(
    State(state): State<Arc<ServerState>>,
    Json(entry): Json<ReadingEntry>,
) -> Response {
    if !entry.destination.valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let probe = Arc::clone(&state);
    let destination = entry.destination.clone();
    if !matches!(
        tokio::task::spawn_blocking(move || available(&probe, &destination)).await,
        Ok(true)
    ) {
        return StatusCode::NOT_FOUND.into_response();
    }
    work(state, move |_, store| {
        store.save(entry)?;
        Ok(json!({"saved":true}))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bookmark {
    destination: Destination,
    bookmarked: bool,
}
pub(super) async fn bookmark(
    State(state): State<Arc<ServerState>>,
    Json(value): Json<Bookmark>,
) -> Response {
    work(state, move |_, store| {
        store.bookmark(&value.destination, value.bookmarked)?;
        Ok(json!({"saved":true}))
    })
    .await
}
pub(super) async fn remove(
    State(state): State<Arc<ServerState>>,
    Json(destination): Json<Destination>,
) -> Response {
    work(state, move |_, store| {
        store.remove(&destination)?;
        Ok(json!({"removed":true}))
    })
    .await
}
