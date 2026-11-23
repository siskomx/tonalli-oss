//! Owned loopback app host with authenticated installed-pack range access.
mod admin;
#[cfg(feature = "ai")]
mod ai;
mod assets;
mod auth;
mod books;
mod content;
mod files;
mod index_transfer;
pub(crate) mod lan;
mod library;
mod managed;
mod map_management;
mod maps;
mod media_stream;
mod music;
pub use music::MusicControl;
mod chat_storage;
mod connections;
mod copies;
#[cfg(feature = "mesh")]
mod mesh;
mod notes;
mod preparation;
mod ranges;
mod reading;
mod remote;
pub use remote::copy::{CopyDispatcher, CopyOwner, CopyRunState};
mod room;
mod search;
pub use preparation::PreparationDispatcher;
mod shared_media;
mod synced;
mod transfer;
mod update;
mod tv;
mod tv_controls;
pub use tv::{TvBrowse, TvCatalog, TvError, TvPage, TvServer, TvService};
pub use tv_controls::{TvControl, TvStatus};
mod sharing;
pub use shared_media::SharedMedia;
pub use sharing::{SharingControl, SharingDevice, SharingHotspot, SharingNetwork, SharingStatus};
#[cfg(test)]
mod search_tests;
mod workshop;
mod zim;

pub use content::{ContentDispatchError, ContentDispatcher};
pub use lan::{Device, Server, ServerCfg, ServerHandle};

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use axum::{
    middleware,
    routing::{get, post, put},
    Router,
};
use tokio::sync::{oneshot, Notify};

use crate::pack::{PackError, PackSource};
use crate::tiles::{TileMetadata, TileSource};
use crate::zim::Zim;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackId(String);
impl PackId {
    pub fn new(value: impl Into<String>) -> Result<Self, PackError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(PackError::Mismatch(
                "pack ID must contain 1–128 ASCII letters, digits, hyphens or underscores".into(),
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Default)]
pub struct PackRegistry {
    zims: RwLock<HashMap<PackId, Arc<Zim>>>,
    tiles: RwLock<HashMap<PackId, RegisteredTiles>>,
    indexes: RwLock<HashMap<PackId, Arc<crate::index::Index>>>,
    managed: Mutex<HashMap<PackId, managed::Origin>>,
    copied: Mutex<HashMap<PackId, PathBuf>>,
}
struct RegisteredTiles {
    source: Arc<dyn PackSource>,
    metadata: TileMetadata,
}
impl PackRegistry {
    /// Attach only a validated index to the exact currently registered source owner.
    pub fn attach_index(
        &self,
        id: PackId,
        index: Arc<crate::index::Index>,
    ) -> Result<(), PackError> {
        let owner = {
            let zims = self.zims.read().map_err(|_| registry_poisoned())?;
            Arc::clone(
                zims.get(&id)
                    .ok_or_else(|| PackError::NotFound(id.as_str().into()))?,
            )
        };
        index.check_zim(&owner)?;
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        let zims = self.zims.read().map_err(|_| registry_poisoned())?;
        if !zims
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, &owner))
        {
            return Err(PackError::Mismatch(
                "Registered source changed during index validation".into(),
            ));
        }
        let previous = self
            .indexes
            .write()
            .map_err(|_| registry_poisoned())?
            .insert(id, index);
        drop(zims);
        drop(managed);
        drop(previous);
        Ok(())
    }
    pub fn register_zim(&self, id: PackId, zim: Arc<Zim>) -> Result<(), PackError> {
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        if managed.contains_key(&id) {
            return Err(PackError::Mismatch("Managed pack ID is reserved".into()));
        }
        let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
        let tiles = self.tiles.read().map_err(|_| registry_poisoned())?;
        if zims.contains_key(&id) || tiles.contains_key(&id) {
            return Err(PackError::Mismatch("pack ID is already registered".into()));
        }
        zims.insert(id, zim);
        Ok(())
    }
    pub fn register_tiles(&self, id: PackId, source: Arc<dyn PackSource>) -> Result<(), PackError> {
        let metadata = TileSource::open(Arc::clone(&source))?.metadata().clone();
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        if managed.contains_key(&id) {
            return Err(PackError::Mismatch("Managed pack ID is reserved".into()));
        }
        let zims = self.zims.read().map_err(|_| registry_poisoned())?;
        let mut tiles = self.tiles.write().map_err(|_| registry_poisoned())?;
        if zims.contains_key(&id) || tiles.contains_key(&id) {
            return Err(PackError::Mismatch("pack ID is already registered".into()));
        }
        tiles.insert(id, RegisteredTiles { source, metadata });
        Ok(())
    }
    pub fn remove(&self, id: &PackId) -> Result<(), PackError> {
        let mut copied = self.copied.lock().map_err(|_| registry_poisoned())?;
        let managed = self.managed.lock().map_err(|_| registry_poisoned())?;
        if managed.contains_key(id) {
            return Err(PackError::Mismatch(
                "Managed pack requires managed removal".into(),
            ));
        }
        let mut zims = self.zims.write().map_err(|_| registry_poisoned())?;
        let mut tiles = self.tiles.write().map_err(|_| registry_poisoned())?;
        let mut indexes = self.indexes.write().map_err(|_| registry_poisoned())?;
        let removed_zim = zims.remove(id);
        let removed_tiles = tiles.remove(id);
        if removed_zim.is_none() && removed_tiles.is_none() {
            return Err(PackError::NotFound(id.as_str().into()));
        }
        let removed_index = indexes.remove(id);
        copied.remove(id);
        drop(indexes);
        drop(tiles);
        drop(zims);
        drop(managed);
        drop((removed_zim, removed_tiles, removed_index));
        Ok(())
    }
}
fn registry_poisoned() -> PackError {
    PackError::Corrupt("pack registry lock poisoned".into())
}

