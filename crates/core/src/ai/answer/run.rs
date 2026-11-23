//! Blocking answer orchestration on the caller's worker, using existing generation jobs.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use super::{
    candidate_sources, collector::Collector, finish_answer, invalid, micros, numbered_prompt,
    prepare_answer, Answer, AnswerSource, PreparedAnswer,
};
use crate::{
    ai::{AiHandle, GenId, GenerationEvent, GenerationStats, TokenSink},
    index::Index,
    pack::PackError,
    zim::Zim,
};

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AnswerRequest {
    pub question: String,
    pub retrieval_query: String,
    pub embedding_query: Option<String>,
    pub max_tokens: u32,
}

impl AnswerRequest {
    fn validate(&self, has_embedding: bool) -> Result<(), PackError> {
        let bounded = |text: &str, bytes: usize| {
            !text.trim().is_empty() && text.len() <= bytes && !text.contains('\0')
        };
        if !bounded(&self.question, 4096)
            || !bounded(&self.retrieval_query, 4096)
            || !(1..=1024).contains(&self.max_tokens)
            || has_embedding != self.embedding_query.is_some()
            || self
                .embedding_query
                .as_ref()
                .is_some_and(|query| !bounded(query, 16_384))
        {
            return Err(invalid(
                "invalid bounded request or embedding option/query pairing",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AnswerTimings {
    pub retrieval_us: u64,
    pub extraction_us: u64,
    pub rerank_us: Option<u64>,
    /// First raw Text callback before forwarding, relative to answer request start; not useful/UI text.
    pub first_raw_received_us: Option<u64>,
    pub total_us: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AnswerOutcome {
    NoSources {
        timings: AnswerTimings,
    },
    Observed {
        answer: Answer,
        id: GenId,
        stats: GenerationStats,
        error: Option<String>,
        terminal_forwarded: bool,
        timings: AnswerTimings,
    },
    Deadline {
        id: Option<GenId>,
        raw_text: String,
        sources: Vec<AnswerSource>,
        /// Only an actually observed Finished event, including its real stats, may appear here.
        terminal: Option<GenerationEvent>,
        terminal_forwarded: bool,
        timings: AnswerTimings,
    },
}

impl AnswerOutcome {
    pub fn timings(&self) -> &AnswerTimings {
        match self {
            Self::NoSources { timings }
            | Self::Observed { timings, .. }
            | Self::Deadline { timings, .. } => timings,
        }
    }
}

/// No global model selection; None explicitly requests BM25-only retrieval.
pub fn answer(
    index: &Index,
    zim: &Zim,
    generation: Arc<AiHandle>,
    embedding: Option<Arc<AiHandle>>,
    request: AnswerRequest,
    sink: Arc<dyn TokenSink>,
) -> Result<AnswerOutcome, PackError> {
    let started = Instant::now();
    request.validate(embedding.is_some())?;
    let mut rerank_us = None;
    let prepared = if let Some(embedding) = embedding {
        let (candidates, retrieval_us, extraction_us) =
            candidate_sources(index, zim, &request.retrieval_query, 20)?;
        let mut sources = Vec::new();
        if !candidates.is_empty() {
            let query = request
                .embedding_query
                .ok_or_else(|| invalid("missing explicit embedding query"))?;
            let reranking = Instant::now();
            let order = crate::ai::rerank(
                embedding,
                query,
                candidates
                    .iter()
                    .map(|source| source.excerpt.clone())
                    .collect(),
            )?;
            rerank_us = Some(micros(reranking));
            for hit in order.into_iter().take(3) {
                let mut source = candidates.get(hit.index as usize).cloned().ok_or_else(|| {
                    PackError::Corrupt("AI answer rerank index outside candidates".into())
                })?;
                source.number = sources.len() as u32 + 1;
                sources.push(source);
            }
        }
        PreparedAnswer {
            prompt: numbered_prompt(&request.question, &sources)?,
            sources,
            retrieval_us,
            extraction_us,
        }
    } else {
        prepare_answer(index, zim, &request.question, &request.retrieval_query)?
    };
    let mut timings = AnswerTimings {
        retrieval_us: prepared.retrieval_us,
        extraction_us: prepared.extraction_us,
        rerank_us,
        first_raw_received_us: None,
        total_us: micros(started),
    };
    let Some(prompt) = prepared.prompt() else {
        return Ok(AnswerOutcome::NoSources { timings });
    };
    let collector = Arc::new(Collector::new(sink, started));
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(90))
        .ok_or_else(|| invalid("callback deadline overflow"))?;
    let id = crate::ai::generate(
        generation,
        prompt.to_owned(),
        request.max_tokens,
        collector.clone(),
    )?;
    if let Err(error) = collector.returned(id) {
        let _ = crate::ai::cancel(id);
        return Err(error);
    }
    let collected = collector.wait(deadline);
    timings.first_raw_received_us = collected.first_raw_received_us;
    let cancellation = collected.cancel_id.map(crate::ai::cancel).transpose();
    if let Some(error) = collected.failure {
        return Err(error);
    }
    cancellation?;
    if collected.deadline {
        timings.total_us = micros(started);
        return Ok(AnswerOutcome::Deadline {
            id: collected.id,
            raw_text: collected.raw_text,
            sources: prepared.sources,
            terminal: collected.terminal,
            terminal_forwarded: collected.terminal_forwarded,
            timings,
        });
    }
    match collected.terminal {
        Some(GenerationEvent::Finished {
            id,
            reason,
            stats,
            error,
        }) => {
            let answer = finish_answer(&prepared, collected.raw_text, reason)?;
            timings.total_us = micros(started);
            Ok(AnswerOutcome::Observed {
                answer,
                id,
                stats,
                error,
                terminal_forwarded: collected.terminal_forwarded,
                timings,
            })
        }
        _ => Err(PackError::Corrupt(
            "AI answer has no observed terminal".into(),
        )),
    }
}

#[cfg(test)]
mod tests;
