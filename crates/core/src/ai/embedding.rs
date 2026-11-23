//! Bounded, synchronous embeddings from an already-owned internal model.

use std::num::NonZeroU32;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::{DecodeError, EncodeError};

use super::model::{backend, Resources};
use super::{AiHandle, ModelKind};
use crate::pack::PackError;

/// Returns one finite L2-normalized vector per input, in input order.
/// Inputs: 1..21 nonblank texts, at most 16KiB each / 256KiB total, and at most
/// min(512, training context) actual tokens each. Inputs are never silently trimmed.
/// Native noncausal CLS/Mean/Last pooling and at most 4096 dimensions are required.
/// Closing the model cancels this call; no generation ID or callback is created.
/// Native context-setup cancellation latency has not been established.
pub fn embed(handle: Arc<AiHandle>, texts: Vec<String>) -> Result<Vec<Vec<f32>>, PackError> {
    input_bounds(&texts)?;
    let (resources, flag, dimensions) = {
        let mut guard = handle
            .inner
            .lock()
            .map_err(|_| native_error("model lock poisoned"))?;
        let open = guard
            .as_mut()
            .ok_or_else(|| PackError::NotFound("closed AI model".into()))?;
        let dimensions = model_bounds(
            open.resources.info.kind,
            open.resources.info.embedding_dimensions,
        )?;
        let flag = claim_abort(&mut open.cancel)?;
        (Arc::clone(&open.resources), flag, dimensions)
    };
    // Native work owns resources and its abort flag, never the handle's mutex.
    match catch_unwind(AssertUnwindSafe(|| {
        infer(&resources, &flag, &texts, dimensions)
    })) {
        Ok(result) => result,
        Err(_) => Err(native_error("native embedding panicked")),
    }
}

fn mismatch(message: &str) -> PackError {
    PackError::Mismatch(format!("AI embedding: {message}"))
}

fn native_error(error: impl std::fmt::Display) -> PackError {
    PackError::Corrupt(format!("AI embedding: {error}"))
}

fn input_bounds(texts: &[String]) -> Result<(), PackError> {
    if !(1..=21).contains(&texts.len()) {
        return Err(mismatch("expected 1..21 texts"));
    }
    let mut total = 0_usize;
    for text in texts {
        if text.len() > 16_384 || text.trim().is_empty() || text.contains('\0') {
            return Err(mismatch("text is empty, contains NUL, or exceeds 16KiB"));
        }
        total = total
            .checked_add(text.len())
            .filter(|bytes| *bytes <= 256 * 1024)
            .ok_or_else(|| mismatch("aggregate text exceeds 256KiB"))?;
    }
    Ok(())
}

fn model_bounds(kind: ModelKind, dimensions: u32) -> Result<usize, PackError> {
    if kind != ModelKind::Embedding {
        return Err(mismatch("requires an embedding model handle"));
    }
    if !(1..=4096).contains(&dimensions) {
        return Err(native_error("native dimensions exceed 1..4096"));
    }
    usize::try_from(dimensions).map_err(native_error)
}

fn token_budget(tokens: usize, training_context: u32) -> Result<u32, PackError> {
    if training_context == 0 {
        return Err(native_error("zero training context"));
    }
    let limit = training_context.min(512);
    if tokens == 0 || tokens > limit as usize {
        return Err(mismatch("text exceeds bounded native token context"));
    }
    Ok(limit)
}

fn pooling_bounds(causal: bool, pooling: LlamaPoolingType) -> Result<(), PackError> {
    if causal
        || !matches!(
            pooling,
            LlamaPoolingType::Mean | LlamaPoolingType::Cls | LlamaPoolingType::Last
        )
    {
        return Err(mismatch(
            "requires native noncausal CLS/Mean/Last vector pooling",
        ));
    }
    Ok(())
}

fn normalized(values: &[f32], dimensions: usize) -> Result<Vec<f32>, PackError> {
    if !(1..=4096).contains(&dimensions)
        || values.len() != dimensions
        || values.iter().any(|value| !value.is_finite())
    {
        return Err(native_error("invalid vector width or nonfinite value"));
    }
    let squared: f64 = values.iter().map(|value| f64::from(*value).powi(2)).sum();
    if !squared.is_finite() || squared <= 0.0 {
        return Err(native_error("zero or nonfinite vector norm"));
    }
    let norm = squared.sqrt();
    Ok(values
        .iter()
        .map(|value| (f64::from(*value) / norm) as f32)
        .collect())
}

