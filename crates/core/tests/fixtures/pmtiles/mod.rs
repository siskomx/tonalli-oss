use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tonalli_core::pack::{PackError, PackSource};

pub const MVT: &[u8] = b"\x1a\x0e\x0a\x07fixture\x28\x80\x20\x78\x02";
pub const COMPRESSED_LIMIT: usize = 16 * 1024 * 1024;

pub fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Tuple fields are tile ID, run length, compressed length, section offset.
pub fn directory(entries: &[(u64, u64, u64, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    varint(entries.len() as u64, &mut out);
    let mut previous = 0;
    for &(id, _, _, _) in entries {
        varint(id - previous, &mut out);
        previous = id;
    }
    for column in 1..=3 {
        for &(_, run, length, offset) in entries {
            varint(
                match column {
                    1 => run,
                    2 => length,
                    _ => offset + 1,
                },
                &mut out,
            );
        }
    }
    out
}

/// Small independent gzip encoder using one uncompressed DEFLATE block.
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let length = u16::try_from(bytes.len()).unwrap();
    let mut out = vec![31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 1];
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&(!length).to_le_bytes());
    out.extend_from_slice(bytes);
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    out.extend_from_slice(&(!crc).to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out
}

pub fn set_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

pub fn archive(root: &[u8], leaves: &[u8], tiles: &[u8], internal: u8, tile_codec: u8) -> Vec<u8> {
    let metadata = br#"{"attribution":"OpenStreetMap contributors","vector_layers":[]}"#;
    let metadata = if internal == 2 {
        gzip(metadata)
    } else {
        metadata.to_vec()
    };
    let metadata_offset = 127 + root.len();
    let leaf_offset = metadata_offset + metadata.len();
    let data_offset = leaf_offset + leaves.len();
    let mut out = vec![0; 127];
    out[..8].copy_from_slice(b"PMTiles\x03");
    for (at, value) in [
        (8, 127),
        (16, root.len()),
        (24, metadata_offset),
        (32, metadata.len()),
        (40, leaf_offset),
        (48, leaves.len()),
        (56, data_offset),
        (64, tiles.len()),
        (72, 1),
        (80, 1),
        (88, 1),
    ] {
        set_u64(&mut out, at, value as u64);
    }
    out[96..102].copy_from_slice(&[1, internal, tile_codec, 1, 0, 8]);
    for (at, value) in [
        (102, -1_000_000_000_i32),
        (106, 100_000_000),
        (110, -990_000_000),
        (114, 200_000_000),
        (119, -995_000_000),
        (123, 150_000_000),
    ] {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    out[118] = 8;
    out.extend_from_slice(root);
    out.extend_from_slice(&metadata);
    out.extend_from_slice(leaves);
    out.extend_from_slice(tiles);
    out
}

pub fn simple() -> Vec<u8> {
    archive(&directory(&[(0, 1, MVT.len() as u64, 0)]), &[], MVT, 1, 1)
}

pub fn replace_metadata(bytes: Vec<u8>, metadata: &[u8]) -> Vec<u8> {
    let at = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    let old_len = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(metadata);
    out.extend_from_slice(&bytes[at + old_len..]);
    set_u64(&mut out, 32, metadata.len() as u64);
    for field in [40, 56] {
        let previous = u64::from_le_bytes(bytes[field..field + 8].try_into().unwrap());
        set_u64(
            &mut out,
            field,
            previous - old_len as u64 + metadata.len() as u64,
        );
    }
    out
}

/// A fixed-length, positioned source with controlled short reads and failures.
pub struct Source {
    pub bytes: Vec<u8>,
    pub length: u64,
    pub chunk: usize,
    pub removed: AtomicBool,
    pub largest_read: AtomicUsize,
    pub calls: AtomicUsize,
    pub zero_at: Option<u64>,
    pub failure: PackError,
    pub virtual_tile: Option<(u64, Vec<u8>)>,
}
impl Source {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            length: bytes.len() as u64,
            bytes,
            chunk: usize::MAX,
            removed: AtomicBool::new(false),
            largest_read: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            zero_at: None,
            failure: PackError::MediaRemoved,
            virtual_tile: None,
        }
    }
    pub fn shared(bytes: Vec<u8>) -> Arc<Self> {
        Arc::new(Self::new(bytes))
    }
}
impl PackSource for Source {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.largest_read.fetch_max(buf.len(), Ordering::Relaxed);
        if self.removed.load(Ordering::SeqCst) {
            return Err(self.failure.clone());
        }
        if off
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.length)
        {
            return Err(PackError::Truncated);
        }
        if self.zero_at.is_some_and(|at| off >= at) {
            return Ok(0);
        }
        let mut count = buf.len().min(self.chunk);
        if let Some(at) = self.zero_at {
            count = count.min(at.saturating_sub(off) as usize);
        }
        buf[..count].fill(0);
        let start = usize::try_from(off).map_err(|_| PackError::Truncated)?;
        if let Some(bytes) = self.bytes.get(start..) {
            let copy = count.min(bytes.len());
            buf[..copy].copy_from_slice(&bytes[..copy]);
        }
        if let Some((at, bytes)) = &self.virtual_tile {
            let begin = off.max(*at);
            let end = (off + count as u64).min(*at + bytes.len() as u64);
            if begin < end {
                let target = (begin - off) as usize;
                let source = (begin - at) as usize;
                let copy = (end - begin) as usize;
                buf[target..target + copy].copy_from_slice(&bytes[source..source + copy]);
            }
        }
        Ok(count)
    }
}
