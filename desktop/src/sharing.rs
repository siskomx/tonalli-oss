//! Native LAN address selection; TLS, pairing and content isolation stay in Core.
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tonalli_core::{
    pack::PackError,
    server::{
        PackId, PackRegistry, Server, ServerCfg, ServerHandle, SharingControl, SharingDevice,
        SharingNetwork, SharingStatus,
    },
};

pub struct DesktopSharing {
    registry: Arc<PackRegistry>,
    web: PathBuf,
    identity: PathBuf,
    owner: Mutex<Owner>,
}
struct Owner {
    server: Option<ServerHandle>,
    closed: bool,
}
impl DesktopSharing {
    pub fn new(registry: Arc<PackRegistry>, web: PathBuf, identity: PathBuf) -> Self {
        Self {
            registry,
            web,
            identity,
            owner: Mutex::new(Owner {
                server: None,
                closed: false,
            }),
        }
    }
}
fn unavailable() -> PackError {
    PackError::Mismatch("Sharing unavailable; check your local network".into())
}
impl SharingControl for DesktopSharing {
    fn room(
        &self,
        request: tonalli_core::room::RoomRequest,
    ) -> Result<tonalli_core::room::RoomSnapshot, tonalli_core::room::RoomError> {
        use tonalli_core::room::{RoomAction, RoomError, RoomSnapshot};
        let owner = self.owner.lock().map_err(|_| RoomError::Unavailable)?;
        match owner.server.as_ref() {
            Some(server) if !owner.closed => server.room_host(request),
            _ if matches!(request.action, RoomAction::Get | RoomAction::Leave) => {
                Ok(RoomSnapshot::default())
            }
            _ => Err(RoomError::Unavailable),
        }
    }
    fn status(&self) -> Result<SharingStatus, PackError> {
        let mut owner = self.owner.lock().map_err(|_| unavailable())?;
        let networks = networks()?;
        if owner.server.as_ref().is_some_and(|s| {
            !networks
                .iter()
                .any(|n| n.id == s.local_addr().ip().to_string())
        }) {
            if let Some(mut server) = owner.server.take() {
                server.shutdown()?;
            }
        }
        let server = owner.server.as_ref();
        Ok(SharingStatus {
            running: server.is_some(),
            busy: false,
            url: server.map(|s| format!("https://{}/pair", s.local_addr())),
            pairing_code: server.map(|s| s.pairing_code()).transpose()?,
            devices: server
                .map(|s| s.devices())
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .map(|d| SharingDevice {
                    id: d.id,
                    label: d.label,
                })
                .collect(),
            certificate_sha256: server.map(|s| s.certificate_sha256()),
            friendly_url: server.map(|s| s.friendly_url()),
            native_invitation: server
                .map(|s| tonalli_core::remote::invitation(s.local_addr(), &s.certificate_der())),
            networks,
            error: None,
            hotspot: None,
        })
    }
    fn start(
        &self,
        network: String,
        packs: Vec<String>,
        media: Arc<tonalli_core::server::SharedMedia>,
    ) -> Result<(), PackError> {
        let mut owner = self.owner.lock().map_err(|_| unavailable())?;
        if owner.closed || owner.server.is_some() || !networks()?.iter().any(|n| n.id == network) {
            return Err(unavailable());
        }
        owner.server = Some(Server::start(
            ServerCfg {
                bind: network.parse::<IpAddr>().map_err(|_| unavailable())?,
                port: 0,
                max_clients: 8,
                packs: packs
                    .into_iter()
                    .map(PackId::new)
                    .collect::<Result<_, _>>()?,
                web_root: self.web.clone(),
                identity_dir: self.identity.clone(),
                media,
            },
            Arc::clone(&self.registry),
        )?);
        Ok(())
    }
    fn stop(&self) -> Result<(), PackError> {
        let mut owner = self.owner.lock().map_err(|_| unavailable())?;
        if let Some(mut server) = owner.server.take() {
            server.shutdown()?;
        }
        Ok(())
    }
    fn kick(&self, id: String) -> Result<(), PackError> {
        self.owner
            .lock()
            .map_err(|_| unavailable())?
            .server
            .as_ref()
            .ok_or_else(unavailable)?
            .kick(&id)
    }
    fn close(&self) -> Result<(), PackError> {
        let mut owner = self.owner.lock().map_err(|_| unavailable())?;
        owner.closed = true;
        if let Some(mut server) = owner.server.take() {
            server.shutdown()?;
        }
        Ok(())
    }
}
impl Drop for DesktopSharing {
    fn drop(&mut self) {
        if let Ok(owner) = self.owner.get_mut() {
            owner.closed = true;
            if let Some(mut s) = owner.server.take() {
                let _ = s.shutdown();
            }
        }
    }
}

