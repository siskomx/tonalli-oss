//! Bounded M1 CPU generation with cancellation independent of sink callbacks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use crate::pack::PackError;

use super::{AiHandle, ModelKind};

mod delivery;
mod worker;

pub type GenId = u64;

/// Role-preserving input to the model's own chat template. Never includes hidden reasoning.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub(crate) fn validate_messages(messages: &[ChatMessage]) -> Result<(), PackError> {
    let valid = !messages.is_empty()
        && messages.len() <= 33
        && messages.len() % 2 == 1
        && messages.iter().enumerate().all(|(index, message)| {
            message.role == if index % 2 == 0 { "user" } else { "assistant" }
                && !message.content.trim().is_empty()
                && !message.content.contains('\0')
        })
        && messages
            .iter()
            .try_fold(0usize, |total, message| {
                total
                    .checked_add(message.content.len())
                    .filter(|total| *total <= 65_536)
            })
            .is_some();
    if valid {
        Ok(())
    } else {
        Err(PackError::Mismatch(
            "conversation exceeds role/input bounds".into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum GenerationEnd {
    Eos,
    Limit,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct GenerationStats {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub setup_us: u64,
    pub prefill_us: u64,
    /// First nonempty UTF8 text ready for enqueue, not callback receipt or UI visibility.
    pub first_token_us: Option<u64>,
    /// Post-prefill generation wall time, including sampling, UTF8 and enqueue waits.
    pub decode_us: u64,
    pub total_us: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum GenerationEvent {
    Started {
        id: GenId,
    },
    Text {
        id: GenId,
        text: String,
    },
    Finished {
        id: GenId,
        reason: GenerationEnd,
        stats: GenerationStats,
        error: Option<String>,
    },
}

/// Callbacks must enqueue UI work promptly. Core isolates callbacks from native
/// inference; cancellation does not wait for a blocked foreign callback.
#[cfg_attr(feature = "ffi", uniffi::export(foreign))]
pub trait TokenSink: Send + Sync + 'static {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError>;
}

struct Job {
    id: GenId,
    flag: Arc<AtomicBool>,
    failure: Mutex<Option<PackError>>,
    result: Mutex<Option<Result<GenerationStats, PackError>>>,
    done: Condvar,
    started: Instant,
    sink_broken: AtomicBool,
    delivery_started: AtomicBool,
    delivery_ended: AtomicBool,
    #[cfg(test)]
    decoding: std::sync::atomic::AtomicU8,
    #[cfg(test)]
    traces: Mutex<Vec<DecodeTrace>>,
    #[cfg(test)]
    cancelled_at: Mutex<Option<Instant>>,
}

#[cfg(test)]
struct DecodeTrace {
    phase: u8,
    before: Instant,
    after: Instant,
    code: i32,
}

#[derive(Default)]
struct Registry {
    next: GenId,
    active: Option<Arc<Job>>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    next: 0,
    active: None,
});

impl Registry {
    fn reserve(&mut self) -> Result<Arc<Job>, PackError> {
        if self.active.is_some() {
            return Err(PackError::Mismatch("AI generation is busy".into()));
        }
        let id = self
            .next
            .checked_add(1)
            .ok_or_else(|| PackError::Corrupt("AI generation IDs exhausted".into()))?;
        let job = Arc::new(Job {
            id,
            flag: Arc::new(AtomicBool::new(false)),
            failure: Mutex::new(None),
            result: Mutex::new(None),
            done: Condvar::new(),
            started: Instant::now(),
            sink_broken: AtomicBool::new(false),
            delivery_started: AtomicBool::new(false),
            delivery_ended: AtomicBool::new(false),
            #[cfg(test)]
            decoding: std::sync::atomic::AtomicU8::new(0),
            #[cfg(test)]
            traces: Mutex::new(Vec::new()),
            #[cfg(test)]
            cancelled_at: Mutex::new(None),
        });
        self.next = id;
        self.active = Some(Arc::clone(&job));
        Ok(job)
    }
    fn cancel(&self, id: GenId) {
        if let Some(job) = self.active.as_ref().filter(|job| job.id == id) {
            #[cfg(test)]
            if let Ok(mut timestamp) = job.cancelled_at.lock() {
                *timestamp = Some(Instant::now());
            }
            job.flag.store(true, Ordering::Release);
        }
    }
    fn complete(&mut self, id: GenId) {
        if self.active.as_ref().is_some_and(|job| job.id == id) {
            self.active.take();
        }
    }
}

impl Job {
    fn fail(&self, error: PackError) {
        if let Ok(mut failure) = self.failure.lock() {
            if failure.is_none() {
                *failure = Some(error);
            }
        }
        self.flag.store(true, Ordering::Release);
    }
}

pub fn generate(
    handle: Arc<AiHandle>,
    prompt: String,
    max_tokens: u32,
    sink: Arc<dyn TokenSink>,
) -> Result<GenId, PackError> {
    generate_chat(
        handle,
        vec![ChatMessage {
            role: "user".into(),
            content: prompt,
        }],
        max_tokens,
        sink,
    )
}

/// Uses the same bounded worker, cancellation and native context as single-turn generation.
pub fn generate_chat(
    handle: Arc<AiHandle>,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    sink: Arc<dyn TokenSink>,
) -> Result<GenId, PackError> {
    validate_messages(&messages)?;
    let mut guard = handle
        .inner
        .lock()
        .map_err(|_| PackError::Corrupt("AI handle lock poisoned".into()))?;
    let open = guard
        .as_mut()
        .ok_or_else(|| PackError::NotFound("closed AI model".into()))?;
    let resources = &open.resources;
    for message in &messages {
        validate_request(
            resources.info.kind,
            resources.native.has_encoder(),
            resources.native.has_decoder(),
            &message.content,
            max_tokens,
        )?;
    }
    resources
        .native
        .chat_template(None)
        .map_err(|_| PackError::Mismatch("generation requires a model chat template".into()))?;
    let job = REGISTRY
        .lock()
        .map_err(|_| PackError::Corrupt("AI job lock poisoned".into()))?
        .reserve()?;
    if let Err(error) = delivery::begin(Arc::clone(&job), sink) {
        REGISTRY
            .lock()
            .map_err(|_| PackError::Corrupt("AI job lock poisoned".into()))?
            .complete(job.id);
        return Err(error);
    }
    open.cancel = Some(Arc::downgrade(&job.flag));
    let resources = Arc::clone(&open.resources);
    // Nothing on the native worker owns the handle or foreign sink.
    drop(guard);
    let id = job.id;
    let worker_job = Arc::clone(&job);
    if let Err(error) = std::thread::Builder::new()
        .name("ai-generation".into())
        .spawn(move || worker::run(resources, worker_job, messages, max_tokens))
    {
        worker::finish(
            &job,
            GenerationStats::default(),
            Err(PackError::Io(format!("AI worker start: {error}"))),
        );
    }
    Ok(id)
}

pub fn cancel(id: GenId) -> Result<(), PackError> {
    REGISTRY
        .lock()
        .map_err(|_| PackError::Corrupt("AI job lock poisoned".into()))?
        .cancel(id);
    Ok(())
}

fn validate_request(
    kind: ModelKind,
    has_encoder: bool,
    has_decoder: bool,
    prompt: &str,
    max_tokens: u32,
) -> Result<(), PackError> {
    if kind != ModelKind::Generation || has_encoder || !has_decoder {
        return Err(PackError::Mismatch(
            "model is not a supported causal decoder".into(),
        ));
    }
    if prompt.trim().is_empty()
        || prompt.len() > 65_536
        || prompt.contains('\0')
        || !(1..=1024).contains(&max_tokens)
    {
        return Err(PackError::Mismatch(
            "generation request exceeds input/output bounds".into(),
        ));
    }
    Ok(())
}

fn token_budget(input: usize, output: u32, training_context: u32) -> Result<u32, PackError> {
    let context = training_context.min(4096);
    let input =
        u32::try_from(input).map_err(|_| PackError::Mismatch("too many prompt tokens".into()))?;
    if input == 0
        || input > 2048
        || input
            .checked_add(output)
            .is_none_or(|total| total > context)
    {
        return Err(PackError::Mismatch(
            "prompt/output exceeds bounded model context".into(),
        ));
    }
    Ok(context)
}

fn utf8(pending: &mut Vec<u8>, bytes: &[u8], finished: bool) -> Result<String, PackError> {
    if pending.len() > 3 || bytes.len() > 16_384 {
        return Err(PackError::Corrupt("oversized token piece".into()));
    }
    pending.extend_from_slice(bytes);
    let mut output = String::new();
    let mut consumed = 0;
    while consumed < pending.len() {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(text) => {
                output.push_str(text);
                consumed = pending.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                output.push_str(
                    std::str::from_utf8(&pending[consumed..consumed + valid])
                        .map_err(|_| PackError::Corrupt("invalid UTF8 decoder state".into()))?,
                );
                consumed += valid;
                if let Some(invalid) = error.error_len() {
                    output.push('\u{fffd}');
                    consumed += invalid;
                } else {
                    break;
                }
            }
        }
    }
    pending.drain(..consumed);
    if finished && !pending.is_empty() {
        output.push('\u{fffd}');
        pending.clear();
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
