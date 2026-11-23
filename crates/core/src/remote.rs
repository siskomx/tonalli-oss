//! Native, certificate-scoped access to a user-selected Tonalli LAN library.
mod sync;
use std::net::SocketAddr;
use std::time::Duration;
pub use sync::{CopyError, CopyOutcome, CopySession};

use reqwest::{header, Client, Response};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

const JSON_LIMIT: usize = 4 * 1024 * 1024;
const CONTENT_LIMIT: usize = 64 * 1024 * 1024;

/// Public certificate and local endpoint only; never includes a pairing code.
pub fn invitation(address: SocketAddr, certificate: &[u8]) -> String {
    let certificate = certificate
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("tonalli://library?address={address}&certificate={certificate}")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RemoteError {
    #[error("Choose a local Tonalli library")]
    InvalidHost,
    #[error("The library request is invalid")]
    InvalidInput,
    #[error("The library connection is unavailable")]
    Unavailable,
    #[error("The library rejected the request ({0})")]
    Rejected(u16),
    #[error("The library sent an invalid response")]
    InvalidResponse,
    #[error("The library connection was closed")]
    Closed,
}

pub struct RemoteHost {
    address: SocketAddr,
    certificate: Vec<u8>,
}
impl RemoteHost {
    pub fn invitation(value: &str) -> Result<Self, RemoteError> {
        if value.len() > 66000 {
            return Err(RemoteError::InvalidHost);
        }
        let url = Url::parse(value).map_err(|_| RemoteError::InvalidHost)?;
        if url.scheme() != "tonalli"
            || url.host_str() != Some("library")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || !url.path().is_empty()
            || url.fragment().is_some()
        {
            return Err(RemoteError::InvalidHost);
        }
        let pairs = url.query_pairs().collect::<Vec<_>>();
        if pairs.len() != 2
            || pairs.iter().filter(|(key, _)| key == "address").count() != 1
            || pairs.iter().filter(|(key, _)| key == "certificate").count() != 1
        {
            return Err(RemoteError::InvalidHost);
        }
        let address = pairs
            .iter()
            .find(|(key, _)| key == "address")
            .ok_or(RemoteError::InvalidHost)?
            .1
            .parse()
            .map_err(|_| RemoteError::InvalidHost)?;
        let certificate = &pairs
            .iter()
            .find(|(key, _)| key == "certificate")
            .ok_or(RemoteError::InvalidHost)?
            .1;
        if certificate.len() % 2 != 0
            || certificate.len() > 65536
            || !certificate.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(RemoteError::InvalidHost);
        }
        let certificate = (0..certificate.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&certificate[i..i + 2], 16).map_err(|_| RemoteError::InvalidHost)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(address, certificate)
    }
    /// `certificate` must come from the host's explicitly accepted invitation.
    pub fn new(address: SocketAddr, certificate: Vec<u8>) -> Result<Self, RemoteError> {
        if address.port() == 0
            || certificate.is_empty()
            || certificate.len() > 32768
            || crate::server::lan::validate_bind(address.ip()).is_err()
        {
            return Err(RemoteError::InvalidHost);
        }
        Ok(Self {
            address,
            certificate,
        })
    }
}

pub struct RemoteLibrary {
    client: Client,
    source_address: Option<std::net::IpAddr>,
    certificate: Vec<u8>,
    origin: Url,
    api: Zeroizing<String>,
    content: Zeroizing<String>,
    csrf: Zeroizing<String>,
    closed: CancellationToken,
}

pub struct RemoteContent {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

fn pinned_client(
    certificate: &[u8],
    source: Option<std::net::IpAddr>,
) -> Result<Client, RemoteError> {
    Client::builder()
        .local_address(source)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_der(certificate).map_err(|_| RemoteError::InvalidHost)?,
        )
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| RemoteError::Unavailable)
}

