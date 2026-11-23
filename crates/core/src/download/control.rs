use std::{ops::ControlFlow, path::PathBuf};

use crate::pack::PackError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadAction {
    Continue,
    Pause,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    Downloading,
    Verifying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    /// Bytes present in the current candidate, including a resumed prefix; not a completion claim.
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Bytes processed by this checksum pass, separate from staged/downloaded bytes.
    pub verified_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadOutcome {
    Complete(PathBuf),
    Paused,
    Cancelled,
}

pub(super) enum Stop {
    Pause,
    Cancel,
    Failed(PackError),
}

pub(super) fn requested(action: &impl Fn() -> DownloadAction) -> Option<Stop> {
    match action() {
        DownloadAction::Continue => None,
        DownloadAction::Pause => Some(Stop::Pause),
        DownloadAction::Cancel => Some(Stop::Cancel),
    }
}

pub(super) fn checkpoint(
    action: &impl Fn() -> DownloadAction,
    progress: &mut impl FnMut(DownloadProgress) -> Result<(), PackError>,
    value: DownloadProgress,
) -> ControlFlow<Stop> {
    if let Some(stop) = requested(action) {
        return ControlFlow::Break(stop);
    }
    if let Err(error) = progress(value) {
        return ControlFlow::Break(Stop::Failed(error));
    }
    match requested(action) {
        Some(stop) => ControlFlow::Break(stop),
        None => ControlFlow::Continue(()),
    }
}

pub(super) fn finish_stop(
    stop: Stop,
    sync_partial: impl FnOnce() -> Result<(), PackError>,
) -> Result<DownloadOutcome, PackError> {
    match stop {
        Stop::Failed(error) => Err(error),
        Stop::Pause => {
            sync_partial()?;
            Ok(DownloadOutcome::Paused)
        }
        Stop::Cancel => {
            sync_partial()?;
            Ok(DownloadOutcome::Cancelled)
        }
    }
}
