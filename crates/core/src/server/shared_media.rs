//! A read-only, selected managed-media snapshot, without private stores or disk paths.
use super::ContentState;
use crate::{
    media::{MediaError, MediaFile, MediaStore},
    music::{self, MusicError},
    pack::{PackError, PackSource},
    video::{self, VideoError},
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::{
    collections::HashSet,
    path::Path as DiskPath,
    sync::{Arc, RwLock},
};

pub(super) const MAX_SHARED_FILES: usize = 1024;
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct SharedMedia {
    pub(super) files: Vec<SharedFile>,
    hidden_files: RwLock<HashSet<String>>,
    hidden_packs: RwLock<HashSet<String>>,
}
pub(super) struct SharedFile {
    pub(super) file: MediaFile,
    pub(super) source: Arc<dyn PackSource>,
    pub(super) mime: Option<&'static str>,
}
fn media_error(problem: MediaError) -> PackError {
    match problem {
        MediaError::Pack(problem) => problem,
        _ => PackError::Mismatch(problem.to_string()),
    }
}
impl SharedMedia {
    pub fn capture(root: &DiskPath, ids: &[String]) -> Result<Self, PackError> {
        if ids.len() > MAX_SHARED_FILES || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
            return Err(PackError::Mismatch(
                "Choose at most 1024 distinct managed files".into(),
            ));
        }
        if ids.is_empty() {
            return Ok(Self::default());
        }
        let store = MediaStore::open(root).map_err(media_error)?;
        let mut files = Vec::with_capacity(ids.len());
        for id in ids {
            let file = store.get(id).map_err(media_error)?;
            let extension = file
                .name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let image = matches!(
                extension.as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "avif"
            );
            let video = matches!(
                extension.as_str(),
                "mp4" | "m4v" | "webm" | "mkv" | "avi" | "mov"
            );
            let opened = if image || video {
                match video::source(root, id, image) {
                    Ok(value) => Some(value),
                    Err(VideoError::Unsupported) => None,
                    Err(VideoError::Pack(problem)) => return Err(problem),
                    Err(problem) => return Err(PackError::Mismatch(problem.to_string())),
                }
            } else if music::audio_name(&file.name) {
                match music::audio_source(root, id) {
                    Ok(value) => Some(value),
                    Err(MusicError::Unsupported) => None,
                    Err(MusicError::Pack(problem)) => return Err(problem),
                    Err(problem) => return Err(PackError::Mismatch(problem.to_string())),
                }
            } else {
                None
            };
            let (current, source, mime) = match opened {
                Some((current, source, mime)) => (current, source, Some(mime)),
                None => {
                    let (current, source) = store.open_file(id).map_err(media_error)?;
                    (current, source, None)
                }
            };
            if current.version != file.version || current.sha256 != file.sha256 {
                return Err(PackError::Mismatch(
                    "The selected file changed; choose it again".into(),
                ));
            }
            files.push(SharedFile {
                file: current,
                source,
                mime,
            });
        }
        Ok(Self {
            files,
            ..Self::default()
        })
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
    /// Stops sharing books, maps and files on a running share. Removal only: nothing hidden is
    /// shared again until sharing restarts with a new selection, and unknown ids change nothing.
    pub fn hide(&self, packs: &[String], files: &[String]) -> Result<(), PackError> {
        if packs.len() > 128
            || files.len() > MAX_SHARED_FILES
            || packs.iter().chain(files).any(|id| id.len() > 128)
        {
            return Err(PackError::Mismatch("Too many items to remove".into()));
        }
        let mut hidden = self
            .hidden_packs
            .write()
            .map_err(|_| super::registry_poisoned())?;
        hidden.extend(packs.iter().cloned());
        let mut hidden = self
            .hidden_files
            .write()
            .map_err(|_| super::registry_poisoned())?;
        hidden.extend(files.iter().cloned());
        Ok(())
    }
    // A poisoned lock counts as hidden, so a failure can only share less.
    pub(super) fn pack_hidden(&self, id: &super::PackId) -> bool {
        self.hidden_packs
            .read()
            .map_or(true, |hidden| hidden.contains(id.as_str()))
    }
    pub(super) fn pack_hidden_name(&self, id: &str) -> bool {
        self.hidden_packs
            .read()
            .map_or(true, |hidden| hidden.contains(id))
    }
    fn file_hidden(&self, id: &str) -> bool {
        self.hidden_files
            .read()
            .map_or(true, |hidden| hidden.contains(id))
    }
    pub(crate) fn metadata(&self) -> Vec<MediaFile> {
        self.files
            .iter()
            .filter(|entry| !self.file_hidden(&entry.file.id))
            .map(|entry| entry.file.clone())
            .collect()
    }
    fn find(&self, id: &str) -> Option<&SharedFile> {
        self.files
            .iter()
            .find(|entry| entry.file.id == id && !self.file_hidden(id))
    }
}
pub(super) async fn listing(State(state): State<Arc<ContentState>>) -> Json<serde_json::Value> {
    Json(
        json!({"files":state.media.files.iter().filter(|entry|!state.media.file_hidden(&entry.file.id)).map(|entry|json!({"file":entry.file,"inline_mime":entry.mime})).collect::<Vec<_>>()}),
    )
}
pub(super) async fn content(
    State(state): State<Arc<ContentState>>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = state.media.find(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(mime) = entry.mime else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let destination = mime.split('/').next().unwrap_or("");
    if headers
        .get("sec-fetch-dest")
        .and_then(|value| value.to_str().ok())
        != Some(destination)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    media_response(entry, method, headers, mime, false).await
}
pub(super) async fn download(
    State(state): State<Arc<ContentState>>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = state.media.find(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    media_response(entry, method, headers, "application/octet-stream", true).await
}
async fn media_response(
    entry: &SharedFile,
    method: Method,
    headers: HeaderMap,
    mime: &'static str,
    attachment: bool,
) -> Response {
    let mut response =
        super::ranges::serve_source(Arc::clone(&entry.source), method, headers).await;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if attachment {
        let name = percent_encoding::utf8_percent_encode(
            &entry.file.name,
            percent_encoding::NON_ALPHANUMERIC,
        );
        let Ok(value) = HeaderValue::from_str(&format!(
            "attachment; filename=download; filename*=UTF-8''{name}"
        )) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}
pub(super) async fn text(
    State(state): State<Arc<ContentState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(entry) = state.media.find(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !entry.file.editable() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let source = Arc::clone(&entry.source);
    let value = tokio::task::spawn_blocking(move || -> Result<String, PackError> {
        if source.len() > crate::media::MAX_TEXT_BYTES {
            return Err(PackError::Mismatch("Text too large".into()));
        }
        let mut bytes = vec![0; usize::try_from(source.len()).map_err(|_| PackError::Truncated)?];
        let mut filled = 0;
        while filled < bytes.len() {
            let count = source.read_at(filled as u64, &mut bytes[filled..])?;
            if count == 0 {
                return Err(PackError::Truncated);
            }
            if count > bytes.len() - filled {
                return Err(PackError::Corrupt("Media read length".into()));
            }
            filled += count;
        }
        if bytes.contains(&0) {
            return Err(PackError::Mismatch("Not readable text".into()));
        }
        String::from_utf8(bytes).map_err(|_| PackError::Mismatch("Not UTF-8 text".into()))
    })
    .await;
    match value {
        Ok(Ok(text)) => Json(json!({"text":text})).into_response(),
        Ok(Err(PackError::Mismatch(_))) => StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
