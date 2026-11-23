use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tonalli_core::iso::IsoImage;
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::server::{LoopbackInfo, LoopbackServer, PackId, PackRegistry};
use tonalli_core::zim::Zim;
use tonalli_desktop::content::ReaderCatalog;
use tonalli_desktop::options::Options;

pub struct Session {
    registry: Arc<PackRegistry>,
    server: Mutex<Option<LoopbackServer>>,
    pub iso_inner: Option<String>,
    pub iso_busy: AtomicBool,
    pub closing: AtomicBool,
    pub content_busy: AtomicBool,
    pub unavailable_sources: usize,
    catalog: Mutex<Option<ReaderCatalog>>,
    reader_ids: Mutex<std::collections::HashSet<String>>,
    downloads: Option<Arc<tonalli_desktop::downloads::DesktopContentDispatcher>>,
    preparation: Option<Arc<tonalli_desktop::preparation::DesktopPreparation>>,
    maps: Option<Arc<tonalli_desktop::maps::DesktopMapDispatcher>>,
    maps_data: std::path::PathBuf,
    #[cfg(windows)]
    sharing: Arc<tonalli_desktop::sharing::DesktopSharing>,
    #[cfg(feature = "ai")]
    ai: Mutex<Option<Arc<tonalli_core::ai::AiHandle>>>,
}

pub struct FolderSummary {
    pub first_id: Option<String>,
    pub opened: usize,
    pub failed: usize,
    pub ignored: usize,
}

impl Session {
    pub fn start(options: &Options) -> Result<(Arc<Self>, LoopbackInfo), PackError> {
        let web = options.web.canonicalize()?;
        std::fs::create_dir_all(&options.profile)?;
        if options.profile.canonicalize()?.starts_with(&web) {
            return Err(PackError::Mismatch(
                "WebView profile must be outside the served web root".into(),
            ));
        }
        let registry = Arc::new(PackRegistry::default());
        for (id, path) in &options.packs {
            let id = PackId::new(id.clone())?;
            let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(path)?);
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if extension.eq_ignore_ascii_case("zim") {
                let mut zim = Zim::open(source)?;
                if let Some(index) = options.title_indexes.get(id.as_str()) {
                    zim = zim.with_title_index(index)?;
                }
                registry.register_zim(id, Arc::new(zim))?;
            } else if extension.eq_ignore_ascii_case("pmtiles")
                && !options.title_indexes.contains_key(id.as_str())
            {
                registry.register_tiles(id, source)?;
            } else {
                return Err(PackError::Mismatch(
                    "In-place packs require .zim/.pmtiles; title indexes require .zim".into(),
                ));
            }
        }
        let mut unavailable_sources = 0;
        let mut reader_ids = std::collections::HashSet::new();
        std::fs::create_dir_all(&options.data)?;
        let catalog = match ReaderCatalog::load(&options.data) {
            Ok(catalog) => {
                for entry in catalog.entries() {
                    match ReaderCatalog::open_entry(entry)
                        .and_then(|zim| registry.register_zim(PackId::new(entry.id.clone())?, zim))
                    {
                        Ok(()) => {
                            reader_ids.insert(entry.id.clone());
                        }
                        Err(_) => unavailable_sources += 1,
                    }
                }
                Some(catalog)
            }
            Err(_) => {
                unavailable_sources += 1;
                None
            }
        };
        let server = LoopbackServer::start(Arc::clone(&registry), options.data.clone(), web)?;
        #[cfg(feature = "ai")]
        server.configure_ai_model_root(&options.data)?;
        let downloads = match attach_downloads(&server, &options.data) {
            Ok(dispatcher) => Some(dispatcher),
            Err(_) => {
                eprintln!("Content downloads unavailable: trusted catalog or private content state unavailable; your library remains usable.");
                None
            }
        };
        #[cfg(feature = "ai")]
        let ai = if options.ai_model.is_some() {
            match verified_ai(options).and_then(|model| {
                server.attach_ai("qwen3-1.7b-q4-k-m".into(), Arc::clone(&model))?;
                Ok(model)
            }) {
                Ok(model) => Some(model),
                Err(_) => {
                    eprintln!("AI off: verified installed Qwen unavailable or current Core admission refused; normal offline tools remain available.");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(feature = "ai"))]
        if options.ai_model.is_some() {
            eprintln!("AI off: this build does not include the optional AI feature; normal offline tools remain available.");
        }
        let maps = attach_maps(&server, &options.data).ok();
        let preparation = attach_preparation(&server, &options.data, Arc::clone(&registry)).ok();
        #[cfg(windows)]
        let sharing = Arc::new(tonalli_desktop::sharing::DesktopSharing::new(
            Arc::clone(&registry),
            options.web.clone(),
            options.data.join("lan-identity"),
        ));
        #[cfg(windows)]
        server.attach_sharing(sharing.clone())?;
        #[cfg(target_os = "ios")]
        server.attach_sharing(Arc::new(tonalli_desktop::sharing::DesktopSharing::new(
            Arc::clone(&registry),
            options.web.clone(),
            options.data.join("lan-identity"),
        )))?;
        #[cfg(windows)]
        server.attach_tv(Arc::new(tonalli_desktop::tv::DesktopTv::default()))?;
        let info = server.info();
        Ok((
            Arc::new(Self {
                registry,
                server: Mutex::new(Some(server)),
                iso_inner: options.iso_inner.clone(),
                iso_busy: AtomicBool::new(false),
                closing: AtomicBool::new(false),
                content_busy: AtomicBool::new(false),
                unavailable_sources,
                catalog: Mutex::new(catalog),
                reader_ids: Mutex::new(reader_ids),
                downloads,
                preparation,
                maps,
                maps_data: options.data.join("country-maps"),
                #[cfg(windows)]
                sharing,
                #[cfg(feature = "ai")]
                ai: Mutex::new(ai),
            }),
            info,
        ))
    }

