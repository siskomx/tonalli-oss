use std::path::{Component, Path};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio_util::io::ReaderStream;

use super::ContentState;

pub(super) async fn serve(State(state): State<Arc<ContentState>>, request: Request) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let decoded = match percent_encoding::percent_decode_str(request.uri().path()).decode_utf8() {
        Ok(path) => path,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let relative = decoded.trim_start_matches('/');
    if relative.contains(['\\', ':', '\0'])
        || Path::new(relative)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let first = relative.split('/').next().unwrap_or("");
    if ["api", "zim", "packs", "launch"]
        .iter()
        .any(|reserved| first.eq_ignore_ascii_case(reserved))
        || relative.split('/').any(|part| part.starts_with('.'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let app_route = (matches!(
        request.uri().path(),
        "/chat"
            | "/content"
            | "/settings"
            | "/files"
            | "/music"
            | "/videos"
            | "/gallery"
            | "/file"
            | "/reader"
            | "/cookbook"
            | "/workshop"
    ) && state.allowed.is_none())
        || matches!(
            request.uri().path(),
            "/" | "/library" | "/article" | "/map" | "/notes" | "/shared" | "/room"
        );
    let path = state
        .web_root
        .join(if app_route { "index.html" } else { relative });
    let path = match tokio::fs::canonicalize(path).await {
        Ok(path) if path.starts_with(&state.web_root) => path,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = if relative == "assets/map/attribution" {
        Some("text/html; charset=utf-8")
    } else {
        mime(&path)
    };
    let Some(mime) = mime else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let metadata = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let body = if request.method() == Method::HEAD {
        Body::empty()
    } else {
        Body::from_stream(ReaderStream::with_capacity(file, 64 * 1024))
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, metadata.len());
    if mime.starts_with("text/html") {
        response = response.header(header::CONTENT_SECURITY_POLICY, "frame-src 'self'");
    }
    match response.body(body) {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn mime(path: &Path) -> Option<&'static str> {
    Some(match path.extension()?.to_str()? {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "bcmap" | "pfb" | "icc" => "application/octet-stream",
        "txt" => "text/plain; charset=utf-8",
        "pbf" => "application/x-protobuf",
        _ => return None,
    })
}
