//! Native-only bridge to the existing search preparation owner.
use super::{ContentDispatch, LoopbackHandle, PackHandle};
use crate::{
    catalog::PUBLISHER_PUBLIC_KEY,
    content::ManagedStore,
    index::{
        jobs::{Action, IndexJobError, IndexJobs, IndexSource, Run, Ticket},
        BuildControl,
    },
    pack::PackError,
    server::{ContentDispatchError, PackId, PreparationDispatcher},
    sync::SyncStore,
    zim::Zim,
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PreparationError {
    #[error("Search preparation is busy")]
    Busy,
    #[error("Search preparation attempt is stale")]
    StaleAttempt,
    #[error("Selected book changed")]
    SourceChanged,
    #[error("Search preparation was not found")]
    NotFound,
    #[error("Invalid search preparation request")]
    InvalidRequest,
    #[error("Search preparation storage is unavailable")]
    Storage,
    #[error("Search preparation boundary failed")]
    Boundary,
}
impl From<IndexJobError> for PreparationError {
    fn from(value: IndexJobError) -> Self {
        match value {
            IndexJobError::Busy => Self::Busy,
            IndexJobError::StaleTicket => Self::StaleAttempt,
            IndexJobError::SourceChanged => Self::SourceChanged,
            IndexJobError::NotFound => Self::NotFound,
            IndexJobError::InvalidRequest => Self::InvalidRequest,
            IndexJobError::Pack(_) => Self::Storage,
        }
    }
}
fn boundary<T>(work: impl FnOnce() -> Result<T, PreparationError>) -> Result<T, PreparationError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(PreparationError::Boundary))
}

#[derive(Clone, uniffi::Record)]
pub struct PreparationTicket {
    pub pack: String,
    pub epoch: u64,
}
impl From<Ticket> for PreparationTicket {
    fn from(value: Ticket) -> Self {
        Self {
            pack: value.pack,
            epoch: value.epoch,
        }
    }
}
impl From<PreparationTicket> for Ticket {
    fn from(value: PreparationTicket) -> Self {
        Self {
            pack: value.pack,
            epoch: value.epoch,
        }
    }
}
#[derive(Clone, Copy, uniffi::Enum)]
pub enum PreparationControl {
    Pause,
    Cancel,
}
impl From<PreparationControl> for Action {
    fn from(value: PreparationControl) -> Self {
        match value {
            PreparationControl::Pause => Self::Pause,
            PreparationControl::Cancel => Self::Cancel,
        }
    }
}
#[derive(uniffi::Enum)]
pub enum PreparationSourceFailure {
    MediaRemoved,
    SourceChanged,
    Unavailable,
}

#[uniffi::export(foreign)]
pub trait PreparationScheduler: Send + Sync + 'static {
    fn dispatch(&self, owner: Arc<PreparationHandle>, ticket: PreparationTicket)
        -> ContentDispatch;
    fn control(
        &self,
        owner: Arc<PreparationHandle>,
        ticket: PreparationTicket,
        action: PreparationControl,
    ) -> ContentDispatch;
}

#[derive(uniffi::Object)]
pub struct PreparationHandle {
    root: PathBuf,
    jobs: Arc<IndexJobs>,
}
#[derive(uniffi::Object)]
pub struct PreparationRunHandle {
    root: PathBuf,
    pack: String,
    jobs: Arc<IndexJobs>,
    run: Mutex<Option<Run>>,
}
struct Dispatcher {
    owner: Arc<PreparationHandle>,
    scheduler: Arc<dyn PreparationScheduler>,
}
fn accepted(value: ContentDispatch) -> Result<(), ContentDispatchError> {
    match value {
        ContentDispatch::Accepted => Ok(()),
        ContentDispatch::Rejected => Err(ContentDispatchError::Rejected),
        ContentDispatch::Uncertain => Err(ContentDispatchError::Uncertain),
    }
}
impl PreparationDispatcher for Dispatcher {
    fn dispatch(&self, _jobs: Arc<IndexJobs>, ticket: Ticket) -> Result<(), ContentDispatchError> {
        accepted(
            self.scheduler
                .dispatch(Arc::clone(&self.owner), ticket.into()),
        )
    }
    fn control(
        &self,
        _jobs: Arc<IndexJobs>,
        ticket: Ticket,
        action: Action,
    ) -> Result<(), ContentDispatchError> {
        let action = match action {
            Action::Pause => PreparationControl::Pause,
            Action::Cancel => PreparationControl::Cancel,
            Action::Resume => return Err(ContentDispatchError::Rejected),
        };
        accepted(
            self.scheduler
                .control(Arc::clone(&self.owner), ticket.into(), action),
        )
    }
}

