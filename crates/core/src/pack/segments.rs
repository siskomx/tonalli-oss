use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::source::check_range;
use super::{PackError, PackSource, PreadSource};

/// A bounded view, for example one extent inside a larger pack image.
pub struct WindowSource {
    source: Arc<dyn PackSource>,
    offset: u64,
    len: u64,
}

impl WindowSource {
    pub fn new(source: Arc<dyn PackSource>, offset: u64, len: u64) -> Result<Self, PackError> {
        if offset.checked_add(len).is_none_or(|end| end > source.len()) {
            return Err(PackError::Mismatch("window extends past its source".into()));
        }
        Ok(Self {
            source,
            offset,
            len,
        })
    }
}

impl PackSource for WindowSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        check_range(self.len, off, buf.len())?;
        read_exact_at(&*self.source, self.offset + off, buf)
    }
}

struct Part {
    source: Arc<dyn PackSource>,
    start: u64,
    len: u64,
}

/// Ordered split files. Once removal is observed, reopen with fresh handles.
pub struct ConcatSource {
    parts: Vec<Part>,
    len: u64,
    removed: AtomicBool,
}

impl ConcatSource {
    pub fn from_files(parts: Vec<(String, File)>) -> Result<Self, PackError> {
        let sources = parts
            .into_iter()
            .map(|(name, file)| {
                Ok((
                    name,
                    Arc::new(PreadSource::from_file(file)?) as Arc<dyn PackSource>,
                ))
            })
            .collect::<Result<_, PackError>>()?;
        Self::from_parts(sources)
    }

    pub fn from_parts(parts: Vec<(String, Arc<dyn PackSource>)>) -> Result<Self, PackError> {
        if parts.is_empty() {
            return Err(PackError::Mismatch("no pack parts".into()));
        }
        let sources: Vec<_> = if parts.len() > 1 {
            let mut ordered = Vec::with_capacity(parts.len());
            for (name, source) in parts {
                let (group, suffix) = split_name(&name)?;
                ordered.push((group.to_owned(), suffix, source));
            }
            ordered.sort_by_key(|(_, suffix, _)| *suffix);
            let group = &ordered[0].0;
            let common_len = ordered[0].2.len();
            if common_len == 0 {
                return Err(PackError::Mismatch("empty split part".into()));
            }
            for (expected, (name, suffix, source)) in ordered.iter().enumerate() {
                if name != group {
                    return Err(PackError::Mismatch(
                        "split parts belong to different files".into(),
                    ));
                }
                if *suffix != expected {
                    let missing = suffix_name(expected);
                    return Err(PackError::Mismatch(format!(
                        "duplicate part or missing part .zim{missing}"
                    )));
                }
                if expected + 1 < ordered.len() && source.len() != common_len {
                    return Err(PackError::Mismatch(
                        "non-last split parts must have equal lengths".into(),
                    ));
                }
            }
            ordered.into_iter().map(|(_, _, source)| source).collect()
        } else {
            parts.into_iter().map(|(_, source)| source).collect()
        };
        let mut result = Vec::with_capacity(sources.len());
        let mut len = 0_u64;
        for source in sources {
            let part_len = source.len();
            let next = len
                .checked_add(part_len)
                .ok_or_else(|| PackError::Mismatch("concatenated length overflow".into()))?;
            result.push(Part {
                source,
                start: len,
                len: part_len,
            });
            len = next;
        }
        Ok(Self {
            parts: result,
            len,
            removed: AtomicBool::new(false),
        })
    }
}

impl PackSource for ConcatSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::Acquire) {
            return Err(PackError::MediaRemoved);
        }
        check_range(self.len, off, buf.len())?;
        let mut filled = 0;
        let mut position = off;
        for part in &self.parts {
            if filled == buf.len() {
                break;
            }
            if position >= part.start + part.len {
                continue;
            }
            let local = position - part.start;
            let count = (part.len - local).min((buf.len() - filled) as u64) as usize;
            if let Err(error) =
                read_exact_at(&*part.source, local, &mut buf[filled..filled + count])
            {
                if error == PackError::MediaRemoved {
                    self.removed.store(true, Ordering::Release);
                }
                return Err(error);
            }
            position += count as u64;
            filled += count;
        }
        if self.removed.load(Ordering::Acquire) {
            return Err(PackError::MediaRemoved);
        }
        if filled != buf.len() {
            return Err(PackError::Truncated);
        }
        Ok(filled)
    }
}

fn read_exact_at(source: &dyn PackSource, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = source.read_at(off + filled as u64, &mut buf[filled..])?;
        if read == 0 || read > buf.len() - filled {
            return Err(PackError::Truncated);
        }
        filled += read;
    }
    Ok(filled)
}

fn split_name(name: &str) -> Result<(&str, usize), PackError> {
    let bytes = name.as_bytes();
    let split = bytes
        .len()
        .checked_sub(2)
        .ok_or_else(|| PackError::Mismatch("invalid split filename".into()))?;
    let suffix = &bytes[split..];
    if !suffix.iter().all(u8::is_ascii_lowercase)
        || !name
            .get(..split)
            .is_some_and(|group| group.ends_with(".zim"))
    {
        return Err(PackError::Mismatch(format!(
            "invalid split filename: {name}"
        )));
    }
    let group = name
        .get(..split)
        .ok_or_else(|| PackError::Mismatch("invalid split filename".into()))?;
    Ok((
        group,
        ((suffix[0] - b'a') as usize) * 26 + (suffix[1] - b'a') as usize,
    ))
}

fn suffix_name(index: usize) -> String {
    format!(
        "{}{}",
        (b'a' + (index / 26) as u8) as char,
        (b'a' + (index % 26) as u8) as char
    )
}
