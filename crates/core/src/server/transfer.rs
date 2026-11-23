//! Read-only export of the same installed sources admitted by the share allowlist.
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::{ContentState, PackId};
use crate::pack::{PackError, PackSource};

struct Selected {
    source: Arc<dyn PackSource>,
    kind: &'static str,
    zim: Option<Arc<crate::zim::Zim>>,
}

fn selected(state: &ContentState, name: &str) -> Result<Selected, StatusCode> {
    let id = PackId::new(name).map_err(|_| StatusCode::NOT_FOUND)?;
    if !state.allows(&id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let zim = state
        .registry
        .zims
        .read()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .get(&id)
        .cloned();
    if let Some(zim) = zim {
        return Ok(Selected {
            source: zim.index_source().0,
            kind: "zim",
            zim: Some(zim),
        });
    }
    state
        .registry
        .tiles
        .read()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .get(&id)
        .map(|entry| Selected {
            source: Arc::clone(&entry.source),
            kind: "map",
            zim: None,
        })
        .ok_or(StatusCode::NOT_FOUND)
}

fn checksum(source: &dyn PackSource, cancelled: &CancellationToken) -> Result<String, PackError> {
    let bytes = source.len();
    let mut offset = 0;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    while offset < bytes {
        if cancelled.is_cancelled() {
            return Err(PackError::Io("Transfer inspection stopped".into()));
        }
        let count = (bytes - offset).min(buffer.len() as u64) as usize;
        let read = source.read_at(offset, &mut buffer[..count])?;
        if read == 0 {
            return Err(PackError::Truncated);
        }
        if read > count {
            return Err(PackError::Corrupt("Invalid source read length".into()));
        }
        hash.update(&buffer[..read]);
        offset += read as u64;
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) async fn manifest(
    State(state): State<Arc<ContentState>>,
    Path(name): Path<String>,
) -> Response {
    let selected = match selected(&state, &name) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    let cancelled = CancellationToken::new();
    let _cancel_on_drop = cancelled.clone().drop_guard();
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, PackError> {
        let bytes = selected.source.len();
        let sha256=checksum(selected.source.as_ref(),&cancelled)?;
        let title = match selected.zim { Some(zim) => zim.metadata_title()?.unwrap_or_else(|| name.clone()), None => name.clone() };
        Ok(json!({"version":1,"id":name,"title":title,"kind":selected.kind,"bytes":bytes,"sha256":sha256}))
    }).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(super) async fn pack(
    State(state): State<Arc<ContentState>>,
    Path(name): Path<String>,
    method: Method,
    mut headers: HeaderMap,
) -> Response {
    let selected = match selected(&state, &name) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    let source = Arc::clone(&selected.source);
    let cancelled = CancellationToken::new();
    let _cancel_on_drop = cancelled.clone().drop_guard();
    let digest =
        match tokio::task::spawn_blocking(move || checksum(source.as_ref(), &cancelled)).await {
            Ok(Ok(hash)) => hash,
            _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
    let etag = format!("\"{digest}\"");
    if let Some(condition) = headers.remove(header::IF_RANGE) {
        if condition.as_bytes() != etag.as_bytes() {
            headers.remove(header::RANGE);
        }
    }
    let mut reply = super::ranges::serve_source(selected.source, method, headers).await;
    reply.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = header::HeaderValue::from_str(&etag) {
        reply.headers_mut().insert(header::ETAG, value);
    }
    reply
}
