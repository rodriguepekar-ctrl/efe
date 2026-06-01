// SPDX-License-Identifier: UNLICENSED
//! M30 — Region error taxonomy.
//!
//! Maps 1 : 1 to HTTP / gRPC status codes via [`RegionError::http_status`].

use thiserror::Error;

/// Canonical region error type.
#[derive(Debug, Error)]
pub enum RegionError {
    /// The current region is not in the configured cluster topology.
    #[error("region not configured: {region}")]
    RegionNotConfigured {
        /// Name of the region that was requested.
        region: String,
    },

    /// Peer health check failed (cross-region probe).
    #[error("peer health check failed: {region} ({reason})")]
    PeerUnhealthy {
        /// Name of the unhealthy peer region.
        region: String,
        /// Sanitized reason.
        reason: String,
    },

    /// Failover request was refused by the policy guard (e.g. the
    /// target region is not ready, or the drill mode is off).
    #[error("failover refused: {reason}")]
    FailoverRefused {
        /// Sanitized reason.
        reason: String,
    },

    /// Probe timed out.
    #[error("probe timeout after {elapsed_ms} ms")]
    ProbeTimeout {
        /// Elapsed wall-clock milliseconds.
        elapsed_ms: u64,
    },

    /// Probe transport-layer error.
    #[error("probe transport error: {source}")]
    ProbeTransport {
        /// Underlying error.
        #[source]
        source: reqwest::Error,
    },

    /// DR drill failed (assertion didn't hold).
    #[error("dr drill assertion failed: {step}: {expected} != {observed}")]
    DrillAssertion {
        /// Drill step name.
        step: String,
        /// Expected value.
        expected: String,
        /// Observed value.
        observed: String,
    },

    /// Catch-all for unexpected internal errors.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl RegionError {
    /// Stable string identifier for downstream mapping.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::RegionNotConfigured { .. } => "region.not_configured",
            Self::PeerUnhealthy { .. } => "region.peer_unhealthy",
            Self::FailoverRefused { .. } => "region.failover_refused",
            Self::ProbeTimeout { .. } => "region.probe_timeout",
            Self::ProbeTransport { .. } => "region.probe_transport",
            Self::DrillAssertion { .. } => "region.drill_assertion",
            Self::Internal { .. } => "region.internal",
        }
    }

    /// Recommended HTTP status mapping.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::RegionNotConfigured { .. } => 404,
            Self::FailoverRefused { .. } => 409,
            Self::PeerUnhealthy { .. } | Self::ProbeTimeout { .. } | Self::ProbeTransport { .. } => 503,
            Self::DrillAssertion { .. } | Self::Internal { .. } => 500,
        }
    }
}

/// Region-crate result alias.
pub type Result<T> = std::result::Result<T, RegionError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(
            RegionError::FailoverRefused { reason: "x".into() }.error_code(),
            "region.failover_refused"
        );
    }

    #[test]
    fn http_status_mapping_sane() {
        assert_eq!(
            RegionError::RegionNotConfigured { region: "x".into() }.http_status(),
            404
        );
        assert_eq!(
            RegionError::FailoverRefused { reason: "x".into() }.http_status(),
            409
        );
        assert_eq!(
            RegionError::Internal { reason: "x".into() }.http_status(),
            500
        );
    }
}
