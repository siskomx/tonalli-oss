use serde::Serialize;
use tonalli_core::ai::{GenerationEnd, GenerationEvent, GenerationStats};
use tonalli_core::pack::PackError;

pub(super) fn calibration_next(
    repeats: usize,
    observed_input_tokens: u32,
) -> Result<Option<usize>, PackError> {
    let invalid = || PackError::Mismatch("calibration exceeds bounded prompt/token budget".into());
    if !(1..=2000).contains(&repeats) || !(1..=2048).contains(&observed_input_tokens) {
        return Err(invalid());
    }
    if observed_input_tokens == 2000 {
        return Ok(None);
    }
    // This proposes the next probe; only its actual rendered token count can accept it.
    let next = repeats
        .checked_add(2000)
        .and_then(|value| value.checked_sub(observed_input_tokens as usize))
        .filter(|value| (1..=2000).contains(value))
        .ok_or_else(invalid)?;
    Ok(Some(next))
}

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub id: u64,
    pub start_us: u64,
    pub end_us: u64,
    pub first_receipt_us: Option<u64>,
    pub chunks: u64,
    pub bytes: u64,
    pub reason: GenerationEnd,
    pub stats: GenerationStats,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct Collector {
    pub request: Option<Request>,
    id: Option<u64>,
    first_receipt_us: Option<u64>,
    chunks: u64,
    bytes: u64,
    last_elapsed_us: u64,
}

impl Collector {
    pub fn observe(&mut self, event: GenerationEvent, elapsed_us: u64) -> Result<(), PackError> {
        if self.request.is_some() || elapsed_us < self.last_elapsed_us {
            return Err(invalid());
        }
        match event {
            GenerationEvent::Started { id } => {
                if id == 0 || self.id.is_some() {
                    return Err(invalid());
                }
                self.id = Some(id);
            }
            GenerationEvent::Text { id, text } => {
                if self.id != Some(id) || text.is_empty() || text.len() > 1024 {
                    return Err(invalid());
                }
                let bytes = self
                    .bytes
                    .checked_add(text.len() as u64)
                    .filter(|bytes| *bytes <= 128 * 1024)
                    .ok_or_else(invalid)?;
                if self.chunks >= 4096 {
                    return Err(invalid());
                }
                self.bytes = bytes;
                self.chunks += 1;
                self.first_receipt_us.get_or_insert(elapsed_us);
            }
            GenerationEvent::Finished {
                id,
                reason,
                stats,
                error,
            } => {
                if self.id != Some(id)
                    || stats.input_tokens > 2048
                    || (stats.input_tokens == 0
                        && matches!(reason, GenerationEnd::Eos | GenerationEnd::Limit))
                    || stats.output_tokens > 128
                    || stats
                        .first_token_us
                        .is_some_and(|first| first > stats.total_us)
                    || (self.chunks > 0
                        && (stats.first_token_us.is_none() || stats.output_tokens == 0))
                {
                    return Err(invalid());
                }
                self.request = Some(Request {
                    id,
                    start_us: 0,
                    end_us: elapsed_us,
                    first_receipt_us: self.first_receipt_us,
                    chunks: self.chunks,
                    bytes: self.bytes,
                    reason,
                    stats,
                    error,
                });
            }
        }
        self.last_elapsed_us = elapsed_us;
        Ok(())
    }
}

fn invalid() -> PackError {
    PackError::Corrupt("invalid AI benchmark callback lifecycle or stats".into())
}

