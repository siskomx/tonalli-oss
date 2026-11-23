//! Owned Kotlin-facing handles. Closing a handle invalidates all its aliases.
#[cfg(feature = "ai")]
mod ai;
#[cfg(feature = "ai")]
pub use ai::{
    ai_answer, ai_cancel, ai_close, ai_embed, ai_generate, ai_info, ai_load, ai_rerank,
    server_attach_ai, server_detach_ai, AiHandle, AiInfo, AiKind, Answer, AnswerFlags,
    AnswerOutcome, AnswerRequest, AnswerSource, AnswerTimings, RerankHit,
};
mod content;
mod copies;
pub use copies::*;
mod preparation;
pub use preparation::*;
mod maps;
mod media;
mod music;
pub use maps::*;
pub use media::*;
pub use music::*;
mod lan;
mod sharing;
mod tv;
mod update;
pub use content::*;
pub use lan::{
    lan_devices, lan_info, lan_kick, lan_pairing_code, lan_room, server_start_lan, server_stop_lan,
    LanConfig, LanDevice, LanHandle, LanInfo,
};
pub use sharing::*;
pub use tv::*;
pub use update::{update_ready, UpdateReady};

use crate::{index, iso::IsoImage, notes, pack::*, server, zim};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(uniffi::Enum)]
pub enum RamTier {
    Lite,
    Standard,
    Reference,
}

#[uniffi::export]
pub fn ram_tier(total_ram_bytes: u64) -> Result<RamTier, PackError> {
    pack_boundary(|| {
        Ok(match crate::tier::tier_for(total_ram_bytes) {
            crate::tier::RamTier::Lite => RamTier::Lite,
            crate::tier::RamTier::Standard => RamTier::Standard,
            crate::tier::RamTier::Reference => RamTier::Reference,
        })
    })
}

fn pack_boundary<T>(f: impl FnOnce() -> Result<T, PackError>) -> Result<T, PackError> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(PackError::Corrupt("panic at FFI boundary".into())))
}
fn notes_boundary<T>(
    f: impl FnOnce() -> Result<T, notes::NotesError>,
) -> Result<T, notes::NotesError> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(notes::NotesError::Corrupt("panic at FFI boundary".into())))
}
#[derive(uniffi::Record)]
pub struct PackPart {
    pub name: String,
    pub fd: i32,
}
struct OpenPack {
    source: Arc<dyn PackSource>,
    zim: Option<Arc<zim::Zim>>,
}
#[derive(uniffi::Object)]
pub struct PackHandle {
    inner: Mutex<Option<OpenPack>>,
}
impl PackHandle {
    fn new(source: Arc<dyn PackSource>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Some(OpenPack { source, zim: None })),
        })
    }
    fn source(&self) -> Result<Arc<dyn PackSource>, PackError> {
        self.inner
            .lock()
            .map_err(|_| PackError::Corrupt("pack lock poisoned".into()))?
            .as_ref()
            .map(|p| p.source.clone())
            .ok_or_else(|| PackError::NotFound("closed pack".into()))
    }
    fn zim(&self) -> Result<Arc<zim::Zim>, PackError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| PackError::Corrupt("pack lock poisoned".into()))?;
        let p = guard
            .as_mut()
            .ok_or_else(|| PackError::NotFound("closed pack".into()))?;
        if p.zim.is_none() {
            p.zim = Some(Arc::new(zim::Zim::open(p.source.clone())?));
        }
        p.zim
            .clone()
            .ok_or_else(|| PackError::Corrupt("missing ZIM".into()))
    }
}

