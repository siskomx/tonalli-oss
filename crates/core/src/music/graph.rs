use super::{MusicError, Playlist, PlaylistItem, MAX_TRACKS};
use std::collections::{HashMap, HashSet};

pub(super) fn validate(playlists: &[Playlist]) -> Result<(), MusicError> {
    let by_id: HashMap<_, _> = playlists
        .iter()
        .map(|playlist| (playlist.id.as_str(), playlist))
        .collect();
    fn depth<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a Playlist>,
        visiting: &mut HashSet<&'a str>,
        done: &mut HashMap<&'a str, usize>,
    ) -> Result<usize, MusicError> {
        if let Some(value) = done.get(id) {
            return Ok(*value);
        }
        if !visiting.insert(id) {
            return Err(MusicError::Invalid);
        }
        if visiting.len() > 16 {
            return Err(MusicError::Invalid);
        }
        let playlist = by_id.get(id).ok_or(MusicError::Missing)?;
        let mut maximum = 1;
        for item in &playlist.items {
            if let PlaylistItem::Playlist { id: child } = item {
                maximum = maximum.max(1 + depth(child, by_id, visiting, done)?);
            }
        }
        visiting.remove(id);
        if maximum > 16 {
            return Err(MusicError::Invalid);
        }
        done.insert(id, maximum);
        Ok(maximum)
    }
    let mut done = HashMap::new();
    for playlist in playlists {
        depth(&playlist.id, &by_id, &mut HashSet::new(), &mut done)?;
    }
    Ok(())
}
pub(super) fn flatten(playlists: &[Playlist], id: &str) -> Result<Vec<String>, MusicError> {
    validate(playlists)?;
    fn append(
        playlists: &[Playlist],
        id: &str,
        tracks: &mut Vec<String>,
        level: usize,
        remaining: &mut usize,
    ) -> Result<(), MusicError> {
        *remaining = remaining.checked_sub(1).ok_or(MusicError::Invalid)?;
        if level > 16 {
            return Err(MusicError::Invalid);
        }
        let playlist = playlists
            .iter()
            .find(|playlist| playlist.id == id)
            .ok_or(MusicError::Missing)?;
        for item in &playlist.items {
            match item {
                PlaylistItem::Track { id } => {
                    if tracks.len() >= MAX_TRACKS {
                        return Err(MusicError::Invalid);
                    }
                    tracks.push(id.clone());
                }
                PlaylistItem::Playlist { id } => {
                    append(playlists, id, tracks, level + 1, remaining)?
                }
            }
        }
        Ok(())
    }
    let mut tracks = vec![];
    // Track count alone cannot bound repeated empty subplaylists.
    let mut remaining = 200_000;
    append(playlists, id, &mut tracks, 1, &mut remaining)?;
    Ok(tracks)
}
