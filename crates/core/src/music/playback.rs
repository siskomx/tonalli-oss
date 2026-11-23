use super::Queue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackAction {
    Load,
    Play,
    Pause,
    Stop,
    Seek,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PlaybackTrack {
    pub id: Option<String>,
    pub generation: u64,
    pub seconds: f64,
}
impl From<&Queue> for PlaybackTrack {
    fn from(queue: &Queue) -> Self {
        Self {
            id: queue.current_id().map(str::to_owned),
            generation: queue.generation,
            seconds: queue.seconds,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PlaybackStatus {
    pub available: bool,
    pub playing: bool,
    pub loading: bool,
    pub id: Option<String>,
    pub generation: u64,
    pub seconds: f64,
    pub duration: f64,
    pub error: Option<String>,
}
