//! Native country extraction. Transport is supplied by the platform's Wi-Fi owner.
mod cleanup;
mod extract;
mod imports;
mod ranges;
mod store;
pub use imports::ImportedMap;

use crate::{
    download::{ensure_regular_or_absent, write_atomic},
    pack::{verify_sha256, PackError, PreadSource},
    tiles::TileSource,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    sync::Arc,
};

pub struct CountryMaps {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub sha256: String,
    pub bytes: u64,
    pub zoom: u8,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub epoch: u64,
    pub state: String,
    pub zoom: u8,
    pub estimate: bool,
    pub completed_bytes: u64,
    pub download_bytes: u64,
    pub output_bytes: u64,
    pub installed: Option<Installed>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<ranges::Snapshot>,
    pending: Option<String>,
    #[serde(default)]
    cache_epoch: u64,
}

pub struct MapRun {
    lease: File,
    ticket: Ticket,
}

pub trait MapDispatcher: Send + Sync {
    fn dispatch(&self, maps: Arc<CountryMaps>, ticket: Ticket) -> Result<(), PackError>;
}

fn country(id: &str) -> bool {
    matches!(id, "mx" | "us" | "ca")
}
fn active(state: &str) -> bool {
    matches!(
        state,
        "queued"
            | "checking"
            | "downloading"
            | "verifying"
            | "pausing"
            | "cancelling"
            | "finishing"
    )
}
fn busy() -> PackError {
    PackError::Mismatch("Another map operation is running".into())
}

impl CountryMaps {
    pub fn open(root: PathBuf) -> Result<Self, PackError> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn list(&self) -> Result<Vec<Row>, PackError> {
        self.transaction(|ledger| Ok(ledger.rows.clone()))
    }

    pub fn prepare(&self, id: &str, zoom: u8, estimate: bool) -> Result<Ticket, PackError> {
        if !country(id) || !matches!(zoom, 8 | 12 | 15) {
            return Err(PackError::Mismatch("Unsupported country or detail".into()));
        }
        let worker = store::lock(&self.root.join("worker.lock"))?;
        worker.try_lock_exclusive().map_err(|_| busy())?;
        self.transaction(|ledger| {
            if ledger.rows.iter().any(|row| active(&row.state)) {
                return Err(busy());
            }
            let existing = ledger.rows.iter().position(|row| row.id == id);
            let old = existing.map(|i| ledger.rows[i].clone());
            if !estimate
                && !old.as_ref().is_some_and(|row| {
                    row.zoom == zoom
                        && row.source.is_some()
                        && matches!(row.state.as_str(), "ready" | "paused" | "failed")
                })
            {
                return Err(PackError::Mismatch("Check download size first".into()));
            }
            let epoch = old
                .as_ref()
                .map_or(Some(1), |row| row.epoch.checked_add(1))
                .ok_or_else(|| PackError::Corrupt("Map epoch".into()))?;
            let row = Row {
                id: id.into(),
                epoch,
                state: "queued".into(),
                zoom,
                estimate,
                completed_bytes: if estimate {
                    0
                } else {
                    old.as_ref().map_or(0, |r| r.completed_bytes)
                },
                download_bytes: old.as_ref().map_or(0, |r| r.download_bytes),
                output_bytes: old.as_ref().map_or(0, |r| r.output_bytes),
                cache_epoch: if estimate {
                    epoch
                } else {
                    old.as_ref().map_or(epoch, |row| row.cache_epoch)
                },
                installed: old.as_ref().and_then(|row| row.installed.clone()),
                source: if estimate {
                    None
                } else {
                    old.and_then(|row| row.source)
                },
                error: None,
                pending: None,
            };
            if let Some(i) = existing {
                ledger.rows[i] = row;
            } else {
                ledger.rows.push(row);
            }
            Ok(Ticket {
                id: id.into(),
                epoch,
            })
        })
    }

    pub fn rejected(&self, ticket: &Ticket) -> Result<(), PackError> {
        self.update(ticket, |row| {
            if row.state == "queued" {
                row.state = "paused".into();
                row.error = Some("waiting_wifi".into());
            }
            Ok(())
        })
    }

    pub fn control(&self, id: &str, cancel: bool) -> Result<(), PackError> {
        self.control_attempt(id, cancel, None)
    }

