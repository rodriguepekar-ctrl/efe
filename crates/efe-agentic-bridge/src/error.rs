// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : closed taxonomy of bridge errors.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeError {
    /// HTTP 429.
    #[error("rate limited")]
    RateLimited,

    /// HTTP 401/403.
    #[error("authentication failed")]
    Authentication,

    /// Tool-use shape violation.
    #[error("protocol violation : {message}")]
    ProtocolViolation { message: String },

    /// ContextBundle exceeds 256 KB cap.
    #[error("bundle too large : {size_bytes} bytes (cap {max_bytes})")]
    BundleTooLarge {
        size_bytes: usize,
        max_bytes: usize,
    },

    /// Other backend / network fault.
    #[error("backend : {message}")]
    Backend { message: String },

    /// **N26 + G10 + I5** : the N17 collector's `pre_call`
    /// returned an error before the HTTP dispatch.  This is
    /// fail-CLOSED : no HTTP call to Anthropic is made.  The
    /// underlying `ProvenanceError` (model registry mismatch,
    /// hash drift, store fault) is surfaced as the `reason`
    /// string.
    #[error("provenance verify failed : {reason}")]
    ProvenanceVerifyFailed { reason: String },

    // ─── Sprint 19 — N27 streaming + retry ────────────────────────

    /// **N27 + F21** : the streaming call was cancelled
    /// (downstream disconnect or explicit
    /// [`tokio_util::sync::CancellationToken::cancel`]).
    #[error("streaming call cancelled")]
    Cancelled,

    /// **N27 + F23** : total stream wall-clock exceeded the
    /// configured ceiling (default 5 min, see
    /// `EFE_BRIDGE_STREAM_TOTAL_TIMEOUT_MS`).
    #[error("stream total timeout exceeded")]
    TotalTimeout,

    /// **N27 + F22** : no SSE event for
    /// `STREAM_READ_TIMEOUT_MS` (default 60 s).
    #[error("stream read timeout (no event received)")]
    ReadTimeout,

    /// **N27** : SSE body ended without a `message_stop`.
    #[error("stream ended prematurely (no message_stop event)")]
    PrematureEof,

    /// **N27** : the SSE parser hit a fatal error (corrupt JSON,
    /// line too long, event too large, invalid UTF-8).
    #[error("SSE parse error : {reason}")]
    SseParseError { reason: String },

    /// **N27** : the response's content-type was not
    /// `text/event-stream` (Anthropic returned something else,
    /// or a proxy stripped the header).
    #[error("invalid response content-type for streaming : {content_type}")]
    InvalidResponseType { content_type: String },

    /// **N27 + F24** : tool-use input JSON could not be parsed
    /// after the full sequence of `input_json_delta` events.
    #[error("tool-use input JSON malformed : {reason}")]
    ToolUseJsonInvalid { reason: String },

    /// **N27 + ADR-N27-3** : retry was attempted after the first
    /// byte was already forwarded downstream.  Should never
    /// surface to the caller (the retry loop catches it) — kept
    /// as a distinct variant for forensic clarity.
    #[error("retry refused : first byte already downstream")]
    RetryAfterFirstByte,

    /// **N27** : HTTP status non-success (non-retryable or
    /// post-exhaustion).  Carries the status code and the
    /// (possibly truncated) response body.
    #[error("HTTP {status} : {body}")]
    HttpStatus { status: u16, body: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_non_empty() {
        let e = BridgeError::RateLimited;
        assert_eq!(format!("{e}"), "rate limited");
    }

    #[test]
    fn serde_round_trip_all_variants() {
        let cases = vec![
            BridgeError::RateLimited,
            BridgeError::Authentication,
            BridgeError::ProtocolViolation {
                message: "x".into(),
            },
            BridgeError::BundleTooLarge {
                size_bytes: 300_000,
                max_bytes: 256 * 1024,
            },
            BridgeError::Backend {
                message: "x".into(),
            },
            BridgeError::ProvenanceVerifyFailed {
                reason: "model_not_registered : anthropic/claude-sonnet-4-6@2023-06-01".into(),
            },
            // **Sprint 19 — N27** : the 8 new variants.
            BridgeError::Cancelled,
            BridgeError::TotalTimeout,
            BridgeError::ReadTimeout,
            BridgeError::PrematureEof,
            BridgeError::SseParseError {
                reason: "bad json".into(),
            },
            BridgeError::InvalidResponseType {
                content_type: "application/json".into(),
            },
            BridgeError::ToolUseJsonInvalid {
                reason: "expected }".into(),
            },
            BridgeError::RetryAfterFirstByte,
            BridgeError::HttpStatus {
                status: 503,
                body: "service unavailable".into(),
            },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: BridgeError = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn n27_error_display_messages_non_empty() {
        for e in [
            BridgeError::Cancelled,
            BridgeError::TotalTimeout,
            BridgeError::ReadTimeout,
            BridgeError::PrematureEof,
            BridgeError::RetryAfterFirstByte,
        ] {
            assert!(!format!("{e}").is_empty());
        }
    }
}
