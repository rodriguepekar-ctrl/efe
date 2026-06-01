// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : retry classification table.
//! See architecture § 4.1.

use crate::retry::{NetworkErrorClass, RetryDecision, RetryOutcomeProbe};

use std::time::Duration;

/// **N27 + § 4.1** : classify a [`RetryOutcomeProbe`] using the
/// canonical retry table.
///
/// The returned [`RetryDecision::Retry { delay }`] carries a
/// PLACEHOLDER `delay` of zero — the [`ExponentialBackoff`]
/// policy overrides with the computed curve.  This separation
/// keeps the classification pure (no allocator, no clock) so
/// tests can pin determinism.
///
/// [`ExponentialBackoff`]: super::ExponentialBackoff
pub fn classify_default(probe: &RetryOutcomeProbe) -> RetryDecision {
    // **ADR-N27-3** : never retry post-first-byte downstream.
    if probe.post_first_byte_downstream {
        return RetryDecision::Abort;
    }

    // HTTP status takes precedence over network class — if we
    // have an HTTP response, we know exactly what the server
    // said.
    if let Some(status) = probe.http_status {
        return match status {
            // Retryable HTTP.
            408 | 429 | 500 | 502 | 503 | 504 | 529 => {
                RetryDecision::Retry { delay: Duration::ZERO }
            }
            // Non-retryable HTTP.
            400 | 401 | 403 | 404 | 413 => RetryDecision::Abort,
            // 2xx never reaches here (we only classify failures),
            // but be safe.
            200..=299 => RetryDecision::Abort,
            // Everything else (e.g. 405 / 451 / 422) defaults to
            // NO retry — we'd rather be visibly broken than
            // silently amplify.
            _ => RetryDecision::Abort,
        };
    }

    // Pure network error : classify by underlying cause.
    match probe.network_class {
        NetworkErrorClass::Timeout => RetryDecision::Retry { delay: Duration::ZERO },
        NetworkErrorClass::Connect => RetryDecision::Retry { delay: Duration::ZERO },
        NetworkErrorClass::BodyMidStream => {
            // Mid-stream body failure : only retryable if we
            // haven't sent any byte downstream yet (the loop
            // also checks via post_first_byte_downstream).
            RetryDecision::Retry { delay: Duration::ZERO }
        }
        NetworkErrorClass::Decode => RetryDecision::Abort, // our parser bug
        NetworkErrorClass::Other => RetryDecision::Abort,
        NetworkErrorClass::None => RetryDecision::Abort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_http(status: u16) -> RetryOutcomeProbe {
        RetryOutcomeProbe::http(status)
    }

    #[test]
    fn classify_retryable_http_statuses() {
        for s in [408u16, 429, 500, 502, 503, 504, 529] {
            assert!(
                matches!(classify_default(&probe_http(s)), RetryDecision::Retry { .. }),
                "status {s} should retry"
            );
        }
    }

    #[test]
    fn classify_non_retryable_http_statuses() {
        for s in [400u16, 401, 403, 404, 413] {
            assert_eq!(
                classify_default(&probe_http(s)),
                RetryDecision::Abort,
                "status {s} should abort"
            );
        }
    }

    #[test]
    fn classify_other_4xx_aborts_by_default() {
        // 405 Method Not Allowed, 422 Unprocessable Entity, 451 Legal
        for s in [405u16, 422, 451] {
            assert_eq!(
                classify_default(&probe_http(s)),
                RetryDecision::Abort,
                "status {s} should default-abort"
            );
        }
    }

    #[test]
    fn classify_post_first_byte_always_aborts_even_on_503() {
        let mut p = probe_http(503);
        p.post_first_byte_downstream = true;
        assert_eq!(classify_default(&p), RetryDecision::Abort);
    }

    #[test]
    fn classify_network_classes() {
        for (nc, expected_retry) in [
            (NetworkErrorClass::Timeout, true),
            (NetworkErrorClass::Connect, true),
            (NetworkErrorClass::BodyMidStream, true),
            (NetworkErrorClass::Decode, false),
            (NetworkErrorClass::Other, false),
            (NetworkErrorClass::None, false),
        ] {
            let probe = RetryOutcomeProbe {
                http_status: None,
                anthropic_error_type: None,
                retry_after: None,
                network_class: nc,
                post_first_byte_downstream: false,
            };
            let d = classify_default(&probe);
            if expected_retry {
                assert!(matches!(d, RetryDecision::Retry { .. }), "{nc:?}");
            } else {
                assert_eq!(d, RetryDecision::Abort, "{nc:?}");
            }
        }
    }

    #[test]
    fn classify_2xx_never_retries_defensive() {
        // 2xx should never reach the classifier (those are
        // successes), but defend against caller bugs : we'd
        // rather abort than infinite-retry.
        for s in [200u16, 201, 204] {
            assert_eq!(classify_default(&probe_http(s)), RetryDecision::Abort);
        }
    }
}
