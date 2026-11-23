use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use subtle::ConstantTimeEq;

use super::ServerState;
use crate::pack::PackError;

pub(super) struct Auth {
    token: String,
    content: String,
    api: String,
    csrf: String,
    exchanged: bool,
}
impl Auth {
    pub(super) fn new() -> Result<Self, PackError> {
        Ok(Self {
            token: random_token()?,
            content: random_token()?,
            api: random_token()?,
            csrf: random_token()?,
            exchanged: false,
        })
    }
    pub(super) fn token(&self) -> &str {
        &self.token
    }
    fn authorize(&self, headers: &HeaderMap, path: &str) -> Result<Option<String>, StatusCode> {
        let api = path == "/api" || path.starts_with("/api/");
        let (name, expected) = if api {
            ("tonalli_api", &self.api)
        } else {
            ("tonalli_content", &self.content)
        };
        if !self.exchanged || !cookie(headers, name).is_some_and(|value| equal(value, expected)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        if api {
            if headers
                .get("sec-fetch-dest")
                .and_then(|value| value.to_str().ok())
                != Some("empty")
            {
                return Err(StatusCode::FORBIDDEN);
            }
            if path != "/api/session"
                && !headers
                    .get("x-tonalli-csrf")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| equal(value, &self.csrf))
            {
                return Err(StatusCode::FORBIDDEN);
            }
            return Ok(Some(self.csrf.clone()));
        }
        Ok(None)
    }
}

#[derive(Clone)]
pub(super) struct Csrf(String);

pub(super) async fn guard(
    State(state): State<Arc<ServerState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let zim_content = request.uri().path().starts_with("/zim/");
    let authorization = (|| {
        let headers = request.headers();
        if headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            != Some(state.host.as_str())
        {
            return Err(StatusCode::FORBIDDEN);
        }
        if headers
            .get_all(header::ORIGIN)
            .iter()
            .any(|value| value.to_str().ok() != Some(state.origin.as_str()))
            || headers.get_all("sec-fetch-site").iter().any(|value| {
                value
                    .to_str()
                    .map_or(true, |value| value.eq_ignore_ascii_case("cross-site"))
            })
        {
            return Err(StatusCode::FORBIDDEN);
        }
        let path = request.uri().path();
        if path == "/launch" {
            if request.method() != Method::GET {
                return Err(StatusCode::METHOD_NOT_ALLOWED);
            }
            return Ok((None, state.notes.admission()));
        }
        let auth = state
            .auth
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let csrf = auth.authorize(headers, path)?;
        Ok((csrf, state.notes.admission()))
    })();
    let mut response = match authorization {
        Ok((csrf, admission)) => {
            request.extensions_mut().insert(admission);
            if let Some(csrf) = csrf {
                request.extensions_mut().insert(Csrf(csrf));
            }
            next.run(request).await
        }
        Err(status) => status.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if zim_content {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(super::zim::CSP),
        );
    }
    response
}

pub(super) async fn launch(State(state): State<Arc<ServerState>>, request: Request) -> Response {
    let Some(token) = request
        .uri()
        .query()
        .and_then(|query| query.strip_prefix("token="))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut auth = match state.auth.lock() {
        Ok(auth) => auth,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if auth.exchanged || !equal(token, &auth.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let content = HeaderValue::from_str(&format!(
        "tonalli_content={}; Path=/; HttpOnly; SameSite=Strict",
        auth.content
    ));
    let api = HeaderValue::from_str(&format!(
        "tonalli_api={}; Path=/api; HttpOnly; SameSite=Strict",
        auth.api
    ));
    let (Ok(content), Ok(api)) = (content, api) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    auth.exchanged = true;
    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static("/"));
    response.headers_mut().append(header::SET_COOKIE, content);
    response.headers_mut().append(header::SET_COOKIE, api);
    response
}

pub(super) async fn session(
    State(state): State<Arc<ServerState>>,
    Extension(Csrf(csrf)): Extension<Csrf>,
) -> Json<serde_json::Value> {
    let private_tv = state.tv.lock().is_ok_and(|slot| slot.is_some());
    let private_content = super::content::available(&state);
    let private_index_preparation = state.preparation.lock().is_ok_and(|slot| slot.is_some());
    let remote = state
        .remote
        .client()
        .map_or(true, |client| client.is_some());
    #[cfg(feature = "ai")]
    let private_ai = state.ai.available();
    #[cfg(not(feature = "ai"))]
    let private_ai = {
        let _ = state;
        false
    };
    Json(
        serde_json::json!({"csrf": csrf, "private_notes": !remote,"private_mesh":cfg!(feature="mesh")&&!remote, "private_ai": private_ai && !remote,"private_ai_management":cfg!(feature = "ai") && !remote, "private_content": private_content && !remote,"private_index_preparation":private_index_preparation && !remote,"lan_room":true,"private_tv":private_tv && !remote,"private_remote":true,"remote_library":remote,"shared_media":remote}),
    )
}

fn cookie<'a>(headers: &'a HeaderMap, wanted: &str) -> Option<&'a str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header.to_str().ok()?.split(';') {
            if let Some((name, value)) = part.trim().split_once('=') {
                if name == wanted {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
    }
    found
}
fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn random_token() -> Result<String, PackError> {
    let mut bytes = zeroize::Zeroizing::new([0; 32]);
    getrandom::fill(&mut bytes[..])
        .map_err(|error| PackError::Io(format!("session randomness failed: {error}")))?;
    let mut token = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes.iter() {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(token)
}
