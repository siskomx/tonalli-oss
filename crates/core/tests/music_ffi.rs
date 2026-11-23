#![cfg(feature = "ffi")]
use std::{fs, sync::Arc};
use tonalli_core::{ffi::music_source_open, media::MediaStore, pack::PreadSource};
#[test]
fn native_audio_handle_is_positioned_bounded_and_independent() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original.wav");
    fs::write(&original,b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0").unwrap();
    let store = MediaStore::open(root.path()).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&original).unwrap()),
            "Track.wav",
            "",
        )
        .unwrap();
    let source = music_source_open(root.path().display().to_string(), file.id.clone()).unwrap();
    assert_eq!(source.size(), 44);
    assert_eq!(source.name(), "Track.wav");
    assert_eq!(source.read(0, 4).unwrap(), b"RIFF");
    assert!(source.read(44, 1).unwrap().is_empty());
    assert!(source.read(u64::MAX, u32::MAX).unwrap().is_empty());
    store.remove(&file.id, file.version).unwrap();
    assert_eq!(source.read(8, 4).unwrap(), b"WAVE");
    assert_eq!(fs::read(original).unwrap().len(), 44);
    assert!(music_source_open(root.path().display().to_string(), "../../notes.db".into()).is_err());
}
