use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tonalli_core::pack::{PackError, PackSource};

pub struct FaultySource {
    pub inner: Arc<dyn PackSource>,
    pub fail_after_bytes: u64,
    pub error_kind: PackError,
    consumed: AtomicU64,
}

impl FaultySource {
    pub fn new(inner: Arc<dyn PackSource>, fail_after_bytes: u64, error_kind: PackError) -> Self {
        Self {
            inner,
            fail_after_bytes,
            error_kind,
            consumed: AtomicU64::new(0),
        }
    }
}

impl PackSource for FaultySource {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        let consumed = self.consumed.load(Ordering::Relaxed);
        if consumed >= self.fail_after_bytes {
            return Err(self.error_kind.clone());
        }
        let allowed = buf.len().min((self.fail_after_bytes - consumed) as usize);
        let read = self.inner.read_at(off, &mut buf[..allowed])?;
        self.consumed.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}
