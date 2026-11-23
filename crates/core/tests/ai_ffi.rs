#![cfg(all(feature = "ai", feature = "ffi"))]

use std::sync::Arc;

use tonalli_core::ffi::{
    ai_answer, ai_cancel, ai_close, ai_embed, ai_generate, ai_info, ai_load, ai_rerank, AiHandle,
    AiKind, AnswerOutcome, AnswerRequest, IndexHandle, PackHandle, RerankHit,
};
use tonalli_core::pack::PackError;

#[test]
fn answer_bridge_has_explicit_handles_and_owned_outcome() {
    type Bridge = fn(
        Arc<IndexHandle>,
        Arc<PackHandle>,
        Arc<AiHandle>,
        Option<Arc<AiHandle>>,
        AnswerRequest,
        Arc<dyn tonalli_core::ai::TokenSink>,
    ) -> Result<AnswerOutcome, PackError>;
    let _bridge: Bridge = ai_answer;
}

#[test]
#[ignore = "requires verified immutable Qwen and ZIM plus a separate guarded real-model lease; run serially"]
fn ai_ffi_real_answer_remains_owned_after_model_and_pack_close() {
    use std::path::{Path, PathBuf};
    use tonalli_core::{
        ai::{GenerationEnd, GenerationEvent, TokenSink},
        ffi::{index_open, pack_close, pack_open_internal},
        index,
        pack::PreadSource,
        zim::Zim,
    };
    struct Sink;
    impl TokenSink for Sink {
        fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
            Ok(())
        }
    }
    let model = std::env::var("TONALLI_TEST_QWEN_INTERNAL").expect("verified internal Qwen");
    let path = std::env::var_os("TONALLI_TEST_ZIM_INTERNAL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim")
        });
    let zim = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    let temp = tempfile::tempdir().unwrap();
    index::build(&zim, temp.path(), 1).unwrap();
    let pack = pack_open_internal(path.to_string_lossy().into_owned()).unwrap();
    let index = index_open(temp.path().to_string_lossy().into_owned(), pack.clone()).unwrap();
    let handle = ai_load(model, AiKind::Generation).unwrap();
    let outcome = ai_answer(
        index,
        pack.clone(),
        handle.clone(),
        None,
        AnswerRequest {
            question: "Explain the trefoil knot using the numbered sources.".into(),
            retrieval_query: "trefoil".into(),
            embedding_query: None,
            max_tokens: 128,
        },
        Arc::new(Sink),
    );
    ai_close(handle).unwrap();
    pack_close(pack).unwrap();
    let AnswerOutcome::Observed {
        answer,
        stats,
        error,
        terminal_forwarded,
        timings,
        ..
    } = outcome.unwrap()
    else {
        panic!("expected actual observed terminal");
    };
    assert!(matches!(
        answer.end,
        GenerationEnd::Eos | GenerationEnd::Limit
    ));
    assert!(error.is_none() && terminal_forwarded && !answer.raw_text.is_empty());
    assert!(stats.input_tokens > 0 && stats.output_tokens > 0 && stats.output_tokens <= 128);
    assert!(!answer.sources.is_empty());
    for source in &answer.sources {
        assert_eq!(zim.article(&source.url).unwrap().url, source.url);
    }
    for citation in &answer.citations {
        assert!(answer
            .sources
            .iter()
            .any(|source| source.number == *citation));
    }
    assert!(timings
        .first_raw_received_us
        .is_some_and(|first| first <= timings.total_us));
    assert!(timings.rerank_us.is_none());
    eprintln!("ffi_answer_owned_after_close=true raw_text={:?} visible_text={:?} citations={:?} sources={:?} flags={:?} stats={stats:?} timings={timings:?}", answer.raw_text, answer.text, answer.citations, answer.sources, answer.flags);
}

#[test]
fn rerank_bridge_returns_owned_indices_and_float64_scores() {
    type Bridge = fn(Arc<AiHandle>, String, Vec<String>) -> Result<Vec<RerankHit>, PackError>;
    let _bridge: Bridge = ai_rerank;
    let hit = RerankHit {
        index: 19,
        score: 0.75,
    };
    let owned = hit.clone();
    assert_eq!(owned.index, 19);
    assert_eq!(owned.score, 0.75);
}

