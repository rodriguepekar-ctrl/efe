//! Error types for the public SDK.
//!
//! These errors are visible to customers writing or invoking nano agents.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────
//  Identifier validation errors
// ─────────────────────────────────────────────────────────────────────

/// Errors raised when constructing a [`crate::types::NanoAgentId`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NanoAgentIdError {
    /// The string was empty.
    #[error("nano agent ID cannot be empty")]
    Empty,

    /// The string exceeded the max length of 64 chars.
    #[error("nano agent ID exceeds 64 chars (got {got})")]
    TooLong { got: usize },

    /// The first char was not a lowercase letter.
    #[error("nano agent ID must start with a lowercase letter (a-z)")]
    InvalidFirstChar,

    /// A char other than `[a-z0-9_]` was present.
    #[error("nano agent ID contains invalid character '{ch}' at position {pos} (only [a-z0-9_] allowed)")]
    InvalidChar { ch: char, pos: usize },
}

// ─────────────────────────────────────────────────────────────────────
//  Version validation errors
// ─────────────────────────────────────────────────────────────────────

/// Errors raised when constructing a [`crate::types::NanoAgentVersion`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NanoAgentVersionError {
    /// The string was empty.
    #[error("nano agent version cannot be empty")]
    Empty,

    /// The string was not a valid semver.
    #[error("nano agent version '{got}' is not valid semver: {reason}")]
    InvalidSemver { got: String, reason: String },
}

/// Errors raised when constructing a [`crate::types::VersionReq`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionReqError {
    /// The string was empty.
    #[error("version requirement cannot be empty")]
    Empty,

    /// The string was not a valid semver requirement.
    #[error("version requirement '{got}' is invalid: {reason}")]
    InvalidVersionReq { got: String, reason: String },
}

// ─────────────────────────────────────────────────────────────────────
//  Public OperationError — surfaced in SwarmResponse to customers
// ─────────────────────────────────────────────────────────────────────

/// Public-facing error for a single nano agent operation.
///
/// Returned to customers inside [`crate::types::OperationResult`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationError {
    /// Closed taxonomy of error kinds.
    pub kind: OperationErrorKind,

    /// Human-readable message.
    pub message: String,

    /// If this op was skipped because a dependency failed, the source op id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_op_id: Option<String>,
}

/// Closed enum of possible operation error kinds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OperationErrorKind {
    /// Input or output failed schema validation.
    SchemaValidationFailed,

    /// CPU / memory / wall-clock limit hit.
    ResourceExceeded,

    /// Generic runtime failure (panic, IO error, etc.).
    RuntimeError,

    /// A dependency op failed, so this op was skipped.
    DependencyFailed,

    /// Tenant budget exhausted.
    BudgetExceeded,

    /// Operation timed out.
    Timeout,

    /// Nano agent not found in catalog.
    AgentNotFound,

    /// The swarm request had an invalid DAG (cycle, etc.).
    InvalidDag,

    /// Internal server error (bug).
    InternalError,

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N4, N7, N8 additions
    // ─────────────────────────────────────────────────────────────────
    /// **N4** — Preflight failure : op crashed BEFORE reaching the
    /// runtime (catalog miss, input AST malformed, deadline already
    /// expired at dispatch).  The dispatcher emits a "negative trace"
    /// for these so the audit chain stays contiguous.
    PreflightFailure,

    /// **N7** — Swarm-level cost cap reached.  Distinct from
    /// `BudgetExceeded` (which is per-tenant) : `CostCapReached` is a
    /// per-`SwarmRequest.cost_cap_cents` hard stop set by the caller.
    /// Ops in-flight when the cap is hit are returned with this kind.
    CostCapReached,

    /// **N8** — Mode C input flagged by the prompt-injection filter.
    /// Returned before any LLM call is made.  The agent's
    /// `ComplianceTags.prompt_injection_strategy` controls whether
    /// this is `Block` (returns this error) or `Warn` (records but
    /// continues).
    PromptInjectionSuspected,

    /// **Sprint 8 — N15** : Backpressure / fairness denial.  The op
    /// (or the whole swarm) was rejected by the admission gate due
    /// to per-tenant or platform-wide capacity exhaustion.  Maps to
    /// HTTP 429 + `Retry-After` header.  The error message carries
    /// the [`BackpressureKind`] discriminator and a retry-after
    /// hint.
    Backpressure,

    /// **Sprint 9 — N17** : Model integrity violation — the
    /// `ModelRegistry`'s expected content hash did NOT match the
    /// model the provider actually served.  The op was rejected
    /// fail-closed (architecture ADR-7) ; a `ModelHashMismatch`
    /// provenance trace was emitted.
    ModelIntegrityViolation,

    /// **Sprint 10 — N18** : Publisher of this agent has been
    /// revoked.  Architecture G4 (revocation cascade) + I3.
    PublisherRevoked,

    /// **Sprint 10 — N18** : Publisher of this agent is currently
    /// suspended (sanctions match, key compromise investigation,
    /// or operator action).  Architecture G11.
    PublisherSuspended,

    /// **Sprint 10 — N18** : Publisher key lookup failed when
    /// verifying the agent's signature.  Architecture F1.
    PublisherKeyMissing,

    /// **Sprint 10 — N18** : Tier 2+ promotion attempted before
    /// the mandatory cooldown elapsed.  Architecture G5 + F14.
    CooldownNotElapsed,

    /// **Sprint 10 — N18** : The publisher's KYC tier is below the
    /// requirement for the requested catalog scope.
    /// Architecture G2 + I9.
    TierInsufficient,

    /// **Sprint 10 — N18** : Catalog signature does not verify
    /// against the publisher's registered public key.
    /// Architecture F2.
    SignatureMismatch,
}

impl OperationError {
    /// Construct a new operation error.
    pub fn new(kind: OperationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source_op_id: None,
        }
    }

    /// Mark this error as caused by a failed dependency.
    pub fn with_source(mut self, source_op_id: impl Into<String>) -> Self {
        self.source_op_id = Some(source_op_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_error_serializes_with_snake_case_kind() {
        let e = OperationError::new(
            OperationErrorKind::SchemaValidationFailed,
            "input missing required field 'x'",
        );
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["kind"], "schema_validation_failed");
        assert_eq!(j["message"], "input missing required field 'x'");
        // source_op_id absent → skipped in serialization
        assert!(j.get("source_op_id").is_none());
    }

    #[test]
    fn operation_error_with_source() {
        let e = OperationError::new(
            OperationErrorKind::DependencyFailed,
            "upstream op_id 'extract-1' failed",
        )
        .with_source("extract-1");
        assert_eq!(e.source_op_id.as_deref(), Some("extract-1"));
    }

    #[test]
    fn operation_error_round_trip_serde() {
        let e = OperationError::new(OperationErrorKind::Timeout, "exceeded 5000 ms");
        let s = serde_json::to_string(&e).unwrap();
        let e2: OperationError = serde_json::from_str(&s).unwrap();
        assert_eq!(e, e2);
    }

    #[test]
    fn identifier_error_display() {
        let e = NanoAgentIdError::InvalidChar { ch: '-', pos: 3 };
        let s = format!("{e}");
        assert!(s.contains("invalid character"));
        assert!(s.contains("'-'"));
        assert!(s.contains("position 3"));
    }

    #[test]
    fn version_error_display() {
        let e = NanoAgentVersionError::InvalidSemver {
            got: "1.x".to_string(),
            reason: "expected three components".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("1.x"));
        assert!(s.contains("not valid semver"));
    }
}
