//! Shared request shape; host control and paired reader have distinct actors.
use crate::room::{RoomAction, RoomError, RoomRequest, RoomSnapshot};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    view: String,
    #[serde(default)]
    epoch: String,
    #[serde(default)]
    after: u64,
}
impl Cursor {
    pub(super) fn request(self) -> RoomRequest {
        RoomRequest {
            action: RoomAction::Get,
            view: self.view,
            epoch: self.epoch,
            after: self.after,
            text: String::new(),
        }
    }
}
pub(super) fn response(result: Result<RoomSnapshot, RoomError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => {
            let (status, code) = match error {
                RoomError::Invalid => (StatusCode::BAD_REQUEST, "room_invalid"),
                RoomError::NotJoined => (StatusCode::FORBIDDEN, "room_join_first"),
                RoomError::Full => (StatusCode::CONFLICT, "room_full"),
                RoomError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "room_rate"),
                RoomError::Changed => (StatusCode::CONFLICT, "room_changed"),
                RoomError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "room_unavailable"),
            };
            (status, Json(serde_json::json!({"error":code}))).into_response()
        }
    }
}
