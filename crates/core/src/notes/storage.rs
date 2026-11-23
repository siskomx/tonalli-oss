use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use super::NotesError;

pub(super) fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub(super) fn keys_path(path: &Path) -> Result<PathBuf, NotesError> {
    if path.file_name().is_none()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("keys"))
    {
        return Err(NotesError::Corrupt(
            "database filename must differ from its .keys sidecar".into(),
        ));
    }
    Ok(path.with_extension("keys"))
}

pub(super) fn require_absent(path: &Path) -> Result<(), NotesError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(NotesError::AlreadyExists(path.display().to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn require_file(path: &Path) -> Result<(), NotesError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(NotesError::Corrupt(
            "notes path is not an ordinary file".into(),
        ));
    }
    Ok(())
}

pub(super) fn stage(path: &Path, bytes: &[u8]) -> Result<NamedTempFile, NotesError> {
    let mut staged = tempfile::Builder::new()
        .prefix(".notes-keys-")
        .tempfile_in(parent(path))?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    Ok(staged)
}

pub(super) fn publish_new(
    database_path: &Path,
    keys_path: &Path,
    mut database: NamedTempFile,
    keys: NamedTempFile,
) -> Result<(), NotesError> {
    keys.persist_noclobber(keys_path)
        .map_err(|error| NotesError::Io(error.error.to_string()))?;
    // Once keys are published, retain the staged encrypted DB even on an error.
    database.disable_cleanup(true);
    let mut recovery_path = database.path().to_owned();
    let result = (|| {
        sync_parent(keys_path)?;
        database
            .persist_noclobber(database_path)
            .map_err(|error| NotesError::Io(error.error.to_string()))?;
        recovery_path = database_path.to_owned();
        sync_parent(database_path)?;
        Ok::<(), NotesError>(())
    })();
    result.map_err(|error| NotesError::IncompleteCreate {
        database: database_path.display().to_string(),
        keys: keys_path.display().to_string(),
        staged_database: recovery_path.display().to_string(),
        reason: error.to_string(),
    })
}

pub(super) fn sync_parent(path: &Path) -> Result<(), NotesError> {
    #[cfg(unix)]
    fs::File::open(parent(path))?.sync_all()?;
    // Windows does not expose a portable directory fsync through std::fs.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{envelope, Notes, Unlock};

    #[test]
    fn failed_database_publication_keeps_keys_and_recoverable_encrypted_stage() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let (notes, recovery) = Notes::create(&source, "pass").unwrap();
        let id = notes.add("Retained", "encrypted staged data").unwrap();
        drop(notes);
        let key = envelope::unwrap(&source.with_extension("keys"), "pass").unwrap();
        let destination = dir.path().join("notes.db");
        let keys_path = keys_path(&destination).unwrap();
        let database = NamedTempFile::new_in(dir.path()).unwrap();
        fs::copy(&source, database.path()).unwrap();
        database.as_file().sync_all().unwrap();
        let keys = stage(&keys_path, &envelope::wrap("pass", &key).unwrap()).unwrap();
        // A competing file appears after create's initial existence checks.
        fs::write(&destination, b"existing destination must survive").unwrap();
        let error = publish_new(&destination, &keys_path, database, keys).unwrap_err();
        let NotesError::IncompleteCreate {
            staged_database, ..
        } = error
        else {
            panic!("wrong publication error");
        };
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"existing destination must survive"
        );
        assert!(envelope::unwrap(&keys_path, "pass").is_ok());
        let recovered =
            Notes::open(Path::new(&staged_database), Unlock::Recovery(&recovery)).unwrap();
        assert_eq!(recovered.get(id).unwrap().body, "encrypted staged data");
    }

    #[test]
    fn bare_relative_filename_stages_in_the_current_directory() {
        let staged = stage(Path::new("notes.keys"), b"bounded staging test").unwrap();
        assert_eq!(
            staged.path().parent().unwrap().canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }
}
