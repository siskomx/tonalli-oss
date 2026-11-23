use super::{JobAction, JobError, JobState};
use crate::{content::storage as managed, pack::PackError, server::PackId};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 512 * 1024;
pub(super) const MAX_JOBS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub id: String,
    pub entry_id: String,
    pub authority_fingerprint: String,
    pub epoch: u64,
    pub attempt: u64,
    pub claimed: bool,
    pub state: JobState,
    pub intent: Option<JobAction>,
    pub downloaded_bytes: u64,
    pub verified_bytes: u64,
    pub error_code: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    version: u32,
    pub jobs: Vec<Row>,
    pub active: Option<String>,
}

pub(super) fn invalid() -> JobError {
    PackError::Corrupt("Invalid or missing content job metadata".into()).into()
}

impl Ledger {
    fn empty() -> Self {
        Self {
            version: 1,
            jobs: Vec::new(),
            active: None,
        }
    }

    fn validate(&self) -> Result<(), JobError> {
        if self.version != 1 || self.jobs.len() > MAX_JOBS {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        let mut protected = std::collections::HashSet::new();
        let mut active = None;
        for row in &self.jobs {
            if row.id.len() != 32
                || !row
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !ids.insert(&row.id)
                || PackId::new(row.entry_id.clone()).is_err()
                || !managed::hex(&row.authority_fingerprint)
                || row.epoch == 0
                || (row.claimed && row.attempt == 0)
                || row.error_code.as_ref().is_some_and(|code| {
                    code.is_empty()
                        || code.len() > 64
                        || !code
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
            {
                return Err(invalid());
            }
            let intent_valid = match row.state {
                JobState::Queued
                | JobState::Downloading
                | JobState::Verifying
                | JobState::Installing
                | JobState::Completed => row.intent.is_none(),
                JobState::Failed => row.intent.is_none() && row.error_code.is_some(),
                JobState::Pausing | JobState::Paused => row.intent == Some(JobAction::Pause),
                JobState::Cancelling | JobState::Cancelled => row.intent == Some(JobAction::Cancel),
            };
            if !intent_valid {
                return Err(invalid());
            }
            if row.state.active() {
                if active.replace(row.id.as_str()).is_some() {
                    return Err(invalid());
                }
            } else if row.claimed {
                return Err(invalid());
            }
            if !matches!(row.state, JobState::Cancelled | JobState::Completed)
                && !protected.insert(&row.entry_id)
            {
                return Err(invalid());
            }
        }
        if active != self.active.as_deref() {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, JobError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        Ok(bytes)
    }
}

pub(super) struct Paths {
    pub ledger: PathBuf,
    gate: PathBuf,
    worker: PathBuf,
}

fn regular(path: &Path) -> Result<(), JobError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| invalid())?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked || !metadata.is_file() {
        return Err(invalid());
    }
    Ok(())
}

impl Paths {
    pub fn new(store: &super::ManagedStore) -> Result<Self, JobError> {
        Ok(Self {
            ledger: store.path("jobs.json")?,
            gate: store.path("jobs.lock")?,
            worker: store.path("jobs-worker.lock")?,
        })
    }

    pub fn initialize(&self) -> Result<(), JobError> {
        let fresh = match fs::symlink_metadata(&self.gate) {
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Retained lock files witness initialization; never recreate a lost ledger/lock.
                if self.ledger.try_exists().map_err(PackError::from)?
                    || self.worker.try_exists().map_err(PackError::from)?
                {
                    return Err(invalid());
                }
                true
            }
            Err(e) => return Err(PackError::from(e).into()),
        };
        let (file, created) = if fresh {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&self.gate)
            {
                Ok(file) => (file, true),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    (open(&self.gate)?, false)
                }
                Err(e) => return Err(PackError::from(e).into()),
            }
        } else {
            (open(&self.gate)?, false)
        };
        FileExt::lock_exclusive(&file).map_err(PackError::from)?;
        if created {
            managed::flush(&file, managed::parent(&self.gate)?).map_err(PackError::from)?;
            let worker = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&self.worker)
                .map_err(PackError::from)?;
            managed::flush(&worker, managed::parent(&self.worker)?).map_err(PackError::from)?;
            self.publish(&Ledger::empty())?;
        }
        self.check()?;
        Ok(())
    }

    pub fn check(&self) -> Result<(), JobError> {
        for path in [&self.ledger, &self.gate, &self.worker] {
            regular(path)?;
        }
        Ok(())
    }

    pub fn read(&self) -> Result<(Vec<u8>, Ledger), JobError> {
        self.check()?;
        let mut bytes = Vec::new();
        File::open(&self.ledger)
            .map_err(PackError::from)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(PackError::from)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        ledger.validate()?;
        Ok((bytes, ledger))
    }

    pub fn lock(&self) -> Result<File, JobError> {
        let file = open(&self.gate)?;
        FileExt::lock_exclusive(&file).map_err(PackError::from)?;
        Ok(file)
    }

    pub fn worker(&self) -> Result<File, JobError> {
        let file = open(&self.worker)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(file),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                Err(JobError::Busy)
            }
            Err(e) => Err(PackError::from(e).into()),
        }
    }

    pub fn publish(&self, ledger: &Ledger) -> Result<(), JobError> {
        managed::publish(&self.ledger, &ledger.encode()?).map_err(Into::into)
    }
}

fn open(path: &Path) -> Result<File, JobError> {
    regular(path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| PackError::from(e).into())
}
