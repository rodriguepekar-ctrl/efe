//! Deterministic mapping between `EfeError` and `tonic::Status`.
//!
//! See `M10_ARCHITECTURE.md` §6.4 for the authoritative table. The
//! mapping MUST remain stable within the `efe.v1` package: callers
//! switch behavior on the gRPC status code, so any change there is a
//! breaking change.

use efe_core::EfeError;
use tonic::{Code, Status};

/// Convert an [`EfeError`] into a properly-scoped [`tonic::Status`].
///
/// Rule `INVARIANT-1`: `EfeError::Internal(&'static str)` never leaks
/// its static message to the client body — the message would reveal
/// implementation paths. The server logs the full string; the status
/// carries only the generic `"internal error"` text.
#[must_use]
pub fn to_status(err: EfeError) -> Status {
    let (code, msg) = match err {
        EfeError::InvalidConfig(s) => (Code::InvalidArgument, s),
        EfeError::AlignmentBreach { delta, max } => (
            Code::FailedPrecondition,
            format!("alignment breach: delta={delta:.6} exceeds max={max:.6}"),
        ),
        EfeError::Internal(path) => {
            tracing::error!(path = %path, "internal error translated to Status");
            (Code::Internal, "internal error".to_string())
        }
        EfeError::HbmAllocFailed { requested } => (
            Code::ResourceExhausted,
            format!("HBM allocation failed: {requested} bytes requested"),
        ),
        EfeError::ModeCollapse { round } => (
            Code::FailedPrecondition,
            format!("mode collapse at round {round}"),
        ),
        EfeError::NumericalExplosion { round } => {
            tracing::error!(round, "numerical explosion translated to Status");
            (Code::Internal, "numerical explosion".to_string())
        }
        EfeError::Divergence { rounds, reason } => (
            Code::FailedPrecondition,
            format!("divergence after {rounds} rounds: {reason}"),
        ),
        EfeError::ConvergenceTimeout { rounds, sigma2 } => (
            Code::DeadlineExceeded,
            format!("convergence timeout at round {rounds}, sigma2={sigma2:.6}"),
        ),
        EfeError::BudgetExhausted { kind } => {
            (Code::ResourceExhausted, format!("budget exhausted: {kind}"))
        }
        // `EfeError` is `#[non_exhaustive]` — future variants fall
        // through here. Status::Unknown keeps the surface honest.
        _ => (Code::Unknown, "unknown domain error".to_string()),
    };
    Status::new(code, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_config_maps_to_invalid_argument() {
        let s = to_status(EfeError::InvalidConfig("bad spec".into()));
        assert_eq!(s.code(), Code::InvalidArgument);
        assert!(s.message().contains("bad spec"));
    }

    #[test]
    fn alignment_breach_maps_to_failed_precondition() {
        let s = to_status(EfeError::AlignmentBreach {
            delta: 0.9,
            max: 0.5,
        });
        assert_eq!(s.code(), Code::FailedPrecondition);
        assert!(s.message().contains("alignment breach"));
    }

    #[test]
    fn internal_never_leaks_static_msg() {
        let s = to_status(EfeError::Internal("secret/kernel/path/FOOBAR"));
        assert_eq!(s.code(), Code::Internal);
        assert!(!s.message().contains("secret/kernel/path/FOOBAR"));
        assert_eq!(s.message(), "internal error");
    }

    #[test]
    fn numerical_explosion_hides_details() {
        let s = to_status(EfeError::NumericalExplosion { round: 3 });
        assert_eq!(s.code(), Code::Internal);
        assert_eq!(s.message(), "numerical explosion");
    }

    #[test]
    fn hbm_alloc_failed_maps_to_resource_exhausted() {
        let s = to_status(EfeError::HbmAllocFailed { requested: 123_456 });
        assert_eq!(s.code(), Code::ResourceExhausted);
        assert!(s.message().contains("123456"));
    }

    #[test]
    fn mode_collapse_maps_to_failed_precondition() {
        let s = to_status(EfeError::ModeCollapse { round: 2 });
        assert_eq!(s.code(), Code::FailedPrecondition);
    }

    #[test]
    fn divergence_maps_to_failed_precondition() {
        let s = to_status(EfeError::Divergence {
            rounds: 5,
            reason: "spiral",
        });
        assert_eq!(s.code(), Code::FailedPrecondition);
        assert!(s.message().contains("spiral"));
    }

    #[test]
    fn convergence_timeout_maps_to_deadline_exceeded() {
        let s = to_status(EfeError::ConvergenceTimeout {
            rounds: 8,
            sigma2: 0.1,
        });
        assert_eq!(s.code(), Code::DeadlineExceeded);
    }

    #[test]
    fn budget_exhausted_maps_to_resource_exhausted() {
        let s = to_status(EfeError::BudgetExhausted { kind: "wall_clock" });
        assert_eq!(s.code(), Code::ResourceExhausted);
        assert!(s.message().contains("wall_clock"));
    }
}
