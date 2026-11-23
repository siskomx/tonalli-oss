use super::*;
use sha2::{Digest, Sha256};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn fixture() -> Questions {
    serde_json::from_str(include_str!(
        "../../../../docs/spike/ai-rerank-questions-v1.json"
    ))
    .unwrap()
}

#[test]
fn parser_requires_explicit_export_mode_and_frozen_small_index_authority() {
    let config = parse(&args(&[
        "--id",
        "wikipedia-en-knots",
        "--index-authority",
        "frozen/export-receipt.json",
        "--out",
        "new",
        "--export-only",
    ]))
    .unwrap();
    assert_eq!(config.id, "wikipedia-en-knots");
    assert_eq!(
        config.authority,
        PathBuf::from("frozen/export-receipt.json")
    );
    assert_eq!(config.output, PathBuf::from("new"));
    assert_eq!(config.manifest, PathBuf::from("docs/spike/packs.toml"));
    assert_eq!(
        config.questions,
        PathBuf::from("docs/spike/ai-rerank-questions-v1.json")
    );
    assert_eq!(config.mode, Mode::ExportOnly);
    let config = parse(&args(&[
        "--id",
        "wikipedia-en-knots",
        "--index-authority",
        "receipt",
        "--out",
        "new",
        "--embedding-id",
        "bge-small-en-v1.5-q8-0",
    ]))
    .unwrap();
    assert_eq!(config.mode, Mode::Bge("bge-small-en-v1.5-q8-0".into()));
}

#[test]
fn parser_rejects_ambiguous_missing_duplicate_and_arbitrary_index_options() {
    let base = [
        "--id",
        "knots",
        "--index-authority",
        "receipt",
        "--out",
        "new",
    ];
    for tail in [
        vec![],
        vec!["--export-only", "--embedding-id", "bge"],
        vec!["--export-only", "--export-only"],
        vec!["--embedding-id"],
        vec!["--export-only", "--id", "again"],
        vec!["--export-only", "--index", "arbitrary-active-index"],
        vec!["--export-only", "--unknown", "value"],
    ] {
        let options: Vec<_> = base.into_iter().chain(tail).collect();
        assert!(matches!(
            parse(&args(&options)),
            Err(PackError::Mismatch(_))
        ));
    }
    assert!(parse(&args(&["--id", "knots", "--out", "new", "--export-only"])).is_err());
}

#[test]
fn fixture_has_ten_paired_questions_with_shared_english_retrieval() {
    let questions = fixture();
    validate_questions(&questions).unwrap();
    assert_eq!(questions.questions.len(), 10);
    assert_eq!(questions.retrieval_language, "en");
    assert_eq!(
        questions.questions[0].retrieval_query_en,
        "What makes the trefoil knot nontrivial?"
    );
    assert_eq!(
        questions.questions[9].retrieval_query_en,
        "How did quipu encode information using knots?"
    );
    assert!(questions
        .questions
        .iter()
        .all(|question| question.retrieval_query_en == question.question_en));
    assert!(questions
        .questions
        .iter()
        .all(|question| !question.question_en.is_empty() && !question.question_es.is_empty()));
}

#[test]
fn malformed_question_sets_are_typed_errors() {
    let mut questions = fixture();
    questions.questions.pop();
    assert!(matches!(
        validate_questions(&questions),
        Err(PackError::Mismatch(_))
    ));
    let mut questions = fixture();
    questions.questions[1].id = questions.questions[0].id.clone();
    assert!(validate_questions(&questions).is_err());
    let mut questions = fixture();
    questions.retrieval_language = "es".into();
    assert!(validate_questions(&questions).is_err());
    let mut questions = fixture();
    questions.questions[0].question_es.clear();
    assert!(validate_questions(&questions).is_err());
    let mut questions = fixture();
    questions.questions[0].id = "../outside".into();
    assert!(validate_questions(&questions).is_err());
    let mut questions = fixture();
    questions.questions[0].retrieval_query_en = "trefoil knot".into();
    assert!(matches!(
        validate_questions(&questions),
        Err(PackError::Mismatch(_))
    ));
}

fn hit(rank: usize) -> Hit {
    Hit {
        title: format!("Title {rank}"),
        url: format!("C/Article_{rank}"),
        score: 10.0 - rank as f32,
    }
}