fn claim_abort(slot: &mut Option<Weak<AtomicBool>>) -> Result<Arc<AtomicBool>, PackError> {
    if slot.as_ref().and_then(Weak::upgrade).is_some() {
        return Err(mismatch("model context is busy"));
    }
    let flag = Arc::new(AtomicBool::new(false));
    *slot = Some(Arc::downgrade(&flag));
    Ok(flag)
}

fn check_cancel(flag: &AtomicBool) -> Result<(), PackError> {
    if flag.load(Ordering::Acquire) {
        return Err(mismatch("cancelled"));
    }
    Ok(())
}

fn infer(
    resources: &Resources,
    flag: &Arc<AtomicBool>,
    texts: &[String],
    dimensions: usize,
) -> Result<Vec<Vec<f32>>, PackError> {
    check_cancel(flag)?;
    let model = &resources.native;
    let vocabulary_size = model.n_vocab();
    if vocabulary_size <= 0 {
        return Err(native_error("empty native vocabulary"));
    }
    let encoder = model.has_encoder();
    if encoder == model.has_decoder() {
        return Err(mismatch("requires one native encode or decode stage"));
    }
    let context_tokens = resources.info.training_context_tokens.min(512);
    let vocabulary = model.vocab();
    let mut tokenized = Vec::with_capacity(texts.len());
    for text in texts {
        check_cancel(flag)?;
        let tokens = vocabulary.tokenize(text.as_bytes(), true, false);
        token_budget(tokens.len(), resources.info.training_context_tokens)?;
        if tokens
            .iter()
            .any(|token| token.0 < 0 || token.0 >= vocabulary_size)
        {
            return Err(native_error("tokenizer returned an invalid token"));
        }
        tokenized.push(tokens);
    }
    check_cancel(flag)?;
    let threads = std::thread::available_parallelism()
        .map_err(native_error)?
        .get()
        .min(4) as i32;
    // Reuse the model's reserved workspace; never charge its resident weights twice.
    let mut workspace = resources.workspace()?;
    let params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(context_tokens))
        .with_n_batch(context_tokens)
        .with_n_ubatch(context_tokens)
        .with_n_seq_max(1)
        .with_n_threads(threads)
        .with_n_threads_batch(threads)
        .with_embeddings(true)
        .with_offload_kqv(false);
    // Context must be destroyed before the workspace reservation becomes uncommitted.
    let mut context = model
        .new_context(backend()?, params)
        .map_err(native_error)?;
    workspace.loaded()?;
    pooling_bounds(context.is_causal(), context.pooling_type())?;
    context.set_abort_flag(Arc::clone(flag));
    let mut batch = LlamaBatch::new(context_tokens as usize, 1);
    let mut vectors = Vec::with_capacity(texts.len());
    for tokens in &tokenized {
        check_cancel(flag)?;
        // Noncausal attention requires the entire sequence in one physical batch.
        if tokens.len() > context.n_ubatch() as usize || tokens.len() > context.n_batch() as usize {
            return Err(native_error("native batch does not fit bounded sequence"));
        }
        context.clear_kv_cache();
        batch.clear();
        batch.add_sequence(tokens, 0, true).map_err(native_error)?;
        if encoder {
            match context.encode(&mut batch) {
                Err(EncodeError::Unknown(2)) if flag.load(Ordering::Acquire) => {
                    return Err(mismatch("cancelled"))
                }
                result => result.map_err(native_error)?,
            }
        } else {
            #[cfg(test)]
            let before = tests::before_decode();
            let outcome = context.decode(&mut batch);
            #[cfg(test)]
            tests::after_decode(before, &outcome);
            match outcome {
                Err(DecodeError::Unknown(2)) if flag.load(Ordering::Acquire) => {
                    return Err(mismatch("cancelled"))
                }
                result => result.map_err(native_error)?,
            }
        }
        check_cancel(flag)?;
        // Copy the pooled sequence vector before the next native batch overwrites it.
        vectors.push(normalized(
            context.embeddings_seq_ith(0).map_err(native_error)?,
            dimensions,
        )?);
    }
    check_cancel(flag)?;
    Ok(vectors)
}

#[cfg(test)]
#[path = "embedding/tests.rs"]
mod tests;
