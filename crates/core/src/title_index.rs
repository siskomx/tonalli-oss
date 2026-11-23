//! Versioned folded-title side table, keyed to a ZIM's UUID and stored MD5.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::pack::{PackError, PackSource, PreadSource};

const MAGIC: &[u8; 8] = b"TONTITL\0";
const VERSION: u32 = 2;
const HEADER_LEN: u64 = 112;
// Increment when the NFD/mark-filter/lowercase algorithm itself changes.
const FOLD_ALGORITHM_REVISION: u32 = 1;
const MAX_KEY_BYTES: usize = 16 * 1024;
// The disposable builder sorts in RAM. This bounds accounted keys/vector storage,
// not allocator overhead or RSS; full-pack measurements must precede app adoption.
const MAX_BUILD_MEMORY: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleIdentity {
    pub zim_uuid: [u8; 16],
    pub zim_md5: [u8; 16],
}

pub struct FoldedTitleTable {
    source: PreadSource,
    count: u64,
    records_start: u64,
}

impl FoldedTitleTable {
    /// Builds sorted folded keys. IDs are title-list positions, not directory IDs.
    /// Iterator errors (including cancellation) leave any installed table intact.
    pub fn build(
        path: &Path,
        identity: TitleIdentity,
        entries: impl IntoIterator<Item = Result<(u32, String), PackError>>,
    ) -> Result<Self, PackError> {
        let mut rows: Vec<(String, u32)> = Vec::new();
        let mut key_bytes = 0_u64;
        for entry in entries {
            let (id, title) = entry?;
            check_key_size(title.len())?;
            let key = fold(&title);
            check_key_size(key.len())?;
            key_bytes = key_bytes
                .checked_add(key.capacity() as u64)
                .ok_or_else(build_limit)?;
            if rows.len() == rows.capacity() {
                let capacity = rows
                    .capacity()
                    .max(4)
                    .checked_mul(2)
                    .ok_or_else(build_limit)?;
                check_build_memory(capacity, key_bytes)?;
                rows.try_reserve_exact(capacity - rows.len())
                    .map_err(|_| build_limit())?;
            }
            check_build_memory(rows.capacity(), key_bytes)?;
            rows.push((key, id));
        }
        rows.sort_unstable();
        let count = rows.len() as u64;
        let records_start = records_start(count)?;
        let parent = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staged = NamedTempFile::new_in(parent)?;
        let mut digest = Sha256::new();
        let mut end = records_start;
        {
            let mut output = BufWriter::with_capacity(64 * 1024, staged.as_file_mut());
            output.write_all(&[0; HEADER_LEN as usize])?;
            for (key, _) in &rows {
                write_payload(&mut output, &mut digest, &end.to_le_bytes())?;
                end = end
                    .checked_add(8 + key.len() as u64)
                    .ok_or_else(|| corrupt("table length overflow"))?;
            }
            write_payload(&mut output, &mut digest, &end.to_le_bytes())?;
            for (key, id) in &rows {
                write_payload(&mut output, &mut digest, &id.to_le_bytes())?;
                write_payload(&mut output, &mut digest, &(key.len() as u32).to_le_bytes())?;
                write_payload(&mut output, &mut digest, key.as_bytes())?;
            }
            output.flush()?;
        }
        drop(rows);
        let mut header = [0; HEADER_LEN as usize];
        header[..8].copy_from_slice(MAGIC);
        header[8..12].copy_from_slice(&VERSION.to_le_bytes());
        header[12..16].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        header[16..32].copy_from_slice(&identity.zim_uuid);
        header[32..48].copy_from_slice(&identity.zim_md5);
        header[48..56].copy_from_slice(&count.to_le_bytes());
        header[56..64].copy_from_slice(&end.to_le_bytes());
        header[64..96].copy_from_slice(&digest.finalize());
        header[96..112].copy_from_slice(&folding_identity());
        staged.seek(SeekFrom::Start(0))?;
        staged.write_all(&header)?;
        staged.as_file().sync_all()?;
        // Verify the staged bytes before activation and retain their owned handle.
        // The lifecycle must close destination readers before replacement on Windows.
        let table = Self::open(staged.path(), &identity)?;
        staged
            .persist(path)
            .map_err(|error| PackError::Io(error.error.to_string()))?;
        Ok(table)
    }