#[uniffi::export]
pub fn pack_open_fds(parts: Vec<PackPart>) -> Result<Arc<PackHandle>, PackError> {
    pack_boundary(|| {
        #[cfg(unix)]
        {
            use std::{collections::HashSet, fs::File, os::fd::FromRawFd};
            // Take each distinct detached descriptor before validating names or files.
            // Duplicate integers own only one OS descriptor and must never double-close.
            let mut seen = HashSet::new();
            let mut invalid = false;
            let files: Vec<_> = parts
                .into_iter()
                .filter_map(|p| {
                    if p.fd < 0 || !seen.insert(p.fd) {
                        invalid = true;
                        None
                    } else {
                        Some((p.name, unsafe { File::from_raw_fd(p.fd) }))
                    }
                })
                .collect();
            if invalid {
                return Err(PackError::Mismatch(
                    "invalid or duplicate detached descriptor".into(),
                ));
            }
            Ok(PackHandle::new(Arc::new(ConcatSource::from_files(files)?)))
        }
        #[cfg(not(unix))]
        {
            let _ = parts;
            Err(PackError::Mismatch(
                "detached descriptors require Unix/Android".into(),
            ))
        }
    })
}
/// Only immutable app-owned internal storage may use this mapping entry point.
#[uniffi::export]
pub fn pack_open_internal(path: String) -> Result<Arc<PackHandle>, PackError> {
    pack_boundary(|| {
        Ok(PackHandle::new(Arc::new(MmapSource::open(Path::new(
            &path,
        ))?)))
    })
}
#[uniffi::export]
pub fn pack_close(handle: Arc<PackHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        handle
            .inner
            .lock()
            .map_err(|_| PackError::Corrupt("pack lock poisoned".into()))?
            .take();
        Ok(())
    })
}
#[uniffi::export]
pub fn zim_attach_title_index(handle: Arc<PackHandle>, path: String) -> Result<(), PackError> {
    pack_boundary(|| {
        let mut guard = handle
            .inner
            .lock()
            .map_err(|_| PackError::Corrupt("pack lock poisoned".into()))?;
        let pack = guard
            .as_mut()
            .ok_or_else(|| PackError::NotFound("closed pack".into()))?;
        let replacement =
            zim::Zim::open(pack.source.clone())?.with_title_index(Path::new(&path))?;
        pack.zim = Some(Arc::new(replacement));
        Ok(())
    })
}

#[uniffi::export]
pub fn zim_identity(handle: Arc<PackHandle>) -> Result<ZimIdentity, PackError> {
    pack_boundary(|| reader_identity(handle.zim()?.as_ref()))
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ZimIdentity {
    pub uuid: String,
    pub stored_md5: String,
}

fn reader_identity(zim: &zim::Zim) -> Result<ZimIdentity, PackError> {
    let hex = |bytes: [u8; 16]| bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(ZimIdentity {
        uuid: hex(zim.uuid()),
        stored_md5: hex(zim.stored_md5()?),
    })
}

/// Check the archive's stored checksum before publishing a new native source reference.
#[uniffi::export]
pub fn zim_verify_identity(handle: Arc<PackHandle>) -> Result<ZimIdentity, PackError> {
    pack_boundary(|| {
        let zim = handle.zim()?;
        zim.verify_md5()?;
        reader_identity(zim.as_ref())
    })
}

#[uniffi::export]
pub fn zim_suggest(
    handle: Arc<PackHandle>,
    prefix: String,
    limit: u32,
) -> Result<Vec<zim::TitleHit>, PackError> {
    pack_boundary(|| handle.zim()?.suggest(&prefix, limit as usize))
}
#[uniffi::export]
pub fn zim_article(handle: Arc<PackHandle>, url: String) -> Result<zim::Article, PackError> {
    pack_boundary(|| handle.zim()?.article(&url))
}
#[uniffi::export]
pub fn iso_open_fd(part: PackPart, inner_path: String) -> Result<Arc<PackHandle>, PackError> {
    pack_boundary(|| {
        let image = pack_open_fds(vec![part])?;
        Ok(PackHandle::new(
            IsoImage::open(image.source()?)?.open_file(&inner_path)?,
        ))
    })
}
#[derive(uniffi::Object)]
pub struct IndexHandle {
    inner: Arc<index::Index>,
}
#[uniffi::export]
pub fn index_open(dir: String, zim: Arc<PackHandle>) -> Result<Arc<IndexHandle>, PackError> {
    pack_boundary(|| {
        Ok(Arc::new(IndexHandle {
            inner: Arc::new(index::Index::open(Path::new(&dir), zim.zim()?.as_ref())?),
        }))
    })
}
#[uniffi::export]
pub fn index_search(
    index: Arc<IndexHandle>,
    q: String,
    limit: u32,
) -> Result<Vec<index::Hit>, PackError> {
    pack_boundary(|| index.inner.search(&q, limit as usize))
}

#[derive(uniffi::Object)]
pub struct LoopbackHandle {
    server: Mutex<Option<server::LoopbackServer>>,
    registry: Arc<server::PackRegistry>,
}
#[uniffi::export]
pub fn server_start_loopback(
    data_dir: String,
    web_root: String,
) -> Result<Arc<LoopbackHandle>, PackError> {
    pack_boundary(|| {
        let registry = Arc::new(server::PackRegistry::default());
        let server =
            server::LoopbackServer::start(registry.clone(), data_dir.into(), web_root.into())?;
        Ok(Arc::new(LoopbackHandle {
            server: Mutex::new(Some(server)),
            registry,
        }))
    })
}
#[uniffi::export]
pub fn server_info(handle: Arc<LoopbackHandle>) -> Result<server::LoopbackInfo, PackError> {
    pack_boundary(|| {
        handle
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("server lock poisoned".into()))?
            .as_ref()
            .map(server::LoopbackServer::info)
            .ok_or_else(|| PackError::NotFound("stopped server".into()))
    })
}
#[uniffi::export]
pub fn server_stop_loopback(handle: Arc<LoopbackHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        if let Some(mut server) = handle
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("server lock poisoned".into()))?
            .take()
        {
            server.shutdown()?;
        }
        Ok(())
    })
}
#[uniffi::export]
pub fn server_register_zim(
    server: Arc<LoopbackHandle>,
    id: String,
    handle: Arc<PackHandle>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .registry
            .register_zim(server::PackId::new(id)?, handle.zim()?)
    })
}
#[uniffi::export]
pub fn server_register_tiles(
    server: Arc<LoopbackHandle>,
    id: String,
    handle: Arc<PackHandle>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .registry
            .register_tiles(server::PackId::new(id)?, handle.source()?)
    })
}
#[uniffi::export]
pub fn server_register_index(
    server: Arc<LoopbackHandle>,
    id: String,
    index: Arc<IndexHandle>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        let live = server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("server lock poisoned".into()))?;
        if live.is_none() {
            return Err(PackError::NotFound("stopped server".into()));
        }
        server
            .registry
            .attach_index(server::PackId::new(id)?, Arc::clone(&index.inner))
    })
}
#[uniffi::export]
pub fn server_unregister_pack(server: Arc<LoopbackHandle>, id: String) -> Result<(), PackError> {
    pack_boundary(|| server.registry.remove(&server::PackId::new(id)?))
}

