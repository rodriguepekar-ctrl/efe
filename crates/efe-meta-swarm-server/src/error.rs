//! Crate-wide error type.
//!
//! Maps internal failures to gRPC `tonic::Status` codes.  See the
//! `MetaSwarmErrorCode` enum in `efe-meta-swarm-proto` for the
//! application-level codes that travel in the gRPC trailer.

use thiserror::Error;

/// Crate-wide error type.  Maps to gRPC `Status` via
/// [`ServerError::to_status`].
#[derive(Debug, Error)]
pub enum ServerError {
    /// Configuration error : invalid env var, missing file, etc.
    #[error("config error: {0}")]
    Config(String),

    /// Manifest error : parseable YAML but invalid content
    /// (dim mismatch, invalid kappa, etc.).
    #[error("manifest error: {0}")]
    Manifest(String),

    /// Discovery error : participant unreachable, DNS resolution
    /// failure.
    #[error("discovery error: {0}")]
    Discovery(String),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Lower-level tonic transport error.
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// Internal error : caller passed a malformed request that
    /// should have been caught earlier in the chain.
    #[error("internal: {0}")]
    Internal(String),

    /// Feature gated off this build.
    #[error("feature {0} is disabled in this build")]
    FeatureDisabled(&'static str),

    /// Auth failure.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
}

impl ServerError {
    /// Map an internal error to a gRPC `Status` ready to return from
    /// an RPC handler.
    ///
    /// Mapping table mirrors `errors.proto` in `efe-meta-swarm-proto`.
    #[must_use]
    pub fn to_status(&self) -> tonic::Status {
        use tonic::Code;
        let code = match self {
            ServerError::Config(_) | ServerError::Manifest(_) => Code::FailedPrecondition,
            ServerError::Discovery(_) => Code::Unavailable,
            ServerError::Io(_) | ServerError::Transport(_) | ServerError::Internal(_) => {
                Code::Internal
            }
            ServerError::FeatureDisabled(_) => Code::Unimplemented,
            ServerError::Unauthorized(_) => Code::Unauthenticated,
        };
        tonic::Status::new(code, self.to_string())
    }
}

impl From<ServerError> for tonic::Status {
    fn from(e: ServerError) -> Self {
        e.to_status()
    }
}

/// Canonical translation Phase-1 [`MetaSwarmError`] → gRPC `Status`.
///
/// Shared by the controller's `Iterate` orchestration (PR #8) and the
/// participant's `Step` / `Reset` / `State` handlers (PR #9) so both
/// sides return the same code for the same underlying fault.
///
/// `MetaSwarmError` is `#[non_exhaustive]` ; the wildcard arm maps
/// every future variant to `INTERNAL`, the safe default for unknown
/// failure modes.
///
/// [`MetaSwarmError`]: efe_meta_swarm::MetaSwarmError
#[must_use]
pub fn meta_swarm_error_to_status(e: &efe_meta_swarm::MetaSwarmError) -> tonic::Status {
    use efe_meta_swarm::MetaSwarmError as E;
    use tonic::Status;
    match e {
        // Caller-fault variants → FAILED_PRECONDITION so the client
        // knows to fix the request, not retry.
        E::DimensionMismatch { .. }
        | E::InvalidCoupling { .. }
        | E::ParticipantNotFound { .. }
        | E::DuplicateParticipantId { .. }
        | E::EmptyMetaSwarm
        | E::MissingOperator { .. } => Status::failed_precondition(e.to_string()),
        // Everything else (convergence, safety, hypothesis, IO, YAML,
        // lock poisoning, internal invariants, future variants) →
        // INTERNAL.  Retry may or may not help ; the operator should
        // inspect server logs.
        _ => Status::internal(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn config_maps_to_failed_precondition() {
        let e = ServerError::Config("bad EFE_META_SWARM_X".into());
        assert_eq!(e.to_status().code(), Code::FailedPrecondition);
    }

    #[test]
    fn manifest_maps_to_failed_precondition() {
        let e = ServerError::Manifest("invalid kappa".into());
        assert_eq!(e.to_status().code(), Code::FailedPrecondition);
    }

    #[test]
    fn discovery_maps_to_unavailable() {
        let e = ServerError::Discovery("participant-1 unreachable".into());
        assert_eq!(e.to_status().code(), Code::Unavailable);
    }

    #[test]
    fn unauthorized_maps_to_unauthenticated() {
        let e = ServerError::Unauthorized("bearer expired".into());
        assert_eq!(e.to_status().code(), Code::Unauthenticated);
    }

    #[test]
    fn feature_disabled_maps_to_unimplemented() {
        let e = ServerError::FeatureDisabled("reflection");
        assert_eq!(e.to_status().code(), Code::Unimplemented);
    }

    #[test]
    fn meta_swarm_error_dim_mismatch_is_failed_precondition() {
        let e = efe_meta_swarm::MetaSwarmError::DimensionMismatch {
            expected: 8,
            actual: 16,
        };
        assert_eq!(
            meta_swarm_error_to_status(&e).code(),
            Code::FailedPrecondition
        );
    }

    #[test]
    fn meta_swarm_error_invalid_coupling_is_failed_precondition() {
        let e = efe_meta_swarm::MetaSwarmError::InvalidCoupling {
            alpha: 0.9,
            gamma: 0.5,
        };
        assert_eq!(
            meta_swarm_error_to_status(&e).code(),
            Code::FailedPrecondition
        );
    }

    #[test]
    fn meta_swarm_error_internal_is_internal() {
        let e = efe_meta_swarm::MetaSwarmError::Internal("bug".into());
        assert_eq!(meta_swarm_error_to_status(&e).code(), Code::Internal);
    }

    #[test]
    fn meta_swarm_error_convergence_failed_is_internal() {
        let e = efe_meta_swarm::MetaSwarmError::ConvergenceFailed {
            iters: 100,
            residual: 0.5,
        };
        assert_eq!(meta_swarm_error_to_status(&e).code(), Code::Internal);
    }
}