    /// Verifies identity, structure and payload SHA-256 with a fixed 64 KiB buffer.
    /// This scans the file once but never loads the table into RAM.
    pub fn open(path: &Path, identity: &TitleIdentity) -> Result<Self, PackError> {
        let source = PreadSource::open(path)?;
        let mut header = [0; HEADER_LEN as usize];
        source.read_at(0, &mut header[..16])?;
        if &header[..8] != MAGIC {
            return Err(corrupt("invalid title-table magic"));
        }
        if read_u32(&header[8..12])? != VERSION || read_u32(&header[12..16])? != HEADER_LEN as u32 {
            return Err(PackError::Mismatch("unsupported title-table format".into()));
        }
        source.read_at(16, &mut header[16..])?;
        if header[96..112] != folding_identity() {
            return Err(PackError::Mismatch(
                "title-table folding algorithm or Unicode versions differ; rebuild explicitly"
                    .into(),
            ));
        }
        if header[16..32] != identity.zim_uuid || header[32..48] != identity.zim_md5 {
            return Err(PackError::Mismatch(
                "title table belongs to another ZIM".into(),
            ));
        }
        let count = read_u64(&header[48..56])?;
        let length = read_u64(&header[56..64])?;
        if source.len() < length {
            return Err(PackError::Truncated);
        }
        if source.len() != length {
            return Err(corrupt("unexpected table length"));
        }
        let records_start = records_start(count)?;
        if records_start > length {
            return Err(PackError::Truncated);
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut off = HEADER_LEN;
        while off < length {
            let count = (length - off).min(buffer.len() as u64) as usize;
            source.read_at(off, &mut buffer[..count])?;
            digest.update(&buffer[..count]);
            off += count as u64;
        }
        if digest.finalize()[..] != header[64..96] {
            return Err(PackError::Checksum);
        }
        let table = Self {
            source,
            count,
            records_start,
        };
        if table.offset(0)? != records_start || table.offset(count)? != length {
            return Err(corrupt("invalid record directory boundaries"));
        }
        Ok(table)
    }

    /// Returns title-list positions in folded-key order, breaking ties by position.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Result<Vec<u32>, PackError> {
        let prefix = prefix.trim();
        if prefix.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        check_key_size(prefix.len())?;
        let prefix = fold(prefix);
        check_key_size(prefix.len())?;
        if prefix.is_empty() {
            return Ok(Vec::new());
        }
        let mut low = 0;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            if self.record(middle)?.0 < prefix {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut result = Vec::new();
        while low < self.count && result.len() < limit {
            let (key, id) = self.record(low)?;
            if !key.starts_with(&prefix) {
                break;
            }
            result.push(id);
            low += 1;
        }
        Ok(result)
    }

    fn offset(&self, index: u64) -> Result<u64, PackError> {
        if index > self.count {
            return Err(corrupt("record directory index out of range"));
        }
        let mut bytes = [0; 8];
        self.source.read_at(HEADER_LEN + index * 8, &mut bytes)?;
        read_u64(&bytes)
    }

    fn record(&self, index: u64) -> Result<(String, u32), PackError> {
        if index >= self.count {
            return Err(corrupt("record index out of range"));
        }
        let start = self.offset(index)?;
        let end = self.offset(index + 1)?;
        let length = end
            .checked_sub(start)
            .ok_or_else(|| corrupt("record offsets are not ordered"))?;
        if start < self.records_start
            || end > self.source.len()
            || !(8..=8 + MAX_KEY_BYTES as u64).contains(&length)
        {
            return Err(corrupt("record range outside the table"));
        }
        let mut header = [0; 8];
        self.source.read_at(start, &mut header)?;
        let id = read_u32(&header[..4])?;
        let key_len = read_u32(&header[4..])? as usize;
        if key_len > MAX_KEY_BYTES || key_len as u64 != length - 8 {
            return Err(corrupt("invalid title-key length"));
        }
        let mut key = vec![0; key_len];
        self.source.read_at(start + 8, &mut key)?;
        let key = String::from_utf8(key).map_err(|_| corrupt("invalid title-key UTF-8"))?;
        Ok((key, id))
    }
}

fn fold(title: &str) -> String {
    title
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .collect()
}

fn folding_identity() -> [u8; 16] {
    let mut identity = [0; 16];
    identity[..4].copy_from_slice(&FOLD_ALGORITHM_REVISION.to_le_bytes());
    let (normalization_major, normalization_minor, normalization_patch) =
        unicode_normalization::UNICODE_VERSION;
    let (lowercase_major, lowercase_minor, lowercase_patch) = char::UNICODE_VERSION;
    for (slot, value) in identity[4..].chunks_exact_mut(2).zip([
        normalization_major,
        normalization_minor,
        normalization_patch,
        lowercase_major,
        lowercase_minor,
        lowercase_patch,
    ]) {
        slot.copy_from_slice(&u16::from(value).to_le_bytes());
    }
    identity
}

fn records_start(count: u64) -> Result<u64, PackError> {
    count
        .checked_add(1)
        .and_then(|count| count.checked_mul(8))
        .and_then(|bytes| HEADER_LEN.checked_add(bytes))
        .ok_or_else(|| corrupt("record directory length overflow"))
}

fn check_key_size(length: usize) -> Result<(), PackError> {
    if length > MAX_KEY_BYTES {
        return Err(PackError::Mismatch(
            "title key exceeds the 16 KiB spike limit".into(),
        ));
    }
    Ok(())
}

fn check_build_memory(capacity: usize, key_bytes: u64) -> Result<(), PackError> {
    let bytes = (capacity as u64)
        .checked_mul(std::mem::size_of::<(String, u32)>() as u64)
        .and_then(|bytes| bytes.checked_add(key_bytes))
        .ok_or_else(build_limit)?;
    if bytes > MAX_BUILD_MEMORY {
        return Err(build_limit());
    }
    Ok(())
}

fn build_limit() -> PackError {
    PackError::Mismatch("title-table build exceeds the 2 GiB spike memory budget".into())
}
fn corrupt(message: &str) -> PackError {
    PackError::Corrupt(message.into())
}

fn read_u32(bytes: &[u8]) -> Result<u32, PackError> {
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| corrupt("invalid u32 field"))?,
    ))
}

fn read_u64(bytes: &[u8]) -> Result<u64, PackError> {
    Ok(u64::from_le_bytes(
        bytes.try_into().map_err(|_| corrupt("invalid u64 field"))?,
    ))
}

fn write_payload(
    output: &mut BufWriter<&mut File>,
    digest: &mut Sha256,
    bytes: &[u8],
) -> Result<(), PackError> {
    output.write_all(bytes)?;
    digest.update(bytes);
    Ok(())
}
