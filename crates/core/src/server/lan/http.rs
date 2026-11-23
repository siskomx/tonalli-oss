use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::super::{content_router, ContentState};
use super::sessions::{self, Session, Sessions};
use super::streams::{self, Bucket};
use super::transport::Peer;

pub(super) struct HttpState {
    pub sessions: Arc<Mutex<Sessions>>,
    pub host: String,
    hostname_authority: Option<String>,
    pub stop: CancellationToken,
    bandwidth: Arc<tokio::sync::Mutex<Bucket>>,
    payloads: Arc<Semaphore>,
    public_payloads: Arc<Semaphore>,
}

impl HttpState {
    pub fn new(sessions: Arc<Mutex<Sessions>>, host: String, stop: CancellationToken) -> Self {
        Self {
            sessions,
            host,
            hostname_authority: None,
            stop,
            bandwidth: Arc::new(tokio::sync::Mutex::new(Bucket::new(
                tokio::time::Instant::now(),
            ))),
            payloads: Arc::new(Semaphore::new(4)),
            public_payloads: Arc::new(Semaphore::new(4)),
        }
    }
    pub fn with_hostname(mut self, authority: String) -> Self {
        self.hostname_authority = Some(authority);
        self
    }
}

pub(super) fn router(content: Arc<ContentState>, state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/pair", get(pair_page).post(pair))
        .route("/api/session", get(session))
        .route(
            "/api/room",
            get(room_get)
                .post(room_post)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .layer(DefaultBodyLimit::max(1024))
        .with_state(Arc::clone(&state))
        .merge(content_router(content))
        .layer(middleware::from_fn_with_state(state, guard))
}

async fn guard(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    mut request: Request,
    next: Next,
) -> Response {
    let zim = request.uri().path().starts_with("/zim/");
    let result = async {
        let authority = request.headers().get(header::HOST).and_then(|value| value.to_str().ok());
        if request.headers().get_all(header::HOST).iter().count() != 1 ||
            !authority.is_some_and(|host| host == state.host || state.hostname_authority.as_deref() == Some(host)) ||
            request.headers().get_all(header::ORIGIN).iter().any(|value| value.to_str().ok() != authority.map(|_| format!("https://{}", authority.unwrap_or_default())).as_deref()) ||
            request.headers().get_all("sec-fetch-site").iter().any(|value| value.to_str().map_or(true, |value| value.eq_ignore_ascii_case("cross-site"))) {
            return StatusCode::FORBIDDEN.into_response();
        }
        if state.stop.is_cancelled() { return StatusCode::SERVICE_UNAVAILABLE.into_response(); }
        let admission = state.sessions.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE).and_then(|mut sessions| sessions.admit(peer.address.ip(), Instant::now()));
        if let Err(status) = admission { return status.into_response(); }
        let Some(path) = normalized(request.uri().path()) else { return StatusCode::NOT_FOUND.into_response(); };
        if private_path(&path) || (path.split('/').nth(1) == Some("pair") && request.uri().path() != "/pair") { return StatusCode::NOT_FOUND.into_response(); }
        if request.uri().path() == "/pair" { return public_response(&state, next.run(request).await).await; }
        let api = path == "/api" || path.starts_with("/api/");
        let name = if api { "tonalli_lan_api" } else { "tonalli_lan_content" };
        let Some(token) = cookie(request.headers(), name) else { return unauthenticated(&request, &path, api); };
        let session = match state.sessions.lock() {
            Ok(sessions) => sessions.authenticate(token, api),
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        let Some(session) = session else { return unauthenticated(&request, &path, api); };
        if api && (!one_header(request.headers(), "sec-fetch-dest", |value| value == "empty") ||
            (path != "/api/session" && !one_header(request.headers(), "x-tonalli-csrf", |value| sessions::equal(value, &session.csrf)))) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let permit = tokio::select! {
            biased;
            _ = session.revoked.cancelled() => return StatusCode::UNAUTHORIZED.into_response(),
            permit = Arc::clone(&state.payloads).acquire_owned() => match permit { Ok(permit) => permit, Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response() },
        };
        let lease = match peer.connection.attach(session.revoked.clone()) { Ok(lease) => lease, Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response() };
        request.extensions_mut().insert(Arc::clone(&session));
        let response = tokio::select! {
            biased;
            _ = session.revoked.cancelled() => return StatusCode::UNAUTHORIZED.into_response(),
            response = next.run(request) => response,
        };
        let (parts, body) = response.into_parts();
        Response::from_parts(parts, streams::wrap(body, session.revoked.clone(), Arc::clone(&state.bandwidth), permit, Some(lease)))
    }.await;
    secure(result, zim)
}

// Connectivity checks made by common operating systems; sending them to pairing is the supported captive-style landing.
const PORTAL_PROBES: [&str; 8] = [
    "/generate_204",
    "/gen_204",
    "/hotspot-detect.html",
    "/library/test/success.html",
    "/success.txt",
    "/ncsi.txt",
    "/connecttest.txt",
    "/canonical.html",
];

fn unauthenticated(request: &Request, path: &str, api: bool) -> Response {
    let reads = matches!(*request.method(), Method::GET | Method::HEAD);
    let landing = path == "/"
        || PORTAL_PROBES.contains(&path)
        || request
            .headers()
            .get_all(header::ACCEPT)
            .iter()
            .any(|value| value.to_str().is_ok_and(|value| value.contains("text/html")));
    if api || !reads || !landing {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static("/pair"));
    response
}

async fn public_response(state: &HttpState, response: Response) -> Response {
    let permit = tokio::select! {
        biased;
        _ = state.stop.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        permit = Arc::clone(&state.public_payloads).acquire_owned() => match permit {
            Ok(permit) => permit,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        },
    };
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        streams::wrap(
            body,
            state.stop.clone(),
            Arc::clone(&state.bandwidth),
            permit,
            None,
        ),
    )
}

