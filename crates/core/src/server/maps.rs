use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::ContentState;

pub(super) async fn listing(State(state): State<Arc<ContentState>>) -> Response {
    let tiles = match state.registry.tiles.read() {
        Ok(tiles) => tiles,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut entries: Vec<_> = tiles.iter().filter(|(id, _)| state.allows(id)).collect();
    entries.sort_unstable_by_key(|(id, _)| id.as_str());
    let packs: Vec<_> = entries
        .into_iter()
        .map(|(id, entry)| {
            let meta = &entry.metadata;
            serde_json::json!({
                "id": id.as_str(), "bounds": meta.bounds,
                "min_zoom": meta.min_zoom, "max_zoom": meta.max_zoom,
                "center": meta.center, "attribution": meta.attribution,
            })
        })
        .collect();
    Json(serde_json::json!({"packs": packs})).into_response()
}
