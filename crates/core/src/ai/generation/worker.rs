use std::num::NonZeroU32;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaChatMessage;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::DecodeError;

use super::super::model::{backend, Resources};
use super::{
    delivery, token_budget, utf8, ChatMessage, GenerationEnd, GenerationEvent, GenerationStats,
    Job, REGISTRY,
};
use crate::pack::PackError;

const PREFILL_BATCH_TOKENS: usize = 64;

fn native_error(error: impl std::fmt::Display) -> PackError {
    PackError::Corrupt(format!("AI generation: {error}"))
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

pub(super) fn delivery_result(job: &Job, result: Result<(), PackError>) -> Result<bool, PackError> {
    match result {
        Err(PackError::Mismatch(_))
            if job.flag.load(Ordering::Acquire)
                && job.failure.lock().is_ok_and(|failure| failure.is_none()) =>
        {
            Ok(false)
        }
        other => other.map(|()| true),
    }
}

pub(super) fn run(
    resources: Arc<Resources>,
    job: Arc<Job>,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
) {
    let mut stats = GenerationStats::default();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        infer(&resources, &job, &messages, max_tokens, &mut stats)
    }))
    .unwrap_or_else(|_| Err(native_error("native worker panic")));
    // infer's context and workspace have already dropped, before the model/file lease.
    drop(resources);
    finish(&job, stats, outcome);
}

pub(super) fn finish(
    job: &Arc<Job>,
    mut stats: GenerationStats,
    outcome: Result<GenerationEnd, PackError>,
) {
    let failure = job.failure.lock().ok().and_then(|failure| failure.clone());
    let outcome = failure.map_or(outcome, Err);
    stats.total_us = micros(job.started);
    let (reason, error) = match &outcome {
        Ok(reason) => (*reason, None),
        Err(error) => (GenerationEnd::Failed, Some(error.to_string())),
    };
    let result = outcome.map(|_| stats.clone());
    if let Ok(mut registry) = REGISTRY.lock() {
        registry.complete(job.id);
        // Queue End before a new Begin can be accepted, with the native slot already free.
        delivery::finish(
            Arc::clone(job),
            GenerationEvent::Finished {
                id: job.id,
                reason,
                stats,
                error,
            },
        );
    }
    if let Ok(mut complete) = job.result.lock() {
        *complete = Some(result);
        job.done.notify_all();
    }
}

fn decode(
    context: &mut LlamaContext<'_>,
    batch: &mut LlamaBatch<'_>,
    job: &Job,
    _phase: u8,
) -> Result<bool, PackError> {
    if job.flag.load(Ordering::Acquire) {
        return Ok(false);
    }
    #[cfg(test)]
    let before = {
        job.decoding.store(_phase, Ordering::Release);
        Instant::now()
    };
    let outcome = context.decode(batch);
    #[cfg(test)]
    {
        let after = Instant::now();
        job.decoding.store(0, Ordering::Release);
        let code = match &outcome {
            Ok(()) => 0,
            Err(DecodeError::Unknown(code)) => *code,
            _ => -1,
        };
        if let Ok(mut traces) = job.traces.lock() {
            if traces.len() < 1056 {
                traces.push(super::DecodeTrace {
                    phase: _phase,
                    before,
                    after,
                    code,
                });
            }
        }
    }
    match outcome {
        Ok(()) => Ok(!job.flag.load(Ordering::Acquire)),
        Err(DecodeError::Unknown(2)) if job.flag.load(Ordering::Acquire) => Ok(false),
        Err(error) => Err(native_error(error)),
    }
}

fn text(
    job: &Arc<Job>,
    text: &str,
    bytes: &mut usize,
    stats: &mut GenerationStats,
) -> Result<bool, PackError> {
    *bytes = bytes
        .checked_add(text.len())
        .filter(|total| *total <= 128 * 1024)
        .ok_or_else(|| native_error("output exceeds byte limit"))?;
    if !text.is_empty() && stats.first_token_us.is_none() {
        stats.first_token_us = Some(micros(job.started));
    }
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(1024);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        if !delivery_result(job, delivery::text(Arc::clone(job), rest[..end].to_owned()))? {
            return Ok(false);
        }
        rest = &rest[end..];
    }
    Ok(true)
}

