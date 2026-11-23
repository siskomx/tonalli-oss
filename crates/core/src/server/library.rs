//! Read-only installed ZIM metadata, shared through the existing content allowlist.
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::ContentState;
use crate::zim::Zim;

pub(super) async fn listing(State(state): State<Arc<ContentState>>) -> Response {
    let entries = {
        let zims = match state.registry.zims.read() {
            Ok(zims) => zims,
            Err(_) => return unavailable(),
        };
        zims.iter()
            .filter(|(id, _)| state.allows(id))
            .map(|(id, zim)| (id.as_str().to_owned(), Arc::clone(zim)))
            .collect::<Vec<_>>()
    };
    // No registry lock or borrowed handles cross the blocking-I/O boundary.
    match tokio::task::spawn_blocking(move || readable_packs(entries)).await {
        Ok(packs) => Json(json!({"packs":packs})).into_response(),
        Err(_) => unavailable(),
    }
}

fn readable_packs(mut entries: Vec<(String, Arc<Zim>)>) -> Vec<serde_json::Value> {
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    entries
        .into_iter()
        .filter_map(|(id, zim)| {
            let total = zim.title_count().ok()?;
            let title = zim.metadata_title().ok()?.unwrap_or_else(|| id.clone());
            Some(json!({"id":id,"title":title,"total":total}))
        })
        .collect()
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"library_unavailable"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{PackError, PackSource, PreadSource};
    use crate::server::{PackId, PackRegistry};
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Weak,
    };

    struct LockProbe {
        inner: PreadSource,
        registry: Weak<PackRegistry>,
        enabled: AtomicBool,
        observed: AtomicBool,
    }
    impl PackSource for LockProbe {
        fn len(&self) -> u64 {
            self.inner.len()
        }
        fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize, PackError> {
            if self.enabled.load(Ordering::SeqCst) {
                let registry = self.registry.upgrade().unwrap();
                assert!(
                    registry.zims.try_write().is_ok(),
                    "registry lock spans metadata I/O"
                );
                self.observed.store(true, Ordering::SeqCst);
            }
            self.inner.read_at(offset, bytes)
        }
    }

    #[tokio::test]
    async fn listing_reads_metadata_after_releasing_registry_lock() {
        let registry = Arc::new(PackRegistry::default());
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
        let source = Arc::new(LockProbe {
            inner: PreadSource::open(&path).unwrap(),
            registry: Arc::downgrade(&registry),
            enabled: AtomicBool::new(false),
            observed: AtomicBool::new(false),
        });
        registry
            .register_zim(
                PackId::new("fixture").unwrap(),
                Arc::new(Zim::open(source.clone()).unwrap()),
            )
            .unwrap();
        source.enabled.store(true, Ordering::SeqCst);
        let state = Arc::new(ContentState {
            registry,
            allowed: None,
            web_root: PathBuf::new(),
            media: Arc::new(super::super::SharedMedia::default()),
        });
        assert_eq!(listing(State(state)).await.status(), StatusCode::OK);
        assert!(source.observed.load(Ordering::SeqCst));
    }
}
