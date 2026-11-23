// Adapted from zim 0.5.0: bounded decoder memory and ranged uncompressed blobs.
use crate::errors::{Error, Result};
use crate::store::{Store, CACHE_BYTES};
use std::borrow::Cow;
use std::io::{self, Read};
use std::sync::Arc;
const DECODER_MEMORY: u64 = 64 * 1024 * 1024;
#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Compression {
    None = 1,
    Lzma2 = 4,
    Zstd = 5,
}
impl From<Compression> for u8 {
    fn from(v: Compression) -> u8 {
        v as u8
    }
}
impl Compression {
    pub fn from(raw: u8) -> Result<Self> {
        match raw {
            0 | 1 => Ok(Self::None),
            4 => Ok(Self::Lzma2),
            5 => Ok(Self::Zstd),
            2 | 3 => Err(Error::UnsupportedCompression(raw)),
            _ => Err(Error::UnknownCompression(raw)),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Cluster<'a> {
    store: &'a Store,
    start: u64,
    end: u64,
    extended: bool,
    compression: Compression,
}
pub struct ClusterGuard<'g, 'a> {
    cluster: &'g Cluster<'a>,
    data: Option<Arc<[u8]>>,
}
impl<'a> Cluster<'a> {
    pub fn new(store: &'a Store, start: u64, end: u64, version: u16) -> Result<Self> {
        if end <= start || end > store.len() {
            return Err(Error::OutOfBounds);
        }
        let info = store.slice(start, 1)?[0];
        if info & 0xe0 != 0 {
            return Err(Error::InvalidClusterExtension);
        }
        let extended = info & 0x10 != 0;
        if extended && version != 6 {
            return Err(Error::InvalidClusterExtension);
        }
        Ok(Self {
            store,
            start,
            end,
            extended,
            compression: Compression::from(info & 15)?,
        })
    }
    pub fn compression(&self) -> Compression {
        self.compression
    }
    pub fn is_loaded(&self) -> bool {
        self.store.cached(self.start).ok().flatten().is_some()
    }
    pub fn decompress(&self) -> Result<()> {
        self.read()?.blob_count().map(|_| ())
    }
    fn decoder(&self) -> Result<Box<dyn Read + '_>> {
        let reader = self.store.reader(self.start + 1, self.end);
        match self.compression {
            Compression::None => Ok(Box::new(reader)),
            Compression::Lzma2 => {
                let stream = xz2::stream::Stream::new_stream_decoder(DECODER_MEMORY, 0)
                    .map_err(|e| Error::Parsing(Box::new(e)))?;
                Ok(Box::new(xz2::read::XzDecoder::new_stream(reader, stream)))
            }
            Compression::Zstd => {
                let mut decoder = zstd::stream::read::Decoder::new(reader)?;
                decoder.window_log_max(26)?;
                Ok(Box::new(decoder))
            }
        }
    }
    pub fn read(&self) -> Result<ClusterGuard<'_, 'a>> {
        let data = if self.compression == Compression::None {
            None
        } else if let Some(data) = self.store.cached(self.start)? {
            Some(data)
        } else {
            self.decode_cached()?
        };
        Ok(ClusterGuard {
            cluster: self,
            data,
        })
    }

    fn decode_cached(&self) -> Result<Option<Arc<[u8]>>> {
        // Bound decoder working memory across concurrent reads of this archive.
        let _permit = self.store.decoder_permit()?;
        if let Some(data) = self.store.cached(self.start)? {
            return Ok(Some(data));
        }
        let mut decoder = self.decoder()?;
        let mut data = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let read = decoder.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let required = data.len().checked_add(read).ok_or(Error::ResourceLimit)?;
            if required > CACHE_BYTES {
                return Ok(None);
            }
            if required > data.capacity() {
                let capacity = data
                    .capacity()
                    .max(buffer.len())
                    .saturating_mul(2)
                    .min(CACHE_BYTES);
                data.try_reserve_exact(capacity - data.len())
                    .map_err(|_| Error::ResourceLimit)?;
            }
            data.extend_from_slice(&buffer[..read]);
        }
        let data: Arc<[u8]> = data.into();
        self.store.cache(self.start, data.clone())?;
        Ok(Some(data))
    }
}
impl ClusterGuard<'_, '_> {
    fn raw(&self, offset: u64, count: usize) -> Result<Cow<'_, [u8]>> {
        let end = offset.checked_add(count as u64).ok_or(Error::OutOfBounds)?;
        if let Some(data) = &self.data {
            let start = usize::try_from(offset)?;
            let end = usize::try_from(end)?;
            return Ok(Cow::Borrowed(
                data.get(start..end).ok_or(Error::OutOfBounds)?,
            ));
        }
        if self.cluster.compression == Compression::None {
            if end > self.cluster.end - self.cluster.start - 1 {
                return Err(Error::OutOfBounds);
            }
            return self
                .cluster
                .store
                .slice(self.cluster.start + 1 + offset, count as u64);
        }
        // Large compressed clusters stream requested windows instead of occupying the cache.
        let _permit = self.cluster.store.decoder_permit()?;
        let mut decoder = self.cluster.decoder()?;
        if io::copy(&mut decoder.by_ref().take(offset), &mut io::sink())? != offset {
            return Err(Error::OutOfBounds);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(count)
            .map_err(|_| Error::ResourceLimit)?;
        bytes.resize(count, 0);
        decoder.read_exact(&mut bytes)?;
        Ok(Cow::Owned(bytes))
    }
    fn offset(&self, position: u64) -> Result<u64> {
        let width = if self.cluster.extended { 8 } else { 4 };
        let raw = self.raw(position, width)?;
        let mut bytes = [0; 8];
        bytes[..width].copy_from_slice(&raw);
        Ok(u64::from_le_bytes(bytes))
    }
    fn bounds(&self, index: u32) -> Result<(u64, u64)> {
        let width = if self.cluster.extended { 8 } else { 4 };
        let first = self.offset(0)?;
        if first < width || first % width != 0 {
            return Err(Error::InvalidBlobList);
        }
        let position = u64::from(index)
            .checked_mul(width)
            .ok_or(Error::OutOfBounds)?;
        if position + width >= first {
            return Err(Error::OutOfBounds);
        }
        let start = self.offset(position)?;
        let end = self.offset(position + width)?;
        if start < first || end < start {
            return Err(Error::InvalidBlobList);
        }
        if let Some(data) = &self.data {
            if end > data.len() as u64 {
                return Err(Error::OutOfBounds);
            }
        }
        if self.cluster.compression == Compression::None
            && end > self.cluster.end - self.cluster.start - 1
        {
            return Err(Error::OutOfBounds);
        }
        Ok((start, end))
    }
    pub fn blob_len(&self, index: u32) -> Result<u64> {
        let (start, end) = self.bounds(index)?;
        Ok(end - start)
    }
    pub fn blob_range(&self, index: u32, offset: u64, count: usize) -> Result<Cow<'_, [u8]>> {
        let (start, end) = self.bounds(index)?;
        if offset
            .checked_add(count as u64)
            .is_none_or(|n| n > end - start)
        {
            return Err(Error::OutOfBounds);
        }
        self.raw(start + offset, count)
    }
    pub fn blob(&self, index: u32) -> Result<Cow<'_, [u8]>> {
        let size = usize::try_from(self.blob_len(index)?)?;
        self.blob_range(index, 0, size)
    }
    pub fn blob_count(&self) -> Result<usize> {
        let width = if self.cluster.extended { 8 } else { 4 };
        let first = self.offset(0)?;
        if first < width || first % width != 0 {
            return Err(Error::InvalidBlobList);
        }
        Ok(usize::try_from(first / width - 1)?)
    }
}