    fn control_attempt(&self, id: &str, cancel: bool, epoch: Option<u64>) -> Result<(), PackError> {
        if !country(id) {
            return Err(PackError::NotFound("Country map".into()));
        }
        let worker = store::lock(&self.root.join("worker.lock"))?;
        let idle = worker.try_lock_exclusive().is_ok();
        self.transaction(|ledger| {
            let row = ledger
                .rows
                .iter_mut()
                .find(|row| row.id == id)
                .ok_or_else(|| PackError::NotFound("Country map".into()))?;
            if epoch.is_some_and(|epoch| row.epoch != epoch) {
                return Err(PackError::Mismatch("Stale map stop request".into()));
            }
            if matches!(row.state.as_str(), "completed" | "cancelled") {
                return Ok(());
            }
            if idle {
                row.state = if cancel { "cancelled" } else { "paused" }.into();
                row.pending = None;
                // Cancellation invalidates admission before cache cleanup; original installed map survives.
                if cancel {
                    row.source = None;
                    row.completed_bytes = 0;
                }
            } else if active(&row.state) {
                row.state = if cancel { "cancelling" } else { "pausing" }.into();
            }
            Ok(())
        })
    }

    pub fn pause_ticket(&self, ticket: &Ticket) -> Result<(), PackError> {
        self.control_attempt(&ticket.id, false, Some(ticket.epoch))
    }

    pub fn reconcile(&self) -> Result<(), PackError> {
        let worker = store::lock(&self.root.join("worker.lock"))?;
        worker.try_lock_exclusive().map_err(|_| busy())?;
        self.transaction(|ledger| {
            for row in &mut ledger.rows {
                if active(&row.state) {
                    row.state = if row.state == "cancelling" {
                        "cancelled"
                    } else {
                        "paused"
                    }
                    .into();
                    row.pending = None;
                }
            }
            Ok(())
        })
    }

    pub fn claim(&self, ticket: Ticket) -> Result<MapRun, PackError> {
        if !country(&ticket.id) {
            return Err(PackError::NotFound("Country map".into()));
        }
        let lease = store::lock(&self.root.join("worker.lock"))?;
        lease.try_lock_exclusive().map_err(|_| busy())?;
        self.update(&ticket, |row| {
            if row.state != "queued" {
                return Err(PackError::Mismatch("Stale map dispatch".into()));
            }
            row.state = if row.estimate {
                "checking"
            } else {
                "downloading"
            }
            .into();
            Ok(())
        })?;
        Ok(MapRun { lease, ticket })
    }

    fn continuing(&self, ticket: &Ticket) -> Result<(), PackError> {
        let row = self.current(ticket)?;
        if !matches!(row.state.as_str(), "checking" | "downloading" | "verifying") {
            return Err(PackError::Mismatch("Map operation stopped".into()));
        }
        Ok(())
    }

