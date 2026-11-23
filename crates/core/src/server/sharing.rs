//! Private consumer controls. Native shells own network selection and server lifetime.
use super::ServerState;
use crate::pack::PackError;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SharingNetwork {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SharingDevice {
    pub id: String,
    pub label: String,
}
/// Credentials of a phone-created local-only hotspot. Served only by the private sharing route.
#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SharingHotspot {
    pub ssid: String,
    pub password: String,
}
#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SharingStatus {
    pub running: bool,
    pub busy: bool,
    pub url: Option<String>,
    pub pairing_code: Option<String>,
    pub devices: Vec<SharingDevice>,
    pub networks: Vec<SharingNetwork>,
    pub error: Option<String>,
    pub certificate_sha256: Option<String>,
    pub friendly_url: Option<String>,
    pub native_invitation: Option<String>,
    pub hotspot: Option<SharingHotspot>,
}
pub trait SharingControl: Send + Sync {
    fn room(
        &self,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
        if matches!(
            request.action,
            crate::room::RoomAction::Get | crate::room::RoomAction::Leave
        ) {
            Ok(crate::room::RoomSnapshot::default())
        } else {
            Err(crate::room::RoomError::Unavailable)
        }
    }
    fn status(&self) -> Result<SharingStatus, PackError>;
    fn start(
        &self,
        network: String,
        packs: Vec<String>,
        media: Arc<super::SharedMedia>,
    ) -> Result<(), PackError>;
    fn stop(&self) -> Result<(), PackError>;
    fn kick(&self, id: String) -> Result<(), PackError>;
    fn close(&self) -> Result<(), PackError> {
        self.stop()
    }
}
fn control(state: &ServerState) -> Result<Arc<dyn SharingControl>, Response> {
    state
        .sharing
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| StatusCode::SERVICE_UNAVAILABLE.into_response())
}
fn response(
    result: Result<Result<serde_json::Value, PackError>, tokio::task::JoinError>,
) -> Response {
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"sharing_unavailable"})),
        )
            .into_response(),
    }
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    let controller = match control(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    response(
        tokio::task::spawn_blocking(move || {
            let status = controller.status()?;
            let books = state
                .registry
                .zims
                .read()
                .map_err(|_| super::registry_poisoned())?
                .iter()
                .map(|(id, zim)| (id.as_str().to_owned(), Arc::clone(zim)))
                .collect::<Vec<_>>();
            let mut packs = Vec::new();
            for (id, zim) in books {
                if let Ok(Some(title)) = zim.metadata_title() {
                    packs.push(serde_json::json!({"id":id,"title":title,"kind":"book"}));
                }
            }
            for (id, _) in state
                .registry
                .tiles
                .read()
                .map_err(|_| super::registry_poisoned())?
                .iter()
            {
                packs.push(serde_json::json!({"id":id.as_str(),"title":"Map file","kind":"map"}));
            }
            let selected = if status.running || status.busy {
                state
                    .shared_selection
                    .lock()
                    .map_err(|_| super::registry_poisoned())?
                    .clone()
            } else {
                Vec::new()
            };
            let shared_packs = if status.running || status.busy {
                state
                    .shared_live
                    .lock()
                    .map_err(|_| super::registry_poisoned())?
                    .as_ref()
                    .map(|(media, started)| {
                        started
                            .iter()
                            .filter(|id| !media.pack_hidden_name(id))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            Ok(serde_json::json!({"status":status,"packs":packs,"selected_media":selected,"shared_packs":shared_packs}))
        })
        .await,
    )
}
pub(super) async fn room_get(
    State(state): State<Arc<ServerState>>,
    Query(cursor): Query<super::room::Cursor>,
) -> Response {
    room_request(state, cursor.request()).await
}
pub(super) async fn room_post(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<crate::room::RoomRequest>,
) -> Response {
    room_request(state, request).await
}
async fn room_request(state: Arc<ServerState>, request: crate::room::RoomRequest) -> Response {
    let controller = state.sharing.lock().ok().and_then(|slot| slot.clone());
    let result = tokio::task::spawn_blocking(move || match controller {
        Some(controller) => controller.room(request),
        None if matches!(
            request.action,
            crate::room::RoomAction::Get | crate::room::RoomAction::Leave
        ) =>
        {
            Ok(crate::room::RoomSnapshot::default())
        }
        None => Err(crate::room::RoomError::Unavailable),
    })
    .await
    .unwrap_or(Err(crate::room::RoomError::Unavailable));
    super::room::response(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    network: String,
    packs: Vec<String>,
    #[serde(default)]
    media: Vec<String>,
}
pub(super) async fn start(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Start>,
) -> Response {
    let controller = match control(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    response(
        tokio::task::spawn_blocking(move || {
            if request.packs.len() > 128
                || request.media.len() > super::shared_media::MAX_SHARED_FILES
                || request.network.len() > 256
            {
                return Err(PackError::Mismatch(
                    "Choose installed content and a local network".into(),
                ));
            }
            let media = Arc::new(super::SharedMedia::capture(
                &state.data_dir,
                &request.media,
            )?);
            let packs = request.packs.clone();
            controller.start(request.network, request.packs, Arc::clone(&media))?;
            *state
                .shared_selection
                .lock()
                .map_err(|_| super::registry_poisoned())? = media.metadata();
            *state
                .shared_live
                .lock()
                .map_err(|_| super::registry_poisoned())? = Some((media, packs));
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hide {
    #[serde(default)]
    packs: Vec<String>,
    #[serde(default)]
    media: Vec<String>,
}
/// Removal only: takes books, maps or files off a running share. There is no route to add one.
pub(super) async fn hide(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Hide>,
) -> Response {
    response(
        tokio::task::spawn_blocking(move || {
            let live = state
                .shared_live
                .lock()
                .map_err(|_| super::registry_poisoned())?
                .as_ref()
                .map(|(media, _)| Arc::clone(media))
                .ok_or_else(|| PackError::Mismatch("Sharing is not running".into()))?;
            live.hide(&request.packs, &request.media)?;
            *state
                .shared_selection
                .lock()
                .map_err(|_| super::registry_poisoned())? = live.metadata();
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
pub(super) async fn stop(State(state): State<Arc<ServerState>>) -> Response {
    let controller = match control(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    response(
        tokio::task::spawn_blocking(move || {
            controller.stop()?;
            state
                .shared_selection
                .lock()
                .map_err(|_| super::registry_poisoned())?
                .clear();
            *state
                .shared_live
                .lock()
                .map_err(|_| super::registry_poisoned())? = None;
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
pub(super) async fn kick(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Response {
    let controller = match control(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    response(
        tokio::task::spawn_blocking(move || {
            if id.len() > 128 {
                return Err(PackError::Mismatch("Device".into()));
            }
            controller.kick(id)?;
            Ok(serde_json::json!({"accepted":true}))
        })
        .await,
    )
}
