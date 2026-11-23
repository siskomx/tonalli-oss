use std::{
    error::Error,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

static WEB_ZIP: &[u8] = include_bytes!("../web.zip");

/// The UI is embedded as one archive and unpacked into private app data, because Core serves a directory.
pub fn install(data: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root = data.join("ui");
    let target = root.join("web");
    let stamp = root.join("web.stamp");
    let id = format!("{:x}", Sha256::digest(WEB_ZIP));
    if fs::read_to_string(&stamp).ok().as_deref() == Some(id.as_str())
        && target.join("index.html").is_file()
    {
        return Ok(target);
    }
    fs::create_dir_all(&root)?;
    let staging = root.join("web.new");
    let _ = fs::remove_dir_all(&staging);
    zip::ZipArchive::new(Cursor::new(WEB_ZIP))?.extract(&staging)?;
    let _ = fs::remove_dir_all(&target);
    fs::rename(&staging, &target)?;
    fs::write(&stamp, id)?;
    Ok(target)
}
