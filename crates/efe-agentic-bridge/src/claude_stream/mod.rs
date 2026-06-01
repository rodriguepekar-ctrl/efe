// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : Anthropic SSE streaming subsystem.
//!
//! See `docs/architecture/EFE_M50_N27_STREAMING_ANTHROPIC_RETRY_PROVENANCE_ARCHITECTURE.md`
//! § 3 (protocol deep dive), § 7 (component map), § 9.1 (parser
//! state machine), § 19.1 (skeleton).
//!
//! # Module map
//!
//! | Module     | Purpose |
//! |---|---|
//! | [`event`]      | `AnthropicSseEvent` enum + per-variant serde |
//! | [`sse_parser`] | line-based RFC-8895 parser ; bounded line/event size |
//! | [`state`]      | `StreamState` accumulator + `BlockBuilder` |
//! | [`finalize`]   | `StreamFinalized` builder + `Completeness` derivation |

pub mod event;
pub mod finalize;
pub mod sse_parser;
pub mod state;

pub use event::{
    AnthropicContentBlock, AnthropicDelta, AnthropicError, AnthropicMessage,
    AnthropicMessageDelta, AnthropicMessageDeltaUsage, AnthropicSseEvent, AnthropicUsage,
};
pub use finalize::StreamFinalized;
pub use sse_parser::{
    parse_sse_stream, SseParseError, MAX_EVENT_BYTES, MAX_LINE_BYTES,
};
pub use state::{BlockBuilder, StreamState};

/// **N27** : per-block accumulator size cap (T8 mitigation).
///
/// Anthropic's largest single text block in production is ~200 KiB ;
/// tool-use JSON is ~50 KiB.  256 KiB per block + 1 MiB per event
/// stops a misbehaving upstream from OOM-ing us.
pub const MAX_PER_BLOCK_BYTES: usize = 256 * 1024;

/// **N27** : per-tool-use-block JSON-accumulator cap.  Bigger than
/// the text cap because tool-use input JSON can be verbose (1 MiB).
pub const MAX_TOOL_USE_JSON_BYTES: usize = 1024 * 1024;