    pub fn open_content(&self, paths: Vec<std::path::PathBuf>) -> Result<String, PackError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("Desktop shell is closing".into()));
        }
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| PackError::Corrupt("Reader catalog lock poisoned".into()))?;
        let catalog = catalog.as_mut().ok_or_else(|| {
            PackError::Corrupt(
                "Saved content list is unavailable; existing files were preserved".into(),
            )
        })?;
        let (id, zim) = catalog.add_paths(paths)?;
        if self.closing.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("Desktop shell is closing".into()));
        }
        let mut registered = self
            .reader_ids
            .lock()
            .map_err(|_| PackError::Corrupt("Reader ownership lock poisoned".into()))?;
        if !registered.contains(&id) {
            self.registry.register_zim(PackId::new(id.clone())?, zim)?;
            registered.insert(id.clone());
        }
        Ok(id)
    }
    pub fn import_map(&self, path: &Path) -> Result<(), PackError> {
        let maps = tonalli_core::country_maps::CountryMaps::open(self.maps_data.clone())?;
        let title = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Map file")
            .to_owned();
        maps.import(Arc::new(PreadSource::open(path)?), title)
            .map(|_| ())
    }

    pub fn open_iso(&self, path: &Path) -> Result<(), PackError> {
        let inner = self.iso_inner.as_deref().ok_or_else(|| {
            PackError::Mismatch("M1 ISO dialog requires explicit --iso-inner /PATH.zim".into())
        })?;
        // All user-selected sources, including images on USB, use positioned reads.
        let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(path)?);
        let image = IsoImage::open(source)?;
        let zim = Arc::new(Zim::open(image.open_file(inner)?)?);
        zim.verify_md5()?;
        if self.closing.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("Desktop shell is closing".into()));
        }
        self.registry.register_zim(PackId::new("iso")?, zim)
    }

    pub fn open_folder(&self, folder: &Path) -> Result<FolderSummary, PackError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("Desktop shell is closing".into()));
        }
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| PackError::Corrupt("Reader catalog lock poisoned".into()))?;
        let result = catalog
            .as_mut()
            .ok_or_else(|| {
                PackError::Corrupt(
                    "Saved content list is unavailable; existing files were preserved".into(),
                )
            })?
            .add_folder(folder)?;
        let mut summary = FolderSummary {
            first_id: None,
            opened: 0,
            failed: result.failed,
            ignored: result.ignored,
        };
        let mut registered = self
            .reader_ids
            .lock()
            .map_err(|_| PackError::Corrupt("Reader ownership lock poisoned".into()))?;
        for (id, zim) in result.opened {
            if self.closing.load(Ordering::Acquire) {
                summary.failed += 1;
                continue;
            }
            if !registered.contains(&id)
                && self
                    .registry
                    .register_zim(PackId::new(id.clone())?, zim)
                    .is_err()
            {
                summary.failed += 1;
                continue;
            }
            registered.insert(id.clone());
            summary.opened += 1;
            summary.first_id.get_or_insert(id);
        }
        Ok(summary)
    }

    #[cfg(windows)]
    pub fn sharing(&self) -> &Arc<tonalli_desktop::sharing::DesktopSharing> {
        &self.sharing
    }

    pub fn shutdown(&self) -> Result<(), PackError> {
        self.closing.store(true, Ordering::Release);
        if let Some(downloads) = &self.downloads {
            downloads.shutdown();
        }
        if let Some(preparation) = &self.preparation {
            preparation.shutdown();
        }
        if let Some(maps) = &self.maps {
            maps.shutdown();
        }
        let server = self
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Desktop lifetime lock poisoned".into()))?
            .take();
        if let Some(mut server) = server {
            let result = server.shutdown(); // Detaches and requests cancellation before releasing model ownership.
            #[cfg(feature = "ai")]
            if let Some(model) = self
                .ai
                .lock()
                .map_err(|_| PackError::Corrupt("Desktop AI lifetime lock poisoned".into()))?
                .take()
            {
                model.close()?;
            }
            result?;
        }
        Ok(())
    }
}

