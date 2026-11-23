//! BAAI-specific caller formatting; all embedding and ranking math stays in Core.

use super::*;
use crate::{
    ai::model::{validate_pack, VerifiedModel},
    manifest::Pack,
};
use tonalli_core::ai::RerankHit;

pub(super) const PREFIX: &str = "Represent this sentence for searching relevant passages: ";

#[derive(Debug, Serialize)]
pub(super) struct QueryInput {
    prefix: &'static str,
    query: String,
    pub(super) query_sha256: String,
    documents: Vec<String>,
}

pub(super) struct Series {
    model: VerifiedModel,
}

pub(super) fn validate_policy(pack: &Pack) -> Result<(), PackError> {
    if pack.id != "bge-small-en-v1.5-q8-0" {
        return Err(invalid(
            "query prefix policy requires the explicit BGE catalog id",
        ));
    }
    validate_pack(pack, tonalli_core::ai::ModelKind::Embedding)
}

pub(super) fn query_input(
    question: &Question,
    passages: &[Passage],
) -> Result<QueryInput, PackError> {
    if !(1..=20).contains(&passages.len()) || question.retrieval_query_en != question.question_en {
        return Err(invalid(
            "BGE requires unchanged full English question and 1..20 candidates",
        ));
    }
    let query = format!("{PREFIX}{}", question.question_en);
    Ok(QueryInput {
        prefix: PREFIX,
        query_sha256: digest(query.as_bytes()),
        query,
        documents: passages
            .iter()
            .map(|passage| passage.text.clone())
            .collect(),
    })
}

fn mapped_order(hits: Vec<RerankHit>, count: usize) -> Result<Vec<Ranked>, PackError> {
    if !(1..=20).contains(&count) || hits.len() != count {
        return Err(corrupt("Core rank count differs from candidates"));
    }
    let mut seen = HashSet::new();
    hits.into_iter()
        .map(|hit| {
            let index = usize::try_from(hit.index).map_err(|error| corrupt(&error.to_string()))?;
            if index >= count
                || !seen.insert(index)
                || !hit.score.is_finite()
                || !(-1.0..=1.0).contains(&hit.score)
            {
                return Err(corrupt("invalid Core rank index/score"));
            }
            Ok(Ranked {
                index,
                cosine: Some(hit.score),
            })
        })
        .collect()
}

pub(super) fn freeze_inputs(
    path: &Path,
    input: &QueryInput,
    passages: &[Passage],
    pin: &serde_json::Value,
) -> Result<(), PackError> {
    if input.documents.len() != passages.len()
        || input
            .documents
            .iter()
            .zip(passages)
            .any(|(text, passage)| text != &passage.text)
        || digest(input.query.as_bytes()) != input.query_sha256
    {
        return Err(corrupt(
            "query/candidate evidence differs from actual inputs",
        ));
    }
    write_json(
        path,
        &serde_json::json!({"query_prefix":input.prefix,"embedding_query":input.query,
        "embedding_query_sha256":input.query_sha256,"documents":passages,"model":pin,
        "embedding_us":null,"ranking_us":null,"rerank_us":null,"captured_before_core_call":true}),
    )
}

impl Series {
    pub(super) fn open(
        base: &Path,
        pack: &Pack,
        on_verified: impl FnOnce(&serde_json::Value) -> Result<(), PackError>,
    ) -> Result<Self, PackError> {
        validate_policy(pack)?;
        Ok(Self {
            model: VerifiedModel::load(
                base,
                pack,
                tonalli_core::ai::ModelKind::Embedding,
                on_verified,
            )?,
        })
    }

    pub(super) fn pin(&self) -> &serde_json::Value {
        &self.model.pin
    }

    pub(super) fn close(&self) -> Result<(), PackError> {
        self.model.close()
    }

    pub(super) fn rerank(&self, input: &QueryInput) -> Result<Vec<Ranked>, PackError> {
        let count = input.documents.len();
        let hits = tonalli_core::ai::rerank(
            std::sync::Arc::clone(&self.model.handle),
            input.query.clone(),
            input.documents.clone(),
        )?;
        mapped_order(hits, count)
    }
}

#[cfg(test)]
#[path = "bge_tests.rs"]
mod tests;
