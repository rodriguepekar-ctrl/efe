// SPDX-License-Identifier: UNLICENSED
//! M31 — Metering error taxonomy.
//!
//! Stable string identifiers map 1:1 to downstream HTTP error codes
//! consumed by the `v1/usage/*` REST layer.

use thiserror::Error;

/// Canonical metering error type.
#[derive(Debug, Error)]
pub enum MeteringError {
    /// Increment value exceeds per-call cap (`MAX_PER_CALL`).
    #[error("increment too large: {observed} > cap {cap}")]
    IncrementTooLarge {
        /// Observed value.
        observed: u64,
        /// Configured cap.
        cap: u64,
    },

    /// Backing store unavailable (Redis transport).
    #[error("cache unavailable: {reason}")]
    CacheUnavailable {
        /// Sanitized reason.
        reason: String,
    },

    /// Cache encode / decode failure.
    #[error("cache encode/decode: {reason}")]
    CacheEncode {
        /// Sanitized reason.
        reason: String,
    },

    /// Persistence store unavailable.
    #[error("storage unavailable: {reason}")]
    StorageUnavailable {
        /// Sanitized reason.
        reason: String,
    },

    /// Invalid meter name (must be one of the 8 enumerated meters).
    #[error("unknown meter: {0}")]
    UnknownMeter(String),

    /// Tenant identifier failed validation.
    #[error("invalid tenant id: {0}")]
    InvalidTenant(String),

    /// Catch-all internal error.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl MeteringError {
    /// Stable string identifier.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::IncrementTooLarge { .. } => "metering.increment_too_large",
            Self::CacheUnavailable { .. } => "metering.cache_unavailable",
            Self::CacheEncode { .. } => "metering.cache_encode",
            Self::StorageUnavailable { .. } => "metering.storage_unavailable",
            Self::UnknownMeter(_) => "metering.unknown_meter",
            Self::InvalidTenant(_) => "metering.invalid_tenant",
            Self::Internal { .. } => "metering.internal",
        }
    }

    /// Recommended HTTP status.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::IncrementTooLarge { .. } => 413,
            Self::UnknownMeter(_) | Self::InvalidTenant(_) => 422,
            Self::CacheUnavailable { .. } | Self::StorageUnavailable { .. } => 503,
            Self::CacheEncode { .. } | Self::Internal { .. } => 500,
        }
    }

    /// `true` iff retrying the operation may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::CacheUnavailable { .. } | Self::StorageUnavailable { .. }
        )
    }
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, MeteringError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable() {
        assert_eq!(
            MeteringError::IncrementTooLarge { observed: 1, cap: 0 }.error_code(),
            "metering.increment_too_large"
        );
        assert_eq!(
            MeteringError::UnknownMeter("x".into()).error_code(),
            "metering.unknown_meter"
        );
    }

    #[test]
    fn http_mapping() {
        assert_eq!(
            MeteringError::IncrementTooLarge { observed: 1, cap: 0 }.http_status(),
            413
        );
        assert_eq!(
            MeteringError::UnknownMeter("x".into()).http_status(),
            422
        );
        assert_eq!(
            MeteringError::CacheUnavailable { reason: "x".into() }.http_status(),
            503
        );
    }

    #[test]
    fn retry_set_minimal() {
        assert!(MeteringError::CacheUnavailable { reason: "x".into() }.is_retryable());
        assert!(!MeteringError::UnknownMeter("x".into()).is_retryable());
    }
}
