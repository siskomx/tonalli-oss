//! Private bounded conversation transport; inference and cancellation stay in Core AI.
mod grounding;
pub(super) mod history;
pub(super) mod models;
mod stream;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

use axum::extract::{rejection::JsonRejection, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::ServerState;
use crate::ai::{AiHandle, ModelInfo, ModelKind};
use crate::pack::PackError;
use stream::{Control, Lease};

#[derive(Default)]
pub(super) struct ChatState {
    slot: Mutex<Slot>,
}
#[derive(Default)]
struct Slot {
    model: Option<(String, Arc<AiHandle>)>,
    owned_model: bool,
    loading: Option<Arc<models::Loading>>,
    legacy_root: Option<std::path::PathBuf>,
    model_error: Option<&'static str>,
    active: Option<Arc<Control>>,
    stopped: bool,
}

fn poisoned() -> PackError {
    PackError::Corrupt("private AI transport lock poisoned".into())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

impl ChatState {
    pub(super) fn attach(&self, id: String, handle: Arc<AiHandle>) -> Result<(), PackError> {
        if !identifier(&id) {
            return Err(PackError::Mismatch("invalid AI model id".into()));
        }
        if handle.info()?.kind != ModelKind::Generation {
            return Err(PackError::Mismatch(
                "chat requires a generation model".into(),
            ));
        }
        let mut slot = self.slot.lock().map_err(|_| poisoned())?;
        if slot.stopped {
            return Err(PackError::NotFound("stopped loopback host".into()));
        }
        if slot.model.is_some() || slot.active.is_some() || slot.loading.is_some() {
            return Err(PackError::Mismatch("private AI attachment is busy".into()));
        }
        slot.model = Some((id, handle));
        slot.owned_model = false;
        Ok(())
    }

    fn snapshot(&self) -> Result<(String, Arc<AiHandle>, ModelInfo), PackError> {
        let (id, handle) = self
            .slot
            .lock()
            .map_err(|_| poisoned())?
            .model
            .clone()
            .ok_or_else(|| PackError::NotFound("no attached AI model".into()))?;
        let info = handle.info()?;
        Ok((id, handle, info))
    }

    pub(super) fn available(&self) -> bool {
        self.snapshot().is_ok()
    }

    pub(super) fn detach(&self, stop: bool) -> Result<(), PackError> {
        self.detach_model(stop, false)
    }

    fn detach_model(&self, stop: bool, close_aliases: bool) -> Result<(), PackError> {
        let (model, active, owned) = {
            let mut slot = self.slot.lock().map_err(|_| poisoned())?;
            slot.stopped |= stop;
            if let Some(loading) = &slot.loading {
                loading.cancel();
            }
            slot.model_error = None;
            let owned = std::mem::take(&mut slot.owned_model);
            (slot.model.take(), slot.active.clone(), owned)
        };
        let result = active.map_or(Ok(()), |active| active.cancel());
        let closed = if owned || close_aliases {
            model.as_ref().map(|(_, handle)| handle.close()).transpose()
        } else {
            Ok(None)
        };
        drop(model);
        result?;
        closed.map(|_| ())
    }

    pub(super) fn cancel_active(&self) -> Result<(), PackError> {
        let active = self.slot.lock().map_err(|_| poisoned())?.active.clone();
        active.map_or(Ok(()), |active| active.cancel())
    }

    fn reserve(
        self: &Arc<Self>,
        request: &ChatRequest,
    ) -> Result<(Arc<AiHandle>, Lease), &'static str> {
        let (id, handle, _) = self.snapshot().map_err(|_| "ai_unavailable")?;
        if id != request.model {
            return Err("ai_model_mismatch");
        }
        let control = Arc::new(Control::new(request.request_id.clone()));
        let mut slot = self.slot.lock().map_err(|_| "ai_unavailable")?;
        if slot.stopped
            || !slot
                .model
                .as_ref()
                .is_some_and(|(_, current)| Arc::ptr_eq(&handle, current))
        {
            return Err("ai_unavailable");
        }
        if slot.active.is_some() {
            return Err("ai_busy");
        }
        slot.active = Some(Arc::clone(&control));
        drop(slot);
        Ok((
            handle,
            Lease {
                owner: Arc::clone(self),
                control,
            },
        ))
    }

    fn cancel_request(&self, id: &str) -> Result<bool, PackError> {
        let active = self
            .slot
            .lock()
            .map_err(|_| poisoned())?
            .active
            .clone()
            .filter(|request| request.request_id == id);
        if let Some(active) = active {
            active.cancel()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatRequest {
    request_id: String,
    model: String,
    messages: Vec<crate::ai::ChatMessage>,
    max_tokens: u32,
    pack: Option<String>,
}
impl ChatRequest {
    fn valid(&self) -> bool {
        identifier(&self.request_id)
            && identifier(&self.model)
            && (1..=128).contains(&self.max_tokens)
            && crate::ai::validate_messages(&self.messages).is_ok()
            && self
                .messages
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>()
                <= 65_536 - "\n/no_think".len()
            && self.pack.as_ref().is_none_or(|pack| {
                super::PackId::new(pack.clone()).is_ok()
                    && self
                        .messages
                        .last()
                        .is_some_and(|message| message.content.len() <= 4096)
            })
    }

    fn conversation(mut self) -> Vec<crate::ai::ChatMessage> {
        // Current explicitly attached catalog is Qwen3. Preserve the working
        // non-thinking convention on the current user turn; roles stay native.
        if let Some(message) = self.messages.last_mut() {
            message.content.push_str("\n/no_think");
        }
        self.messages
    }
}

fn failure(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}

pub(super) async fn model(State(state): State<Arc<ServerState>>) -> Response {
    match state.ai.snapshot() {
        Ok((id, _, info)) => Json(json!({"model":id, "kind":"generation", "file_bytes":info.file_bytes,
            "training_context_tokens":info.training_context_tokens, "embedding_dimensions":info.embedding_dimensions,
            "estimated_reserved_bytes":info.estimated_reserved_bytes})).into_response(),
        Err(_) => failure(StatusCode::SERVICE_UNAVAILABLE, "ai_unavailable"),
    }
}

pub(super) async fn chat(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<ChatRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    };
    if !request.valid() {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    }
    let (handle, lease) = match state.ai.reserve(&request) {
        Ok(reserved) => reserved,
        Err(code) => {
            return failure(
                if code == "ai_unavailable" {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::CONFLICT
                },
                code,
            )
        }
    };
    let max_tokens = request.max_tokens;
    if request.pack.is_some() {
        grounding::start(state.registry.clone(), handle, lease, request).await
    } else {
        stream::start(handle, lease, request.conversation(), max_tokens, None).await
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelRequest {
    request_id: String,
}

pub(super) async fn cancel(
    State(state): State<Arc<ServerState>>,
    body: Result<Json<CancelRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    };
    if !identifier(&request.request_id) {
        return failure(StatusCode::BAD_REQUEST, "ai_invalid_request");
    }
    match state.ai.cancel_request(&request.request_id) {
        Ok(true) => Json(json!({"request_id":request.request_id,"cancellation_requested":true}))
            .into_response(),
        Ok(false) => failure(StatusCode::NOT_FOUND, "ai_request_not_found"),
        Err(_) => failure(StatusCode::SERVICE_UNAVAILABLE, "ai_cancel_failed"),
    }
}
