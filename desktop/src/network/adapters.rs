use super::{Adapter, Identity, NetworkError, WifiLease};
use windows_sys::Win32::NetworkManagement::{
    IpHelper::{FreeMibTable, GetIfTable2, IF_TYPE_IEEE80211, MIB_IF_TABLE2},
    Ndis::{IfOperStatusUp, MediaConnectStateConnected},
};

struct Table(*mut MIB_IF_TABLE2);
impl Drop for Table {
    fn drop(&mut self) {
        unsafe {
            FreeMibTable(self.0.cast());
        }
    }
}

fn enumerate() -> Result<Vec<Adapter>, NetworkError> {
    let mut raw = std::ptr::null_mut();
    if unsafe { GetIfTable2(&mut raw) } != 0 || raw.is_null() {
        return Err(NetworkError::Network);
    }
    let table = Table(raw);
    let count = unsafe { (*table.0).NumEntries } as usize;
    // Bound even native-provided allocation lengths before creating a slice.
    if count > 16_384 {
        return Err(NetworkError::Network);
    }
    let rows =
        unsafe { std::slice::from_raw_parts(std::ptr::addr_of!((*table.0).Table).cast(), count) };
    Ok(rows
        .iter()
        .map(
            |row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2| Adapter {
                identity: Identity {
                    index: row.InterfaceIndex,
                    luid: unsafe { row.InterfaceLuid.Value },
                },
                ieee80211: row.Type == IF_TYPE_IEEE80211,
                operational: row.OperStatus == IfOperStatusUp,
                connected: row.MediaConnectState == MediaConnectStateConnected,
            },
        )
        .collect())
}

impl WifiLease {
    pub(super) fn select_native() -> Result<Self, NetworkError> {
        Self::from_adapters(&enumerate()?)
    }
    pub(crate) fn observe_native(&self) -> Result<(), NetworkError> {
        match enumerate() {
            Ok(adapters) => self.observe(&adapters),
            Err(_) => {
                self.revoked
                    .store(true, std::sync::atomic::Ordering::Release);
                Err(NetworkError::Revoked)
            }
        }
    }
}
