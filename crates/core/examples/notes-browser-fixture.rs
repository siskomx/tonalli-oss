//! Explicit browser-test setup only; never a production CLI or HTTP create path.
use std::{fs, path::PathBuf};
use tonalli_core::notes::Notes;

fn initialize() -> Result<(), ()> {
    let mut arguments = std::env::args_os().skip(1);
    let directory = PathBuf::from(arguments.next().ok_or(())?);
    if arguments.next().is_some() {
        return Err(());
    }
    let metadata = fs::symlink_metadata(&directory).map_err(|_| ())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || fs::read_dir(&directory).map_err(|_| ())?.next().is_some()
    {
        return Err(());
    }
    let (notes, _recovery) =
        Notes::create(&directory.join("notes.db"), "tonalli-browser-test-only").map_err(|_| ())?;
    let id = notes
        .add(
            "Tonalli M1 synthetic sentinel",
            "TONALLI_M1_NOTES_SENTINEL_20261111",
        )
        .map_err(|_| ())?;
    let note = notes.get(id).map_err(|_| ())?;
    if note.title != "Tonalli M1 synthetic sentinel"
        || note.body != "TONALLI_M1_NOTES_SENTINEL_20261111"
    {
        return Err(());
    }
    Ok(())
}
fn main() {
    if initialize().is_err() {
        eprintln!("notes_fixture_setup_failed");
        std::process::exit(1);
    }
}
