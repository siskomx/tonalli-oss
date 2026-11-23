#![cfg(feature = "ai")]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tempfile::TempDir;
use tonalli_core::{
    ai::prepare_answer,
    index::{self, Index},
    pack::{PackError, PreadSource},
    zim::Zim,
};

fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("TONALLI_TEST_ZIM_INTERNAL") {
        return PathBuf::from(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim")
}
fn archive(path: &Path) -> Zim {
    Zim::open(Arc::new(PreadSource::open(path).unwrap())).unwrap()
}
fn built() -> &'static TempDir {
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        index::build(&archive(&path()), dir.path(), 1).unwrap();
        dir
    })
}

#[test]
fn each_citation_maps_to_an_article_url_in_the_zim() {
    let zim = archive(&path());
    let index = Index::open(built().path(), &zim).unwrap();
    let prepared = prepare_answer(&index, &zim, "Explain the trefoil knot.", "trefoil").unwrap();
    assert!(!prepared.sources().is_empty() && prepared.sources().len() <= 3);
    for (position, source) in prepared.sources().iter().enumerate() {
        assert_eq!(source.number as usize, position + 1);
        assert_eq!(zim.article(&source.url).unwrap().url, source.url);
        assert!(source.excerpt.len() <= 1024);
    }
}

#[test]
fn empty_retrieval_yields_no_generation_prompt() {
    let zim = archive(&path());
    let index = Index::open(built().path(), &zim).unwrap();
    let prepared =
        prepare_answer(&index, &zim, "An unknown subject?", "zztonallinosuchtermzz").unwrap();
    assert!(prepared.sources().is_empty() && prepared.prompt().is_none());
}

#[test]
fn a_different_uuid_or_stored_md5_is_refused_before_retrieval() {
    let zim = archive(&path());
    let index = Index::open(built().path(), &zim).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (name, offset) in [
        ("uuid", 8),
        ("md5", std::fs::metadata(path()).unwrap().len() - 1),
    ] {
        use std::io::{Read, Seek, SeekFrom, Write};
        let changed = temp.path().join(format!("{name}.zim"));
        // Copy bytes into a private writable file without inheriting immutable source permissions.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&changed)
            .unwrap();
        std::io::copy(&mut std::fs::File::open(path()).unwrap(), &mut file).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 1;
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&byte).unwrap();
        drop(file);
        assert!(matches!(
            prepare_answer(&index, &archive(&changed), "Question?", "trefoil"),
            Err(PackError::Mismatch(_))
        ));
    }
}

struct AnswerSink {
    cancel_on_text: bool,
    cancellation: std::sync::Mutex<Option<(tonalli_core::ai::GenId, std::time::Instant)>>,
}

impl tonalli_core::ai::TokenSink for AnswerSink {
    fn on_event(&self, event: tonalli_core::ai::GenerationEvent) -> Result<(), PackError> {
        if let tonalli_core::ai::GenerationEvent::Text { id, text } = event {
            if self.cancel_on_text && !text.is_empty() {
                let should_cancel = {
                    let mut requested = self.cancellation.lock().unwrap();
                    if requested.is_none() {
                        *requested = Some((id, std::time::Instant::now()));
                        true
                    } else {
                        false
                    }
                };
                if should_cancel {
                    tonalli_core::ai::cancel(id)?;
                }
            }
        }
        Ok(())
    }
}

fn real_answer(
    cancel_on_text: bool,
) -> (
    tonalli_core::ai::AnswerOutcome,
    Arc<AnswerSink>,
    std::time::Instant,
) {
    use tonalli_core::ai::{answer, load_internal, AnswerRequest, ModelKind};
    let model = std::env::var("TONALLI_TEST_QWEN_INTERNAL")
        .expect("verified immutable internal Qwen path and separate runtime RAM admission");
    let handle = load_internal(Path::new(&model), ModelKind::Generation).unwrap();
    let zim = archive(&path());
    let index = Index::open(built().path(), &zim).unwrap();
    let sink = Arc::new(AnswerSink {
        cancel_on_text,
        cancellation: std::sync::Mutex::new(None),
    });
    let outcome = answer(
        &index,
        &zim,
        handle.clone(),
        None,
        AnswerRequest {
            question: "Explain the trefoil knot using the numbered sources.".into(),
            retrieval_query: "trefoil".into(),
            embedding_query: None,
            max_tokens: 128,
        },
        sink.clone(),
    );
    let returned = std::time::Instant::now();
    // Close even if the proof subsequently rejects the returned outcome.
    handle.close().unwrap();
    (outcome.unwrap(), sink, returned)
}

#[test]
#[ignore = "requires verified immutable Qwen and a separate guarded real-model lease; run serially"]
fn real_public_answer_owns_sources_and_stats_after_close() {
    use tonalli_core::ai::{AnswerOutcome, GenerationEnd};
    let (outcome, _, _) = real_answer(false);
    let AnswerOutcome::Observed {
        answer,
        stats,
        error,
        terminal_forwarded,
        timings,
        ..
    } = outcome
    else {
        panic!("real answer must observe its actual terminal");
    };
    assert!(matches!(
        answer.end,
        GenerationEnd::Eos | GenerationEnd::Limit
    ));
    assert!(error.is_none() && terminal_forwarded);
    assert!(stats.input_tokens > 0 && stats.output_tokens > 0 && stats.output_tokens <= 128);
    assert!(timings
        .first_raw_received_us
        .is_some_and(|first| first <= timings.total_us));
    assert!(timings.rerank_us.is_none());
    assert!(!answer.raw_text.is_empty() && !answer.sources.is_empty());
    let zim = archive(&path());
    for source in &answer.sources {
        assert_eq!(zim.article(&source.url).unwrap().url, source.url);
    }
    for number in &answer.citations {
        assert!(answer.sources.iter().any(|source| source.number == *number));
    }
    assert_eq!(answer.flags.uncited, answer.citations.is_empty());
    assert_eq!(answer.flags.truncated, answer.end == GenerationEnd::Limit);
    eprintln!("answer_owned_after_close=true sources={} citations={} flags={:?} stats={stats:?} timings={timings:?}", answer.sources.len(), answer.citations.len(), answer.flags);
    eprintln!(
        "answer_public_fixture raw_text={:?} visible_text={:?} citations={:?} canonical_urls={:?}",
        answer.raw_text,
        answer.text,
        answer.citations,
        answer
            .sources
            .iter()
            .map(|source| &source.url)
            .collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "requires verified immutable Qwen and a separate guarded real-model lease; run serially"]
fn real_public_answer_text_cancel_observes_cancelled_within_one_second() {
    use tonalli_core::ai::{AnswerOutcome, GenerationEnd};
    let (outcome, sink, returned) = real_answer(true);
    let (requested_id, requested) = sink
        .cancellation
        .lock()
        .unwrap()
        .expect("actual nonempty Text before cancel");
    let AnswerOutcome::Observed {
        answer,
        id,
        error,
        terminal_forwarded,
        ..
    } = outcome
    else {
        panic!("cancel must have an observed terminal, not a deadline");
    };
    assert_eq!(id, requested_id);
    assert_eq!(answer.end, GenerationEnd::Cancelled);
    assert!(error.is_none() && terminal_forwarded && !answer.raw_text.is_empty());
    let elapsed = returned.duration_since(requested);
    assert!(
        elapsed <= std::time::Duration::from_secs(1),
        "cancel-to-return {elapsed:?}"
    );
    eprintln!("answer_cancel_to_return_us={}", elapsed.as_micros());
}
