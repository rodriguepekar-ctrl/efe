//! Closed-taxonomy enums for Prometheus label values (PR #17 § 4 / AD4 / AD5).
//!
//! Every label whose value space is finite is represented here as an
//! `enum` + an `as_str()` projection.  The enum keeps callers honest
//! (compiler-enforced exhaustiveness) and the `all()` constructor
//! drives the pre-touch policy (§ 15 of the architecture doc) so
//! Prometheus `gather()` emits every (label-value) tuple with value
//! `0` from boot, before any observation.
//!
//! Adding a new variant to ANY enum in this file requires :
//!
//!   * A chart-minor-version bump (AD3 — public API).
//!   * Updating the dashboards that filter on the label.
//!   * Updating the Rego rule that lints the closed taxonomy in PR #15.
//!
//! Removing a variant requires a one-minor-version deprecation notice
//! (AD10 — stability contract).

// ─────────────────────────────────────────────────────────────────────
// Result — top-level outcome bucket for Iterate / Step.
// ─────────────────────────────────────────────────────────────────────

/// Top-level outcome bucket for an Iterate or Step gRPC call.
///
/// See § 6.1.b for the full mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// `Ok(_)` return path.
    Ok,
    /// `Err(_)` return path with a non-deadline, non-throttle, non-cancel error.
    Error,
    /// `ServerError::Deadline` — wall-clock budget exhausted.
    Deadline,
    /// `tonic::Code::Cancelled` propagated from the gRPC client.
    Canceled,
    /// `ServerError::Throttle` — `InflightSemaphore` exhausted.
    Throttled,
}

impl Outcome {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Deadline => "deadline",
            Self::Canceled => "canceled",
            Self::Throttled => "throttled",
        }
    }

    /// Every variant.  Used by [`super::IterateMetrics::register`] +
    /// siblings to pre-touch the closed product.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Ok,
            Self::Error,
            Self::Deadline,
            Self::Canceled,
            Self::Throttled,
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────
// FailureReason — secondary discriminant carried alongside Outcome::Error.
// ─────────────────────────────────────────────────────────────────────

/// Closed taxonomy for the `failure_reason` label per AD5.
///
/// Mapped from [`crate::error::ServerError`] discriminants by
/// [`from_server_error`] (AD18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureReason {
    /// Generic timeout class (no `Deadline` variant fired but the
    /// operation timed out at the transport / outer-loop level).
    Timeout,
    /// Tensor `dim` did not match the participant's manifest row.
    DimMismatch,
    /// Safety projection rejected the post-step result.
    SafetyViolation,
    /// Participant connection lost mid-Iterate.
    ParticipantDown,
    /// Internal-only error (post-condition failure, invariant break).
    Internal,
    /// `ServerError::Deadline` — explicit deadline-exceeded path.  Distinct
    /// from `Timeout` to surface "we hit our own wall-clock budget" vs
    /// "the network was slow".
    DeadlineExceeded,
    /// Client cancellation propagated from the request.
    Cancelled,
    /// Catch-all when none of the above apply.  Operators chasing a
    /// growing `other` bucket should add a discriminator.
    Other,
}

