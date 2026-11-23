mod discovery;
mod http;
mod identity;
mod sessions;
mod streams;
mod transport;
pub(super) use streams::{wrap_selected_media, Bucket as SelectedBandwidth};

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::{ContentState, PackId, PackRegistry};
use crate::pack::PackError;

pub struct Server;
const PREFERRED_PORT: u16 = 8443;
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(2);

pub struct ServerCfg {
    pub bind: IpAddr,
    pub port: u16,
    pub max_clients: usize,
    pub packs: Vec<PackId>,
    pub web_root: PathBuf,
    pub identity_dir: PathBuf,
    pub media: Arc<super::SharedMedia>,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            bind: Ipv4Addr::LOCALHOST.into(),
            port: 0,
            max_clients: 8,
            packs: Vec::new(),
            web_root: PathBuf::new(),
            identity_dir: PathBuf::new(),
            media: Arc::new(super::SharedMedia::default()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub peer: IpAddr,
}

pub struct ServerHandle {
    address: SocketAddr,
    certificate: Vec<u8>,
    pin: String,
    sessions: Arc<Mutex<sessions::Sessions>>,
    stop: CancellationToken,
    worker: Option<JoinHandle<Result<(), PackError>>>,
    name_published: bool,
}

impl Server {
    pub fn start(cfg: ServerCfg, registry: Arc<PackRegistry>) -> Result<ServerHandle, PackError> {
        validate_bind(cfg.bind)?;
        if !(1..=128).contains(&cfg.max_clients)
            || cfg.web_root.as_os_str().is_empty()
            || cfg.identity_dir.as_os_str().is_empty()
        {
            return Err(PackError::Mismatch(
                "LAN needs explicit resource paths and a device cap of 1..128".into(),
            ));
        }
        let web_root = cfg.web_root.canonicalize()?;
        if !web_root.is_dir() || !web_root.join("index.html").is_file() {
            return Err(PackError::Mismatch(
                "LAN web root must contain a built index.html".into(),
            ));
        }
        std::fs::create_dir_all(&cfg.identity_dir)?;
        let identity_dir = cfg.identity_dir.canonicalize()?;
        if identity_dir.starts_with(&web_root) || web_root.starts_with(&identity_dir) {
            return Err(PackError::Mismatch(
                "LAN identity and web roots must not overlap".into(),
            ));
        }
        let allowed: std::collections::HashSet<_> = cfg.packs.into_iter().collect();
        {
            let zims = registry
                .zims
                .read()
                .map_err(|_| super::registry_poisoned())?;
            let tiles = registry
                .tiles
                .read()
                .map_err(|_| super::registry_poisoned())?;
            if allowed
                .iter()
                .any(|id| !zims.contains_key(id) && !tiles.contains_key(id))
            {
                return Err(PackError::Mismatch(
                    "LAN shares may name installed packs only".into(),
                ));
            }
        }
        let identity = identity::load(&identity_dir, cfg.bind)?;
        let sessions = Arc::new(Mutex::new(sessions::Sessions::new(
            cfg.max_clients,
            Instant::now(),
        )?));
        let stop = sessions.lock().map_err(|_| poisoned())?.stopped.clone();
        // Port 0 on a real LAN address: prefer a stable port so the link survives restarts.
        let socket = match cfg.port {
            0 if !cfg.bind.is_loopback() => TcpListener::bind((cfg.bind, PREFERRED_PORT))
                .or_else(|_| TcpListener::bind((cfg.bind, 0)))?,
            port => TcpListener::bind((cfg.bind, port))?,
        };
        socket.set_nonblocking(true)?;
        let address = socket.local_addr()?;
        let content = Arc::new(ContentState {
            registry,
            allowed: Some(allowed),
            web_root,
            media: cfg.media,
        });
        let state = Arc::new(
            http::HttpState::new(Arc::clone(&sessions), address.to_string(), stop.clone())
                .with_hostname(format!("{}:{}", identity.hostname, address.port())),
        );
        let app = http::router(content, state);
        let mut name_published = true;
        let mut discovery =
            match discovery::Discovery::start(cfg.bind, address.port(), &identity.spki_sha256) {
                Ok(discovery) => discovery,
                // Sideloaded iPhone apps cannot join multicast groups, so the `.local` name stays
                // unpublished; the address and QR code still work.
                Err(_) if cfg!(target_os = "ios") => {
                    name_published = false;
                    None
                }
                Err(error) => return Err(error),
            };
        let (ready, readiness) = std::sync::mpsc::sync_channel(1);
        let worker_stop = stop.clone();
        let config = identity.config;
        let worker = thread::Builder::new()
            .name("tonalli-lan".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(4)
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let error = PackError::Io(error.to_string());
                        let _ = ready.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let (result, remaining) = runtime.block_on(async {
                    let socket = match tokio::net::TcpListener::from_std(socket) {
                        Ok(socket) => socket,
                        Err(error) => {
                            let error = PackError::Io(error.to_string());
                            let _ = ready.send(Err(error.clone()));
                            return (Err(error), Duration::ZERO);
                        }
                    };
                    let listener = transport::Listener::new(socket, config, worker_stop.clone());
                    if ready.send(Ok(())).is_err() {
                        return (
                            Err(PackError::Io("LAN startup caller disappeared".into())),
                            Duration::ZERO,
                        );
                    }
                    run(listener, app, worker_stop.clone(), discovery.as_ref()).await
                });
                worker_stop.cancel();
                runtime.shutdown_timeout(remaining);
                let withdrawal = discovery
                    .as_mut()
                    .map_or(Ok(()), discovery::Discovery::stop);
                result.and(withdrawal)
            })?;
        let mut handle = ServerHandle {
            address,
            certificate: identity.certificate_der,
            pin: identity.spki_sha256,
            sessions,
            stop,
            worker: Some(worker),
            name_published,
        };
        match readiness.recv() {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => {
                let _ = handle.shutdown();
                Err(error)
            }
            Err(_) => {
                let _ = handle.shutdown();
                Err(PackError::Io("LAN runtime did not become ready".into()))
            }
        }
    }
}

