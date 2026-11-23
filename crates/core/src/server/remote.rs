//! Native client-mode owner and authenticated reader-route adapter.
pub(super) mod copy;
use super::ServerState;
use crate::remote::{RemoteError, RemoteHost, RemoteLibrary};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct Connection {
    state: Mutex<Slot>,
    copies: Mutex<Arc<copy::Copies>>,
}
#[derive(Default)]
struct Slot {
    epoch: u64,
    client: Option<Arc<RemoteLibrary>>,
    pending: Option<CancellationToken>,
    stopped: bool,
}
impl Connection {
    pub(super) fn copies(&self) -> Result<Arc<copy::Copies>, StatusCode> {
        self.copies
            .lock()
            .map(|value| Arc::clone(&value))
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    pub(super) fn attach_copies(
        &self,
        copies: Arc<copy::Copies>,
    ) -> Result<(), crate::pack::PackError> {
        let connection = self.state.lock().map_err(|_| super::registry_poisoned())?;
        let mut current = self.copies.lock().map_err(|_| super::registry_poisoned())?;
        if connection.stopped
            || connection.client.is_some()
            || connection.pending.is_some()
            || current.busy()?
            || copies.is_closed()?
        {
            return Err(crate::pack::PackError::Mismatch(
                "Copy owner unavailable".into(),
            ));
        }
        *current = copies;
        Ok(())
    }
    pub(super) fn client(&self) -> Result<Option<Arc<RemoteLibrary>>, StatusCode> {
        self.state
            .lock()
            .map(|s| s.client.clone())
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
    pub(super) fn close(&self, stop: bool) {
        if let Ok(copies) = self.copies() {
            if !stop || !copies.is_native_owned() {
                copies.stop(stop);
            }
        }
        if let Ok(mut slot) = self.state.lock() {
            slot.epoch = slot.epoch.wrapping_add(1);
            slot.stopped |= stop;
            if let Some(pending) = slot.pending.take() {
                pending.cancel();
            }
            if let Some(client) = slot.client.take() {
                client.close();
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Connect {
    invitation: String,
    code: String,
    label: String,
}
fn problem(error: RemoteError) -> Response {
    let status = match error {
        RemoteError::InvalidHost | RemoteError::InvalidInput => StatusCode::BAD_REQUEST,
        RemoteError::Rejected(401) => StatusCode::UNAUTHORIZED,
        RemoteError::Rejected(404) => StatusCode::NOT_FOUND,
        RemoteError::Rejected(429) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    };
    (status, Json(json!({"error":"remote_unavailable"}))).into_response()
}
pub(super) async fn connect(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Connect>,
) -> Response {
    let host = match RemoteHost::invitation(&input.invitation) {
        Ok(h) => h,
        Err(e) => return problem(e),
    };
    let (epoch, cancel) = {
        let mut slot = match state.remote.state.lock() {
            Ok(s) => s,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        if slot.stopped || slot.client.is_some() || slot.pending.is_some() {
            return StatusCode::CONFLICT.into_response();
        }
        slot.epoch = slot.epoch.wrapping_add(1);
        let cancel = CancellationToken::new();
        slot.pending = Some(cancel.clone());
        (slot.epoch, cancel)
    };
    let paired = tokio::select! {biased;_=cancel.cancelled()=>Err(RemoteError::Closed),result=RemoteLibrary::pair(host,&input.code,&input.label)=>result};
    let mut slot = match state.remote.state.lock() {
        Ok(s) => s,
        Err(_) => {
            if let Ok(client) = paired {
                client.close();
            }
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    if slot.stopped || slot.epoch != epoch {
        if let Ok(client) = paired {
            client.close();
        }
        return StatusCode::CONFLICT.into_response();
    }
    slot.pending = None;
    match paired {
        Ok(client) => {
            state.notes.revoke();
            state.admin.lock();
            slot.client = Some(Arc::new(client));
            Json(json!({"connected":true})).into_response()
        }
        Err(error) => problem(error),
    }
}
pub(super) async fn disconnect(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    state.remote.close(false);
    Json(json!({"connected":false}))
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    match state.remote.state.lock() {
        Ok(slot) => {
            let copies = state.remote.copies().ok();
            Json(
                json!({"connected":slot.client.is_some(),"connecting":slot.pending.is_some(),
                "background":copies.as_ref().is_some_and(|copies| copies.is_native_owned()),
                "copy":copies.and_then(|copies| copies.snapshot().ok())}),
            )
            .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
pub(super) async fn reader(
    State(state): State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Response {
    let client = match state.remote.client() {
        Ok(Some(client)) => client,
        Ok(None) => return next.run(request).await,
        Err(status) => return status.into_response(),
    };
    let path = request.uri().path();
    if matches!(
        path,
        "/api/session"
            | "/api/remote"
            | "/api/remote/disconnect"
            | "/api/remote/copy"
            | "/api/remote/copy/control"
    ) {
        return next.run(request).await;
    }
    if !crate::remote::reader_path(path) {
        if matches!(request.method(), &Method::GET | &Method::HEAD)
            && (matches!(
                path,
                "/" | "/library"
                    | "/article"
                    | "/map"
                    | "/shared"
                    | "/room"
                    | "/settings"
                    | "/favicon.ico"
                    | "/manifest.webmanifest"
                    | "/sw.js"
            ) || path.starts_with("/assets/"))
        {
            // Only shipped UI stays local; private file/media routes never fall through.
            return next.run(request).await;
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, 16384).await {
        Ok(bytes) => bytes.to_vec(),
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let target = parts.uri.path_and_query().map(|s| s.as_str()).unwrap_or("");
    let limit = if parts.uri.path().starts_with("/api/") {
        4 * 1024 * 1024_u64
    } else {
        u64::MAX
    };
    let reply = match client
        .reader_request(parts.method, target, &parts.headers, body)
        .await
    {
        Ok(r) => r,
        Err(e) => return problem(e),
    };
    let status = reply.status();
    if reply.content_length().is_some_and(|length| length > limit) {
        return problem(RemoteError::InvalidResponse);
    }
    if !matches!(
        status.as_u16(),
        200 | 206 | 304 | 401 | 403 | 404 | 416 | 429
    ) {
        return problem(RemoteError::Unavailable);
    }
    let mut response = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CONTENT_DISPOSITION,
    ] {
        if let Some(value) = reply.headers().get(&name) {
            response = response.header(name, value);
        }
    }
    response = response.header(header::CONTENT_SECURITY_POLICY, super::zim::CSP);
    let cancel = client.cancellation();
    let stream = futures_util::stream::unfold(
        (reply, cancel, false, limit),
        |(mut reply, cancel, ended, remaining)| async move {
            if ended {
                return None;
            }
            let chunk = tokio::select! {biased;_=cancel.cancelled()=>return Some((Err(RemoteError::Closed),(reply,cancel,true,remaining))),result=reply.chunk()=>result};
            match chunk {
                Ok(Some(bytes)) if bytes.len() as u64 <= remaining => {
                    let left = remaining - bytes.len() as u64;
                    Some((Ok(bytes), (reply, cancel, false, left)))
                }
                Ok(Some(_)) => Some((Err(RemoteError::InvalidResponse), (reply, cancel, true, 0))),
                Ok(None) => None,
                Err(_) => Some((
                    Err(RemoteError::Unavailable),
                    (reply, cancel, true, remaining),
                )),
            }
        },
    );
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}
