//! Opt-in app updates. Private local controls: the LAN content router never owns these routes.
use super::ServerState;
use crate::download::{
    fetch_pack_controlled_with_client, DownloadAction, DownloadOutcome, DownloadSpec,
};
use crate::pack::PackError;
use crate::update::{app_version, current_platform, Store};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

type Problem = (StatusCode, &'static str);
const UNAVAILABLE: Problem = (StatusCode::SERVICE_UNAVAILABLE, "update_unavailable");
const INVALID: Problem = (StatusCode::BAD_REQUEST, "invalid_update_settings");
const OFF: Problem = (StatusCode::CONFLICT, "updates_off");
const NONE: Problem = (StatusCode::CONFLICT, "no_update");

#[derive(Default)]
enum Checked {
    #[default]
    Never,
    UpToDate,
    Available,
    Failed(&'static str),
}

#[derive(Default)]
enum Download {
    #[default]
    Idle,
    Running {
        downloaded: u64,
        total: u64,
        cancel: Arc<AtomicBool>,
    },
    Done,
    Failed(&'static str),
}

#[derive(Default)]
struct Inner {
    checked: Checked,
    download: Download,
}

#[derive(Default)]
pub(super) struct Update {
    inner: Mutex<Inner>,
}

fn failure(error: &PackError) -> &'static str {
    match error {
        PackError::Checksum => "update_signature",
        PackError::Http(_) | PackError::Io(_) | PackError::NotFound(_) | PackError::Truncated => {
            "update_unreachable"
        }
        PackError::NoSpace => "update_no_space",
        _ => "update_invalid",
    }
}

fn client() -> Result<reqwest::blocking::Client, PackError> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| PackError::Http("update client initialization failed".into()))
}

/// Small, bounded GET for the manifest and its signature. The address is never supplied by the page.
fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, PackError> {
    let response = client()?
        .get(url)
        .send()
        .map_err(|error| PackError::Http(error.to_string()))?;
    if !response.status().is_success() {
        return Err(PackError::Http(format!("update source answered {}", response.status())));
    }
    let mut bytes = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PackError::Http(error.to_string()))?;
    if bytes.len() > limit {
        return Err(PackError::Mismatch("update file exceeds its size bound".into()));
    }
    Ok(bytes)
}

fn mode() -> &'static str {
    if cfg!(target_os = "android") {
        "install"
    } else if cfg!(target_os = "ios") {
        "link"
    } else {
        "apply"
    }
}

fn status(state: &ServerState) -> Result<Value, PackError> {
    let store = Store::new(&state.data_dir);
    let settings = store.settings()?;
    let installed = app_version()?;
    let platform = current_platform();
    let offer = match platform {
        Some(platform) => store.offer(&installed, platform).ok().flatten(),
        None => None,
    };
    let available = match (&offer, platform) {
        (Some((manifest, asset)), Some(platform)) => Some(json!({
            "version": manifest.version_text(),
            "published": manifest.published,
            "notes": manifest.notes,
            "page": manifest.page,
            "bytes": asset.bytes,
            "downloaded": store.staged(&installed, platform).unwrap_or(false),
            "mode": mode(),
        })),
        _ => None,
    };
    let inner = state.update.inner.lock().map_err(|_| PackError::Corrupt("update lock".into()))?;
    let (check, error) = match &inner.checked {
        Checked::Never => ("never", None),
        Checked::UpToDate => ("up_to_date", None),
        Checked::Available => ("available", None),
        Checked::Failed(code) => ("error", Some(*code)),
    };
    let download = match &inner.download {
        Download::Idle => json!({"state":"idle","downloaded":0,"total":0,"error":null}),
        Download::Running { downloaded, total, .. } => {
            json!({"state":"running","downloaded":downloaded,"total":total,"error":null})
        }
        Download::Done => json!({"state":"done","downloaded":0,"total":0,"error":null}),
        Download::Failed(code) => json!({"state":"failed","downloaded":0,"total":0,"error":code}),
    };
    Ok(json!({
        "supported": platform.is_some(),
        "platform": platform,
        "current": env!("CARGO_PKG_VERSION"),
        "enabled": settings.enabled,
        "source": settings.source,
        "public_key": settings.public_key,
        "check": check,
        "error": error,
        "available": available,
        "download": download,
    }))
}

async fn respond(
    state: Arc<ServerState>,
    operation: impl FnOnce(&Arc<ServerState>) -> Result<Value, Problem> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || operation(&state)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err((code, error))) => (code, Json(json!({"error":error}))).into_response(),
        Err(_) => (UNAVAILABLE.0, Json(json!({"error":UNAVAILABLE.1}))).into_response(),
    }
}

