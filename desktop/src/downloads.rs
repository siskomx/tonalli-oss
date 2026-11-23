//! Native ownership for the shared content job dispatcher.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use tonalli_core::{
    content::{ContentJobs, JobAction, JobError, JobState, JobTicket},
    server::{ContentDispatchError, ContentDispatcher},
};

#[derive(Clone)]
struct ActiveJob {
    jobs: Arc<ContentJobs>,
    ticket: JobTicket,
}

#[derive(Default)]
pub struct DesktopContentDispatcher {
    closing: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    active: Arc<Mutex<Option<ActiveJob>>>,
}

impl DesktopContentDispatcher {
    /// Called before this dispatcher is exposed to HTTP or owns any native task.
    pub fn reconcile_startup(&self, jobs: &ContentJobs) -> Result<Option<JobTicket>, JobError> {
        let worker = self.worker.lock().map_err(|_| JobError::Busy)?;
        if worker.is_some() || self.closing.load(Ordering::Acquire) {
            return Err(JobError::Busy);
        }
        let Some(ticket) = jobs.active_ticket()? else {
            return Ok(None);
        };
        let snapshot = note_job(
            "startup_reconcile",
            &ticket,
            jobs.reconcile_interrupted(&ticket),
        )?;
        // Cancel still needs the existing Core execution/cleanup path, not a fabricated terminal.
        Ok((snapshot.state == JobState::Cancelling).then_some(ticket))
    }

    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        let active = self.active.lock().ok().and_then(|active| active.clone());
        if let Some(active) = active {
            let _ = note_job(
                "shutdown_pause",
                &active.ticket,
                active.jobs.request_ticket(&active.ticket, JobAction::Pause),
            );
        }
        let worker = self.worker.lock().ok().and_then(|mut worker| worker.take());
        // A request/timeout is not native exit. Keep the task owned until it returns.
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

impl ContentDispatcher for DesktopContentDispatcher {
    fn dispatch(
        &self,
        jobs: Arc<ContentJobs>,
        ticket: JobTicket,
    ) -> Result<(), ContentDispatchError> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| ContentDispatchError::Rejected)?;
        if self.closing.load(Ordering::Acquire) {
            eprintln!(
                "content_worker stage=dispatch epoch={} error=closing",
                ticket.epoch
            );
            return Err(ContentDispatchError::Rejected);
        }
        let (handoff, predecessor) = std::sync::mpsc::sync_channel::<Option<JoinHandle<()>>>(1);
        let active = Arc::clone(&self.active);
        let closing = Arc::clone(&self.closing);
        let epoch = ticket.epoch;
        let task = std::thread::Builder::new()
            .name("desktop-content-download".into())
            .spawn(move || {
                // Core can admit the next ticket after terminal publication but before
                // the predecessor thread returns. Fence that tail off the HTTP thread.
                match predecessor.recv() {
                    Ok(Some(previous)) => {
                        let _ = previous.join();
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                if let Ok(mut active) = active.lock() {
                    *active = Some(ActiveJob {
                        jobs: Arc::clone(&jobs),
                        ticket: ticket.clone(),
                    });
                }
                if closing.load(Ordering::Acquire) {
                    let _ = note_job(
                        "closing_pause",
                        &ticket,
                        jobs.request_ticket(&ticket, JobAction::Pause),
                    );
                }
                run(&jobs, &ticket);
                if let Ok(mut active) = active.lock() {
                    *active = None;
                }
            })
            .map_err(|_| {
                eprintln!("content_worker stage=dispatch epoch={epoch} error=spawn");
                ContentDispatchError::Rejected
            })?;
        // Until spawn succeeds the predecessor stays in its original owned slot.
        let previous = worker.replace(task);
        if let Err(error) = handoff.send(previous) {
            if let Some(failed) = worker.take() {
                let _ = failed.join();
            }
            *worker = error.0;
            eprintln!("content_worker stage=dispatch epoch={epoch} error=handoff");
            return Err(ContentDispatchError::Rejected);
        }
        eprintln!("content_worker stage=dispatch epoch={epoch} result=accepted");
        Ok(())
    }
}

#[cfg(windows)]
fn run(jobs: &Arc<ContentJobs>, ticket: &JobTicket) {
    let Ok(mut run) = note_job("claim", ticket, jobs.claim(ticket)) else {
        return;
    };
    let (lease, client) = match crate::network::client() {
        Ok(client) => client,
        Err(reason) => {
            // The ordinary Core client sees the persisted stop before HTTP. This
            // also handles cancel cleanup for an unpublished retained candidate.
            let _ = note_job(
                "no_wifi_pause",
                ticket,
                jobs.request_ticket(ticket, JobAction::Pause),
            );
            let _ = note_job(
                "execute_stopped",
                ticket,
                jobs.execute(&mut run, |_| Ok(())),
            );
            let _ = note_job("finish", ticket, jobs.finish_exited(&mut run));
            if reason == crate::network::NetworkError::WaitingWifi {
                eprintln!("Content download paused: Wi-Fi unavailable.");
            } else {
                eprintln!("Content download paused: network initialization unavailable.");
            }
            return;
        }
    };
    let (stop, stopped) = std::sync::mpsc::sync_channel::<()>(1);
    let monitor_jobs = Arc::clone(jobs);
    let monitor_lease = Arc::clone(&lease);
    let monitor_ticket = ticket.clone();
    let monitor = std::thread::Builder::new()
        .name("desktop-content-network".into())
        .spawn(move || {
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_millis(250)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                let state = monitor_jobs
                    .list()
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|row| row.id == monitor_ticket.id))
                    .map(|row| row.state);
                if matches!(state, Some(JobState::Pausing | JobState::Cancelling)) {
                    monitor_lease.revoke();
                    return;
                }
                if monitor_lease.observe_native().is_err() {
                    let _ = note_job(
                        "network_loss_pause",
                        &monitor_ticket,
                        monitor_jobs.request_ticket(&monitor_ticket, JobAction::Pause),
                    );
                    monitor_lease.revoke();
                    return;
                }
            }
        });
    if monitor.is_err() {
        let _ = note_job(
            "monitor_failure_pause",
            ticket,
            jobs.request_ticket(ticket, JobAction::Pause),
        );
    }
    let _ = note_job(
        "execute",
        ticket,
        jobs.execute_with_client(&mut run, &client, |_| Ok(())),
    );
    // DNS can fail before the periodic monitor observes link loss.
    if lease.observe_native().is_err() {
        let _ = note_job(
            "post_execute_loss_pause",
            ticket,
            jobs.request_ticket(ticket, JobAction::Pause),
        );
    }
    let _ = stop.send(());
    if let Ok(monitor) = monitor {
        let _ = monitor.join();
    }
    lease.revoke();
    drop(client);
    eprintln!(
        "content_worker stage=client_drop epoch={} result=returned",
        ticket.epoch
    );
    // No synthetic terminal: a dropped resolver future still has to receive its
    // WinDNS callback. Only then may Core publish the native job's final state.
    match lease.finish() {
        Ok(()) => {
            eprintln!(
                "content_worker stage=dns_drain epoch={} result=completed",
                ticket.epoch
            );
            let _ = note_job("finish", ticket, jobs.finish_exited(&mut run));
        }
        Err(error) => eprintln!(
            "content_worker stage=dns_drain epoch={} error={error}",
            ticket.epoch
        ),
    }
}