#[uniffi::export]
pub fn preparation_open(data_dir: String) -> Result<Arc<PreparationHandle>, PreparationError> {
    boundary(|| {
        let root = PathBuf::from(data_dir);
        let jobs = Arc::new(IndexJobs::open(&root)?);
        let root = root.canonicalize().map_err(|_| PreparationError::Storage)?;
        Ok(Arc::new(PreparationHandle { root, jobs }))
    })
}
#[uniffi::export]
pub fn server_attach_preparation(
    server: Arc<LoopbackHandle>,
    owner: Arc<PreparationHandle>,
    scheduler: Arc<dyn PreparationScheduler>,
) -> Result<(), PreparationError> {
    boundary(|| {
        let live = server
            .server
            .lock()
            .map_err(|_| PreparationError::Boundary)?;
        live.as_ref()
            .ok_or(PreparationError::NotFound)?
            .attach_preparation(
                Arc::clone(&owner.jobs),
                Arc::new(Dispatcher { owner, scheduler }),
            )
            .map_err(|_| PreparationError::Storage)
    })
}
#[uniffi::export]
pub fn preparation_active_ticket(
    owner: Arc<PreparationHandle>,
) -> Result<Option<PreparationTicket>, PreparationError> {
    boundary(|| Ok(owner.jobs.active_ticket()?.map(Into::into)))
}
/// Native caller first confirms that no matching pending/running service remains.
#[uniffi::export]
pub fn preparation_reconcile(
    owner: Arc<PreparationHandle>,
    ticket: PreparationTicket,
) -> Result<(), PreparationError> {
    boundary(|| {
        owner.jobs.reconcile_interrupted(&ticket.into())?;
        Ok(())
    })
}
#[uniffi::export]
pub fn preparation_request_stop(
    owner: Arc<PreparationHandle>,
    ticket: PreparationTicket,
    action: PreparationControl,
) -> Result<(), PreparationError> {
    boundary(|| {
        owner.jobs.request(&ticket.into(), action.into())?;
        Ok(())
    })
}
#[uniffi::export]
pub fn preparation_reserve(
    owner: Arc<PreparationHandle>,
    ticket: PreparationTicket,
) -> Result<Arc<PreparationRunHandle>, PreparationError> {
    boundary(|| {
        let pack = ticket.pack.clone();
        let run = owner.jobs.reserve(&ticket.into())?;
        Ok(Arc::new(PreparationRunHandle {
            root: owner.root.clone(),
            pack,
            jobs: Arc::clone(&owner.jobs),
            run: Mutex::new(Some(run)),
        }))
    })
}
fn with_run<T>(
    handle: &PreparationRunHandle,
    work: impl FnOnce(&mut Run) -> Result<T, IndexJobError>,
) -> Result<T, PreparationError> {
    let mut slot = handle.run.lock().map_err(|_| PreparationError::Boundary)?;
    work(slot.as_mut().ok_or(PreparationError::StaleAttempt)?).map_err(Into::into)
}
#[uniffi::export]
pub fn preparation_run_control(
    handle: Arc<PreparationRunHandle>,
) -> Result<bool, PreparationError> {
    boundary(|| with_run(&handle, |run| handle.jobs.execute_unbound_control(run)))
}
fn bind(
    handle: &PreparationRunHandle,
    run: &mut Run,
    source: Result<IndexSource, IndexJobError>,
) -> Result<(), IndexJobError> {
    match source {
        Ok(source) => handle.jobs.bind_source(run, &source),
        Err(error) => {
            handle.jobs.fail_unbound(run, error.clone())?;
            Err(error)
        }
    }
}
#[uniffi::export]
pub fn preparation_bind_pack(
    handle: Arc<PreparationRunHandle>,
    pack: Arc<PackHandle>,
) -> Result<(), PreparationError> {
    boundary(|| {
        with_run(&handle, |run| {
            let source = (|| IndexSource::registered(PackId::new(&handle.pack)?, pack.zim()?))();
            bind(&handle, run, source)
        })
    })
}
/// Reopen only actual app-managed content; reader sources require native SAF authority.
#[uniffi::export]
pub fn preparation_bind_stored(
    handle: Arc<PreparationRunHandle>,
) -> Result<bool, PreparationError> {
    boundary(|| {
        with_run(&handle, |run| {
            if handle.pack.starts_with("reader-") {
                return Ok(false);
            }
            let source = (|| {
                if handle.pack.starts_with("copy-") {
                    let store = SyncStore::open(&handle.root)?;
                    let manifest = store
                        .list()?
                        .into_iter()
                        .find(|row| row.local_id() == handle.pack)
                        .ok_or(IndexJobError::NotFound)?;
                    if manifest.kind != crate::catalog::CatalogKind::Zim {
                        return Err(IndexJobError::InvalidRequest);
                    }
                    IndexSource::registered(
                        PackId::new(&handle.pack)?,
                        Arc::new(Zim::open(store.open_source(&manifest)?)?),
                    )
                } else {
                    IndexSource::managed(
                        &ManagedStore::open(&handle.root, PUBLISHER_PUBLIC_KEY)?,
                        &handle.pack,
                    )
                }
            })();
            bind(&handle, run, source)?;
            Ok(true)
        })
    })
}
#[uniffi::export]
pub fn preparation_source_failed(
    handle: Arc<PreparationRunHandle>,
    reason: PreparationSourceFailure,
) -> Result<(), PreparationError> {
    boundary(|| {
        with_run(&handle, |run| {
            let error = match reason {
                PreparationSourceFailure::MediaRemoved => PackError::MediaRemoved.into(),
                PreparationSourceFailure::SourceChanged => IndexJobError::SourceChanged,
                PreparationSourceFailure::Unavailable => {
                    PackError::NotFound("Selected book".into()).into()
                }
            };
            handle.jobs.fail_unbound(run, error)
        })
    })
}
#[uniffi::export]
pub fn preparation_execute(handle: Arc<PreparationRunHandle>) -> Result<(), PreparationError> {
    boundary(|| {
        with_run(&handle, |run| {
            handle.jobs.execute(run, 1, |_| Ok(BuildControl::Continue))
        })
    })
}
/// Close native provider handles first. A true result requires a real stop-only continuation.
#[uniffi::export]
pub fn preparation_finish(handle: Arc<PreparationRunHandle>) -> Result<bool, PreparationError> {
    boundary(|| {
        let mut slot = handle.run.lock().map_err(|_| PreparationError::Boundary)?;
        let result = handle
            .jobs
            .finish_exited(slot.as_mut().ok_or(PreparationError::StaleAttempt)?)?;
        slot.take();
        Ok(result.dispatch)
    })
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
