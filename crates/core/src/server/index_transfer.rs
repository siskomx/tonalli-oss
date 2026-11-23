//! Read-only search snapshots for an already selected installed book.
use super::{ContentState, PackId};
use crate::{
    index::{snapshot::Snapshot, Index},
    pack::{PackError, PackSource},
    zim::Zim,
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn selected(state: &ContentState, name: &str) -> Result<(Arc<Zim>, Arc<Index>), StatusCode> {
    let id = PackId::new(name).map_err(|_| StatusCode::NOT_FOUND)?;
    if !state.allows(&id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let zims = state
        .registry
        .zims
        .read()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let zim = Arc::clone(zims.get(&id).ok_or(StatusCode::NOT_FOUND)?);
    let indexes = state
        .registry
        .indexes
        .read()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let index = Arc::clone(indexes.get(&id).ok_or(StatusCode::NOT_FOUND)?);
    Ok((zim, index))
}

pub(super) async fn manifest(
    State(state): State<Arc<ContentState>>,
    Path(name): Path<String>,
) -> Response {
    let (zim, index) = match selected(&state, &name) {
        Ok(pair) => pair,
        Err(status) => return status.into_response(),
    };
    let cancelled = CancellationToken::new();
    let _guard = cancelled.clone().drop_guard();
    let result = tokio::task::spawn_blocking(move || {
        Snapshot::export_controlled(&index, &zim, || !cancelled.is_cancelled())
    })
    .await;
    match result {
        Ok(Ok(Some(snapshot))) => Json(snapshot.manifest()).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

// Keep the source/reader lease through the final streamed byte, including media probes.
struct IndexFile {
    source: Arc<dyn PackSource>,
    zim: Arc<Zim>,
    index: Arc<Index>,
}
impl PackSource for IndexFile {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        self.index.check_zim(&self.zim)?;
        self.source.read_at(off, buf)
    }
}

pub(super) async fn file(
    State(state): State<Arc<ContentState>>,
    Path((id, generation, name)): Path<(String, String, String)>,
    method: Method,
    mut headers: HeaderMap,
) -> Response {
    let (zim, index) = match selected(&state, &id) {
        Ok(pair) => pair,
        Err(status) => return status.into_response(),
    };
    let etag = format!("\"{generation}-{name}\"");
    let source = match tokio::task::spawn_blocking(move || {
        let source = Snapshot::open_file(&index, &zim, &generation, &name)?;
        Ok::<_, PackError>(Arc::new(IndexFile { source, zim, index }) as Arc<dyn PackSource>)
    })
    .await
    {
        Ok(Ok(source)) => source,
        Ok(Err(PackError::NotFound(_))) => return StatusCode::NOT_FOUND.into_response(),
        _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if headers
        .remove(header::IF_RANGE)
        .is_some_and(|condition| condition.as_bytes() != etag.as_bytes())
    {
        headers.remove(header::RANGE);
    }
    let mut reply = super::ranges::serve_source(source, method, headers).await;
    reply.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = header::HeaderValue::from_str(&etag) {
        reply.headers_mut().insert(header::ETAG, value);
    }
    reply
}
