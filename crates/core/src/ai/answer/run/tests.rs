use super::*;
use crate::pack::PreadSource;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

fn request() -> AnswerRequest {
    AnswerRequest {
        question: "Explain the trefoil knot.".into(),
        retrieval_query: "trefoil".into(),
        embedding_query: None,
        max_tokens: 128,
    }
}

#[test]
fn request_is_bounded_and_embedding_requires_an_explicit_query() {
    request().validate(false).unwrap();
    let mut with_embedding = request();
    assert!(with_embedding.validate(true).is_err());
    with_embedding.embedding_query = Some("Explicit model-specific query.".into());
    with_embedding.validate(true).unwrap();
    assert!(with_embedding.validate(false).is_err());
    for cap in [0, 1025] {
        let mut invalid = request();
        invalid.max_tokens = cap;
        assert!(matches!(
            invalid.validate(false),
            Err(PackError::Mismatch(_))
        ));
    }
    for value in ["".into(), "bad\0input".into(), "x".repeat(4097)] {
        let mut invalid = request();
        invalid.question = value;
        assert!(invalid.validate(false).is_err());
    }
    for value in ["".into(), "bad\0query".into(), "x".repeat(4097)] {
        let mut invalid = request();
        invalid.retrieval_query = value;
        assert!(invalid.validate(false).is_err());
    }
    for value in ["".into(), "bad\0query".into(), "x".repeat(16_385)] {
        let mut invalid = request();
        invalid.embedding_query = Some(value);
        assert!(invalid.validate(true).is_err());
    }
}

fn fixture() -> Zim {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
    Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()
}
fn index(zim: &Zim) -> Index {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        crate::index::build(zim, dir.path(), 1).unwrap();
        dir
    });
    Index::open(root.path(), zim).unwrap()
}

#[derive(Default)]
struct Count(AtomicUsize);
impl TokenSink for Count {
    fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn empty_retrieval_never_touches_either_closed_model_or_foreign_sink() {
    let zim = fixture();
    let index = index(&zim);
    let sink = Arc::new(Count::default());
    let mut request = request();
    request.retrieval_query = "zztonallinosuchtermzz".into();
    request.embedding_query = Some("Explicit embedding query.".into());
    let result = answer(
        &index,
        &zim,
        AiHandle::closed_for_test(),
        Some(AiHandle::closed_for_test()),
        request,
        sink.clone(),
    )
    .unwrap();
    assert!(matches!(result, AnswerOutcome::NoSources { .. }));
    assert_eq!(sink.0.load(Ordering::SeqCst), 0);
    let timings = result.timings();
    assert!(timings.rerank_us.is_none() && timings.first_raw_received_us.is_none());
    assert!(timings.retrieval_us <= timings.total_us && timings.extraction_us <= timings.total_us);
}

#[test]
fn source_mismatch_is_refused_before_any_model_or_callback() {
    let zim = fixture();
    let index = index(&zim);
    let other = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    let other = Zim::open(Arc::new(PreadSource::open(&other).unwrap())).unwrap();
    let sink = Arc::new(Count::default());
    assert!(matches!(
        answer(
            &index,
            &other,
            AiHandle::closed_for_test(),
            None,
            request(),
            sink.clone()
        ),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(sink.0.load(Ordering::SeqCst), 0);
}

#[test]
fn nonempty_bm25_request_preserves_closed_generation_error() {
    let zim = fixture();
    let index = index(&zim);
    let sink = Arc::new(Count::default());
    assert!(matches!(
        answer(
            &index,
            &zim,
            AiHandle::closed_for_test(),
            None,
            request(),
            sink.clone()
        ),
        Err(PackError::NotFound(_))
    ));
    assert_eq!(sink.0.load(Ordering::SeqCst), 0);
}
