//! Private element-scoped grants share the positioned range streamer; never mounted on LAN.
use super::ServerState;
use crate::{
    music::{audio_source, MusicError},
    pack::{PackError, PackSource},
    video::{self, VideoError},
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Audio,
    Video,
    Image,
    Pdf,
}
impl Kind {
    fn destination(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Image => "image",
            Self::Pdf => "empty",
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Image => 128,
            Self::Pdf => 4,
            _ => 32,
        }
    }
}
struct Grant {
    token: String,
    source: Arc<dyn PackSource>,
    mime: &'static str,
    kind: Kind,
    id: String,
    title: String,
    sequence: Option<u64>,
}
#[derive(Default)]
pub(super) struct Grants(Mutex<VecDeque<Grant>>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: String,
}
pub(super) async fn audio(state: State<Arc<ServerState>>, request: Json<Request>) -> Response {
    authorize(state.0, request.0, Kind::Audio).await
}
pub(super) async fn video(state: State<Arc<ServerState>>, request: Json<Request>) -> Response {
    authorize(state.0, request.0, Kind::Video).await
}
pub(super) async fn poster(state: State<Arc<ServerState>>, request: Json<Request>) -> Response {
    authorize(state.0, request.0, Kind::Image).await
}
pub(super) async fn pdf(state: State<Arc<ServerState>>, request: Json<Request>) -> Response {
    authorize(state.0, request.0, Kind::Pdf).await
}
fn pdf_source(
    root: &std::path::Path,
    id: &str,
) -> Result<(crate::media::MediaFile, Arc<dyn PackSource>, &'static str), VideoError> {
    use crate::media::{MediaError, MediaStore};
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(VideoError::Invalid);
    }
    let (file, source) = MediaStore::open(root)
        .and_then(|store| store.open_file(id))
        .map_err(|problem| match problem {
            MediaError::NotFound => VideoError::Missing,
            MediaError::Invalid => VideoError::Invalid,
            MediaError::Pack(problem) => VideoError::Pack(problem),
            _ => VideoError::Pack(PackError::Corrupt("Managed document catalog".into())),
        })?;
    if !file.name.to_ascii_lowercase().ends_with(".pdf")
        || !(8..=512 * 1024 * 1024).contains(&file.bytes)
    {
        return Err(VideoError::Unsupported);
    }
    let mut prefix = [0; 8];
    let mut filled = 0;
    while filled < prefix.len() {
        let count = source.read_at(filled as u64, &mut prefix[filled..])?;
        if count == 0 {
            return Err(PackError::Truncated.into());
        }
        if count > prefix.len() - filled {
            return Err(PackError::Corrupt("Invalid document read length".into()).into());
        }
        filled += count;
    }
    if !prefix.starts_with(b"%PDF-")
        || !matches!(prefix[5], b'1' | b'2')
        || prefix[6] != b'.'
        || !prefix[7].is_ascii_digit()
    {
        return Err(VideoError::Unsupported);
    }
    Ok((file, source, "application/pdf"))
}
fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"media_unavailable"})),
    )
        .into_response()
}
async fn authorize(state: Arc<ServerState>, request: Request, kind: Kind) -> Response {
    let root = state.data_dir.clone();
    let opened = tokio::task::spawn_blocking(move || {
        if kind == Kind::Pdf {
            pdf_source(&root, &request.id)
        } else if kind == Kind::Audio {
            audio_source(&root, &request.id).map_err(|problem| match problem {
                MusicError::Invalid => VideoError::Invalid,
                MusicError::Missing => VideoError::Missing,
                MusicError::Unsupported => VideoError::Unsupported,
                MusicError::Pack(problem) => VideoError::Pack(problem),
                _ => VideoError::Pack(PackError::Corrupt("Audio catalog".into())),
            })
        } else {
            video::source(&root, &request.id, kind == Kind::Image)
        }
    })
    .await;
    let (file, source, mime) = match opened {
        Ok(Ok(value)) => value,
        Ok(Err(problem)) => {
            let (status, code) = match problem {
                VideoError::Invalid => (StatusCode::BAD_REQUEST, "invalid_media"),
                VideoError::Missing => (StatusCode::NOT_FOUND, "media_missing"),
                VideoError::Unsupported => {
                    (StatusCode::UNSUPPORTED_MEDIA_TYPE, "media_unsupported")
                }
                _ => return unavailable(),
            };
            return (status, Json(json!({"error":code}))).into_response();
        }
        Err(_) => return unavailable(),
    };
    let mut random = [0; 32];
    if getrandom::fill(&mut random).is_err() {
        return unavailable();
    }
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let Ok(mut grants) = state.streams.0.lock() else {
        return unavailable();
    };
    // Browsing covers must never evict a playing Music/Video capability.
    if grants.iter().filter(|grant| grant.kind == kind).count() >= kind.limit() {
        if let Some(index) = grants.iter().position(|grant| grant.kind == kind) {
            grants.remove(index);
        }
    }
    grants.push_back(Grant {
        token: token.clone(),
        source,
        mime,
        kind,
        id: file.id,
        title: file.name,
        sequence: None,
    });
    let url = if kind == Kind::Pdf {
        format!("/api/documents/content/{token}")
    } else {
        format!("/media/{token}")
    };
    Json(json!({"url":url})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PositionRequest {
    token: String,
    sequence: u64,
    seconds: f64,
}
pub(super) async fn position(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<PositionRequest>,
) -> Response {
    if request.token.len() != 64
        || !request
            .token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !request.seconds.is_finite()
        || !(0.0..=1_000_000_000.0).contains(&request.seconds)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, PackError> {
        use crate::reading::{Destination, Position, ProgressStore, ReadingEntry};
        let mut grants = state
            .streams
            .0
            .lock()
            .map_err(|_| PackError::Corrupt("Playback grants".into()))?;
        let Some(index) = grants.iter().position(|grant| {
            grant.kind == Kind::Video
                && bool::from(grant.token.as_bytes().ct_eq(request.token.as_bytes()))
        }) else {
            return Ok(false);
        };
        let grant = &grants[index];
        if grant.sequence.is_some_and(|last| request.sequence <= last)
            || grants
                .range(index + 1..)
                .any(|newer| newer.kind == Kind::Video && newer.id == grant.id)
        {
            return Ok(false);
        }
        // Keep the grant fence through the durable write, so a reopened player wins.
        ProgressStore::open(&state.data_dir)?.save(ReadingEntry {
            destination: Destination::Media {
                id: grant.id.clone(),
            },
            position: Position::Time {
                seconds: request.seconds,
            },
            title: grant.title.clone(),
            bookmarked: false,
        })?;
        grants[index].sequence = Some(request.sequence);
        Ok(true)
    })
    .await;
    match result {
        Ok(Ok(accepted)) => Json(json!({"accepted":accepted})).into_response(),
        _ => unavailable(),
    }
}
pub(super) async fn serve(
    State(state): State<Arc<ServerState>>,
    Path(token): Path<String>,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let captured = state.streams.0.lock().ok().and_then(|grants| {
        grants
            .iter()
            .find(|grant| bool::from(grant.token.as_bytes().ct_eq(token.as_bytes())))
            .map(|grant| (Arc::clone(&grant.source), grant.mime, grant.kind))
    });
    let Some((source, mime, kind)) = captured else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // PDF fetches must never alias the element endpoint and bypass API CSRF.
    if (kind == Kind::Pdf) != uri.path().starts_with("/api/documents/content/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers
        .get("sec-fetch-dest")
        .and_then(|value| value.to_str().ok())
        != Some(kind.destination())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = super::ranges::serve_source(source, method, headers).await;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
