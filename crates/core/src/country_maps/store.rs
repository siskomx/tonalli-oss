use super::{ranges::Snapshot, CountryMaps, Row, Ticket};
use crate::{
    download::{ensure_regular_or_absent, write_atomic},
    pack::PackError,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Ledger {
    pub rows: Vec<Row>,
}

pub(super) fn lock(path: &Path) -> Result<File, PackError> {
    ensure_regular_or_absent(path)?;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?)
}

impl CountryMaps {
    pub(super) fn transaction<T>(
        &self,
        work: impl FnOnce(&mut Ledger) -> Result<T, PackError>,
    ) -> Result<T, PackError> {
        let gate = lock(&self.root.join("store.lock"))?;
        gate.lock_exclusive()?;
        let path = self.root.join("maps.json");
        ensure_regular_or_absent(&path)?;
        let mut ledger = if path.try_exists()? {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(PackError::Corrupt("Map ledger too large".into()));
            }
            serde_json::from_slice::<Ledger>(&fs::read(&path)?)
                .map_err(|_| PackError::Corrupt("Map ledger".into()))?
        } else {
            Ledger::default()
        };
        if ledger.rows.len() > 3
            || ledger.rows.iter().any(|row| {
                !super::country(&row.id)
                    || row.epoch == 0
                    || row
                        .source
                        .as_ref()
                        .is_some_and(|source| !valid_snapshot(source))
            })
        {
            return Err(PackError::Corrupt("Map ledger entries".into()));
        }
        let before =
            serde_json::to_vec(&ledger).map_err(|_| PackError::Corrupt("Map ledger".into()))?;
        let result = work(&mut ledger)?;
        let after =
            serde_json::to_vec(&ledger).map_err(|_| PackError::Corrupt("Map ledger".into()))?;
        if after != before {
            write_atomic(&path, &after)?;
        }
        Ok(result)
    }

    pub(super) fn current(&self, ticket: &Ticket) -> Result<Row, PackError> {
        self.transaction(|ledger| {
            ledger
                .rows
                .iter()
                .find(|row| row.id == ticket.id && row.epoch == ticket.epoch)
                .cloned()
                .ok_or_else(|| PackError::Mismatch("Stale map attempt".into()))
        })
    }

    pub(super) fn update(
        &self,
        ticket: &Ticket,
        work: impl FnOnce(&mut Row) -> Result<(), PackError>,
    ) -> Result<(), PackError> {
        self.transaction(|ledger| {
            work(
                ledger
                    .rows
                    .iter_mut()
                    .find(|row| row.id == ticket.id && row.epoch == ticket.epoch)
                    .ok_or_else(|| PackError::Mismatch("Stale map attempt".into()))?,
            )
        })
    }
}

fn valid_snapshot(source: &Snapshot) -> bool {
    let Some(key) = source.url.strip_prefix("https://build.protomaps.com/") else {
        return false;
    };
    key.len() == 16
        && key.ends_with(".pmtiles")
        && key.as_bytes()[..8].iter().all(u8::is_ascii_digit)
        && source.size >= 127
        && source.etag.len() >= 3
        && source.etag.len() < 256
        && source.etag.starts_with('"')
        && source.etag.ends_with('"')
}

pub(super) fn snapshot(client: &reqwest::blocking::Client) -> Result<Snapshot, PackError> {
    // Publisher metadata supplies the dated immutable source; web callers cannot supply URLs.
    let response = client
        .get("https://build-metadata.protomaps.dev/builds.json")
        .header("User-Agent", "Tonalli/0.1")
        .send()
        .map_err(|_| PackError::Http("Map source unavailable".into()))?;
    if !response.status().is_success() {
        return Err(PackError::Http("Map source unavailable".into()));
    }
    use std::io::Read;
    let mut data = Vec::new();
    response.take(1024 * 1024 + 1).read_to_end(&mut data)?;
    if data.len() > 1024 * 1024 {
        return Err(PackError::Corrupt("Map source metadata too large".into()));
    }
    let builds: Vec<serde_json::Value> = serde_json::from_slice(&data)
        .map_err(|_| PackError::Corrupt("Map source metadata".into()))?;
    let key = builds
        .iter()
        .rev()
        .filter(|row| {
            row.get("version")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v.starts_with("4."))
        })
        .filter_map(|row| row.get("key").and_then(|v| v.as_str()))
        .find(|key| {
            key.len() == 16
                && key.ends_with(".pmtiles")
                && key.as_bytes()[..8].iter().all(u8::is_ascii_digit)
        })
        .ok_or_else(|| PackError::NotFound("Compatible map source".into()))?;
    let url = format!("https://build.protomaps.com/{key}");
    let mut response = client
        .get(&url)
        .header("User-Agent", "Tonalli/0.1")
        .header("Range", "bytes=0-126")
        .send()
        .map_err(|_| PackError::Http("Map source unavailable".into()))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(PackError::Http("Map source cannot resume".into()));
    }
    let range = response
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let size = range
        .strip_prefix("bytes 0-126/")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 127)
        .ok_or_else(|| PackError::Corrupt("Map source range".into()))?;
    let etag = response
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with('"') && v.ends_with('"') && v.len() >= 3 && v.len() < 256)
        .ok_or_else(|| PackError::Corrupt("Map source validator".into()))?
        .to_owned();
    let mut header = Vec::new();
    response.by_ref().take(128).read_to_end(&mut header)?;
    if header.len() != 127 || header.get(..8) != Some(b"PMTiles\x03") {
        return Err(PackError::Corrupt("Map source format".into()));
    }
    Ok(Snapshot { url, etag, size })
}
