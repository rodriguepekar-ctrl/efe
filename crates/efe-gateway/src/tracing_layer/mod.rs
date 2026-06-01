// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : W3C Trace Context propagation — § 7.4 + G10.
//!
//! Parses `traceparent` header on incoming requests ; generates one
//! if absent.  The trace_id is stored as a request extension so
//! handlers + error path can read it.

use std::sync::OnceLock;

use axum::http::HeaderValue;

/// **N22 + W3C Trace Context** : a 16-byte (32 hex chars) trace_id.
///
/// Per W3C Trace Context spec, `traceparent` is :
/// `version-traceid-parentid-flags`
/// e.g. `00-0123456789abcdef0123456789abcdef-0123456789abcdef-01`.
pub const VERSION: &str = "00";
pub const FLAGS: &str = "01"; // sampled

/// **N22** : extract the trace_id (32-hex string) from a
/// `traceparent` header value.  Returns `None` on malformed input.
pub fn parse_trace_id(header: &str) -> Option<String> {
    let parts: Vec<&str> = header.split('-').collect();
    if parts.len() < 4 {
        return None;
    }
    let trace_id = parts[1];
    if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if trace_id.chars().all(|c| c == '0') {
        // W3C : all-zero trace_id is invalid.
        return None;
    }
    Some(trace_id.to_lowercase())
}

/// **N22** : extract or generate a trace_id.  If `header` is
/// `Some(v)` and parses as a valid traceparent, use that ; else
/// generate a fresh one.
pub fn trace_id_from_header(header: Option<&HeaderValue>) -> String {
    if let Some(h) = header.and_then(|h| h.to_str().ok()) {
        if let Some(t) = parse_trace_id(h) {
            return t;
        }
    }
    generate_trace_id()
}

/// **N22** : Generate a fresh trace_id as 32 lowercase hex chars
/// (128-bit).
pub fn generate_trace_id() -> String {
    let u = uuid::Uuid::new_v4();
    // UUID v4 = 128 bits.  Format as 32 lowercase hex chars,
    // stripping dashes.
    u.simple().to_string()
}

/// **N22** : Format a traceparent header value for outbound calls.
pub fn format_traceparent(trace_id: &str, parent_id: &str) -> String {
    let _ = static_inits();
    format!("{VERSION}-{trace_id}-{parent_id}-{FLAGS}")
}

fn static_inits() -> &'static () {
    // Touch a OnceLock so the constants module isn't optimised away
    // in a future no-op refactor.
    static O: OnceLock<()> = OnceLock::new();
    O.get_or_init(|| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_traceparent() {
        let h = "00-0123456789abcdef0123456789abcdef-aaaaaaaaaaaaaaaa-01";
        let id = parse_trace_id(h).unwrap();
        assert_eq!(id, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn parse_uppercase_trace_id_normalises() {
        let h = "00-0123456789ABCDEF0123456789ABCDEF-aaaaaaaaaaaaaaaa-01";
        let id = parse_trace_id(h).unwrap();
        assert_eq!(id, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn parse_too_few_parts_returns_none() {
        assert!(parse_trace_id("00-foo-bar").is_none());
    }

    #[test]
    fn parse_wrong_trace_id_length_returns_none() {
        let h = "00-abc-def-01";
        assert!(parse_trace_id(h).is_none());
    }

    #[test]
    fn parse_non_hex_returns_none() {
        let h = "00-ZZZZ56789abcdef0123456789abcdef-aaaaaaaaaaaaaaaa-01";
        assert!(parse_trace_id(h).is_none());
    }

    #[test]
    fn parse_all_zero_trace_id_invalid() {
        let h = "00-00000000000000000000000000000000-aaaaaaaaaaaaaaaa-01";
        assert!(parse_trace_id(h).is_none());
    }

    #[test]
    fn generate_returns_32_hex_chars() {
        let id = generate_trace_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_is_unique() {
        let a = generate_trace_id();
        let b = generate_trace_id();
        assert_ne!(a, b);
    }

    #[test]
    fn trace_id_from_header_uses_provided() {
        let h = HeaderValue::from_static(
            "00-0123456789abcdef0123456789abcdef-aaaaaaaaaaaaaaaa-01",
        );
        let id = trace_id_from_header(Some(&h));
        assert_eq!(id, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn trace_id_from_header_generates_when_absent() {
        let id = trace_id_from_header(None);
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn trace_id_from_header_generates_when_malformed() {
        let h = HeaderValue::from_static("garbage");
        let id = trace_id_from_header(Some(&h));
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn format_traceparent_round_trips() {
        let h = format_traceparent(
            "0123456789abcdef0123456789abcdef",
            "0123456789abcdef",
        );
        assert_eq!(
            h,
            "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01"
        );
        let id = parse_trace_id(&h).unwrap();
        assert_eq!(id, "0123456789abcdef0123456789abcdef");
    }
}
