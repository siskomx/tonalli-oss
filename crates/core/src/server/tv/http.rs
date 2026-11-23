//! Explicit, separately owned HTTP serving; never mounts app or paired-share routes.
use super::super::lan::{wrap_selected_media, SelectedBandwidth};
use super::{description, events::Events, transport, TvCatalog, TvService};
use crate::pack::PackError;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Router,
};
use std::{
    future::IntoFuture,
    net::{IpAddr, SocketAddr, TcpListener},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(2);
struct HttpState {
    catalog: Arc<TvCatalog>,
    address: SocketAddr,
    uuid: String,
    stop: CancellationToken,
    permits: Arc<Semaphore>,
    bandwidth: Arc<Mutex<SelectedBandwidth>>,
    events: Arc<Events>,
}
pub struct TvServer {
    address: SocketAddr,
    stop: CancellationToken,
    worker: Option<JoinHandle<Result<(), PackError>>>,
}
impl TvServer {
    pub fn start(bind: IpAddr, port: u16, catalog: Arc<TvCatalog>) -> Result<Self, PackError> {
        super::super::lan::validate_bind(bind)?;
        let socket = TcpListener::bind((bind, port))?;
        socket.set_nonblocking(true)?;
        let address = socket.local_addr()?;
        let uuid = super::random_uuid()?;
        let discovery = match bind {
            IpAddr::V4(bind) => Some(super::ssdp::Prepared::new(bind, address, &uuid)?),
            IpAddr::V6(_) => None,
        };
        let stop = CancellationToken::new();
        let events = Events::new(bind, stop.clone(), catalog.protocol_info())?;
        let state = Arc::new(HttpState {
            catalog,
            address,
            uuid,
            stop: stop.clone(),
            permits: Arc::new(Semaphore::new(4)),
            bandwidth: Arc::new(Mutex::new(SelectedBandwidth::new(
                tokio::time::Instant::now(),
            ))),
            events,
        });
        let (ready, readiness) = std::sync::mpsc::sync_channel(1);
        let worker_stop = stop.clone();
        let worker = thread::Builder::new().name("tonalli-tv".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(4).build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let error = PackError::Io(error.to_string());
                    let _ = ready.send(Err(error.clone()));
                    return Err(error);
                }
            };
            let (result, remaining) = runtime.block_on(async {
                let sockets = tokio::net::TcpListener::from_std(socket).and_then(|socket| {
                    discovery.map(super::ssdp::Prepared::into_async).transpose().map(|discovery| (socket, discovery))
                });
                let (socket, discovery) = match sockets {
                    Ok(value) => value,
                    Err(error) => {
                        let error = PackError::Io(error.to_string());
                        let _ = ready.send(Err(error.clone()));
                        return (Err(error), Duration::ZERO);
                    }
                };
                let app = router(state);
                if ready.send(Ok(())).is_err() {
                    return (Err(PackError::Io("TV startup caller disappeared".into())), Duration::ZERO);
                }
                let discovery = discovery.map(|discovery| {
                    let stop = worker_stop.clone();
                    tokio::spawn(async move {
                        let result = discovery.run(stop.clone(), |ip| super::super::lan::validate_bind(ip).is_ok()).await;
                        if result.is_err() { stop.cancel(); }
                        result
                    })
                });
                let serving = axum::serve(transport::Listener::new(socket, worker_stop.clone()), app.into_make_service_with_connect_info::<transport::Peer>())
                    .with_graceful_shutdown(worker_stop.clone().cancelled_owned()).into_future();
                tokio::pin!(serving);
                let (mut result, mut remaining) = tokio::select! {
                    result = &mut serving => (result.map_err(Into::into), SHUTDOWN_LIMIT),
                    _ = worker_stop.cancelled() => {
                        let started = Instant::now();
                        let result = match tokio::time::timeout(SHUTDOWN_LIMIT, &mut serving).await {
                            Ok(result) => result.map_err(Into::into),
                            Err(_) => Ok(()),
                        };
                        (result, SHUTDOWN_LIMIT.saturating_sub(started.elapsed()))
                    }
                };
                worker_stop.cancel();
                if let Some(discovery) = discovery {
                    let started = Instant::now();
                    match tokio::time::timeout(remaining, discovery).await {
                        Ok(Ok(Err(error))) if result.is_ok() => result = Err(error.into()),
                        Ok(Err(_)) if result.is_ok() => result = Err(PackError::Corrupt("TV discovery task panicked".into())),
                        _ => {}
                    }
                    remaining = remaining.saturating_sub(started.elapsed());
                }
                (result, remaining)
            });
            worker_stop.cancel();
            runtime.shutdown_timeout(remaining);
            result
        })?;
        let mut server = Self {
            address,
            stop,
            worker: Some(worker),
        };
        match readiness.recv() {
            Ok(Ok(())) => Ok(server),
            Ok(Err(error)) => {
                let _ = server.shutdown();
                Err(error)
            }
            Err(_) => {
                let _ = server.shutdown();
                Err(PackError::Io("TV startup stopped before readiness".into()))
            }
        }
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn is_running(&self) -> bool {
        !self.stop.is_cancelled()
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }
    pub fn shutdown(&mut self) -> Result<(), PackError> {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| PackError::Corrupt("TV runtime thread panicked".into()))?
        } else {
            Ok(())
        }
    }
}
impl Drop for TvServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/description.xml", get(device))
        .route("/content-directory.xml", get(cds))
        .route("/connection-manager.xml", get(cm))
        .route("/control/content-directory", post(control_cds))
        .route("/control/connection-manager", post(control_cm))
        .route("/events/content-directory", any(events_cds))
        .route("/events/connection-manager", any(events_cm))
        .route("/media/{id}", get(media))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(Arc::clone(&state), guard))
        .with_state(state)
}
async fn guard(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<transport::Peer>,
    request: Request,
    next: Next,
) -> Response {
    if state.stop.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let expected = state.address.to_string();
    let headers = request.headers();
    let mut host = headers.get_all(header::HOST).iter();
    if super::super::lan::validate_bind(peer.0.ip()).is_err()
        || host.next().and_then(|value| value.to_str().ok()) != Some(expected.as_str())
        || host.next().is_some()
        || headers
            .get_all(header::ORIGIN)
            .iter()
            .any(|value| value.to_str().ok() != Some(format!("http://{expected}").as_str()))
        || headers.get_all("sec-fetch-site").iter().any(|value| {
            value
                .to_str()
                .map_or(true, |value| value.eq_ignore_ascii_case("cross-site"))
        })
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let permit = match Arc::clone(&state.permits).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    let mut response = Response::from_parts(
        parts,
        wrap_selected_media(
            body,
            state.stop.clone(),
            Arc::clone(&state.bandwidth),
            permit,
        ),
    );
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("content-security-policy", "default-src 'none'; sandbox"),
        ("referrer-policy", "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
fn xml(value: String) -> Response {
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], value).into_response()
}
async fn events_cds(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<transport::Peer>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    state
        .events
        .request(method, &headers, peer.0.ip(), TvService::ContentDirectory)
}
async fn events_cm(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<transport::Peer>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    state
        .events
        .request(method, &headers, peer.0.ip(), TvService::ConnectionManager)
}
async fn device(State(state): State<Arc<HttpState>>) -> Response {
    xml(description::device(&state.uuid))
}
async fn cds() -> Response {
    xml(description::service(TvService::ContentDirectory))
}
async fn cm() -> Response {
    xml(description::service(TvService::ConnectionManager))
}
async fn media(
    State(state): State<Arc<HttpState>>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Some((source, mime)) = state.catalog.source(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = super::super::ranges::serve_source(source, method, headers).await;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
}
async fn control_cds(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    control(state, TvService::ContentDirectory, headers, &body)
}
async fn control_cm(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    control(state, TvService::ConnectionManager, headers, &body)
}
fn control(state: Arc<HttpState>, service: TvService, headers: HeaderMap, body: &[u8]) -> Response {
    let mut values = headers.get_all("soapaction").iter();
    let action = values.next().and_then(|value| value.to_str().ok());
    let result = match action {
        Some(action) if values.next().is_none() => {
            state.catalog.control(state.address, service, action, body)
        }
        _ => Err(super::TvError::Invalid),
    };
    match result {
        Ok(value) => xml(value),
        Err(error) => {
            let body=format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{}</errorCode><errorDescription>{}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>",error.code(),super::escaped(&error.to_string()));
            let mut response = xml(body);
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}
