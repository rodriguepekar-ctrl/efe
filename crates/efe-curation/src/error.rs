//! Error types for the curation crate.
//!
//! Errors map to gRPC status codes via `Into<tonic::Status>` ; see § 15.4 of
//! the M23 architecture doc.

use thiserror::Error;
use uuid::Uuid;

pub type CurationResult<T> = Result<T, CurationError>;

#[derive(Debug, Error)]
pub enum CurationError {
    // ── Input validation ─────────────────────────────────────────────
    #[error("invalid class_id : '{0}' must be valid kebab-case (2..64 chars)")]
    InvalidClassId(String),

    #[error("invalid semver : '{0}'")]
    InvalidSemver(String),

    #[error("invalid input : {0}")]
    InvalidInput(String),

    #[error("unknown class : tenant={tenant} catalog={catalog} class={class}")]
    UnknownClass { tenant: String, catalog: String, class: String },

    #[error("proposal not found : {0}")]
    ProposalNotFound(Uuid),

    #[error("taxonomy version not found : {0}")]
    TaxonomyVersionNotFound(Uuid),

    #[error("gold dataset not found : {0}")]
    GoldDatasetNotFound(Uuid),

    // ── Feature flag / dependency ────────────────────────────────────
    #[error("M23 curation not enabled for tenant '{0}'")]
    M23NotEnabledForTenant(String),

    #[error("internal LLM mode requires M21 LLMRouter to be ON server-wide")]
    M21Required,

    #[error("internal LLM mode not allowed for tenant '{0}'")]
    InternalLlmNotAllowed(String),

    #[error("M23 curation disabled at the server level")]
    M23ServerDisabled,

    // ── State machine / preconditions ────────────────────────────────
    #[error("invalid status transition : {from:?} -> {to:?}")]
    InvalidTransition { from: String, to: String },

    #[error("4-eyes principle requires 2 distinct reviewers ; only {actual} found")]
    FourEyesRequired { actual: usize },

    #[error("proposal {0} already promoted")]
    ProposalAlreadyPromoted(Uuid),

    #[error("proposal {0} expired ({expired_at:?})")]
    ProposalExpired { id: Uuid, expired_at: chrono::DateTime<chrono::Utc> },

    // ── Authorization ────────────────────────────────────────────────
    #[error("permission denied : role '{role}' cannot perform '{action}'")]
    PermissionDenied { role: String, action: String },

    #[error("tenant mismatch : caller tenant '{caller}' != requested tenant '{requested}'")]
    TenantMismatch { caller: String, requested: String },

    #[error("OIDC token freshness check failed : iat too old ({age_seconds}s, max {max}s)")]
    OidcTokenStale { age_seconds: i64, max: i64 },

    // ── Cost / budget ────────────────────────────────────────────────
    #[error("curation budget exceeded for tenant '{tenant}' : {spent_cents}/{budget_cents} cents")]
    BudgetExceeded { tenant: String, spent_cents: i64, budget_cents: i64 },

    #[error("estimated cost {estimate_cents} cents exceeds remaining budget {remaining_cents} cents")]
    EstimateExceedsBudget { estimate_cents: i64, remaining_cents: i64 },

    // ── DAG / version ────────────────────────────────────────────────
    #[error("DAG cycle detected : adding parent {parent} to {child} would create a cycle")]
    DagCycle { parent: Uuid, child: Uuid },

    #[error("multiple ACTIVE versions detected for catalog ; this is a critical invariant violation")]
    MultipleActiveVersions,

    #[error("parent version not found : {0}")]
    ParentVersionNotFound(Uuid),

    #[error("cross-tenant parent reference : parent tenant '{parent_tenant}' != new tenant '{new_tenant}'")]
    CrossTenantParent { parent_tenant: String, new_tenant: String },

    #[error("semver not increasing : new '{new}' must be > parent '{parent}'")]
    SemverNotIncreasing { new: String, parent: String },

    // ── Audit chain ──────────────────────────────────────────────────
    #[error("audit chain broken : parent_id={parent} not found or wrong tenant")]
    AuditChainBroken { parent: Uuid },

    #[error("HMAC verification failed for audit row {0}")]
    AuditHmacFailure(Uuid),

