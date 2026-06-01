// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : retry policy module.
//!
//! See `docs/architecture/EFE_M50_N27_STREAMING_ANTHROPIC_RETRY_PROVENANCE_ARCHITECTURE.md`
//! § 4 (retry policy deep dive) and § 13 (ADR-N27-3, ADR-N27-9, ADR-N27-10).
//!
//! # Surface
//!
//! | Module | Purpose |
//! |---|---|
//! | [`classify`]      | HTTP status / network class → [`RetryDecision`] table |
//! | [`backoff`]       | [`ExponentialBackoff`] policy + `Retry-After` parser + jitter |
//! | [`loop_executor`] | [`RetryLoop`] driver — the actual loop that wraps an attempt fn |
//!
//! # Key types re-exported here
//!
//! * [`RetryPolicy`]         — trait : classify (attempt, probe) → decision
//! * [`RetryDecision`]       — Retry { delay } | Abort
//! * [`RetryOutcomeProbe`]   — input to classification (status, headers, network class)
//! * [`NetworkErrorClass`]   — Timeout / Connect / BodyMidStream / Decode / Other
//! * [`ExponentialBackoff`]  — default impl
//! * [`RetryLoop`]           — executor that runs `attempt_fn` until success or
//!                             policy says abort, capturing the [`RetrySummary`]
//!                             stamped into the N17 record.
//! * [`AttemptOutcome`]      — yielded by `attempt_fn` : Success(T) | Failure { error, probe }
//! * [`RetrySummary`]        — final tally for the provenance record.

use std::time::Duration;

pub mod backoff;
pub mod classify;
pub mod loop_executor;

pub use backoff::{
    parse_retry_after, ExponentialBackoff, MAX_RETRY_AFTER, N27_DEFAULT_BASE_MS,
    N27_DEFAULT_JITTER_PCT, N27_DEFAULT_MAX_PER_RETRY_MS, N27_DEFAULT_MAX_RETRIES,
    N27_DEFAULT_MAX_TOTAL_WALL_MS, N27_DEFAULT_OVERLOADED_BASE_MS,
    N27_DEFAULT_OVERLOADED_MAX_MS,
};
pub use classify::classify_default;
pub use loop_executor::{AttemptOutcome, RetryLoop, RetrySummary};

/// **N27 + ADR-N27-3** : the decision a [`RetryPolicy`] makes
/// after each failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry after `delay`.  The loop sleeps then increments the
    /// attempt counter.
    Retry { delay: Duration },
    /// Do not retry ; the original error is final.
    Abort,
}

/// **N27** : information about a failed attempt fed to the
/// [`RetryPolicy`] for classification.
///
/// The struct is exhaustively populated by the bridge ; the
/// policy is free to inspect only the fields it cares about.
#[derive(Debug, Clone)]
pub struct RetryOutcomeProbe {
    /// HTTP status code if the failure was an HTTP response.
    /// `None` for pure network / decode failures.
    pub http_status: Option<u16>,
    /// Anthropic-specific error type (`overloaded_error`,
    /// `rate_limit_error`, `api_error`, etc.) if the body
    /// could be parsed.  `None` otherwise.
    pub anthropic_error_type: Option<String>,
    /// Parsed `Retry-After` header value, capped at
    /// [`MAX_RETRY_AFTER`] (5 min).
    pub retry_after: Option<Duration>,
    /// Classification of the underlying network error if any.
    pub network_class: NetworkErrorClass,
    /// **ADR-N27-3** : `true` iff any byte was already forwarded
    /// to the downstream consumer.  Once `true`, the policy MUST
    /// return [`RetryDecision::Abort`] (no replay possible).
    pub post_first_byte_downstream: bool,
}

impl RetryOutcomeProbe {
    /// **N27** : a probe for a "pure HTTP status" failure with no
    /// network class.
    pub fn http(status: u16) -> Self {
        Self {
            http_status: Some(status),
            anthropic_error_type: None,
            retry_after: None,
            network_class: NetworkErrorClass::None,
            post_first_byte_downstream: false,
        }
    }
}

/// **N27** : classification of the underlying network error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorClass {
    /// Not a network error (e.g. pure HTTP status).
    None,
    /// TCP / TLS / read timeout.
    Timeout,
    /// TCP refused / reset before HTTP started.
    Connect,
    /// Mid-stream body read failure (post HTTP 200, mid-event).
    /// Only retryable iff `post_first_byte_downstream == false`.
    BodyMidStream,
    /// Local body-stream decode failure (our parser bug). Not
    /// retryable.
    Decode,
    /// Other / unclassified.
    Other,
}

impl NetworkErrorClass {
    /// **N27** : stable Prometheus label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::BodyMidStream => "body_mid_stream",
            Self::Decode => "decode",
            Self::Other => "other",
        }
    }
}

/// **N27** : the retry policy trait.
///
/// Implementors are typically zero-cost data carriers (the
/// configuration values) ; the logic lives in `classify`.
pub trait RetryPolicy: Send + Sync + 'static {
    /// Classify an attempt's outcome.
    ///
    /// * `attempt` is 0-indexed (0 = first attempt's failure →
    ///   policy decides whether to do the **first** retry).
    /// * `probe` carries the error metadata.
    fn classify(&self, attempt: u8, probe: &RetryOutcomeProbe) -> RetryDecision;

    /// Maximum retries this policy permits ; the loop also
    /// enforces this independently for safety.
    fn max_retries(&self) -> u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_decision_eq_semantics() {
        let a = RetryDecision::Retry {
            delay: Duration::from_millis(500),
        };
        let b = RetryDecision::Retry {
            delay: Duration::from_millis(500),
        };
        let c = RetryDecision::Abort;
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn probe_http_helper_defaults_post_byte_false() {
        let p = RetryOutcomeProbe::http(503);
        assert_eq!(p.http_status, Some(503));
        assert!(p.anthropic_error_type.is_none());
        assert!(p.retry_after.is_none());
        assert_eq!(p.network_class, NetworkErrorClass::None);
        assert!(!p.post_first_byte_downstream);
    }

    #[test]
    fn network_error_class_labels_stable() {
        assert_eq!(NetworkErrorClass::None.label(), "none");
        assert_eq!(NetworkErrorClass::Timeout.label(), "timeout");
        assert_eq!(NetworkErrorClass::Connect.label(), "connect");
        assert_eq!(NetworkErrorClass::BodyMidStream.label(), "body_mid_stream");
        assert_eq!(NetworkErrorClass::Decode.label(), "decode");
        assert_eq!(NetworkErrorClass::Other.label(), "other");
    }
}
