//! Serves explicitly named installed packs on one private LAN address without a window.
//! The release executable has no console, so progress and errors go to the private status file.
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tonalli_core::server::{SharedMedia, SharingControl};
use tonalli_desktop::headless::{choose_address, status_value, stop_path, write_status, Report};
use tonalli_desktop::options::{Headless, Options};
use tonalli_desktop::sharing::eligible_addresses;

use crate::session::Session;

const POLL: Duration = Duration::from_millis(500);
const REFRESH: Duration = Duration::from_secs(2);

pub fn run(options: Options) -> Result<(), String> {
    let headless = options
        .headless
        .as_ref()
        .ok_or("Headless options are missing")?;
    let _ = std::fs::remove_file(stop_path(&headless.status_file));
    let outcome = start(&options, headless);
    let message = outcome.as_ref().err().map(String::as_str);
    publish(headless, Snapshot::stopped(message));
    outcome
}

fn start(options: &Options, headless: &Headless) -> Result<(), String> {
    let (session, _) = Session::start(options)
        .map_err(|error| format!("Library could not start: {error}"))?;
    let served = serve(&session, headless);
    let closed = session.shutdown().map_err(|error| format!("Shutdown failed: {error}"));
    served.and(closed)
}

fn serve(session: &Session, headless: &Headless) -> Result<(), String> {
    let choices = eligible_addresses().map_err(|error| error.to_string())?;
    let address = choose_address(&choices, headless.address)?;
    let sharing = session.sharing();
    sharing
        .start(
            address.to_string(),
            headless.share_packs.clone(),
            Arc::new(SharedMedia::default()),
        )
        .map_err(|error| format!("Sharing could not start: {error}"))?;
    let stop = stop_path(&headless.status_file);
    let mut last = Instant::now() - REFRESH;
    let result = loop {
        if stop.is_file() {
            break Ok(());
        }
        if last.elapsed() >= REFRESH {
            last = Instant::now();
            match sharing.status() {
                Ok(status) if status.running => publish(
                    headless,
                    Snapshot {
                        running: true,
                        url: status.url,
                        code: status.pairing_code,
                        devices: status.devices.len(),
                        error: None,
                    },
                ),
                Ok(_) => break Err("Sharing stopped; the LAN address may have changed".into()),
                Err(error) => break Err(format!("Sharing status unavailable: {error}")),
            }
        }
        std::thread::sleep(POLL);
    };
    let closed = sharing.close().map_err(|error| format!("Sharing did not stop: {error}"));
    result.and(closed)
}

struct Snapshot {
    running: bool,
    url: Option<String>,
    code: Option<String>,
    devices: usize,
    error: Option<String>,
}

impl Snapshot {
    fn stopped(error: Option<&str>) -> Self {
        Self {
            running: false,
            url: None,
            code: None,
            devices: 0,
            error: error.map(str::to_owned),
        }
    }
}

fn publish(headless: &Headless, snapshot: Snapshot) {
    let updated = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let value = status_value(&Report {
        pid: std::process::id(),
        running: snapshot.running,
        url: snapshot.url.as_deref(),
        pairing_code: snapshot.code.as_deref(),
        devices: snapshot.devices,
        error: snapshot.error.as_deref(),
        updated,
    });
    let _ = write_status(&headless.status_file, &value);
}
