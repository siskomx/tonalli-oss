use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, BodyDataStream, Bytes};
use futures_util::StreamExt;
use tokio::sync::{Mutex, OwnedSemaphorePermit};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const CHUNK: usize = 64 * 1024;
const RATE: u64 = 8 * 1024 * 1024;
const NANOS: u64 = 1_000_000_000;

pub(in crate::server) struct Bucket {
    updated: Instant,
    credit: u64,
}

impl Bucket {
    pub fn new(now: Instant) -> Self {
        Self {
            updated: now,
            credit: CHUNK as u64 * NANOS,
        }
    }
    pub fn delay(&mut self, bytes: usize, now: Instant) -> Duration {
        let elapsed = now
            .saturating_duration_since(self.updated)
            .as_nanos()
            .min(u128::from(NANOS)) as u64;
        self.credit = self
            .credit
            .saturating_add(elapsed * RATE)
            .min(CHUNK as u64 * NANOS);
        self.updated = now;
        let needed = (bytes as u64).saturating_mul(NANOS);
        if self.credit >= needed {
            self.credit -= needed;
            Duration::ZERO
        } else {
            Duration::from_nanos((needed - self.credit).div_ceil(RATE))
        }
    }
}

struct Stream {
    body: BodyDataStream,
    pending: Option<Bytes>,
    cancelled: CancellationToken,
    bandwidth: Arc<Mutex<Bucket>>,
    _permit: OwnedSemaphorePermit,
    _connection: Option<Arc<super::transport::ResponseLease>>,
}

pub(in crate::server) fn wrap_selected_media(
    body: Body,
    cancelled: CancellationToken,
    bandwidth: Arc<Mutex<Bucket>>,
    permit: OwnedSemaphorePermit,
) -> Body {
    wrap(body, cancelled, bandwidth, permit, None)
}

pub(super) fn wrap(
    body: Body,
    cancelled: CancellationToken,
    bandwidth: Arc<Mutex<Bucket>>,
    permit: OwnedSemaphorePermit,
    connection: Option<Arc<super::transport::ResponseLease>>,
) -> Body {
    let state = Stream {
        body: body.into_data_stream(),
        pending: None,
        cancelled,
        bandwidth,
        _permit: permit,
        _connection: connection,
    };
    Body::from_stream(futures_util::stream::unfold(
        state,
        |mut stream| async move {
            loop {
                if stream.cancelled.is_cancelled() {
                    return None;
                }
                if stream.pending.is_some() {
                    break;
                }
                let next = tokio::select! {
                    biased;
                    _ = stream.cancelled.cancelled() => return None,
                    next = stream.body.next() => next,
                };
                match next {
                    Some(Ok(bytes)) if bytes.is_empty() => continue,
                    Some(Ok(bytes)) => stream.pending = Some(bytes),
                    Some(Err(error)) => return Some((Err(error), stream)),
                    None => return None,
                }
            }
            let mut pending = stream.pending.take()?;
            let count = pending.len().min(CHUNK);
            let chunk = pending.split_to(count);
            if !pending.is_empty() {
                stream.pending = Some(pending);
            }
            loop {
                let mut budget = tokio::select! {
                    biased;
                    _ = stream.cancelled.cancelled() => return None,
                    budget = stream.bandwidth.lock() => budget,
                };
                let wait = budget.delay(count, Instant::now());
                drop(budget);
                if wait.is_zero() {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = stream.cancelled.cancelled() => return None,
                    _ = tokio::time::sleep(wait) => {}
                }
            }
            if stream.cancelled.is_cancelled() {
                return None;
            }
            Some((Ok(chunk), stream))
        },
    ))
}
