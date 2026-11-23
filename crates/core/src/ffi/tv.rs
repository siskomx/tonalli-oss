//! TV adapters transfer only a validated immutable catalog, never private app ownership.
use super::{pack_boundary, LoopbackHandle};
use crate::{
    pack::PackError,
    server::{TvCatalog, TvStatus},
};
use std::sync::{Arc, Mutex};
#[derive(uniffi::Record)]
pub struct TvInfo {
    pub address: String,
    pub port: u16,
    pub url: String,
}
#[derive(uniffi::Object)]
pub struct TvHandle {
    server: Mutex<Option<crate::server::TvServer>>,
}
#[uniffi::export]
pub fn server_start_tv(
    catalog: Arc<TvCatalog>,
    bind_address: String,
    port: u16,
) -> Result<Arc<TvHandle>, PackError> {
    pack_boundary(|| {
        let bind = bind_address
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| PackError::Mismatch("Choose an IPv4 local network".into()))?;
        let server = crate::server::TvServer::start(bind.into(), port, catalog)?;
        Ok(Arc::new(TvHandle {
            server: Mutex::new(Some(server)),
        }))
    })
}
#[uniffi::export]
pub fn tv_info(handle: Arc<TvHandle>) -> Result<TvInfo, PackError> {
    pack_boundary(|| {
        let server = handle
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("TV handle lock poisoned".into()))?;
        let server = server
            .as_ref()
            .filter(|s| s.is_running())
            .ok_or_else(|| PackError::NotFound("stopped TV server".into()))?;
        let address = server.address();
        Ok(TvInfo {
            address: address.ip().to_string(),
            port: address.port(),
            url: format!("http://{address}/description.xml"),
        })
    })
}
#[uniffi::export]
pub fn server_stop_tv(handle: Arc<TvHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        let server = handle
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("TV handle lock poisoned".into()))?
            .take();
        if let Some(mut server) = server {
            server.shutdown()?;
        }
        Ok(())
    })
}
#[uniffi::export(foreign)]
pub trait TvScheduler: Send + Sync {
    fn status(&self) -> Result<TvStatus, PackError>;
    fn start(&self, network: String, catalog: Arc<TvCatalog>) -> Result<(), PackError>;
    fn stop(&self) -> Result<(), PackError>;
    /// Release the private view; an independent foreground owner may remain active.
    fn detach_view(&self) -> Result<(), PackError>;
}
struct Dispatcher {
    scheduler: Arc<dyn TvScheduler>,
    closed: Mutex<bool>,
}
impl crate::server::TvControl for Dispatcher {
    fn status(&self) -> Result<TvStatus, PackError> {
        self.scheduler.status()
    }
    fn start(&self, network: String, catalog: Arc<TvCatalog>) -> Result<(), PackError> {
        let closed = self
            .closed
            .lock()
            .map_err(|_| PackError::Corrupt("TV dispatcher lock".into()))?;
        if *closed {
            return Err(PackError::NotFound("Reader closed".into()));
        }
        self.scheduler.start(network, catalog)
    }
    fn stop(&self) -> Result<(), PackError> {
        self.scheduler.stop()
    }
    fn close(&self) -> Result<(), PackError> {
        *self
            .closed
            .lock()
            .map_err(|_| PackError::Corrupt("TV dispatcher lock".into()))? = true;
        self.scheduler.detach_view()
    }
}
#[uniffi::export]
pub fn server_attach_tv(
    source: Arc<LoopbackHandle>,
    scheduler: Arc<dyn TvScheduler>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        source
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback lock".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Loopback".into()))?
            .attach_tv(Arc::new(Dispatcher {
                scheduler,
                closed: Mutex::new(false),
            }))
    })
}
