//! Runtime-side error hierarchy.
//!
//! These errors are **internal** to the M50 runtime ; they are
//! converted to [`efe_nano_sdk::OperationError`] when surfaced to
//! customers via SwarmResponse.

use efe_nano_sdk::{
    NanoAgentId, NanoAgentIdError, NanoAgentVersion, NanoAgentVersionError, OperationError,
    OperationErrorKind, TenantId, VersionReqError,
};
use thiserror::Error;

/// Top-level runtime error.
#[derive(Debug, Error)]
pub enum NanoAgentError {
    /// Schema validation failed (input or output).
    #[error("schema validation failed : {what} — {message}")]
    SchemaValidation { what: SchemaSide, message: String },

    /// Resource limit was exceeded.
    #[error("resource exceeded : {kind:?}")]
    ResourceExceeded { kind: ResourceKind },

    /// Generic runtime failure.
    #[error("runtime error : {message}")]
    Runtime { message: String },

    /// Catalog operation failed.
    #[error("catalog error : {0}")]
    Catalog(#[from] CatalogError),

    /// Dispatcher error.
    #[error("dispatcher error : {0}")]
    Dispatcher(#[from] DispatcherError),

    /// Budget exhausted.
    #[error("budget exceeded for tenant {tenant_id} (remaining {remaining_cents} cents)")]
    BudgetExceeded {
        tenant_id: TenantId,
        remaining_cents: f64,
    },

    /// Operation timed out.
    #[error("operation timed out after {0} ms")]
    Timeout(u32),

    /// Mode B local model error (Sprint 4).
    #[error("local model error : {0}")]
    LocalModel(#[from] ModelInferError),

    /// Mode B model loader error (Sprint 4).
    #[error("model loader error : {0}")]
    ModelLoad(#[from] ModelLoaderError),

    /// Mode B/C VM pool error (Sprint 4).
    #[error("VM pool error : {0}")]
    VmPool(#[from] VmPoolError),

    /// Mode requested by `agent.execution_mode` is not wired in the
    /// current runtime.  In Sprint 4 this surfaced for Mode C ; in
    /// Sprint 5 Mode C is wired, so this now only surfaces if a runtime
    /// constructed via `MultiModeRuntime::mode_a_only` is asked to run
    /// a Mode B or Mode C agent.
    #[error("execution mode '{0}' is not wired in this runtime")]
    ModeNotWired(String),

    /// Mode C LLM call error (Sprint 5).
    #[error("LLM call error : {0}")]
    LlmCall(#[from] LlmCallError),

    /// Output validator error (Sprint 5).
    #[error("output validation error : {0}")]
    OutputValidation(#[from] ValidationError),

    /// Budget enforcer error (Sprint 5).
    #[error("budget error : {0}")]
    Budget(#[from] BudgetError),

    /// PII redaction error (Sprint 5).
    #[error("PII redaction error : {0}")]
    Pii(#[from] PiiError),

    /// HMAC audit chain error (Sprint 6).
    #[error("audit chain error : {0}")]
    AuditChain(#[from] AuditChainError),

    /// Replay engine error (Sprint 6).
    #[error("replay error : {0}")]
    Replay(#[from] ReplayError),

    /// **Sprint 7 — N8** : Mode C input flagged by the prompt-injection
    /// filter and `agent.compliance.prompt_injection_strategy == Block`.
    /// No LLM call was made.  Maps to
    /// `OperationErrorKind::PromptInjectionSuspected`.
    #[error("prompt injection suspected (score={score}, patterns={patterns:?})")]
    PromptInjection { score: u32, patterns: Vec<String> },

    /// Internal bug / unexpected state.
    #[error("internal error : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  Mode B errors (Sprint 4)
// ─────────────────────────────────────────────────────────────────────

/// Errors emitted by a [`crate::runtime::LocalModel`] during inference.
#[derive(Debug, Error)]
pub enum ModelInferError {
    /// The model returned an output that didn't match the expected shape.
    #[error("inference failed : {0}")]
    Inference(String),

    /// Input data could not be converted into the model's tensor format.
    #[error("input conversion failed : {0}")]
    InputConversion(String),

    /// Output tensor could not be converted back to JSON.
    #[error("output conversion failed : {0}")]
    OutputConversion(String),

    /// Inference exceeded the per-call wall-clock budget.
    #[error("inference timed out after {0} ms")]
    Timeout(u32),
}

/// Errors emitted by a [`crate::runtime::ModelLoader`] during load/lookup.
#[derive(Debug, Error)]
pub enum ModelLoaderError {
    /// No model registered with the given (id, version) tuple.
    #[error("model '{id}'@{version} not found in registry")]
    NotFound { id: String, version: String },

    /// Disk read or model deserialization failed.
    #[error("model load failed : {0}")]
    LoadFailed(String),

    /// Loader internal error (rare ; should be unreachable).
    #[error("model loader internal error : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  Mode C errors (Sprint 5)
// ─────────────────────────────────────────────────────────────────────

/// Closed taxonomy of Mode C LLM call errors.
#[derive(Debug, Error)]
pub enum LlmCallError {
    /// LLM provider returned an error (network, rate limit, auth, etc.).
    #[error("provider error : {0}")]
    Provider(String),

    /// LLM response is not valid JSON.
    #[error("response is not valid JSON : {0}")]
    InvalidJson(String),

    /// LLM response is valid JSON but doesn't match the agent's output_schema.
    #[error("response failed schema validation : {0}")]
    SchemaMismatch(String),

    /// Budget pre-check failed (insufficient cents remaining).
    #[error("budget insufficient : {remaining_cents} cents remaining")]
    BudgetInsufficient { remaining_cents: f64 },

    /// LLM call timed out.
    #[error("LLM call timed out after {0} ms")]
    Timeout(u32),

    /// Prompt template not found in registry.
    #[error("prompt template '{0}' not found in registry")]
    PromptNotFound(String),

    /// PII leak detected in LLM output (PiiStrategy::Strict mode).
    #[error("PII leak detected in LLM output : token '{0}' found")]
    PiiLeakDetected(String),

    /// Internal error (rare ; should be unreachable).
    #[error("internal : {0}")]
    Internal(String),
}

/// Closed taxonomy of output validation errors.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// JSONSchema compile failed.
    #[error("schema compile failed : {0}")]
    SchemaCompile(String),

    /// Value doesn't match schema.
    #[error("output does not match schema : {0}")]
    SchemaMismatch(String),
}

/// Closed taxonomy of budget enforcer errors.
#[derive(Debug, Error)]
pub enum BudgetError {
    /// Insufficient budget headroom for the requested cents.
    #[error("budget insufficient : requested {requested_cents} cents, {remaining_cents} remaining")]
    Insufficient {
        requested_cents: f64,
        remaining_cents: f64,
    },

    /// Reservation has already been committed/released or doesn't exist.
    #[error("reservation {0} not found or already committed/released")]
    UnknownReservation(uuid::Uuid),

    /// Budget service internal error (Phase 4 M31 wiring will populate).
    #[error("budget service internal error : {0}")]
    Internal(String),
}

/// Closed taxonomy of PII redaction errors.
#[derive(Debug, Error)]
pub enum PiiError {
    /// PII redactor backend failed.
    #[error("PII redaction failed : {0}")]
    RedactionFailed(String),

    /// PiiStrategy::Strict detected a leaked redaction token in LLM output.
    #[error("PII leak detected : redaction token '{0}' found in output")]
    LeakDetected(String),
}

// ─────────────────────────────────────────────────────────────────────
//  Audit chain + Replay errors (Sprint 6)
// ─────────────────────────────────────────────────────────────────────

/// Errors emitted by the HMAC audit chain layer.
#[derive(Debug, Error)]
pub enum AuditChainError {
    /// HMAC master key is missing or invalid.
    #[error("HMAC master key missing or invalid : {0}")]
    KeyMissing(String),

    /// Chain integrity broken at a specific trace.
    #[error("chain broken at trace {0}")]
    ChainBroken(uuid::Uuid),

    /// HMAC mismatch at a specific trace.
    #[error("HMAC mismatch at trace {0}")]
    HmacMismatch(uuid::Uuid),

    /// Chain store backend error.
    #[error("chain store error : {0}")]
    Store(#[from] ChainStoreError),

    /// Feature not yet implemented (e.g. Vault Transit).
    #[error("not implemented : {0}")]
    NotImplemented(String),
}

/// Errors emitted by the `ChainStore` trait.
#[derive(Debug, Error)]
pub enum ChainStoreError {
    /// Connection / I/O error.
    #[error("chain store I/O : {0}")]
    Io(String),

    /// Constraint violation (e.g. duplicate trace_id).
    #[error("chain store constraint : {0}")]
    Constraint(String),

    /// Trace not found.
    #[error("trace {0} not found")]
    NotFound(uuid::Uuid),

    /// Cross-tenant access attempt.
    #[error("cross-tenant access : tenant {actual} can't access tenant {requested}")]
    CrossTenant {
        actual: efe_nano_sdk::TenantId,
        requested: efe_nano_sdk::TenantId,
    },
}

/// Errors emitted by the `ReplayEngine`.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// No traces found for the requested swarm_request_id.
    #[error("no traces found for swarm request {0}")]
    NotFound(uuid::Uuid),

    /// HMAC chain verification failed.
    #[error("chain integrity broken : {0}")]
    ChainIntegrityBroken(#[from] AuditChainError),

    /// Some traces are missing their full_input — replay impossible.
    #[error("trace {0} has no full_input — agent.audit.include_input was false")]
    MissingFullInput(uuid::Uuid),

    /// Reconstructed swarm request couldn't be re-dispatched.
    ///
    /// **Not emitted by Sprint 6** : the engine currently surfaces dispatch
    /// failures via the `dispatch_error: Option<String>` field on the
    /// returned [`crate::replay::ReplayReport`] (soft-fail mode — every
    /// op is marked `ReplayFailed`, the report still returns Ok).
    /// Reserved for Phase 4 / Sprint 7 "strict replay" mode where the
    /// regulator wants a hard `Err` on dispatch failure (e.g.  catalog
    /// purged the agent between original run and replay) rather than a
    /// soft report.
    #[error("replay dispatch failed : {0}")]
    DispatchFailed(String),

    /// Internal error (e.g.  trace carries an invalid `NanoAgentId`).
    #[error("replay internal error : {0}")]
    Internal(String),

    /// **Sprint 8 — N14** : the swarm has no envelope row (pre-N14
    /// swarm OR the envelope was tombstoned).  Caller falls back to
    /// per-trace reconstruction.
    #[error("swarm {0} has no envelope row")]
    EnvelopeNotFound(uuid::Uuid),

    /// **Sprint 8 — N14** : the envelope row exists but its
    /// `full_input` body was erased (tombstoned).  Replay impossible.
    #[error("swarm {0} envelope was tombstoned")]
    EnvelopeTombstoned(uuid::Uuid),

    /// **Sprint 8 — N14** : the envelope row's `full_input` doesn't
    /// deserialize as a `SwarmEnvelopePayload`.  Chain corruption.
    #[error("swarm {0} envelope body malformed : {1}")]
    EnvelopeMalformed(uuid::Uuid, String),
}

/// Errors emitted by a [`crate::runtime::VmPool`] during acquire/execute.
#[derive(Debug, Error)]
pub enum VmPoolError {
    /// Pool is at max capacity and no permits are available within budget.
    #[error("VM pool exhausted : kind {kind:?}, max {max} reached")]
    Exhausted { kind: String, max: u32 },

    /// `acquire(wait_budget)` exceeded its budget waiting for a permit.
    #[error("VM pool acquire timed out after {0} ms")]
    AcquireTimeout(u32),

    /// Provisioning a new VM (cold path) failed.
    #[error("VM provisioning failed : {0}")]
    ProvisionFailed(String),

    /// VM crashed during execution.
    #[error("VM crashed during execution")]
    VmCrashed,

    /// Internal error (rare ; should be unreachable).
    #[error("VM pool internal error : {0}")]
    Internal(String),
}

/// Which side of the schema failed validation ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaSide {
    Input,
    Output,
}

impl std::fmt::Display for SchemaSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaSide::Input => f.write_str("input"),
            SchemaSide::Output => f.write_str("output"),
        }
    }
}

/// Which resource was exceeded ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Memory,
    Cpu,
    WallClock,
    NetworkEgress,
    LlmTokens,
}

/// Catalog-specific errors.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// Invalid agent ID.
    #[error("invalid agent ID : {0}")]
    InvalidId(#[from] NanoAgentIdError),

    /// Invalid version.
    #[error("invalid version : {0}")]
    InvalidVersion(#[from] NanoAgentVersionError),

    /// Invalid version requirement.
    #[error("invalid version requirement : {0}")]
    InvalidVersionReq(#[from] VersionReqError),

    /// No version of the agent matches the version requirement.
    #[error("no version of '{id}' matches requirement '{req}'")]
    VersionNotFound { id: NanoAgentId, req: String },

    /// Tenant is not authorized to publish to this ID.
    #[error("tenant {tenant} not authorized to publish to '{id}'")]
    Unauthorized {
        tenant: TenantId,
        id: NanoAgentId,
    },

    /// Agent + version already published.
    #[error("agent '{id}' v{version} already exists")]
    AlreadyExists {
        id: NanoAgentId,
        version: NanoAgentVersion,
    },

    /// Storage backend error.
    #[error("storage error : {0}")]
    Storage(String),
}

/// Dispatcher-specific errors.
#[derive(Debug, Error)]
pub enum DispatcherError {
    /// SwarmRequest had an invalid DAG.
    #[error("invalid DAG : {reason}")]
    InvalidDag { reason: String },

    /// Cycle detected in dependency graph.
    #[error("cycle detected involving ops : {ops:?}")]
    CycleDetected { ops: Vec<String> },

    /// Reference to an unknown op_id.
    #[error("op_id '{0}' referenced but not defined")]
    UnknownOpId(String),

    /// Duplicate op_id within a SwarmRequest.
    #[error("duplicate op_id '{0}' in SwarmRequest")]
    DuplicateOpId(String),

    /// Max parallelism was exceeded by some layer.
    #[error("layer width {actual} exceeds max_parallelism {max}")]
    TooParallel { actual: u32, max: u32 },

    /// Overall batch timeout.
    #[error("swarm timed out after {0} ms")]
    Timeout(u32),

    /// **Sprint 8 — N15** : the admission gate rejected this swarm
    /// at the front door.  Maps to HTTP 429 + `Retry-After` per the
    /// embedded [`efe_nano_sdk::BackpressureError`].
    #[error("backpressure : {0}")]
    Backpressure(#[from] efe_nano_sdk::BackpressureError),
}

/// Convert a runtime error into a customer-facing OperationError.
impl From<&NanoAgentError> for OperationError {
    fn from(err: &NanoAgentError) -> Self {
        let kind = match err {
            NanoAgentError::SchemaValidation { .. } => OperationErrorKind::SchemaValidationFailed,
            NanoAgentError::ResourceExceeded { .. } => OperationErrorKind::ResourceExceeded,
            NanoAgentError::Runtime { .. } => OperationErrorKind::RuntimeError,
            NanoAgentError::Catalog(_) => OperationErrorKind::AgentNotFound,
            NanoAgentError::Dispatcher(DispatcherError::Backpressure(_)) => {
                OperationErrorKind::Backpressure
            }
            NanoAgentError::Dispatcher(_) => OperationErrorKind::InvalidDag,
            NanoAgentError::BudgetExceeded { .. } => OperationErrorKind::BudgetExceeded,
            NanoAgentError::Timeout(_) => OperationErrorKind::Timeout,
            NanoAgentError::LocalModel(ModelInferError::Timeout(_)) => OperationErrorKind::Timeout,
            NanoAgentError::LocalModel(_) => OperationErrorKind::RuntimeError,
            NanoAgentError::ModelLoad(ModelLoaderError::NotFound { .. }) => {
                OperationErrorKind::AgentNotFound
            }
            NanoAgentError::ModelLoad(_) => OperationErrorKind::RuntimeError,
            NanoAgentError::VmPool(VmPoolError::AcquireTimeout(_)) => OperationErrorKind::Timeout,
            NanoAgentError::VmPool(_) => OperationErrorKind::ResourceExceeded,
            NanoAgentError::ModeNotWired(_) => OperationErrorKind::InternalError,
            // Sprint 5 :
            NanoAgentError::LlmCall(LlmCallError::Timeout(_)) => OperationErrorKind::Timeout,
            NanoAgentError::LlmCall(LlmCallError::BudgetInsufficient { .. }) => {
                OperationErrorKind::BudgetExceeded
            }
            NanoAgentError::LlmCall(LlmCallError::SchemaMismatch(_))
            | NanoAgentError::LlmCall(LlmCallError::InvalidJson(_)) => {
                OperationErrorKind::SchemaValidationFailed
            }
            NanoAgentError::LlmCall(_) => OperationErrorKind::RuntimeError,
            NanoAgentError::OutputValidation(_) => OperationErrorKind::SchemaValidationFailed,
            NanoAgentError::Budget(BudgetError::Insufficient { .. }) => {
                OperationErrorKind::BudgetExceeded
            }
            NanoAgentError::Budget(_) => OperationErrorKind::InternalError,
            NanoAgentError::Pii(_) => OperationErrorKind::RuntimeError,
            // Sprint 6 :
            NanoAgentError::AuditChain(_) => OperationErrorKind::InternalError,
            NanoAgentError::Replay(_) => OperationErrorKind::InternalError,
            // Sprint 7 — N8 : dedicated mapping for prompt-injection.
            NanoAgentError::PromptInjection { .. } => {
                OperationErrorKind::PromptInjectionSuspected
            }
            NanoAgentError::Internal(_) => OperationErrorKind::InternalError,
        };
        OperationError::new(kind, err.to_string())
    }
}

/// Convenience Result alias.
pub type Result<T, E = NanoAgentError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_side_display() {
        assert_eq!(SchemaSide::Input.to_string(), "input");
        assert_eq!(SchemaSide::Output.to_string(), "output");
    }

    #[test]
    fn schema_validation_error_display() {
        let e = NanoAgentError::SchemaValidation {
            what: SchemaSide::Input,
            message: "missing required field 'x'".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("input"));
        assert!(s.contains("missing required field"));
    }

    #[test]
    fn resource_exceeded_error_display() {
        let e = NanoAgentError::ResourceExceeded {
            kind: ResourceKind::Memory,
        };
        let s = format!("{e}");
        assert!(s.contains("Memory"));
    }

    #[test]
    fn timeout_error_display() {
        let e = NanoAgentError::Timeout(5000);
        let s = format!("{e}");
        assert!(s.contains("5000 ms"));
    }

    #[test]
    fn dispatcher_cycle_error() {
        let e = DispatcherError::CycleDetected {
            ops: vec!["op-1".to_string(), "op-2".to_string()],
        };
        let s = format!("{e}");
        assert!(s.contains("op-1"));
        assert!(s.contains("op-2"));
    }

    #[test]
    fn budget_exceeded_error() {
        let e = NanoAgentError::BudgetExceeded {
            tenant_id: TenantId::system(),
            remaining_cents: 0.5,
        };
        let s = format!("{e}");
        assert!(s.contains("budget exceeded"));
        assert!(s.contains("0.5"));
    }

    #[test]
    fn error_to_operation_error_kind_mapping() {
        let e = NanoAgentError::Timeout(1000);
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::Timeout);

        let e = NanoAgentError::SchemaValidation {
            what: SchemaSide::Output,
            message: "missing required field".to_string(),
        };
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::SchemaValidationFailed);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 4 — Mode B error variants
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn model_load_error_not_found_maps_to_agent_not_found() {
        let e = NanoAgentError::ModelLoad(ModelLoaderError::NotFound {
            id: "missing".into(),
            version: "1.0.0".into(),
        });
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::AgentNotFound);
        assert!(oe.message.contains("missing"));
    }

    #[test]
    fn model_load_error_load_failed_maps_to_runtime() {
        let e = NanoAgentError::ModelLoad(ModelLoaderError::LoadFailed(
            "disk read fail".into(),
        ));
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::RuntimeError);
    }

    #[test]
    fn local_model_inference_error_maps_to_runtime() {
        let e = NanoAgentError::LocalModel(ModelInferError::Inference("bad shape".into()));
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::RuntimeError);
    }

    #[test]
    fn local_model_timeout_maps_to_timeout() {
        let e = NanoAgentError::LocalModel(ModelInferError::Timeout(123));
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::Timeout);
    }

    #[test]
    fn vm_pool_acquire_timeout_maps_to_timeout() {
        let e = NanoAgentError::VmPool(VmPoolError::AcquireTimeout(500));
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::Timeout);
    }

    #[test]
    fn vm_pool_exhausted_maps_to_resource_exceeded() {
        let e = NanoAgentError::VmPool(VmPoolError::Exhausted {
            kind: "ModelInference".into(),
            max: 200,
        });
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::ResourceExceeded);
    }

    #[test]
    fn mode_not_wired_maps_to_internal() {
        let e = NanoAgentError::ModeNotWired("llm_augmented".into());
        let oe: OperationError = (&e).into();
        assert_eq!(oe.kind, OperationErrorKind::InternalError);
        assert!(oe.message.contains("llm_augmented"));
    }

    #[test]
    fn vm_pool_error_display() {
        let e = VmPoolError::Exhausted {
            kind: "ModelInference".into(),
            max: 200,
        };
        let s = format!("{e}");
        assert!(s.contains("exhausted"));
        assert!(s.contains("200"));
    }

    #[test]
    fn model_loader_error_display() {
        let e = ModelLoaderError::NotFound {
            id: "ghost".into(),
            version: "0.1.0".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("ghost"));
        assert!(s.contains("0.1.0"));
    }
}
