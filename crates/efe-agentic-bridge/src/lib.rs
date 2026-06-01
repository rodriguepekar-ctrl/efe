// SPDX-License-Identifier: UNLICENSED
//! # `efe-agentic-bridge` — M50 Sprint 13 N21.
//!
//! See `docs/architecture/EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md`.
//!
//! Wraps a swarm-produced `ContextBundle` as a `tool_use` input
//! for Claude (Anthropic Messages API).

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod bridge;
pub mod claude;
/// **Sprint 19 — N27** : SSE parser + per-turn state machine +
/// finalizer for the Anthropic Messages API streaming wire.  See
/// `docs/architecture/EFE_M50_N27_STREAMING_ANTHROPIC_RETRY_PROVENANCE_ARCHITECTURE.md` § 3.
pub mod claude_stream;
pub mod error;
/// **Sprint 18 — N26 + ADR-4** : baked Anthropic pricing table +
/// per-call cost computation helpers.
pub mod pricing;
/// **Sprint 19 — N27** : retry policy (classification + exponential
/// backoff + retry-after parser + loop executor).  See architecture § 4.
pub mod retry;
pub mod task;
pub mod tool_use;

pub use bridge::AgenticBridge;
pub use claude::{
    ClaudeBridge, StreamConfig, DEFAULT_ANTHROPIC_ENDPOINT, DEFAULT_CLAUDE_MODEL,
    N27_DEFAULT_CHANNEL_CAP, N27_DEFAULT_STREAM_CONNECT_TIMEOUT_MS,
    N27_DEFAULT_STREAM_READ_TIMEOUT_MS, N27_DEFAULT_STREAM_TOTAL_TIMEOUT_MS,
};
pub use claude_stream::{
    AnthropicSseEvent, SseParseError, StreamFinalized, StreamState,
};
pub use error::BridgeError;
pub use pricing::{
    anthropic_pricing_for, compute_cost_cents, compute_cost_microcents, ModelPricing,
    FALLBACK_PRICING, PRICES_TABLE_VERSION,
};
pub use retry::{
    AttemptOutcome, ExponentialBackoff, NetworkErrorClass, RetryDecision, RetryLoop,
    RetryOutcomeProbe, RetryPolicy, RetrySummary,
};
pub use task::{
    AbortReason, AgenticAuditContext, AgenticEvent, AgenticResponse, AgenticTask, StopReason,
    TokenUsage,
};
pub use tool_use::{ToolDefinition, ToolUseBlock, SWARM_CONTEXT_TOOL_NAME};

/// **N21** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
