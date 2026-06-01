//! Error types for the agent runner crate.

use thiserror::Error;
use uuid::Uuid;

pub type AgentRunnerResult<T> = Result<T, AgentRunnerError>;

#[derive(Debug, Error)]
pub enum AgentRunnerError {
    // ── Validation ──────────────────────────────────────────────────
    #[error("invalid input : {0}")]
    InvalidInput(String),

    #[error("agent run not found : {0}")]
    RunNotFound(Uuid),

    #[error("agent step not found : {0}")]
    StepNotFound(Uuid),

    #[error("agent template not found : {0}")]
    TemplateNotFound(String),

    #[error("approval request not found : {0}")]
    ApprovalNotFound(Uuid),

    // ── Feature flag / dependency ───────────────────────────────────
    #[error("M25 agent runtime not enabled at server level")]
    M25ServerDisabled,

    #[error("M25 not enabled for tenant '{0}'")]
    M25NotEnabledForTenant(String),

    #[error("M21 LLMRouter required (M25 hard dependency)")]
    M21Required,

    #[error("Agent runtime CRDs not installed in cluster ; install Helm chart efe-agent-runner first")]
    CrdMissing,

    #[error("KVM not available on host node ; M25 requires hardware virtualization")]
    KvmUnavailable,

    // ── DPIA / compliance ───────────────────────────────────────────
    #[error("DPIA not signed for tenant '{0}'")]
    DpiaNotSigned(String),

    #[error("DPIA expired for tenant '{tenant}' (expired at {expires_at:?})")]
    DpiaExpired { tenant: String, expires_at: chrono::DateTime<chrono::Utc> },

    #[error("DPIA addendum hash mismatch : expected '{expected}' got '{actual}'")]
    DpiaHashMismatch { expected: String, actual: String },

    // ── State / preconditions ──────────────────────────────────────
    #[error("invalid status transition : {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("agent run already terminated : {0}")]
    RunAlreadyTerminated(Uuid),

    #[error("agent run is not in resumable state : status={0}")]
    NotResumable(String),

    // ── Tool ACL ────────────────────────────────────────────────────
    #[error("tool '{tool}' not in whitelist for run {run_id}")]
    ToolNotInWhitelist { run_id: Uuid, tool: String },

    #[error("write tool '{tool}' requires approval (run={run_id} step={step})")]
    WriteToolRequiresApproval { run_id: Uuid, tool: String, step: u32 },

    #[error("approval token invalid or already consumed")]
    InvalidApprovalToken,

    #[error("approval token expired")]
    ApprovalTokenExpired,

    // ── Authorization ───────────────────────────────────────────────
    #[error("permission denied : role '{role}' cannot perform '{action}'")]
    PermissionDenied { role: String, action: String },

    #[error("tenant mismatch : caller='{caller}' requested='{requested}'")]
    TenantMismatch { caller: String, requested: String },

    #[error("OIDC token freshness check failed : age={age_seconds}s, max={max}s")]
    OidcTokenStale { age_seconds: i64, max: i64 },

    // ── Cost / budget ───────────────────────────────────────────────
    #[error("agent run cost cap exceeded : spent={spent_cents}c, cap={cap_cents}c (run={run_id})")]
    CostCapExceeded { run_id: Uuid, spent_cents: i64, cap_cents: i64 },

    #[error("requested max_cost_usd {req:.2} exceeds tenant cap {cap:.2}")]
    CostAboveTenantCap { req: f64, cap: f64 },

    // ── Sandbox ────────────────────────────────────────────────────
    #[error("microVM provisioning failed : {0}")]
    MicroVmProvisioningFailed(String),

    #[error("microVM not found : {0}")]
    MicroVmNotFound(String),

    #[error("sandbox escape attempt detected : {description}")]
    SandboxEscapeAttempt { description: String },

    #[error("image '{0}' not in whitelist")]
    ImageNotInWhitelist(String),

    // ── Profile ────────────────────────────────────────────────────
    #[error("profile '{0}' not in tenant.allowed_profiles")]
    ProfileNotAllowed(String),

    #[error("profile '{0}' unknown — not in registry")]
    ProfileUnknown(String),

    #[error("provider '{0}' not in tenant.allowed_providers")]
    ProviderNotAllowed(String),

    // ── Concurrency ────────────────────────────────────────────────
    #[error("max concurrent runs reached for tenant '{tenant}' : {current}/{max}")]
    MaxConcurrentRunsReached { tenant: String, current: u32, max: u32 },

