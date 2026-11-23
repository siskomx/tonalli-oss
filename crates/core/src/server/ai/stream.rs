use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::mpsc;

use super::{failure, ChatState};
use crate::ai::{self, AiHandle, GenId, GenerationEnd, GenerationEvent, TokenSink};
use crate::pack::PackError;

#[derive(Default)]
struct Progress {
    id: Option<GenId>,
    cancelled: bool,
    terminal: bool,
}

/// Holds only request identity/flags, never a model, pack or native resource.
pub(super) struct Control {
    pub(super) request_id: String,
    progress: Mutex<Progress>,
}
impl Control {
    pub(super) fn new(request_id: String) -> Self {
        Self {
            request_id,
            progress: Mutex::new(Progress::default()),
        }
    }
    fn bind(&self, id: GenId) -> Result<Option<GenId>, PackError> {
        let mut state = self.progress.lock().map_err(|_| super::poisoned())?;
        if state.id.is_some_and(|old| old != id) {
            return Err(PackError::Corrupt("AI stream generation id changed".into()));
        }
        state.id = Some(id);
        Ok(state.cancelled.then_some(id))
    }
    fn cancellation_id(&self) -> Result<Option<GenId>, PackError> {
        let mut state = self.progress.lock().map_err(|_| super::poisoned())?;
        state.cancelled = true;
        Ok(state.id)
    }
    pub(super) fn cancel(&self) -> Result<(), PackError> {
        if let Some(id) = self.cancellation_id()? {
            ai::cancel(id)?;
        }
        Ok(())
    }
    fn accept(&self, id: GenId, terminal: bool) -> Result<bool, PackError> {
        let mut state = self.progress.lock().map_err(|_| super::poisoned())?;
        if state.id != Some(id) {
            return Err(PackError::Corrupt("AI stream event id mismatch".into()));
        }
        if state.terminal {
            return Ok(false);
        }
        if terminal {
            state.terminal = true;
        }
        Ok(terminal || !state.cancelled)
    }
}

pub(super) struct Lease {
    pub(super) owner: Arc<ChatState>,
    pub(super) control: Arc<Control>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.control.cancel();
        if let Ok(mut slot) = self.owner.slot.lock() {
            if slot
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.control))
            {
                slot.active.take();
            }
        }
    }
}

struct Wire {
    bytes: String,
    terminal: bool,
}
fn wire(value: serde_json::Value, terminal: bool) -> Wire {
    Wire {
        bytes: format!("data: {value}\n\n"),
        terminal,
    }
}
fn stream_error(code: &'static str) -> Wire {
    // Stream termination is not a fabricated native Finished event.
    wire(
        json!({"error":true,"code":code,"message":"AI request did not complete."}),
        true,
    )
}

struct Forwarder {
    control: Arc<Control>,
    send: mpsc::Sender<Wire>,
    grounding: Option<Mutex<(super::grounding::Grounding, String)>>,
}
impl TokenSink for Forwarder {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
        let message = match event {
            GenerationEvent::Started { id } => {
                if let Some(id) = self.control.bind(id)? {
                    ai::cancel(id)?;
                }
                return Ok(());
            }
            GenerationEvent::Text { id, text } => {
                if !self.control.accept(id, false)? {
                    return Ok(());
                }
                if let Some(grounding) = &self.grounding {
                    let mut captured = grounding.lock().map_err(|_| super::poisoned())?;
                    if captured.1.len().saturating_add(text.len()) > 128 * 1024 {
                        return Err(PackError::Mismatch("AI answer exceeds byte bound".into()));
                    }
                    captured.1.push_str(&text);
                }
                wire(json!({"message":{"content":text},"done":false}), false)
            }
            GenerationEvent::Finished { id, reason, .. } => {
                if !self.control.accept(id, true)? {
                    return Ok(());
                }
                match reason {
                    GenerationEnd::Failed => stream_error("ai_generation_failed"),
                    reason => {
                        let mut value = json!({"message":{"content":""},"done":true,"done_reason":match reason {
                            GenerationEnd::Eos => "stop", GenerationEnd::Limit => "length",
                            GenerationEnd::Cancelled => "cancelled", GenerationEnd::Failed => "error",
                        }});
                        if let Some(grounding) = &self.grounding {
                            let captured = grounding.lock().map_err(|_| super::poisoned())?;
                            value["grounding"] = captured.0.metadata(captured.1.clone(), reason)?;
                        }
                        wire(value, true)
                    }
                }
            }
        };
        // No network writes, blocking sends or native calls under callback locks.
        self.send
            .try_send(message)
            .map_err(|_| PackError::Io("AI response consumer unavailable".into()))
    }
}

struct Reading {
    _lease: Lease,
    receive: mpsc::Receiver<Wire>,
    deadline: tokio::time::Instant,
    ended: bool,
}

