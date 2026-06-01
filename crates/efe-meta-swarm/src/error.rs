//! Error types for the meta-swarm crate.
//!
//! A single enum [`MetaSwarmError`] aggregates every fallible operation in
//! the crate.  The `?` operator is used pervasively to bubble errors up to
//! the user.  All variants are `non_exhaustive` so the enum can grow in
//! later phases (e.g. transport errors in Phase 2) without breaking
//! downstream pattern-matching.

use thiserror::Error;

/// Crate-wide error enumeration.
///
/// Each variant documents the situation in which it is raised and, where
/// appropriate, embeds the metric that triggered the error so the caller
/// can introspect it programmatically.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum MetaSwarmError {
    /// Two tensors did not share the expected dimension.
    #[error("dimension mismatch : expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Dimension that was expected (typically the declared dim of a
        /// lift, pull, or participant).
        expected: usize,
        /// Dimension that was actually provided at runtime.
        actual: usize,
    },

    /// A participant id was referenced that was not registered.
    #[error("participant '{id}' not registered in this meta-swarm")]
    ParticipantNotFound {
        /// The id that was looked up.
        id: String,
    },

    /// Two participants registered with the same id.
    #[error("duplicate participant id : '{id}'")]
    DuplicateParticipantId {
        /// The duplicated id.
        id: String,
    },

    /// The coupling parameters violate the invariant `alpha + gamma < 1`,
    /// required by the convexity preservation in Eq. (1*) of the A1
    /// architecture.
    #[error("invalid coupling : alpha + gamma must be < 1 (got alpha={alpha}, gamma={gamma})")]
    InvalidCoupling {
        /// Relaxation parameter alpha.
        alpha: f64,
        /// Coupling parameter gamma.
        gamma: f64,
    },

    /// The iteration reached `max_iters` without satisfying `tolerance`.
    /// Embeds the last residual and iteration count for diagnostics.
    #[error("convergence failed after {iters} iterations (residual={residual})")]
    ConvergenceFailed {
        /// Number of iterations completed before giving up.
        iters: usize,
        /// Final residual at termination.
        residual: f64,
    },

    /// An iterate strayed outside the declared safety set by more than the
    /// allowed numerical tolerance.  In Phase 1 (lenient mode), this is
    /// only emitted as a `Vec<SafetyViolation>` warning ; in Phase 3+
    /// (strict mode) it will become a fatal error.
    #[error("safety violation : distance to C is {distance}, tolerance is {tolerance}")]
    SafetyViolation {
        /// Measured distance to the safety set.
        distance: f64,
        /// Numerical tolerance configured.
        tolerance: f64,
    },

    /// A hypothesis (H1, H-π, H-ρ, H3) failed empirical verification.
    /// The string identifies which hypothesis.
    #[error("hypothesis check failed : '{hypothesis}' not satisfied")]
    HypothesisCheckFailed {
        /// One of `"H1"`, `"H3"`, `"H-pi"`, `"H-rho"`.
        hypothesis: &'static str,
    },

    /// The meta-swarm was built without at least one participant.
    #[error("meta-swarm must contain at least one participant")]
    EmptyMetaSwarm,

    /// A participant was added but its lift or pull was forgotten.
    #[error("participant '{id}' has no {missing}")]
    MissingOperator {
        /// The participant id.
        id: String,
        /// `"lift"` or `"pull"`.
        missing: &'static str,
    },

    /// I/O error during config or weights loading.
    #[error("io error : {0}")]
    Io(#[from] std::io::Error),

    /// YAML deserialisation error.
    #[error("yaml deserialisation error : {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// Internal panic / mutex poisoning.
    #[error("lock poisoned (concurrent panic in a participant)")]
    LockPoisoned,

    /// Catch-all for unexpected internal errors that don't fit elsewhere.
    /// **Never** raised in stable code paths — use only for inherently
    /// impossible-to-recover situations.
    #[error("internal invariant violated : {0}")]
    Internal(String),
}

/// Crate-wide `Result` alias.
pub type Result<T> = std::result::Result<T, MetaSwarmError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_dimension_mismatch() {
        let e = MetaSwarmError::DimensionMismatch { expected: 384, actual: 512 };
        let s = e.to_string();
        assert!(s.contains("384"));
        assert!(s.contains("512"));
    }

    #[test]
    fn error_display_invalid_coupling() {
        let e = MetaSwarmError::InvalidCoupling { alpha: 0.7, gamma: 0.5 };
        let s = e.to_string();
        assert!(s.contains("0.7"));
        assert!(s.contains("0.5"));
    }

    #[test]
    fn error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MetaSwarmError>();
    }
}
