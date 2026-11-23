use super::LoopbackHandle;
use crate::{
    country_maps::{CountryMaps, MapDispatcher, MapRun, Ticket},
    pack::PackError,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, uniffi::Record)]
pub struct MapTicket {
    pub id: String,
    pub epoch: u64,
}
#[uniffi::export(foreign)]
pub trait MapScheduler: Send + Sync {
    fn dispatch(&self, ticket: MapTicket) -> bool;
}
struct Dispatcher(Arc<dyn MapScheduler>);
impl MapDispatcher for Dispatcher {
    fn dispatch(&self, _maps: Arc<CountryMaps>, ticket: Ticket) -> Result<(), PackError> {
        if self.0.dispatch(MapTicket {
            id: ticket.id,
            epoch: ticket.epoch,
        }) {
            Ok(())
        } else {
            Err(PackError::Http("Waiting for Wi-Fi".into()))
        }
    }
}
#[derive(uniffi::Object)]
pub struct MapsHandle {
    maps: Arc<CountryMaps>,
}
#[derive(uniffi::Object)]
pub struct MapRunHandle {
    maps: Arc<CountryMaps>,
    run: Mutex<Option<MapRun>>,
}
#[uniffi::export]
pub fn maps_open(root: String) -> Result<Arc<MapsHandle>, PackError> {
    super::pack_boundary(|| {
        Ok(Arc::new(MapsHandle {
            maps: Arc::new(CountryMaps::open(PathBuf::from(root).join("country-maps"))?),
        }))
    })
}
#[uniffi::export]
pub fn maps_import(
    owner: Arc<MapsHandle>,
    source: Arc<super::PackHandle>,
    title: String,
) -> Result<(), PackError> {
    super::pack_boundary(|| owner.maps.import(source.source()?, title).map(|_| ()))
}
#[uniffi::export]
pub fn maps_reconcile(owner: Arc<MapsHandle>) -> Result<(), PackError> {
    super::pack_boundary(|| owner.maps.reconcile())
}
#[uniffi::export]
pub fn server_attach_country_maps(
    server: Arc<LoopbackHandle>,
    owner: Arc<MapsHandle>,
    scheduler: Arc<dyn MapScheduler>,
) -> Result<(), PackError> {
    super::pack_boundary(|| {
        server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback lock".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Loopback".into()))?
            .attach_country_maps(Arc::clone(&owner.maps), Arc::new(Dispatcher(scheduler)))
    })
}
#[uniffi::export]
pub fn maps_claim(
    owner: Arc<MapsHandle>,
    ticket: MapTicket,
) -> Result<Arc<MapRunHandle>, PackError> {
    super::pack_boundary(|| {
        Ok(Arc::new(MapRunHandle {
            maps: Arc::clone(&owner.maps),
            run: Mutex::new(Some(owner.maps.claim(Ticket {
                id: ticket.id,
                epoch: ticket.epoch,
            })?)),
        }))
    })
}
#[uniffi::export]
pub fn maps_pause(owner: Arc<MapsHandle>, ticket: MapTicket) -> Result<(), PackError> {
    super::pack_boundary(|| {
        owner.maps.pause_ticket(&Ticket {
            id: ticket.id,
            epoch: ticket.epoch,
        })
    })
}
#[uniffi::export]
pub fn maps_execute(handle: Arc<MapRunHandle>) -> Result<(), PackError> {
    super::pack_boundary(|| {
        let run = handle
            .run
            .lock()
            .map_err(|_| PackError::Corrupt("Map run lock".into()))?;
        // Called only by the dedicated UIDT process after native Wi-Fi binding.
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| PackError::Http("Map connection unavailable".into()))?;
        handle.maps.execute(
            run.as_ref()
                .ok_or_else(|| PackError::NotFound("Map run".into()))?,
            &client,
        )
    })
}
#[uniffi::export]
pub fn maps_finish(handle: Arc<MapRunHandle>) -> Result<(), PackError> {
    super::pack_boundary(|| {
        let run = handle
            .run
            .lock()
            .map_err(|_| PackError::Corrupt("Map run lock".into()))?
            .take()
            .ok_or_else(|| PackError::NotFound("Map run".into()))?;
        handle.maps.finish(run)
    })
}
