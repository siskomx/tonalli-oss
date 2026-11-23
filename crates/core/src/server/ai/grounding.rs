//! Private chat adapter over the existing source-bound answer engine.
use std::sync::Arc;

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};

use super::{
    failure,
    stream::{self, Lease},
    ChatRequest,
};
use crate::{
    ai::{self, AiHandle, GenerationEnd, PreparedAnswer},
    pack::PackError,
    server::{PackId, PackRegistry},
};

pub(super) struct Grounding {
    pack: String,
    prepared: PreparedAnswer,
}

fn prepare(registry: &PackRegistry, pack: String, question: &str) -> Result<Grounding, PackError> {
    let id = PackId::new(pack.clone())?;
    // Same coherent zims -> indexes capture as article search; no lock crosses I/O.
    let (zim, index) = {
        let zims = registry.zims.read().map_err(|_| super::poisoned())?;
        let indexes = registry.indexes.read().map_err(|_| super::poisoned())?;
        let zim = zims
            .get(&id)
            .cloned()
            .ok_or_else(|| PackError::NotFound("book unavailable".into()))?;
        let index = indexes
            .get(&id)
            .cloned()
            .ok_or_else(|| PackError::NotFound("article search unavailable".into()))?;
        (zim, index)
    };
    // Questions are words, not Tantivy field/operator/recursive query syntax.
    let query = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let prepared = ai::prepare_answer(&index, &zim, question, &query)?;
    Ok(Grounding { pack, prepared })
}

impl Grounding {
    pub(super) fn metadata(&self, raw: String, end: GenerationEnd) -> Result<Value, PackError> {
        let (citations, flags) = if self.prepared.prompt().is_some() {
            let answer = ai::finish_answer(&self.prepared, raw, end)?;
            (answer.citations, answer.flags)
        } else {
            (Vec::new(), ai::AnswerFlags::default())
        };
        // Source authority is exclusively the passages actually injected into the prompt.
        Ok(
            json!({"pack":self.pack, "no_sources":self.prepared.prompt().is_none(),
            "sources":self.prepared.sources().iter().map(|source| json!({
                "number":source.number,"title":source.title,"url":source.url,"excerpt":source.excerpt
            })).collect::<Vec<_>>(), "citations":citations,
            "flags":{"uncited":flags.uncited,"invalid_citations":flags.invalid_citations,
                "incomplete":flags.incomplete,"truncated":flags.truncated}}),
        )
    }
}

pub(super) async fn start(
    registry: Arc<PackRegistry>,
    handle: Arc<AiHandle>,
    lease: Lease,
    request: ChatRequest,
) -> Response {
    let max_tokens = request.max_tokens;
    // Retain admission in the blocking worker, including when its HTTP future is dropped.
    let prepared = tokio::task::spawn_blocking(move || {
        let mut messages = request.messages;
        let current = messages
            .last_mut()
            .ok_or_else(|| PackError::Mismatch("missing current question".into()))?;
        let grounding = prepare(
            &registry,
            request.pack.unwrap_or_default(),
            &current.content,
        )?;
        if let Some(prompt) = grounding.prepared.prompt() {
            // Only current retrieval supplies citation authority; prior turns retain their roles.
            current.content = prompt.to_owned();
        }
        Ok::<_, PackError>((grounding, lease, messages))
    })
    .await;
    let (grounding, lease, messages) = match prepared {
        Ok(Ok(result)) => result,
        Ok(Err(PackError::NotFound(_))) => {
            return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_index_unavailable")
        }
        Ok(Err(PackError::Mismatch(_))) => {
            return failure(StatusCode::BAD_REQUEST, "ai_invalid_request")
        }
        _ => return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_retrieval_failed"),
    };
    if grounding.prepared.prompt().is_none() {
        let metadata = match grounding.metadata(String::new(), GenerationEnd::Eos) {
            Ok(value) => value,
            Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "ai_retrieval_failed"),
        };
        drop(lease);
        return ([(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-store")],
            format!("data: {}\n\n", json!({"message":{"content":""},"done":true,"done_reason":"stop","grounding":metadata}))).into_response();
    }
    stream::start(handle, lease, messages, max_tokens, Some(grounding)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        index::{self, Index},
        pack::PreadSource,
        zim::Zim,
    };

    #[test]
    fn real_book_sources_are_canonical_and_missing_retrieval_has_no_prompt() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
        let zim = Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap());
        let directory = tempfile::tempdir().unwrap();
        index::build(&zim, directory.path(), 1).unwrap();
        let registry = PackRegistry::default();
        let id = PackId::new("book").unwrap();
        registry.register_zim(id.clone(), zim.clone()).unwrap();
        assert!(prepare(&registry, "book".into(), "bowline").is_err());
        registry
            .attach_index(
                id.clone(),
                Arc::new(Index::open(directory.path(), &zim).unwrap()),
            )
            .unwrap();
        let grounding = prepare(&registry, "book".into(), "bowline?").unwrap();
        assert!(grounding.prepared.prompt().is_some());
        let metadata = grounding
            .metadata("Claim [1] [999]".into(), GenerationEnd::Eos)
            .unwrap();
        assert_eq!(metadata["citations"], json!([1]));
        assert_eq!(metadata["flags"]["invalid_citations"], true);
        for source in grounding.prepared.sources() {
            assert_eq!(zim.article(&source.url).unwrap().url, source.url);
        }
        let empty = prepare(&registry, "book".into(), "zztonallinosuchtermzz").unwrap();
        assert!(empty.prepared.prompt().is_none());
        assert_eq!(
            empty.metadata(String::new(), GenerationEnd::Eos).unwrap()["no_sources"],
            true
        );
        registry.remove(&id).unwrap();
        assert!(prepare(&registry, "book".into(), "bowline").is_err());
    }

    #[tokio::test]
    async fn empty_retrieval_finishes_without_using_a_model_and_releases_admission() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/articles/benign-v1.zim");
        let zim = Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap());
        let directory = tempfile::tempdir().unwrap();
        index::build(&zim, directory.path(), 1).unwrap();
        let registry = Arc::new(PackRegistry::default());
        let id = PackId::new("book").unwrap();
        registry.register_zim(id.clone(), zim.clone()).unwrap();
        registry
            .attach_index(id, Arc::new(Index::open(directory.path(), &zim).unwrap()))
            .unwrap();
        let owner = Arc::new(super::super::ChatState::default());
        let control = Arc::new(super::super::stream::Control::new("empty".into()));
        owner.slot.lock().unwrap().active = Some(control.clone());
        let lease = Lease {
            owner: owner.clone(),
            control,
        };
        let request =
            serde_json::from_value(json!({"request_id":"empty", "model":"local", "pack":"book",
            "messages":[{"role":"user","content":"Welcome"},
                        {"role":"assistant","content":"Earlier answer [1]."},
                        {"role":"user","content":"zztonallinosuchtermzz"}], "max_tokens":128}))
            .unwrap();
        // A closed real handle would fail if generation were attempted.
        let response = start(registry, AiHandle::closed_for_test(), lease, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        let value: Value =
            serde_json::from_str(text.trim().strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(value["grounding"]["no_sources"], true);
        assert_eq!(value["grounding"]["sources"], json!([]));
        assert_eq!(value["done"], true);
        assert!(owner.slot.lock().unwrap().active.is_none());
    }
}
