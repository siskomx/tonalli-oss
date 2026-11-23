//! Private controls for the separate, explicitly selected-media TV owner.
use super::ServerState;
use crate::pack::PackError;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone, Default, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TvStatus {
    pub running: bool,
    pub busy: bool,
    pub url: Option<String>,
    pub networks: Vec<super::SharingNetwork>,
    pub error: Option<String>,
    #[serde(skip)]
    pub selected_media: Vec<crate::media::MediaFile>,
}
pub trait TvControl: Send + Sync {
    fn status(&self) -> Result<TvStatus, PackError>;
    fn start(&self, network: String, catalog: Arc<super::TvCatalog>) -> Result<(), PackError>;
    fn stop(&self) -> Result<(), PackError>;
    fn close(&self) -> Result<(), PackError> {
        self.stop()
    }
}

fn control(state: &ServerState) -> Result<Arc<dyn TvControl>, StatusCode> {
    state
        .tv
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
fn response(
    result: Result<Result<serde_json::Value, PackError>, tokio::task::JoinError>,
) -> Response {
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"tv_unavailable"})),
        )
            .into_response(),
    }
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    let controller = match control(&state) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    response(
        tokio::task::spawn_blocking(move || {
            let status = controller.status()?;
            let selected = if status.running || status.busy {
                if !status.selected_media.is_empty() {
                    status.selected_media.clone()
                } else {
                    state
                        .tv_selection
                        .lock()
                        .map_err(|_| super::registry_poisoned())?
                        .clone()
                }
            } else {
                Vec::new()
            };
            Ok(serde_json::json!({"status":status,"selected_media":selected}))
        })
        .await,
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    network: String,
    media: Vec<String>,
}
pub(super) async fn start(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Start>,
) -> Response {
    let controller = match control(&state) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    response(
        tokio::task::spawn_blocking(move || {
            if request.network.is_empty()
                || request.network.len() > 256
                || request.media.len() > 128
            {
                return Err(PackError::Mismatch(
                    "Choose media and a local network".into(),
                ));
            }
            let media = Arc::new(super::SharedMedia::capture(
                &state.data_dir,
                &request.media,
            )?);
            let selected = media.metadata();
            let catalog = Arc::new(super::TvCatalog::new(media)?);
            controller.start(request.network, catalog)?;
            *state
                .tv_selection
                .lock()
                .map_err(|_| super::registry_poisoned())? = selected;
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
pub(super) async fn stop(State(state): State<Arc<ServerState>>) -> Response {
    let controller = match control(&state) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    response(
        tokio::task::spawn_blocking(move || {
            controller.stop()?;
            state
                .tv_selection
                .lock()
                .map_err(|_| super::registry_poisoned())?
                .clear();
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