fn infer(
    resources: &Resources,
    job: &Arc<Job>,
    messages: &[ChatMessage],
    max_tokens: u32,
    stats: &mut GenerationStats,
) -> Result<GenerationEnd, PackError> {
    if job.flag.load(Ordering::Acquire) {
        return Ok(GenerationEnd::Cancelled);
    }
    let model = &resources.native;
    let template = model.chat_template(None).map_err(native_error)?;
    let messages = messages
        .iter()
        .map(|message| {
            LlamaChatMessage::new(message.role.clone(), message.content.clone())
                .map_err(native_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rendered = model
        .apply_chat_template(&template, &messages, true)
        .map_err(native_error)?;
    if rendered.len() > 128 * 1024 {
        return Err(native_error("rendered prompt exceeds byte limit"));
    }
    if model.n_vocab() <= 0 {
        return Err(native_error("empty model vocabulary"));
    }
    let vocab = model.vocab();
    let tokens = vocab.tokenize(rendered.as_bytes(), true, true);
    let context_tokens = token_budget(
        tokens.len(),
        max_tokens,
        resources.info.training_context_tokens,
    )?;
    stats.input_tokens = u32::try_from(tokens.len()).map_err(native_error)?;
    let threads = std::thread::available_parallelism()
        .map_err(native_error)?
        .get()
        .min(4) as i32;
    // The model's existing reservation already holds this workspace allowance.
    let mut workspace = resources.workspace()?;
    let params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(context_tokens))
        .with_n_batch(PREFILL_BATCH_TOKENS as u32)
        .with_n_ubatch(PREFILL_BATCH_TOKENS as u32)
        .with_n_seq_max(1)
        .with_n_threads(threads)
        .with_n_threads_batch(threads)
        .with_embeddings(false)
        .with_offload_kqv(false);
    // Declared after workspace: native context dies before its allowance is restored.
    let mut context = model
        .new_context(backend()?, params)
        .map_err(native_error)?;
    workspace.loaded()?;
    if !context.is_causal() {
        return Err(PackError::Mismatch(
            "generation requires native causal attention".into(),
        ));
    }
    context.set_abort_flag(Arc::clone(&job.flag));
    let mut batch = LlamaBatch::new(PREFILL_BATCH_TOKENS, 1);
    stats.setup_us = micros(job.started);
    let prefill = Instant::now();
    for (chunk_index, chunk) in tokens.chunks(PREFILL_BATCH_TOKENS).enumerate() {
        batch.clear();
        for (offset, token) in chunk.iter().enumerate() {
            let position = chunk_index * PREFILL_BATCH_TOKENS + offset;
            batch
                .add(
                    *token,
                    i32::try_from(position).map_err(native_error)?,
                    &[0],
                    position + 1 == tokens.len(),
                )
                .map_err(native_error)?;
        }
        if !decode(&mut context, &mut batch, job, 1)? {
            stats.prefill_us = micros(prefill);
            return Ok(GenerationEnd::Cancelled);
        }
    }
    stats.prefill_us = micros(prefill);
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::top_k(20),
        LlamaSampler::top_p(0.8, 1),
        LlamaSampler::temp(0.7),
        LlamaSampler::dist(42),
    ]);
    let decoding = Instant::now();
    let mut pending = Vec::new();
    let mut output_bytes = 0;
    let mut reason = GenerationEnd::Limit;
    for output in 0..max_tokens {
        if job.flag.load(Ordering::Acquire) {
            reason = GenerationEnd::Cancelled;
            break;
        }
        // Successful decode explicitly requested its last logits before any native sampling.
        if context.get_logits_ith(-1).is_empty() {
            return Err(native_error("empty decoder logits"));
        }
        let token = sampler.sample(&context, -1);
        if token.0 < 0 || token.0 >= model.n_vocab() {
            return Err(native_error("invalid sampled token"));
        }
        if vocab.is_eog(token) {
            reason = GenerationEnd::Eos;
            break;
        }
        stats.output_tokens += 1;
        let piece = vocab.token_to_piece(token, false, None);
        let decoded = utf8(&mut pending, &piece, false)?;
        if !text(job, &decoded, &mut output_bytes, stats)? {
            reason = GenerationEnd::Cancelled;
            break;
        }
        if output + 1 == max_tokens {
            break;
        }
        batch.clear();
        batch
            .add(
                token,
                i32::try_from(tokens.len() + output as usize).map_err(native_error)?,
                &[0],
                true,
            )
            .map_err(native_error)?;
        if !decode(&mut context, &mut batch, job, 2)? {
            reason = GenerationEnd::Cancelled;
            break;
        }
    }
    if reason != GenerationEnd::Cancelled {
        let tail = utf8(&mut pending, &[], true)?;
        if !text(job, &tail, &mut output_bytes, stats)? {
            reason = GenerationEnd::Cancelled;
        }
    }
    stats.decode_us = micros(decoding);
    Ok(reason)
}
