//! Unified error type for EFE.
//!
//! All fallible public functions return [`Result<T>`]. Never panic in the
//! hot path — use [`EfeError::Internal`] with a descriptive message for
//! unreachable invariants.
//!
//! # Classification
//!
//! Errors are categorized into three severities (see [`ErrorSeverity`])
//! to let the controller decide between retry, fail-fast, and escalation.

use thiserror::Error;

/// Severity of an [`EfeError`] as perceived by the orchestrator.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ErrorSeverity {
    /// Non-recoverable — escalate immediately to client / operator.
    Fatal,
    /// Recoverable by the controller (retry, respawn, inject noise).
    Recoverable,
    /// Terminal but expected (normal failure mode; returned to client).
    Terminal,
}

/// Unified error type for EFE operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EfeError {
    // ==========================================================
    // Fatal: escalate immediately, no retry.
    // ==========================================================
    /// Output centroid violated the alignment cone.
    #[error("alignment breach: delta={delta:.6} exceeds max={max:.6}")]
    AlignmentBreach {
        /// Observed projection distance.
        delta: f32,
        /// Configured maximum.
        max: f32,
    },

    /// Internal invariant was violated (indicates a bug, not a bad input).
    #[error("internal invariant violated: {0}")]
    Internal(&'static str),

    /// The configuration is not structurally valid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// HBM allocation failure (GPU out of memory).
    #[error("HBM allocation failed: requested {requested} bytes")]
    HbmAllocFailed {
        /// Bytes requested.
        requested: u64,
    },

    // ==========================================================
    // Recoverable by the controller.
    // ==========================================================
    /// Mode collapse detected (variance near zero too early).
    #[error("mode collapse detected at round {round}")]
    ModeCollapse {
        /// Round at which collapse was detected.
        round: u32,
    },

    /// Numerical instability (NaN / Inf) produced by a kernel.
    #[error("numerical explosion at round {round}")]
    NumericalExplosion {
        /// Round at which the explosion was detected.
        round: u32,
    },

    // ==========================================================
    // Terminal but expected.
    // ==========================================================
    /// Swarm diverged (variance increasing, gradient exploding).
    #[error("divergence detected after {rounds} rounds: {reason}")]
    Divergence {
        /// Number of rounds before divergence.
        rounds: u32,
        /// Human-readable reason.
        reason: &'static str,
    },

    /// Convergence was not reached within `max_rounds`.
    #[error("convergence timeout at round {rounds}, variance {sigma2:.6}")]
    ConvergenceTimeout {
        /// Rounds attempted.
        rounds: u32,
        /// Final variance observed (NaN if not measurable).
        sigma2: f32,
    },

    /// Resource budget exhausted (wall clock, energy, HBM).
    #[error("resource budget exhausted: {kind}")]
    BudgetExhausted {
        /// Which dimension of the budget was exhausted.
        kind: &'static str,
    },

    /// Requested template is not present in the registry.
    #[error("template missing: {0}")]
    TemplateMissing(&'static str),

    /// Kernel launch or execution failure.
    #[error("kernel failure: {0}")]
    KernelFailure(String),

    /// I/O failure (file, network).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization / deserialization failure.
    #[error("serde error: {0}")]
    Serde(String),
}

/// Convenience type alias.
pub type Result<T> = std::result::Result<T, EfeError>;

impl EfeError {
    /// Classification used by the orchestrator to decide retry/abort.
    #[must_use]
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::AlignmentBreach { .. }
            | Self::Internal(_)
            | Self::InvalidConfig(_)
            | Self::HbmAllocFailed { .. } => ErrorSeverity::Fatal,

            Self::ModeCollapse { .. } | Self::NumericalExplosion { .. } => {
                ErrorSeverity::Recoverable
            }

            Self::Divergence { .. }
            | Self::ConvergenceTimeout { .. }
            | Self::BudgetExhausted { .. }
            | Self::TemplateMissing(_)
            | Self::KernelFailure(_)
            | Self::Io(_)
            | Self::Serde(_) => ErrorSeverity::Terminal,
        }
    }

    /// Stable code for telemetry labels and gRPC status mapping.
    ///
    /// These values are part of the observability contract and must not
    /// change across MINOR bumps.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AlignmentBreach { .. } => "alignment_breach",
            Self::Internal(_) => "internal",
            Self::InvalidConfig(_) => "invalid_config",
            Self::HbmAllocFailed { .. } => "hbm_alloc_failed",
            Self::ModeCollapse { .. } => "mode_collapse",
            Self::NumericalExplosion { .. } => "numerical_explosion",
            Self::Divergence { .. } => "divergence",
            Self::ConvergenceTimeout { .. } => "convergence_timeout",
            Self::BudgetExhausted { .. } => "budget_exhausted",
            Self::TemplateMissing(_) => "template_missing",
            Self::KernelFailure(_) => "kernel_failure",
            Self::Io(_) => "io",
            Self::Serde(_) => "serde",
        }
    }

    /// True if the controller should attempt internal recovery.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        matches!(self.severity(), ErrorSeverity::Recoverable)
    }

    /// True if this error originates from a user input problem (vs system).
    #[must_use]
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidConfig(_) | Self::TemplateMissing(_) | Self::BudgetExhausted { .. }
        )
    }
}

impl From<serde_json::Error> for EfeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err.to_string())
    }
}

impl From<toml::de::Error> for EfeError {
    fn from(err: toml::de::Error) -> Self {
        Self::InvalidConfig(format!("TOML parse: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_contains_cause() {
        let e = EfeError::Divergence {
            rounds: 5,
            reason: "variance_increasing",
        };
        let s = format!("{e}");
        assert!(s.contains('5'));
        assert!(s.contains("variance_increasing"));
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let efe_err: EfeError = io_err.into();
        assert!(matches!(efe_err, EfeError::Io(_)));
        assert_eq!(efe_err.code(), "io");
    }

    #[test]
    fn alignment_breach_is_fatal() {
        let e = EfeError::AlignmentBreach {
            delta: 0.3,
            max: 0.15,
        };
        assert_eq!(e.severity(), ErrorSeverity::Fatal);
        assert!(!e.is_recoverable());
        assert!(!e.is_user_error());
    }

    #[test]
    fn mode_collapse_is_recoverable() {
        let e = EfeError::ModeCollapse { round: 1 };
        assert_eq!(e.severity(), ErrorSeverity::Recoverable);
        assert!(e.is_recoverable());
    }

    #[test]
    fn divergence_is_terminal() {
        let e = EfeError::Divergence {
            rounds: 5,
            reason: "test",
        };
        assert_eq!(e.severity(), ErrorSeverity::Terminal);
        assert!(!e.is_recoverable());
    }

    #[test]
    fn invalid_config_is_user_error() {
        let e = EfeError::InvalidConfig("bad value".into());
        assert!(e.is_user_error());
    }

    #[test]
    fn codes_are_stable_lowercase_snake() {
        let codes = [
            EfeError::AlignmentBreach {
                delta: 0.0,
                max: 0.0,
            }
            .code(),
            EfeError::Internal("x").code(),
            EfeError::ModeCollapse { round: 1 }.code(),
            EfeError::Divergence {
                rounds: 1,
                reason: "x",
            }
            .code(),
        ];
        for c in codes {
            assert!(c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        }
    }
}
