//! Native content scheduling only. Web callers never provide roots, URLs or transport policy.
#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
use super::LoopbackHandle;
use crate::{
    catalog::{Catalog, PUBLISHER_PUBLIC_KEY},
    content::{ContentJobs, JobAction, JobError, JobRun, JobState, JobTicket, ManagedStore},
    download::DownloadAction,
    server::{ContentDispatchError, ContentDispatcher},
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ContentError {
    #[error("Content worker is busy")]
    Busy,
    #[error("Stale content attempt")]
    StaleAttempt,
    #[error("Content catalog changed")]
    CatalogChanged,
    #[error("Content was not found")]
    NotFound,
    #[error("Invalid content request")]
    InvalidRequest,
    #[error("Content storage is unavailable")]
    Storage,
    #[error("Content boundary failed")]
    Boundary,
}
impl From<JobError> for ContentError {
    fn from(value: JobError) -> Self {
        match value {
            JobError::Busy => Self::Busy,
            JobError::StaleAttempt => Self::StaleAttempt,
            JobError::CatalogChanged => Self::CatalogChanged,
            JobError::NotFound => Self::NotFound,
            JobError::InvalidRequest => Self::InvalidRequest,
            JobError::Pack(_) => Self::Storage,
        }
    }
}
fn boundary<T>(work: impl FnOnce() -> Result<T, ContentError>) -> Result<T, ContentError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(ContentError::Boundary))
}

#[derive(Clone, uniffi::Record)]
pub struct ContentTicket {
    pub id: String,
    pub epoch: u64,
}
impl From<JobTicket> for ContentTicket {
    fn from(value: JobTicket) -> Self {
        Self {
            id: value.id,
            epoch: value.epoch,
        }
    }
}
impl From<ContentTicket> for JobTicket {
    fn from(value: ContentTicket) -> Self {
        Self {
            id: value.id,
            epoch: value.epoch,
        }
    }
}
#[derive(Clone, Copy, uniffi::Enum)]
pub enum ContentControl {
    Pause,
    Cancel,
}
#[derive(uniffi::Enum)]
pub enum ContentDispatch {
    Accepted,
    Rejected,
    Uncertain,
}
fn accepted(result: ContentDispatch) -> Result<(), ContentDispatchError> {
    match result {
        ContentDispatch::Accepted => Ok(()),
        ContentDispatch::Rejected => Err(ContentDispatchError::Rejected),
        ContentDispatch::Uncertain => Err(ContentDispatchError::Uncertain),
    }
}

#[uniffi::export(foreign)]
pub trait ContentScheduler: Send + Sync + 'static {
    fn dispatch(&self, owner: Arc<ContentHandle>, ticket: ContentTicket) -> ContentDispatch;
    fn control(
        &self,
        owner: Arc<ContentHandle>,
        ticket: ContentTicket,
        action: ContentControl,
    ) -> ContentDispatch;
}

#[derive(uniffi::Object)]
pub struct ContentHandle {
    store: Arc<ManagedStore>,
    catalog: Arc<Catalog>,
    jobs: Arc<ContentJobs>,
}
#[derive(uniffi::Object)]
pub struct ContentRunHandle {
    jobs: Arc<ContentJobs>,
    run: Mutex<Option<JobRun>>,
}
struct Dispatcher {
    owner: Arc<ContentHandle>,
    scheduler: Arc<dyn ContentScheduler>,
}
impl Dispatcher {
    fn owner(&self, jobs: Arc<ContentJobs>) -> Arc<ContentHandle> {
        Arc::new(ContentHandle {
            store: Arc::clone(&self.owner.store),
            catalog: Arc::clone(&self.owner.catalog),
            jobs,
        })
    }
}
impl ContentDispatcher for Dispatcher {
    fn dispatch(
        &self,
        jobs: Arc<ContentJobs>,
        ticket: JobTicket,
    ) -> Result<(), ContentDispatchError> {
        // A paused-job Cancel is a new Core dispatch, but needs no Wi-Fi OS admission.
        let action = jobs
            .list()
            .map_err(|_| ContentDispatchError::Uncertain)?
            .into_iter()
            .find(|job| job.id == ticket.id)
            .and_then(|job| match job.state {
                JobState::Pausing => Some(ContentControl::Pause),
                JobState::Cancelling => Some(ContentControl::Cancel),
                _ => None,
            });
        if let Some(action) = action {
            accepted(
                self.scheduler
                    .control(self.owner(jobs), ticket.into(), action),
            )
        } else {
            accepted(self.scheduler.dispatch(self.owner(jobs), ticket.into()))
        }
    }
    fn control(
        &self,
        jobs: Arc<ContentJobs>,
        ticket: JobTicket,
        action: JobAction,
    ) -> Result<(), ContentDispatchError> {
        let action = match action {
            JobAction::Pause => ContentControl::Pause,
            JobAction::Cancel => ContentControl::Cancel,
            _ => return Err(ContentDispatchError::Rejected),
        };
        accepted(
            self.scheduler
                .control(self.owner(jobs), ticket.into(), action),
        )
    }
}

