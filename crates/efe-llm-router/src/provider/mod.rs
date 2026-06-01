//! LLMProvider trait + factory for the 8 backends.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "azure-openai")]
pub mod azure_openai;
#[cfg(feature = "bedrock")]
pub mod bedrock;
#[cfg(feature = "hf-tgi")]
pub mod hf_tgi;
#[cfg(feature = "mistral")]
pub mod mistral;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "vertex")]
pub mod vertex;
#[cfg(feature = "vllm")]
pub mod vllm;

/// One message in the conversation, in OpenAI-compatible shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
}

/// Tool schema (tool/function calling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteRequest {
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub seed: Option<u64>,
    pub timeout_ms: u32,
    pub tools: Option<Vec<ToolSchema>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    Safety,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteResponse {
    pub text: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f64,
    pub finish_reason: FinishReason,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub cache_hit: bool,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub available: bool,
    pub latency_p50_ms: Option<u32>,
    pub latency_p99_ms: Option<u32>,
    pub error_rate_30d: Option<f64>,
    pub last_check_unix: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CostModel {
    pub input_per_1m_tokens_usd: f64,
    pub output_per_1m_tokens_usd: f64,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider {0} timeout after {1}ms")]
    Timeout(String, u32),

    #[error("provider {0} authentication failed")]
    AuthError(String),

    #[error("provider {0} rate-limited (429)")]
    RateLimited(String),

    #[error("provider {0} unavailable: {1}")]
    Unavailable(String, String),

    #[error("provider {0} returned invalid response: {1}")]
    InvalidResponse(String, String),

    #[error("provider {0} returned safety block")]
    SafetyBlock(String),
}

impl ProviderError {
    /// Whether the error is retryable (failover candidate).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Timeout(..)
                | ProviderError::RateLimited(..)
                | ProviderError::Unavailable(..)
        )
    }
}

/// Provider abstraction.
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Stable identifier, e.g. `"anthropic:claude-sonnet-4-5"`.
    fn name(&self) -> &str;

    /// Whether the provider supports tool/function calling.
    fn supports_tools(&self) -> bool;

    /// Max input context tokens for the configured model.
    fn max_context_tokens(&self) -> u32;

    /// Pricing parameters used by cost governor.
    fn cost_model(&self) -> CostModel;

    /// Single-shot completion.
    async fn complete(
        &self,
        request: CompleteRequest,
    ) -> Result<CompleteResponse, ProviderError>;

    /// Liveness probe (best-effort).
    async fn health(&self) -> ProviderHealth;
}

/// Type alias for an Arc'd provider (cloneable across tasks).
pub type ProviderHandle = Arc<dyn LLMProvider>;
