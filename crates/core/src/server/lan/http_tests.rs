use axum::body::Body;
use futures_util::{FutureExt, StreamExt};
use std::time::Duration;

use super::*;

fn state() -> HttpState {
    let sessions = Arc::new(Mutex::new(Sessions::new(8, Instant::now()).unwrap()));
    let stop = sessions.lock().unwrap().stopped.clone();
    HttpState::new(sessions, "127.0.0.1:1234".into(), stop)
}

#[tokio::test(start_paused = true)]
async fn actual_pair_page_and_content_share_one_aggregate_payload_budget() {
    let state = state();
    let content = streams::wrap(
        Body::from(vec![0_u8; 65536]),
        state.stop.clone(),
        Arc::clone(&state.bandwidth),
        Arc::clone(&state.payloads).try_acquire_owned().unwrap(),
        None,
    );
    let mut content = content.into_data_stream();
    assert_eq!(content.next().await.unwrap().unwrap().len(), 65536);
    let paired = public_response(&state, pair_page().await).await;
    let mut paired = paired.into_body().into_data_stream();
    assert!(
        paired.next().now_or_never().is_none(),
        "pairing response bypassed the shared aggregate payload cap"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        paired.next().await.unwrap().unwrap().len(),
        include_bytes!("pair.html").len()
    );
}

#[tokio::test(start_paused = true)]
async fn public_payload_slots_are_bounded_and_owner_stop_releases_waiters() {
    let state = state();
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(public_response(&state, pair_page().await).await);
    }
    let response = public_response(&state, pair_page().await);
    tokio::pin!(response);
    assert!(
        response.as_mut().now_or_never().is_none(),
        "unpaired response work was not bounded"
    );
    state.stop.cancel();
    assert_eq!(response.await.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn public_response_admission_does_not_take_an_authenticated_payload_slot() {
    let state = state();
    let held: Vec<_> = (0..4)
        .map(|_| Arc::clone(&state.payloads).try_acquire_owned().unwrap())
        .collect();
    assert!(public_response(&state, pair_page().await)
        .now_or_never()
        .is_some());
    drop(held);
}