#[derive(uniffi::Object)]
pub struct NotesHandle {
    inner: Mutex<notes::Notes>,
}
#[uniffi::export]
pub fn notes_create(path: String, passphrase: String) -> Result<String, notes::NotesError> {
    let passphrase = zeroize::Zeroizing::new(passphrase);
    notes_boundary(|| {
        let (_, key) = notes::Notes::create(Path::new(&path), &passphrase)?;
        Ok(key.to_string())
    })
}
#[uniffi::export]
pub fn notes_open_passphrase(
    path: String,
    passphrase: String,
) -> Result<Arc<NotesHandle>, notes::NotesError> {
    let passphrase = zeroize::Zeroizing::new(passphrase);
    notes_boundary(|| {
        Ok(Arc::new(NotesHandle {
            inner: Mutex::new(notes::Notes::open(
                Path::new(&path),
                notes::Unlock::Passphrase(&passphrase),
            )?),
        }))
    })
}
#[uniffi::export]
pub fn notes_open_recovery(
    path: String,
    recovery_key: String,
) -> Result<Arc<NotesHandle>, notes::NotesError> {
    let recovery_key = zeroize::Zeroizing::new(recovery_key);
    notes_boundary(|| {
        let key = recovery_key.parse()?;
        Ok(Arc::new(NotesHandle {
            inner: Mutex::new(notes::Notes::open(
                Path::new(&path),
                notes::Unlock::Recovery(&key),
            )?),
        }))
    })
}
#[uniffi::export]
pub fn notes_add(
    handle: Arc<NotesHandle>,
    title: String,
    body: String,
) -> Result<i64, notes::NotesError> {
    notes_boundary(|| {
        handle
            .inner
            .lock()
            .map_err(|_| notes::NotesError::Corrupt("notes lock poisoned".into()))?
            .add(&title, &body)
    })
}
#[uniffi::export]
pub fn notes_get(handle: Arc<NotesHandle>, id: i64) -> Result<notes::Note, notes::NotesError> {
    notes_boundary(|| {
        handle
            .inner
            .lock()
            .map_err(|_| notes::NotesError::Corrupt("notes lock poisoned".into()))?
            .get(id)
    })
}
#[uniffi::export]
pub fn notes_search(
    handle: Arc<NotesHandle>,
    q: String,
) -> Result<Vec<notes::Note>, notes::NotesError> {
    notes_boundary(|| {
        handle
            .inner
            .lock()
            .map_err(|_| notes::NotesError::Corrupt("notes lock poisoned".into()))?
            .search(&q)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panics_are_typed() {
        assert!(matches!(
            pack_boundary::<()>(|| panic!("injected")),
            Err(PackError::Corrupt(_))
        ));
        assert!(matches!(
            notes_boundary::<()>(|| panic!("injected")),
            Err(notes::NotesError::Corrupt(_))
        ));
    }
}