impl Request {
    pub fn wall_us(&self) -> Option<u64> {
        self.end_us
            .checked_sub(self.start_us)
            .filter(|wall| *wall > 0)
    }
    pub fn wall_tokens_s(&self) -> Option<f64> {
        self.wall_us()
            .map(|wall| f64::from(self.stats.output_tokens) * 1_000_000.0 / wall as f64)
    }
    fn qualified(&self) -> bool {
        self.id > 0
            && self.stats.input_tokens == 2000
            && (1..=128).contains(&self.stats.output_tokens)
            && matches!(self.reason, GenerationEnd::Eos | GenerationEnd::Limit)
            && self.error.is_none()
            && self.chunks > 0
            && self.chunks <= 4096
            && (1..=128 * 1024).contains(&self.bytes)
            && self.wall_us().is_some_and(|wall| {
                self.stats.total_us <= wall
                    && self
                        .first_receipt_us
                        .is_some_and(|first| first < 10_000_000 && first <= wall)
            })
            && self
                .stats
                .first_token_us
                .is_some_and(|first| first <= self.stats.total_us)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub scheduled_us: u64,
    pub observed_us: u64,
    pub latest_request_id: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(super) struct Assessment {
    pub eligible: bool,
    pub passed: bool,
    pub first_minute_weighted_wall_tokens_s: Option<f64>,
    pub reasons: Vec<String>,
}

pub(super) fn assess(
    requests: &[Request],
    snapshots: &[Snapshot],
    elapsed_us: u64,
    smoke: bool,
) -> Assessment {
    let mut reasons = Vec::new();
    let mut add = |reason: &str| {
        if !reasons.iter().any(|item| item == reason) {
            reasons.push(reason.to_owned());
        }
    };
    if smoke {
        add("short smoke is ineligible for the ten-minute assessment");
    }
    let total_wall: Option<u64> = requests
        .iter()
        .try_fold(0_u64, |sum, request| sum.checked_add(request.wall_us()?));
    if elapsed_us < 600_000_000
        || total_wall.is_none_or(|wall| wall < 600_000_000)
        || requests.first().is_none_or(|request| request.start_us != 0)
        || requests
            .last()
            .is_none_or(|request| request.end_us > elapsed_us)
    {
        add("inadequate complete ten-minute request coverage");
    }
    if requests.is_empty()
        || requests.len() > 4096
        || requests.iter().any(|request| !request.qualified())
    {
        add("request failed, missed first sink receipt target, or has unqualified stats");
    }
    let mut ids = std::collections::HashSet::new();
    if requests.iter().any(|request| !ids.insert(request.id))
        || requests
            .windows(2)
            .any(|pair| pair[1].start_us < pair[0].end_us)
    {
        add("invalid request ordering or duplicate IDs");
    }
    let baseline_jobs: Vec<_> = requests
        .iter()
        .filter(|request| request.end_us <= 60_000_000 && request.qualified())
        .collect();
    let first_minute_weighted_wall_tokens_s = if baseline_jobs.is_empty() {
        None
    } else {
        baseline_jobs
            .iter()
            .try_fold((0_u64, 0_u64), |(tokens, wall), request| {
                Some((
                    tokens.checked_add(u64::from(request.stats.output_tokens))?,
                    wall.checked_add(request.wall_us()?)?,
                ))
            })
            .filter(|(_, wall)| *wall > 0)
            .map(|(tokens, wall)| tokens as f64 * 1_000_000.0 / wall as f64)
    };
    if first_minute_weighted_wall_tokens_s.is_none() {
        add("no qualified first-minute completed request baseline");
    }
    if requests
        .iter()
        .any(|request| request.wall_tokens_s().is_none_or(|tps| tps < 5.0))
    {
        add("request wall throughput below five tokens per second");
    }
    if let Some(baseline) = first_minute_weighted_wall_tokens_s {
        if requests
            .iter()
            .filter(|request| request.end_us > 60_000_000)
            .any(|request| {
                request
                    .wall_tokens_s()
                    .is_none_or(|tps| tps < baseline / 2.0)
            })
        {
            add("sustained request wall throughput below half first-minute baseline");
        }
    }
    let expected = elapsed_us / 30_000_000;
    if snapshots.len() as u64 != expected
        || snapshots.iter().enumerate().any(|(index, snapshot)| {
            let scheduled = (index as u64 + 1) * 30_000_000;
            snapshot.scheduled_us != scheduled
                || snapshot.observed_us < scheduled
                || snapshot.observed_us >= scheduled + 30_000_000
                || snapshot.observed_us > elapsed_us
                || snapshot.latest_request_id
                    != requests
                        .iter()
                        .rev()
                        .find(|request| request.end_us <= snapshot.observed_us)
                        .map(|request| request.id)
        })
    {
        add("inadequate thirty-second snapshot coverage");
    }
    Assessment {
        eligible: !smoke,
        passed: reasons.is_empty(),
        first_minute_weighted_wall_tokens_s,
        reasons,
    }
}
