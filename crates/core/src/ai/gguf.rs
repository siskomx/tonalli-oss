//! Bounded structural metadata checks before native model/backend initialization.
//! M1 accepts one verified file. Native split loading needs a verified whole-pack
//! importer, leases and aggregate accounting before it can be enabled.

use std::collections::HashSet;

use crate::pack::{PackError, PackSource};

const MAX_METADATA_BYTES: u64 = 64 << 20;
const MAX_KEY_BYTES: u64 = 1 << 20;
const MAX_ARRAY_ITEMS: u64 = 1 << 20;

fn corrupt() -> PackError {
    PackError::Corrupt("invalid GGUF metadata".into())
}

pub(super) fn preflight(source: &dyn PackSource) -> Result<(), PackError> {
    let mut reader = Reader {
        source,
        offset: 0,
        buffer_start: 0,
        buffered: 0,
        buffer: [0; 64 * 1024],
    };
    let header = reader.array::<24>()?;
    if &header[..4] != b"GGUF" {
        return Err(corrupt());
    }
    let version = u32::from_le_bytes(header[4..8].try_into().map_err(|_| PackError::Truncated)?);
    if !matches!(version, 2 | 3) {
        return Err(PackError::Mismatch("unsupported GGUF version".into()));
    }
    let tensors = u64::from_le_bytes(header[8..16].try_into().map_err(|_| PackError::Truncated)?);
    let count = u64::from_le_bytes(
        header[16..24]
            .try_into()
            .map_err(|_| PackError::Truncated)?,
    );
    if tensors == 0 || tensors > 16_384 || count == 0 || count > 65_536 {
        return Err(corrupt());
    }
    let mut keys = HashSet::new();
    let mut key_bytes = 0_u64;
    for _ in 0..count {
        let length = reader.u64()?;
        if length == 0 || length > 1024 {
            return Err(corrupt());
        }
        key_bytes = key_bytes.checked_add(length).ok_or_else(corrupt)?;
        if key_bytes > MAX_KEY_BYTES {
            return Err(PackError::Mismatch(
                "GGUF metadata key budget exceeded".into(),
            ));
        }
        let mut key = vec![0; usize::try_from(length).map_err(|_| corrupt())?];
        reader.read(&mut key)?;
        // Native key lookup uses C strings. Never permit truncation/aliasing at NUL.
        if key.contains(&0) || std::str::from_utf8(&key).is_err() {
            return Err(corrupt());
        }
        let split_count = key == b"split.count";
        if !keys.insert(key) {
            return Err(corrupt());
        }
        let kind = reader.u32()?;
        if split_count {
            if kind != 2 {
                // GGUF_TYPE_UINT16, matching native get_key<uint16_t>.
                return Err(corrupt());
            }
            if u16::from_le_bytes(reader.array()?) > 1 {
                return Err(PackError::Mismatch(
                    "multi-file GGUF requires verified whole-pack installation".into(),
                ));
            }
        } else {
            reader.value(kind)?;
        }
    }
    // Metadata parsing must leave at least the minimum encoded tensor-info table.
    let minimum = tensors.checked_mul(24).ok_or(PackError::Truncated)?;
    if source.len().saturating_sub(reader.offset) < minimum {
        return Err(PackError::Truncated);
    }
    Ok(())
}

struct Reader<'a> {
    source: &'a dyn PackSource,
    offset: u64,
    buffer_start: u64,
    buffered: usize,
    buffer: [u8; 64 * 1024],
}

impl Reader<'_> {
    fn end(&self, count: u64) -> Result<u64, PackError> {
        let end = self.offset.checked_add(count).ok_or(PackError::Truncated)?;
        if end > self.source.len() {
            return Err(PackError::Truncated);
        }
        if end > MAX_METADATA_BYTES {
            return Err(PackError::Mismatch(
                "GGUF metadata scan budget exceeded".into(),
            ));
        }
        Ok(end)
    }

    fn skip(&mut self, count: u64) -> Result<(), PackError> {
        self.offset = self.end(count)?;
        Ok(())
    }

    fn read(&mut self, output: &mut [u8]) -> Result<(), PackError> {
        self.end(output.len() as u64)?;
        let mut filled = 0;
        while filled < output.len() {
            let index = usize::try_from(self.offset.saturating_sub(self.buffer_start))
                .map_err(|_| PackError::Truncated)?;
            if self.offset < self.buffer_start || index >= self.buffered {
                self.buffer_start = self.offset;
                let count = self.buffer.len().min(
                    usize::try_from(
                        (self.source.len() - self.offset).min(MAX_METADATA_BYTES - self.offset),
                    )
                    .map_err(|_| PackError::Truncated)?,
                );
                self.buffered = self
                    .source
                    .read_at(self.offset, &mut self.buffer[..count])?;
                if self.buffered == 0 || self.buffered > count {
                    return Err(PackError::Truncated);
                }
                continue;
            }
            let count = (output.len() - filled).min(self.buffered - index);
            output[filled..filled + count].copy_from_slice(&self.buffer[index..index + count]);
            self.offset += count as u64;
            filled += count;
        }
        Ok(())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PackError> {
        let mut bytes = [0; N];
        self.read(&mut bytes)?;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, PackError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, PackError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn string(&mut self) -> Result<(), PackError> {
        let length = self.u64()?;
        self.skip(length)
    }

    fn value(&mut self, kind: u32) -> Result<(), PackError> {
        if kind == 8 {
            return self.string();
        }
        if kind == 9 {
            let element = self.u32()?;
            let count = self.u64()?;
            if count > MAX_ARRAY_ITEMS {
                return Err(corrupt());
            }
            if element == 8 {
                for _ in 0..count {
                    self.string()?;
                }
                return Ok(());
            }
            // Arrays of arrays are not a GGUF value type.
            let bytes = width(element)
                .ok_or_else(corrupt)?
                .checked_mul(count)
                .ok_or(PackError::Truncated)?;
            return self.skip(bytes);
        }
        self.skip(width(kind).ok_or_else(corrupt)?)
    }
}

fn width(kind: u32) -> Option<u64> {
    match kind {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}
