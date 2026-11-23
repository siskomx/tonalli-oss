//! Private playlists and one persistent playback queue; platform decoders remain native.
mod graph;
mod playback;
pub use playback::{PlaybackAction, PlaybackStatus, PlaybackTrack};
mod source;
use crate::{content::storage, download::write_atomic, media::MediaStore, pack::PackError};
use serde::{Deserialize, Serialize};
pub use source::audio_source;
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_TRACKS: usize = 10000;
#[derive(Debug, thiserror::Error)]
pub enum MusicError {
    #[error("This file is not a supported audio container")]
    Unsupported,
    #[error("Invalid playlist or queue request")]
    Invalid,
    #[error("Music changed; refresh and try again")]
    Conflict,
    #[error("A playlist or track is unavailable")]
    Missing,
    #[error("Music metadata is corrupt or unavailable")]
    Corrupt,
    #[error(transparent)]
    Pack(#[from] PackError),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlaylistItem {
    Track { id: String },
    Playlist { id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub items: Vec<PlaylistItem>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    pub generation: u64,
    pub tracks: Vec<String>,
    pub current: Option<u32>,
    pub seconds: f64,
}
impl Queue {
    pub fn current_id(&self) -> Option<&str> {
        self.current
            .and_then(|index| self.tracks.get(index as usize))
            .map(String::as_str)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicState {
    pub revision: u64,
    pub playlists: Vec<Playlist>,
    pub queue: Queue,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    state: MusicState,
}
pub struct MusicStore {
    root: PathBuf,
}
fn id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn name(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 240 && !value.chars().any(char::is_control)
}
fn seconds(value: f64) -> bool {
    value.is_finite() && (0.0..=1_000_000_000.0).contains(&value)
}
pub fn audio_name(value: &str) -> bool {
    matches!(
        value
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .as_deref(),
        Some("wav" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus")
    )
}
impl MusicState {
    fn valid(&self) -> Result<(), MusicError> {
        let queue = &self.queue;
        if self.playlists.len() > 256
            || queue.tracks.len() > MAX_TRACKS
            || !seconds(queue.seconds)
            || queue.tracks.iter().any(|track| !id(track))
            || (queue
                .current
                .is_some_and(|index| index as usize >= queue.tracks.len()))
            || (queue.current.is_none() && queue.seconds != 0.0)
            || self.playlists.iter().any(|playlist| {
                !id(&playlist.id) || !name(&playlist.name) || playlist.items.len() > MAX_TRACKS
            })
            || self
                .playlists
                .iter()
                .map(|playlist| playlist.items.len())
                .sum::<usize>()
                > MAX_TRACKS
        {
            return Err(MusicError::Invalid);
        }
        let unique: HashSet<_> = self.playlists.iter().map(|playlist| &playlist.id).collect();
        if unique.len() != self.playlists.len() {
            return Err(MusicError::Invalid);
        }
        for playlist in &self.playlists {
            for item in &playlist.items {
                match item {
                    PlaylistItem::Track { id: track } if id(track) => {}
                    PlaylistItem::Playlist { id: child } if unique.contains(child) => {}
                    _ => return Err(MusicError::Invalid),
                }
            }
        }
        graph::validate(&self.playlists)
    }
    fn changed(&mut self) -> Result<(), MusicError> {
        self.revision = self.revision.checked_add(1).ok_or(MusicError::Invalid)?;
        Ok(())
    }
}
impl MusicStore {
    pub fn open(root: &Path) -> Result<Self, MusicError> {
        Ok(Self {
            root: root.canonicalize().map_err(PackError::from)?,
        })
    }
    fn path(&self, name: &str) -> Result<PathBuf, MusicError> {
        Ok(storage::path(&self.root, &format!("packs/private/{name}"))?)
    }
    fn read(&self) -> Result<MusicState, MusicError> {
        let file = match File::open(self.path("music.json")?) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MusicState::default())
            }
            Err(error) => return Err(PackError::from(error).into()),
        };
        let mut bytes = vec![];
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(PackError::from)?;
        if bytes.len() > 1024 * 1024 {
            return Err(MusicError::Corrupt);
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| MusicError::Corrupt)?;
        if document.version != 1 {
            return Err(MusicError::Corrupt);
        }
        document.state.valid().map_err(|_| MusicError::Corrupt)?;
        Ok(document.state)
    }
    fn write(&self, state: &MusicState) -> Result<(), MusicError> {
        state.valid()?;
        let bytes = serde_json::to_vec(&Document {
            version: 1,
            state: state.clone(),
        })
        .map_err(|_| MusicError::Corrupt)?;
        if bytes.len() > 1024 * 1024 {
            return Err(MusicError::Invalid);
        }
        let path = self.path("music.json")?;
        write_atomic(&path, &bytes)?;
        storage::durable(&path)?;
        Ok(())
    }
    fn edit<T>(
        &self,
        revision: u64,
        operation: impl FnOnce(&mut MusicState) -> Result<T, MusicError>,
    ) -> Result<T, MusicError> {
        let _lock = storage::lock(&self.path("music.lock")?)?;
        let mut state = self.read()?;
        if state.revision != revision {
            return Err(MusicError::Conflict);
        }
        let result = operation(&mut state)?;
        state.changed()?;
        self.write(&state)?;
        Ok(result)
    }
    fn check_tracks(&self, tracks: &[String]) -> Result<(), MusicError> {
        if tracks.len() > MAX_TRACKS || tracks.iter().any(|track| !id(track)) {
            return Err(MusicError::Invalid);
        }
        let allowed: HashSet<_> = MediaStore::open(&self.root)
            .and_then(|store| store.list())
            .map_err(|_| MusicError::Missing)?
            .into_iter()
            .filter(|file| audio_name(&file.name))
            .map(|file| file.id)
            .collect();
        if tracks.iter().any(|track| !allowed.contains(track)) {
            return Err(MusicError::Missing);
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Result<MusicState, MusicError> {
        let _lock = storage::lock(&self.path("music.lock")?)?;
        self.read()
    }
    pub fn create_playlist(&self, revision: u64, label: &str) -> Result<Playlist, MusicError> {
        if !name(label) {
            return Err(MusicError::Invalid);
        }
        self.edit(revision, |state| {
            if state.playlists.len() >= 256 {
                return Err(MusicError::Invalid);
            }
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|e| PackError::Io(e.to_string()))?;
            let playlist = Playlist {
                id: random.iter().map(|b| format!("{b:02x}")).collect(),
                name: label.into(),
                items: vec![],
            };
            state.playlists.push(playlist.clone());
            Ok(playlist)
        })
    }
    pub fn replace_playlist(
        &self,
        revision: u64,
        playlist_id: &str,
        label: &str,
        items: Vec<PlaylistItem>,
    ) -> Result<(), MusicError> {
        if !name(label) || items.len() > MAX_TRACKS {
            return Err(MusicError::Invalid);
        }
        self.edit(revision, |state| {
            let tracks: Vec<_> = items
                .iter()
                .filter_map(|item| {
                    if let PlaylistItem::Track { id } = item {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            self.check_tracks(&tracks)?;
            let playlist = state
                .playlists
                .iter_mut()
                .find(|playlist| playlist.id == playlist_id)
                .ok_or(MusicError::Missing)?;
            playlist.name = label.into();
            playlist.items = items;
            Ok(())
        })
    }
    pub fn delete_playlist(
        &self,
        revision: u64,
        playlist_id: &str,
        confirmed: bool,
    ) -> Result<(), MusicError> {
        if !confirmed {
            return Err(MusicError::Invalid);
        }
        self.edit(revision, |state| {
            let index = state
                .playlists
                .iter()
                .position(|playlist| playlist.id == playlist_id)
                .ok_or(MusicError::Missing)?;
            state.playlists.remove(index);
            for playlist in &mut state.playlists {
                playlist
                    .items
                    .retain(|item| !matches!(item,PlaylistItem::Playlist{id} if id==playlist_id));
            }
            Ok(())
        })
    }
    pub fn set_queue(
        &self,
        revision: u64,
        tracks: Vec<String>,
        current: Option<u32>,
    ) -> Result<Queue, MusicError> {
        self.edit(revision, |state| {
            self.check_tracks(&tracks)?;
            if current.is_some_and(|index| index as usize >= tracks.len())
                || (current.is_none() && !tracks.is_empty())
            {
                return Err(MusicError::Invalid);
            }
            let selected = current.and_then(|index| tracks.get(index as usize));
            let same = selected.map(String::as_str) == state.queue.current_id();
            let position = if same {
                state.queue.seconds
            } else {
                self.resume_position(selected.map(String::as_str))?
            };
            let generation = if same {
                state.queue.generation
            } else {
                state
                    .queue
                    .generation
                    .checked_add(1)
                    .ok_or(MusicError::Invalid)?
            };
            state.queue = Queue {
                generation,
                tracks,
                current,
                seconds: position,
            };
            Ok(state.queue.clone())
        })
    }
    pub fn play_playlist(&self, revision: u64, playlist_id: &str) -> Result<Queue, MusicError> {
        self.edit(revision, |state| {
            let tracks = graph::flatten(&state.playlists, playlist_id)?;
            self.check_tracks(&tracks)?;
            if tracks.is_empty() {
                return Err(MusicError::Missing);
            }
            state.queue = Queue {
                generation: state
                    .queue
                    .generation
                    .checked_add(1)
                    .ok_or(MusicError::Invalid)?,
                tracks,
                current: Some(0),
                seconds: 0.0,
            };
            Ok(state.queue.clone())
        })
    }
    pub fn step(&self, generation: u64, delta: i32) -> Result<Queue, MusicError> {
        if delta != 1 && delta != -1 {
            return Err(MusicError::Invalid);
        }
        let _lock = storage::lock(&self.path("music.lock")?)?;
        let mut state = self.read()?;
        if state.queue.generation != generation {
            return Err(MusicError::Conflict);
        }
        if state.queue.tracks.is_empty() {
            return Err(MusicError::Missing);
        }
        state.queue.current = match (state.queue.current, delta) {
            (Some(index), 1) => {
                ((index as usize + 1) < state.queue.tracks.len()).then_some(index + 1)
            }
            (Some(index), _) => Some(index.saturating_sub(1)),
            (None, 1) => Some(0),
            (None, _) => Some((state.queue.tracks.len() - 1) as u32),
        };
        state.queue.seconds = 0.0;
        state.queue.generation = state
            .queue
            .generation
            .checked_add(1)
            .ok_or(MusicError::Invalid)?;
        state.changed()?;
        self.write(&state)?;
        Ok(state.queue)
    }
    pub fn seek(&self, generation: u64, position: f64) -> Result<Queue, MusicError> {
        if !seconds(position) {
            return Err(MusicError::Invalid);
        }
        let _lock = storage::lock(&self.path("music.lock")?)?;
        let mut state = self.read()?;
        if state.queue.generation != generation {
            return Err(MusicError::Conflict);
        }
        if state.queue.current.is_none() {
            return Err(MusicError::Missing);
        }
        state.queue.seconds = position;
        state.queue.generation = state
            .queue
            .generation
            .checked_add(1)
            .ok_or(MusicError::Invalid)?;
        state.changed()?;
        self.write(&state)?;
        self.save_history(&state.queue)?;
        Ok(state.queue)
    }
    pub fn update_position(&self, generation: u64, position: f64) -> Result<bool, MusicError> {
        if !seconds(position) {
            return Err(MusicError::Invalid);
        }
        let _lock = storage::lock(&self.path("music.lock")?)?;
        let mut state = self.read()?;
        if state.queue.generation != generation || state.queue.current.is_none() {
            return Ok(false);
        }
        state.queue.seconds = position;
        self.write(&state)?;
        self.save_history(&state.queue)?;
        Ok(true)
    }
    fn save_history(&self, queue: &Queue) -> Result<(), MusicError> {
        use crate::reading::{Destination, Position, ProgressStore, ReadingEntry};
        let Some(id) = queue.current_id() else {
            return Ok(());
        };
        let file = match MediaStore::open(&self.root).and_then(|store| store.get(id)) {
            Ok(file) => file,
            Err(crate::media::MediaError::NotFound) => return Ok(()),
            Err(_) => return Err(MusicError::Corrupt),
        };
        ProgressStore::open(&self.root)?.save(ReadingEntry {
            destination: Destination::Media { id: id.into() },
            position: Position::Time {
                seconds: queue.seconds,
            },
            title: file.name,
            bookmarked: false,
        })?;
        Ok(())
    }
    fn resume_position(&self, id: Option<&str>) -> Result<f64, MusicError> {
        use crate::reading::{Destination, Position, ProgressStore};
        let Some(id) = id else { return Ok(0.0) };
        Ok(ProgressStore::open(&self.root)?
            .list()?
            .into_iter()
            .find_map(|entry| {
                if matches!(entry.destination, Destination::Media { id: ref old } if old == id) {
                    if let Position::Time { seconds } = entry.position {
                        return Some(seconds);
                    }
                }
                None
            })
            .unwrap_or(0.0))
    }
}
