//! The native copy owner holds no private HTTP server, Notes, Admin or AI handle.
use super::{pack_boundary, LoopbackHandle};
use crate::{
    pack::PackError,
    server::{CopyDispatcher, CopyOwner, CopyRunState},
};
use std::{path::Path, sync::Arc};

#[uniffi::export(foreign)]
pub trait NearbyCopyScheduler: Send + Sync {
    /// Admit a user-started service and return its selected Wi-Fi source address.
    fn dispatch(&self, epoch: u64, ipv6: bool) -> Result<String, PackError>;
}
struct Dispatcher(Arc<dyn NearbyCopyScheduler>);
impl CopyDispatcher for Dispatcher {
    fn dispatch(&self, epoch: u64, ipv6: bool) -> Result<std::net::IpAddr, PackError> {
        self.0
            .dispatch(epoch, ipv6)?
            .parse()
            .map_err(|_| PackError::Mismatch("Choose a local Wi-Fi network".into()))
    }
}
#[derive(uniffi::Object)]
pub struct NearbyCopyHandle {
    owner: CopyOwner,
}

#[uniffi::export]
pub fn nearby_copy_open(
    data_dir: String,
    scheduler: Arc<dyn NearbyCopyScheduler>,
) -> Result<Arc<NearbyCopyHandle>, PackError> {
    pack_boundary(|| {
        Ok(Arc::new(NearbyCopyHandle {
            owner: CopyOwner::open(Path::new(&data_dir), Arc::new(Dispatcher(scheduler)))?,
        }))
    })
}
#[uniffi::export]
pub fn nearby_copy_state(handle: Arc<NearbyCopyHandle>) -> Result<CopyRunState, PackError> {
    pack_boundary(|| handle.owner.status())
}
#[uniffi::export]
pub fn nearby_copy_pause(handle: Arc<NearbyCopyHandle>, epoch: u64) -> Result<(), PackError> {
    pack_boundary(|| handle.owner.pause(epoch))
}
#[uniffi::export]
pub fn nearby_copy_close(handle: Arc<NearbyCopyHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        handle.owner.close();
        Ok(())
    })
}
#[uniffi::export]
pub fn server_attach_nearby_copies(
    server: Arc<LoopbackHandle>,
    handle: Arc<NearbyCopyHandle>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback lock".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Loopback".into()))?
            .attach_copy_owner(&handle.owner)
    })
}
