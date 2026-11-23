//! Private managed files; never mounted on the public content router.
use super::ServerState;
use crate::{
    media::{MediaError, MediaStore, MAX_FILE_BYTES},
    pack::PackError,
};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;

fn error(error: MediaError) -> Response {
    let (status, code) = match error {
        MediaError::Cancelled => (StatusCode::CONFLICT, "import_stopped"),
        MediaError::Invalid => (StatusCode::BAD_REQUEST, "invalid_file"),
        MediaError::Conflict => (StatusCode::CONFLICT, "file_changed"),
        MediaError::NotFound => (StatusCode::NOT_FOUND, "file_missing"),
        MediaError::Pack(PackError::NoSpace) => (StatusCode::INSUFFICIENT_STORAGE, "storage_full"),
        MediaError::Pack(PackError::Checksum | PackError::Truncated | PackError::Corrupt(_)) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "file_damaged")
        }
        MediaError::Pack(PackError::PublicationUncertain(_)) => {
            (StatusCode::SERVICE_UNAVAILABLE, "file_state_uncertain")
        }
        _ => (StatusCode::SERVICE_UNAVAILABLE, "files_unavailable"),
    };
    (status, Json(json!({"error":code}))).into_response()
}
fn locked() -> Response {
    (StatusCode::LOCKED, Json(json!({"error":"admin_locked"}))).into_response()
}
fn token(headers: &HeaderMap) -> String {
    let mut values = headers.get_all("x-tonalli-admin").iter();
    let value = values.next().and_then(|v| v.to_str().ok()).unwrap_or("");
    if values.next().is_some() {
        String::new()
    } else {
        value.to_owned()
    }
}
async fn work(
    state: Arc<ServerState>,
    headers: Option<HeaderMap>,
    operation: impl FnOnce(MediaStore) -> Result<Value, MediaError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        if headers
            .as_ref()
            .is_some_and(|h| !state.admin.permits(&token(h)))
        {
            return Ok(None);
        }
        operation(MediaStore::open(&state.data_dir)?).map(Some)
    })
    .await
    {
        Ok(Ok(Some(value))) => Json(value).into_response(),
        Ok(Ok(None)) => locked(),
        Ok(Err(e)) => error(e),
        Err(_) => error(MediaError::Catalog),
    }
}
pub(super) async fn listing(State(state): State<Arc<ServerState>>) -> Response {
    work(state, None, |store| Ok(json!({"files":store.list()?}))).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UploadQuery {
    name: String,
    #[serde(default)]
    folder: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeImport {
    #[serde(default)]
    folder: String,
}
pub(super) async fn native_import(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<NativeImport>,
) -> Response {
    work(state, Some(headers), move |store| {
        Ok(json!({"ticket":store.authorize_native_import(&body.folder)?}))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PackFile {
    pack: String,
    url: String,
}
pub(super) async fn import_document(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<PackFile>,
) -> Response {
    let owner = Arc::clone(&state);
    let key = token(&headers);
    work(state, Some(headers), move |store| {
        use std::io::Write;
        let destination = crate::reading::Destination::Article {
            pack: body.pack.clone(),
            url: body.url.clone(),
        };
        if !destination.valid() {
            return Err(MediaError::Invalid);
        }
        let id = super::PackId::new(body.pack).map_err(|_| MediaError::Invalid)?;
        let zim = owner
            .registry
            .zims
            .read()
            .map_err(|_| MediaError::Catalog)?
            .get(&id)
            .cloned()
            .ok_or(MediaError::NotFound)?;
        // The existing ZIM whole-body limit bounds this selected entry to64MiB.
        let article = zim.article(&body.url).map_err(|error| match error {
            PackError::NotFound(_) => MediaError::NotFound,
            other => MediaError::Pack(other),
        })?;
        let name = article.url.rsplit('/').next().ok_or(MediaError::Invalid)?;
        crate::download::check_space(
            fs2::available_space(&owner.data_dir).map_err(PackError::from)?,
            article.bytes.len() as u64,
        )?;
        let staged = store.begin(name, "Saved documents")?;
        {
            let mut file = staged.file()?;
            file.write_all(&article.bytes)?;
        }
        if !owner.admin.permits(&key) {
            return Err(MediaError::Cancelled);
        }
        Ok(json!(staged.finish()?))
    })
    .await
}
pub(super) async fn upload(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<UploadQuery>,
    request: Request,
) -> Response {
    let key = token(request.headers());
    if !state.admin.permits(&key) {
        return locked();
    }
    let Some(bytes) = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v <= MAX_FILE_BYTES)
    else {
        return error(MediaError::Invalid);
    };
    let root = state.data_dir.clone();
    let staged = tokio::task::spawn_blocking(move || {
        let store = MediaStore::open(&root)?;
        crate::download::check_space(fs2::available_space(&root).map_err(PackError::from)?, bytes)?;
        store.begin(&query.name, &query.folder)
    })
    .await;
    let staged = match staged {
        Ok(Ok(staged)) => staged,
        Ok(Err(e)) => return error(e),
        Err(_) => return error(MediaError::Catalog),
    };
    let file = match staged.file() {
        Ok(file) => file,
        Err(e) => return error(e),
    };
    let mut file = tokio::fs::File::from_std(file);
    let mut received = 0u64;
    let mut stream = request.into_body().into_data_stream();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                received = match received.checked_add(chunk.len() as u64) {
                    Some(n) if n <= bytes => n,
                    _ => return error(MediaError::Invalid),
                };
                if let Err(e) = file.write_all(&chunk).await {
                    return error(e.into());
                }
            }
            Ok(None) => break,
            _ => return error(PackError::Truncated.into()),
        }
    }
    if received != bytes {
        return error(PackError::Truncated.into());
    }
    if let Err(e) = file.flush().await {
        return error(e.into());
    }
    drop(file);
    match tokio::task::spawn_blocking(move || {
        if !state.admin.permits(&key) {
            return Ok(None);
        }
        staged.finish().map(|row| Some(json!(row)))
    })
    .await
    {
        Ok(Ok(Some(row))) => Json(row).into_response(),
        Ok(Ok(None)) => locked(),
        Ok(Err(e)) => error(e),
        Err(_) => error(MediaError::Catalog),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rename {
    version: u64,
    name: String,
}
pub(super) async fn rename(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Rename>,
) -> Response {
    work(state, Some(headers), move |store| {
        Ok(json!(store.rename(&id, body.version, &body.name)?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Text {
    version: u64,
    text: String,
}
pub(super) async fn edit(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Text>,
) -> Response {
    work(state, Some(headers), move |store| {
        Ok(json!(store.edit_text(&id, body.version, &body.text)?))
    })
    .await
}
pub(super) async fn text(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Response {
    work(state, None, move |store| {
        Ok(json!({"file":store.get(&id)?,"text":store.read_text(&id)?}))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Remove {
    version: u64,
    confirmed: bool,
}
pub(super) async fn remove(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Remove>,
) -> Response {
    work(state, Some(headers), move |store| {
        if !body.confirmed {
            return Err(MediaError::Invalid);
        }
        store.remove(&id, body.version)?;
        Ok(json!({"removed":true}))
    })
    .await
}
pub(super) async fn download(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let source =
        tokio::task::spawn_blocking(move || MediaStore::open(&state.data_dir)?.open_file(&id))
            .await;
    let (row, source) = match source {
        Ok(Ok(value)) => value,
        Ok(Err(e)) => return error(e),
        Err(_) => return error(MediaError::Catalog),
    };
    let mut response = super::ranges::serve_source(source, method, headers).await;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let name = percent_encoding::utf8_percent_encode(&row.name, percent_encoding::NON_ALPHANUMERIC);
    if let Ok(value) = HeaderValue::from_str(&format!(
        "attachment; filename=download; filename*=UTF-8''{name}"
    )) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response
}
