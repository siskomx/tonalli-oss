//! Per-attempt Windows Wi-Fi selection, client binding and DNS ownership.
#[cfg(windows)]
#[path = "network/adapters.rs"]
mod adapters;
#[path = "network/dns.rs"]
mod dns;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    index: u32,
    luid: u64,
}
#[derive(Clone, Copy)]
struct Adapter {
    identity: Identity,
    ieee80211: bool,
    operational: bool,
    connected: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NetworkError {
    WaitingWifi,
    Revoked,
    Dns,
    Network,
    #[cfg(test)]
    Timeout,
}
impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NetworkError {}

pub(crate) struct WifiLease {
    identity: Identity,
    revoked: std::sync::atomic::AtomicBool,
    pending: std::sync::Mutex<Vec<std::sync::Arc<dns::PendingDns>>>,
}
impl WifiLease {
    fn from_adapters(adapters: &[Adapter]) -> Result<Self, NetworkError> {
        let identity = adapters
            .iter()
            .filter(|a| {
                a.ieee80211
                    && a.operational
                    && a.connected
                    && a.identity.index != 0
                    && a.identity.luid != 0
            })
            .map(|a| a.identity)
            .min_by_key(|i| (i.index, i.luid))
            .ok_or(NetworkError::WaitingWifi)?;
        Ok(Self {
            identity,
            revoked: std::sync::atomic::AtomicBool::new(false),
            pending: std::sync::Mutex::new(Vec::with_capacity(2)),
        })
    }
    fn observe(&self, adapters: &[Adapter]) -> Result<(), NetworkError> {
        use std::sync::atomic::Ordering;
        if !adapters
            .iter()
            .any(|a| a.identity == self.identity && a.ieee80211 && a.operational && a.connected)
        {
            self.revoked.store(true, Ordering::Release);
        }
        if self.revoked.load(Ordering::Acquire) {
            Err(NetworkError::Revoked)
        } else {
            Ok(())
        }
    }
    fn track_pending(&self, query: std::sync::Arc<dns::PendingDns>) -> Result<(), NetworkError> {
        let mut pending = self.pending.lock().map_err(|_| NetworkError::Dns)?;
        if self.revoked.load(std::sync::atomic::Ordering::Acquire) {
            return Err(NetworkError::Revoked);
        }
        pending.retain(|q| !q.completed());
        if pending.len() == 2 {
            return Err(NetworkError::Dns);
        }
        pending.push(query);
        Ok(())
    }
    pub(crate) fn finish(&self) -> Result<(), NetworkError> {
        self.revoked
            .store(true, std::sync::atomic::Ordering::Release);
        let pending = self.pending.lock().map_err(|_| NetworkError::Dns)?.clone();
        // Native cancellation and callback draining never hold the attempt lock.
        for query in &pending {
            query.cancel_owned();
        }
        for query in &pending {
            query.drain()?;
        }
        self.pending.lock().map_err(|_| NetworkError::Dns)?.clear();
        Ok(())
    }
    pub(crate) fn revoke(&self) {
        self.revoked
            .store(true, std::sync::atomic::Ordering::Release);
        let pending = self.pending.lock().map(|queries| queries.clone());
        if let Ok(pending) = pending {
            for query in pending {
                query.cancel_owned();
            }
        }
    }
}

#[cfg(windows)]
pub(crate) fn client(
) -> Result<(std::sync::Arc<WifiLease>, reqwest::blocking::Client), NetworkError> {
    let lease = std::sync::Arc::new(WifiLease::select_native()?);
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .windows_interface(lease.identity.index, lease.identity.luid)
        .dns_resolver(std::sync::Arc::new(dns::WifiResolver(
            std::sync::Arc::clone(&lease),
        )))
        .pool_max_idle_per_host(0)
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| NetworkError::Network)?;
    Ok((lease, client))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter(index: u32, luid: u64, wifi: bool) -> Adapter {
        Adapter {
            identity: Identity { index, luid },
            ieee80211: wifi,
            operational: true,
            connected: true,
        }
    }
    #[test]
    fn no_wifi_never_selects_an_operational_ethernet_or_cellular_fallback() {
        assert!(matches!(
            WifiLease::from_adapters(&[adapter(2, 20, false), adapter(3, 30, false)]),
            Err(NetworkError::WaitingWifi)
        ));
        let mut down = adapter(4, 40, true);
        down.operational = false;
        assert!(matches!(
            WifiLease::from_adapters(&[down]),
            Err(NetworkError::WaitingWifi)
        ));
        assert!(matches!(
            WifiLease::from_adapters(&[adapter(0, 40, true), adapter(4, 0, true)]),
            Err(NetworkError::WaitingWifi)
        ));
    }
    #[test]
    fn captured_identity_not_enumeration_order_is_retained_and_loss_is_sticky() {
        let wifi = adapter(4, 40, true);
        let second = adapter(5, 50, true);
        let lease = WifiLease::from_adapters(&[second, wifi]).unwrap();
        assert_eq!(lease.identity, wifi.identity);
        assert_eq!(lease.observe(&[wifi, second]), Ok(()));
        // Same index now belongs to a different adapter, including a non-Wi-Fi one.
        assert_eq!(
            lease.observe(&[adapter(4, 99, false), second]),
            Err(NetworkError::Revoked)
        );
        assert_eq!(lease.observe(&[wifi, second]), Err(NetworkError::Revoked));
    }
}
