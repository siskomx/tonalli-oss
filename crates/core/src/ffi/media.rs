use super::pack_boundary;
use crate::{
    media::{MediaError, MediaFile, MediaStore},
    pack::PackError,
};
#[cfg(unix)]
use std::sync::atomic::AtomicUsize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(uniffi::Object)]
pub struct MediaImportHandle {
    cancelled: AtomicBool,
    #[cfg(unix)]
    store: MediaStore,
    #[cfg(unix)]
    folder: String,
    #[cfg(unix)]
    remaining: AtomicUsize,
}
#[uniffi::export]
pub fn media_import_begin(
    root: String,
    ticket: String,
) -> Result<Arc<MediaImportHandle>, PackError> {
    pack_boundary(|| {
        #[cfg(unix)]
        {
            let store = MediaStore::open(Path::new(&root)).map_err(error)?;
            let folder = store.claim_native_import(&ticket).map_err(error)?;
            Ok(Arc::new(MediaImportHandle {
                store,
                folder,
                cancelled: AtomicBool::new(false),
                remaining: AtomicUsize::new(10000),
            }))
        }
        #[cfg(not(unix))]
        {
            let _ = (root, ticket);
            Err(PackError::Mismatch(
                "Native SAF imports require Android/Unix".into(),
            ))
        }
    })
}
#[uniffi::export]
pub fn media_import_cancel(handle: Arc<MediaImportHandle>) {
    handle.cancelled.store(true, Ordering::Release);
}
#[uniffi::export]
pub fn media_import_fd(
    handle: Arc<MediaImportHandle>,
    name: String,
    relative_folder: String,
    fd: i32,
) -> Result<MediaFile, PackError> {
    pack_boundary(|| {
        #[cfg(unix)]
        {
            use std::{fs::File, os::fd::FromRawFd};
            if fd < 0 {
                return Err(PackError::Mismatch("Invalid input descriptor".into()));
            }
            let file = unsafe { File::from_raw_fd(fd) };
            handle
                .remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                .map_err(|_| PackError::Mismatch("Import selection limit reached".into()))?;
            let folder = if relative_folder.is_empty() {
                handle.folder.clone()
            } else if handle.folder.is_empty() {
                relative_folder
            } else {
                format!("{}/{relative_folder}", handle.folder)
            };
            let source = Arc::new(crate::pack::PreadSource::from_file(file)?);
            handle
                .store
                .import_controlled(source, &name, &folder, || {
                    !handle.cancelled.load(Ordering::Acquire)
                })
                .map_err(error)
        }
        #[cfg(not(unix))]
        {
            let _ = (handle, name, relative_folder, fd);
            Err(PackError::Mismatch(
                "Detached input descriptors require Android/Unix".into(),
            ))
        }
    })
}

fn error(error: MediaError) -> PackError {
    match error {
        MediaError::Cancelled => PackError::Mismatch("Import stopped".into()),
        MediaError::Pack(error) => error,
        MediaError::NotFound => PackError::NotFound("Managed file".into()),
        MediaError::Invalid => PackError::Mismatch("Invalid managed file".into()),
        MediaError::Conflict => PackError::Mismatch("Managed file changed".into()),
        MediaError::Catalog => PackError::Corrupt("Media catalog unavailable".into()),
    }
}
#[uniffi::export]
pub fn media_file_info(root: String, id: String) -> Result<MediaFile, PackError> {
    pack_boundary(|| {
        MediaStore::open(Path::new(&root))
            .and_then(|store| store.get(&id))
            .map_err(error)
    })
}
/// Takes ownership of a descriptor supplied by the native Save-a-copy picker.
#[uniffi::export]
pub fn media_export_fd(root: String, id: String, fd: i32) -> Result<(), PackError> {
    pack_boundary(|| {
        #[cfg(unix)]
        {
            use std::{fs::File, os::fd::FromRawFd};
            if fd < 0 {
                return Err(PackError::Mismatch("Invalid output descriptor".into()));
            }
            let mut output = unsafe { File::from_raw_fd(fd) };
            MediaStore::open(Path::new(&root))
                .and_then(|store| store.export_to(&id, &mut output))
                .map_err(error)?;
            if output.metadata()?.is_file() {
                output.sync_all()?;
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (root, id, fd);
            Err(PackError::Mismatch(
                "Detached output descriptors require Android/Unix".into(),
            ))
        }
    })
}
