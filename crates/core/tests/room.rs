use std::time::{Duration, Instant};
use tonalli_core::room::{Room, RoomAction, RoomError, RoomMember, RoomRequest};
fn member(id: u8) -> RoomMember {
    RoomMember {
        id: if id == 0 {
            "host".into()
        } else {
            format!("{id:064x}")
        },
        label: if id == 0 {
            "Host".into()
        } else {
            format!("Reader {id}")
        },
        is_host: id == 0,
    }
}
fn request(action: RoomAction, view: u8, epoch: &str, text: &str, after: u64) -> RoomRequest {
    RoomRequest {
        action,
        view: format!("{view:032x}"),
        epoch: epoch.into(),
        text: text.into(),
        after,
    }
}
fn epoch(room: &mut Room, now: Instant) -> String {
    let view = room
        .request(member(0), request(RoomAction::Get, 1, "", "", 0), now)
        .unwrap();
    assert!(view.active);
    assert!(!view.joined);
    assert!(view.messages.is_empty());
    view.epoch
}
#[test]
fn explicit_join_broadcast_incremental_reads_and_leave_are_bounded() {
    let now = Instant::now();
    let mut room = Room::new().unwrap();
    let epoch = epoch(&mut room, now);
    assert_eq!(
        room.request(
            member(1),
            request(RoomAction::Send, 1, &epoch, "hello", 0),
            now
        )
        .unwrap_err(),
        RoomError::NotJoined
    );
    room.request(member(0), request(RoomAction::Join, 1, &epoch, "", 0), now)
        .unwrap();
    room.request(member(1), request(RoomAction::Join, 2, &epoch, "", 0), now)
        .unwrap();
    let sent = room
        .request(
            member(0),
            request(
                RoomAction::Send,
                1,
                &epoch,
                "Hola 🦊 <script>literal</script>",
                0,
            ),
            now,
        )
        .unwrap();
    assert_eq!(sent.latest, 1);
    assert!(sent.messages[0].sender.is_host);
    let peer = room
        .request(member(1), request(RoomAction::Get, 2, &epoch, "", 0), now)
        .unwrap();
    assert_eq!(peer.messages[0].text, "Hola 🦊 <script>literal</script>");
    assert_eq!(peer.members.len(), 2);
    assert!(room
        .request(member(1), request(RoomAction::Get, 2, &epoch, "", 1), now)
        .unwrap()
        .messages
        .is_empty());
    let left = room
        .request(member(1), request(RoomAction::Leave, 2, &epoch, "", 0), now)
        .unwrap();
    assert!(!left.joined && left.messages.is_empty());
    assert_eq!(
        room.request(
            member(1),
            request(RoomAction::Send, 2, &epoch, "no", 0),
            now
        )
        .unwrap_err(),
        RoomError::NotJoined
    );
}
#[test]
fn view_leases_prevent_old_page_leaves_from_ejecting_new_views() {
    let now = Instant::now();
    let mut room = Room::new().unwrap();
    let epoch = epoch(&mut room, now);
    for view in 1..=4 {
        room.request(
            member(1),
            request(RoomAction::Join, view, &epoch, "", 0),
            now,
        )
        .unwrap();
    }
    assert_eq!(
        room.request(member(1), request(RoomAction::Join, 5, &epoch, "", 0), now)
            .unwrap_err(),
        RoomError::Full
    );
    room.request(member(1), request(RoomAction::Leave, 1, &epoch, "", 0), now)
        .unwrap();
    let current = room
        .request(member(1), request(RoomAction::Get, 2, &epoch, "", 0), now)
        .unwrap();
    assert!(current.joined);
    assert_eq!(current.members.len(), 1);
    assert!(
        !room
            .request(
                member(1),
                request(RoomAction::Get, 2, &epoch, "", 0),
                now + Duration::from_secs(31)
            )
            .unwrap()
            .joined
    );
}
#[test]
fn limits_rates_epochs_and_host_identity_do_not_mutate_on_failure() {
    let now = Instant::now();
    let mut room = Room::new().unwrap();
    let epoch = epoch(&mut room, now);
    room.request(member(0), request(RoomAction::Join, 1, &epoch, "", 0), now)
        .unwrap();
    for text in ["", "\0", &"🦊".repeat(1001)] {
        assert_eq!(
            room.request(
                member(0),
                request(RoomAction::Send, 1, &epoch, text, 0),
                now
            )
            .unwrap_err(),
            RoomError::Invalid
        );
    }
    assert_eq!(
        room.request(member(0), request(RoomAction::Send, 1, "old", "no", 0), now)
            .unwrap_err(),
        RoomError::Changed
    );
    let mut forged = member(1);
    forged.is_host = true;
    assert_eq!(
        room.request(forged, request(RoomAction::Join, 1, &epoch, "", 0), now)
            .unwrap_err(),
        RoomError::Invalid
    );
    room.request(
        member(0),
        request(RoomAction::Send, 1, &epoch, &"🦊".repeat(1000), 0),
        now,
    )
    .unwrap();
    for _ in 0..3 {
        room.request(
            member(0),
            request(RoomAction::Send, 1, &epoch, "ok", 0),
            now,
        )
        .unwrap();
    }
    assert_eq!(
        room.request(
            member(0),
            request(RoomAction::Send, 1, &epoch, "limited", 0),
            now
        )
        .unwrap_err(),
        RoomError::RateLimited
    );
    assert_eq!(
        room.request(member(0), request(RoomAction::Get, 1, &epoch, "", 0), now)
            .unwrap()
            .latest,
        4
    );
    assert_eq!(
        room.request(
            member(0),
            request(RoomAction::Send, 1, &epoch, "refilled", 0),
            now + Duration::from_secs(1)
        )
        .unwrap()
        .latest,
        5
    );
}
#[test]
fn history_tail_expiry_kick_and_stop_drop_room_access() {
    let now = Instant::now();
    let mut room = Room::new().unwrap();
    let epoch = epoch(&mut room, now);
    room.request(member(0), request(RoomAction::Join, 1, &epoch, "", 0), now)
        .unwrap();
    for i in 1..=105 {
        room.request(
            member(0),
            request(RoomAction::Send, 1, &epoch, &format!("message {i}"), 0),
            now + Duration::from_secs(i),
        )
        .unwrap();
    }
    let view = room
        .request(
            member(0),
            request(RoomAction::Get, 1, &epoch, "", 0),
            now + Duration::from_secs(105),
        )
        .unwrap();
    assert_eq!(view.messages.len(), 100);
    assert_eq!(view.messages[0].id, 6);
    assert_eq!(view.latest, 105);
    room.remove_actor("host");
    assert!(
        !room
            .request(
                member(0),
                request(RoomAction::Get, 1, &epoch, "", 0),
                now + Duration::from_secs(105)
            )
            .unwrap()
            .joined
    );
    room.end();
    let ended = room
        .request(member(0), request(RoomAction::Get, 1, &epoch, "", 0), now)
        .unwrap();
    assert!(!ended.active && ended.messages.is_empty() && ended.members.is_empty());
    assert_eq!(
        room.request(member(0), request(RoomAction::Join, 1, &epoch, "", 0), now)
            .unwrap_err(),
        RoomError::Unavailable
    );
}