    // ── LLM / output schema ──────────────────────────────────────────
    #[error("LLM output schema validation failed : {0}")]
    LlmSchemaInvalid(String),

    #[error("LLM provider unavailable : {0}")]
    LlmProviderUnavailable(String),

    #[error("PII leak detected in LLM output : {0}")]
    PiiLeakDetected(String),

    // ── Concurrency ──────────────────────────────────────────────────
    #[error("concurrent modification ; advisory lock contention on (tenant={tenant}, catalog={catalog})")]
    AdvisoryLockContention { tenant: String, catalog: String },

    // ── Storage ──────────────────────────────────────────────────────
    #[error("database error : {0}")]
    Database(#[from] sqlx::Error),

    #[error("serde JSON : {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error : {0}")]
    Internal(String),
}

impl CurationError {
    /// Map this error to a `tonic::Code` for gRPC response.
    pub fn grpc_code(&self) -> tonic::Code {
        use tonic::Code::*;
        match self {
            // Validation
            Self::InvalidClassId(_)
            | Self::InvalidSemver(_)
            | Self::InvalidInput(_) => InvalidArgument,

            // Not found
            Self::UnknownClass { .. }
            | Self::ProposalNotFound(_)
            | Self::TaxonomyVersionNotFound(_)
            | Self::GoldDatasetNotFound(_)
            | Self::ParentVersionNotFound(_) => NotFound,

            // Preconditions
            Self::M23NotEnabledForTenant(_)
            | Self::M21Required
            | Self::InternalLlmNotAllowed(_)
            | Self::M23ServerDisabled
            | Self::InvalidTransition { .. }
            | Self::FourEyesRequired { .. }
            | Self::ProposalAlreadyPromoted(_)
            | Self::ProposalExpired { .. }
            | Self::SemverNotIncreasing { .. }
            | Self::CrossTenantParent { .. }
            | Self::DagCycle { .. } => FailedPrecondition,

            // Auth
            Self::PermissionDenied { .. }
            | Self::TenantMismatch { .. }
            | Self::OidcTokenStale { .. } => PermissionDenied,

            // Budget
            Self::BudgetExceeded { .. }
            | Self::EstimateExceedsBudget { .. } => ResourceExhausted,

            // Concurrency
            Self::AdvisoryLockContention { .. } => Aborted,

            // External / transient
            Self::LlmProviderUnavailable(_) => Unavailable,

            // Critical / internal
            Self::MultipleActiveVersions
            | Self::AuditChainBroken { .. }
            | Self::AuditHmacFailure(_)
            | Self::LlmSchemaInvalid(_)
            | Self::PiiLeakDetected(_)
            | Self::Database(_)
            | Self::Json(_)
            | Self::Internal(_) => Internal,
        }
    }
}

impl From<CurationError> for tonic::Status {
    fn from(err: CurationError) -> Self {
        let code = err.grpc_code();
        // Detailed message for debugging, but careful : do not leak PII
        // or tenant data via the error. The match above already
        // sanitizes the variant types.
        let msg = match &err {
            CurationError::Database(_) | CurationError::Internal(_) => {
                // Don't leak DB details to the caller.
                tracing::error!(?err, "internal error in CurationService");
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
    fn maps_invalid_argument() {
        let e = CurationError::InvalidClassId("BAD".into());
        assert_eq!(e.grpc_code(), Code::InvalidArgument);
    }

    #[test]
    fn maps_resource_exhausted() {
        let e = CurationError::BudgetExceeded {
            tenant: "acme".into(), spent_cents: 100, budget_cents: 50,
        };
        assert_eq!(e.grpc_code(), Code::ResourceExhausted);
    }

    #[test]
    fn maps_permission_denied() {
        let e = CurationError::TenantMismatch {
            caller: "acme".into(), requested: "beta".into(),
        };
        assert_eq!(e.grpc_code(), Code::PermissionDenied);
    }

    #[test]
    fn maps_failed_precondition() {
        let e = CurationError::M21Required;
        assert_eq!(e.grpc_code(), Code::FailedPrecondition);
    }

    #[test]
    fn db_error_does_not_leak() {
        let e = CurationError::Internal("password is hunter2".into());
        let status: tonic::Status = e.into();
        // The user-facing message is sanitized.
        assert_eq!(status.message(), "internal error");
    }
}