pub(super) async fn start(
    handle: Arc<AiHandle>,
    lease: Lease,
    messages: Vec<ai::ChatMessage>,
    max_tokens: u32,
    grounding: Option<super::grounding::Grounding>,
) -> Response {
    let (send, receive) = mpsc::channel(32);
    let sink = Arc::new(Forwarder {
        control: Arc::clone(&lease.control),
        send,
        grounding: grounding.map(|grounding| Mutex::new((grounding, String::new()))),
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    // Handler/response Lease drop latches cancellation even before generate returns.
    let starting =
        tokio::task::spawn_blocking(move || ai::generate_chat(handle, messages, max_tokens, sink));
    let started = match tokio::time::timeout_at(deadline, starting).await {
        Ok(result) => result,
        Err(_) => return failure(StatusCode::GATEWAY_TIMEOUT, "ai_deadline"),
    };
    match started {
        Ok(Ok(id)) => match lease.control.bind(id) {
            Ok(Some(id)) => {
                if ai::cancel(id).is_err() {
                    return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_cancel_failed");
                }
            }
            Ok(None) => {}
            Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "ai_start_failed"),
        },
        Ok(Err(PackError::NotFound(_))) => {
            return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_unavailable")
        }
        Ok(Err(PackError::Mismatch(_))) => {
            return failure(StatusCode::CONFLICT, "ai_request_refused")
        }
        _ => return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_start_failed"),
    }
    let state = Reading {
        _lease: lease,
        receive,
        deadline,
        ended: false,
    };
    let stream = futures_util::stream::unfold(state, |mut state| async move {
        if state.ended {
            return None;
        }
        let message = match tokio::time::timeout_at(state.deadline, state.receive.recv()).await {
            Ok(Some(message)) => message,
            Ok(None) => stream_error("ai_stream_incomplete"),
            Err(_) => {
                let _ = state._lease.control.cancel();
                stream_error("ai_deadline")
            }
        };
        state.ended = message.terminal;
        Some((Ok::<_, Infallible>(message.bytes), state))
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_started_and_returned_id_stays_latched() {
        let control = Control::new("first".into());
        assert_eq!(control.cancellation_id().unwrap(), None);
        assert_eq!(control.bind(17).unwrap(), Some(17));
        assert_eq!(control.bind(17).unwrap(), Some(17));
        assert!(!control.accept(17, false).unwrap());
        assert!(control.accept(17, true).unwrap());
        assert!(!control.accept(17, true).unwrap());
        assert!(control.bind(18).is_err());
    }

    #[test]
    fn disconnect_before_native_id_and_stale_disconnect_are_request_scoped() {
        let owner = Arc::new(ChatState::default());
        let old = Arc::new(Control::new("old".into()));
        owner.slot.lock().unwrap().active = Some(Arc::clone(&old));
        drop(Lease {
            owner: Arc::clone(&owner),
            control: Arc::clone(&old),
        });
        assert!(owner.slot.lock().unwrap().active.is_none());
        assert_eq!(old.bind(19).unwrap(), Some(19));

        let replacement = Arc::new(Control::new("new".into()));
        owner.slot.lock().unwrap().active = Some(Arc::clone(&replacement));
        drop(Lease {
            owner: Arc::clone(&owner),
            control: old,
        });
        assert!(!owner.cancel_request("old").unwrap());
        assert_eq!(replacement.bind(20).unwrap(), None);
        assert!(owner.cancel_request("new").unwrap());
        assert_eq!(replacement.bind(20).unwrap(), Some(20));
    }

    #[test]
    fn sse_is_bounded_nonblocking_and_preserves_raw_thinking() {
        let control = Arc::new(Control::new("stream".into()));
        let (send, mut receive) = mpsc::channel(1);
        let sink = Forwarder {
            control,
            send,
            grounding: None,
        };
        sink.on_event(GenerationEvent::Started { id: 20 }).unwrap();
        sink.on_event(GenerationEvent::Text {
            id: 20,
            text: "<think>x</think>hello\n".into(),
        })
        .unwrap();
        assert!(sink
            .on_event(GenerationEvent::Text {
                id: 20,
                text: "full".into()
            })
            .is_err());
        let first = receive.try_recv().unwrap();
        let value: serde_json::Value =
            serde_json::from_str(first.bytes.trim().strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(value["message"]["content"], "<think>x</think>hello\n");
        assert_eq!(value["done"], false);
        assert!(sink
            .on_event(GenerationEvent::Text {
                id: 21,
                text: "wrong".into()
            })
            .is_err());
        sink.on_event(GenerationEvent::Finished {
            id: 20,
            reason: GenerationEnd::Limit,
            stats: Default::default(),
            error: None,
        })
        .unwrap();
        assert!(receive
            .try_recv()
            .unwrap()
            .bytes
            .contains("\"done_reason\":\"length\""));
    }
}
