//! Ephemeral, peer-scoped GENA leases. Callbacks never resolve names or follow redirects.
use super::{escaped, random_uuid, TvService};
use crate::pack::PackError;
use axum::{
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};
const MAX_LEASES: usize = 32;
struct Lease {
    peer: IpAddr,
    service: TvService,
    expires: Instant,
    cancel: CancellationToken,
}
pub(super) struct Events {
    leases: Mutex<HashMap<String, Lease>>,
    client: reqwest::Client,
    stop: CancellationToken,
    protocols: String,
}
impl Events {
    pub fn new(
        bind: IpAddr,
        stop: CancellationToken,
        protocols: String,
    ) -> Result<Arc<Self>, PackError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .local_address(bind)
            .timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|problem| PackError::Io(problem.to_string()))?;
        Ok(Arc::new(Self {
            leases: Mutex::new(HashMap::new()),
            client,
            stop,
            protocols,
        }))
    }
    pub fn request(
        self: &Arc<Self>,
        method: Method,
        headers: &HeaderMap,
        peer: IpAddr,
        service: TvService,
    ) -> Response {
        self.handle(method, headers, peer, service)
            .unwrap_or_else(IntoResponse::into_response)
    }
    fn handle(
        self: &Arc<Self>,
        method: Method,
        headers: &HeaderMap,
        peer: IpAddr,
        service: TvService,
    ) -> Result<Response, StatusCode> {
        if self.stop.is_cancelled() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let subscribe = method.as_str() == "SUBSCRIBE";
        let unsubscribe = method.as_str() == "UNSUBSCRIBE";
        if !subscribe && !unsubscribe {
            return Err(StatusCode::METHOD_NOT_ALLOWED);
        }
        let sid = single(headers, "sid")?;
        let callback = single(headers, "callback")?;
        let nt = single(headers, "nt")?;
        let timeout = single(headers, "timeout")?;
        if sid.is_some() && (callback.is_some() || nt.is_some()) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let now = Instant::now();
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        leases.retain(|_, lease| {
            if now >= lease.expires {
                lease.cancel.cancel();
                false
            } else {
                true
            }
        });
        if unsubscribe {
            if callback.is_some() || nt.is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let sid = sid.ok_or(StatusCode::PRECONDITION_FAILED)?;
            let lease = leases
                .get(sid)
                .filter(|lease| lease.peer == peer && lease.service == service)
                .ok_or(StatusCode::PRECONDITION_FAILED)?;
            lease.cancel.cancel();
            leases.remove(sid);
            return Ok(StatusCode::OK.into_response());
        }
        let seconds = timeout_seconds(timeout)?;
        if let Some(sid) = sid {
            let lease = leases
                .get_mut(sid)
                .filter(|lease| lease.peer == peer && lease.service == service)
                .ok_or(StatusCode::PRECONDITION_FAILED)?;
            lease.expires = now + Duration::from_secs(seconds);
            return granted(sid, seconds);
        }
        if nt != Some("upnp:event") {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
        let callbacks = callbacks(callback.ok_or(StatusCode::PRECONDITION_FAILED)?, peer)?;
        if leases.len() >= MAX_LEASES {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let sid = format!(
            "uuid:{}",
            random_uuid().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        );
        if leases.contains_key(&sid) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let cancel = self.stop.child_token();
        leases.insert(
            sid.clone(),
            Lease {
                peer,
                service,
                expires: now + Duration::from_secs(seconds),
                cancel: cancel.clone(),
            },
        );
        drop(leases);
        let owner = Arc::clone(self);
        let event_sid = sid.clone();
        tokio::spawn(async move {
            // Let the small subscription response flush before the initial event connection.
            tokio::select! {_=cancel.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            owner.notify(service, event_sid, callbacks, cancel).await;
        });
        granted(&sid, seconds)
    }
    fn active(&self, sid: &str) -> bool {
        self.leases
            .lock()
            .ok()
            .and_then(|leases| {
                leases
                    .get(sid)
                    .map(|lease| Instant::now() < lease.expires && !lease.cancel.is_cancelled())
            })
            .unwrap_or(false)
    }
    async fn notify(
        &self,
        service: TvService,
        sid: String,
        callbacks: Vec<Url>,
        cancel: CancellationToken,
    ) {
        let properties=match service{TvService::ContentDirectory=>"<e:property><SystemUpdateID>0</SystemUpdateID></e:property>".into(),TvService::ConnectionManager=>format!("<e:property><SourceProtocolInfo>{}</SourceProtocolInfo></e:property><e:property><SinkProtocolInfo></SinkProtocolInfo></e:property><e:property><CurrentConnectionIDs>0</CurrentConnectionIDs></e:property>",escaped(&self.protocols))};
        let body=format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><e:propertyset xmlns:e=\"urn:schemas-upnp-org:event-1-0\">{properties}</e:propertyset>");
        let Ok(method) = Method::from_bytes(b"NOTIFY") else {
            return;
        };
        for callback in callbacks {
            if cancel.is_cancelled() || !self.active(&sid) {
                return;
            }
            let request = self
                .client
                .request(method.clone(), callback)
                .header("Content-Type", "text/xml; charset=utf-8")
                .header("NT", "upnp:event")
                .header("NTS", "upnp:propchange")
                .header("SID", &sid)
                .header("SEQ", "0")
                .header("Connection", "close")
                .body(body.clone());
            let result =
                tokio::select! {_=cancel.cancelled()=>return,result=request.send()=>result};
            if result.is_ok_and(|response| response.status() == StatusCode::OK) {
                return;
            }
        }
        if let Ok(mut leases) = self.leases.lock() {
            if let Some(lease) = leases.remove(&sid) {
                lease.cancel.cancel();
            }
        }
    }
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|value| value.to_str().map_err(|_| StatusCode::BAD_REQUEST))
        .transpose()?;
    if values.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value)
}
fn timeout_seconds(value: Option<&str>) -> Result<u64, StatusCode> {
    let Some(value) = value else {
        return Ok(300);
    };
    if value == "Second-infinite" {
        return Ok(300);
    }
    let value = value
        .strip_prefix("Second-")
        .ok_or(StatusCode::BAD_REQUEST)?;
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|value| value.is_ascii_digit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let value = value.parse::<u64>().map_err(|_| StatusCode::BAD_REQUEST)?;
    if value == 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value.min(300))
}
fn callbacks(mut value: &str, peer: IpAddr) -> Result<Vec<Url>, StatusCode> {
    if value.len() > 2048 {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    let mut result = Vec::new();
    loop {
        value = value.trim_start();
        if value.is_empty() {
            break;
        }
        if result.len() >= 3 {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
        let rest = value
            .strip_prefix('<')
            .ok_or(StatusCode::PRECONDITION_FAILED)?;
        let (raw, tail) = rest
            .split_once('>')
            .ok_or(StatusCode::PRECONDITION_FAILED)?;
        if raw.len() > 512
            || raw
                .chars()
                .any(|value| value.is_whitespace() || value.is_control())
        {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
        let url = Url::parse(raw).map_err(|_| StatusCode::PRECONDITION_FAILED)?;
        let address = match url.host() {
            Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
            Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
            _ => return Err(StatusCode::PRECONDITION_FAILED),
        };
        if url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() == Some(0)
            || address != peer
            || super::super::lan::validate_bind(address).is_err()
        {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
        result.push(url);
        value = tail;
    }
    if result.is_empty() {
        Err(StatusCode::PRECONDITION_FAILED)
    } else {
        Ok(result)
    }
}
fn granted(sid: &str, seconds: u64) -> Result<Response, StatusCode> {
    let mut response = StatusCode::OK.into_response();
    let sid = HeaderValue::from_str(sid).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let timeout = HeaderValue::from_str(&format!("Second-{seconds}"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    response.headers_mut().insert("sid", sid);
    response.headers_mut().insert("timeout", timeout);
    Ok(response)
}