impl RemoteLibrary {
    pub(crate) fn uses_ipv6(&self) -> bool {
        matches!(self.origin.host(), Some(url::Host::Ipv6(_)))
    }
    pub(crate) async fn reader_request(
        &self,
        method: reqwest::Method,
        target: &str,
        headers: &header::HeaderMap,
        body: Vec<u8>,
    ) -> Result<Response, RemoteError> {
        let mut url = self.origin.clone();
        let (path, query) = target
            .split_once('?')
            .map_or((target, None), |(path, q)| (path, Some(q)));
        if path.len() > 8192 || query.is_some_and(|q| q.len() > 8192) || !reader_path(path) {
            return Err(RemoteError::InvalidInput);
        }
        let is_api = path.starts_with("/api/");
        let repeatable = method == reqwest::Method::GET || method == reqwest::Method::HEAD;
        if method != reqwest::Method::GET
            && method != reqwest::Method::HEAD
            && !(method == reqwest::Method::POST && path == "/api/room")
        {
            return Err(RemoteError::InvalidInput);
        }
        if body.len() > 16384 {
            return Err(RemoteError::InvalidInput);
        }
        url.set_path(path);
        url.set_query(query);
        if url.origin() != self.origin.origin() || url.path() != path {
            return Err(RemoteError::InvalidInput);
        }
        let mut request = self.client.request(method, url).header(
            header::COOKIE,
            if is_api { &*self.api } else { &*self.content },
        );
        if is_api {
            request = request
                .timeout(Duration::from_secs(30))
                .header("sec-fetch-dest", "empty")
                .header("x-tonalli-csrf", &*self.csrf);
        }
        for name in [
            header::RANGE,
            header::IF_RANGE,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::HeaderName::from_static("sec-fetch-dest"),
        ] {
            if is_api && name.as_str() == "sec-fetch-dest" {
                continue;
            }
            if let Some(value) = headers.get(&name) {
                request = request.header(name, value);
            }
        }
        let request = request.body(body);
        tokio::select! {biased;_=self.closed.cancelled()=>Err(RemoteError::Closed),reply=async{if repeatable{send_backing_off(request).await}else{request.send().await}}=>reply.map_err(|_|RemoteError::Unavailable)}
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.closed.clone()
    }
    pub async fn pair(host: RemoteHost, code: &str, label: &str) -> Result<Self, RemoteError> {
        if code.len() != 6
            || !code.bytes().all(|b| b.is_ascii_digit())
            || label.is_empty()
            || label.len() > 64
            || label.chars().any(char::is_control)
        {
            return Err(RemoteError::InvalidInput);
        }
        let client = pinned_client(&host.certificate, None)?;
        let origin = Url::parse(&format!("https://{}/", host.address))
            .map_err(|_| RemoteError::InvalidHost)?;
        let reply = client
            .post(origin.join("pair").map_err(|_| RemoteError::InvalidHost)?)
            .timeout(Duration::from_secs(30))
            .header(header::CONTENT_TYPE, "application/json")
            .body(
                serde_json::to_vec(&json!({"code":code,"label":label}))
                    .map_err(|_| RemoteError::InvalidInput)?,
            )
            .send()
            .await
            .map_err(|_| RemoteError::Unavailable)?;
        accepted(&reply)?;
        let api = credential(reply.headers(), "tonalli_lan_api")?;
        let content = credential(reply.headers(), "tonalli_lan_content")?;
        let body: Value = serde_json::from_slice(&bounded(reply, 4096).await?)
            .map_err(|_| RemoteError::InvalidResponse)?;
        if body["status"] != "paired" {
            return Err(RemoteError::InvalidResponse);
        }
        let mut library = Self {
            client,
            source_address: None,
            certificate: host.certificate,
            origin,
            api,
            content,
            csrf: Zeroizing::new(String::new()),
            closed: CancellationToken::new(),
        };
        let session = library.json("/api/session", &[]).await?;
        let csrf = session["csrf"]
            .as_str()
            .filter(|s| token(s))
            .ok_or(RemoteError::InvalidResponse)?;
        library.csrf = Zeroizing::new(csrf.to_owned());
        Ok(library)
    }