impl ServerHandle {
    pub fn room_host(
        &self,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
        self.sessions
            .lock()
            .map_err(|_| crate::room::RoomError::Unavailable)?
            .room_host(request)
    }
    pub fn friendly_url(&self) -> String {
        if !self.name_published {
            return format!("https://{}/pair", self.address);
        }
        format!(
            "https://{}:{}/pair",
            identity::HOSTNAME,
            self.address.port()
        )
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
    pub fn certificate_der(&self) -> Vec<u8> {
        self.certificate.clone()
    }
    pub fn certificate_sha256(&self) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(&self.certificate)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }
    pub fn spki_sha256(&self) -> String {
        self.pin.clone()
    }
    pub fn pairing_code(&self) -> Result<String, PackError> {
        if self.stop.is_cancelled() {
            return Err(PackError::Mismatch(
                "Sharing stopped; check the network or a local-name conflict".into(),
            ));
        }
        self.sessions
            .lock()
            .map_err(|_| poisoned())?
            .code(Instant::now())
    }
    pub fn devices(&self) -> Result<Vec<Device>, PackError> {
        Ok(self.sessions.lock().map_err(|_| poisoned())?.devices())
    }
    pub fn kick(&self, id: &str) -> Result<(), PackError> {
        if self.sessions.lock().map_err(|_| poisoned())?.kick(id) {
            Ok(())
        } else {
            Err(PackError::NotFound("LAN device".into()))
        }
    }
    pub fn shutdown(&mut self) -> Result<(), PackError> {
        self.stop.cancel();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.stop();
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| PackError::Corrupt("LAN runtime thread panicked".into()))?
        } else {
            Ok(())
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn poisoned() -> PackError {
    PackError::Corrupt("LAN session lock poisoned".into())
}

async fn run(
    listener: transport::Listener,
    app: axum::Router,
    stop: CancellationToken,
    discovery: Option<&discovery::Discovery>,
) -> (Result<(), PackError>, Duration) {
    use std::future::IntoFuture;
    let serving = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<transport::Peer>(),
    )
    .with_graceful_shutdown(stop.clone().cancelled_owned())
    .into_future();
    tokio::pin!(serving);
    tokio::select! {
        result = &mut serving => (result.map_err(Into::into), SHUTDOWN_LIMIT),
        _ = stop.cancelled() => {
            let started = Instant::now();
            let result = match tokio::time::timeout(SHUTDOWN_LIMIT, &mut serving).await { Ok(result) => result.map_err(Into::into), Err(_) => Ok(()) };
            (result, SHUTDOWN_LIMIT.saturating_sub(started.elapsed()))
        },
        failure = async {
            let Some(discovery) = discovery else { return std::future::pending::<PackError>().await; };
            loop { tokio::time::sleep(Duration::from_millis(250)).await; if let Err(error) = discovery.check() { return error; } }
        } => (Err(failure), SHUTDOWN_LIMIT),
    }
}

pub(crate) fn validate_bind(bind: IpAddr) -> Result<(), PackError> {
    let allowed = match bind {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            ip.to_ipv4_mapped().is_none() && (ip.is_loopback() || ip.is_unique_local())
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(PackError::Mismatch(
            "LAN bind must be literal loopback, RFC1918, IPv4 link-local or IPv6 ULA".into(),
        ))
    }
}

#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod sessions_tests;
#[cfg(test)]
mod streams_tests;
#[cfg(test)]
mod tests;
