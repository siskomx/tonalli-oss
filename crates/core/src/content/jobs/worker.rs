use super::*;
use crate::{
    content::AdmittedInstall,
    download::{self, DownloadOutcome, DownloadPhase, DownloadProgress},
};
use reqwest::blocking::Client;
use std::{
    cell::RefCell,
    ops::ControlFlow,
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) enum ExitProof {
    Installed,
    Stopped(DownloadAction),
    Failed(JobError),
    TransportFailed(PackError),
    Uncertain(PackError),
}

impl ContentJobs {
    /// Product default: normal rustls trust, HTTPS only, unchanged bounded request timeouts.
    pub fn execute(
        &self,
        run: &mut JobRun,
        progress: impl FnMut(JobSnapshot) -> Result<(), PackError>,
    ) -> Result<(), JobError> {
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .build();
        let client = match client {
            Ok(client) => client,
            Err(_) => {
                let error = JobError::Pack(PackError::Http(
                    "Content client initialization failed".into(),
                ));
                if run.proof.is_none() && run.lease.is_some() {
                    run.proof = Some(ExitProof::Failed(error.clone()));
                }
                return Err(error);
            }
        };
        self.execute_with_client(run, &client, progress)
    }

    /// Only trusted native code supplies this client, after binding its allowed network.
    /// It must preserve HTTPS certificate/hostname verification and the native network policy.
    /// JS never supplies a client, URL, destination or transport policy.
    pub fn execute_with_client(
        &self,
        run: &mut JobRun,
        client: &Client,
        mut progress: impl FnMut(JobSnapshot) -> Result<(), PackError>,
    ) -> Result<(), JobError> {
        if run.proof.is_some() {
            return Err(JobError::StaleAttempt);
        }
        let (_, ledger) = self.paths.read()?;
        self.run_row(&ledger, run)?;
        let callback_failed = std::cell::Cell::new(false);
        let mut observed_progress = |sample| {
            let result = progress(sample);
            if result.is_err() {
                callback_failed.set(true);
            }
            result
        };
        let result = self
            .execute_inner(run, client, &mut observed_progress)
            .and_then(|proof| {
                if matches!(proof, ExitProof::Stopped(DownloadAction::Pause)) {
                    self.flush_candidates(run)?;
                }
                Ok(proof)
            });
        match result {
            Ok(proof) => {
                run.proof = Some(proof);
                Ok(())
            }
            Err(error) => {
                run.proof = Some(match &error {
                    JobError::Pack(error @ PackError::PublicationUncertain(_)) => {
                        ExitProof::Uncertain(error.clone())
                    }
                    JobError::Pack(error @ PackError::Http(_)) if !callback_failed.get() => {
                        ExitProof::TransportFailed(error.clone())
                    }
                    _ => ExitProof::Failed(error.clone()),
                });
                Err(error)
            }
        }
    }

    fn execute_inner(
        &self,
        run: &JobRun,
        client: &Client,
        progress: &mut impl FnMut(JobSnapshot) -> Result<(), PackError>,
    ) -> Result<ExitProof, JobError> {
        match self.requested(run)? {
            DownloadAction::Pause => return Ok(ExitProof::Stopped(DownloadAction::Pause)),
            DownloadAction::Cancel => return Ok(ExitProof::Stopped(DownloadAction::Cancel)),
            DownloadAction::Continue => {}
        }
        let entry = run
            .binding
            .entries()
            .iter()
            .find(|entry| entry.id == run.spec.id)
            .ok_or_else(invalid)?;
        if self
            .store
            .version_path(entry)?
            .try_exists()
            .map_err(PackError::from)?
        {
            // A precommit failure may already have moved the verified cache to its immutable
            // version. Reuse the Store's same validation/admission path, without a new fetch.
            progress(self.checkpoint(
                run,
                DownloadProgress {
                    phase: DownloadPhase::Verifying,
                    downloaded_bytes: run.spec.bytes,
                    total_bytes: run.spec.bytes,
                    verified_bytes: 0,
                },
            )?)?;
            return self.install_run(run);
        }
        // Each run retains its authenticated catalog/spec. Poll only bounded latest policy;
        // do not rehash every historical signed catalog for every 64KiB checksum chunk.
        let control = RefCell::new((None::<Instant>, DownloadAction::Continue, None::<JobError>));
        let mut emitted = None::<(Instant, DownloadPhase)>;
        let fetched = download::fetch_pack_controlled_with_client(
            &run.spec,
            self.store.root(),
            client,
            || {
                let mut state = control.borrow_mut();
                if state
                    .0
                    .is_none_or(|last| last.elapsed() >= Duration::from_millis(100))
                {
                    match self.requested(run) {
                        Ok(action) => state.1 = action,
                        Err(error) => {
                            state.1 = DownloadAction::Cancel;
                            state.2 = Some(error);
                        }
                    }
                    state.0 = Some(Instant::now());
                }
                state.1
            },
            |sample| {
                if emitted.is_some_and(|(last, phase)| {
                    phase == sample.phase && last.elapsed() < Duration::from_millis(100)
                }) && sample.verified_bytes != sample.total_bytes
                {
                    return Ok(());
                }
                let snapshot = self.checkpoint(run, sample).map_err(as_pack)?;
                emitted = Some((Instant::now(), sample.phase));
                progress(snapshot)
            },
        )?;
        if let Some(error) = control.into_inner().2 {
            return Err(error);
        }
        match fetched {
            DownloadOutcome::Paused => return Ok(ExitProof::Stopped(DownloadAction::Pause)),
            DownloadOutcome::Cancelled => return Ok(ExitProof::Stopped(DownloadAction::Cancel)),
            DownloadOutcome::Complete(_) => {}
        }
        self.install_run(run)
    }