fn current(state: &ServerState) -> Result<Value, Problem> {
    status(state).map_err(|_| UNAVAILABLE)
}

pub(super) async fn get(State(state): State<Arc<ServerState>>) -> Response {
    respond(state, |state| current(state)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsBody {
    enabled: bool,
    source: String,
    public_key: String,
}

pub(super) async fn settings(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<SettingsBody>,
) -> Response {
    respond(state, move |state| {
        Store::new(&state.data_dir)
            .save_settings(body.enabled, &body.source, &body.public_key)
            .map_err(|error| match error {
                PackError::Mismatch(_) => INVALID,
                _ => UNAVAILABLE,
            })?;
        if let Ok(mut inner) = state.update.inner.lock() {
            inner.checked = Checked::Never;
        }
        current(state)
    })
    .await
}

pub(super) async fn check(State(state): State<Arc<ServerState>>) -> Response {
    respond(state, |state| {
        let store = Store::new(&state.data_dir);
        if !store.settings().map_err(|_| UNAVAILABLE)?.enabled {
            return Err(OFF);
        }
        let installed = app_version().map_err(|_| UNAVAILABLE)?;
        let outcome = store.check(&installed, fetch);
        if let Ok(mut inner) = state.update.inner.lock() {
            inner.checked = match &outcome {
                Ok(Some(_)) => Checked::Available,
                Ok(None) => Checked::UpToDate,
                Err(error) => Checked::Failed(failure(error)),
            };
        }
        current(state)
    })
    .await
}

fn run_download(state: &Arc<ServerState>, spec: DownloadSpec, cancel: Arc<AtomicBool>) {
    let progress_state = Arc::clone(state);
    let result = client().and_then(|client| {
        fetch_pack_controlled_with_client(
            &spec,
            &state.data_dir,
            &client,
            || {
                if cancel.load(Ordering::Acquire) {
                    DownloadAction::Cancel
                } else {
                    DownloadAction::Continue
                }
            },
            |progress| {
                if let Ok(mut inner) = progress_state.update.inner.lock() {
                    if let Download::Running { downloaded, total, .. } = &mut inner.download {
                        *downloaded = progress.downloaded_bytes;
                        *total = progress.total_bytes;
                    }
                }
                Ok(())
            },
        )
    });
    if let Ok(mut inner) = state.update.inner.lock() {
        inner.download = match result {
            Ok(DownloadOutcome::Complete(_)) => Download::Done,
            Ok(_) => Download::Idle,
            Err(error) => Download::Failed(failure(&error)),
        };
    }
}

/// Removes packages of other releases so a phone keeps at most one staged update.
fn clear_stale(state: &ServerState, keep: &str) {
    let folder = state.data_dir.join("packs/updates");
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let keep = keep.rsplit('/').next().unwrap_or(keep);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(keep) && entry.path().is_file() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub(super) async fn download(State(state): State<Arc<ServerState>>) -> Response {
    respond(state, |state| {
        let store = Store::new(&state.data_dir);
        let installed = app_version().map_err(|_| UNAVAILABLE)?;
        let platform = current_platform().ok_or(NONE)?;
        let (manifest, asset) = store
            .offer(&installed, platform)
            .map_err(|_| UNAVAILABLE)?
            .ok_or(NONE)?;
        let relative = Store::staged_name(&manifest, platform).map_err(|_| UNAVAILABLE)?;
        let cancel = {
            let mut inner = state.update.inner.lock().map_err(|_| UNAVAILABLE)?;
            if matches!(inner.download, Download::Running { .. }) {
                drop(inner);
                return current(state);
            }
            let cancel = Arc::new(AtomicBool::new(false));
            inner.download = Download::Running {
                downloaded: 0,
                total: asset.bytes,
                cancel: Arc::clone(&cancel),
            };
            cancel
        };
        clear_stale(state, &relative);
        let spec = DownloadSpec {
            id: "app-update".into(),
            url: Some(asset.url),
            bytes: asset.bytes,
            sha256: asset.sha256,
            path: relative,
        };
        let worker_state = Arc::clone(state);
        let started = std::thread::Builder::new()
            .name("app-update-download".into())
            .spawn(move || run_download(&worker_state, spec, cancel));
        if started.is_err() {
            if let Ok(mut inner) = state.update.inner.lock() {
                inner.download = Download::Failed("update_unavailable");
            }
        }
        current(state)
    })
    .await
}

pub(super) async fn cancel(State(state): State<Arc<ServerState>>) -> Response {
    respond(state, |state| {
        if let Ok(inner) = state.update.inner.lock() {
            if let Download::Running { cancel, .. } = &inner.download {
                cancel.store(true, Ordering::Release);
            }
        }
        current(state)
    })
    .await
}
