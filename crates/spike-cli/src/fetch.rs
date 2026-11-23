//! CLI manifest adapter to the shared Core download engine.
use crate::manifest::Pack;
use std::path::{Path, PathBuf};
use tonalli_core::{download, pack::PackError};

pub(crate) use download::check_space;

pub fn fetch_pack(pack: &Pack, base: &Path) -> Result<PathBuf, PackError> {
    download::fetch_pack(&pack.into(), base)
}

pub fn fetch_pack_with_space(
    pack: &Pack,
    base: &Path,
    available_space: impl Fn(&Path) -> Result<u64, PackError>,
) -> Result<PathBuf, PackError> {
    download::fetch_pack_with_space(&pack.into(), base, available_space)
}
