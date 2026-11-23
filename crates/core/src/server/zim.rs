use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Deserialize;

use super::{ContentState, PackId};
use crate::pack::PackError;
use crate::zim::Zim;

pub(super) const CSP: &str = "default-src 'none'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'";
const PATH_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
const MAX_INPUT: usize = 4096;
const MAX_ROWS: u64 = 200;

#[derive(Deserialize)]
pub(super) struct TitlesQuery {
    pack: Option<String>,
    #[serde(default)]
    offset: u64,
    limit: Option<u64>,
}
#[derive(Deserialize)]
pub(super) struct SuggestQuery {
    pack: Option<String>,
    #[serde(default)]
    q: String,
    limit: Option<u64>,
}
#[derive(Deserialize)]
pub(super) struct ArticleQuery {
    pack: Option<String>,
    url: String,
}

pub(super) async fn titles(
    State(state): State<Arc<ContentState>>,
    Query(query): Query<TitlesQuery>,
) -> Response {
    let (pack, zim) = match selected(&state, query.pack.as_deref()) {
        Ok(selected) => selected,
        Err((status, code)) => return problem(status, code),
    };
    json_read(move || {
        let total = zim.title_count()? as u64;
        let end = query
            .offset
            .saturating_add(query.limit.unwrap_or(MAX_ROWS).min(MAX_ROWS))
            .min(total);
        let mut rows = Vec::new();
        for position in query.offset.min(total)..end {
            let position = u32::try_from(position)
                .map_err(|_| PackError::Corrupt("title position overflow".into()))?;
            let row = zim.title_at(position)?;
            rows.push(serde_json::json!({"title": row.title, "url": row.url}));
        }
        Ok(serde_json::json!({"pack":pack,"total":total,"offset":query.offset,"rows":rows}))
    })
    .await
}

pub(super) async fn suggest(
    State(state): State<Arc<ContentState>>,
    Query(query): Query<SuggestQuery>,
) -> Response {
    if query.q.len() > MAX_INPUT {
        return bad_request();
    }
    let (pack, zim) = match selected(&state, query.pack.as_deref()) {
        Ok(selected) => selected,
        Err((status, code)) => return problem(status, code),
    };
    json_read(move || {
        let hits: Vec<_> = zim
            .suggest(&query.q, query.limit.unwrap_or(20).min(MAX_ROWS) as usize)?
            .into_iter()
            .map(|hit| serde_json::json!({"title":hit.title,"url":hit.url}))
            .collect();
        Ok(serde_json::json!({"pack":pack,"hits":hits}))
    })
    .await
}

pub(super) async fn article_info(
    State(state): State<Arc<ContentState>>,
    Query(query): Query<ArticleQuery>,
) -> Response {
    if query.url.is_empty() || query.url.len() > MAX_INPUT {
        return bad_request();
    }
    let (pack, zim) = match selected(&state, query.pack.as_deref()) {
        Ok(selected) => selected,
        Err((status, code)) => return problem(status, code),
    };
    json_read(move || {
        let info = zim.entry_info(&query.url)?;
        let iframe_url = content_path(&pack, &info.url)?;
        Ok(
            serde_json::json!({"pack":pack,"title":info.title,"url":info.url,
            "mime_type":info.mime_type,"iframe_url":iframe_url}),
        )
    })
    .await
}

pub(super) async fn content(
    State(state): State<Arc<ContentState>>,
    Path((pack, path)): Path<(String, String)>,
) -> Response {
    if path.is_empty() || path.len() > MAX_INPUT {
        return bad_request();
    }
    let (pack, zim) = match selected(&state, Some(&pack)) {
        Ok(selected) => selected,
        Err((status, code)) => return problem(status, code),
    };
    let result = blocking(move || {
        let info = zim.entry_info(&path)?;
        let canonical = content_path(&pack, &info.url)?;
        if info.redirected || info.url != path {
            return Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, canonical)
                .body(Body::empty())
                .map_err(|_| PackError::Corrupt("invalid redirect location".into()));
        }
        let article = zim.article(&info.url)?;
        let mime = HeaderValue::from_str(&article.mime_type)
            .map_err(|_| PackError::Corrupt("invalid entry MIME header".into()))?;
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CONTENT_LENGTH, article.bytes.len())
            .body(Body::from(article.bytes))
            .map_err(|_| PackError::Corrupt("invalid article response".into()))
    })
    .await;
    match result {
        Ok(response) => response,
        Err(error) => read_error(error),
    }
}

fn selected(
    state: &ContentState,
    requested: Option<&str>,
) -> Result<(String, Arc<Zim>), (StatusCode, &'static str)> {
    let id = requested
        .map(PackId::new)
        .transpose()
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad_request"))?;
    let zims = state
        .registry
        .zims
        .read()
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
    let (id, zim) = if let Some(id) = &id {
        if !state.allows(id) {
            return Err((StatusCode::NOT_FOUND, "not_found"));
        }
        zims.get_key_value(id)
            .ok_or((StatusCode::NOT_FOUND, "not_found"))?
    } else {
        let mut allowed = zims.iter().filter(|(id, _)| state.allows(id));
        let first = allowed
            .next()
            .ok_or((StatusCode::BAD_REQUEST, "select_pack"))?;
        if allowed.next().is_some() {
            return Err((StatusCode::BAD_REQUEST, "select_pack"));
        }
        first
    };
    // Only owned handles cross the worker boundary; no registry lock spans I/O.
    Ok((id.as_str().into(), Arc::clone(zim)))
}

fn content_path(pack: &str, url: &str) -> Result<String, PackError> {
    if url.len() > MAX_INPUT || url.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(PackError::Mismatch(
            "entry URL cannot be mounted safely".into(),
        ));
    }
    let path = url
        .split('/')
        .map(|part| utf8_percent_encode(part, PATH_COMPONENT).to_string())
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!("/zim/{pack}/{path}"))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, PackError> + Send + 'static,
) -> Result<T, PackError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| PackError::Corrupt("ZIM read worker panicked".into()))?
}
async fn json_read(
    work: impl FnOnce() -> Result<serde_json::Value, PackError> + Send + 'static,
) -> Response {
    match blocking(work).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => read_error(error),
    }
}
fn bad_request() -> Response {
    problem(StatusCode::BAD_REQUEST, "bad_request")
}
fn problem(status: StatusCode, code: &str) -> Response {
    (status, Json(serde_json::json!({"error":code}))).into_response()
}
fn read_error(error: PackError) -> Response {
    let (status, code) = match error {
        PackError::MediaRemoved => (StatusCode::SERVICE_UNAVAILABLE, "media_removed"),
        PackError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        PackError::Mismatch(_) => (StatusCode::BAD_REQUEST, "bad_request"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "corrupt_pack"),
    };
    problem(status, code)
}
