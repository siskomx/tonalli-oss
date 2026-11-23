use super::*;
use crate::{
    ffi::{index_open, pack_close, pack_open_internal, IndexHandle, PackHandle},
    index,
    pack::PreadSource,
    zim::Zim,
};
use std::{path::PathBuf, sync::OnceLock};

struct NoCallback;
impl ai::TokenSink for NoCallback {
    fn on_event(&self, _: ai::GenerationEvent) -> Result<(), PackError> {
        panic!("invalid or empty answer must not call the sink");
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim")
}

fn handles() -> (Arc<IndexHandle>, Arc<PackHandle>) {
    static BUILT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let directory = BUILT.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap();
        let zim = Zim::open(Arc::new(PreadSource::open(&fixture()).unwrap())).unwrap();
        index::build(&zim, directory.path(), 1).unwrap();
        directory
    });
    let pack = pack_open_internal(fixture().to_string_lossy().into_owned()).unwrap();
    let index = index_open(
        directory.path().to_string_lossy().into_owned(),
        pack.clone(),
    )
    .unwrap();
    (index, pack)
}

fn closed() -> Arc<AiHandle> {
    Arc::new(AiHandle {
        core: ai::AiHandle::closed_for_test(),
    })
}

fn request(query: &str) -> ai::AnswerRequest {
    ai::AnswerRequest {
        question: "Explain the trefoil knot.".into(),
        retrieval_query: query.into(),
        embedding_query: None,
        max_tokens: 128,
    }
}

#[test]
fn answer_closed_pack_remains_not_found() {
    let (index, pack) = handles();
    pack_close(pack.clone()).unwrap();
    assert!(matches!(
        ai_answer(
            index,
            pack,
            closed(),
            None,
            request("trefoil"),
            Arc::new(NoCallback)
        ),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn answer_embedding_option_pairing_is_not_guessed() {
    for with_handle in [false, true] {
        let (index, pack) = handles();
        let mut request = request("trefoil");
        if !with_handle {
            request.embedding_query = Some("explicit query".into());
        }
        let embedding = with_handle.then(closed);
        assert!(matches!(
            ai_answer(
                index,
                pack,
                closed(),
                embedding,
                request,
                Arc::new(NoCallback)
            ),
            Err(PackError::Mismatch(_))
        ));
    }
}

#[test]
fn answer_empty_retrieval_does_not_touch_closed_models() {
    let (index, pack) = handles();
    let mut request = request("zztonallinosuchtermzz");
    request.embedding_query = Some("explicit query".into());
    let outcome = ai_answer(
        index,
        pack,
        closed(),
        Some(closed()),
        request,
        Arc::new(NoCallback),
    )
    .unwrap();
    assert!(
        matches!(outcome, ai::AnswerOutcome::NoSources { timings } if timings.rerank_us.is_none() && timings.first_raw_received_us.is_none())
    );
}

#[test]
fn answer_nonempty_retrieval_preserves_closed_generation_error() {
    let (index, pack) = handles();
    assert!(matches!(
        ai_answer(
            index,
            pack,
            closed(),
            None,
            request("trefoil"),
            Arc::new(NoCallback)
        ),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn answer_mismatched_pack_is_refused_before_closed_model_access() {
    let (index, _) = handles();
    let directory = tempfile::tempdir().unwrap();
    let changed = directory.path().join("different.zim");
    let mut bytes = std::fs::read(fixture()).unwrap();
    bytes[8] ^= 1;
    std::fs::write(&changed, bytes).unwrap();
    let pack = pack_open_internal(changed.to_string_lossy().into_owned()).unwrap();
    assert!(matches!(
        ai_answer(
            index,
            pack,
            closed(),
            None,
            request("trefoil"),
            Arc::new(NoCallback)
        ),
        Err(PackError::Mismatch(_))
    ));
}