#[cfg(test)]
#[path = "downloads_tests.rs"]
mod tests;

/// iPhone: downloads are Wi-Fi only like Windows. Without a Wi-Fi lease the job is paused for the user
/// to resume; while it runs, losing Wi-Fi requests the same pause, which Core reads between chunks.
#[cfg(target_os = "ios")]
fn run(jobs: &Arc<ContentJobs>, ticket: &JobTicket) {
    let Ok(mut run) = note_job("claim", ticket, jobs.claim(ticket)) else {
        return;
    };
    if !crate::sharing::wifi_available() {
        let _ = note_job(
            "no_wifi_pause",
            ticket,
            jobs.request_ticket(ticket, JobAction::Pause),
        );
        let _ = note_job("execute_stopped", ticket, jobs.execute(&mut run, |_| Ok(())));
        let _ = note_job("finish", ticket, jobs.finish_exited(&mut run));
        return;
    }
    let (stop, stopped) = std::sync::mpsc::sync_channel::<()>(1);
    let monitor_jobs = Arc::clone(jobs);
    let monitor_ticket = ticket.clone();
    let monitor = std::thread::Builder::new()
        .name("ios-content-network".into())
        .spawn(move || {
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_secs(2)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                if !crate::sharing::wifi_available() {
                    let _ = note_job(
                        "network_loss_pause",
                        &monitor_ticket,
                        monitor_jobs.request_ticket(&monitor_ticket, JobAction::Pause),
                    );
                    return;
                }
            }
        });
    if monitor.is_err() {
        let _ = note_job(
            "monitor_failure_pause",
            ticket,
            jobs.request_ticket(ticket, JobAction::Pause),
        );
    }
    let _ = note_job("execute", ticket, jobs.execute(&mut run, |_| Ok(())));
    let _ = stop.send(());
    if let Ok(monitor) = monitor {
        let _ = monitor.join();
    }
    let _ = note_job("finish", ticket, jobs.finish_exited(&mut run));
}

/// Other platforms use Core's ordinary HTTPS client.
#[cfg(not(any(windows, target_os = "ios")))]
fn run(jobs: &Arc<ContentJobs>, ticket: &JobTicket) {
    if let Ok(mut run) = note_job("claim", ticket, jobs.claim(ticket)) {
        let _ = note_job("execute", ticket, jobs.execute(&mut run, |_| Ok(())));
        let _ = note_job("finish", ticket, jobs.finish_exited(&mut run));
    }
}

fn note_job<T>(
    stage: &str,
    ticket: &JobTicket,
    result: Result<T, JobError>,
) -> Result<T, JobError> {
    if let Err(error) = &result {
        use tonalli_core::pack::PackError;
        let code = match error {
            JobError::Busy => "busy",
            JobError::CatalogChanged => "catalog_changed",
            JobError::NotFound => "not_found",
            JobError::StaleAttempt => "stale_attempt",
            JobError::InvalidRequest => "invalid_request",
            JobError::Pack(PackError::Io(_)) => "io",
            JobError::Pack(PackError::Http(_)) => "network",
            JobError::Pack(PackError::Checksum) => "checksum",
            JobError::Pack(PackError::PublicationUncertain(_)) => "publication_uncertain",
            JobError::Pack(_) => "pack",
        };
        eprintln!(
            "content_worker stage={stage} epoch={} error={code}",
            ticket.epoch
        );
    } else {
        eprintln!(
            "content_worker stage={stage} epoch={} result=ok",
            ticket.epoch
        );
    }
    result
}
