//! Native lifecycle adapter for the existing controlled index engine, not a queue or indexer.
mod source;
mod storage;
mod worker;
use super::{build, BuildControl, BuildOutcome, BuildPhase, BuildProgress, Index};
use crate::{pack::PackError, server::PackId, zim::Zim};
use serde::{Deserialize, Serialize};
pub use source::IndexSource;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};
use storage::{Ledger, Paths, Row};
use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum IndexJobError {
    #[error("Index preparation is already owned")]
    Busy,
    #[error("Index preparation ticket is stale")]
    StaleTicket,
    #[error("Invalid index preparation request")]
    InvalidRequest,
    #[error("Index preparation source changed")]
    SourceChanged,
    #[error("Index preparation was not found")]
    NotFound,
    #[error(transparent)]
    Pack(#[from] PackError),
}
impl From<std::io::Error> for IndexJobError {
    fn from(value: std::io::Error) -> Self {
        Self::Pack(value.into())
    }
}
fn invalid() -> IndexJobError {
    PackError::Corrupt("Invalid index preparation metadata".into()).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pause,
    Resume,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchFailure {
    Rejected,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Checking,
    Indexing,
    Validating,
    Activating,
    Pausing,
    Cancelling,
    Paused,
    Cancelled,
    Ready,
    Failed,
}
impl State {
    fn active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Checking
                | Self::Indexing
                | Self::Validating
                | Self::Activating
                | Self::Pausing
                | Self::Cancelling
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub pack: String,
    pub epoch: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub job_id: String,
    pub ticket: Ticket,
    pub state: State,
    pub progress: Option<BuildProgress>,
    pub error_code: Option<String>,
    pub dispatch: bool,
}

pub struct IndexJobs {
    paths: Paths,
}
/// Held until native work actually exits; no caller-selected successful outcome is accepted.
pub struct Run {
    root: PathBuf,
    ticket: Ticket,
    source: Option<IndexSource>,
    lease: Option<File>,
    proof: Option<worker::Proof>,
}

impl IndexJobs {
    /// Adopt verified transfer files into this source's existing native index namespace.
    pub fn install_snapshot(
        &self,
        source: &IndexSource,
        manifest: &super::snapshot::Manifest,
        files: impl FnMut(
            &super::snapshot::FileRecord,
        ) -> Result<Arc<dyn crate::pack::PackSource>, PackError>,
        admitted: impl FnMut() -> bool,
    ) -> Result<Option<Index>, IndexJobError> {
        let _owner = self.paths.worker()?;
        {
            let _gate = self.paths.gate()?;
            if self.paths.read()?.active.is_some() {
                return Err(IndexJobError::Busy);
            }
        }
        let out = self.paths.output(&source.namespace)?;
        // CURRENT is also the authority used by safe_ready; do not invent a build ticket.
        Ok(super::snapshot::install(
            &out,
            &source.zim,
            manifest,
            files,
            admitted,
        )?)
    }

    pub(crate) fn belongs_to(&self, root: &Path) -> bool {
        self.paths.belongs_to(root)
    }

    pub(crate) fn ticket_for_job(&self, id: &str, epoch: u64) -> Result<Ticket, IndexJobError> {
        if id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || epoch == 0
        {
            return Err(IndexJobError::InvalidRequest);
        }
        let _gate = self.paths.gate()?;
        let ledger = self.paths.read()?;
        let row = ledger
            .rows
            .iter()
            .find(|row| row.job_id() == id)
            .ok_or(IndexJobError::NotFound)?;
        if row.ticket.epoch != epoch {
            return Err(IndexJobError::StaleTicket);
        }
        Ok(row.ticket.clone())
    }

    pub(crate) fn source_matches(
        &self,
        ticket: &Ticket,
        source: &IndexSource,
    ) -> Result<bool, IndexJobError> {
        let _gate = self.paths.gate()?;
        let ledger = self.paths.read()?;
        let row = ledger.row(ticket)?;
        Ok(ticket.pack == source.id.as_str()
            && row.namespace == source.namespace
            && row.meta == source.meta)
    }
    pub fn open(root: &Path) -> Result<Self, IndexJobError> {
        Ok(Self {
            paths: Paths::new(root)?,
        })
    }

    fn update<T>(
        &self,
        change: impl FnOnce(&mut Ledger) -> Result<T, IndexJobError>,
    ) -> Result<T, IndexJobError> {
        let _gate = self.paths.gate()?;
        let mut ledger = self.paths.read()?;
        let before = ledger.clone();
        let result = change(&mut ledger)?;
        if ledger != before {
            self.paths.publish(&ledger)?;
        }
        Ok(result)
    }

    pub fn start(&self, source: &IndexSource) -> Result<Snapshot, IndexJobError> {
        let _owner = self.paths.worker()?;
        self.update(|ledger| {
            if ledger.active.is_some() {
                return Err(IndexJobError::Busy);
            }
            if ledger.rows.iter().any(|r| {
                r.ticket.pack == source.id.as_str()
                    && !matches!(r.state, State::Ready | State::Cancelled)
            }) {
                return Err(IndexJobError::Busy);
            }
            let out = self.paths.output(&source.namespace)?;
            std::fs::create_dir_all(&out)?;
            let before = build::capture_preparation_current(&out)?;
            let row = Row {
                ticket: Ticket {
                    pack: source.id.as_str().to_owned(),
                    epoch: ledger.next_epoch()?,
                },
                namespace: source.namespace.clone(),
                binding: source.binding.clone(),
                meta: source.meta.clone(),
                before,
                claimed: false,
                state: State::Queued,
                intent: None,
                progress: None,
                error_code: None,
            };
            ledger.rows.retain(|r| r.ticket.pack != row.ticket.pack);
            ledger.active = Some(row.ticket.pack.clone());
            let reply = row.snapshot(true);
            ledger.rows.push(row);
            Ok(reply)
        })
    }

    pub fn status(&self, id: &PackId) -> Result<Option<Snapshot>, IndexJobError> {
        let _gate = self.paths.gate()?;
        Ok(self
            .paths
            .read()?
            .rows
            .iter()
            .find(|r| r.ticket.pack == id.as_str())
            .map(|r| r.snapshot(false)))
    }

    pub fn active_ticket(&self) -> Result<Option<Ticket>, IndexJobError> {
        let _gate = self.paths.gate()?;
        let ledger = self.paths.read()?;
        Ok(ledger.active.as_ref().and_then(|id| {
            ledger
                .rows
                .iter()
                .find(|row| &row.ticket.pack == id)
                .map(|row| row.ticket.clone())
        }))
    }

    pub fn request(&self, ticket: &Ticket, action: Action) -> Result<Snapshot, IndexJobError> {
        self.update(|ledger| {
            let state = ledger.row(ticket)?.state;
            match action {
                Action::Resume if matches!(state, State::Paused | State::Failed) => {
                    if ledger.active.is_some() {
                        return Err(IndexJobError::Busy);
                    }
                    let out = self.paths.output(&ledger.row(ticket)?.namespace)?;
                    let before = build::capture_preparation_current(&out)?;
                    let epoch = ledger.next_epoch()?;
                    let row = ledger.row_mut(ticket)?;
                    row.ticket.epoch = epoch;
                    row.state = State::Queued;
                    row.intent = None;
                    row.error_code = None;
                    row.before = before;
                    let reply = row.snapshot(true);
                    ledger.active = Some(ticket.pack.clone());
                    Ok(reply)
                }
                Action::Pause if state.active() && state != State::Cancelling => {
                    let row = ledger.row_mut(ticket)?;
                    row.state = State::Pausing;
                    row.intent = Some(action);
                    Ok(row.snapshot(false))
                }
                Action::Cancel
                    if state.active() || matches!(state, State::Paused | State::Failed) =>
                {
                    if ledger.active.as_ref().is_some_and(|id| id != &ticket.pack) {
                        return Err(IndexJobError::Busy);
                    }
                    let row = ledger.row_mut(ticket)?;
                    row.state = State::Cancelling;
                    row.intent = Some(action);
                    row.error_code = None;
                    let reply = row.snapshot(!state.active());
                    ledger.active = Some(ticket.pack.clone());
                    Ok(reply)
                }
                _ => Err(IndexJobError::InvalidRequest),
            }
        })
    }

    pub fn claim(&self, ticket: &Ticket, source: &IndexSource) -> Result<Run, IndexJobError> {
        self.claim_inner(ticket, Some(source), false)
    }
    /// Acquire native ownership before reopening a possibly removable provider.
    pub fn reserve(&self, ticket: &Ticket) -> Result<Run, IndexJobError> {
        self.claim_inner(ticket, None, false)
    }
    pub fn claim_stop(&self, ticket: &Ticket) -> Result<Run, IndexJobError> {
        self.claim_inner(ticket, None, true)
    }
    fn claim_inner(
        &self,
        ticket: &Ticket,
        source: Option<&IndexSource>,
        stop_only: bool,
    ) -> Result<Run, IndexJobError> {
        let lease = self.paths.worker()?;
        self.update(|ledger| {
            let row = ledger.row_mut(ticket)?;
            if !row.state.active() {
                return Err(IndexJobError::InvalidRequest);
            }
            if row.claimed && !stop_only {
                return Err(IndexJobError::Busy);
            }
            if let Some(source) = source {
                if row.claimed {
                    return Err(IndexJobError::Busy);
                }
                if row.state == State::Cancelling {
                    return Err(IndexJobError::InvalidRequest);
                }
                if ticket.pack != source.id.as_str()
                    || row.namespace != source.namespace
                    || row.meta != source.meta
                {
                    return Err(IndexJobError::SourceChanged);
                }
            } else if stop_only && row.state != State::Cancelling {
                return Err(IndexJobError::InvalidRequest);
            }
            row.claimed = true;
            Ok(())
        })?;
        Ok(Run {
            root: self.paths.base.clone(),
            ticket: ticket.clone(),
            source: source.cloned(),
            lease: Some(lease),
            proof: None,
        })
    }

    pub fn dispatch_rejected(&self, ticket: &Ticket) -> Result<Snapshot, IndexJobError> {
        self.dispatch_failed(ticket, DispatchFailure::Rejected)?
            .ok_or(IndexJobError::InvalidRequest)
    }
    /// Rejected means native scheduling definitively accepted no matching task; timeouts are uncertain.
    pub fn dispatch_failed(
        &self,
        ticket: &Ticket,
        failure: DispatchFailure,
    ) -> Result<Option<Snapshot>, IndexJobError> {
        if failure == DispatchFailure::Uncertain {
            return Ok(None);
        }
        let _owner = self.paths.worker()?;
        self.update(|ledger| {
            let row = ledger.row_mut(ticket)?;
            if row.claimed {
                return Err(IndexJobError::Busy);
            }
            if !row.state.active() {
                return Err(IndexJobError::InvalidRequest);
            }
            if row.intent == Some(Action::Cancel) {
                return Ok(Some(row.snapshot(true)));
            }
            if row.intent == Some(Action::Pause) {
                row.state = State::Paused;
            } else {
                row.state = State::Failed;
                row.error_code = Some("dispatch_rejected".into());
            }
            let reply = row.snapshot(false);
            ledger.active = None;
            Ok(Some(reply))
        })
    }

    /// Call only after native reconciliation confirms the OS task exited; the lease also fences live work.
    pub fn reconcile_interrupted(&self, ticket: &Ticket) -> Result<Snapshot, IndexJobError> {
        let _owner = self.paths.worker()?;
        self.update(|ledger| {
            let row = ledger.row_mut(ticket)?;
            if !row.state.active() {
                return Err(IndexJobError::InvalidRequest);
            }
            let out = self.paths.output(&row.namespace)?;
            row.claimed = false;
            if build::capture_preparation_current(&out)? != row.before {
                row.state = State::Failed;
                row.intent = None;
                row.error_code = Some("publication_uncertain".into());
            } else {
                row.state = State::Paused;
                row.intent = Some(Action::Pause);
                row.error_code = None;
            }
            let reply = row.snapshot(false);
            ledger.active = None;
            Ok(reply)
        })
    }

    pub fn safe_ready(&self, source: &IndexSource) -> Result<Option<Index>, IndexJobError> {
        let out = self.paths.output(&source.namespace)?;
        match std::fs::symlink_metadata(out.join("CURRENT")) {
            Ok(_) => Ok(Some(Index::open(&out, &source.zim)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