    pub async fn packs(&self) -> Result<Value, RemoteError> {
        self.json("/api/packs", &[]).await
    }
    pub async fn titles(&self, pack: &str, offset: u64, limit: u64) -> Result<Value, RemoteError> {
        pack_id(pack)?;
        if !(1..=200).contains(&limit) {
            return Err(RemoteError::InvalidInput);
        }
        self.json(
            "/api/zim/titles",
            &[
                ("pack", pack),
                ("offset", &offset.to_string()),
                ("limit", &limit.to_string()),
            ],
        )
        .await
    }

    pub async fn suggest(&self, pack: &str, query: &str, limit: u64) -> Result<Value, RemoteError> {
        pack_id(pack)?;
        if query.len() > 4096 || !(1..=200).contains(&limit) {
            return Err(RemoteError::InvalidInput);
        }
        self.json(
            "/api/zim/suggest",
            &[("pack", pack), ("q", query), ("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn search(&self, pack: &str, query: &str, limit: u64) -> Result<Value, RemoteError> {
        pack_id(pack)?;
        if query.len() > 4096 || !(1..=20).contains(&limit) {
            return Err(RemoteError::InvalidInput);
        }
        self.json(
            "/api/zim/search",
            &[("pack", pack), ("q", query), ("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn article(&self, pack: &str, url: &str) -> Result<Value, RemoteError> {
        pack_id(pack)?;
        entry_path(url)?;
        self.json("/api/zim/article", &[("pack", pack), ("url", url)])
            .await
    }

    pub async fn content(&self, pack: &str, path: &str) -> Result<RemoteContent, RemoteError> {
        let info = self.article(pack, path).await?;
        if info["pack"].as_str() != Some(pack) {
            return Err(RemoteError::InvalidResponse);
        }
        let canonical = info["url"].as_str().ok_or(RemoteError::InvalidResponse)?;
        entry_path(canonical)?;
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| RemoteError::InvalidHost)?
            .clear()
            .extend(["zim", pack])
            .extend(canonical.split('/'));
        tokio::select! {
            biased;
            _=self.closed.cancelled()=>Err(RemoteError::Closed),
            result=async {
                let reply=send_backing_off(self.client.get(url).header(header::COOKIE,&*self.content))
                    .await.map_err(|_|RemoteError::Unavailable)?;
                accepted(&reply)?;
                let mime_type=reply.headers().get(header::CONTENT_TYPE).and_then(|h|h.to_str().ok())
                    .filter(|s|s.len()<=256).ok_or(RemoteError::InvalidResponse)?.to_owned();
                Ok(RemoteContent{mime_type,bytes:bounded(reply,CONTENT_LIMIT).await?})
            }=>result,
        }
    }

    pub fn close(&self) {
        self.closed.cancel();
    }

    async fn json(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, RemoteError> {
        self.json_bounded(path, query, JSON_LIMIT).await
    }
    async fn json_bounded(
        &self,
        path: &str,
        query: &[(&str, &str)],
        limit: usize,
    ) -> Result<Value, RemoteError> {
        let mut url = self
            .origin
            .join(path)
            .map_err(|_| RemoteError::InvalidInput)?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        tokio::select! {
            biased;
            _=self.closed.cancelled()=>Err(RemoteError::Closed),
            result=async {
                let reply=send_backing_off(self.client.get(url).timeout(Duration::from_secs(30)).header(header::COOKIE,&*self.api)
                    .header("sec-fetch-dest","empty").header("x-tonalli-csrf",&*self.csrf))
                    .await.map_err(|_|RemoteError::Unavailable)?;
                accepted(&reply)?;
                serde_json::from_slice(&bounded(reply,limit).await?).map_err(|_|RemoteError::InvalidResponse)
            }=>result,
        }
    }
}

pub(crate) fn reader_path(path: &str) -> bool {
    if path.contains(['\\', '\0']) || !path.starts_with('/') {
        return false;
    }
    let decoded = match percent_encoding::percent_decode_str(path).decode_utf8() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if decoded.contains(['\\', '\0', ':', '%'])
        || decoded
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return false;
    }
    matches!(
        path,
        "/api/packs"
            | "/api/maps"
            | "/api/zim/titles"
            | "/api/zim/suggest"
            | "/api/zim/search"
            | "/api/zim/article"
            | "/api/shared-media"
            | "/api/room"
    ) || (path.starts_with("/zim/") && decoded.split('/').count() >= 4)
        || (path.starts_with("/packs/") && decoded.split('/').count() == 3)
        || (path.starts_with("/shared-media/")
            && decoded.split('/').count() == 4
            && matches!(decoded.rsplit('/').next(), Some("content" | "download")))
        || (path.starts_with("/api/shared-media/")
            && decoded.split('/').count() == 5
            && decoded.ends_with("/text"))
}

/// The host limits how fast one device may ask, so a quick run of reads (copying a book with many
/// search parts) can be told to wait. Retry a few times with a growing pause, then report the limit.
/// Only for repeatable reads: never for pairing or posting.
const RATE_LIMIT_RETRIES: u32 = 4;

async fn send_backing_off(request: reqwest::RequestBuilder) -> reqwest::Result<Response> {
    let mut pause = Duration::from_millis(150);
    for _ in 0..RATE_LIMIT_RETRIES {
        let Some(attempt) = request.try_clone() else {
            break;
        };
        let reply = attempt.send().await?;
        if reply.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Ok(reply);
        }
        tokio::time::sleep(pause).await;
        pause *= 2;
    }
    request.send().await
}

fn accepted(reply: &Response) -> Result<(), RemoteError> {
    if reply.status() == reqwest::StatusCode::OK {
        Ok(())
    } else {
        Err(RemoteError::Rejected(reply.status().as_u16()))
    }
}
async fn bounded(mut reply: Response, limit: usize) -> Result<Vec<u8>, RemoteError> {
    if reply.content_length().is_some_and(|n| n > limit as u64) {
        return Err(RemoteError::InvalidResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = reply.chunk().await.map_err(|_| RemoteError::Unavailable)? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(RemoteError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn credential(headers: &header::HeaderMap, name: &str) -> Result<Zeroizing<String>, RemoteError> {
    let mut found = None;
    for h in headers.get_all(header::SET_COOKIE) {
        let cookie = h
            .to_str()
            .map_err(|_| RemoteError::InvalidResponse)?
            .split(';')
            .next()
            .unwrap_or("");
        if let Some((key, value)) = cookie.split_once('=') {
            if key == name {
                if found.is_some() || !token(value) {
                    return Err(RemoteError::InvalidResponse);
                }
                found = Some(Zeroizing::new(format!("{name}={value}")));
            }
        }
    }
    found.ok_or(RemoteError::InvalidResponse)
}
fn pack_id(pack: &str) -> Result<(), RemoteError> {
    crate::server::PackId::new(pack)
        .map(|_| ())
        .map_err(|_| RemoteError::InvalidInput)
}
fn entry_path(path: &str) -> Result<(), RemoteError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        Err(RemoteError::InvalidInput)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Serves `connections` requests, one connection each: the first `limited` get a 429, the rest 200.
    fn rate_limited_server(
        limited: usize,
        connections: usize,
    ) -> (String, std::thread::JoinHandle<usize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for stream in listener.incoming().take(connections) {
                let mut stream = stream.unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let reply = if served < limited {
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                stream.write_all(reply.as_bytes()).unwrap();
                served += 1;
            }
            served
        });
        (url, handle)
    }

    #[tokio::test]
    async fn a_rate_limited_read_is_retried_instead_of_failing_the_copy() {
        let (url, server) = rate_limited_server(3, 4);
        let reply = send_backing_off(reqwest::Client::new().get(&url))
            .await
            .unwrap();
        assert_eq!(reply.status(), reqwest::StatusCode::OK);
        assert_eq!(server.join().unwrap(), 4);
    }

    #[tokio::test]
    async fn retrying_a_rate_limited_read_is_bounded() {
        let attempts = RATE_LIMIT_RETRIES as usize + 1;
        let (url, server) = rate_limited_server(attempts, attempts);
        let reply = send_backing_off(reqwest::Client::new().get(&url))
            .await
            .unwrap();
        assert_eq!(reply.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(server.join().unwrap(), attempts);
    }
}
