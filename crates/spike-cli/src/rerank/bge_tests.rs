use super::*;

fn question() -> Question {
    Question {
        id: "q01".into(),
        retrieval_query_en: "What makes the trefoil knot nontrivial?".into(),
        question_en: "What makes the trefoil knot nontrivial?".into(),
        question_es: "¿Qué hace que el nudo trébol sea no trivial?".into(),
    }
}

fn passage() -> Passage {
    let text = "Unchanged candidate 🌎".to_owned();
    Passage {
        bm25_rank: 1,
        title: "Trefoil".into(),
        url: "C/Trefoil_knot".into(),
        article_sha256: "a".repeat(64),
        passage_sha256: digest(text.as_bytes()),
        source_text_bytes: text.len(),
        truncated: false,
        text,
    }
}

#[test]
fn query_only_prefix_preserves_full_english_question_and_document_bytes() {
    let q = question();
    let p = passage();
    let input = query_input(&q, std::slice::from_ref(&p)).unwrap();
    assert_eq!(input.prefix, PREFIX);
    assert_eq!(input.query, format!("{PREFIX}{}", q.question_en));
    assert_eq!(input.query_sha256, digest(input.query.as_bytes()));
    assert_eq!(input.documents, vec![p.text]);
    assert_eq!(q.retrieval_query_en, q.question_en);
}

#[test]
fn core_rank_mapping_preserves_core_order_scores_and_rejects_bad_permutations() {
    let order = mapped_order(
        vec![
            RerankHit {
                index: 2,
                score: 0.9,
            },
            RerankHit {
                index: 0,
                score: 0.7,
            },
            RerankHit {
                index: 1,
                score: 0.7,
            },
        ],
        3,
    )
    .unwrap();
    assert_eq!(
        order.iter().map(|hit| hit.index).collect::<Vec<_>>(),
        [2, 0, 1]
    );
    assert_eq!(
        order
            .iter()
            .map(|hit| hit.cosine.unwrap())
            .collect::<Vec<_>>(),
        [0.9, 0.7, 0.7]
    );
    for hits in [
        vec![],
        vec![RerankHit {
            index: 3,
            score: 0.0,
        }],
        vec![RerankHit {
            index: 0,
            score: f64::NAN,
        }],
        vec![
            RerankHit {
                index: 0,
                score: 0.5,
            },
            RerankHit {
                index: 0,
                score: 0.4,
            },
        ],
    ] {
        assert!(matches!(mapped_order(hits, 2), Err(PackError::Corrupt(_))));
    }
}

#[test]
fn bge_policy_uses_explicit_catalog_identity_not_filename_or_handle_hint() {
    let mut pack = Pack {
        id: "bge-small-en-v1.5-q8-0".into(),
        kind: "gguf".into(),
        version: "test".into(),
        url: None,
        bytes: 17,
        sha256: "a".repeat(64),
        path: "packs/arbitrary.gguf".into(),
        deferred: false,
        extra: [("role".into(), "embedding".into())].into_iter().collect(),
    };
    validate_policy(&pack).unwrap();
    pack.id = "another-embedding-model".into();
    assert!(validate_policy(&pack).is_err());
    pack.id = "bge-small-en-v1.5-q8-0".into();
    pack.extra.insert("role".into(), "generation".into());
    assert!(validate_policy(&pack).is_err());
}

#[test]
fn frozen_query_evidence_has_exact_inputs_pin_and_unavailable_phase_counters() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("inputs.json");
    let p = passage();
    let input = QueryInput {
        prefix: PREFIX,
        query: format!("{PREFIX}{}", question().question_en),
        query_sha256: digest(format!("{PREFIX}{}", question().question_en).as_bytes()),
        documents: vec![p.text.clone()],
    };
    let pin = serde_json::json!({"id":"bge-small-en-v1.5-q8-0","sha256":"a".repeat(64)});
    freeze_inputs(&path, &input, std::slice::from_ref(&p), &pin).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["query_prefix"], PREFIX);
    assert_eq!(value["embedding_query"], input.query);
    assert_eq!(value["embedding_query_sha256"], input.query_sha256);
    assert_eq!(value["model"], pin);
    assert_eq!(value["documents"][0]["text"], p.text);
    assert_eq!(value.get("embedding_us"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("ranking_us"), Some(&serde_json::Value::Null));
    assert!(freeze_inputs(&path, &input, std::slice::from_ref(&p), &pin).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