#[derive(Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LoopbackInfo {
    pub port: u16,
    pub token: String,
    pub launch_url: String,
}

struct ServerState {
    auth: Mutex<auth::Auth>,
    origin: String,
    host: String,
    web_root: PathBuf,
    data_dir: PathBuf,
    notes: notes::Vault,
    remote: remote::Connection,
    chat: connections::Connection,
    #[cfg(feature = "mesh")]
    mesh: mesh::Owner,
    admin: admin::Admin,
    update: update::Update,
    music: music::Music,
    streams: media_stream::Grants,
    books: books::Grants,
    model_slots: Arc<tokio::sync::Semaphore>,
    registry: Arc<PackRegistry>,
    content: Mutex<Option<Arc<content::Manager>>>,
    preparation: Mutex<Option<Arc<preparation::Manager>>>,
    maps: Mutex<Option<Arc<map_management::Manager>>>,
    sharing: Mutex<Option<Arc<dyn SharingControl>>>,
    shared_selection: Mutex<Vec<crate::media::MediaFile>>,
    // The running share's media snapshot and the books/maps it started with, so items can be removed live.
    shared_live: Mutex<Option<(Arc<SharedMedia>, Vec<String>)>>,
    tv: Mutex<Option<Arc<dyn TvControl>>>,
    tv_selection: Mutex<Vec<crate::media::MediaFile>>,
    #[cfg(feature = "ai")]
    ai: Arc<ai::ChatState>,
}

struct ContentState {
    registry: Arc<PackRegistry>,
    allowed: Option<HashSet<PackId>>,
    web_root: PathBuf,
    media: Arc<SharedMedia>,
}

impl ContentState {
    fn allows(&self, id: &PackId) -> bool {
        self.allowed
            .as_ref()
            .is_none_or(|allowed| allowed.contains(id) && !self.media.pack_hidden(id))
    }
}