#[test]
#[ignore = "requires verified internal BGE and a separate guarded real-model lease"]
fn ai_ffi_real_rerank_preserves_owned_scores_bounds_and_closed_aliases() {
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").expect("verified internal BGE path");
    let handle = ai_load(path, AiKind::Embedding).unwrap();
    let alias = Arc::clone(&handle);
    let text = "A red fox crosses a snowy field.".to_owned();
    let ranked = ai_rerank(Arc::clone(&handle), text.clone(), vec![text.clone(), text]).unwrap();
    for documents in [vec![], vec!["text".into(); 21]] {
        assert!(matches!(
            ai_rerank(Arc::clone(&handle), "query".into(), documents),
            Err(PackError::Mismatch(_))
        ));
    }
    ai_close(handle).unwrap();
    assert_eq!(
        ranked.iter().map(|hit| hit.index).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(ranked
        .iter()
        .all(|hit| hit.score.is_finite() && (hit.score - 1.0).abs() < 0.00001));
    assert!(matches!(
        ai_rerank(Arc::clone(&alias), "query".into(), vec!["text".into()]),
        Err(PackError::NotFound(_))
    ));
    ai_close(alias).unwrap();
}

#[test]
fn embedding_bridge_returns_owned_nested_float_vectors() {
    type Bridge = fn(Arc<AiHandle>, Vec<String>) -> Result<Vec<Vec<f32>>, PackError>;
    let _bridge: Bridge = ai_embed;
}

#[test]
#[ignore = "requires verified internal BGE and a separate guarded real-model lease"]
fn ai_ffi_real_embedding_preserves_vectors_input_errors_and_closed_aliases() {
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").expect("verified internal BGE path");
    let handle = ai_load(path, AiKind::Embedding).unwrap();
    let alias = Arc::clone(&handle);
    let vectors = ai_embed(
        Arc::clone(&handle),
        vec!["A red fox crosses a snowy field.".into()],
    )
    .unwrap();
    assert_eq!(vectors.len(), 1);
    assert_eq!(vectors[0].len(), 384);
    for texts in [vec![], vec!["text".into(); 22], vec!["bad\0text".into()]] {
        assert!(matches!(
            ai_embed(Arc::clone(&handle), texts),
            Err(PackError::Mismatch(_))
        ));
    }
    ai_close(handle).unwrap();
    assert!(vectors[0].iter().all(|value| value.is_finite()));
    assert!(
        (vectors[0]
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            - 1.0)
            .abs()
            < 0.00001
    );
    assert!(matches!(
        ai_embed(Arc::clone(&alias), vec!["text".into()]),
        Err(PackError::NotFound(_))
    ));
    ai_close(alias).unwrap();
}

#[test]
fn unknown_ai_generation_cancellation_is_idempotent() {
    ai_cancel(0).unwrap();
    ai_cancel(0).unwrap();
}

#[test]
fn unexpected_foreign_callback_error_uses_typed_conversion() {
    let error = uniffi::UnexpectedUniFFICallbackError::new("test callback failure");
    let converted: Result<PackError, _> = uniffi::convert_unexpected_error!(error, PackError);
    assert!(matches!(converted, Ok(PackError::Corrupt(_))));
}

#[test]
fn ai_ffi_rejects_missing_directory_and_corrupt_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.gguf");
    assert!(matches!(
        ai_load(missing.to_string_lossy().into_owned(), AiKind::Generation),
        Err(PackError::NotFound(_))
    ));
    assert!(matches!(
        ai_load(
            directory.path().to_string_lossy().into_owned(),
            AiKind::Embedding
        ),
        Err(PackError::Mismatch(_))
    ));
    let corrupt = directory.path().join("corrupt.gguf");
    std::fs::write(&corrupt, b"GGUF").unwrap();
    assert!(matches!(
        ai_load(corrupt.to_string_lossy().into_owned(), AiKind::Generation),
        Err(PackError::Truncated)
    ));
}

#[test]
#[ignore = "requires verified immutable internal Qwen input and real RAM admission"]
fn ai_ffi_real_model_info_and_close_invalidate_aliases() {
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").expect("verified internal Qwen path");
    let handle = ai_load(path, AiKind::Generation).unwrap();
    let alias = Arc::clone(&handle);
    let info = ai_info(Arc::clone(&alias)).unwrap();
    assert!(matches!(info.kind, AiKind::Generation));
    assert!(info.file_bytes > 0);
    assert!(info.training_context_tokens > 0);
    assert!(info.embedding_dimensions > 0);
    assert!(info.estimated_reserved_bytes > info.file_bytes);
    ai_close(handle).unwrap();
    assert!(matches!(
        ai_info(Arc::clone(&alias)),
        Err(PackError::NotFound(_))
    ));
    ai_close(alias).unwrap();
}

#[test]
#[ignore = "requires verified immutable internal Qwen input and real RAM admission"]
fn ai_ffi_real_generation_delivers_started_text_and_terminal() {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tonalli_core::ai::{GenerationEnd, GenerationEvent, TokenSink};

    struct Sink(mpsc::Sender<GenerationEvent>);
    impl TokenSink for Sink {
        fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
            self.0
                .send(event)
                .map_err(|_| PackError::Io("test sink closed".into()))
        }
    }

    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").expect("verified internal Qwen path");
    let handle = ai_load(path, AiKind::Generation).unwrap();
    let (sender, receiver) = mpsc::channel();
    let started_at = Instant::now();
    let id = ai_generate(
        Arc::clone(&handle),
        "What is two plus two? Answer briefly. /no_think".into(),
        32,
        Arc::new(Sink(sender)),
    )
    .unwrap();
    assert!(started_at.elapsed() < Duration::from_secs(1));
    let mut started = false;
    let mut text = String::new();
    let deadline = started_at + Duration::from_secs(90);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("generation deadline");
        match receiver.recv_timeout(remaining).expect("generation event") {
            GenerationEvent::Started { id: observed } => {
                assert_eq!(observed, id);
                assert!(!started);
                started = true;
            }
            GenerationEvent::Text {
                id: observed,
                text: chunk,
            } => {
                assert!(started);
                assert_eq!(observed, id);
                text.push_str(&chunk);
            }
            GenerationEvent::Finished {
                id: observed,
                reason,
                stats,
                error,
            } => {
                assert!(started);
                assert_eq!(observed, id);
                assert!(matches!(reason, GenerationEnd::Eos | GenerationEnd::Limit));
                assert!(error.is_none());
                assert!(stats.input_tokens > 0 && stats.output_tokens > 0);
                assert!(stats.first_token_us.is_some());
                break;
            }
        }
    }
    assert!(!text.trim().is_empty());
    ai_cancel(id).unwrap();
    ai_close(handle).unwrap();
}
