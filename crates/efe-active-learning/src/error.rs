//! Error types for the active learning crate.

use thiserror::Error;
use uuid::Uuid;

pub type ActiveLearningResult<T> = Result<T, ActiveLearningError>;

#[derive(Debug, Error)]
pub enum ActiveLearningError {
    // ── Validation ──────────────────────────────────────────────────
    #[error("invalid input : {0}")]
    InvalidInput(String),

    #[error("queue row not found : {0}")]
    QueueRowNotFound(Uuid),

    #[error("proposal not found : {0}")]
    ProposalNotFound(Uuid),

    #[error("retune run not found : {0}")]
    RetuneRunNotFound(Uuid),

    #[error("gold dataset not found : {0}")]
    GoldDatasetNotFound(Uuid),

    #[error("active bundle not found for tenant={tenant} catalog={catalog}")]
    ActiveBundleNotFound { tenant: String, catalog: String },

    // ── Feature flag / dependency ───────────────────────────────────
    #[error("M24 active learning not enabled for tenant '{0}'")]
    M24NotEnabledForTenant(String),

    #[error("M24 server-wide flag is off")]
    M24ServerDisabled,

    #[error("M21 LLMRouter required (M24 dependency)")]
    M21Required,

    #[error("M23 curation required (M24 dependency)")]
    M23Required,

    // ── State / transitions ─────────────────────────────────────────
    #[error("invalid status transition : {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("centroid drift {drift} exceeds maximum {max} (likely poisoning)")]
    CentroidDriftExceeded { drift: f32, max: f32 },

    #[error("insufficient gold examples : {actual} < threshold {required}")]
    InsufficientGoldExamples { actual: u64, required: u64 },

    // ── Authorization ───────────────────────────────────────────────
    #[error("permission denied : role '{role}' cannot perform '{action}'")]
    PermissionDenied { role: String, action: String },

    #[error("tenant mismatch : caller='{caller}' requested='{requested}'")]
    TenantMismatch { caller: String, requested: String },

    #[error("OIDC token freshness check failed : iat too old ({age_seconds}s, max {max}s)")]
    OidcTokenStale { age_seconds: i64, max: i64 },

    #[error("DPO consent required for auto-promotion (signed_at: {signed_at:?})")]
    DpoConsentRequired { signed_at: Option<chrono::DateTime<chrono::Utc>> },

    #[error("auto-promotion blocked : catalog stakes_tier must be LOW (got {got})")]
    AutoPromotionStakesBlocked { got: String },

    // ── Cost / budget ───────────────────────────────────────────────
    #[error("active learning budget exceeded : {spent_cents}/{budget_cents} cents (tenant {tenant})")]
    BudgetExceeded { tenant: String, spent_cents: i64, budget_cents: i64 },

    // ── LLM / output schema ─────────────────────────────────────────
    #[error("LLM output schema invalid : {0}")]
    LlmSchemaInvalid(String),

    #[error("LLM proposed unknown class : '{0}'")]
    LlmUnknownClass(String),

    #[error("LLM provider unavailable : {0}")]
    LlmProviderUnavailable(String),

    #[error("PII redactor failed : {0}")]
    PiiRedactionFailed(String),

    // ── Concurrency ─────────────────────────────────────────────────
    #[error("concurrent retune attempt for (tenant={tenant}, catalog={catalog})")]
    ConcurrentRetune { tenant: String, catalog: String },

    // ── Audit chain ─────────────────────────────────────────────────
    #[error("audit chain broken : parent {parent} not found or wrong tenant")]
    AuditChainBroken { parent: Uuid },

    #[error("HMAC verification failed for audit row {0}")]
    AuditHmacFailure(Uuid),

    // ── Storage ─────────────────────────────────────────────────────
    #[error("database error : {0}")]
    Database(#[from] sqlx::Error),

    #[error("serde JSON error : {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error : {0}")]
    Internal(String),
}

impl ActiveLearningError {
    pub fn grpc_code(&self) -> tonic::Code {
        use tonic::Code::*;
        match self {
            Self::InvalidInput(_) => InvalidArgument,
            Self::QueueRowNotFound(_)
            | Self::ProposalNotFound(_)
            | Self::RetuneRunNotFound(_)
            | Self::GoldDatasetNotFound(_)
            | Self::ActiveBundleNotFound { .. } => NotFound,
            Self::M24NotEnabledForTenant(_)
            | Self::M24ServerDisabled
            | Self::M21Required
            | Self::M23Required
            | Self::InvalidTransition { .. }
            | Self::CentroidDriftExceeded { .. }
            | Self::InsufficientGoldExamples { .. }
            | Self::DpoConsentRequired { .. }
            | Self::AutoPromotionStakesBlocked { .. } => FailedPrecondition,
            Self::PermissionDenied { .. }
            | Self::TenantMismatch { .. }
            | Self::OidcTokenStale { .. } => PermissionDenied,
            Self::BudgetExceeded { .. } => ResourceExhausted,
            Self::ConcurrentRetune { .. } => Aborted,
            Self::LlmProviderUnavailable(_) => Unavailable,
            Self::AuditChainBroken { .. }
            | Self::AuditHmacFailure(_)
            | Self::LlmSchemaInvalid(_)
            | Self::LlmUnknownClass(_)
            | Self::PiiRedactionFailed(_)
            | Self::Database(_)
            | Self::Json(_)
            | Self::Internal(_) => Internal,
        }
    }
}

impl From<ActiveLearningError> for tonic::Status {
    fn from(err: ActiveLearningError) -> Self {
        let code = err.grpc_code();
        let msg = match &err {
            ActiveLearningError::Database(_) | ActiveLearningError::Internal(_) => {
                tracing::error!(?err, "internal error in ActiveLearningService");
                "internal error".to_string()
            }
            _ => err.to_string(),
        };
        tonic::Status::new(code, msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn maps_codes() {
        assert_eq!(
            ActiveLearningError::InvalidInput("x".into()).grpc_code(),
            Code::InvalidArgument
        );
        assert_eq!(
            ActiveLearningError::M21Required.grpc_code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            ActiveLearningError::CentroidDriftExceeded { drift: 0.5, max: 0.3 }.grpc_code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            ActiveLearningError::BudgetExceeded {
                tenant: "x".into(), spent_cents: 100, budget_cents: 50
            }.grpc_code(),
            Code::ResourceExhausted
        );
    }

    #[test]
    fn db_error_does_not_leak() {
        let e = ActiveLearningError::Internal("password is hunter2".into());
        let status: tonic::Status = e.into();
        assert_eq!(status.message(), "internal error");
    }
}