fn content_router(state: Arc<ContentState>) -> Router {
    Router::new()
        .route("/api/shared-media", get(shared_media::listing))
        .route("/api/shared-media/{id}/text", get(shared_media::text))
        .route("/shared-media/{id}/content", get(shared_media::content))
        .route("/shared-media/{id}/download", get(shared_media::download))
        .route("/api/packs", get(library::listing))
        .route("/api/pack-transfer/{id}", get(transfer::manifest))
        .route("/transfer/{id}/pack", get(transfer::pack))
        .route("/api/index-transfer/{id}", get(index_transfer::manifest))
        .route(
            "/transfer/{id}/search/{generation}/{name}",
            get(index_transfer::file),
        )
        .route("/api/zim/titles", get(zim::titles))
        .route("/api/zim/suggest", get(zim::suggest))
        .route("/api/zim/search", get(search::search))
        .route("/api/zim/article", get(zim::article_info))
        .route("/zim/{id}/{*path}", get(zim::content))
        .route("/api/maps", get(maps::listing))
        .route("/packs/{id}", get(ranges::serve))
        .fallback(assets::serve)
        .with_state(state)
}

pub struct LoopbackServer {
    state: Arc<ServerState>,
    port: u16,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<Result<(), PackError>>>,
}

impl LoopbackServer {
    pub fn attach_copy_owner(&self, owner: &CopyOwner) -> Result<(), PackError> {
        if self.worker.is_none() || owner.root != self.state.data_dir {
            return Err(PackError::Mismatch(
                "Copy owner belongs to a different library".into(),
            ));
        }
        self.state.remote.attach_copies(Arc::clone(&owner.copies))?;
        owner.copies.bind(&self.state.registry)?;
        owner
            .copies
            .restore(&self.state.registry, &self.state.data_dir);
        Ok(())
    }
    /// Attach only a native owner for this private host's data root.
    pub fn attach_preparation(
        &self,
        jobs: Arc<crate::index::jobs::IndexJobs>,
        dispatcher: Arc<dyn PreparationDispatcher>,
    ) -> Result<(), PackError> {
        if self.worker.is_none() || !jobs.belongs_to(&self.state.data_dir) {
            return Err(PackError::Mismatch(
                "Preparation storage must belong to this active private host".into(),
            ));
        }
        let mut slot = self
            .state
            .preparation
            .lock()
            .map_err(|_| registry_poisoned())?;
        if slot.is_some() {
            return Err(PackError::Mismatch(
                "Preparation owner already attached".into(),
            ));
        }
        let manager = Arc::new(preparation::Manager::new(
            jobs,
            dispatcher,
            Arc::clone(&self.state.registry),
        ));
        manager.restore()?;
        *slot = Some(manager);
        Ok(())
    }
    pub fn attach_tv(&self, control: Arc<dyn TvControl>) -> Result<(), PackError> {
        let mut slot = self.state.tv.lock().map_err(|_| registry_poisoned())?;
        if slot.is_some() {
            return Err(PackError::Mismatch("TV owner already attached".into()));
        }
        *slot = Some(control);
        Ok(())
    }
    pub fn attach_sharing(&self, control: Arc<dyn SharingControl>) -> Result<(), PackError> {
        *self.state.sharing.lock().map_err(|_| registry_poisoned())? = Some(control);
        Ok(())
    }
    pub fn attach_country_maps(
        &self,
        maps: Arc<crate::country_maps::CountryMaps>,
        dispatcher: Arc<dyn crate::country_maps::MapDispatcher>,
    ) -> Result<(), PackError> {
        let manager = Arc::new(map_management::Manager {
            maps,
            dispatcher,
            registry: Arc::clone(&self.state.registry),
        });
        manager.sync()?;
        *self.state.maps.lock().map_err(|_| registry_poisoned())? = Some(manager);
        Ok(())
    }
    pub fn start(
        registry: Arc<PackRegistry>,
        data_dir: PathBuf,
        web_root: PathBuf,
    ) -> Result<Self, PackError> {
        let web_root = web_root.canonicalize()?;
        if !web_root.is_dir() || !web_root.join("index.html").is_file() {
            return Err(PackError::Mismatch(
                "web root must contain a built index.html".into(),
            ));
        }
        std::fs::create_dir_all(&data_dir)?;
        let data_dir = data_dir.canonicalize()?;
        if data_dir.starts_with(&web_root) {
            return Err(PackError::Mismatch(
                "private data directory must be outside the served web root".into(),
            ));
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let host = format!("127.0.0.1:{port}");
        let state = Arc::new(ServerState {
            auth: Mutex::new(auth::Auth::new()?),
            origin: format!("http://{host}"),
            host,
            web_root,
            data_dir,
            notes: notes::Vault::default(),
            remote: remote::Connection::default(),
            chat: connections::Connection::default(),
            #[cfg(feature = "mesh")]
            mesh: mesh::Owner::default(),
            admin: admin::Admin::default(),
            update: update::Update::default(),
            music: music::Music::default(),
            streams: media_stream::Grants::default(),
            books: books::Grants::default(),
            model_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            registry,
            content: Mutex::new(None),
            preparation: Mutex::new(None),
            maps: Mutex::new(None),
            sharing: Mutex::new(None),
            shared_selection: Mutex::new(Vec::new()),
            shared_live: Mutex::new(None),
            tv: Mutex::new(None),
            tv_selection: Mutex::new(Vec::new()),
            #[cfg(feature = "ai")]
            ai: Arc::new(ai::ChatState::default()),
        });
        if let Ok(copies) = state.remote.copies() {
            copies.bind(&state.registry)?;
            copies.restore(&state.registry, &state.data_dir);
        }
        let router = router(Arc::clone(&state));
        let (stop, stopped) = oneshot::channel();
        let (ready, readiness) = std::sync::mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("tonalli-loopback".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(4)
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let error = PackError::Io(error.to_string());
                        let _ = ready.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let result = runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(listener) => listener,
                        Err(error) => {
                            let error = PackError::Io(error.to_string());
                            let _ = ready.send(Err(error.clone()));
                            return Err(error);
                        }
                    };
                    let _ = ready.send(Ok(()));
                    run(listener, router, stopped).await
                });
                runtime.shutdown_timeout(Duration::from_secs(1));
                result
            })?;
        match readiness.recv() {
            Ok(Ok(())) => Ok(Self {
                state,
                port,
                stop: Some(stop),
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(PackError::Io(
                    "loopback runtime stopped before startup".into(),
                ))
            }
        }
    }

    /// The launch URL is single-use; consumption does not make `info()` refresh it.
    /// Reuse authenticated root cookies or rotate for a fresh UI launch.
    pub fn info(&self) -> LoopbackInfo {
        // This is diagnostic output only; poisoned auth locks fail closed in handlers.
        let auth = self
            .state
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.info_with_token(auth.token().into())
    }
    fn info_with_token(&self, token: String) -> LoopbackInfo {
        LoopbackInfo {
            port: self.port,
            launch_url: format!("{}/launch?token={token}", self.state.origin),
            token,
        }
    }
    pub fn rotate_token(&self) -> Result<LoopbackInfo, PackError> {
        let fresh = auth::Auth::new()?;
        let info = self.info_with_token(fresh.token().into());
        let mut auth = self
            .state
            .auth
            .lock()
            .map_err(|_| PackError::Corrupt("loopback auth lock poisoned".into()))?;
        *auth = fresh;
        self.state.notes.revoke();
        drop(auth);
        #[cfg(feature = "ai")]
        self.state.ai.cancel_active()?;
        Ok(info)
    }
    /// Attach native-owned managed storage independently of ordinary reader startup.
    /// This authenticates and durably accepts catalog metadata; call from a native worker.
    pub fn attach_content(
        &self,
        store: Arc<crate::content::ManagedStore>,
        catalog: Arc<crate::catalog::Catalog>,
    ) -> Result<(), PackError> {
        if self.worker.is_none() || store.root() != self.state.data_dir.as_path() {
            return Err(PackError::Mismatch(
                "Content storage must belong to this active private host".into(),
            ));
        }
        let mut content = self
            .state
            .content
            .lock()
            .map_err(|_| PackError::Corrupt("Content attachment lock poisoned".into()))?;
        if content.is_some() {
            return Err(PackError::Mismatch(
                "Content management is already attached".into(),
            ));
        }
        *content = Some(Arc::new(content::Manager::new(
            store,
            catalog,
            Arc::clone(&self.state.registry),
        )?));
        Ok(())
    }
    /// Native opt-in only. The dispatcher owns permitted OS/network scheduling; HTTP
    /// receives neither transport options nor filesystem authority and never executes work.
    pub fn attach_content_jobs(
        &self,
        store: Arc<crate::content::ManagedStore>,
        catalog: Arc<crate::catalog::Catalog>,
        dispatcher: Arc<dyn ContentDispatcher>,
    ) -> Result<(), PackError> {
        if self.worker.is_none() || store.root() != self.state.data_dir.as_path() {
            return Err(PackError::Mismatch(
                "Content storage must belong to this active private host".into(),
            ));
        }
        let mut content = self
            .state
            .content
            .lock()
            .map_err(|_| PackError::Corrupt("Content attachment lock poisoned".into()))?;
        if content.is_some() {
            return Err(PackError::Mismatch(
                "Content management is already attached".into(),
            ));
        }
        *content = Some(Arc::new(content::Manager::with_jobs(
            store,
            catalog,
            dispatcher,
            Arc::clone(&self.state.registry),
        )?));
        Ok(())
    }
    /// Attach a caller-verified internal generation model to this private host.
    /// The caller retains model-close ownership; no paths are accepted from HTTP.
    #[cfg(feature = "ai")]
    pub fn attach_ai(
        &self,
        model_id: String,
        model: Arc<crate::ai::AiHandle>,
    ) -> Result<(), PackError> {
        self.state.ai.attach(model_id, model)
    }
    /// The native shell supplies its actual private files root, never a web input.
    #[cfg(feature = "ai")]
    pub fn configure_ai_model_root(&self, root: &std::path::Path) -> Result<(), PackError> {
        self.state.ai.configure_model_root(root)
    }
    /// Request cancellation and remove this host's model attachment. Does not
    /// wait for native cleanup or close other aliases of the model.
    #[cfg(feature = "ai")]
    pub fn detach_ai(&self) -> Result<(), PackError> {
        self.state.ai.detach(false)
    }
    pub fn attach_music(&self, control: Arc<dyn MusicControl>) -> Result<(), PackError> {
        if self.worker.is_none() {
            return Err(PackError::NotFound("Reader closed".into()));
        }
        self.state.music.attach(control)
    }
    pub fn shutdown(&mut self) -> Result<(), PackError> {
        self.state.remote.close(true);
        #[cfg(not(target_os = "android"))]
        self.state.chat.close(true);
        #[cfg(all(feature = "mesh", not(target_os = "android")))]
        self.state.mesh.stop();
        let sharing = self
            .state
            .sharing
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let sharing_cleanup = sharing.map(|control| control.close()).transpose();
        let tv = self.state.tv.lock().ok().and_then(|mut slot| slot.take());
        let tv_cleanup = tv.map(|control| control.close()).transpose();
        #[cfg(all(feature = "mesh", not(target_os = "android")))]
        let radio_cleanup = self.state.mesh.wait_stopped();
        self.state.notes.stop();
        self.state.admin.lock();
        #[cfg(feature = "ai")]
        let ai_cleanup = self.state.ai.detach(true);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let result = if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| PackError::Corrupt("loopback runtime thread panicked".into()))?
        } else {
            Ok(())
        };
        #[cfg(feature = "ai")]
        ai_cleanup?;
        sharing_cleanup?;
        tv_cleanup?;
        #[cfg(all(feature = "mesh", not(target_os = "android")))]
        radio_cleanup?;
        result
    }
}
impl Drop for LoopbackServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn router(state: Arc<ServerState>) -> Router {
    let content = Arc::new(ContentState {
        registry: Arc::clone(&state.registry),
        allowed: None,
        web_root: state.web_root.clone(),
        media: Arc::new(SharedMedia::default()),
    });
    let app = Router::new()
        .route("/launch", get(auth::launch))
        .route("/api/session", get(auth::session))
        .route("/api/remote", get(remote::status))
        .route(
            "/api/remote/connect",
            post(remote::connect).layer(axum::extract::DefaultBodyLimit::max(70000)),
        )
        .route("/api/remote/disconnect", post(remote::disconnect))
        .route(
            "/api/remote/copy",
            post(remote::copy::start).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/remote/copy/control",
            post(remote::copy::control).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route("/api/content/catalog", get(content::catalog))
        .route("/api/zim/index-status", get(preparation::status))
        .route(
            "/api/zim/index-start",
            post(preparation::start).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/zim/index-control",
            post(preparation::control).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route("/api/copies", get(copies::listing))
        .route("/api/copies/{id}", axum::routing::delete(copies::remove))
        .route("/api/sharing", get(sharing::status))
        .route("/api/tv", get(tv_controls::status))
        .route(
            "/api/tv/start",
            post(tv_controls::start).layer(axum::extract::DefaultBodyLimit::max(16384)),
        )
        .route("/api/tv/stop", post(tv_controls::stop))
        .route(
            "/api/sharing/start",
            post(sharing::start).layer(axum::extract::DefaultBodyLimit::max(131072)),
        )
        .route(
            "/api/sharing/hide",
            post(sharing::hide).layer(axum::extract::DefaultBodyLimit::max(131072)),
        )
        .route("/api/sharing/stop", post(sharing::stop))
        .route("/api/sharing/devices/{id}/disconnect", post(sharing::kick))
        .route(
            "/api/map-management",
            get(map_management::list).post(map_management::start),
        )
        .route(
            "/api/map-management/{id}/{action}",
            post(map_management::control),
        )
        .route("/api/content/jobs", get(content::jobs).post(content::start))
        .route("/api/content/preflight", post(content::preflight))
        .route("/api/content/jobs/{id}/{action}", post(content::control))
        .route(
            "/api/content/installed/{id}",
            axum::routing::delete(content::remove),
        )
        .route(
            "/api/reading",
            get(reading::listing)
                .post(reading::save)
                .delete(reading::remove)
                .layer(axum::extract::DefaultBodyLimit::max(16384)),
        )
        .route(
            "/api/reading/bookmark",
            post(reading::bookmark).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/api/files", get(files::listing))
        .route(
            "/api/music",
            get(music::listing)
                .post(music::command)
                .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/api/music/stream",
            post(media_stream::audio).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/music/playback",
            get(music::playback_status)
                .post(music::playback_control)
                .layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/video/stream",
            post(media_stream::video).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/video/poster",
            post(media_stream::poster).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/video/progress",
            post(media_stream::position).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/media/{token}", get(media_stream::serve))
        .route(
            "/api/documents/stream",
            post(media_stream::pdf).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/api/documents/content/{token}", get(media_stream::serve))
        .route(
            "/api/documents/archive",
            post(books::open).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/book/{token}/{*path}", get(books::resource))
        .route(
            "/api/workshop/model",
            post(workshop::open).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/documents/import",
            post(files::import_document).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/files/native-import",
            post(files::native_import).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/files/upload",
            post(files::upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route(
            "/api/files/{id}",
            axum::routing::delete(files::remove).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/files/{id}/rename",
            post(files::rename).layer(axum::extract::DefaultBodyLimit::max(2048)),
        )
        .route(
            "/api/files/{id}/text",
            get(files::text)
                .put(files::edit)
                .layer(axum::extract::DefaultBodyLimit::max(512 * 1024)),
        )
        .route("/api/files/{id}/download", get(files::download))
        .route("/api/admin", get(admin::status))
        .route("/api/connections/chat", get(connections::status))
        .route("/api/connections/chat/forget", post(connections::forget))
        .route(
            "/api/connections/chat/connect",
            post(connections::connect).layer(axum::extract::DefaultBodyLimit::max(32768)),
        )
        .route(
            "/api/connections/chat/disconnect",
            post(connections::disconnect),
        )
        .route(
            "/api/connections/chat/proxy/{*path}",
            get(connections::proxy)
                .post(connections::proxy)
                .layer(axum::extract::DefaultBodyLimit::max(32768)),
        )
        .route(
            "/api/admin/setup",
            post(admin::setup).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/admin/login",
            post(admin::login).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/api/admin/lock", post(admin::lock))
        .route(
            "/api/room",
            get(sharing::room_get)
                .post(sharing::room_post)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/preferences",
            get(admin::preferences)
                .put(admin::update)
                .layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route("/api/update", get(update::get))
        .route(
            "/api/update/settings",
            put(update::settings).layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/update/check",
            post(update::check).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/update/download",
            post(update::download).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/update/cancel",
            post(update::cancel).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route("/api/notes/create", post(notes::create))
        .route("/api/notes/unlock", post(notes::unlock))
        .route("/api/notes/lock", post(notes::lock))
        .route("/api/notes/passphrase", post(notes::passphrase))
        .route("/api/notes/recovery", post(notes::recovery))
        .route("/api/notes", get(notes::list).post(notes::add))
        .route("/api/notes/search", get(notes::search))
        .route(
            "/api/notes/{id}",
            get(notes::get).put(notes::update).delete(notes::remove),
        );
    #[cfg(feature = "ai")]
    let app = app
        .route("/api/ai/model", get(ai::model))
        .route("/api/ai/models", get(ai::models::listing))
        .route("/api/ai/history", get(ai::history::listing))
        .route(
            "/api/ai/history/{id}",
            get(ai::history::get)
                .put(ai::history::save)
                .delete(ai::history::remove)
                .layer(axum::extract::DefaultBodyLimit::max(512 * 1024)),
        )
        .route(
            "/api/ai/models/load",
            post(ai::models::load).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/ai/models/off",
            post(ai::models::off).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/ai/chat",
            post(ai::chat).layer(axum::extract::DefaultBodyLimit::max(256 * 1024)),
        )
        .route(
            "/api/ai/cancel",
            post(ai::cancel).layer(axum::extract::DefaultBodyLimit::max(1024)),
        );
    #[cfg(feature = "mesh")]
    let app = app
        .route("/api/connections/radio", get(mesh::status))
        .route(
            "/api/connections/chat/bridge",
            get(connections::bridge::status)
                .post(connections::bridge::start)
                .layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/connections/chat/bridge/stop",
            post(connections::bridge::stop),
        )
        .route(
            "/api/connections/radio/devices",
            post(mesh::devices).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/connections/radio/connect",
            post(mesh::connect).layer(axum::extract::DefaultBodyLimit::max(8192)),
        )
        .route(
            "/api/connections/radio/send",
            post(mesh::send).layer(axum::extract::DefaultBodyLimit::max(2048)),
        )
        .route("/api/connections/radio/stop", post(mesh::stop));
    #[cfg(feature = "test-routes")]
    let app = app.route("/api/ping", get(|| async { "ok" }));
    app.with_state(Arc::clone(&state))
        .merge(content_router(content))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            remote::reader,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::guard,
        ))
}

async fn run(
    listener: tokio::net::TcpListener,
    app: Router,
    stopped: oneshot::Receiver<()>,
) -> Result<(), PackError> {
    use std::future::IntoFuture;
    let draining = Arc::new(Notify::new());
    let timeout = Arc::clone(&draining);
    let serving = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = stopped.await;
        draining.notify_one();
    });
    tokio::select! {
        result = serving.into_future() => result.map_err(Into::into),
        _ = async { timeout.notified().await; tokio::time::sleep(Duration::from_secs(2)).await; } => Ok(()),
    }
}