fn secure(mut response: Response, zim: bool) -> Response {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    if zim {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(super::super::zim::CSP),
        );
    }
    response
}

fn normalized(path: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    if decoded.contains(['\\', ':', '\0']) {
        return None;
    }
    Some(decoded.to_ascii_lowercase())
}

fn private_path(path: &str) -> bool {
    let mut pieces = path.trim_start_matches('/').split('/').map(|piece| {
        piece
            .trim_end_matches([' ', '.'])
            .split('.')
            .next()
            .unwrap_or("")
    });
    match pieces.next().unwrap_or("") {
        "notes" | "settings" | "admin" | "preferences" | "files" | "reading" | "music"
        | "videos" | "video" | "gallery" | "file" | "media" | "native" | "private" | "identity"
        | "chat" | "content" | "reader" | "book" | "cookbook" | "workshop" => true,
        "api" => matches!(
            pieces.next().unwrap_or(""),
            "notes"
                | "settings"
                | "admin"
                | "preferences"
                | "files"
                | "reading"
                | "music"
                | "video"
                | "documents"
                | "workshop"
                | "index"
                | "ai"
                | "content"
                | "map-management"
                | "sharing"
                | "remote"
                | "update"
        ),
        _ => false,
    }
}

fn one_header(headers: &HeaderMap, name: &str, accepts: impl FnOnce(&str) -> bool) -> bool {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    values.next().is_none() && accepts(value)
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for piece in value.to_str().ok()?.split(';') {
            let Some((key, value)) = piece.trim().split_once('=') else {
                continue;
            };
            if key == name {
                if found.is_some() {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    code: String,
    label: String,
}

async fn pair(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Json(pair): Json<Pair>,
) -> Response {
    let session = match state.sessions.lock() {
        Ok(mut sessions) => {
            sessions.pair(peer.address.ip(), &pair.code, &pair.label, Instant::now())
        }
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let session = match session {
        Ok(session) => session,
        Err(status) => return status.into_response(),
    };
    let mut response = Json(serde_json::json!({"status":"paired"})).into_response();
    for (name, value, path) in [
        ("tonalli_lan_content", &session.content, "/"),
        ("tonalli_lan_api", &session.api, "/api"),
    ] {
        let cookie = format!("{name}={value}; Path={path}; Secure; HttpOnly; SameSite=Strict");
        let Ok(cookie) = HeaderValue::from_str(&cookie) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

async fn session(Extension(session): Extension<Arc<Session>>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"csrf":session.csrf, "private_notes": false, "private_ai": false,"private_ai_management":false, "private_content": false,"shared_media":true,"lan_room":true,"private_tv":false}),
    )
}
async fn room_get(
    State(state): State<Arc<HttpState>>,
    Extension(session): Extension<Arc<Session>>,
    Query(cursor): Query<super::super::room::Cursor>,
) -> Response {
    room_request(state, session, cursor.request())
}
async fn room_post(
    State(state): State<Arc<HttpState>>,
    Extension(session): Extension<Arc<Session>>,
    Json(request): Json<crate::room::RoomRequest>,
) -> Response {
    room_request(state, session, request)
}
fn room_request(
    state: Arc<HttpState>,
    session: Arc<Session>,
    request: crate::room::RoomRequest,
) -> Response {
    super::super::room::response(
        state
            .sessions
            .lock()
            .map_err(|_| crate::room::RoomError::Unavailable)
            .and_then(|mut sessions| sessions.room_peer(&session, request)),
    )
}

async fn pair_page() -> Response {
    let mut response = Html(include_str!("pair.html")).into_response();
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"));
    response
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