#[test]
fn passage_freezes_exact_utf8_prefix_and_distinct_source_hashes() {
    let plain = format!("{}ðŸŒŽtail", "a".repeat(1023));
    let passage = freeze_passage(&hit(1), 1, b"<p>original article</p>", &plain).unwrap();
    assert_eq!(passage.text.as_bytes(), "a".repeat(1023).as_bytes());
    assert!(passage.truncated);
    assert_eq!(passage.source_text_bytes, plain.len());
    assert_eq!(
        passage.passage_sha256,
        format!("{:x}", Sha256::digest("a".repeat(1023).as_bytes()))
    );
    assert_eq!(
        passage.article_sha256,
        format!("{:x}", Sha256::digest(b"<p>original article</p>"))
    );
    let short = freeze_passage(&hit(1), 1, b"raw", "short").unwrap();
    assert!(!short.truncated);
    assert_eq!(short.text, "short");
}

fn dataset() -> Dataset {
    Dataset {
        pack_id: "knots".into(),
        pack_sha256: "a".repeat(64),
        index_manifest_sha256: "b".repeat(64),
        index_generation: "generation-fixture".into(),
    }
}

fn prepared_passage(rank: usize) -> Passage {
    let text = format!("actual passage {rank} ðŸŒŽ");
    Passage {
        bm25_rank: rank,
        title: hit(rank).title,
        url: hit(rank).url,
        article_sha256: format!("{:x}", Sha256::digest(b"article")),
        passage_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        source_text_bytes: text.len(),
        truncated: false,
        text,
    }
}

#[test]
fn export_preserves_payloads_bindings_raw_ranks_and_blank_judgments() {
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("q01");
    let passages: Vec<_> = (1..=4).map(prepared_passage).collect();
    let order: Vec<_> = [3, 1, 0, 2]
        .into_iter()
        .map(|index| Ranked {
            index,
            cosine: None,
        })
        .collect();
    let question = fixture().questions.remove(0);
    export_question(&output, &dataset(), &question, &passages, &order).unwrap();
    assert_eq!(
        std::fs::read(output.join("passage-1.txt")).unwrap(),
        passages[3].text.as_bytes()
    );
    let metadata = std::fs::read(output.join("question.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&metadata).unwrap();
    assert_eq!(value["dataset"]["pack_sha256"], "a".repeat(64));
    assert_eq!(value["dataset"]["index_manifest_sha256"], "b".repeat(64));
    assert_eq!(
        value["question"]["retrieval_query_en"],
        question.retrieval_query_en
    );
    assert_eq!(value["question"]["question_es"], question.question_es);
    assert_eq!(value["ranking"].as_array().unwrap().len(), 4);
    assert_eq!(value["selected"][0]["url"], passages[3].url);
    assert_eq!(
        value["selected"][0]["passage_sha256"],
        passages[3].passage_sha256
    );
    assert_eq!(
        value["selected"][0].get("input_tokens"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        value["judgments"].get("bm25_top3_supported"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        value["judgments"].get("reranked_top3_supported"),
        Some(&serde_json::Value::Null)
    );
    assert!(!output.join("passage-4.txt").exists());
    assert!(export_question(&output, &dataset(), &question, &passages, &order).is_err());
    assert_eq!(
        std::fs::read(output.join("question.json")).unwrap(),
        metadata
    );
}

#[test]
fn export_rejects_out_of_range_and_duplicate_source_ranks() {
    let temporary = tempfile::tempdir().unwrap();
    let question = fixture().questions.remove(0);
    let passage = prepared_passage(1);
    for (name, order) in [
        (
            "outside",
            vec![Ranked {
                index: usize::MAX,
                cosine: None,
            }],
        ),
        (
            "duplicate",
            vec![
                Ranked {
                    index: 0,
                    cosine: None,
                },
                Ranked {
                    index: 0,
                    cosine: None,
                },
            ],
        ),
    ] {
        assert!(matches!(
            export_question(
                &temporary.path().join(name),
                &dataset(),
                &question,
                std::slice::from_ref(&passage),
                &order
            ),
            Err(PackError::Corrupt(_))
        ));
    }
}

#[test]
#[cfg(not(feature = "ai"))]
fn disabled_bge_mode_refuses_before_input_io_or_output_creation() {
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("untouched");
    let options = vec![
        "--id".into(),
        "knots".into(),
        "--index-authority".into(),
        "missing".into(),
        "--out".into(),
        output.to_string_lossy().into_owned(),
        "--embedding-id".into(),
        "bge".into(),
    ];
    assert!(
        matches!(run(&options), Err(PackError::Mismatch(message)) if message.contains("ai feature"))
    );
    assert!(!output.exists());
}
