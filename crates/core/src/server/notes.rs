//! Private loopback vault. Requests carry the generation captured at authorization.
use super::ServerState;
use crate::notes::{Note, Notes, NotesError, Unlock};
use axum::{
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use zeroize::Zeroizing;

const MAX_BODY: usize = 1024 * 1024;
const MAX_INPUT: usize = 4096;
type Problem = (StatusCode, &'static str);
const LOCKED: Problem = (StatusCode::LOCKED, "notes_locked");
const UNAVAILABLE: Problem = (StatusCode::SERVICE_UNAVAILABLE, "notes_unavailable");
const INVALID: Problem = (StatusCode::BAD_REQUEST, "invalid_request");
const TOO_LARGE: Problem = (StatusCode::PAYLOAD_TOO_LARGE, "input_too_large");

#[derive(Clone, Copy)]
pub(super) struct Admission(u64);
pub(super) struct Vault {
    epoch: AtomicU64,
    stopped: AtomicBool,
    notes: Mutex<Option<Notes>>,
}
impl Default for Vault {
    fn default() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            notes: Mutex::new(None),
        }
    }
}
impl Vault {
    pub(super) fn admission(&self) -> Admission {
        Admission(self.epoch.load(Ordering::Acquire))
    }
    pub(super) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn clear(&self) {
        self.notes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    pub(super) fn revoke(&self) {
        self.invalidate();
        self.clear();
    }
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.revoke();
    }
    fn lock_request(&self, admission: Admission) -> Result<Admission, Problem> {
        self.epoch
            .compare_exchange(
                admission.0,
                admission.0.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| LOCKED)?;
        Ok(Admission(admission.0.wrapping_add(1)))
    }
    fn run<T>(
        &self,
        admission: Admission,
        work: impl FnOnce(&mut Option<Notes>) -> Result<T, Problem>,
    ) -> Result<T, Problem> {
        let mut notes = match self.notes.lock() {
            Ok(notes) => notes,
            Err(poisoned) => {
                poisoned.into_inner().take();
                self.invalidate();
                return Err(UNAVAILABLE);
            }
        };
        if self.stopped.load(Ordering::Acquire) {
            return Err(UNAVAILABLE);
        }
        if self.admission().0 != admission.0 {
            return Err(LOCKED);
        }
        let result = catch_unwind(AssertUnwindSafe(|| work(&mut notes)));
        // A lock/rotation can invalidate us during a KDF or SQL operation.
        if self.admission().0 != admission.0 {
            notes.take();
            return Err(LOCKED);
        }
        match result {
            Ok(Err(error)) if error.0.is_server_error() => {
                notes.take();
                self.invalidate();
                Err(error)
            }
            Ok(result) => result,
            Err(_) => {
                notes.take();
                self.invalidate();
                Err(UNAVAILABLE)
            }
        }
    }
}

fn response(error: Problem) -> Response {
    (error.0, Json(json!({"error":error.1}))).into_response()
}
fn error(error: NotesError) -> Problem {
    match error {
        NotesError::UnlockFailed => (StatusCode::UNAUTHORIZED, "unlock_failed"),
        NotesError::MissingKeys => (StatusCode::CONFLICT, "recovery_required"),
        NotesError::NotFound(_) => (StatusCode::NOT_FOUND, "note_not_found"),
        NotesError::RotationUncertain(_) => (StatusCode::SERVICE_UNAVAILABLE, "rotation_uncertain"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "notes_unavailable"),
    }
}
async fn work(
    state: Arc<ServerState>,
    admission: Admission,
    status: StatusCode,
    operation: impl FnOnce(&mut Option<Notes>) -> Result<Value, Problem> + Send + 'static,
) -> Response {
    let cleanup = state.clone();
    match tokio::task::spawn_blocking(move || state.notes.run(admission, operation)).await {
        Ok(Ok(_)) if status == StatusCode::NO_CONTENT => status.into_response(),
        Ok(Ok(value)) => (status, Json(value)).into_response(),
        Ok(Err(problem)) => response(problem),
        Err(_) => {
            cleanup.notes.invalidate();
            let _ = tokio::task::spawn_blocking(move || cleanup.notes.clear()).await;
            response(UNAVAILABLE)
        }
    }
}
async fn body(request: Request) -> Result<Zeroizing<Vec<u8>>, Problem> {
    if request
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_none_or(|h| h.split(';').next() != Some("application/json"))
    {
        return Err(INVALID);
    }
    to_bytes(request.into_body(), MAX_BODY)
        .await
        .map(|b| Zeroizing::new(b.to_vec()))
        .map_err(|_| TOO_LARGE)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    passphrase: Option<String>,
    recovery_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    passphrase: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassphraseChange {
    passphrase: Option<String>,
    recovery_key: Option<String>,
    new_passphrase: String,
}

pub(super) async fn passphrase(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    work(state, admission, StatusCode::NO_CONTENT, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let input: PassphraseChange = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        let passphrase = input.passphrase.map(Zeroizing::new);
        let recovery = input.recovery_key.map(Zeroizing::new);
        let new = Zeroizing::new(input.new_passphrase);
        if new.is_empty() || passphrase.is_some() == recovery.is_some() {
            return Err(INVALID);
        }
        if new.len() > MAX_INPUT
            || passphrase.as_ref().is_some_and(|p| p.len() > MAX_INPUT)
            || recovery.as_ref().is_some_and(|p| p.len() > MAX_INPUT)
        {
            return Err(TOO_LARGE);
        }
        if let Some(old) = passphrase {
            notes.change_passphrase(&old, &new).map_err(error)?;
        } else {
            let key = recovery
                .ok_or(INVALID)?
                .parse()
                .map_err(|_| (StatusCode::UNAUTHORIZED, "unlock_failed"))?;
            notes.recover_passphrase(&key, &new).map_err(error)?;
        }
        Ok(Value::Null)
    })
    .await
}

