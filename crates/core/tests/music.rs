use std::{fs, sync::Arc};
use tonalli_core::{
    media::MediaStore,
    music::{MusicStore, PlaylistItem},
    pack::PreadSource,
};

fn tracks(root: &std::path::Path) -> [String; 2] {
    let source = root.join("source.wav");
    fs::write(&source,b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0").unwrap();
    let media = MediaStore::open(root).unwrap();
    ["First.wav", "Second.wav"].map(|name| {
        media
            .import(Arc::new(PreadSource::open(&source).unwrap()), name, "")
            .unwrap()
            .id
    })
}
#[test]
fn nested_playlists_queue_resume_and_stale_events_preserve_data() {
    let root = tempfile::tempdir().unwrap();
    let [a, b] = tracks(root.path());
    let store = MusicStore::open(root.path()).unwrap();
    let parent = store.create_playlist(0, "Weekend").unwrap();
    let child = store.create_playlist(1, "Morning").unwrap();
    store
        .replace_playlist(
            2,
            &child.id,
            "Morning",
            vec![PlaylistItem::Track { id: b.clone() }],
        )
        .unwrap();
    store
        .replace_playlist(
            3,
            &parent.id,
            "Weekend",
            vec![
                PlaylistItem::Track { id: a.clone() },
                PlaylistItem::Playlist {
                    id: child.id.clone(),
                },
            ],
        )
        .unwrap();
    let queue = store.play_playlist(4, &parent.id).unwrap();
    assert_eq!(queue.tracks, vec![a.clone(), b.clone()]);
    assert_eq!(queue.current, Some(0));
    assert!(store.update_position(queue.generation, 2.5).unwrap());
    drop(store);
    let store = MusicStore::open(root.path()).unwrap();
    assert_eq!(store.snapshot().unwrap().queue.seconds, 2.5);
    let next = store.step(queue.generation, 1).unwrap();
    assert_eq!(next.current, Some(1));
    assert_eq!(next.seconds, 0.0);
    let before = fs::read(root.path().join("packs/private/music.json")).unwrap();
    assert!(!store.update_position(queue.generation, 99.0).unwrap());
    assert_eq!(
        fs::read(root.path().join("packs/private/music.json")).unwrap(),
        before
    );
    let state = store.snapshot().unwrap();
    assert!(store
        .replace_playlist(
            state.revision,
            &child.id,
            "Cycle",
            vec![PlaylistItem::Playlist {
                id: parent.id.clone()
            }]
        )
        .is_err());
    assert_eq!(
        fs::read(root.path().join("packs/private/music.json")).unwrap(),
        before
    );
    assert!(store
        .delete_playlist(state.revision, &child.id, false)
        .is_err());
    store
        .delete_playlist(state.revision, &child.id, true)
        .unwrap();
    let state = store.snapshot().unwrap();
    assert_eq!(state.playlists.len(), 1);
    assert_eq!(state.playlists[0].items.len(), 1);
    assert_eq!(state.queue.tracks, vec![a, b]);
    assert_eq!(fs::read(root.path().join("source.wav")).unwrap().len(), 44);
}
#[test]
fn queue_edits_keep_current_playback_and_corruption_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let [a, b] = tracks(root.path());
    let store = MusicStore::open(root.path()).unwrap();
    let queue = store
        .set_queue(0, vec![a.clone(), b.clone()], Some(0))
        .unwrap();
    assert!(store.update_position(queue.generation, 8.0).unwrap());
    let queue = store
        .set_queue(1, vec![b.clone(), a.clone()], Some(1))
        .unwrap();
    assert_eq!(queue.seconds, 8.0);
    let state = store.snapshot().unwrap();
    assert!(store
        .set_queue(state.revision, vec!["../../notes.db".into()], Some(0))
        .is_err());
    assert!(store.update_position(queue.generation, f64::NAN).is_err());
    assert!(store.set_queue(0, vec![a], Some(0)).is_err());
    let path = root.path().join("packs/private/music.json");
    fs::write(&path, b"damaged music catalog").unwrap();
    assert!(store.snapshot().is_err());
    assert!(store.create_playlist(0, "Replacement").is_err());
    assert_eq!(fs::read(path).unwrap(), b"damaged music catalog");
}

#[test]
fn repeated_empty_subplaylists_cannot_expand_without_a_work_bound() {
    use tonalli_core::music::MusicError;
    let root = tempfile::tempdir().unwrap();
    let store = MusicStore::open(root.path()).unwrap();
    let mut child = store.create_playlist(0, "Empty").unwrap().id;
    for _ in 0..14 {
        let revision = store.snapshot().unwrap().revision;
        let parent = store.create_playlist(revision, "Nested").unwrap();
        store
            .replace_playlist(
                revision + 1,
                &parent.id,
                "Nested",
                vec![
                    PlaylistItem::Playlist { id: child.clone() },
                    PlaylistItem::Playlist { id: child },
                ],
            )
            .unwrap();
        child = parent.id;
    }
    let revision = store.snapshot().unwrap().revision;
    let parent = store.create_playlist(revision, "Repeated").unwrap();
    store
        .replace_playlist(
            revision + 1,
            &parent.id,
            "Repeated",
            vec![PlaylistItem::Playlist { id: child }; 100],
        )
        .unwrap();
    let path = root.path().join("packs/private/music.json");
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        store.play_playlist(revision + 2, &parent.id),
        Err(MusicError::Invalid)
    ));
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn a_seek_rejects_late_position_and_completion_events() {
    let root = tempfile::tempdir().unwrap();
    let [track, _] = tracks(root.path());
    let store = MusicStore::open(root.path()).unwrap();
    let queue = store.set_queue(0, vec![track], Some(0)).unwrap();
    let sought = store.seek(queue.generation, 12.0).unwrap();
    let history = tonalli_core::reading::ProgressStore::open(root.path()).unwrap();
    assert_eq!(
        history.list().unwrap()[0].position,
        tonalli_core::reading::Position::Time { seconds: 12.0 }
    );
    assert_ne!(sought.generation, queue.generation);
    assert!(!store.update_position(queue.generation, 1.0).unwrap());
    assert!(store.step(queue.generation, 1).is_err());
    assert_eq!(store.snapshot().unwrap().queue.seconds, 12.0);
    assert_eq!(
        history.list().unwrap()[0].position,
        tonalli_core::reading::Position::Time { seconds: 12.0 }
    );
}

#[test]
fn selecting_a_previous_track_resumes_its_private_history_position() {
    let root = tempfile::tempdir().unwrap();
    let [a, b] = tracks(root.path());
    let store = MusicStore::open(root.path()).unwrap();
    let queue = store.set_queue(0, vec![a.clone()], Some(0)).unwrap();
    store.update_position(queue.generation, 7.0).unwrap();
    store.set_queue(1, vec![b], Some(0)).unwrap();
    let resumed = store.set_queue(2, vec![a], Some(0)).unwrap();
    assert_eq!(resumed.seconds, 7.0);
}
