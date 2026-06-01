//! PII redaction helpers.
//!
//! **Rule of thumb**: no user-supplied content (prompts, embeddings
//! derived from PII, authorization tokens) is ever emitted verbatim to
//! telemetry sinks. Use the helpers here every time you touch such a
//! field.

/// Fixed marker inserted in place of a redacted value.
pub const REDACTED_MARKER: &str = "<redacted>";

/// Field names that are **always** replaced by [`REDACTED_MARKER`] in
/// log records unless [`crate::TelemetryConfig::log_prompts`] is `true`.
pub const BLOCKED_FIELDS: &[&str] = &[
    "raw_prompt",
    "prompt",
    "user_text",
    "user_message",
    "authorization",
    "jwt_secret",
    "hs256_secrets",
];

/// Replace a raw user prompt with a short, non-reversible marker so logs
/// can still carry an indicator that "a prompt was processed" without
/// leaking its content.
///
/// The marker has the shape `<redacted:len=N>`. Callers that need
/// deduplication can compute a hash out-of-band; this helper deliberately
/// avoids bringing in a hashing dependency.
#[must_use]
pub fn redact_prompt(raw: &str) -> String {
    if raw.is_empty() {
        String::new()
    } else {
        format!("<redacted:len={}>", raw.len())
    }
}

/// Truncate a string to `max` bytes (guaranteed UTF-8 safe). Appends an
/// ellipsis `…` when truncated.
#[must_use]
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Walk characters until we exceed `max` bytes.
    let mut out = String::with_capacity(max + 3);
    for c in s.chars() {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out.push('\u{2026}'); // …
    out
}

/// Mask a JWT or similar secret to `"abcd…wxyz"`. Returns
/// [`REDACTED_MARKER`] for values too short to mask safely.
#[must_use]
pub fn mask_token(token: &str) -> String {
    if token.len() < 12 {
        REDACTED_MARKER.to_string()
    } else {
        format!("{}\u{2026}{}", &token[..4], &token[token.len() - 4..])
    }
}

/// Return `true` if `field_name` is on the blocklist.
#[must_use]
pub fn is_blocked(field_name: &str) -> bool {
    BLOCKED_FIELDS.iter().any(|b| *b == field_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_prompt_empty_is_empty() {
        assert_eq!(redact_prompt(""), "");
    }

    #[test]
    fn redact_prompt_non_empty_masks_length() {
        assert_eq!(redact_prompt("hello world"), "<redacted:len=11>");
    }

    #[test]
    fn redact_prompt_never_contains_original_substring() {
        let secret = "super-secret-prompt-do-not-leak";
        let r = redact_prompt(secret);
        assert!(!r.contains("secret"));
        assert!(!r.contains("prompt"));
    }

    #[test]
    fn truncate_keeps_short() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_cuts_long() {
        let out = truncate("abcdefghij", 5);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.len() <= 5 + '\u{2026}'.len_utf8());
    }

    #[test]
    fn truncate_is_utf8_safe() {
        // "éé" = 4 bytes (2 × 2-byte chars). Truncate at 3 bytes must
        // not split a char.
        let out = truncate("éé", 3);
        // "é" (2 bytes) + "…" (3 bytes) = 5 bytes, valid utf-8.
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn mask_token_short_is_fully_redacted() {
        assert_eq!(mask_token("short"), REDACTED_MARKER);
    }

    #[test]
    fn mask_token_long_shows_edges() {
        let t = "abcdefghijklmnop";
        let m = mask_token(t);
        assert!(m.starts_with("abcd"));
        assert!(m.ends_with("mnop"));
    }

    #[test]
    fn blocked_fields_contains_expected_entries() {
        for name in ["raw_prompt", "prompt", "authorization", "hs256_secrets"] {
            assert!(is_blocked(name), "{name} must be blocked");
        }
        assert!(!is_blocked("swarm_id"));
    }
}
