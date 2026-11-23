//! Private native broker connection; no credentials or control routes are shared over LAN.
use super::ServerState;
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct Connection {
    slot: Arc<Mutex<Slot>>,
}
impl Default for Connection {
    fn default() -> Self {
        #[cfg(target_os = "android")]
        {
            static SLOT: std::sync::OnceLock<Arc<Mutex<Slot>>> = std::sync::OnceLock::new();
            Self {
                slot: Arc::clone(SLOT.get_or_init(|| Arc::new(Mutex::new(Slot::default())))),
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            Self {
                slot: Arc::new(Mutex::new(Slot::default())),
            }
        }
    }
}
#[derive(Default)]
struct Slot {
    epoch: u64,
    client: Option<Arc<Client>>,
    pending: Option<CancellationToken>,
    stopped: bool,
    #[cfg(feature = "mesh")]
    bridge: Option<bridge::Bridge>,
}
struct Pending {
    slot: Arc<Mutex<Slot>>,
    epoch: u64,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            if slot.epoch == self.epoch {
                if let Some(cancel) = slot.pending.take() {
                    cancel.cancel();
                    slot.epoch = slot.epoch.wrapping_add(1);
                }
            }
        }
    }
}
#[cfg(feature = "mesh")]
pub(super) mod bridge;
struct Client {
    http: reqwest::Client,
    origin: String,
    cookie: zeroize::Zeroizing<String>,
    csrf: String,
    user: Value,
    cancel: CancellationToken,
    slots: tokio::sync::Semaphore,
}
impl Connection {
    pub(super) fn close(&self, stopped: bool) {
        #[cfg(feature = "mesh")]
        self.stop_bridge();
        if let Ok(mut s) = self.slot.lock() {
            s.epoch = s.epoch.wrapping_add(1);
            s.stopped |= stopped;
            if let Some(c) = s.pending.take() {
                c.cancel();
            }
            if let Some(c) = s.client.take() {
                c.cancel.cancel();
            }
        }
    }
    fn client(&self) -> Result<Arc<Client>, StatusCode> {
        self.slot
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .client
            .clone()
            .ok_or(StatusCode::CONFLICT)
    }
    #[cfg(feature = "mesh")]
    pub(super) fn stop_bridge(&self) {
        if let Ok(s) = self.slot.lock() {
            if let Some(bridge) = s.bridge.as_ref() {
                bridge.cancel.cancel();
            }
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Connect {
    #[serde(default)]
    url: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    password: String,
    certificate: Option<String>,
    #[serde(default)]
    remember: bool,
    #[serde(default)]
    saved: bool,
}
async fn value(mut response: reqwest::Response) -> Result<(StatusCode, Value), ()> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, serde_json::from_slice(&bytes).map_err(|_| ())?))
}
async fn pair(input: Connect, cancel: CancellationToken) -> Result<Client, ()> {
    let password = zeroize::Zeroizing::new(input.password);
    let url = url::Url::parse(&input.url).map_err(|_| ())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || input.user.len() > 64
        || password.len() > 256
    {
        return Err(());
    }
    let origin = url.origin().ascii_serialization();
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15));
    if let Some(pem) = input.certificate {
        if pem.len() > 16384 {
            return Err(());
        }
        builder = builder
            .add_root_certificate(reqwest::Certificate::from_pem(pem.as_bytes()).map_err(|_| ())?);
    }
    let http = builder.build().map_err(|_| ())?;
    let body = zeroize::Zeroizing::new(
        serde_json::to_string(&json!({"user":input.user,"password":&*password})).map_err(|_| ())?,
    );
    let login = http
        .post(format!("{origin}/api/chat/login"))
        .header(header::ORIGIN, &origin)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|_| ())?;
    if !login.status().is_success() {
        return Err(());
    }
    let cookies: Vec<_> = login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .filter(|v| v.starts_with("tonalli_chat="))
        .collect();
    if cookies.len() != 1
        || cookies[0].len() != 61
        || !cookies[0][13..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(());
    }
    let cookie = zeroize::Zeroizing::new(cookies[0].to_owned());
    let (status, session) = value(
        http.get(format!("{origin}/api/session"))
            .header(header::COOKIE, &*cookie)
            .send()
            .await
            .map_err(|_| ())?,
    )
    .await?;
    let csrf = session["csrf"]
        .as_str()
        .filter(|v| v.len() == 48 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(())?
        .to_owned();
    if status != StatusCode::OK || session["chat_user"]["id"].as_str().is_none() {
        return Err(());
    }
    let user = &session["chat_user"];
    let identifier = |v: &Value| {
        v.as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    };
    if !identifier(&user["id"])
        || !user["name"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control))
        || !matches!(
            user["role"].as_str(),
            Some("admin" | "participant" | "gateway")
        )
        || ["read", "send", "bridge"].iter().any(|field| {
            !user[*field]
                .as_array()
                .is_some_and(|v| v.len() <= 64 && v.iter().all(identifier))
        })
    {
        return Err(());
    }
    let user = json!({"id":user["id"],"name":user["name"],"role":user["role"],"read":user["read"],"send":user["send"],"bridge":user["bridge"]});
    Ok(Client {
        http,
        origin,
        cookie,
        csrf,
        user,
        cancel,
        slots: tokio::sync::Semaphore::new(4),
    })
}
pub(super) async fn connect(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Connect>,
) -> Response {
    let (epoch, cancel) = {
        let Ok(mut s) = state.chat.slot.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if s.stopped || s.pending.is_some() || s.client.is_some() {
            return StatusCode::CONFLICT.into_response();
        }
        s.epoch = s.epoch.wrapping_add(1);
        let cancel = CancellationToken::new();
        s.pending = Some(cancel.clone());
        (s.epoch, cancel)
    };
    let _pending = Pending {
        slot: Arc::clone(&state.chat.slot),
        epoch,
    };
    let work = async {
        let input = if input.saved {
            let directory = state.data_dir.clone();
            tokio::task::spawn_blocking(move || {
                let value = super::chat_storage::open(&directory)
                    .map_err(|_| ())?
                    .connection()
                    .map_err(|_| ())?
                    .ok_or(())?;
                let mut input: Connect = serde_json::from_value(value).map_err(|_| ())?;
                input.saved = false;
                input.remember = false;
                Ok::<_, ()>(input)
            })
            .await
            .map_err(|_| ())??
        } else {
            input
        };
        let saved = input
            .remember
            .then(|| serde_json::to_value(&input))
            .transpose()
            .map_err(|_| ())?;
        let client = pair(input, cancel.clone()).await?;
        if let Some(value) = saved {
            let directory = state.data_dir.clone();
            tokio::task::spawn_blocking(move || {
                super::chat_storage::open(&directory)
                    .map_err(|_| ())?
                    .save_connection(&value)
                    .map_err(|_| ())
            })
            .await
            .map_err(|_| ())??;
        }
        Ok::<_, ()>(client)
    };
    let result = tokio::select! {biased;_=cancel.cancelled()=>Err(()),r=work=>r};
    let Ok(mut s) = state.chat.slot.lock() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if s.epoch != epoch || s.stopped {
        return StatusCode::CONFLICT.into_response();
    }
    s.pending = None;
    match result {Ok(client)=>{s.client=Some(Arc::new(client));Json(json!({"connected":true})).into_response()},Err(())=>(StatusCode::BAD_GATEWAY,Json(json!({"error":"Could not connect. Check the address, certificate and credentials."}))).into_response()}
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    let mut snapshot = {
        let Ok(s) = state.chat.slot.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        json!({"connected":s.client.is_some(),"connecting":s.pending.is_some(),"epoch":s.epoch,"url":s.client.as_ref().map(|c|&c.origin),"user":s.client.as_ref().map(|c|&c.user)})
    };
    let directory = state.data_dir.clone();
    let saved =
        tokio::task::spawn_blocking(move || super::chat_storage::open(&directory)?.connection())
            .await;
    match saved {
        Ok(Ok(value)) => {
            snapshot["saved"] =
                value.map_or(Value::Null, |v| json!({"url":v["url"],"user":v["user"]}));
            Json(snapshot).into_response()
        }
        _ => {
            snapshot["saved"] = Value::Null;
            snapshot["storage_error"] = json!("Encrypted saved connections unavailable");
            Json(snapshot).into_response()
        }
    }
}
pub(super) async fn forget(State(state): State<Arc<ServerState>>) -> Response {
    let directory = state.data_dir.clone();
    match tokio::task::spawn_blocking(move || {
        super::chat_storage::open(&directory)?.forget_connection()
    })
    .await
    {
        Ok(Ok(())) => Json(json!({"forgotten":true})).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
pub(super) async fn disconnect(State(state): State<Arc<ServerState>>) -> Json<Value> {
    let client = state.chat.client().ok();
    state.chat.close(false);
    if let Some(client) = client {
        let logout = client
            .http
            .post(format!("{}/api/chat/logout", client.origin))
            .header(header::ORIGIN, &client.origin)
            .header(header::COOKIE, &*client.cookie)
            .header("X-Tonalli-Csrf", &client.csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send();
        let _ = tokio::time::timeout(Duration::from_secs(2), logout).await;
    }
    Json(json!({"connected":false}))
}
pub(super) async fn proxy(State(state): State<Arc<ServerState>>, request: Request) -> Response {
    let client = match state.chat.client() {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let Ok(_permit) = client.slots.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let route = request
        .uri()
        .path()
        .strip_prefix("/api/connections/chat/proxy")
        .unwrap_or("");
    let room = route.strip_prefix("/rooms/").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    });
    if !room
        && !matches!(
            route,
            "/rooms" | "/admin" | "/admin/stop" | "/admin/invite" | "/logout" | "/session"
        )
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let target = if route == "/session" {
        format!("{}/api/session", client.origin)
    } else {
        format!("{}/api/chat{route}", client.origin)
    };
    let method = request.method().clone();
    let body = match to_bytes(request.into_body(), 32768).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let future = async {
        let response = client
            .http
            .request(method, target)
            .header(header::ORIGIN, &client.origin)
            .header(header::COOKIE, &*client.cookie)
            .header("X-Tonalli-Csrf", &client.csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| ())?;
        value(response).await
    };
    match tokio::select! {biased;_=client.cancel.cancelled()=>Err(()),v=future=>v} {
        Ok((status, body)) => (status, Json(body)).into_response(),
        Err(()) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":"Chat connection unavailable"})),
        )
            .into_response(),
    }
}