pub(super) async fn recovery(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    work(state, admission, StatusCode::OK, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let input: Create = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        let passphrase = Zeroizing::new(input.passphrase);
        if passphrase.len() > MAX_INPUT {
            return Err(TOO_LARGE);
        }
        let key = notes.export_recovery_key(&passphrase).map_err(error)?;
        Ok(json!({"recovery_key":key.to_string()}))
    })
    .await
}

pub(super) async fn create(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    let path = state.data_dir.join("notes.db");
    work(state, admission, StatusCode::CREATED, move |slot| {
        if slot.is_some() {
            return Err((StatusCode::CONFLICT, "already_unlocked"));
        }
        let input: Create = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        let passphrase = Zeroizing::new(input.passphrase);
        if passphrase.is_empty() {
            return Err(INVALID);
        }
        if passphrase.len() > MAX_INPUT {
            return Err(TOO_LARGE);
        }
        // Core creation refuses every existing DB/key sidecar and publishes encrypted files only.
        let (notes, recovery) =
            Notes::create(&path, &passphrase).map_err(|failure| match failure {
                NotesError::AlreadyExists(_) => (StatusCode::CONFLICT, "vault_exists"),
                failure => error(failure),
            })?;
        *slot = Some(notes);
        Ok(json!({"recovery_key":recovery.to_string()}))
    })
    .await
}

