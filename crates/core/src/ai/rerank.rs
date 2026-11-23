//! Shared bounded reranking; callers supply model-specific query formatting.

use std::sync::Arc;

use super::{embed, AiHandle};
use crate::pack::PackError;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RerankHit {
    /// Zero-based original BM25 candidate position.
    pub index: u32,
    pub score: f64,
}

/// Reranks 1..20 documents using one embedding call for query plus documents.
/// Existing embedding byte/token limits and close cancellation apply; no text is trimmed.
pub fn rerank(
    handle: Arc<AiHandle>,
    query: String,
    documents: Vec<String>,
) -> Result<Vec<RerankHit>, PackError> {
    let count = documents.len();
    if !(1..=20).contains(&count) {
        return Err(PackError::Mismatch(
            "AI rerank: expected 1..20 documents".into(),
        ));
    }
    let mut inputs = Vec::with_capacity(count + 1);
    inputs.push(query);
    inputs.extend(documents);
    let vectors = embed(handle, inputs)?;
    let (query, documents) = vectors
        .split_first()
        .ok_or_else(|| corrupt("missing query vector"))?;
    if documents.len() != count {
        return Err(corrupt("embedding count differs from input count"));
    }
    cosine_order(query, documents)
}

fn corrupt(message: &str) -> PackError {
    PackError::Corrupt(format!("AI rerank: {message}"))
}

fn cosine_order(query: &[f32], documents: &[Vec<f32>]) -> Result<Vec<RerankHit>, PackError> {
    if !(1..=4096).contains(&query.len()) || documents.len() > 20 {
        return Err(corrupt("invalid vector dimensions/count"));
    }
    let norm = |values: &[f32]| -> Result<f64, PackError> {
        if values.len() != query.len() || values.iter().any(|value| !value.is_finite()) {
            return Err(corrupt("invalid vector shape/value"));
        }
        let squared: f64 = values.iter().map(|value| f64::from(*value).powi(2)).sum();
        if squared <= 0.0 || !squared.is_finite() {
            return Err(corrupt("zero/nonfinite vector norm"));
        }
        Ok(squared.sqrt())
    };
    let query_norm = norm(query)?;
    let mut scores = Vec::with_capacity(documents.len());
    for (index, document) in documents.iter().enumerate() {
        let divisor = query_norm * norm(document)?;
        let dot: f64 = query
            .iter()
            .zip(document)
            .map(|(left, right)| f64::from(*left) * f64::from(*right))
            .sum();
        scores.push((index, (dot / divisor).clamp(-1.0, 1.0)));
    }
    scores.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    Ok(scores
        .into_iter()
        .map(|(index, score)| RerankHit {
            index: index as u32,
            score,
        })
        .collect())
}

#[cfg(test)]
#[path = "rerank/tests.rs"]
mod tests;
