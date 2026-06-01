//! efe-llm-router — Multi-provider LLM abstraction (M21).
//!
//! Crate root. Public surface :
//!
//!   - [`LLMRouter`]      : main orchestrator
//!   - [`LLMProvider`]    : trait every provider implements
//!   - [`CostGovernor`]   : per-tenant budget enforcement (Postgres)
//!   - [`Cache`]          : exact-match + semantic (Redis Stack HNSW)
//!   - [`CircuitBreaker`] : per-provider failure state machine
//!   - [`AuditEmitter`]   : extends D14-bis HMAC chain to llm_call_audit
//!
//! See `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` for the
//! full architecture document.
#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod audit;
pub mod cache;
pub mod circuit_breaker;
pub mod config;
pub mod cost_governor;
pub mod error;
pub mod provider;
pub mod router;
#[cfg(feature = "server")]
pub mod grpc_server;

pub use audit::{AuditEmitter, LlmCallAuditEvent};
pub use cache::{Cache, CachedResponse, CacheError};
pub use circuit_breaker::{CircuitBreaker, CircuitState};
pub use config::{RouterConfig, TenantLLMConfig};
pub use cost_governor::{BudgetState, CostError, CostGovernor};
pub use error::RouterError;
pub use provider::{
    CompleteRequest, CompleteResponse, CostModel, FinishReason, LLMProvider, ProviderError,
    ProviderHealth,
};
pub use router::{LLMRouter, RouterRequest, RouterResponse};