    // ── Audit chain ────────────────────────────────────────────────
    #[error("audit chain broken at row {row_id} : expected parent {expected_parent:?}, got {got_parent:?}")]
    AuditChainBroken {
        expected_parent: Option<Uuid>,
        got_parent: Option<Uuid>,
        row_id: Uuid,
    },

    #[error("HMAC verification failed for audit row {0}")]
    AuditHmacFailure(Uuid),

    // ── Storage ────────────────────────────────────────────────────
    #[error("database error : {0}")]
    Database(#[from] sqlx::Error),

    #[error("serde JSON : {0}")]
    Json(#[from] serde_json::Error),

    #[error("kubernetes API : {0}")]
    Kube(String),

    #[error("internal error : {0}")]
    Internal(String),
}

impl AgentRunnerError {
    pub fn grpc_code(&self) -> tonic::Code {
        use tonic::Code::*;
        match self {
            Self::InvalidInput(_) => InvalidArgument,
            Self::RunNotFound(_)
            | Self::StepNotFound(_)
            | Self::TemplateNotFound(_)
            | Self::ApprovalNotFound(_) => NotFound,
            Self::M25ServerDisabled
            | Self::M25NotEnabledForTenant(_)
            | Self::M21Required
            | Self::CrdMissing
            | Self::KvmUnavailable
            | Self::DpiaNotSigned(_)
            | Self::DpiaExpired { .. }
            | Self::DpiaHashMismatch { .. }
            | Self::InvalidTransition { .. }
            | Self::RunAlreadyTerminated(_)
            | Self::NotResumable(_)
            | Self::ToolNotInWhitelist { .. }
            | Self::ImageNotInWhitelist(_)
            | Self::ProfileNotAllowed(_)
            | Self::ProfileUnknown(_)
            | Self::ProviderNotAllowed(_)
            | Self::MaxConcurrentRunsReached { .. }
            | Self::WriteToolRequiresApproval { .. }
            | Self::InvalidApprovalToken
            | Self::ApprovalTokenExpired => FailedPrecondition,
            Self::PermissionDenied { .. }
            | Self::TenantMismatch { .. }
            | Self::OidcTokenStale { .. } => PermissionDenied,
            Self::CostCapExceeded { .. }
            | Self::CostAboveTenantCap { .. } => ResourceExhausted,
            Self::MicroVmProvisioningFailed(_)
            | Self::MicroVmNotFound(_) => Unavailable,
            // CRITICAL : sandbox escape is a security incident,
            // surfaces as Internal but logged + alerted at server.
            Self::SandboxEscapeAttempt { .. }
            | Self::AuditChainBroken { .. }
            | Self::AuditHmacFailure(_)
            | Self::Database(_)
            | Self::Json(_)
            | Self::Kube(_)
            | Self::Internal(_) => Internal,
        }
    }
}

impl From<AgentRunnerError> for tonic::Status {
    fn from(err: AgentRunnerError) -> Self {
        let code = err.grpc_code();
        let msg = match &err {
            // Sandbox escape : NEVER leak details to caller — security alert.
            AgentRunnerError::SandboxEscapeAttempt { description } => {
                tracing::error!(?description, "SANDBOX ESCAPE ATTEMPT");
                "internal error".to_string()
            }
            AgentRunnerError::Database(_) | AgentRunnerError::Internal(_) => {
                tracing::error!(?err, "internal error in AgentRuntimeService");
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
            AgentRunnerError::DpiaNotSigned("acme".into()).grpc_code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            AgentRunnerError::ToolNotInWhitelist {
                run_id: Uuid::new_v4(), tool: "x".into()
            }.grpc_code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            AgentRunnerError::CostCapExceeded {
                run_id: Uuid::new_v4(), spent_cents: 100, cap_cents: 50,
            }.grpc_code(),
            Code::ResourceExhausted
        );
        assert_eq!(
            AgentRunnerError::SandboxEscapeAttempt {
                description: "ptrace attempt".into()
            }.grpc_code(),
            Code::Internal
        );
    }

    #[test]
    fn sandbox_escape_does_not_leak() {
        let e = AgentRunnerError::SandboxEscapeAttempt {
            description: "secret_internal_details".into(),
        };
        let status: tonic::Status = e.into();
        assert_eq!(status.message(), "internal error");
    }
}
