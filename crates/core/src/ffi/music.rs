use super::{pack_boundary, LoopbackHandle};
use crate::{
    music::{audio_source, MusicError, MusicStore, PlaybackAction, PlaybackStatus, PlaybackTrack},
    pack::{PackError, PackSource},
    server::MusicControl,
};
use std::{path::Path, sync::Arc};

fn error(problem: MusicError) -> PackError {
    match problem {
        MusicError::Pack(error) => error,
        MusicError::Missing => PackError::NotFound("Music track".into()),
        MusicError::Corrupt => PackError::Corrupt("Music metadata".into()),
        other => PackError::Mismatch(other.to_string()),
    }
}
#[derive(uniffi::Object)]
pub struct AudioSource {
    source: Arc<dyn PackSource>,
    name: String,
}
#[uniffi::export]
impl AudioSource {
    pub fn size(&self) -> u64 {
        self.source.len()
    }
    pub fn name(&self) -> String {
        self.name.clone()
    }
    pub fn read(&self, offset: u64, size: u32) -> Result<Vec<u8>, PackError> {
        pack_boundary(|| {
            let count =
                (self.source.len().saturating_sub(offset)).min(size.min(65536) as u64) as usize;
            if count == 0 {
                return Ok(vec![]);
            }
            let mut bytes = vec![0; count];
            let n = self.source.read_at(offset, &mut bytes)?;
            if n == 0 {
                return Err(PackError::Truncated);
            }
            if n > count {
                return Err(PackError::Corrupt("Invalid audio read length".into()));
            }
            bytes.truncate(n);
            Ok(bytes)
        })
    }
}
#[uniffi::export]
pub fn music_source_open(root: String, id: String) -> Result<Arc<AudioSource>, PackError> {
    pack_boundary(|| {
        let (file, source, _) = audio_source(Path::new(&root), &id).map_err(error)?;
        Ok(Arc::new(AudioSource {
            source,
            name: file.name,
        }))
    })
}
#[uniffi::export]
pub fn music_current(root: String) -> Result<PlaybackTrack, PackError> {
    pack_boundary(|| {
        let queue = MusicStore::open(Path::new(&root))
            .and_then(|store| store.snapshot())
            .map_err(error)?
            .queue;
        Ok(PlaybackTrack::from(&queue))
    })
}
#[uniffi::export]
pub fn music_step(root: String, generation: u64, delta: i32) -> Result<PlaybackTrack, PackError> {
    pack_boundary(|| {
        let queue = MusicStore::open(Path::new(&root))
            .and_then(|store| store.step(generation, delta))
            .map_err(error)?;
        Ok(PlaybackTrack::from(&queue))
    })
}
#[uniffi::export]
pub fn music_seek(root: String, generation: u64, seconds: f64) -> Result<PlaybackTrack, PackError> {
    pack_boundary(|| {
        let queue = MusicStore::open(Path::new(&root))
            .and_then(|store| store.seek(generation, seconds))
            .map_err(error)?;
        Ok(PlaybackTrack::from(&queue))
    })
}
#[uniffi::export]
pub fn music_save_position(root: String, generation: u64, seconds: f64) -> Result<bool, PackError> {
    pack_boundary(|| {
        MusicStore::open(Path::new(&root))
            .and_then(|store| store.update_position(generation, seconds))
            .map_err(error)
    })
}
#[uniffi::export(foreign)]
pub trait MusicScheduler: Send + Sync {
    fn status(&self) -> Result<PlaybackStatus, PackError>;
    fn control(&self, action: PlaybackAction, track: PlaybackTrack) -> Result<(), PackError>;
}
struct Dispatcher(Arc<dyn MusicScheduler>);
impl MusicControl for Dispatcher {
    fn status(&self) -> Result<PlaybackStatus, PackError> {
        self.0.status()
    }
    fn control(&self, action: PlaybackAction, track: PlaybackTrack) -> Result<(), PackError> {
        self.0.control(action, track)
    }
}
#[uniffi::export]
pub fn server_attach_music(
    source: Arc<LoopbackHandle>,
    scheduler: Arc<dyn MusicScheduler>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        source
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback lock".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Reader closed".into()))?
            .attach_music(Arc::new(Dispatcher(scheduler)))
    })
}
