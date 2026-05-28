// SPDX-License-Identifier: UNLICENSED
//! Bridge multi-LLM gros — abstraction de Claude/GPT-4/Gemini/local.

mod bridge;
mod mock;
mod provider;
pub mod providers;
mod registry;

pub use bridge::{MultiLlmBridge, ProviderHandle};
pub use mock::MockLlmProvider;
pub use provider::{
    cents_to_microcents, compute_cost_microcents, microcents_to_cents, LlmCall, LlmProvider,
    LlmResponse, ProviderId, TokenChunk,
};
pub use registry::ProviderRegistry;

#[cfg(feature = "anthropic")]
pub use providers::ClaudeProvider;
#[cfg(feature = "gemini")]
pub use providers::GeminiProvider;
#[cfg(feature = "openai")]
pub use providers::GptProvider;
