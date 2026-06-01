//! Router config + tenant config types.

use serde::{Deserialize, Serialize};
use std::env;
use thiserror::Error;

/// Server-level configuration parsed from environment.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// gRPC bind address (default `0.0.0.0:50061`).
    pub grpc_bind: String,
    /// Postgres DSN (required).
    pub pg_dsn: String,
    /// Redis URL (required for cache).
    pub redis_url: String,
    /// Cache signing HMAC key (rotated quarterly via M19 BYOK).
    pub cache_hmac_key: Vec<u8>,
    /// Audit chain HMAC key (delegated to backend's M14-bis chain).
    pub audit_hmac_key: Vec<u8>,
    /// Default semantic-cache cosine similarity threshold.
    pub semantic_cache_threshold: f32,
    /// Maximum concurrent provider calls per replica.
    pub max_concurrent_calls: u32,
    /// Default per-provider timeout (ms).
    pub default_provider_timeout_ms: u32,
    /// Enabled provider names (compile-time + runtime double check).
    pub enabled_providers: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("required env-var {0} missing or empty")]
    Missing(&'static str),
    #[error("env-var {0} invalid: {1}")]
    Invalid(&'static str, String),
}

impl RouterConfig {
    /// Load from environment with sensible defaults.
    ///
    /// Required env-vars :
    ///   - `EFE_LLMROUTER_PG_DSN`      : Postgres connection string.
    ///   - `EFE_LLMROUTER_REDIS_URL`   : Redis URL.
    ///   - `EFE_LLMROUTER_CACHE_HMAC_KEY_HEX` : 32-byte hex.
    ///   - `EFE_LLMROUTER_AUDIT_HMAC_KEY_HEX` : 32-byte hex.
    pub fn from_env() -> Result<Self, ConfigError> {
        let grpc_bind = env::var("EFE_LLMROUTER_GRPC_BIND")
            .unwrap_or_else(|_| "0.0.0.0:50061".to_string());
        let pg_dsn = env::var("EFE_LLMROUTER_PG_DSN")
            .map_err(|_| ConfigError::Missing("EFE_LLMROUTER_PG_DSN"))?;
        let redis_url = env::var("EFE_LLMROUTER_REDIS_URL")
            .map_err(|_| ConfigError::Missing("EFE_LLMROUTER_REDIS_URL"))?;

        let cache_hmac_hex = env::var("EFE_LLMROUTER_CACHE_HMAC_KEY_HEX")
            .map_err(|_| ConfigError::Missing("EFE_LLMROUTER_CACHE_HMAC_KEY_HEX"))?;
        let cache_hmac_key = hex::decode(&cache_hmac_hex).map_err(|e| {
            ConfigError::Invalid("EFE_LLMROUTER_CACHE_HMAC_KEY_HEX", e.to_string())
        })?;
        if cache_hmac_key.len() != 32 {
            return Err(ConfigError::Invalid(
                "EFE_LLMROUTER_CACHE_HMAC_KEY_HEX",
                format!("must be 32 bytes, got {}", cache_hmac_key.len()),
            ));
        }

        let audit_hmac_hex = env::var("EFE_LLMROUTER_AUDIT_HMAC_KEY_HEX")
            .map_err(|_| ConfigError::Missing("EFE_LLMROUTER_AUDIT_HMAC_KEY_HEX"))?;
        let audit_hmac_key = hex::decode(&audit_hmac_hex).map_err(|e| {
            ConfigError::Invalid("EFE_LLMROUTER_AUDIT_HMAC_KEY_HEX", e.to_string())
        })?;

        let semantic_cache_threshold = env::var("EFE_LLMROUTER_SEMANTIC_CACHE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.95_f32);

        let max_concurrent_calls: u32 = env::var("EFE_LLMROUTER_MAX_CONCURRENT_CALLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64);

        let default_provider_timeout_ms = env::var("EFE_LLMROUTER_DEFAULT_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5_000);

        let enabled_providers = env::var("EFE_LLMROUTER_PROVIDERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        Ok(RouterConfig {
            grpc_bind,
            pg_dsn,
            redis_url,
            cache_hmac_key,
            audit_hmac_key,
            semantic_cache_threshold,
            max_concurrent_calls,
            default_provider_timeout_ms,
            enabled_providers,
        })
    }
}

/// Per-tenant LLM configuration (resolved from Postgres).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantLLMConfig {
    pub tenant_id: String,
    pub enabled: bool,
    pub allowed_providers: Vec<String>,
    pub primary_provider: Option<String>,
    pub fallback_providers: Vec<String>,
    pub optimize_for: OptimizeFor,
    pub max_context_tokens: u32,
    pub require_baa: bool,
    pub require_eu_residency: bool,
    pub require_no_training: bool,
    pub prompt_redaction_level: RedactionLevel,
    pub audit_full_prompts: bool,
    pub semantic_cache_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptimizeFor {
    Cost,
    Quality,
    Latency,
    Balanced,
}

impl Default for OptimizeFor {
    fn default() -> Self {
        OptimizeFor::Balanced
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactionLevel {
    None,
    Mild,
    Aggressive,
}

impl Default for RedactionLevel {
    fn default() -> Self {
        RedactionLevel::Aggressive
    }
}