    fn install_run(&self, run: &JobRun) -> Result<ExitProof, JobError> {
        match self
            .store
            .install_cached_admitted(&run.binding, &run.spec.id, || {
                self.admit(run).map_err(as_pack)
            })? {
            AdmittedInstall::Installed(installed) => {
                if installed.sha256 != run.spec.sha256 {
                    return Err(invalid());
                }
                Ok(ExitProof::Installed)
            }
            AdmittedInstall::Stopped(action) => Ok(ExitProof::Stopped(action)),
        }
    }

    fn checkpoint(&self, run: &JobRun, sample: DownloadProgress) -> Result<JobSnapshot, JobError> {
        let _gate = self.paths.lock()?;
        let (_, mut ledger) = self.paths.read()?;
        self.run_row(&ledger, run)?;
        if sample.total_bytes != run.spec.bytes
            || sample.downloaded_bytes > sample.total_bytes
            || sample.verified_bytes > sample.total_bytes
        {
            return Err(invalid());
        }
        let row = ledger
            .jobs
            .iter_mut()
            .find(|r| r.id == run.ticket.id)
            .ok_or(JobError::NotFound)?;
        if row.intent.is_none() {
            row.state = match sample.phase {
                DownloadPhase::Downloading => JobState::Downloading,
                DownloadPhase::Verifying => JobState::Verifying,
            };
        }
        row.downloaded_bytes = sample.downloaded_bytes;
        row.verified_bytes = sample.verified_bytes;
        let snapshot = snapshot(row, run.spec.bytes);
        self.paths.publish(&ledger)?;
        Ok(snapshot)
    }

    // Called with StoreMeta held. Retain Jobs through the store's final publication.
    fn admit(&self, run: &JobRun) -> Result<ControlFlow<DownloadAction, File>, JobError> {
        for _ in 0..8 {
            let verified = self.read_verified()?; // Fresh signed authority, outside Jobs.
            let gate = self.paths.lock()?;
            let (raw, mut ledger) = self.paths.read()?;
            if raw != verified.raw {
                continue;
            }
            let row = self.run_row(&ledger, run)?;
            if row.authority_fingerprint
                != managed::fingerprint(&run.binding.snapshot_for_key(&self.store.pin)?)
            {
                return Err(invalid());
            }
            if let Some(action @ (JobAction::Pause | JobAction::Cancel)) = row.intent {
                return Ok(ControlFlow::Break(if action == JobAction::Pause {
                    DownloadAction::Pause
                } else {
                    DownloadAction::Cancel
                }));
            }
            let row = ledger
                .jobs
                .iter_mut()
                .find(|r| r.id == run.ticket.id)
                .ok_or(JobError::NotFound)?;
            row.state = JobState::Installing;
            self.paths.publish(&ledger)?;
            return Ok(ControlFlow::Continue(gate));
        }
        Err(JobError::Busy)
    }