pub fn eligible_addresses() -> Result<Vec<IpAddr>, PackError> {
    Ok(networks()?
        .into_iter()
        .filter_map(|network| network.id.parse().ok())
        .collect())
}

/// iPhone interface names: en* is Wi-Fi, bridge* is this phone's own Personal Hotspot. Cellular
/// (pdp_ip*), tunnels, AirDrop links and loopback are never offered, and neither are public addresses.
#[cfg(any(test, target_os = "ios"))]
fn apple_choices(interfaces: impl IntoIterator<Item = (String, Ipv4Addr)>) -> Vec<SharingNetwork> {
    let mut choices: Vec<SharingNetwork> = interfaces
        .into_iter()
        .filter_map(|(name, ip)| {
            let label = if name.starts_with("bridge") {
                "Personal Hotspot"
            } else if name.starts_with("en") {
                "Wi-Fi"
            } else {
                return None;
            };
            (ip.is_private() || ip.is_link_local()).then(|| SharingNetwork {
                id: ip.to_string(),
                label: label.into(),
            })
        })
        .collect();
    choices.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    choices.dedup_by(|a, b| a.id == b.id);
    choices
}

/// Wi-Fi-only downloads: an en* interface with a real IPv4 lease. Link-local means no DHCP; cellular,
/// tunnels, loopback and the phone's own Personal Hotspot are not Wi-Fi.
#[cfg(any(test, target_os = "ios"))]
fn wifi_present(interfaces: impl IntoIterator<Item = (String, Ipv4Addr)>) -> bool {
    interfaces.into_iter().any(|(name, ip)| {
        name.starts_with("en") && !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local()
    })
}

#[cfg(target_os = "ios")]
pub fn wifi_available() -> bool {
    if_addrs::get_if_addrs().is_ok_and(|interfaces| {
        wifi_present(interfaces.into_iter().filter_map(|interface| {
            match interface.ip() {
                IpAddr::V4(ip) => Some((interface.name, ip)),
                IpAddr::V6(_) => None,
            }
        }))
    })
}

#[cfg(target_os = "ios")]
pub(crate) fn networks() -> Result<Vec<SharingNetwork>, PackError> {
    let interfaces = if_addrs::get_if_addrs().map_err(|_| unavailable())?;
    Ok(apple_choices(interfaces.into_iter().filter_map(|interface| {
        match interface.ip() {
            IpAddr::V4(ip) => Some((interface.name, ip)),
            IpAddr::V6(_) => None,
        }
    })))
}

