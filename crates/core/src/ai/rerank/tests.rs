use super::*;

#[test]
fn cosine_uses_actual_norms_and_preserves_bm25_order_for_ties() {
    let ranked = cosine_order(
        &[3.0, 4.0],
        &[
            vec![30.0, 40.0],
            vec![3.0, 4.0],
            vec![4.0, -3.0],
            vec![-3.0, -4.0],
        ],
    )
    .unwrap();
    assert_eq!(
        ranked.iter().map(|row| row.index).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!((ranked[0].score - 1.0).abs() < 0.00001);
    assert!(ranked[2].score.abs() < 0.00001);
    assert!((ranked[3].score + 1.0).abs() < 0.00001);
    assert!(cosine_order(&[1.0], &[]).unwrap().is_empty());
}

#[test]
fn cosine_rejects_invalid_native_vectors_without_panicking() {
    for (query, documents) in [
        (vec![], vec![vec![1.0]]),
        (vec![1.0], vec![vec![1.0, 2.0]]),
        (vec![0.0], vec![vec![1.0]]),
        (vec![1.0], vec![vec![0.0]]),
        (vec![f32::NAN], vec![vec![1.0]]),
        (vec![1.0], vec![vec![f32::INFINITY]]),
        (vec![1.0], vec![vec![1.0]; 21]),
        (vec![1.0; 4097], vec![vec![1.0; 4097]]),
    ] {
        assert!(matches!(
            cosine_order(&query, &documents),
            Err(PackError::Corrupt(_))
        ));
    }
}

#[test]
fn large_finite_vectors_do_not_overflow_float32_products() {
    let ranked = cosine_order(&[f32::MAX, f32::MAX], &[vec![f32::MAX, f32::MAX]]).unwrap();
    assert_eq!(ranked.len(), 1);
    assert!(ranked[0].score.is_finite() && (ranked[0].score - 1.0).abs() < 0.00001);
}

#[test]
fn document_count_is_rejected_before_touching_the_closed_native_handle() {
    for documents in [vec![], vec!["document".into(); 21]] {
        assert!(matches!(
            rerank(AiHandle::closed_for_test(), "query".into(), documents),
            Err(PackError::Mismatch(_))
        ));
    }
}

#[test]
fn valid_input_preserves_the_embedding_closed_handle_error() {
    assert!(matches!(
        rerank(
            AiHandle::closed_for_test(),
            "query".into(),
            vec!["document".into()]
        ),
        Err(PackError::NotFound(_))
    ));
}

#[test]
#[ignore = "requires verified immutable internal BGE and a separate guarded real-model lease"]
fn real_bge_rerank_returns_owned_indices_and_identical_document_ties() {
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").expect("verified internal BGE path");
    let handle =
        crate::ai::load_internal(std::path::Path::new(&path), crate::ai::ModelKind::Embedding)
            .unwrap();
    let text = "A red fox crosses a snowy field.".to_owned();
    let ranked = rerank(
        handle.clone(),
        text.clone(),
        vec![
            text.clone(),
            "Mitochondria produce cellular energy.".into(),
            text,
        ],
    )
    .unwrap();
    assert_eq!(ranked.len(), 3);
    assert_eq!(
        ranked.iter().map(|hit| hit.index).collect::<Vec<_>>(),
        [0, 2, 1]
    );
    assert!(ranked
        .iter()
        .all(|hit| hit.score.is_finite() && (-1.0..=1.0).contains(&hit.score)));
    assert!((ranked[0].score - ranked[1].score).abs() < 0.00001);
    assert!((ranked[0].score - 1.0).abs() < 0.00001);
    handle.close().unwrap();
}
