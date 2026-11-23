//! Persistent content jobs. Native hosts dispatch and confirm task/binding teardown explicitly.
mod storage;
#[cfg(test)]
mod tests;
mod worker;

use super::{storage as managed, ManagedStore};
use crate::{
    catalog::{Catalog, CatalogEntry, CatalogKind},
    download::{pack_path, DownloadAction},
    pack::{verify_sha256, PackError},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    path::PathBuf,
    sync::Arc,
};
use storage::{invalid, Ledger, Paths, Row, MAX_JOBS};
use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum JobError {
    #[error("Content job is busy")]
    Busy,
    #[error("Content catalog changed")]
    CatalogChanged,
    #[error("Content job or entry was not found")]
    NotFound,
    #[error("Stale content job attempt")]
    StaleAttempt,
    #[error("Invalid content job request")]
    InvalidRequest,
    #[error(transparent)]
    Pack(#[from] PackError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobAction {
    Pause,
    Resume,
    Cancel,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Downloading,
    Verifying,
    Installing,
    Pausing,
    Paused,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl JobState {
    fn active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Downloading
                | Self::Verifying
                | Self::Installing
                | Self::Pausing
                | Self::Cancelling
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub entry_id: String,
    pub state: JobState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub verified_bytes: u64,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobTicket {
    pub id: String,
    pub epoch: u64,
}

pub struct JobReply {
    pub job: JobSnapshot,
    pub ticket: JobTicket,
    pub dispatch: bool,
}

/// Linear worker ownership: native code returns this only after its task and network cleanup end.
/// No caller-supplied completed/installed outcome exists in this policy-only API.
pub struct JobRun {
    root: PathBuf,
    ticket: JobTicket,
    attempt: u64,
    lease: Option<File>,
    binding: Arc<Catalog>,
    spec: crate::download::DownloadSpec,
    proof: Option<worker::ExitProof>,
}

pub struct ContentJobs {
    store: Arc<ManagedStore>,
    catalog: Arc<Catalog>,
    authority: String,
    paths: Paths,
}

struct Verified {
    raw: Vec<u8>,
    ledger: Ledger,
    authorities: HashMap<String, Arc<Catalog>>,
}

impl Verified {
    fn entry(&self, row: &Row) -> Result<&CatalogEntry, JobError> {
        self.authorities
            .get(&row.authority_fingerprint)
            .and_then(|catalog| {
                catalog
                    .entries()
                    .iter()
                    .find(|entry| entry.id == row.entry_id)
            })
            .ok_or_else(invalid)
    }

    fn snapshot(&self, row: &Row) -> Result<JobSnapshot, JobError> {
        Ok(JobSnapshot {
            id: row.id.clone(),
            entry_id: row.entry_id.clone(),
            state: row.state,
            downloaded_bytes: row.downloaded_bytes,
            total_bytes: self.entry(row)?.bytes,
            verified_bytes: row.verified_bytes,
            error_code: row.error_code.clone(),
        })
    }

    fn reply(&self, row: &Row, dispatch: bool) -> Result<JobReply, JobError> {
        Ok(JobReply {
            job: self.snapshot(row)?,
            ticket: JobTicket {
                id: row.id.clone(),
                epoch: row.epoch,
            },
            dispatch,
        })
    }
}

impl ContentJobs {
    pub(crate) fn managed_store(&self) -> &ManagedStore {
        &self.store
    }
    pub fn open(store: Arc<ManagedStore>, catalog: Arc<Catalog>) -> Result<Self, JobError> {
        let snapshot = catalog.snapshot_for_key(&store.pin)?;
        let authority = managed::fingerprint(&snapshot);
        let paths = Paths::new(&store)?;
        // Global order is StoreMeta -> Jobs. Authority/proofs are resolved outside jobs.lock;
        // updates recheck exact ledger bytes under that lock before any policy mutation.
        store.accept_catalog(&catalog)?;
        {
            let _store_gate = store.gate()?;
            managed::immutable(
                &store.path(&format!("authority/{authority}.tcat"))?,
                &snapshot,
            )?;
            paths.initialize()?;
        }
        let jobs = Self {
            store,
            catalog,
            authority,
            paths,
        };
        jobs.read_verified()?;
        Ok(jobs)
    }

    fn read_verified(&self) -> Result<Verified, JobError> {
        let (raw, ledger) = self.paths.read()?;
        let mut authorities = HashMap::new();
        for fingerprint in std::iter::once(&self.authority)
            .chain(ledger.jobs.iter().map(|row| &row.authority_fingerprint))
        {
            if authorities.contains_key(fingerprint) {
                continue;
            }
            let path = self.store.path(&format!("authority/{fingerprint}.tcat"))?;
            if fs::metadata(&path).map_err(PackError::from)?.len() > 1024 * 1024 + 73 {
                return Err(invalid());
            }
            verify_sha256(&path, fingerprint)?;
            authorities.insert(
                fingerprint.clone(),
                Arc::new(Catalog::load_accepted(&path, &self.store.pin)?),
            );
        }
        let verified = Verified {
            raw,
            ledger,
            authorities,
        };
        for row in &verified.ledger.jobs {
            let entry = verified.entry(row)?;
            if row.downloaded_bytes > entry.bytes || row.verified_bytes > entry.bytes {
                return Err(invalid());
            }
            if row.state == JobState::Completed
                && (row.downloaded_bytes != entry.bytes || row.verified_bytes != entry.bytes)
            {
                return Err(invalid());
            }
        }
        Ok(verified)
    }

    fn update<T>(
        &self,
        change: impl FnOnce(&mut Ledger, &Verified) -> Result<T, JobError>,
    ) -> Result<T, JobError> {
        for _ in 0..8 {
            let verified = self.read_verified()?;
            let _gate = self.paths.lock()?;
            let (current, mut ledger) = self.paths.read()?;
            if current != verified.raw {
                continue;
            }
            let result = change(&mut ledger, &verified)?;
            self.paths.publish(&ledger)?;
            return Ok(result);
        }
        Err(JobError::Busy)
    }

    pub fn list(&self) -> Result<Vec<JobSnapshot>, JobError> {
        let verified = self.read_verified()?;
        verified
            .ledger
            .jobs
            .iter()
            .map(|row| verified.snapshot(row))
            .collect()
    }

    /// Native restart reconciliation only; this does not infer OS scheduling or resume work.
    pub fn active_ticket(&self) -> Result<Option<JobTicket>, JobError> {
        let verified = self.read_verified()?;
        Ok(verified
            .ledger
            .active
            .as_ref()
            .and_then(|id| verified.ledger.jobs.iter().find(|row| &row.id == id))
            .map(|row| JobTicket {
                id: row.id.clone(),
                epoch: row.epoch,
            }))
    }

    pub fn start(&self, entry_id: &str, revision: u64) -> Result<JobReply, JobError> {
        if revision != self.catalog.revision() {
            return Err(JobError::CatalogChanged);
        }
        let entry = self
            .catalog
            .entries()
            .iter()
            .find(|entry| entry.id == entry_id)
            .ok_or(JobError::NotFound)?;
        if !matches!(entry.kind, CatalogKind::Zim | CatalogKind::Model) {
            return Err(JobError::InvalidRequest);
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| PackError::Io("Content job ID allocation failed".into()))?;
        let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        self.update(|ledger, verified| {
            if ledger.active.is_some() {
                return Err(JobError::Busy);
            }
            let _idle = self.paths.worker()?;
            if ledger.jobs.iter().any(|row| {
                row.entry_id == entry_id
                    && !matches!(row.state, JobState::Cancelled | JobState::Completed)
            }) {
                return Err(JobError::Busy);
            }
            if ledger.jobs.len() == MAX_JOBS {
                let index = ledger
                    .jobs
                    .iter()
                    .position(|row| matches!(row.state, JobState::Cancelled | JobState::Completed))
                    .ok_or(JobError::Busy)?;
                ledger.jobs.remove(index);
            }
            if ledger.jobs.iter().any(|row| row.id == id) {
                return Err(JobError::Busy);
            }
            let row = Row {
                id,
                entry_id: entry_id.to_owned(),
                authority_fingerprint: self.authority.clone(),
                epoch: 1,
                attempt: 0,
                claimed: false,
                state: JobState::Queued,
                intent: None,
                downloaded_bytes: 0,
                verified_bytes: 0,
                error_code: None,
            };
            let reply = verified.reply(&row, true)?;
            ledger.active = Some(row.id.clone());
            ledger.jobs.push(row);
            Ok(reply)
        })
    }

    pub fn request(&self, id: &str, action: JobAction) -> Result<JobReply, JobError> {
        self.request_epoch(id, None, action)
    }

    /// Native OS callbacks must carry their admitted ticket, never stop a replacement epoch.
    pub fn request_ticket(
        &self,
        ticket: &JobTicket,
        action: JobAction,
    ) -> Result<JobReply, JobError> {
        self.request_epoch(&ticket.id, Some(ticket.epoch), action)
    }

    fn request_epoch(
        &self,
        id: &str,
        epoch: Option<u64>,
        action: JobAction,
    ) -> Result<JobReply, JobError> {
        self.update(|ledger, verified| {
            let index = ledger
                .jobs
                .iter()
                .position(|row| row.id == id)
                .ok_or(JobError::NotFound)?;
            let row = &ledger.jobs[index];
            if epoch.is_some_and(|epoch| epoch != row.epoch) {
                return Err(JobError::StaleAttempt);
            }
            if matches!(action, JobAction::Resume | JobAction::Retry) {
                if ledger.active.is_some() {
                    return Err(JobError::Busy);
                }
                let _idle = self.paths.worker()?;
                // Resume keeps the admitted, pinned historical snapshot; it does not
                // reactivate that catalog or lower the current rollback floor.
                if action == JobAction::Retry && row.authority_fingerprint != self.authority {
                    return Err(JobError::CatalogChanged);
                }
                if !matches!(
                    (action, row.state),
                    (JobAction::Resume, JobState::Paused)
                        | (JobAction::Retry, JobState::Cancelled | JobState::Failed)
                ) {
                    return Err(JobError::InvalidRequest);
                }
                if ledger.jobs.iter().any(|other| {
                    other.id != id
                        && other.entry_id == row.entry_id
                        && !matches!(other.state, JobState::Cancelled | JobState::Completed)
                }) {
                    return Err(JobError::Busy);
                }
                let row = &mut ledger.jobs[index];
                row.epoch = row.epoch.checked_add(1).ok_or(JobError::InvalidRequest)?;
                row.claimed = false;
                row.intent = None;
                row.error_code = None;
                row.state = JobState::Queued;
                ledger.active = Some(id.to_owned());
                return verified.reply(row, true);
            }
            let dispatch = action == JobAction::Cancel
                && matches!(row.state, JobState::Paused | JobState::Failed);
            let _idle = if dispatch {
                if ledger.active.is_some() {
                    return Err(JobError::Busy);
                }
                Some(self.paths.worker()?)
            } else {
                None
            };
            let row = &mut ledger.jobs[index];
            match (action, row.state) {
                (
                    JobAction::Pause,
                    JobState::Queued | JobState::Downloading | JobState::Verifying,
                ) => {
                    row.intent = Some(action);
                    row.state = JobState::Pausing;
                }
                (JobAction::Pause, JobState::Pausing | JobState::Paused | JobState::Cancelling) => {
                }
                (
                    JobAction::Cancel,
                    JobState::Queued
                    | JobState::Downloading
                    | JobState::Verifying
                    | JobState::Pausing,
                ) => {
                    row.intent = Some(action);
                    row.state = JobState::Cancelling;
                }
                (JobAction::Cancel, JobState::Paused | JobState::Failed) => {
                    row.intent = Some(action);
                    row.state = JobState::Cancelling;
                    ledger.active = Some(id.to_owned());
                }
                (
                    JobAction::Cancel,
                    JobState::Cancelling | JobState::Cancelled | JobState::Completed,
                ) => {}
                _ => return Err(JobError::InvalidRequest),
            }
            verified.reply(row, dispatch)
        })
    }

    pub fn claim(&self, ticket: &JobTicket) -> Result<JobRun, JobError> {
        self.update(|ledger, verified| {
            let row = ledger
                .jobs
                .iter_mut()
                .find(|row| row.id == ticket.id)
                .ok_or(JobError::NotFound)?;
            if row.epoch != ticket.epoch
                || row.claimed
                || !row.state.active()
                || ledger.active.as_deref() != Some(&row.id)
            {
                return Err(JobError::StaleAttempt);
            }
            let lease = self.paths.worker()?;
            row.attempt = row.attempt.checked_add(1).ok_or(JobError::InvalidRequest)?;
            row.claimed = true;
            let binding = Arc::clone(
                verified
                    .authorities
                    .get(&row.authority_fingerprint)
                    .ok_or_else(invalid)?,
            );
            let spec = self.store.cache_spec(&binding, &row.entry_id)?;
            Ok(JobRun {
                root: self.store.root().to_owned(),
                ticket: ticket.clone(),
                attempt: row.attempt,
                lease: Some(lease),
                binding,
                spec,
                proof: None,
            })
        })
    }

    fn run_row<'a>(&self, ledger: &'a Ledger, run: &JobRun) -> Result<&'a Row, JobError> {
        if run.root != self.store.root() || run.lease.is_none() {
            return Err(JobError::StaleAttempt);
        }
        let row = ledger
            .jobs
            .iter()
            .find(|row| row.id == run.ticket.id)
            .ok_or(JobError::NotFound)?;
        if row.epoch != run.ticket.epoch || row.attempt != run.attempt {
            return Err(JobError::StaleAttempt);
        }
        Ok(row)
    }

    pub fn requested(&self, run: &JobRun) -> Result<DownloadAction, JobError> {
        let (_, ledger) = self.paths.read()?;
        let row = self.run_row(&ledger, run)?;
        Ok(match row.intent {
            Some(JobAction::Pause) => DownloadAction::Pause,
            Some(JobAction::Cancel) => DownloadAction::Cancel,
            _ => DownloadAction::Continue,
        })
    }

    /// Caller must first join/finish the unique native task and release its network binding.
    /// Execution outcomes come only from Core; an unexecuted empty policy run can confirm stops.
    pub fn finish_exited(&self, run: &mut JobRun) -> Result<JobSnapshot, JobError> {
        if run.proof.is_some() {
            return self.finish_execution(run);
        }
        self.finish_policy_exited(run)
    }

    fn finish_policy_exited(&self, run: &mut JobRun) -> Result<JobSnapshot, JobError> {
        let verified = self.read_verified()?;
        let row = self.run_row(&verified.ledger, run)?;
        if row.intent == Some(JobAction::Pause) {
            self.flush_candidates(run)?;
        }
        let authority = verified
            .authorities
            .get(&row.authority_fingerprint)
            .ok_or_else(invalid)?;
        let spec = self.store.cache_spec(authority, &row.entry_id)?;
        let staging = [
            &spec.path,
            &format!("{}.part", spec.path),
            &format!("{}.part.meta", spec.path),
        ]
        .iter()
        .map(|relative| pack_path(self.store.root(), relative).map_err(JobError::from))
        .collect::<Result<Vec<_>, _>>()?;
        let remaining = staging.iter().try_fold(false, |any, path| {
            path.try_exists()
                .map(|exists| any || exists)
                .map_err(PackError::from)
        })?;
        let result =
            self.store
                .with_unpublished_candidate(&run.binding, &run.spec.id, |candidate| {
                    self.update(|ledger, verified| {
                        let row = self.run_row(ledger, run)?;
                        let state = match row.intent {
                            Some(JobAction::Pause) => JobState::Paused,
                            Some(JobAction::Cancel)
                                if !remaining
                                    && !candidate
                                        .map(|path| path.try_exists().map_err(PackError::from))
                                        .transpose()?
                                        .unwrap_or(false) =>
                            {
                                JobState::Cancelled
                            }
                            _ => return Err(JobError::InvalidRequest),
                        };
                        let index = ledger
                            .jobs
                            .iter()
                            .position(|row| row.id == run.ticket.id)
                            .ok_or(JobError::NotFound)?;
                        let row = &mut ledger.jobs[index];
                        row.state = state;
                        row.claimed = false;
                        ledger.active = None;
                        verified.snapshot(row)
                    })
                })?;
        run.lease.take();
        Ok(result)
    }

    /// Explicit native reconciliation only. The host must first confirm there is no matching
    /// pending or running OS dispatch, including a task admitted but never claimed.
    pub fn reconcile_interrupted(&self, ticket: &JobTicket) -> Result<JobSnapshot, JobError> {
        let verified = self.read_verified()?;
        let row = verified
            .ledger
            .jobs
            .iter()
            .find(|row| row.id == ticket.id)
            .ok_or(JobError::NotFound)?;
        let authority = row.authority_fingerprint.clone();
        let binding = verified.authorities.get(&authority).ok_or_else(invalid)?;
        let spec = self.store.cache_spec(binding, &row.entry_id)?;
        self.update(|ledger, verified| {
            let _idle = self.paths.worker()?;
            let row = ledger
                .jobs
                .iter_mut()
                .find(|row| row.id == ticket.id)
                .ok_or(JobError::NotFound)?;
            if row.epoch != ticket.epoch
                || !row.state.active()
                || row.authority_fingerprint != authority
            {
                return Err(JobError::StaleAttempt);
            }
            row.claimed = false;
            let mut flush_failed = false;
            if row.intent != Some(JobAction::Cancel) {
                flush_failed = self.flush_spec(&spec).is_err();
                row.intent = if flush_failed {
                    None
                } else {
                    Some(JobAction::Pause)
                };
                row.state = if flush_failed {
                    JobState::Failed
                } else {
                    JobState::Paused
                };
                ledger.active = None;
            }
            row.error_code = Some(if flush_failed { "io" } else { "interrupted" }.into());
            verified.snapshot(row)
        })
    }

    /// Native-only: scheduling definitely rejected this ticket and guarantees that no matching
    /// pending/running task exists. An uncertain scheduling result must never call this method.
    pub fn dispatch_rejected(&self, ticket: &JobTicket) -> Result<JobSnapshot, JobError> {
        let verified = self.read_verified()?;
        let row = verified
            .ledger
            .jobs
            .iter()
            .find(|row| row.id == ticket.id)
            .ok_or(JobError::NotFound)?;
        let authority = row.authority_fingerprint.clone();
        let binding = verified.authorities.get(&authority).ok_or_else(invalid)?;
        let spec = self.store.cache_spec(binding, &row.entry_id)?;
        self.update(|ledger, verified| {
            let _idle = self.paths.worker()?;
            let row = ledger
                .jobs
                .iter_mut()
                .find(|row| row.id == ticket.id)
                .ok_or(JobError::NotFound)?;
            if row.epoch != ticket.epoch
                || row.claimed
                || !row.state.active()
                || row.authority_fingerprint != authority
            {
                return Err(JobError::StaleAttempt);
            }
            // Invalidate even repeated rejection callbacks before a new cleanup admission.
            row.epoch = row.epoch.checked_add(1).ok_or(JobError::InvalidRequest)?;
            let flush_failed =
                row.intent != Some(JobAction::Cancel) && self.flush_spec(&spec).is_err();
            if row.intent == Some(JobAction::Cancel) || flush_failed {
                row.state = JobState::Failed;
                row.intent = None;
            } else {
                row.state = JobState::Paused;
                row.intent = Some(JobAction::Pause);
            }
            row.error_code = Some(
                if flush_failed {
                    "io"
                } else {
                    "dispatch_rejected"
                }
                .into(),
            );
            ledger.active = None;
            verified.snapshot(row)
        })
    }

    /// Product removal shares the native store's StoreMeta -> Jobs final-admission fence.
    pub fn remove_installed(&self, id: &str) -> Result<super::RemoveOutcome, JobError> {
        self.read_verified()?;
        self.store
            .remove_admitted(id, || {
                let guard = self.paths.lock().map_err(policy_pack_error)?;
                let (_, ledger) = self.paths.read().map_err(policy_pack_error)?;
                if ledger.jobs.iter().any(|row| {
                    row.entry_id == id
                        && !matches!(row.state, JobState::Completed | JobState::Cancelled)
                }) {
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                Ok(std::ops::ControlFlow::Continue(guard))
            })
            .map_err(JobError::from)
    }
}

fn policy_pack_error(error: JobError) -> PackError {
    match error {
        JobError::Pack(error) => error,
        _ => PackError::Corrupt("Content job metadata unavailable".into()),
    }
}
