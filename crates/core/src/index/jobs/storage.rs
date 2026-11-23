use super::*;
use crate::content::storage as managed;
use fs2::FileExt;
use std::{
    fs::{self, OpenOptions},
    io::Read,
};

const MAX_BYTES: u64 = 512 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub ticket: Ticket,
    pub namespace: String,
    pub binding: String,
    pub meta: super::super::IndexMeta,
    pub before: Option<PathBuf>,
    pub claimed: bool,
    pub state: State,
    pub intent: Option<Action>,
    pub progress: Option<BuildProgress>,
    pub error_code: Option<String>,
}

impl Row {
    pub fn job_id(&self) -> String {
        self.namespace.chars().take(32).collect()
    }
    pub fn snapshot(&self, dispatch: bool) -> Snapshot {
        Snapshot {
            job_id: self.job_id(),
            ticket: self.ticket.clone(),
            state: self.state,
            progress: self.progress,
            error_code: self.error_code.clone(),
            dispatch,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    pub version: u32,
    pub epoch: u64,
    pub rows: Vec<Row>,
    pub active: Option<String>,
}

impl Ledger {
    fn validate(&self) -> Result<(), IndexJobError> {
        if self.version != 1 || self.rows.len() > 64 {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        let mut active = None;
        for row in &self.rows {
            if row.namespace != super::source::namespace(&row.ticket.pack, &row.meta, &row.binding)?
            {
                return Err(invalid());
            }
            if PackId::new(&row.ticket.pack).is_err()
                || !ids.insert(&row.ticket.pack)
                || row.ticket.epoch == 0
                || row.ticket.epoch > self.epoch
                || row.namespace.len() != 64
                || !row
                    .namespace
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || row
                    .progress
                    .is_some_and(|p| p.committed_articles > p.processed_articles)
            {
                return Err(invalid());
            }
            if row.state.active() {
                if active.replace(row.ticket.pack.as_str()).is_some() {
                    return Err(invalid());
                }
            } else if row.claimed {
                return Err(invalid());
            }
            let valid = match row.state {
                State::Pausing | State::Paused => row.intent == Some(Action::Pause),
                State::Cancelling | State::Cancelled => row.intent == Some(Action::Cancel),
                _ => row.intent.is_none(),
            };
            if !valid
                || (row.state == State::Failed) != row.error_code.is_some()
                || row.error_code.as_ref().is_some_and(|s| {
                    !matches!(
                        s.as_str(),
                        "dispatch_rejected"
                            | "preparation_failed"
                            | "publication_uncertain"
                            | "media_removed"
                            | "no_space"
                    )
                })
            {
                return Err(invalid());
            }
        }
        if active != self.active.as_deref() {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn row(&self, ticket: &Ticket) -> Result<&Row, IndexJobError> {
        let row = self
            .rows
            .iter()
            .find(|r| r.ticket.pack == ticket.pack)
            .ok_or(IndexJobError::NotFound)?;
        if row.ticket != *ticket {
            return Err(IndexJobError::StaleTicket);
        }
        Ok(row)
    }

    pub fn row_mut(&mut self, ticket: &Ticket) -> Result<&mut Row, IndexJobError> {
        self.row(ticket)?;
        self.rows
            .iter_mut()
            .find(|r| r.ticket == *ticket)
            .ok_or(IndexJobError::NotFound)
    }

    pub fn next_epoch(&mut self) -> Result<u64, IndexJobError> {
        self.epoch = self.epoch.checked_add(1).ok_or_else(invalid)?;
        Ok(self.epoch)
    }
}

pub(super) struct Paths {
    root: PathBuf,
    pub base: PathBuf,
}

impl Paths {
    pub fn belongs_to(&self, root: &Path) -> bool {
        root.canonicalize().is_ok_and(|root| root == self.root)
    }
    pub fn new(root: &Path) -> Result<Self, IndexJobError> {
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let gate_path = managed::path(&root, "packs/index-jobs/jobs.lock")?;
        let base = gate_path.parent().ok_or_else(invalid)?.to_path_buf();
        fs::create_dir_all(&base)?;
        let paths = Self { root, base };
        let gate_path = paths.path("jobs.lock")?;
        let (gate, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&gate_path)
        {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (open(&gate_path)?, false),
            Err(e) => return Err(e.into()),
        };
        FileExt::lock_exclusive(&gate)?;
        if created {
            if paths.path("jobs.json")?.try_exists()? || paths.path("worker.lock")?.try_exists()? {
                return Err(invalid());
            }
            managed::flush(&gate, &paths.base)?;
            let worker = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(paths.path("worker.lock")?)?;
            managed::flush(&worker, &paths.base)?;
            paths.publish(&Ledger {
                version: 1,
                ..Ledger::default()
            })?;
        }
        open(&paths.path("worker.lock")?)?;
        paths.read()?;
        Ok(paths)
    }

    fn path(&self, name: &str) -> Result<PathBuf, IndexJobError> {
        Ok(managed::path(
            &self.root,
            &format!("packs/index-jobs/{name}"),
        )?)
    }

    pub fn output(&self, namespace: &str) -> Result<PathBuf, IndexJobError> {
        if namespace.len() != 64
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        Ok(self
            .path(&format!("generations/{namespace}/CURRENT"))?
            .parent()
            .ok_or_else(invalid)?
            .to_path_buf())
    }

    pub fn gate(&self) -> Result<File, IndexJobError> {
        let file = open(&self.path("jobs.lock")?)?;
        FileExt::lock_exclusive(&file)?;
        Ok(file)
    }

    pub fn worker(&self) -> Result<File, IndexJobError> {
        let file = open(&self.path("worker.lock")?)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(file),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                Err(IndexJobError::Busy)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn read(&self) -> Result<Ledger, IndexJobError> {
        open(&self.path("jobs.lock")?)?;
        open(&self.path("worker.lock")?)?;
        let mut bytes = Vec::new();
        open(&self.path("jobs.json")?)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        ledger.validate()?;
        Ok(ledger)
    }

    pub fn publish(&self, ledger: &Ledger) -> Result<(), IndexJobError> {
        ledger.validate()?;
        let bytes = serde_json::to_vec(ledger).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        Ok(managed::publish(&self.path("jobs.json")?, &bytes)?)
    }
}

fn open(path: &Path) -> Result<File, IndexJobError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(invalid());
    }
    Ok(OpenOptions::new().read(true).write(true).open(path)?)
}
