use std::sync::{Arc, Mutex};

use super::{pack_boundary, LoopbackHandle};
use crate::{pack::PackError, server};

#[derive(uniffi::Record)]
pub struct LanConfig {
    pub bind_address: String,
    pub port: u16,
    pub max_clients: u32,
    pub pack_ids: Vec<String>,
    pub web_root: String,
    pub identity_dir: String,
    pub media: Option<Arc<server::SharedMedia>>,
}

#[derive(uniffi::Record)]
pub struct LanInfo {
    pub address: String,
    pub port: u16,
    /// Explicit public pairing entry; the unauthenticated origin root is not a bootstrap.
    pub url: String,
    pub certificate_der: Vec<u8>,
    pub spki_sha256: String,
    pub certificate_sha256: String,
    pub friendly_url: String,
    pub native_invitation: String,
}

#[derive(uniffi::Record)]
pub struct LanDevice {
    pub id: String,
    pub label: String,
    pub peer_address: String,
}

#[derive(uniffi::Object)]
pub struct LanHandle {
    server: Mutex<Option<server::ServerHandle>>,
}
#[uniffi::export]
pub fn lan_room(
    handle: Arc<LanHandle>,
    request: crate::room::RoomRequest,
) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle
            .server
            .lock()
            .map_err(|_| crate::room::RoomError::Unavailable)?
            .as_ref()
            .ok_or(crate::room::RoomError::Unavailable)?
            .room_host(request)
    }))
    .unwrap_or(Err(crate::room::RoomError::Unavailable))
}

fn with_server<T>(
    handle: &LanHandle,
    action: impl FnOnce(&server::ServerHandle) -> Result<T, PackError>,
) -> Result<T, PackError> {
    let guard = handle
        .server
        .lock()
        .map_err(|_| PackError::Corrupt("LAN handle lock poisoned".into()))?;
    let server = guard
        .as_ref()
        .ok_or_else(|| PackError::NotFound("stopped LAN server".into()))?;
    action(server)
}

#[uniffi::export]
pub fn server_start_lan(
    source: Arc<LoopbackHandle>,
    config: LanConfig,
) -> Result<Arc<LanHandle>, PackError> {
    pack_boundary(|| {
        let bind = config
            .bind_address
            .parse()
            .map_err(|_| PackError::Mismatch("LAN bind must be an IP address".into()))?;
        let packs = config
            .pack_ids
            .into_iter()
            .map(server::PackId::new)
            .collect::<Result<Vec<_>, _>>()?;
        let max_clients = usize::try_from(config.max_clients)
            .map_err(|_| PackError::Mismatch("invalid LAN device cap".into()))?;
        // Transfer only content ownership, never the private host or unlocked vault.
        let registry = Arc::clone(&source.registry);
        let server = server::Server::start(
            server::ServerCfg {
                bind,
                port: config.port,
                max_clients,
                packs,
                web_root: config.web_root.into(),
                identity_dir: config.identity_dir.into(),
                media: config
                    .media
                    .unwrap_or_else(|| Arc::new(server::SharedMedia::default())),
            },
            registry,
        )?;
        Ok(Arc::new(LanHandle {
            server: Mutex::new(Some(server)),
        }))
    })
}

#[uniffi::export]
pub fn lan_info(handle: Arc<LanHandle>) -> Result<LanInfo, PackError> {
    pack_boundary(|| {
        with_server(&handle, |server| {
            let address = server.local_addr();
            Ok(LanInfo {
                address: address.ip().to_string(),
                port: address.port(),
                url: format!("https://{address}/pair"),
                certificate_der: server.certificate_der(),
                spki_sha256: server.spki_sha256(),
                certificate_sha256: server.certificate_sha256(),
                friendly_url: server.friendly_url(),
                native_invitation: crate::remote::invitation(address, &server.certificate_der()),
            })
        })
    })
}

#[uniffi::export]
pub fn lan_pairing_code(handle: Arc<LanHandle>) -> Result<String, PackError> {
    pack_boundary(|| with_server(&handle, server::ServerHandle::pairing_code))
}

#[uniffi::export]
pub fn lan_devices(handle: Arc<LanHandle>) -> Result<Vec<LanDevice>, PackError> {
    pack_boundary(|| {
        with_server(&handle, |server| {
            Ok(server
                .devices()?
                .into_iter()
                .map(|device| LanDevice {
                    id: device.id,
                    label: device.label,
                    peer_address: device.peer.to_string(),
                })
                .collect())
        })
    })
}

#[uniffi::export]
pub fn lan_kick(handle: Arc<LanHandle>, id: String) -> Result<(), PackError> {
    pack_boundary(|| with_server(&handle, |server| server.kick(&id)))
}

#[uniffi::export]
pub fn server_stop_lan(handle: Arc<LanHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        let server = handle
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("LAN handle lock poisoned".into()))?
            .take();
        if let Some(mut server) = server {
            server.shutdown()?;
        }
        Ok(())
    })
}
