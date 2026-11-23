//! Private book capabilities contain only an immutable archive, never a data root.
use super::ServerState;
use crate::{
    documents::{Book, BookKind},
    media::{MediaError, MediaStore},
    pack::{PackError, PackSource},
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;

struct Grant {
    token: String,
    book: Arc<Book>,
}
#[derive(Default)]
pub(super) struct Grants(Mutex<VecDeque<Grant>>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: Option<String>,
    pack: Option<String>,
    url: Option<String>,
}
struct EntryBytes(Vec<u8>);
impl PackSource for EntryBytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
        let bytes = self.0.get(start..).unwrap_or(&[]);
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}
fn problem(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}
fn media_error(error: MediaError) -> (StatusCode, &'static str) {
    match error {
        MediaError::NotFound => (StatusCode::NOT_FOUND, "document_missing"),
        MediaError::Invalid => (StatusCode::BAD_REQUEST, "invalid_document"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"),
    }
}
fn book_error(error: PackError) -> (StatusCode, &'static str) {
    match error {
        PackError::NotFound(_) => (StatusCode::NOT_FOUND, "document_missing"),
        PackError::Corrupt(_) | PackError::Truncated | PackError::Mismatch(_) => {
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, "document_unsupported")
        }
        _ => (StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"),
    }
}
pub(super) async fn open(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Request>,
) -> Response {
    let root = state.data_dir.clone();
    let registry = Arc::clone(&state.registry);
    let opened = tokio::task::spawn_blocking(move || {
        let invalid = (StatusCode::BAD_REQUEST, "invalid_document");
        let (name, source): (String, Arc<dyn PackSource>) =
            match (request.id, request.pack, request.url) {
                (Some(id), None, None) => {
                    if id.len() != 32
                        || !id
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        return Err(invalid);
                    }
                    let (file, source) = MediaStore::open(&root)
                        .and_then(|store| store.open_file(&id))
                        .map_err(media_error)?;
                    (file.name, source)
                }
                (None, Some(pack), Some(url)) => {
                    if !(crate::reading::Destination::Article {
                        pack: pack.clone(),
                        url: url.clone(),
                    })
                    .valid()
                    {
                        return Err(invalid);
                    }
                    let id = super::PackId::new(pack).map_err(|_| invalid)?;
                    let zim = registry
                        .zims
                        .read()
                        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"))?
                        .get(&id)
                        .cloned()
                        .ok_or((StatusCode::NOT_FOUND, "document_missing"))?;
                    let article = zim.article(&url).map_err(book_error)?;
                    if !matches!(
                        article.mime_type.split(';').next().unwrap_or("").trim(),
                        "application/epub+zip"
                            | "application/vnd.comicbook+zip"
                            | "application/x-cbz"
                            | "application/octet-stream"
                            | "application/zip"
                    ) {
                        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "document_unsupported"));
                    }
                    (article.url, Arc::new(EntryBytes(article.bytes)))
                }
                _ => return Err(invalid),
            };
        let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let kind = match extension.as_str() {
            "epub" => BookKind::Epub,
            "cbz" => BookKind::Comic,
            _ => return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "document_unsupported")),
        };
        let book = Book::open(source, kind).map_err(book_error)?;
        Ok(Arc::new(book))
    })
    .await;
    let book = match opened {
        Ok(Ok(book)) => book,
        Ok(Err((status, code))) => return problem(status, code),
        Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"),
    };
    let mut bytes = [0; 32];
    if getrandom::fill(&mut bytes).is_err() {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "document_unavailable");
    }
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let Ok(mut grants) = state.books.0.lock() else {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "document_unavailable");
    };
    if grants.len() >= 4 {
        grants.pop_front();
    }
    grants.push_back(Grant {
        token: token.clone(),
        book: Arc::clone(&book),
    });
    drop(grants);
    Json(json!({"base_url":format!("/book/{token}/"),"book":book.metadata()})).into_response()
}
pub(super) async fn resource(
    State(state): State<Arc<ServerState>>,
    Path((token, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let book = state.books.0.lock().ok().and_then(|grants| {
        grants
            .iter()
            .find(|grant| bool::from(grant.token.as_bytes().ct_eq(token.as_bytes())))
            .map(|grant| Arc::clone(&grant.book))
    });
    let Some(book) = book else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = tokio::task::spawn_blocking(move || book.resource(&path)).await;
    let resource = match result {
        Ok(Ok(resource)) => resource,
        Ok(Err(error)) => {
            let (status, code) = book_error(error);
            return problem(status, code);
        }
        Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"),
    };
    let destination = if resource.mime.starts_with("text/html") {
        "iframe"
    } else if resource.mime.starts_with("text/css") {
        "style"
    } else if resource.mime.starts_with("image/") {
        "image"
    } else if resource.mime.starts_with("font/") {
        "font"
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let actual = headers
        .get("sec-fetch-dest")
        .and_then(|value| value.to_str().ok());
    if actual != Some(destination)
        && !(resource.mime == "image/svg+xml" && actual == Some("iframe"))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, resource.mime)
        .header(header::CONTENT_LENGTH, resource.bytes.len())
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_SECURITY_POLICY,
            format!("{}; sandbox allow-same-origin", super::zim::CSP),
        )
        .body(axum::body::Body::from(resource.bytes))
    {
        Ok(response) => response,
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "document_unavailable"),
    }
}