pub(super) async fn unlock(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    let path = state.data_dir.join("notes.db");
    work(state, admission, StatusCode::OK, move |slot| {
        if slot.is_some() {
            return Err((StatusCode::CONFLICT, "already_unlocked"));
        }
        let credentials: Credentials = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        let passphrase = credentials.passphrase.map(Zeroizing::new);
        let recovery = credentials.recovery_key.map(Zeroizing::new);
        if passphrase.is_some() == recovery.is_some() {
            return Err(INVALID);
        }
        if passphrase.as_ref().is_some_and(|p| p.len() > MAX_INPUT)
            || recovery.as_ref().is_some_and(|k| k.len() > MAX_INPUT)
        {
            return Err(TOO_LARGE);
        }
        if !path
            .try_exists()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "notes_unavailable"))?
        {
            return Err((StatusCode::CONFLICT, "notes_not_initialized"));
        }
        let notes = if let Some(passphrase) = passphrase {
            Notes::open(&path, Unlock::Passphrase(&passphrase)).map_err(error)?
        } else {
            let key = recovery
                .ok_or(INVALID)?
                .parse()
                .map_err(|_| (StatusCode::UNAUTHORIZED, "unlock_failed"))?;
            Notes::open(&path, Unlock::Recovery(&key)).map_err(error)?
        };
        *slot = Some(notes);
        Ok(json!({"unlocked":true}))
    })
    .await
}
pub(super) async fn lock(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
) -> Response {
    let next = match state.notes.lock_request(admission) {
        Ok(next) => next,
        Err(error) => return response(error),
    };
    work(state, next, StatusCode::NO_CONTENT, |slot| {
        slot.take();
        Ok(Value::Null)
    })
    .await
}
fn note(note: Note) -> Value {
    json!({"id":note.id,"title":note.title,"body":note.body})
}
pub(super) async fn list(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
) -> Response {
    read(state, admission, String::new()).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Search {
    #[serde(default)]
    q: String,
}
pub(super) async fn search(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    query: Result<Query<Search>, axum::extract::rejection::QueryRejection>,
) -> Response {
    match query {
        Ok(Query(query)) => read(state, admission, query.q).await,
        Err(_) => response(INVALID),
    }
}
async fn read(state: Arc<ServerState>, admission: Admission, query: String) -> Response {
    work(state, admission, StatusCode::OK, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        if query.len() > MAX_INPUT { return Err(TOO_LARGE); }
        Ok(json!({"notes":notes.search(&query).map_err(error)?.into_iter().map(note).collect::<Vec<_>>()}))
    }).await
}
pub(super) async fn get(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    Path(id): Path<String>,
) -> Response {
    work(state, admission, StatusCode::OK, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let id = id.parse::<i64>().ok().filter(|id| *id > 0).ok_or(INVALID)?;
        Ok(note(notes.get(id).map_err(error)?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewNote {
    title: String,
    body: String,
}

pub(super) async fn update(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    work(state, admission, StatusCode::NO_CONTENT, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let id = id.parse::<i64>().ok().filter(|id| *id > 0).ok_or(INVALID)?;
        let input: NewNote = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        if input.title.len() > MAX_INPUT {
            return Err(TOO_LARGE);
        }
        notes.update(id, &input.title, &input.body).map_err(error)?;
        Ok(Value::Null)
    })
    .await
}

pub(super) async fn remove(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    Path(id): Path<String>,
) -> Response {
    work(state, admission, StatusCode::NO_CONTENT, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let id = id.parse::<i64>().ok().filter(|id| *id > 0).ok_or(INVALID)?;
        notes.remove(id).map_err(error)?;
        Ok(Value::Null)
    })
    .await
}
pub(super) async fn add(
    State(state): State<Arc<ServerState>>,
    Extension(admission): Extension<Admission>,
    request: Request,
) -> Response {
    let bytes = match body(request).await {
        Ok(bytes) => bytes,
        Err(error) => return response(error),
    };
    work(state, admission, StatusCode::CREATED, move |slot| {
        let notes = slot.as_ref().ok_or(LOCKED)?;
        let input: NewNote = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        if input.title.len() > MAX_INPUT {
            return Err(TOO_LARGE);
        }
        Ok(json!({"id":notes.add(&input.title, &input.body).map_err(error)?}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Barrier};

    fn queued_unlock(revoke: impl FnOnce(&Vault, Admission)) {
        let vault = Arc::new(Vault::default());
        let admission = vault.admission();
        let queued = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let entered = Arc::new(AtomicBool::new(false));
        let worker = {
            let (vault, queued, release, entered) = (
                vault.clone(),
                queued.clone(),
                release.clone(),
                entered.clone(),
            );
            std::thread::spawn(move || {
                queued.wait();
                release.wait();
                vault.run(admission, |_| {
                    entered.store(true, Ordering::Release);
                    Ok(())
                })
            })
        };
        queued.wait();
        revoke(&vault, admission);
        release.wait();
        assert_eq!(worker.join().expect("worker"), Err(LOCKED));
        assert!(!entered.load(Ordering::Acquire));
        assert!(vault
            .run(vault.admission(), |slot| Ok(slot.is_none()))
            .expect("new admission"));
    }
    #[test]
    fn explicit_lock_rejects_already_admitted_queued_unlock() {
        queued_unlock(|vault, admission| {
            let next = vault.lock_request(admission).expect("lock admitted");
            vault
                .run(next, |slot| {
                    slot.take();
                    Ok(())
                })
                .expect("lock complete");
        });
    }
    #[test]
    fn rotation_rejects_already_admitted_queued_unlock() {
        queued_unlock(|vault, _| vault.revoke());
    }
    #[test]
    fn lock_during_actual_unlock_drops_newly_opened_vault() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("notes.db");
        let (notes, _) = Notes::create(&path, "synthetic").expect("create");
        drop(notes);
        let vault = Arc::new(Vault::default());
        let admission = vault.admission();
        let opened = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let (vault, opened, release) = (vault.clone(), opened.clone(), release.clone());
            std::thread::spawn(move || {
                vault.run(admission, |slot| {
                    let notes =
                        Notes::open(&path, Unlock::Passphrase("synthetic")).map_err(error)?;
                    opened.wait();
                    release.wait();
                    *slot = Some(notes);
                    Ok(())
                })
            })
        };
        opened.wait();
        let next = vault
            .lock_request(admission)
            .expect("invalidate without waiting for SQL mutex");
        release.wait();
        assert_eq!(worker.join().expect("worker"), Err(LOCKED));
        assert!(vault
            .run(next, |slot| Ok(slot.is_none()))
            .expect("locked state"));
    }
    #[test]
    fn panic_drops_vault_and_invalidates_waiting_work() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("notes.db");
        let (notes, _) = Notes::create(&path, "synthetic").expect("create");
        let vault = Vault::default();
        let admission = vault.admission();
        assert_eq!(
            vault.run::<()>(admission, |slot| {
                *slot = Some(notes);
                panic!("injected")
            }),
            Err(UNAVAILABLE)
        );
        assert_eq!(vault.run(admission, |_| Ok(())), Err(LOCKED));
        assert!(vault
            .run(vault.admission(), |slot| Ok(slot.is_none()))
            .expect("vault cleared"));
        std::fs::remove_file(path).expect("database handle released");
    }
    #[test]
    fn poisoned_vault_lock_drops_retained_database() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("notes.db");
        let (notes, _) = Notes::create(&path, "synthetic").expect("create");
        let vault = Arc::new(Vault::default());
        let worker = vault.clone();
        assert!(std::thread::spawn(move || {
            *worker.notes.lock().expect("lock") = Some(notes);
            let _guard = worker.notes.lock().expect("lock");
            panic!("injected lock poison");
        })
        .join()
        .is_err());
        assert_eq!(vault.run(vault.admission(), |_| Ok(())), Err(UNAVAILABLE));
        std::fs::remove_file(path).expect("database handle released despite poisoned mutex");
    }
}
