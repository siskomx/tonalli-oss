use super::{PackId, PackRegistry, ServerState};
use crate::{
    country_maps::{CountryMaps, MapDispatcher},
    pack::{PackError, PreadSource},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

pub(super) struct Manager {
    pub maps: Arc<CountryMaps>,
    pub dispatcher: Arc<dyn MapDispatcher>,
    pub registry: Arc<PackRegistry>,
}
impl Manager {
    pub fn sync(&self) -> Result<(), PackError> {
        let rows = self.maps.list()?;
        let mut wanted = Vec::new();
        for row in &rows {
            if let Some(installed) = &row.installed {
                let id = PackId::new(format!("country-{}-{}", row.id, installed.sha256))?;
                wanted.push(id.clone());
                let exists = self
                    .registry
                    .tiles
                    .read()
                    .map_err(|_| super::registry_poisoned())?
                    .contains_key(&id);
                if !exists {
                    if let Some(path) = self.maps.installed_path(row)? {
                        self.registry
                            .register_tiles(id, Arc::new(PreadSource::open(&path)?))?;
                    }
                }
            }
        }
        for entry in self.maps.imports()? {
            let id = PackId::new(entry.id.clone())?;
            wanted.push(id.clone());
            if !self
                .registry
                .tiles
                .read()
                .map_err(|_| super::registry_poisoned())?
                .contains_key(&id)
            {
                let path = self.maps.imported_path(&entry)?;
                self.registry
                    .register_tiles(id, Arc::new(PreadSource::open(&path)?))?;
            }
        }
        let stale: Vec<_> = self
            .registry
            .tiles
            .read()
            .map_err(|_| super::registry_poisoned())?
            .keys()
            .filter(|id| {
                ["country-mx-", "country-us-", "country-ca-", "mapfile-"]
                    .iter()
                    .any(|prefix| id.as_str().starts_with(prefix))
                    && !wanted.contains(id)
            })
            .cloned()
            .collect();
        for id in stale {
            self.registry.remove(&id)?;
        }
        self.maps.cleanup()?;
        Ok(())
    }
}
fn manager(state: &ServerState) -> Result<Arc<Manager>, Response> {
    state
        .maps
        .lock()
        .ok()
        .and_then(|manager| manager.clone())
        .ok_or_else(|| StatusCode::SERVICE_UNAVAILABLE.into_response())
}
fn failure(error: PackError) -> Response {
    let code = match error {
        PackError::NoSpace => "no_space",
        PackError::NotFound(_) => "not_found",
        PackError::Mismatch(_) => "busy",
        _ => "map_unavailable",
    };
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"error":code})),
    )
        .into_response()
}
pub(super) async fn list(State(state): State<Arc<ServerState>>) -> Response {
    let manager = match manager(&state) {
        Ok(manager) => manager,
        Err(reply) => return reply,
    };
    match tokio::task::spawn_blocking(move || {
        manager.sync()?;
        let imports = manager.maps.imports()?;
        let rows = manager.maps.list()?.into_iter().map(|row| serde_json::json!({"id":row.id,"epoch":row.epoch,"state":row.state,"zoom":row.zoom,
            "estimate":row.estimate,"completedBytes":row.completed_bytes,"downloadBytes":row.download_bytes,"outputBytes":row.output_bytes,
            "installed":row.installed.map(|map| serde_json::json!({"bytes":map.bytes,"zoom":map.zoom,"generation":map.sha256})),"error":row.error})).collect::<Vec<_>>();
        Ok::<_,PackError>(serde_json::json!({"rows":rows,"imports":imports}))
    }).await {
        Ok(Ok(data)) => Json(data).into_response(),
        Ok(Err(error)) => failure(error), Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: String,
    zoom: u8,
    estimate: bool,
}
pub(super) async fn start(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Request>,
) -> Response {
    let manager = match manager(&state) {
        Ok(manager) => manager,
        Err(reply) => return reply,
    };
    match tokio::task::spawn_blocking(move || {
        let ticket = manager
            .maps
            .prepare(&request.id, request.zoom, request.estimate)?;
        if let Err(error) = manager
            .dispatcher
            .dispatch(Arc::clone(&manager.maps), ticket.clone())
        {
            manager.maps.rejected(&ticket)?;
            return Err(error);
        }
        Ok(())
    })
    .await
    {
        Ok(Ok(())) => StatusCode::ACCEPTED.into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
pub(super) async fn control(
    State(state): State<Arc<ServerState>>,
    Path((id, action)): Path<(String, String)>,
) -> Response {
    let manager = match manager(&state) {
        Ok(manager) => manager,
        Err(reply) => return reply,
    };
    match tokio::task::spawn_blocking(move || {
        match action.as_str() {
            "pause" => manager.maps.control(&id, false)?,
            "cancel" => manager.maps.control(&id, true)?,
            "remove" => {
                if id.starts_with("mapfile-") {
                    manager.maps.remove_import(&id)?;
                } else {
                    manager.maps.remove(&id)?;
                }
            }
            _ => return Err(PackError::NotFound("Map action".into())),
        }
        manager.sync()
    })
    .await
    {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
