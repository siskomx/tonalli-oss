use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use futures_util::{FutureExt, StreamExt};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::streams::{self, Bucket};

#[test]
fn shared_bucket_enforces_eight_mib_per_second_with_one_64kib_burst() {
    let now = Instant::now();
    let mut bucket = Bucket::new(now);
    assert_eq!(bucket.delay(32768, now), Duration::ZERO);
    assert_eq!(bucket.delay(32768, now), Duration::ZERO);
    assert_eq!(bucket.delay(65536, now), Duration::from_nanos(7_812_500));
    let later = now + Duration::from_nanos(7_812_500);
    assert_eq!(bucket.delay(65536, later), Duration::ZERO);
    assert_eq!(bucket.delay(65536, later), Duration::from_nanos(7_812_500));
}

#[tokio::test]
async fn body_is_split_into_bounded_chunks_and_revocation_drops_pending_bytes() {
    let cancellation = CancellationToken::new();
    let bandwidth = Arc::new(Mutex::new(Bucket::new(Instant::now())));
    let slots = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&slots).try_acquire_owned().unwrap();
    let body = streams::wrap(
        Body::from(vec![7_u8; 128 * 1024]),
        cancellation.clone(),
        bandwidth,
        permit,
        None,
    );
    let mut stream = body.into_data_stream();
    assert_eq!(stream.next().await.unwrap().unwrap().len(), 64 * 1024);
    cancellation.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .is_none()
    );
    drop(stream);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn two_real_bodies_share_one_aggregate_bandwidth_budget() {
    let bandwidth = Arc::new(Mutex::new(Bucket::new(Instant::now())));
    let slots = Arc::new(Semaphore::new(2));
    let first = streams::wrap(
        Body::from(vec![1_u8; 65536]),
        CancellationToken::new(),
        Arc::clone(&bandwidth),
        Arc::clone(&slots).try_acquire_owned().unwrap(),
        None,
    );
    let second = streams::wrap(
        Body::from(vec![2_u8; 65536]),
        CancellationToken::new(),
        bandwidth,
        slots.try_acquire_owned().unwrap(),
        None,
    );
    let mut first = first.into_data_stream();
    let mut second = second.into_data_stream();
    assert_eq!(first.next().await.unwrap().unwrap().len(), 65536);
    assert!(
        second.next().now_or_never().is_none(),
        "second stream got an independent initial burst"
    );
    tokio::time::advance(Duration::from_nanos(7_812_500)).await;
    assert_eq!(second.next().await.unwrap().unwrap().len(), 65536);
}
