//! Optional local AI. Internal model paths must already be checksum-verified,
//! immutable app-owned installations; this is the same caller contract as
//! `pack_open_internal`, never a guess based on a path's spelling.

mod answer;
mod embedding;
mod generation;
use crate::gguf;
mod memory;
mod model;
mod rerank;

pub use answer::{
    answer, finish_answer, prepare_answer, Answer, AnswerFlags, AnswerOutcome, AnswerRequest,
    AnswerSource, AnswerTimings, PreparedAnswer,
};
pub use embedding::embed;
pub(crate) use generation::validate_messages;
pub use generation::{
    cancel, generate, generate_chat, ChatMessage, GenId, GenerationEnd, GenerationEvent,
    GenerationStats, TokenSink,
};
pub use model::{
    load_internal, load_managed, require_internal_storage, AiHandle, ModelInfo, ModelKind,
};
pub use rerank::{rerank, RerankHit};
