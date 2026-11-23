//! Source-bound article-text search over a native-attached internal index.
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use super::{ContentState, PackId};
use crate::pack::PackError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchQuery {
    pack: String,
    #[serde(default)]
    q: String,
    limit: Option<usize>,
}

pub(super) async fn search(
    State(state): State<Arc<ContentState>>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let limit = query.limit.unwrap_or(20);
    if query.q.len() > 4096 || !(1..=20).contains(&limit) {
        return problem(StatusCode::BAD_REQUEST, "bad_request");
    }
    let id = match PackId::new(query.pack) {
        Ok(id) => id,
        Err(_) => return problem(StatusCode::BAD_REQUEST, "bad_request"),
    };
    if !state.allows(&id) {
        return problem(StatusCode::NOT_FOUND, "not_found");
    }
    // Capture one coherent pair in the registry's zims -> indexes lock order.
    // Neither lock crosses source I/O or Tantivy work.
    let captured = (|| {
        let zims = state
            .registry
            .zims
            .read()
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
        let zim = Arc::clone(zims.get(&id).ok_or((StatusCode::NOT_FOUND, "not_found"))?);
        let indexes = state
            .registry
            .indexes
            .read()
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
        let index = Arc::clone(
            indexes
                .get(&id)
                .ok_or((StatusCode::SERVICE_UNAVAILABLE, "index_unavailable"))?,
        );
        Ok::<_, (StatusCode, &'static str)>((zim, index))
    })();
    let (zim, index) = match captured {
        Ok(pair) => pair,
        Err((status, code)) => return problem(status, code),
    };
    let result = tokio::task::spawn_blocking(move || {
        // Probe both the index's retained source and the registered source,
        // including separate source handles with the same archive identity.
        index.check_zim(&zim)?;
        let hits = index.search(&query.q, limit)?;
        let mut rows = Vec::with_capacity(hits.len());
        for hit in hits {
            if !hit.score.is_finite() || zim.entry_info(&hit.url)?.url != hit.url {
                return Err(PackError::Corrupt("Noncanonical body search hit".into()));
            }
            rows.push(serde_json::json!({"title":hit.title,"url":hit.url,"score":hit.score}));
        }
        Ok(serde_json::json!({"pack":id.as_str(),"hits":rows}))
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(PackError::MediaRemoved)) => {
            problem(StatusCode::SERVICE_UNAVAILABLE, "media_removed")
        }
        Ok(Err(PackError::Mismatch(_))) => problem(StatusCode::BAD_REQUEST, "bad_request"),
        Ok(Err(PackError::NotFound(_))) => problem(StatusCode::NOT_FOUND, "not_found"),
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "corrupt_pack"),
    }
}

fn problem(status: StatusCode, code: &str) -> Response {
    (status, Json(serde_json::json!({"error":code}))).into_response()
}
