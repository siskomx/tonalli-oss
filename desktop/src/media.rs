//! Atomic copies to a native Save-a-copy destination; never overwrite app state or UI.
use std::{
    error::Error,
    path::{Path, PathBuf},
};
use tonalli_core::{download::ensure_regular_or_absent, media::MediaStore};

/// Where an iPhone "Save a copy" lands: Documents/Exports, which the Files app shows. Only the plain
/// file name is kept, so a stored name can never choose another folder.
pub fn exports_destination(documents: &Path, name: &str) -> Result<PathBuf, Box<dyn Error>> {
    // Checked on the original text: drive prefixes and backslashes parse differently per host.
    if name.contains([':', '\\']) {
        return Err("The file name cannot be used for a copy".into());
    }
    let leaf = Path::new(name)
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .filter(|leaf| !leaf.is_empty())
        .ok_or("The file name cannot be used for a copy")?;
    Ok(documents.join("Exports").join(leaf))
}

pub fn export_copy(
    data: &Path,
    web: &Path,
    id: &str,
    destination: &Path,
) -> Result<(), Box<dyn Error>> {
    let parent = destination
        .parent()
        .ok_or("Destination unavailable")?
        .canonicalize()?;
    let executable = std::env::current_exe()?;
    let bundle = executable
        .parent()
        .ok_or("Application folder unavailable")?
        .canonicalize()?;
    if parent.starts_with(data.canonicalize()?)
        || parent.starts_with(web.canonicalize()?)
        || parent.starts_with(bundle)
    {
        return Err("Choose a destination outside application folders".into());
    }
    ensure_regular_or_absent(destination)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    MediaStore::open(data)?.export_to(id, staged.as_file_mut())?;
    staged.as_file().sync_all()?;
    staged.persist(destination).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn iphone_copies_land_in_the_documents_exports_folder_by_plain_file_name() {
        let documents = Path::new("Documents");
        assert_eq!(
            exports_destination(documents, "Song.mp3").unwrap(),
            PathBuf::from("Documents").join("Exports").join("Song.mp3")
        );
        assert_eq!(
            exports_destination(documents, "folder/inner/Clip.mp4").unwrap(),
            PathBuf::from("Documents").join("Exports").join("Clip.mp4")
        );
    }

    #[test]
    fn unusable_names_never_choose_a_destination() {
        let documents = Path::new("Documents");
        for name in ["", ".", "..", "dir/..", "C:evil.txt", "a:b", "/"] {
            assert!(exports_destination(documents, name).is_err(), "{name:?}");
        }
    }
}
