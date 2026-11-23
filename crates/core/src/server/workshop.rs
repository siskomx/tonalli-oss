//! Private managed geometry only; no arbitrary paths, textures or linked files.
use super::ServerState;
use crate::{
    media::{MediaError, MediaFile, MediaStore},
    pack::PackError,
    workshop::{self, Model, ModelFormat},
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: String,
}
#[derive(Serialize)]
struct Preview {
    file: MediaFile,
    model: Model,
}
fn problem(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}
pub(super) async fn open(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Request>,
) -> Response {
    if request.id.len() != 32
        || !request
            .id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return problem(StatusCode::BAD_REQUEST, "invalid_model");
    }
    let Ok(permit) = Arc::clone(&state.model_slots).try_acquire_owned() else {
        return problem(StatusCode::TOO_MANY_REQUESTS, "model_busy");
    };
    let root = state.data_dir.clone();
    let opened = tokio::task::spawn_blocking(move || {
        // Keep the slot until parsing finishes even if the requesting page departs.
        let _permit = permit;
        let (file, source) = MediaStore::open(&root)
            .and_then(|store| store.open_file(&request.id))
            .map_err(|error| match error {
                MediaError::NotFound => (StatusCode::NOT_FOUND, "model_missing"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "model_unavailable"),
            })?;
        let format = match file
            .name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "stl" => ModelFormat::Stl,
            "obj" => ModelFormat::Obj,
            "3mf" => ModelFormat::ThreeMf,
            _ => return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "model_unsupported")),
        };
        let model = workshop::load(source, format).map_err(|error| match error {
            PackError::Corrupt(_)
            | PackError::Mismatch(_)
            | PackError::Truncated
            | PackError::NotFound(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "model_unsupported"),
            _ => (StatusCode::SERVICE_UNAVAILABLE, "model_unavailable"),
        })?;
        Ok(Preview { file, model })
    })
    .await;
    match opened {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err((status, code))) => problem(status, code),
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "model_unavailable"),
    }
}