fn attach_preparation(
    server: &LoopbackServer,
    data: &Path,
    registry: Arc<PackRegistry>,
) -> Result<Arc<tonalli_desktop::preparation::DesktopPreparation>, PackError> {
    let jobs = Arc::new(
        tonalli_core::index::jobs::IndexJobs::open(data)
            .map_err(|_| PackError::Mismatch("Search preparation storage unavailable".into()))?,
    );
    let owner = Arc::new(tonalli_desktop::preparation::DesktopPreparation::new(
        registry,
    ));
    owner
        .reconcile_startup(&jobs)
        .map_err(|_| PackError::Mismatch("Search preparation is owned by another task".into()))?;
    server.attach_preparation(jobs, owner.clone())?;
    Ok(owner)
}

fn attach_maps(
    server: &LoopbackServer,
    data: &Path,
) -> Result<Arc<tonalli_desktop::maps::DesktopMapDispatcher>, PackError> {
    let maps = Arc::new(tonalli_core::country_maps::CountryMaps::open(
        data.join("country-maps"),
    )?);
    maps.reconcile()?;
    let dispatcher = Arc::new(tonalli_desktop::maps::DesktopMapDispatcher::default());
    server.attach_country_maps(maps, dispatcher.clone())?;
    Ok(dispatcher)
}

