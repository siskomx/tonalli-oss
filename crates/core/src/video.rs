//! Bounded container admission for managed videos and raster covers. The OS decodes them.
use crate::{
    media::{MediaError, MediaFile, MediaStore},
    pack::{PackError, PackSource},
};
use std::{path::Path, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("Invalid managed media identifier")]
    Invalid,
    #[error("This file is unavailable")]
    Missing,
    #[error("This media container is unsupported")]
    Unsupported,
    #[error(transparent)]
    Pack(#[from] PackError),
}
pub fn source(
    root: &Path,
    id: &str,
    image: bool,
) -> Result<(MediaFile, Arc<dyn PackSource>, &'static str), VideoError> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(VideoError::Invalid);
    }
    let (file, source) = MediaStore::open(root)
        .and_then(|store| store.open_file(id))
        .map_err(|error| match error {
            MediaError::NotFound => VideoError::Missing,
            MediaError::Invalid => VideoError::Invalid,
            MediaError::Pack(problem) => VideoError::Pack(problem),
            _ => VideoError::Pack(PackError::Corrupt("Managed media catalog".into())),
        })?;
    let extension = file
        .name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let mut prefix = [0; 64];
    let wanted = source.len().min(prefix.len() as u64) as usize;
    let mut filled = 0;
    while filled < wanted {
        let count = source.read_at(filled as u64, &mut prefix[filled..wanted])?;
        if count == 0 {
            return Err(PackError::Truncated.into());
        }
        if count > wanted - filled {
            return Err(PackError::Corrupt("Invalid media read length".into()).into());
        }
        filled += count;
    }
    let bytes = &prefix[..filled];
    let mime = if image {
        if matches!(extension.as_str(), "jpg" | "jpeg")
            && bytes.starts_with(&[0xff, 0xd8, 0xff])
            && bytes.len() >= 12
        {
            "image/jpeg"
        } else if extension == "png"
            && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            && bytes.get(12..16) == Some(b"IHDR")
            && bytes.len() >= 24
        {
            "image/png"
        } else if extension == "webp"
            && bytes.starts_with(b"RIFF")
            && bytes.get(8..12) == Some(b"WEBP")
            && bytes.len() >= 20
        {
            "image/webp"
        } else if extension == "gif"
            && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
            && bytes.len() >= 13
            && (bytes[6] != 0 || bytes[7] != 0)
            && (bytes[8] != 0 || bytes[9] != 0)
        {
            "image/gif"
        } else if extension == "bmp" && bytes.starts_with(b"BM") && bytes.len() >= 26 {
            "image/bmp"
        } else if extension == "avif"
            && bytes.len() >= 24
            && bytes.get(4..8) == Some(b"ftyp")
            && (bytes.get(8..12) == Some(b"avif") || bytes.get(8..12) == Some(b"avis"))
        {
            let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
            if size < 16 || size > source.len() {
                return Err(VideoError::Unsupported);
            }
            "image/avif"
        } else {
            return Err(VideoError::Unsupported);
        }
    } else {
        if matches!(extension.as_str(), "mp4" | "m4v" | "mov")
            && bytes.len() >= 24
            && bytes.get(4..8) == Some(b"ftyp")
        {
            let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
            if size < 16 || size > source.len() {
                return Err(VideoError::Unsupported);
            }
            if extension == "mov" {
                "video/quicktime"
            } else {
                "video/mp4"
            }
        } else if matches!(extension.as_str(), "webm" | "mkv")
            && bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
            && bytes.len() >= 16
        {
            if extension == "webm" {
                "video/webm"
            } else {
                "video/x-matroska"
            }
        } else if extension == "avi"
            && bytes.starts_with(b"RIFF")
            && bytes.get(8..12) == Some(b"AVI ")
            && bytes.len() >= 20
        {
            "video/x-msvideo"
        } else {
            return Err(VideoError::Unsupported);
        }
    };
    Ok((file, source, mime))
}