#[uniffi::export]
pub fn content_open(data_dir: String) -> Result<Arc<ContentHandle>, ContentError> {
    boundary(|| {
        let store = Arc::new(
            ManagedStore::open(Path::new(&data_dir), PUBLISHER_PUBLIC_KEY)
                .map_err(|_| ContentError::Storage)?,
        );
        let bundled = Catalog::bundled().map_err(|_| ContentError::Storage)?;
        let path = store.root().join("packs/managed/content.catalog");
        let catalog = if path.try_exists().map_err(|_| ContentError::Storage)? {
            let saved = Catalog::load_accepted(&path, PUBLISHER_PUBLIC_KEY)
                .map_err(|_| ContentError::Storage)?;
            if saved.revision() >= bundled.revision() {
                saved
            } else {
                bundled
            }
        } else {
            bundled
        };
        let catalog = Arc::new(catalog);
        let jobs = Arc::new(ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog))?);
        Ok(Arc::new(ContentHandle {
            store,
            catalog,
            jobs,
        }))
    })
}

#[uniffi::export]
pub fn server_attach_content_jobs(
    server: Arc<LoopbackHandle>,
    owner: Arc<ContentHandle>,
    scheduler: Arc<dyn ContentScheduler>,
) -> Result<(), ContentError> {
    boundary(|| {
        let live = server.server.lock().map_err(|_| ContentError::Boundary)?;
        let live = live.as_ref().ok_or(ContentError::NotFound)?;
        live.attach_content_jobs(
            Arc::clone(&owner.store),
            Arc::clone(&owner.catalog),
            Arc::new(Dispatcher { owner, scheduler }),
        )
        .map_err(|_| ContentError::Storage)
    })
}

#[uniffi::export]
pub fn content_claim(
    owner: Arc<ContentHandle>,
    ticket: ContentTicket,
) -> Result<Arc<ContentRunHandle>, ContentError> {
    boundary(|| {
        let run = owner.jobs.claim(&ticket.into())?;
        Ok(Arc::new(ContentRunHandle {
            jobs: Arc::clone(&owner.jobs),
            run: Mutex::new(Some(run)),
        }))
    })
}

#[uniffi::export]
pub fn content_active_ticket(
    owner: Arc<ContentHandle>,
) -> Result<Option<ContentTicket>, ContentError> {
    boundary(|| Ok(owner.jobs.active_ticket()?.map(Into::into)))
}

/// Native caller first confirms no matching pending/running OS dispatch; Core also tests its file lease.
#[uniffi::export]
pub fn content_reconcile(
    owner: Arc<ContentHandle>,
    ticket: ContentTicket,
) -> Result<(), ContentError> {
    boundary(|| {
        owner.jobs.reconcile_interrupted(&ticket.into())?;
        Ok(())
    })
}

#[uniffi::export]
pub fn content_request_stop(
    owner: Arc<ContentHandle>,
    ticket: ContentTicket,
    action: ContentControl,
) -> Result<(), ContentError> {
    boundary(|| {
        let action = match action {
            ContentControl::Pause => JobAction::Pause,
            ContentControl::Cancel => JobAction::Cancel,
        };
        owner.jobs.request_ticket(&ticket.into(), action)?;
        Ok(())
    })
}

#[uniffi::export]
pub fn content_run_needs_network(handle: Arc<ContentRunHandle>) -> Result<bool, ContentError> {
    boundary(|| {
        let run = handle.run.lock().map_err(|_| ContentError::Boundary)?;
        Ok(handle
            .jobs
            .requested(run.as_ref().ok_or(ContentError::StaleAttempt)?)?
            == DownloadAction::Continue)
    })
}

fn execute(handle: Arc<ContentRunHandle>, stop_only: bool) -> Result<(), ContentError> {
    boundary(|| {
        let mut run = handle.run.lock().map_err(|_| ContentError::Boundary)?;
        let run = run.as_mut().ok_or(ContentError::StaleAttempt)?;
        if stop_only && handle.jobs.requested(run)? == DownloadAction::Continue {
            return Err(ContentError::InvalidRequest);
        }
        handle.jobs.execute(run, |_| Ok(()))?;
        Ok(())
    })
}
#[uniffi::export]
pub fn content_execute(handle: Arc<ContentRunHandle>) -> Result<(), ContentError> {
    execute(handle, false)
}
#[uniffi::export]
pub fn content_execute_stop(handle: Arc<ContentRunHandle>) -> Result<(), ContentError> {
    execute(handle, true)
}
#[uniffi::export]
pub fn content_finish(handle: Arc<ContentRunHandle>) -> Result<(), ContentError> {
    boundary(|| {
        let mut run = handle.run.lock().map_err(|_| ContentError::Boundary)?;
        handle
            .jobs
            .finish_exited(run.as_mut().ok_or(ContentError::StaleAttempt)?)?;
        run.take();
        Ok(())
    })
}