#[cfg(windows)]
pub(crate) fn networks() -> Result<Vec<SharingNetwork>, PackError> {
    use windows_sys::Win32::{
        NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
        Networking::WinSock::{AF_INET, SOCKADDR_IN},
    };
    // u64 storage provides the native structure alignment, including on ARM64.
    let mut bytes = 16_384u32;
    for _ in 0..3 {
        let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
        let head = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let result =
            unsafe { GetAdaptersAddresses(AF_INET as u32, 0, std::ptr::null(), head, &mut bytes) };
        if result == 111 {
            continue;
        }
        if result != 0 {
            return Err(unavailable());
        }
        let mut choices = Vec::new();
        let mut adapter = head;
        unsafe {
            while !adapter.is_null() {
                let a = &*adapter;
                if a.OperStatus == IfOperStatusUp
                    && (a.IfType == IF_TYPE_IEEE80211 || a.IfType == IF_TYPE_ETHERNET_CSMACD)
                {
                    let label = if a.FriendlyName.is_null() {
                        "Local network".into()
                    } else {
                        let mut len = 0;
                        while *a.FriendlyName.add(len) != 0 {
                            len += 1;
                        }
                        String::from_utf16_lossy(std::slice::from_raw_parts(a.FriendlyName, len))
                    };
                    let mut entry = a.FirstUnicastAddress;
                    while !entry.is_null() {
                        let socket = (*entry).Address;
                        if !socket.lpSockaddr.is_null()
                            && socket.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN>()
                            && (*socket.lpSockaddr).sa_family == AF_INET
                        {
                            let address = &*socket.lpSockaddr.cast::<SOCKADDR_IN>();
                            let ip = Ipv4Addr::from(address.sin_addr.S_un.S_addr.to_ne_bytes());
                            if ip.is_private() || ip.is_link_local() {
                                choices.push(SharingNetwork {
                                    id: ip.to_string(),
                                    label: label.clone(),
                                });
                            }
                        }
                        entry = (*entry).Next;
                    }
                }
                adapter = a.Next;
            }
        }
        choices.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        choices.dedup_by(|a, b| a.id == b.id);
        return Ok(choices);
    }
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(rows: &[(&str, &str)]) -> Vec<(String, Ipv4Addr)> {
        rows.iter()
            .map(|(name, ip)| ((*name).to_owned(), ip.parse().unwrap()))
            .collect()
    }

    #[test]
    fn iphone_offers_only_wifi_and_its_own_hotspot_on_private_addresses() {
        let choices = apple_choices(pairs(&[
            ("lo0", "127.0.0.1"),
            ("pdp_ip0", "10.77.3.9"),
            ("utun2", "10.8.0.4"),
            ("awdl0", "169.254.7.7"),
            ("en0", "192.168.1.20"),
            ("en0", "192.168.1.20"),
            ("en2", "8.8.8.8"),
            ("bridge100", "172.20.10.1"),
            ("en1", "169.254.3.3"),
        ]));
        let shown: Vec<_> = choices
            .iter()
            .map(|choice| (choice.label.as_str(), choice.id.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Personal Hotspot", "172.20.10.1"),
                ("Wi-Fi", "169.254.3.3"),
                ("Wi-Fi", "192.168.1.20"),
            ]
        );
    }

    #[test]
    fn only_a_real_wifi_address_counts_as_wifi_for_downloads() {
        assert!(wifi_present(pairs(&[("en0", "192.168.1.20")])));
        assert!(wifi_present(pairs(&[("pdp_ip0", "10.1.2.3"), ("en1", "10.0.0.7")])));
        // Link-local means no DHCP lease; the phone's own hotspot, cellular, tunnels and loopback are not Wi-Fi.
        assert!(!wifi_present(pairs(&[("en0", "169.254.3.3")])));
        assert!(!wifi_present(pairs(&[("bridge100", "172.20.10.1")])));
        assert!(!wifi_present(pairs(&[("pdp_ip0", "10.77.3.9"), ("utun2", "10.8.0.4"), ("lo0", "127.0.0.1")])));
        assert!(!wifi_present(Vec::new()));
    }

    #[test]
    fn no_usable_interface_means_no_sharing_choice() {
        assert!(apple_choices(pairs(&[("lo0", "127.0.0.1"), ("pdp_ip0", "10.0.0.2")])).is_empty());
        assert!(apple_choices(Vec::new()).is_empty());
    }
}
