use super::{audio_name, MusicError};
use crate::{
    media::{MediaError, MediaFile, MediaStore},
    pack::{PackError, PackSource},
};
use std::{path::Path, sync::Arc};

pub fn audio_source(
    root: &Path,
    id: &str,
) -> Result<(MediaFile, Arc<dyn PackSource>, &'static str), MusicError> {
    if !super::id(id) {
        return Err(MusicError::Invalid);
    }
    let (file, source) = MediaStore::open(root)
        .and_then(|store| store.open_file(id))
        .map_err(|error| match error {
            MediaError::Invalid => MusicError::Invalid,
            MediaError::NotFound => MusicError::Missing,
            MediaError::Pack(error) => MusicError::Pack(error),
            _ => MusicError::Corrupt,
        })?;
    if !audio_name(&file.name) {
        return Err(MusicError::Unsupported);
    }
    let mut prefix = [0; 64];
    let wanted = (source.len().min(prefix.len() as u64)) as usize;
    let mut filled = 0;
    while filled < wanted {
        let n = source.read_at(filled as u64, &mut prefix[filled..wanted])?;
        if n == 0 {
            return Err(PackError::Truncated.into());
        }
        if n > wanted - filled {
            return Err(PackError::Corrupt("Invalid audio read length".into()).into());
        }
        filled += n;
    }
    let bytes = &prefix[..filled];
    let mime = if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        "audio/wav"
    } else if bytes.starts_with(b"fLaC") {
        "audio/flac"
    } else if bytes.starts_with(b"OggS") {
        "audio/ogg"
    } else if bytes.get(4..8) == Some(b"ftyp") {
        "audio/mp4"
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
    {
        if file.name.to_ascii_lowercase().ends_with(".aac") {
            "audio/aac"
        } else {
            "audio/mpeg"
        }
    } else {
        return Err(MusicError::Unsupported);
    };
    Ok((file, source, mime))
}
