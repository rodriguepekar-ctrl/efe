//! Mode-3 errors — typed, mappable to gRPC status codes.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Mode3Error {
    /// Mode-3 feature flag not enabled.
    #[error("Mode-3 not enabled: {0}")]
    NotEnabled(String),

    /// M21 (Mode-2) is not active ; M22 requires it.
    #[error("Mode-3 requires Mode-2 (M21) to be active")]
    M21NotActive,

    /// Tenant has no Mode-3 config row.
    #[error("tenant {0} not configured for Mode-3")]
    TenantNotConfigured(String),

    /// Catalog has `mode3_enabled = false`.
    #[error("catalog {0} not opted into Mode-3")]
    CatalogNotEnabled(String),

    /// `max_turns` outside [1, MAX_TURNS_HARD_CAP].
    #[error("max_turns {0} out of range [1, {1}]")]
    InvalidMaxTurns(u32, u32),

    /// `threshold` outside [0.0, 1.0].
    #[error("threshold {0} out of range [0, 1]")]
    InvalidThreshold(f32),

    /// Compliance violation : high-stakes catalog without
    /// `human_review_required=true`.
    #[error(
        "compliance violation: catalog {catalog} has stakes_tier=high \
         but tenant.human_review_required=false (GDPR Art. 22(3) + AI Act Art. 14)"
    )]
    HighStakesNoReview { catalog: String },

    /// LLMRouter failed across all providers.
    #[error("all providers failed at turn {turn}: {summary}")]
    AllProvidersFailed { turn: u32, summary: String },

    /// LLM output failed validation (e.g. anchors not in candidates).
    #[error("output validation failed at turn {turn}: {reason}")]
    OutputValidation { turn: u32, reason: String },

    /// Swarm Mode-1 backend error.
    #[error("swarm backend error: {0}")]
    SwarmBackend(String),

    /// Postgres operational error.
    #[error("postgres error: {0}")]
    Pg(String),

    /// Cost governor rejected the request (budget exhausted).
    #[error("budget exhausted (M21 cost governor)")]
    BudgetExhausted,

    /// Generic infrastructure error.
    #[error("infrastructure: {0}")]
    Infrastructure(String),
}

impl Mode3Error {
    /// Whether the error is permanent vs retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Mode3Error::AllProvidersFailed { .. } | Mode3Error::Infrastructure(_)
        )
    }

    /// Map to a stable error code string for tool responses.
    pub fn code(&self) -> &'static str {
        match self {
            Mode3Error::NotEnabled(_) => "feature_disabled",
            Mode3Error::M21NotActive => "m21_required",
            Mode3Error::TenantNotConfigured(_) => "tenant_not_configured",
            Mode3Error::CatalogNotEnabled(_) => "catalog_not_enabled",
            Mode3Error::InvalidMaxTurns(_, _) => "invalid_arguments",
            Mode3Error::InvalidThreshold(_) => "invalid_arguments",
            Mode3Error::HighStakesNoReview { .. } => "compliance_violation",
            Mode3Error::AllProvidersFailed { .. } => "providers_failed",
            Mode3Error::OutputValidation { .. } => "validation_failed",
            Mode3Error::SwarmBackend(_) => "swarm_backend_error",
            Mode3Error::Pg(_) => "infrastructure_error",
            Mode3Error::BudgetExhausted => "budget_exhausted",
            Mode3Error::Infrastructure(_) => "infrastructure_error",
        }
    }
}
