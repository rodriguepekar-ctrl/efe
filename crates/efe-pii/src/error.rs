//! Error types for the PII crate.

use thiserror::Error;

/// All operations return this error.
#[derive(Debug, Error)]
pub enum PiiError {
    /// HTTP error talking to the Presidio sidecar.
    #[error("presidio HTTP error : {0}")]
    Http(String),

    /// Presidio returned a non-success status.
    #[error("presidio backend status {status} : {body}")]
    BackendStatus {
        /// HTTP status code returned by Presidio.
        status: u16,
        /// Truncated response body for diagnostics.
        body: String,
    },

    /// Response JSON could not be parsed.
    #[error("presidio response decode : {0}")]
    Decode(#[from] serde_json::Error),

    /// Presidio took longer than the configured timeout.
    #[error("presidio timeout after {0:?}")]
    Timeout(std::time::Duration),

    /// Circuit breaker is open ; caller should degrade.
    #[error("presidio circuit breaker open")]
    CircuitOpen,

    /// Per-tenant configuration invalid.
    #[error("invalid PII policy : {0}")]
    InvalidPolicy(String),
}

impl From<reqwest::Error> for PiiError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Self::Timeout(std::time::Duration::from_millis(0))
        } else {
            Self::Http(e.to_string())
        }
    }
}

/// Convenience alias.
pub type PiiResult<T> = Result<T, PiiError>;
