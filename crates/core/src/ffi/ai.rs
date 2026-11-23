//! Optional AI adapters; ownership and memory policy remain in the Core loader.

use std::{path::Path, sync::Arc};

use super::{pack_boundary, IndexHandle, LoopbackHandle, PackHandle};
pub use crate::ai::{
    Answer, AnswerFlags, AnswerOutcome, AnswerRequest, AnswerSource, AnswerTimings, RerankHit,
};
use crate::{ai, pack::PackError};

impl From<uniffi::UnexpectedUniFFICallbackError> for PackError {
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        // Foreign exception text may contain private content; keep this bounded.
        Self::Corrupt("AI token callback failed".into())
    }
}

#[derive(uniffi::Enum)]
pub enum AiKind {
    Generation,
    Embedding,
}

impl From<AiKind> for ai::ModelKind {
    fn from(kind: AiKind) -> Self {
        match kind {
            AiKind::Generation => Self::Generation,
            AiKind::Embedding => Self::Embedding,
        }
    }
}

#[derive(uniffi::Record)]
pub struct AiInfo {
    pub kind: AiKind,
    pub file_bytes: u64,
    pub training_context_tokens: u32,
    pub embedding_dimensions: u32,
    /// Provisional admission estimate, not measured resident memory or PSS.
    pub estimated_reserved_bytes: u64,
}

#[derive(uniffi::Object)]
pub struct AiHandle {
    core: Arc<ai::AiHandle>,
}

/// Caller provides a checksum-verified, immutable, app-owned internal model.
/// Removable inputs must be verified and atomically imported before this call.
#[uniffi::export]
pub fn ai_load(path: String, kind: AiKind) -> Result<Arc<AiHandle>, PackError> {
    pack_boundary(|| {
        Ok(Arc::new(AiHandle {
            core: ai::load_internal(Path::new(&path), kind.into())?,
        }))
    })
}

#[uniffi::export]
pub fn ai_info(handle: Arc<AiHandle>) -> Result<AiInfo, PackError> {
    pack_boundary(|| {
        let info = handle.core.info()?;
        Ok(AiInfo {
            kind: match info.kind {
                ai::ModelKind::Generation => AiKind::Generation,
                ai::ModelKind::Embedding => AiKind::Embedding,
            },
            file_bytes: info.file_bytes,
            training_context_tokens: info.training_context_tokens,
            embedding_dimensions: info.embedding_dimensions,
            estimated_reserved_bytes: info.estimated_reserved_bytes,
        })
    })
}

#[uniffi::export]
pub fn ai_close(handle: Arc<AiHandle>) -> Result<(), PackError> {
    pack_boundary(|| handle.core.close())
}

/// Attach an already verified loaded model. The platform retains close ownership.
#[uniffi::export]
pub fn server_attach_ai(
    server: Arc<LoopbackHandle>,
    model_id: String,
    model: Arc<AiHandle>,
) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("server lock poisoned".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("stopped server".into()))?
            .attach_ai(model_id, Arc::clone(&model.core))
    })
}

/// Remove this server's attachment and request cancellation; not a terminal wait.
#[uniffi::export]
pub fn server_detach_ai(server: Arc<LoopbackHandle>) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("server lock poisoned".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("stopped server".into()))?
            .detach_ai()
    })
}

/// Native Context supplies its private files directory; paths never enter HTTP.
#[uniffi::export]
pub fn server_configure_ai_model_root(
    server: Arc<LoopbackHandle>,
    root: String,
) -> Result<(), PackError> {
    pack_boundary(|| {
        server
            .server
            .lock()
            .map_err(|_| PackError::Corrupt("Loopback handle lock poisoned".into()))?
            .as_ref()
            .ok_or_else(|| PackError::NotFound("Reader closed".into()))?
            .configure_ai_model_root(Path::new(&root))
    })
}

/// Blocking bounded embeddings; call from a worker. Closing the model requests cancellation.
#[uniffi::export]
pub fn ai_embed(handle: Arc<AiHandle>, texts: Vec<String>) -> Result<Vec<Vec<f32>>, PackError> {
    pack_boundary(|| ai::embed(Arc::clone(&handle.core), texts))
}

/// Blocking Core reranking; query formatting and BM25 candidate order belong to the caller.
#[uniffi::export]
pub fn ai_rerank(
    handle: Arc<AiHandle>,
    query: String,
    documents: Vec<String>,
) -> Result<Vec<RerankHit>, PackError> {
    pack_boundary(|| ai::rerank(Arc::clone(&handle.core), query, documents))
}

/// Blocking Core answer pipeline; call from a worker. None selects BM25-only retrieval.
/// Sinks must enqueue UI work promptly; ai_cancel remains independent of callbacks.
#[uniffi::export]
pub fn ai_answer(
    index: Arc<IndexHandle>,
    pack: Arc<PackHandle>,
    generation: Arc<AiHandle>,
    embedding: Option<Arc<AiHandle>>,
    request: AnswerRequest,
    sink: Arc<dyn ai::TokenSink>,
) -> Result<AnswerOutcome, PackError> {
    pack_boundary(|| {
        let zim = pack.zim()?;
        ai::answer(
            &index.inner,
            &zim,
            Arc::clone(&generation.core),
            embedding.map(|handle| Arc::clone(&handle.core)),
            request,
            sink,
        )
    })
}

/// Returns promptly; the sink receives started, text and terminal events.
/// Sinks must enqueue UI work promptly instead of blocking the callback thread.
#[uniffi::export]
pub fn ai_generate(
    handle: Arc<AiHandle>,
    prompt: String,
    max_tokens: u32,
    sink: Arc<dyn ai::TokenSink>,
) -> Result<u64, PackError> {
    pack_boundary(|| ai::generate(Arc::clone(&handle.core), prompt, max_tokens, sink))
}

/// Cancellation is idempotent and does not wait for a native decode or callback.
#[uniffi::export]
pub fn ai_cancel(id: u64) -> Result<(), PackError> {
    pack_boundary(|| ai::cancel(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed() -> Arc<AiHandle> {
        Arc::new(AiHandle {
            core: ai::AiHandle::closed_for_test(),
        })
    }

    #[test]
    fn rerank_document_bounds_precede_native_handle_access() {
        for documents in [vec![], vec!["text".into(); 21]] {
            assert!(matches!(
                ai_rerank(closed(), "query".into(), documents),
                Err(PackError::Mismatch(_))
            ));
        }
    }

    #[test]
    fn rerank_valid_input_preserves_closed_handle_error() {
        assert!(matches!(
            ai_rerank(closed(), "query".into(), vec!["text".into()]),
            Err(PackError::NotFound(_))
        ));
    }

    #[test]
    fn loopback_attachment_refuses_stopped_server_without_reopening_model() {
        let server = Arc::new(super::super::LoopbackHandle {
            server: std::sync::Mutex::new(None),
            registry: Arc::new(crate::server::PackRegistry::default()),
        });
        let model = closed();
        assert!(matches!(
            server_attach_ai(
                Arc::clone(&server),
                "qwen3-1.7b-q4-k-m".into(),
                Arc::clone(&model)
            ),
            Err(PackError::NotFound(_))
        ));
        assert!(matches!(
            server_detach_ai(server),
            Err(PackError::NotFound(_))
        ));
        assert!(matches!(ai_info(model), Err(PackError::NotFound(_))));
    }
}

#[cfg(test)]
mod answer_tests;
