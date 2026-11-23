// Adapted from zim 0.5.0: positioned sources and a bounded decoded-cluster cache.
use crate::errors::{Error, Result};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

pub const CACHE_BYTES: usize = 64 * 1024 * 1024;
pub trait ReadAt: Send + Sync {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> io::Result<usize>;
}
pub struct Store {
    source: Arc<dyn ReadAt>,
    cache: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
    decoding: Mutex<()>,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}
impl Store {
    pub fn from_source(source: Arc<dyn ReadAt>) -> Self {
        Self {
            source,
            cache: Mutex::new(VecDeque::new()),
            decoding: Mutex::new(()),
        }
    }
    pub fn len(&self) -> u64 {
        self.source.len()
    }
    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }
    pub fn slice(&self, start: u64, len: u64) -> Result<Cow<'_, [u8]>> {
        let end = start.checked_add(len).ok_or(Error::OutOfBounds)?;
        if end > self.len() {
            return Err(Error::OutOfBounds);
        }
        let size = usize::try_from(len)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| Error::ResourceLimit)?;
        bytes.resize(size, 0);
        self.reader(start, end).read_exact(&mut bytes)?;
        Ok(Cow::Owned(bytes))
    }
    pub fn slice_upto(&self, start: u64, len: u64) -> Result<Cow<'_, [u8]>> {
        if start > self.len() {
            return Err(Error::OutOfBounds);
        }
        self.slice(start, len.min(self.len() - start))
    }
    pub fn reader(&self, start: u64, end: u64) -> StoreReader<'_> {
        StoreReader {
            store: self,
            pos: start,
            end: end.min(self.len()),
        }
    }
    pub(crate) fn cached(&self, key: u64) -> Result<Option<Arc<[u8]>>> {
        let mut cache = self.cache.lock().map_err(|_| Error::Poisoned)?;
        if let Some(pos) = cache.iter().position(|(k, _)| *k == key) {
            let value = cache.remove(pos).ok_or(Error::OutOfBounds)?;
            let bytes = value.1.clone();
            cache.push_back(value);
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn decoder_permit(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.decoding.lock().map_err(|_| Error::Poisoned)
    }
    pub(crate) fn cache(&self, key: u64, bytes: Arc<[u8]>) -> Result<()> {
        let mut cache = self.cache.lock().map_err(|_| Error::Poisoned)?;
        if let Some(pos) = cache.iter().position(|(k, _)| *k == key) {
            cache.remove(pos);
        }
        while cache.iter().map(|(_, v)| v.len()).sum::<usize>() + bytes.len() > CACHE_BYTES {
            if cache.pop_front().is_none() {
                return Ok(());
            }
        }
        cache.push_back((key, bytes));
        Ok(())
    }
}
pub struct StoreReader<'a> {
    store: &'a Store,
    pos: u64,
    end: u64,
}
impl Read for StoreReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.end || buf.is_empty() {
            return Ok(0);
        }
        let count = (self.end - self.pos).min(buf.len() as u64) as usize;
        let read = self.store.source.read_at(self.pos, &mut buf[..count])?;
        if read == 0 || read > count {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.pos += read as u64;
        Ok(read)
    }
}
