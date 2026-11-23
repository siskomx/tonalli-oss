//! Native thread lifetime for the existing controlled search builder.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use tonalli_core::{
    index::{
        jobs::{Action, IndexJobError, IndexJobs, Ticket},
        BuildControl,
    },
    server::{ContentDispatchError, PackId, PackRegistry, PreparationDispatcher},
};

pub struct DesktopPreparation {
    registry: Arc<PackRegistry>,
    closing: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    active: Arc<Mutex<Option<(Arc<IndexJobs>, Ticket)>>>,
}

impl DesktopPreparation {
    pub fn new(registry: Arc<PackRegistry>) -> Self {
        Self {
            registry,
            closing: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
            active: Arc::new(Mutex::new(None)),
        }
    }

    pub fn reconcile_startup(&self, jobs: &IndexJobs) -> Result<(), IndexJobError> {
        let worker = self.worker.lock().map_err(|_| IndexJobError::Busy)?;
        if worker.is_some() || self.closing.load(Ordering::Acquire) {
            return Err(IndexJobError::Busy);
        }
        if let Some(ticket) = jobs.active_ticket()? {
            jobs.reconcile_interrupted(&ticket)?;
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        if let Some((jobs, ticket)) = self.active.lock().ok().and_then(|slot| slot.clone()) {
            let _ = jobs.request(&ticket, Action::Pause);
        }
        if let Some(worker) = self.worker.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = worker.join();
        }
    }
}

impl PreparationDispatcher for DesktopPreparation {
    fn dispatch(&self, jobs: Arc<IndexJobs>, ticket: Ticket) -> Result<(), ContentDispatchError> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| ContentDispatchError::Rejected)?;
        if self.closing.load(Ordering::Acquire) {
            return Err(ContentDispatchError::Rejected);
        }
        let (handoff, predecessor) = std::sync::mpsc::sync_channel::<Option<JoinHandle<()>>>(1);
        let active = Arc::clone(&self.active);
        let closing = Arc::clone(&self.closing);
        let registry = Arc::clone(&self.registry);
        let task = std::thread::Builder::new()
            .name("desktop-search-preparation".into())
            .spawn(move || {
                match predecessor.recv() {
                    Ok(Some(previous)) => {
                        let _ = previous.join();
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                if let Ok(mut slot) = active.lock() {
                    *slot = Some((Arc::clone(&jobs), ticket.clone()));
                }
                if closing.load(Ordering::Acquire) {
                    let _ = jobs.request(&ticket, Action::Pause);
                }
                if let Err(error) = run(&registry, &jobs, &ticket) {
                    // Do not publish paths or provider error text to logs.
                    let code = match error {
                        IndexJobError::Busy => "busy",
                        IndexJobError::StaleTicket => "stale",
                        _ => "preparation_failed",
                    };
                    eprintln!("search_worker epoch={} error={code}", ticket.epoch);
                }
                if let Ok(mut slot) = active.lock() {
                    *slot = None;
                }
            })
            .map_err(|_| ContentDispatchError::Rejected)?;
        let previous = worker.replace(task);
        if let Err(error) = handoff.send(previous) {
            if let Some(failed) = worker.take() {
                let _ = failed.join();
            }
            *worker = error.0;
            return Err(ContentDispatchError::Rejected);
        }
        Ok(())
    }

    fn control(
        &self,
        jobs: Arc<IndexJobs>,
        ticket: Ticket,
        _action: Action,
    ) -> Result<(), ContentDispatchError> {
        // HTTP has already persisted the intent. The accepted task polls that exact epoch.
        let slot = self
            .worker
            .lock()
            .map_err(|_| ContentDispatchError::Uncertain)?;
        if self.closing.load(Ordering::Acquire) {
            return Err(ContentDispatchError::Uncertain);
        }
        if slot.as_ref().is_some_and(|worker| !worker.is_finished()) {
            return Ok(());
        }
        drop(slot);
        self.dispatch(jobs, ticket)
    }
}

fn run(registry: &PackRegistry, jobs: &IndexJobs, ticket: &Ticket) -> Result<(), IndexJobError> {
    let mut run = jobs.reserve(ticket)?;
    let result = (|| {
        if jobs.execute_unbound_control(&mut run)? {
            return Ok(());
        }
        let id = PackId::new(&ticket.pack)?;
        let source = match registry.preparation_source(&id) {
            Ok(source) => source,
            Err(error) => {
                jobs.fail_unbound(&mut run, error.clone())?;
                return Err(error);
            }
        };
        jobs.bind_source(&mut run, &source)?;
        jobs.execute(&mut run, 1, |_| Ok(BuildControl::Continue))?;
        if let Some(index) = jobs.safe_ready(&source)? {
            registry.attach_index(id, Arc::new(index))?;
        }
        Ok(())
    })();
    let finished = jobs.finish_exited(&mut run);
    result?;
    let snapshot = finished?;
    if snapshot.dispatch {
        // A Cancel arriving during a completed Pause still needs real source-free cleanup.
        let mut stop = jobs.claim_stop(ticket)?;
        let result = jobs.execute_stop(&mut stop);
        let finished = jobs.finish_exited(&mut stop);
        result?;
        finished?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
