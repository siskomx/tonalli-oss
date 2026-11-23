use super::*;

#[derive(Clone)]
pub(super) enum Proof {
    Ready,
    Paused,
    Cancelled,
    Failed(IndexJobError),
}

impl IndexJobs {
    fn unbound_row<'a>(&self, ledger: &'a Ledger, run: &Run) -> Result<&'a Row, IndexJobError> {
        if run.source.is_some() || run.proof.is_some() {
            return Err(IndexJobError::InvalidRequest);
        }
        self.run_row(ledger, run)
    }

    pub fn bind_source(&self, run: &mut Run, source: &IndexSource) -> Result<(), IndexJobError> {
        let _gate = self.paths.gate()?;
        let ledger = self.paths.read()?;
        let row = self.unbound_row(&ledger, run)?;
        if run.ticket.pack != source.id.as_str()
            || row.namespace != source.namespace
            || row.meta != source.meta
        {
            run.proof = Some(Proof::Failed(IndexJobError::SourceChanged));
            return Err(IndexJobError::SourceChanged);
        }
        run.source = Some(source.clone());
        Ok(())
    }

    /// Native provider-open failures cannot manufacture a successful stop.
    pub fn fail_unbound(&self, run: &mut Run, error: IndexJobError) -> Result<(), IndexJobError> {
        let _gate = self.paths.gate()?;
        self.unbound_row(&self.paths.read()?, run)?;
        run.proof = Some(Proof::Failed(error));
        Ok(())
    }

    pub fn execute_unbound_control(&self, run: &mut Run) -> Result<bool, IndexJobError> {
        let intent = {
            let _gate = self.paths.gate()?;
            let ledger = self.paths.read()?;
            self.unbound_row(&ledger, run)?.intent
        };
        match intent {
            Some(Action::Pause) => {
                run.proof = Some(Proof::Paused);
                Ok(true)
            }
            Some(Action::Cancel) => {
                self.execute_stop(run)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn run_row<'a>(&self, ledger: &'a Ledger, run: &Run) -> Result<&'a Row, IndexJobError> {
        if run.root != self.paths.base || run.lease.is_none() {
            return Err(IndexJobError::InvalidRequest);
        }
        let row = ledger.row(&run.ticket)?;
        if !row.claimed || !row.state.active() {
            return Err(IndexJobError::InvalidRequest);
        }
        Ok(row)
    }

    pub fn execute(
        &self,
        run: &mut Run,
        threads: usize,
        mut progress: impl FnMut(BuildProgress) -> Result<BuildControl, PackError>,
    ) -> Result<(), IndexJobError> {
        if run.proof.is_some() {
            return Err(IndexJobError::InvalidRequest);
        }
        let ledger = {
            let _gate = self.paths.gate()?;
            self.paths.read()?
        };
        let row = self.run_row(&ledger, run)?;
        let out = self.paths.output(&row.namespace)?;
        let source = run.source.as_ref().ok_or(IndexJobError::InvalidRequest)?;
        let result = build::build_controlled(&source.zim, &out, threads, |sample| {
            let requested = progress(sample)?;
            self.update(|ledger| {
                self.run_row(ledger, run)?;
                let row = ledger.row_mut(&run.ticket)?;
                row.progress = Some(sample);
                let control = if row.intent == Some(Action::Cancel)
                    || requested == BuildControl::Cancel
                {
                    BuildControl::Cancel
                } else if row.intent == Some(Action::Pause) || requested == BuildControl::Pause {
                    BuildControl::Pause
                } else {
                    BuildControl::Continue
                };
                match control {
                    BuildControl::Cancel => {
                        row.intent = Some(Action::Cancel);
                        row.state = State::Cancelling;
                    }
                    BuildControl::Pause => {
                        row.intent = Some(Action::Pause);
                        row.state = State::Pausing;
                    }
                    BuildControl::Continue => {
                        row.state = match sample.phase {
                            BuildPhase::Checking => State::Checking,
                            BuildPhase::Indexing => State::Indexing,
                            BuildPhase::Validating => State::Validating,
                            BuildPhase::Activating => State::Activating,
                        };
                    }
                }
                Ok(control)
            })
            .map_err(as_pack)
        });
        let proof = match result {
            Ok(BuildOutcome::Ready(_)) => Proof::Ready,
            Ok(BuildOutcome::Cancelled) => Proof::Cancelled,
            Ok(BuildOutcome::Paused(sample)) => {
                if let Err(error) = self.update(|ledger| {
                    ledger.row_mut(&run.ticket)?.progress = Some(sample);
                    Ok(())
                }) {
                    run.proof = Some(Proof::Failed(error.clone()));
                    return Err(error);
                }
                Proof::Paused
            }
            Err(error) => {
                let error = IndexJobError::Pack(error);
                run.proof = Some(Proof::Failed(error.clone()));
                return Err(error);
            }
        };
        run.proof = Some(proof);
        Ok(())
    }

    pub fn execute_stop(&self, run: &mut Run) -> Result<(), IndexJobError> {
        if run.proof.is_some() || run.source.is_some() {
            return Err(IndexJobError::InvalidRequest);
        }
        let ledger = {
            let _gate = self.paths.gate()?;
            self.paths.read()?
        };
        let row = self.run_row(&ledger, run)?;
        if row.state != State::Cancelling {
            return Err(IndexJobError::InvalidRequest);
        }
        let out = self.paths.output(&row.namespace)?;
        match build::cancel_owned_preparation(&out, &row.meta, &row.before) {
            Ok(()) => {
                run.proof = Some(Proof::Cancelled);
                Ok(())
            }
            Err(error) => {
                let error = IndexJobError::Pack(error);
                run.proof = Some(Proof::Failed(error.clone()));
                Err(error)
            }
        }
    }

    pub fn finish_exited(&self, run: &mut Run) -> Result<Snapshot, IndexJobError> {
        let proof = run
            .proof
            .as_ref()
            .ok_or(IndexJobError::InvalidRequest)?
            .clone();
        let reply = self.update(|ledger| {
            self.run_row(ledger, run)?;
            let row = ledger.row_mut(&run.ticket)?;
            let pending_cancel =
                matches!(proof, Proof::Paused) && row.intent == Some(Action::Cancel);
            row.state = match &proof {
                Proof::Ready => State::Ready,
                Proof::Paused if pending_cancel => State::Cancelling,
                Proof::Paused => State::Paused,
                Proof::Cancelled => State::Cancelled,
                Proof::Failed(_) => State::Failed,
            };
            row.intent = match row.state {
                State::Paused => Some(Action::Pause),
                State::Cancelled | State::Cancelling => Some(Action::Cancel),
                _ => None,
            };
            row.error_code = if let Proof::Failed(error) = &proof {
                Some(error_code(error).into())
            } else {
                None
            };
            row.claimed = false;
            let reply = row.snapshot(pending_cancel);
            if !pending_cancel {
                ledger.active = None;
            }
            Ok(reply)
        })?;
        run.proof = None;
        run.source = None;
        run.lease = None;
        Ok(reply)
    }
}

fn as_pack(error: IndexJobError) -> PackError {
    match error {
        IndexJobError::Pack(error) => error,
        _ => PackError::Mismatch("index preparation ownership changed".into()),
    }
}
fn error_code(error: &IndexJobError) -> &'static str {
    match error {
        IndexJobError::Pack(PackError::PublicationUncertain(_)) => "publication_uncertain",
        IndexJobError::Pack(PackError::MediaRemoved) => "media_removed",
        IndexJobError::Pack(PackError::NoSpace) => "no_space",
        _ => "preparation_failed",
    }
}
