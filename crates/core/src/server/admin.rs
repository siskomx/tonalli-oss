//! Private local controls; the LAN content router never owns this state.
use super::ServerState;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

type Problem = (StatusCode, &'static str);
const UNAVAILABLE: Problem = (StatusCode::SERVICE_UNAVAILABLE, "admin_unavailable");
const LOCKED: Problem = (StatusCode::LOCKED, "admin_locked");
const INVALID: Problem = (StatusCode::BAD_REQUEST, "invalid_request");
const CONFLICT: Problem = (StatusCode::CONFLICT, "configuration_changed");

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preferences {
    revision: u64,
    theme: Theme,
    show_maps: bool,
    show_notes: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Theme {
    System,
    Light,
    Dark,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            revision: 0,
            theme: Theme::System,
            show_maps: true,
            show_notes: true,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    salt: [u8; 16],
    hash: [u8; 32],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    credential: Option<Credential>,
    preferences: Preferences,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            credential: None,
            preferences: Preferences::default(),
        }
    }
}
#[derive(Default)]
struct Session {
    token: Option<(Zeroizing<String>, Instant, u64)>,
    retry_after: Option<Instant>,
}
#[derive(Default)]
pub(super) struct Admin {
    session: Mutex<Session>,
    epoch: AtomicU64,
}
impl Admin {
    pub(super) fn permits(&self, token: &str) -> bool {
        self.session
            .lock()
            .is_ok_and(|session| self.authorized(&session, token))
    }
    pub(super) fn lock(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
    fn authorized(&self, session: &Session, token: &str) -> bool {
        session
            .token
            .as_ref()
            .is_some_and(|(expected, until, epoch)| {
                *epoch == self.epoch.load(Ordering::Acquire)
                    && Instant::now() < *until
                    && bool::from(expected.as_bytes().ct_eq(token.as_bytes()))
            })
    }
}

// Reuse the pack-root policy; also exclude Windows junctions at every fixed ancestor.
fn owned_path(root: &Path, name: &str) -> Result<PathBuf, Problem> {
    let path = crate::download::pack_path(root, &format!("packs/private/{name}"))
        .map_err(|_| UNAVAILABLE)?;
    let mut current = root.to_path_buf();
    for part in path
        .strip_prefix(root)
        .map_err(|_| UNAVAILABLE)?
        .components()
    {
        current.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let linked = metadata.file_type().is_symlink();
            if linked {
                return Err(UNAVAILABLE);
            }
        }
    }
    Ok(path)
}
fn document(root: &Path) -> Result<(File, PathBuf, Document), Problem> {
    let path = owned_path(root, "admin.json")?;
    fs::create_dir_all(path.parent().ok_or(UNAVAILABLE)?).map_err(|_| UNAVAILABLE)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(owned_path(root, "admin.lock")?)
        .map_err(|_| UNAVAILABLE)?;
    FileExt::lock_exclusive(&lock).map_err(|_| UNAVAILABLE)?;
    let document = match File::open(&path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| UNAVAILABLE)?;
            if bytes.len() > 4096 {
                return Err(UNAVAILABLE);
            }
            let value: Document = serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE)?;
            if value.version != 1 {
                return Err(UNAVAILABLE);
            }
            value
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Document::default(),
        Err(_) => return Err(UNAVAILABLE),
    };
    Ok((lock, path, document))
}
fn save(path: &Path, document: &Document) -> Result<(), Problem> {
    let bytes = serde_json::to_vec(document).map_err(|_| UNAVAILABLE)?;
    crate::download::write_atomic(path, &bytes).map_err(|_| UNAVAILABLE)
}
fn derive(secret: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, Problem> {
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| UNAVAILABLE)?;
    let mut blocks = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(params.block_count())
        .map_err(|_| UNAVAILABLE)?;
    blocks.resize(params.block_count(), Block::default());
    let mut hash = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(secret.as_bytes(), salt, &mut hash[..], &mut blocks[..])
        .map_err(|_| UNAVAILABLE)?;
    Ok(hash)
}
async fn work(
    state: Arc<ServerState>,
    operation: impl FnOnce(&ServerState, &mut Session, &Path, &mut Document) -> Result<Value, Problem>
        + Send
        + 'static,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let mut session = state.admin.session.lock().map_err(|_| UNAVAILABLE)?;
        let (_lock, path, mut document) = document(&state.data_dir)?;
        operation(&state, &mut session, &path, &mut document)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err((status, error))) => (status, Json(json!({"error":error}))).into_response(),
        Err(_) => (UNAVAILABLE.0, Json(json!({"error":UNAVAILABLE.1}))).into_response(),
    }
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    work(state, |_, _, _, document| {
        Ok(json!({"configured":document.credential.is_some(), "unlocked":false}))
    })
    .await
}
pub(super) async fn preferences(State(state): State<Arc<ServerState>>) -> Response {
    work(state, |_, _, _, document| {
        serde_json::to_value(&document.preferences).map_err(|_| UNAVAILABLE)
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Secret {
    passphrase: String,
}
async fn authenticate(state: Arc<ServerState>, secret: Secret, setup: bool) -> Response {
    let secret = Zeroizing::new(secret.passphrase);
    let epoch = state.admin.epoch.load(Ordering::Acquire);
    work(state, move |state, session, path, document| {
        if secret.len() < 12 || secret.len() > 1024 {
            return Err(INVALID);
        }
        if session
            .retry_after
            .is_some_and(|until| Instant::now() < until)
        {
            return Err((StatusCode::TOO_MANY_REQUESTS, "try_again_later"));
        }
        if setup {
            if document.credential.is_some() {
                return Err(CONFLICT);
            }
            let mut salt = [0; 16];
            getrandom::fill(&mut salt).map_err(|_| UNAVAILABLE)?;
            document.credential = Some(Credential {
                salt,
                hash: *derive(&secret, &salt)?,
            });
        } else {
            let credential = document.credential.as_ref().ok_or(CONFLICT)?;
            let hash = derive(&secret, &credential.salt)?;
            if !bool::from(hash[..].ct_eq(&credential.hash)) {
                session.retry_after = Some(Instant::now() + Duration::from_secs(2));
                return Err((StatusCode::UNAUTHORIZED, "unlock_failed"));
            }
        }
        if epoch != state.admin.epoch.load(Ordering::Acquire) {
            return Err(LOCKED);
        }
        let mut random = Zeroizing::new([0; 32]);
        getrandom::fill(&mut random[..]).map_err(|_| UNAVAILABLE)?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        if setup {
            save(path, document)?;
        }
        session.token = Some((
            Zeroizing::new(token.clone()),
            Instant::now() + Duration::from_secs(15 * 60),
            epoch,
        ));
        session.retry_after = None;
        Ok(json!({"token":token}))
    })
    .await
}
pub(super) async fn setup(
    State(state): State<Arc<ServerState>>,
    Json(secret): Json<Secret>,
) -> Response {
    authenticate(state, secret, true).await
}
pub(super) async fn login(
    State(state): State<Arc<ServerState>>,
    Json(secret): Json<Secret>,
) -> Response {
    authenticate(state, secret, false).await
}
pub(super) async fn lock(State(state): State<Arc<ServerState>>) -> Json<Value> {
    state.admin.lock();
    Json(json!({"locked":true}))
}
pub(super) async fn update(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(preferences): Json<Preferences>,
) -> Response {
    let token = headers
        .get("x-tonalli-admin")
        .and_then(|x| x.to_str().ok())
        .unwrap_or("")
        .to_owned();
    work(state, move |state, session, path, document| {
        if !state.admin.authorized(session, &token) {
            return Err(LOCKED);
        }
        if preferences.revision != document.preferences.revision {
            return Err(CONFLICT);
        }
        document.preferences = preferences;
        document.preferences.revision = document
            .preferences
            .revision
            .checked_add(1)
            .ok_or(CONFLICT)?;
        save(path, document)?;
        serde_json::to_value(&document.preferences).map_err(|_| UNAVAILABLE)
    })
    .await
}
