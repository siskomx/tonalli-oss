use crate::pack::{PackError, PackSource, PreadSource};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// One content-ID lease covers every immutable version retained by any reader process.
pub(crate) struct LeasedSource {
    source: PreadSource,
    // Source closes before this lock file; lock files are never deleted/replaced.
    _lease: File,
}

pub(crate) fn file(path: &Path) -> Result<File, PackError> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?)
}

pub(crate) fn exclusive(file: &File) -> Result<bool, PackError> {
    match FileExt::try_lock_exclusive(file) {
        Ok(()) => Ok(true),
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(false),
        Err(e) => Err(e.into()),
    }
}

impl LeasedSource {
    pub fn open(path: &Path, lease: File) -> Result<Self, PackError> {
        FileExt::lock_shared(&lease)?;
        Ok(Self {
            source: PreadSource::open(path)?,
            _lease: lease,
        })
    }
}

impl PackSource for LeasedSource {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize, PackError> {
        self.source.read_at(offset, bytes)
    }
}
