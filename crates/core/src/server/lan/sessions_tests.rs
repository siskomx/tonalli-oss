use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::http::StatusCode;

use super::sessions::Sessions;

fn peer(last: u8) -> IpAddr {
    IpAddr::from([192, 168, 1, last])
}

#[test]
fn expired_pairing_code_cannot_pair_until_owner_requests_a_fresh_code() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    let old_code = sessions.code(now).unwrap();
    let later = now + Duration::from_secs(300);
    assert_eq!(
        sessions.pair(peer(1), &old_code, "laptop", later).err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    let current = sessions.code(later).unwrap();
    assert_eq!(current.len(), 6);
    assert!(current.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(sessions.pair(peer(1), &current, "laptop", later).is_ok());
}

#[test]
fn five_failures_lock_only_that_peer_for_five_minutes() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    let code = sessions.code(now).unwrap();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        assert!(sessions.pair(peer(1), wrong, "wrong", now).is_err());
    }
    assert_eq!(
        sessions
            .pair(peer(1), &code, "locked", now + Duration::from_secs(299))
            .err(),
        Some(StatusCode::TOO_MANY_REQUESTS)
    );
    assert!(sessions.pair(peer(2), &code, "another peer", now).is_ok());
    let later = now + Duration::from_secs(300);
    let fresh = sessions.code(later).unwrap();
    assert!(sessions
        .pair(peer(1), &fresh, "unlocked peer", later)
        .is_ok());
}

#[test]
fn global_bad_attempt_budget_cannot_be_bypassed_with_many_actual_peers() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    let code = sessions.code(now).unwrap();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for last in 1..=5 {
        for _ in 0..4 {
            assert!(sessions.pair(peer(last), wrong, "wrong", now).is_err());
        }
    }
    assert_eq!(
        sessions.pair(peer(6), &code, "blocked globally", now).err(),
        Some(StatusCode::TOO_MANY_REQUESTS)
    );
    assert!(sessions
        .pair(peer(6), &code, "next minute", now + Duration::from_secs(60))
        .is_ok());
}

#[test]
fn request_bucket_has_burst_64_and_refills_32_per_second_without_sleeping() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    for _ in 0..64 {
        assert!(sessions.admit(peer(1), now).is_ok());
    }
    assert_eq!(
        sessions.admit(peer(1), now),
        Err(StatusCode::TOO_MANY_REQUESTS)
    );
    assert!(sessions.admit(peer(2), now).is_ok());
    let later = now + Duration::from_micros(31_250);
    assert!(sessions.admit(peer(1), later).is_ok());
    assert_eq!(
        sessions.admit(peer(1), later),
        Err(StatusCode::TOO_MANY_REQUESTS)
    );
    let later = now + Duration::from_secs(3);
    for _ in 0..64 {
        assert!(sessions.admit(peer(1), later).is_ok());
    }
    assert_eq!(
        sessions.admit(peer(1), later),
        Err(StatusCode::TOO_MANY_REQUESTS)
    );
}

#[test]
fn full_peer_table_fails_closed_without_evicting_a_live_lockout() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    let code = sessions.code(now).unwrap();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        let _ = sessions.pair(peer(1), wrong, "wrong", now);
    }
    let mut refused = false;
    for value in 1..=4096_u16 {
        let ip = IpAddr::from([10, 0, (value >> 8) as u8, value as u8]);
        if sessions.admit(ip, now).is_err() {
            refused = true;
            break;
        }
    }
    assert!(
        refused,
        "peer accounting must have a fixed bound below 4096"
    );
    assert_eq!(
        sessions.pair(peer(1), &code, "still locked", now).err(),
        Some(StatusCode::TOO_MANY_REQUESTS)
    );
    assert!(
        sessions.admit(peer(250), now).is_err(),
        "unknown peers cannot bypass a full accounting table"
    );
    assert!(
        sessions
            .admit(peer(250), now + Duration::from_secs(601))
            .is_ok(),
        "expired inactive entries must be reclaimed"
    );
}

#[test]
fn pair_label_limit_counts_utf8_bytes_and_refuses_controls() {
    let now = Instant::now();
    let mut sessions = Sessions::new(8, now).unwrap();
    let code = sessions.code(now).unwrap();
    assert!(sessions.pair(peer(1), &code, &"é".repeat(32), now).is_ok());
    for label in ["é".repeat(33), "a\nb".to_owned(), String::new()] {
        assert_eq!(
            sessions.pair(peer(2), &code, &label, now).err(),
            Some(StatusCode::BAD_REQUEST)
        );
    }
}
