//! Native-only Music metadata and native playback controls.
use super::ServerState;
use crate::{
    music::{MusicError, MusicStore, PlaybackAction, PlaybackStatus, PlaybackTrack, PlaylistItem},
    pack::PackError,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Music {
    engine: Mutex<Option<Arc<dyn MusicControl>>>,
}
pub trait MusicControl: Send + Sync {
    fn status(&self) -> Result<PlaybackStatus, PackError>;
    fn control(&self, action: PlaybackAction, track: PlaybackTrack) -> Result<(), PackError>;
}
impl Music {
    pub(super) fn attach(&self, engine: Arc<dyn MusicControl>) -> Result<(), PackError> {
        let mut slot = self
            .engine
            .lock()
            .map_err(|_| PackError::Corrupt("Music engine lock".into()))?;
        if slot.is_some() {
            return Err(PackError::Mismatch("Music engine already attached".into()));
        }
        *slot = Some(engine);
        Ok(())
    }
    fn engine(&self) -> Result<Option<Arc<dyn MusicControl>>, PackError> {
        Ok(self
            .engine
            .lock()
            .map_err(|_| PackError::Corrupt("Music engine lock".into()))?
            .clone())
    }
}
fn error(error: MusicError) -> Response {
    let (status, code) = match error {
        MusicError::Invalid => (StatusCode::BAD_REQUEST, "invalid_music"),
        MusicError::Conflict => (StatusCode::CONFLICT, "music_changed"),
        MusicError::Missing => (StatusCode::NOT_FOUND, "track_missing"),
        MusicError::Unsupported => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "audio_unsupported"),
        MusicError::Pack(PackError::NoSpace) => (StatusCode::INSUFFICIENT_STORAGE, "storage_full"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "music_unavailable"),
    };
    (status, Json(json!({"error":code}))).into_response()
}
async fn work(
    state: Arc<ServerState>,
    operation: impl FnOnce(MusicStore) -> Result<Value, MusicError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || operation(MusicStore::open(&state.data_dir)?)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(problem)) => error(problem),
        Err(_) => error(MusicError::Corrupt),
    }
}
pub(super) async fn listing(State(state): State<Arc<ServerState>>) -> Response {
    let native = match state.music.engine() {
        Ok(value) => value.is_some(),
        Err(_) => return error(MusicError::Corrupt),
    };
    work(state, move |store| {
        Ok(json!({"state":store.snapshot()?,"native":native}))
    })
    .await
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Command {
    CreatePlaylist {
        revision: u64,
        name: String,
    },
    ReplacePlaylist {
        revision: u64,
        id: String,
        name: String,
        items: Vec<PlaylistItem>,
    },
    DeletePlaylist {
        revision: u64,
        id: String,
        confirmed: bool,
    },
    Queue {
        revision: u64,
        tracks: Vec<String>,
        current: Option<u32>,
    },
    PlayPlaylist {
        revision: u64,
        id: String,
    },
    Step {
        generation: u64,
        delta: i32,
    },
    Seek {
        generation: u64,
        seconds: f64,
    },
    Position {
        generation: u64,
        seconds: f64,
    },
}
pub(super) async fn command(
    State(state): State<Arc<ServerState>>,
    Json(command): Json<Command>,
) -> Response {
    let owner = Arc::clone(&state);
    work(state, move |store| {
        let before = store.snapshot()?;
        let explicit = match &command {
            Command::PlayPlaylist { .. } => Some(PlaybackAction::Play),
            Command::Seek { .. } => Some(PlaybackAction::Seek),
            _ => None,
        };
        let mut accepted = true;
        match command {
            Command::CreatePlaylist { revision, name } => {
                store.create_playlist(revision, &name)?;
            }
            Command::ReplacePlaylist {
                revision,
                id,
                name,
                items,
            } => store.replace_playlist(revision, &id, &name, items)?,
            Command::DeletePlaylist {
                revision,
                id,
                confirmed,
            } => store.delete_playlist(revision, &id, confirmed)?,
            Command::Queue {
                revision,
                tracks,
                current,
            } => {
                store.set_queue(revision, tracks, current)?;
            }
            Command::PlayPlaylist { revision, id } => {
                store.play_playlist(revision, &id)?;
            }
            Command::Step { generation, delta } => {
                store.step(generation, delta)?;
            }
            Command::Seek {
                generation,
                seconds,
            } => {
                store.seek(generation, seconds)?;
            }
            Command::Position {
                generation,
                seconds,
            } => {
                accepted = store.update_position(generation, seconds)?;
            }
        }
        let after = store.snapshot()?;
        if let Some(engine) = owner.music.engine()? {
            let action = explicit.or_else(|| {
                (before.queue.generation != after.queue.generation).then_some(PlaybackAction::Load)
            });
            if let Some(mut action) = action {
                if action == PlaybackAction::Load && engine.status()?.playing {
                    action = PlaybackAction::Play;
                }
                engine.control(action, PlaybackTrack::from(&after.queue))?;
            }
        }
        Ok(json!({"state":after,"accepted":accepted}))
    })
    .await
}

pub(super) async fn playback_status(State(state): State<Arc<ServerState>>) -> Response {
    match tokio::task::spawn_blocking(move || -> Result<PlaybackStatus, PackError> {
        state
            .music
            .engine()?
            .map_or_else(|| Ok(PlaybackStatus::default()), |engine| engine.status())
    })
    .await
    {
        Ok(Ok(status)) => Json(status).into_response(),
        _ => error(MusicError::Corrupt),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlaybackRequest {
    action: PlaybackAction,
}
pub(super) async fn playback_control(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<PlaybackRequest>,
) -> Response {
    work(state.clone(), move |store| {
        let engine = state.music.engine()?.ok_or(MusicError::Unsupported)?;
        let track = if matches!(request.action, PlaybackAction::Pause | PlaybackAction::Stop) {
            PlaybackTrack::default()
        } else {
            let mut queue = store.snapshot()?.queue;
            if queue.current.is_none()
                && request.action == PlaybackAction::Play
                && !queue.tracks.is_empty()
            {
                queue = store.step(queue.generation, 1)?;
            }
            if queue.current.is_none() {
                return Err(MusicError::Missing);
            }
            PlaybackTrack::from(&queue)
        };
        engine.control(request.action, track)?;
        Ok(json!({"accepted":true}))
    })
    .await
}