impl FailureReason {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::DimMismatch => "dim_mismatch",
            Self::SafetyViolation => "safety_violation",
            Self::ParticipantDown => "participant_down",
            Self::Internal => "internal",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::Other => "other",
        }
    }

    /// All 8 variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Timeout,
            Self::DimMismatch,
            Self::SafetyViolation,
            Self::ParticipantDown,
            Self::Internal,
            Self::DeadlineExceeded,
            Self::Cancelled,
            Self::Other,
        ]
    }

    /// Map a [`crate::error::ServerError`] to the corresponding
    /// failure reason.  Compile-time exhaustive : adding a `ServerError`
    /// variant forces a build-fail here, so the taxonomy never silently
    /// drifts (AC31).
    #[must_use]
    pub fn from_server_error(err: &crate::error::ServerError) -> Self {
        use crate::error::ServerError;
        match err {
            ServerError::Config(_) => Self::Internal,
            ServerError::Manifest(_) => Self::Internal,
            ServerError::Discovery(_) => Self::ParticipantDown,
            ServerError::Io(_) => Self::Internal,
            ServerError::Transport(_) => Self::ParticipantDown,
            ServerError::Internal(_) => Self::Internal,
            ServerError::FeatureDisabled(_) => Self::Internal,
            ServerError::Unauthorized(_) => Self::Other,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Convergence — outcome of the inner Phase-1 iteration loop.
// ─────────────────────────────────────────────────────────────────────

/// Outcome of the inner `MetaSwarm::iterate` Phase-1 loop, surfaced via
/// the `convergence` label on iteration-count and residual-error
/// histograms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Convergence {
    /// Residual fell below the configured `tolerance`.
    Converged,
    /// Inner loop hit `manifest.iteration.max_iters`.
    MaxIters,
    /// Residual grew (Phase-3 detection ; set to 0 in Phase 2).
    Diverged,
    /// Cancellation propagated mid-loop.
    Canceled,
}

impl Convergence {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Converged => "converged",
            Self::MaxIters => "max_iters",
            Self::Diverged => "diverged",
            Self::Canceled => "canceled",
        }
    }

    /// All 4 variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Converged,
            Self::MaxIters,
            Self::Diverged,
            Self::Canceled,
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────
// SafetyKind — the configured safety variant on a manifest.
// ─────────────────────────────────────────────────────────────────────

/// The manifest's safety policy variant.  Mirrors
/// [`crate::manifest::SafetyManifest`] enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafetyKind {
    /// O(d) box / ball test against a fixed radius.
    Box,
    /// O(d × constraints) polyhedral set membership test.
    Poly,
    /// O(d × hull-vertices) convex-hull membership test (Phase-3 only).
    Hull,
}

impl SafetyKind {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Poly => "poly",
            Self::Hull => "hull",
        }
    }

    /// All 3 variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Box, Self::Poly, Self::Hull]
    }
}

// ─────────────────────────────────────────────────────────────────────
// PayloadDirection — `in` (controller-side, received) vs `out` (sent).
// ─────────────────────────────────────────────────────────────────────

/// Direction of a Step payload across the wire.  Used as the
/// `direction` label on `efe_meta_swarm_step_payload_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadDirection {
    /// `pull` tensor received by the participant from the controller.
    In,
    /// Post-step tensor sent by the participant back to the controller.
    Out,
}

impl PayloadDirection {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }

    /// Both variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::In, Self::Out]
    }
}

// ─────────────────────────────────────────────────────────────────────
// ReloadTrigger — what kicked the manifest reload.
// ─────────────────────────────────────────────────────────────────────

/// The trigger that caused a manifest reload.  Labels
/// `efe_meta_swarm_manifest_reload_total{trigger=...}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReloadTrigger {
    /// `SIGHUP` signal received by the binary (PR #12 path).
    Sighup,
    /// `ReloadConfig` gRPC RPC (PR #12 / PR #18 wire path).
    Grpc,
    /// File-system watcher fired (PR #16 TLS cert path, reused by PR #17 for manifest).
    FileChange,
}

impl ReloadTrigger {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sighup => "sighup",
            Self::Grpc => "grpc",
            Self::FileChange => "file_change",
        }
    }

    /// All 3 variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Sighup, Self::Grpc, Self::FileChange]
    }
}

// ─────────────────────────────────────────────────────────────────────
// ReloadOutcome — success / failure for the reload-total counter.
// ─────────────────────────────────────────────────────────────────────

/// Top-level outcome of a manifest reload attempt.  Labels
/// `efe_meta_swarm_manifest_reload_total{result=...}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReloadOutcome {
    /// Reload completed successfully.
    Ok,
    /// Reload was rejected (any reason).
    Error,
}

impl ReloadOutcome {
    /// Lowercase snake-case Prometheus label form.  STABLE per AD10.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }

    /// Both variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Ok, Self::Error]
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ServerError;