    /// For shells without a native network lease (iPhone): same limits as the Android bridge's client.
    pub fn execute_with_default_client(&self, run: &MapRun) -> Result<(), PackError> {
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| PackError::Http("Map connection unavailable".into()))?;
        self.execute(run, &client)
    }

    pub fn execute(
        &self,
        run: &MapRun,
        client: &reqwest::blocking::Client,
    ) -> Result<(), PackError> {
        let result = self.extract(run, client);
        self.update(&run.ticket, |row| {
            let pending = if row.state == "cancelling" {
                "cancelled"
            } else if row.state == "pausing" {
                "paused"
            } else if result.is_ok() {
                if row.estimate {
                    "ready"
                } else {
                    "completed"
                }
            } else {
                "failed"
            };
            row.pending = Some(pending.into());
            if let Err(error) = &result {
                row.error = Some(
                    match error {
                        PackError::NoSpace => "no_space",
                        PackError::Checksum => "checksum",
                        PackError::Http(_) => "network",
                        _ => "map_unavailable",
                    }
                    .into(),
                );
            }
            row.state = "finishing".into();
            Ok(())
        })?;
        result
    }

    pub fn finish(&self, run: MapRun) -> Result<(), PackError> {
        self.update(&run.ticket, |row| {
            row.state = if row.state == "cancelling" {
                "cancelled".into()
            } else if row.state == "pausing" {
                "paused".into()
            } else {
                row.pending.take().unwrap_or_else(|| "paused".into())
            };
            if row.state == "cancelled" {
                row.source = None;
                row.completed_bytes = 0;
            }
            Ok(())
        })?;
        drop(run.lease);
        Ok(())
    }

    fn extract(&self, run: &MapRun, client: &reqwest::blocking::Client) -> Result<(), PackError> {
        let ticket = &run.ticket;
        self.continuing(ticket)?;
        let mut row = self.current(ticket)?;
        let directory = self.root.join(&ticket.id);
        fs::create_dir_all(&directory)?;
        let region = directory.join("country.geojson");
        let geometry: &[u8] = match ticket.id.as_str() {
            "mx" => include_bytes!("../../../native/maps-extract/countries/mx.geojson"),
            "us" => include_bytes!("../../../native/maps-extract/countries/us.geojson"),
            "ca" => include_bytes!("../../../native/maps-extract/countries/ca.geojson"),
            _ => return Err(PackError::Mismatch("Country map".into())),
        };
        write_atomic(&region, geometry)?;
        if row.source.is_none() {
            let source = store::snapshot(client)?;
            self.update(ticket, |row| {
                row.source = Some(source.clone());
                Ok(())
            })?;
            row.source = Some(source);
        }
        let source = row
            .source
            .ok_or_else(|| PackError::NotFound("Map source".into()))?;
        let identity = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}:{}:{}",
                source.url, source.etag, row.zoom, row.cache_epoch
            ))
        );
        let cache = directory.join(format!("cache-{identity}"));
        fs::create_dir_all(&cache)?;
        let output = directory.join(format!("stage-{}.pmtiles", ticket.epoch));
        ensure_regular_or_absent(&output)?;
        let mut progress = |bytes| {
            self.continuing(ticket)?;
            if bytes != 0 {
                self.update(ticket, |row| {
                    row.completed_bytes = row.completed_bytes.saturating_add(bytes);
                    Ok(())
                })?;
            }
            Ok(())
        };
        let mut preflight = |output: u64, transfer: u64| {
            self.continuing(ticket)?;
            self.update(ticket, |row| {
                row.output_bytes = output;
                row.download_bytes = transfer;
                Ok(())
            })?;
            if !row.estimate
                && fs2::available_space(&directory)?
                    < output
                        .saturating_add(transfer)
                        .saturating_add(64 * 1024 * 1024)
            {
                return Err(PackError::NoSpace);
            }
            Ok(())
        };
        extract::run(
            &cache,
            &source,
            client,
            &region,
            &output,
            row.zoom,
            row.estimate,
            &mut progress,
            &mut preflight,
        )?;
        if row.estimate {
            return Ok(());
        }
        self.continuing(ticket)?;
        self.update(ticket, |row| {
            row.state = "verifying".into();
            Ok(())
        })?;
        let source_file = Arc::new(PreadSource::open(&output)?);
        TileSource::open(source_file)?;
        let mut file = File::open(&output)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            self.continuing(ticket)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let sha256 = format!("{:x}", digest.finalize());
        let bytes = file.metadata()?.len();
        drop(file);
        let installed_path = directory.join(format!("{sha256}.pmtiles"));
        ensure_regular_or_absent(&installed_path)?;
        if !installed_path.try_exists()? {
            fs::rename(&output, &installed_path)?;
        }
        verify_sha256(&installed_path, &sha256)?;
        self.update(ticket, |row| {
            if row.state != "verifying" {
                return Err(PackError::Mismatch("Map operation stopped".into()));
            }
            row.installed = Some(Installed {
                sha256,
                bytes,
                zoom: row.zoom,
                source: source.url,
            });
            Ok(())
        })?;
        Ok(())
    }

    pub fn installed_path(&self, row: &Row) -> Result<Option<PathBuf>, PackError> {
        let Some(installed) = &row.installed else {
            return Ok(None);
        };
        if !country(&row.id)
            || installed.sha256.len() != 64
            || !installed.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(PackError::Corrupt("Installed map reference".into()));
        }
        let path = self
            .root
            .join(&row.id)
            .join(format!("{}.pmtiles", installed.sha256));
        ensure_regular_or_absent(&path)?;
        if fs::metadata(&path)?.len() != installed.bytes {
            return Err(PackError::Truncated);
        }
        verify_sha256(&path, &installed.sha256)?;
        Ok(Some(path))
    }

    pub fn remove(&self, id: &str) -> Result<(), PackError> {
        if !country(id) {
            return Err(PackError::NotFound("Country map".into()));
        }
        let worker = store::lock(&self.root.join("worker.lock"))?;
        worker.try_lock_exclusive().map_err(|_| busy())?;
        self.transaction(|ledger| {
            if let Some(row) = ledger.rows.iter_mut().find(|r| r.id == id) {
                if active(&row.state) {
                    return Err(busy());
                }
                row.installed = None;
                row.state = "cancelled".into();
            }
            Ok(())
        })
    }
}