    pub(super) fn finish_execution(&self, run: &mut JobRun) -> Result<JobSnapshot, JobError> {
        let mut proof = run.proof.clone().ok_or(JobError::InvalidRequest)?;
        if let ExitProof::Uncertain(error) = &proof {
            // Store reconciliation precedes Jobs locking; repeat the store durability barriers.
            match self.store.confirm_published(&run.binding, &run.spec.id)? {
                Some(_) => proof = ExitProof::Installed,
                None => proof = ExitProof::Failed(JobError::Pack(error.clone())),
            }
        }
        let finalize = |candidate: Option<&std::path::Path>| {
            self.update(|ledger, verified| {
                let latest = self.run_row(ledger, run)?;
                let terminal = if let ExitProof::TransportFailed(error) = &proof {
                    match latest.intent {
                        Some(JobAction::Cancel) => ExitProof::Stopped(DownloadAction::Cancel),
                        Some(JobAction::Pause) => match self.flush_candidates(run) {
                            Ok(()) => ExitProof::Stopped(DownloadAction::Pause),
                            Err(error) => ExitProof::Failed(error),
                        },
                        _ => ExitProof::Failed(JobError::Pack(error.clone())),
                    }
                } else {
                    proof.clone()
                };
                let cancelled = matches!(terminal, ExitProof::Stopped(DownloadAction::Cancel))
                    || matches!(terminal, ExitProof::Stopped(_))
                        && latest.intent == Some(JobAction::Cancel);
                // The worker has exited; StoreMeta -> Jobs fences the latest stop intent,
                // exact candidate cleanup and terminal publication together.
                if cancelled {
                    self.cleanup(run, candidate)?;
                }
                let row = ledger
                    .jobs
                    .iter_mut()
                    .find(|r| r.id == run.ticket.id)
                    .ok_or(JobError::NotFound)?;
                row.state = match &terminal {
                    ExitProof::Installed => JobState::Completed,
                    ExitProof::Stopped(_) if cancelled => JobState::Cancelled,
                    ExitProof::Stopped(DownloadAction::Pause) => JobState::Paused,
                    ExitProof::Failed(_) => JobState::Failed,
                    _ => return Err(JobError::InvalidRequest),
                };
                if row.state == JobState::Completed {
                    row.downloaded_bytes = run.spec.bytes;
                    row.verified_bytes = run.spec.bytes;
                }
                row.intent = match row.state {
                    JobState::Cancelled => Some(JobAction::Cancel),
                    JobState::Paused => Some(JobAction::Pause),
                    _ => None,
                };
                row.error_code = match &terminal {
                    ExitProof::Failed(error) => Some(error_code(error).into()),
                    _ => None,
                };
                row.claimed = false;
                ledger.active = None;
                verified.snapshot(row)
            })
        };
        let snapshot = if matches!(proof, ExitProof::Stopped(_) | ExitProof::TransportFailed(_)) {
            self.store
                .with_unpublished_candidate(&run.binding, &run.spec.id, finalize)?
        } else {
            finalize(None)?
        };
        run.lease.take();
        Ok(snapshot)
    }

    fn cleanup(&self, run: &JobRun, candidate: Option<&std::path::Path>) -> Result<(), JobError> {
        for relative in [
            run.spec.path.clone(),
            format!("{}.part", run.spec.path),
            format!("{}.part.meta", run.spec.path),
        ] {
            let path = pack_path(self.store.root(), &relative)?;
            remove_stage_file(&path)?;
        }
        if let Some(path) = candidate {
            remove_stage_file(path)?;
        }
        Ok(())
    }

    pub(super) fn flush_candidates(&self, run: &JobRun) -> Result<(), JobError> {
        self.flush_spec(&run.spec)
    }

    pub(super) fn flush_spec(&self, spec: &download::DownloadSpec) -> Result<(), JobError> {
        for relative in [
            spec.path.clone(),
            format!("{}.part", spec.path),
            format!("{}.part.meta", spec.path),
        ] {
            let path = pack_path(self.store.root(), &relative)?;
            download::ensure_regular_or_absent(&path)?;
            match fs::OpenOptions::new().read(true).write(true).open(path) {
                Ok(file) => file.sync_all().map_err(PackError::from)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(PackError::from(error).into()),
            }
        }
        Ok(())
    }
}

fn remove_stage_file(path: &std::path::Path) -> Result<(), JobError> {
    download::ensure_regular_or_absent(path)?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PackError::from(error).into()),
    }
    #[cfg(unix)]
    File::open(managed::parent(path)?)
        .map_err(PackError::from)?
        .sync_all()
        .map_err(PackError::from)?;
    Ok(())
}

fn snapshot(row: &Row, total: u64) -> JobSnapshot {
    JobSnapshot {
        id: row.id.clone(),
        entry_id: row.entry_id.clone(),
        state: row.state,
        downloaded_bytes: row.downloaded_bytes,
        total_bytes: total,
        verified_bytes: row.verified_bytes,
        error_code: row.error_code.clone(),
    }
}

fn as_pack(error: JobError) -> PackError {
    match error {
        JobError::Pack(error) => error,
        _ => PackError::Corrupt("Content worker policy changed".into()),
    }
}

fn error_code(error: &JobError) -> &'static str {
    match error {
        JobError::Pack(PackError::NoSpace) => "no_space",
        JobError::Pack(PackError::PublicationUncertain(_)) => "publication_uncertain",
        JobError::Pack(
            PackError::Checksum
            | PackError::Corrupt(_)
            | PackError::Truncated
            | PackError::Mismatch(_),
        ) => "invalid_content",
        JobError::Pack(PackError::Http(_)) => "network",
        JobError::Pack(PackError::MediaRemoved) => "media_removed",
        _ => "io",
    }
}