fn attach_downloads(
    server: &LoopbackServer,
    data: &Path,
) -> Result<Arc<tonalli_desktop::downloads::DesktopContentDispatcher>, PackError> {
    use tonalli_core::{
        catalog::{Catalog, PUBLISHER_PUBLIC_KEY},
        content::{ContentJobs, ManagedStore},
        server::ContentDispatcher,
    };
    let store = Arc::new(ManagedStore::open(data, PUBLISHER_PUBLIC_KEY)?);
    let mut catalog = Catalog::bundled()?;
    let accepted = data.join("packs/managed/content.catalog");
    match std::fs::symlink_metadata(&accepted) {
        Ok(_) => {
            let saved = Catalog::load_accepted(&accepted, PUBLISHER_PUBLIC_KEY)?;
            if saved.revision() >= catalog.revision() {
                catalog = saved;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let dispatcher = Arc::new(tonalli_desktop::downloads::DesktopContentDispatcher::default());
    let catalog = Arc::new(catalog);
    let jobs = Arc::new(
        ContentJobs::open(Arc::clone(&store), Arc::clone(&catalog)).map_err(content_job_error)?,
    );
    let cleanup = dispatcher
        .reconcile_startup(&jobs)
        .map_err(content_job_error)?;
    server.attach_content_jobs(store, catalog, dispatcher.clone())?;
    if let Some(ticket) = cleanup {
        if let Err(error) = dispatcher.dispatch(Arc::clone(&jobs), ticket.clone()) {
            eprintln!(
                "content_worker stage=startup_cleanup_dispatch epoch={} error={error:?}",
                ticket.epoch
            );
            if jobs.dispatch_rejected(&ticket).is_err() {
                eprintln!(
                    "content_worker stage=startup_cleanup_rejection epoch={} error=unconfirmed",
                    ticket.epoch
                );
            }
        }
    }
    Ok(dispatcher)
}

fn content_job_error(error: tonalli_core::content::JobError) -> PackError {
    match error {
        tonalli_core::content::JobError::Pack(error) => error,
        _ => PackError::Corrupt("Content startup reconciliation unavailable".into()),
    }
}

#[cfg(feature = "ai")]
fn verified_ai(options: &Options) -> Result<Arc<tonalli_core::ai::AiHandle>, PackError> {
    const SHA: &str = "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5";
    // Explicit native --data-dir is the authority. Descendants may not be links,
    // and Windows model loading is limited to fixed local volumes.
    let data = options.data.canonicalize()?;
    #[cfg(windows)]
    tonalli_core::ai::require_internal_storage(&data)?;
    let mut path = data;
    for child in ["ai", "models", SHA, "model.gguf"] {
        path.push(child);
        let metadata = std::fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked || path.canonicalize()? != path {
            return Err(PackError::Mismatch("Linked AI installation refused".into()));
        }
    }
    if !path.is_file() || std::fs::metadata(&path)?.len() != 1_282_439_264 {
        return Err(PackError::Mismatch(
            "Installed AI model length mismatch".into(),
        ));
    }
    tonalli_core::pack::verify_sha256(&path, SHA)?;
    tonalli_core::ai::load_internal(&path, tonalli_core::ai::ModelKind::Generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonalli_core::iso::Extent;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/core/tests/fixtures")
            .join(name)
    }

    fn session() -> Session {
        Session {
            registry: Arc::new(PackRegistry::default()),
            server: Mutex::new(None),
            iso_inner: Some("/FIXTURE.ZIM".into()),
            iso_busy: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            content_busy: AtomicBool::new(false),
            unavailable_sources: 0,
            catalog: Mutex::new(None),
            reader_ids: Mutex::new(std::collections::HashSet::new()),
            downloads: None,
            preparation: None,
            maps: None,
            maps_data: std::path::PathBuf::new(),
            #[cfg(windows)]
            sharing: Arc::new(tonalli_desktop::sharing::DesktopSharing::new(
                Arc::new(PackRegistry::default()),
                std::path::PathBuf::new(),
                std::path::PathBuf::new(),
            )),
            #[cfg(feature = "ai")]
            ai: Mutex::new(None),
        }
    }

    fn install_kept(session: &Session) -> Arc<Zim> {
        let source: Arc<dyn PackSource> =
            Arc::new(PreadSource::open(&fixture("articles/benign-v1.zim")).unwrap());
        let kept = Arc::new(Zim::open(source).unwrap());
        session
            .registry
            .register_zim(PackId::new("kept").unwrap(), Arc::clone(&kept))
            .unwrap();
        kept
    }

    #[test]
    fn fresh_data_with_separate_profile_keeps_content_opening_available() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        std::fs::create_dir(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<html>reader</html>").unwrap();
        let options = Options::parse([
            "--web".to_owned(),
            web.to_str().unwrap().to_owned(),
            "--data-dir".to_owned(),
            temp.path().join("new-data").to_str().unwrap().to_owned(),
            "--profile-dir".to_owned(),
            temp.path().join("profile").to_str().unwrap().to_owned(),
        ])
        .unwrap();
        let (session, _) = Session::start(&options).unwrap();
        let opened = session.open_content(vec![fixture("articles/benign-v1.zim")]);
        session.shutdown().unwrap();
        assert!(opened.is_ok(), "fresh catalog unavailable: {opened:?}");
    }

    #[test]
    fn valid_iso_registers_its_explicit_embedded_zim() {
        let session = session();
        session.open_iso(&fixture("iso/plain-v1.iso")).unwrap();
        session
            .registry
            .remove(&PackId::new("iso").unwrap())
            .unwrap();
    }

    #[test]
    fn corrupt_inner_is_not_published_and_prior_pack_remains() {
        let original = fixture("iso/plain-v1.iso");
        let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&original).unwrap());
        let image = IsoImage::open(source).unwrap();
        let extents = image.find("/FIXTURE.ZIM").unwrap();
        let [Extent::Recorded { offset, len }] = extents.as_slice() else {
            panic!("plain fixture must have one recorded extent")
        };
        let temp = tempfile::tempdir().unwrap();
        for (name, position) in [("checksum", offset + len - 1), ("body", offset + len - 17)] {
            let session = session();
            let kept = install_kept(&session);
            let mut bytes = std::fs::read(&original).unwrap();
            bytes[position as usize] ^= 1;
            let path = temp.path().join(format!("bad-{name}.iso"));
            std::fs::write(&path, bytes).unwrap();
            assert!(
                session.open_iso(&path).is_err(),
                "corrupt embedded {name} was published"
            );
            assert!(session
                .registry
                .remove(&PackId::new("iso").unwrap())
                .is_err());
            session
                .registry
                .remove(&PackId::new("kept").unwrap())
                .unwrap();
            assert!(kept.title_count().unwrap() > 0);
        }
    }

    #[test]
    fn truncated_iso_preserves_the_registered_pack() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("truncated.iso");
        let bytes = std::fs::read(fixture("iso/plain-v1.iso")).unwrap();
        std::fs::write(&path, &bytes[..32768]).unwrap();
        let session = session();
        let kept = install_kept(&session);
        assert!(session.open_iso(&path).is_err());
        assert!(session
            .registry
            .remove(&PackId::new("iso").unwrap())
            .is_err());
        session
            .registry
            .remove(&PackId::new("kept").unwrap())
            .unwrap();
        assert!(kept.title_count().unwrap() > 0);
    }
}
