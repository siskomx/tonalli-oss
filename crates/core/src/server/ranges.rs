use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{ContentState, PackId};
use crate::pack::{PackError, PackSource};

const CHUNK: usize = 64 * 1024;

struct RangeStream {
    source: Arc<dyn PackSource>,
    offset: u64,
    remaining: u64,
    first: Option<Bytes>,
}

pub(super) async fn serve(
    State(state): State<Arc<ContentState>>,
    Path(name): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Some(id) = name
        .strip_suffix(".pmtiles")
        .and_then(|id| PackId::new(id).ok())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !state.allows(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let source = match state.registry.tiles.read() {
        Ok(tiles) => match tiles.get(&id) {
            Some(entry) => Arc::clone(&entry.source),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    serve_source(source, method, headers).await
}

pub(super) async fn serve_source(
    source: Arc<dyn PackSource>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let length = source.len();
    let selection = if method == Method::HEAD {
        Ok(None)
    } else {
        select_range(&headers, length)
    };
    let selection = match selection {
        Ok(selection) => selection,
        Err(()) => {
            return response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                0,
                Some(format!("bytes */{length}")),
                Body::empty(),
            )
        }
    };
    let (start, count, status, content_range) = match selection {
        Some((start, end)) => (
            start,
            end - start + 1,
            StatusCode::PARTIAL_CONTENT,
            Some(format!("bytes {start}-{end}/{length}")),
        ),
        None => (0, length, StatusCode::OK, None),
    };
    if method == Method::HEAD || count == 0 {
        return response(status, count, content_range, Body::empty());
    }
    // Prefetch one bounded chunk so immediate media failures have an HTTP status.
    let first = match read_chunk(Arc::clone(&source), start, count).await {
        Ok(bytes) => bytes,
        Err(error) => return error_status(&error).into_response(),
    };
    let stream = RangeStream {
        source,
        offset: start + first.len() as u64,
        remaining: count - first.len() as u64,
        first: Some(first),
    };
    let body = Body::from_stream(futures_util::stream::unfold(
        stream,
        |mut stream| async move {
            if let Some(first) = stream.first.take() {
                return Some((Ok(first), stream));
            }
            if stream.remaining == 0 {
                return None;
            }
            let result =
                read_chunk(Arc::clone(&stream.source), stream.offset, stream.remaining).await;
            match &result {
                Ok(bytes) => {
                    stream.offset += bytes.len() as u64;
                    stream.remaining -= bytes.len() as u64;
                }
                Err(_) => stream.remaining = 0,
            }
            Some((result, stream))
        },
    ));
    response(status, count, content_range, body)
}

fn response(status: StatusCode, count: u64, range: Option<String>, body: Body) -> Response {
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/vnd.pmtiles")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, count);
    if let Some(range) = range {
        response = response.header(header::CONTENT_RANGE, range);
    }
    match response.body(body) {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn error_status(error: &PackError) -> StatusCode {
    match error {
        PackError::MediaRemoved => StatusCode::SERVICE_UNAVAILABLE,
        PackError::NotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn read_chunk(
    source: Arc<dyn PackSource>,
    offset: u64,
    remaining: u64,
) -> Result<Bytes, PackError> {
    tokio::task::spawn_blocking(move || {
        let count = remaining.min(CHUNK as u64) as usize;
        let mut bytes = vec![0; count];
        let mut done = 0;
        while done < count {
            let read = source.read_at(offset + done as u64, &mut bytes[done..])?;
            if read == 0 {
                return Err(PackError::Truncated);
            }
            if read > count - done {
                return Err(PackError::Corrupt(
                    "source returned an impossible read length".into(),
                ));
            }
            done += read;
        }
        Ok(Bytes::from(bytes))
    })
    .await
    .map_err(|_| PackError::Corrupt("positioned range reader panicked".into()))?
}

// A single byte range suffices for PMTiles. Unsupported units or multipart
// requests are ignored (whole representation), as permitted by RFC 9110 §14.2.
fn select_range(headers: &HeaderMap, length: u64) -> Result<Option<(u64, u64)>, ()> {
    // No representation validator is issued yet, so If-Range cannot match.
    if headers.contains_key(header::IF_RANGE) {
        return Ok(None);
    }
    let mut values = headers.get_all(header::RANGE).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Ok(None);
    }
    let value = value.to_str().map_err(|_| ())?;
    let Some((unit, value)) = value.split_once('=') else {
        return Ok(None);
    };
    if !unit.eq_ignore_ascii_case("bytes") {
        return Ok(None);
    }
    if value.contains(',') {
        return Ok(None);
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    let number = |value: &str| {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(());
        }
        value.parse::<u64>().map_err(|_| ())
    };
    if length == 0 {
        return Err(());
    }
    let (start, end) = if start.is_empty() {
        let suffix = number(end)?;
        if suffix == 0 {
            return Err(());
        }
        (length.saturating_sub(suffix), length - 1)
    } else {
        let start = number(start)?;
        let end = if end.is_empty() {
            length - 1
        } else {
            number(end)?.min(length - 1)
        };
        if start >= length || end < start {
            return Err(());
        }
        (start, end)
    };
    Ok(Some((start, end)))
}
