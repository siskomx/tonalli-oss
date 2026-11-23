use super::LoopbackHandle;
use crate::{
    pack::PackError,
    server::{PackRegistry, SharingControl, SharingStatus},
};
use std::sync::{Arc, Mutex};

#[uniffi::export(foreign)]
pub trait SharingScheduler: Send + Sync {
    fn room(
        &self,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError>;
    fn status(&self) -> Result<SharingStatus, PackError>;
    fn start(
        &self,
        source: Arc<LoopbackHandle>,
        network: String,
        packs: Vec<String>,
        media: Arc<crate::server::SharedMedia>,
    ) -> Result<(), PackError>;
    fn stop(&self) -> Result<(), PackError>;
    fn kick(&self, id: String) -> Result<(), PackError>;
}
struct Dispatcher {
    registry: Arc<PackRegistry>,
    scheduler: Arc<dyn SharingScheduler>,
    closed: Mutex<bool>,
}
impl SharingControl for Dispatcher {
    fn room(
        &self,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
        if *self
            .closed
            .lock()
            .map_err(|_| crate::room::RoomError::Unavailable)?
        {
            return Err(crate::room::RoomError::Unavailable);
        }
        self.scheduler.room(request)
    }
    fn status(&self) -> Result<SharingStatus, PackError> {
        self.scheduler.status()
    }
    fn start(
        &self,
        network: String,
        packs: Vec<String>,
        media: Arc<crate::server::SharedMedia>,
    ) -> Result<(), PackError> {
        let closed = self
            .closed
            .lock()
            .map_err(|_| PackError::Corrupt("Sharing lock".into()))?;
        if *closed {
            return Err(PackError::NotFound("Reader closed".into()));
        }
        // Independent content-only handle: no private server, vault, or reference cycle.
        self.scheduler.start(
            Arc::new(LoopbackHandle {
                server: Mutex::new(None),
                registry: Arc::clone(&self.registry),
            }),
            network,
            packs,
            media,
        )
    }
    fn stop(&self) -> Result<(), PackError> {
        self.scheduler.stop()
    }
    fn kick(&self, id: String) -> Result<(), PackError> {
        self.scheduler.kick(id)
    }
    fn close(&self) -> Result<(), PackError> {
        *self
            .closed
            .lock()
            .map_err(|_| PackError::Corrupt("Sharing lock".into()))? = true;
        self.scheduler.stop()
    }
}
#[uniffi::export]
pub fn server_attach_sharing(
    source: Arc<LoopbackHandle>,
    scheduler: Arc<dyn SharingScheduler>,
) -> Result<(), PackError> {
    super::pack_boundary(|| {
        source
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback lock".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Loopback".into()))?
            .attach_sharing(Arc::new(Dispatcher {
                registry: Arc::clone(&source.registry),
                scheduler,
                closed: Mutex::new(false),
            }))
    })
}
