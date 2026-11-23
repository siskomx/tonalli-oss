use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tonalli_core::{
    country_maps::{CountryMaps, MapDispatcher, Ticket},
    pack::PackError,
};

#[derive(Default)]
pub struct DesktopMapDispatcher {
    closing: Arc<AtomicBool>,
    task: Mutex<Option<std::thread::JoinHandle<()>>>,
    active: Arc<Mutex<Option<(Arc<CountryMaps>, Ticket)>>>,
}
impl DesktopMapDispatcher {
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        if let Some((maps, ticket)) = self.active.lock().ok().and_then(|a| a.clone()) {
            let _ = maps.pause_ticket(&ticket);
        }
        if let Some(task) = self.task.lock().ok().and_then(|mut t| t.take()) {
            let _ = task.join();
        }
    }
}
impl MapDispatcher for DesktopMapDispatcher {
    fn dispatch(&self, maps: Arc<CountryMaps>, ticket: Ticket) -> Result<(), PackError> {
        let mut task = self
            .task
            .lock()
            .map_err(|_| PackError::Corrupt("Map dispatcher".into()))?;
        if self.closing.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("App is closing".into()));
        }
        let active = Arc::clone(&self.active);
        let closing = Arc::clone(&self.closing);
        if let Some(previous) = task.take() {
            let _ = previous.join();
        }
        *active
            .lock()
            .map_err(|_| PackError::Corrupt("Map dispatcher".into()))? =
            Some((Arc::clone(&maps), ticket.clone()));
        *task = Some(
            std::thread::Builder::new()
                .name("desktop-country-map".into())
                .spawn(move || {
                    if closing.load(Ordering::Acquire) {
                        let _ = maps.pause_ticket(&ticket);
                    } else {
                        run(&maps, &ticket);
                    }
                    if let Ok(mut active) = active.lock() {
                        *active = None;
                    }
                })?,
        );
        Ok(())
    }
}
#[cfg(windows)]
fn run(maps: &Arc<CountryMaps>, ticket: &Ticket) {
    let Ok(run) = maps.claim(ticket.clone()) else {
        return;
    };
    let Ok((lease, client)) = crate::network::client() else {
        let _ = maps.pause_ticket(ticket);
        let _ = maps.finish(run);
        return;
    };
    let (stop, stopped) = std::sync::mpsc::sync_channel::<()>(1);
    let monitor_maps = Arc::clone(maps);
    let monitor_lease = Arc::clone(&lease);
    let id = ticket.id.clone();
    let monitor_ticket = ticket.clone();
    let monitor = std::thread::spawn(move || {
        while matches!(
            stopped.recv_timeout(std::time::Duration::from_millis(250)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let stop = monitor_maps
                .list()
                .ok()
                .and_then(|rows| rows.into_iter().find(|row| row.id == id))
                .is_some_and(|row| matches!(row.state.as_str(), "pausing" | "cancelling"));
            if stop || monitor_lease.observe_native().is_err() {
                if !stop {
                    let _ = monitor_maps.pause_ticket(&monitor_ticket);
                }
                monitor_lease.revoke();
                return;
            }
        }
    });
    let _ = maps.execute(&run, &client);
    if lease.observe_native().is_err() {
        let _ = maps.pause_ticket(ticket);
    }
    let _ = stop.send(());
    let _ = monitor.join();
    lease.revoke();
    drop(client);
    if lease.finish().is_ok() {
        let _ = maps.finish(run);
    }
}
/// iPhone: Wi-Fi only like Windows. No Wi-Fi pauses the job; losing it while running requests the same
/// pause, which the extractor reads from the job state as it goes.
#[cfg(not(windows))]
fn run(maps: &Arc<CountryMaps>, ticket: &Ticket) {
    let Ok(run) = maps.claim(ticket.clone()) else {
        let _ = maps.rejected(ticket);
        return;
    };
    if !crate::sharing::wifi_available() {
        let _ = maps.pause_ticket(ticket);
        let _ = maps.finish(run);
        return;
    }
    let (stop, stopped) = std::sync::mpsc::sync_channel::<()>(1);
    let monitor_maps = Arc::clone(maps);
    let monitor_ticket = ticket.clone();
    let monitor = std::thread::spawn(move || {
        while matches!(
            stopped.recv_timeout(std::time::Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            if !crate::sharing::wifi_available() {
                let _ = monitor_maps.pause_ticket(&monitor_ticket);
                return;
            }
        }
    });
    let _ = maps.execute_with_default_client(&run);
    let _ = stop.send(());
    let _ = monitor.join();
    let _ = maps.finish(run);
}
