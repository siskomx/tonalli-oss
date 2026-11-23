use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use memmap2::{Mmap, MmapOptions};

use super::PackError;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::{FileExt, OpenOptionsExt};

/// A fixed-length pack view. Reads use explicit offsets and do not depend on a
/// shared cursor position. Windows `seek_read` may still update that cursor.
///
/// Requests outside the source return `Truncated`; an empty read at EOF returns 0.
/// Implementations may return a short read, so adapters must handle it.
pub trait PackSource: Send + Sync {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError>;
}

/// Read-only mapping for immutable, app-owned files on internal storage only.
///
/// The pack lifecycle must keep these files immutable while mapped (updates use
/// a new file). Never use this source for SD, USB, ISO extents or other removable
/// storage: operating-system removal/truncation can invalidate a mapping.
pub struct MmapSource {
    mapping: Option<Mmap>,
    _file: File,
    len: u64,
}

impl MmapSource {
    /// Opens an internal pack whose contents remain immutable for this lifetime.
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let mut options = OpenOptions::new();
        options.read(true);
        // Deny new writers/deleters while mapped on Windows, in addition to the
        // app's immutable-file invariant required on every platform.
        #[cfg(windows)]
        options.share_mode(1); // FILE_SHARE_READ
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(PackError::Mismatch("pack must be an ordinary file".into()));
        }
        let len = metadata.len();
        let mapping = if len == 0 {
            None
        } else {
            // SAFETY: this source is restricted to immutable app-owned internal
            // packs by the lifecycle contract above; the file stays owned here.
            Some(unsafe { MmapOptions::new().map(&file)? })
        };
        Ok(Self {
            mapping,
            _file: file,
            len,
        })
    }
}

impl PackSource for MmapSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        check_range(self.len, off, buf.len())?;
        if let Some(mapping) = &self.mapping {
            let start = usize::try_from(off).map_err(|_| PackError::Truncated)?;
            let end = start.checked_add(buf.len()).ok_or(PackError::Truncated)?;
            buf.copy_from_slice(mapping.get(start..end).ok_or(PackError::Truncated)?);
        }
        Ok(buf.len())
    }
}

/// Owned file handle read by position; safe against removable-media mmap faults.
/// Android callers transfer a detached descriptor into `File` before `from_file`.
pub struct PreadSource {
    file: File,
    len: u64,
}

impl PreadSource {
    pub fn open(path: &Path) -> Result<Self, PackError> {
        Self::from_file(File::open(path).map_err(source_error)?)
    }

    pub fn from_file(file: File) -> Result<Self, PackError> {
        let metadata = file.metadata().map_err(source_error)?;
        if !metadata.is_file() {
            return Err(PackError::Mismatch("pack must be an ordinary file".into()));
        }
        Ok(Self {
            file,
            len: metadata.len(),
        })
    }
}

impl PackSource for PreadSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        check_range(self.len, off, buf.len())?;
        let mut filled = 0;
        while filled < buf.len() {
            #[cfg(unix)]
            let result = self.file.read_at(&mut buf[filled..], off + filled as u64);
            #[cfg(windows)]
            let result = self.file.seek_read(&mut buf[filled..], off + filled as u64);
            match result {
                Ok(0) => return Err(PackError::Truncated),
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(source_error(error)),
            }
        }
        Ok(filled)
    }
}

pub(super) fn check_range(len: u64, off: u64, count: usize) -> Result<(), PackError> {
    match off.checked_add(count as u64) {
        Some(end) if end <= len => Ok(()),
        _ => Err(PackError::Truncated),
    }
}

fn source_error(error: io::Error) -> PackError {
    #[cfg(windows)]
    let removed = matches!(error.raw_os_error(), Some(21 | 1167 | 55));
    // EIO, ENXIO, ENODEV, ENOTCONN, EBADF. Other Unix platforms have different
    // errno assignments and must not inherit the Linux ENOTCONN value.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    let removed = matches!(error.raw_os_error(), Some(5 | 6 | 19 | 107 | 9));
    #[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
    let removed = false;
    if removed {
        PackError::MediaRemoved
    } else {
        PackError::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removable_device_errors_are_typed_and_other_errors_stay_io() {
        #[cfg(windows)]
        let codes = [21, 1167, 55];
        #[cfg(any(target_os = "android", target_os = "linux"))]
        let codes = [5, 6, 19, 107, 9];
        #[cfg(any(windows, target_os = "android", target_os = "linux"))]
        for code in codes {
            assert_eq!(
                source_error(io::Error::from_raw_os_error(code)),
                PackError::MediaRemoved
            );
        }
        assert!(matches!(
            source_error(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission"
            )),
            PackError::Io(_)
        ));
        assert!(matches!(
            source_error(io::Error::from_raw_os_error(12345)),
            PackError::Io(_)
        ));
    }
}