    #[test]
    fn outcome_label_strings_are_stable() {
        // Public API : any change here breaks dashboards.
        assert_eq!(Outcome::Ok.as_str(), "ok");
        assert_eq!(Outcome::Error.as_str(), "error");
        assert_eq!(Outcome::Deadline.as_str(), "deadline");
        assert_eq!(Outcome::Canceled.as_str(), "canceled");
        assert_eq!(Outcome::Throttled.as_str(), "throttled");
    }

    #[test]
    fn failure_reason_label_strings_are_stable() {
        assert_eq!(FailureReason::Timeout.as_str(), "timeout");
        assert_eq!(FailureReason::DimMismatch.as_str(), "dim_mismatch");
        assert_eq!(FailureReason::SafetyViolation.as_str(), "safety_violation");
        assert_eq!(FailureReason::ParticipantDown.as_str(), "participant_down");
        assert_eq!(FailureReason::Internal.as_str(), "internal");
        assert_eq!(
            FailureReason::DeadlineExceeded.as_str(),
            "deadline_exceeded"
        );
        assert_eq!(FailureReason::Cancelled.as_str(), "cancelled");
        assert_eq!(FailureReason::Other.as_str(), "other");
    }

    #[test]
    fn failure_reason_all_returns_eight() {
        assert_eq!(FailureReason::all().len(), 8);
    }

    #[test]
    fn outcome_all_returns_five() {
        assert_eq!(Outcome::all().len(), 5);
    }

    #[test]
    fn convergence_label_strings_are_stable() {
        assert_eq!(Convergence::Converged.as_str(), "converged");
        assert_eq!(Convergence::MaxIters.as_str(), "max_iters");
        assert_eq!(Convergence::Diverged.as_str(), "diverged");
        assert_eq!(Convergence::Canceled.as_str(), "canceled");
    }

    #[test]
    fn safety_kind_label_strings_are_stable() {
        assert_eq!(SafetyKind::Box.as_str(), "box");
        assert_eq!(SafetyKind::Poly.as_str(), "poly");
        assert_eq!(SafetyKind::Hull.as_str(), "hull");
    }

    #[test]
    fn payload_direction_label_strings_are_stable() {
        assert_eq!(PayloadDirection::In.as_str(), "in");
        assert_eq!(PayloadDirection::Out.as_str(), "out");
    }

    #[test]
    fn reload_trigger_label_strings_are_stable() {
        assert_eq!(ReloadTrigger::Sighup.as_str(), "sighup");
        assert_eq!(ReloadTrigger::Grpc.as_str(), "grpc");
        assert_eq!(ReloadTrigger::FileChange.as_str(), "file_change");
    }

    #[test]
    fn reload_outcome_label_strings_are_stable() {
        assert_eq!(ReloadOutcome::Ok.as_str(), "ok");
        assert_eq!(ReloadOutcome::Error.as_str(), "error");
    }

    #[test]
    fn server_error_config_maps_to_internal() {
        let err = ServerError::Config("test".into());
        assert_eq!(
            FailureReason::from_server_error(&err),
            FailureReason::Internal
        );
    }

    #[test]
    fn server_error_manifest_maps_to_internal() {
        let err = ServerError::Manifest("test".into());
        assert_eq!(
            FailureReason::from_server_error(&err),
            FailureReason::Internal
        );
    }

    #[test]
    fn server_error_discovery_maps_to_participant_down() {
        let err = ServerError::Discovery("test".into());
        assert_eq!(
            FailureReason::from_server_error(&err),
            FailureReason::ParticipantDown
        );
    }

    #[test]
    fn server_error_internal_maps_to_internal() {
        let err = ServerError::Internal("test".into());
        assert_eq!(
            FailureReason::from_server_error(&err),
            FailureReason::Internal
        );
    }

    #[test]
    fn server_error_unauthorized_maps_to_other() {
        let err = ServerError::Unauthorized("test".into());
        assert_eq!(FailureReason::from_server_error(&err), FailureReason::Other);
    }
}
