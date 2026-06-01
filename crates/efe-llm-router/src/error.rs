//! Top-level RouterError hierarchy.

use crate::cost_governor::CostError;
use crate::provider::ProviderError;
use thiserror::Error;

/// Errors at the LLMRouter facade level.
#[derive(Debug, Error)]
pub enum RouterError {
    /// Tenant has no eligible provider (config empty / all filtered out).
    #[error("no eligible provider for tenant request")]
    NoEligibleProvider,

    /// All providers in the chain failed.
    #[error("all {count} providers failed: {summary}")]
    AllProvidersFailed {
        count: usize,
        summary: String,
        #[source]
        last_error: Option<ProviderError>,
    },

    /// Cost governor rejected the request.
    #[error("cost governor: {0}")]
    Cost(#[from] CostError),

    /// Cache subsystem error (non-fatal except on poisoning).
    #[error("cache error: {0}")]
    Cache(#[from] crate::cache::CacheError),

    /// Audit emission failure (treated as warning, not fatal).
    #[error("audit emission failed: {0}")]
    Audit(String),

    /// Tenant feature flag check failed.
    #[error("feature disabled for tenant: {0}")]
    FeatureDisabled(String),

    /// Tenant config missing or malformed.
    #[error("tenant config error: {0}")]
    TenantConfig(String),

    /// Output failed validation (e.g. anchors not in candidates).
    #[error("output validation failed: {0}")]
    OutputValidation(String),

    /// Generic infrastructure error (Postgres, Redis, etc.).
    #[error("infrastructure error: {0}")]
    Infrastructure(#[from] anyhow::Error),
}

impl RouterError {
    /// Map RouterError → gRPC status code (for the server layer).
    pub fn grpc_code(&self) -> tonic_status_code::Code {
        use RouterError::*;
        match self {
            FeatureDisabled(_) => tonic_status_code::Code::Unimplemented,
            Cost(CostError::BudgetExhausted { .. }) => tonic_status_code::Code::ResourceExhausted,
            Cost(CostError::TenantNotConfigured(_)) => tonic_status_code::Code::FailedPrecondition,
            // Cost(CostError::Pg(_)) and any future CostError variants
            // map to Internal.
            Cost(_) => tonic_status_code::Code::Internal,
            NoEligibleProvider => tonic_status_code::Code::FailedPrecondition,
            AllProvidersFailed { .. } => tonic_status_code::Code::Unavailable,
            OutputValidation(_) => tonic_status_code::Code::Internal,
            TenantConfig(_) => tonic_status_code::Code::FailedPrecondition,
            Audit(_) => tonic_status_code::Code::Internal,
            Cache(_) => tonic_status_code::Code::Internal,
            Infrastructure(_) => tonic_status_code::Code::Internal,
        }
    }
}

/// Tiny shim re-export so the lib doesn't need `tonic` as direct dep.
pub mod tonic_status_code {
    /// Minimal subset of `tonic::Code` needed for `grpc_code()`.
    #[derive(Debug, Clone, Copy)]
    pub enum Code {
        Unimplemented,
        ResourceExhausted,
        FailedPrecondition,
        Unavailable,
        Internal,
    }
}
