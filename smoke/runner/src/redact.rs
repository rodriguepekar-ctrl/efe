//! PR #23 § 18 — log scrubber (AC24 + T107 mitigation).
//!
//! When `--redact-logs` is set, the orchestrator runs every check's
//! `error_summary` + `details` JSON values through [`scrub`].
//!
//! The patterns are conservative — false-positives are preferable to
//! a credential leak.  The list is **closed** : only the explicitly-
//! enumerated regexes are scrubbed.  Additions require a STABILITY
//! contract update (so operators reading scrubbed output know exactly
//! what's masked).

/// Replacement marker emitted for any matched secret pattern.
pub const MARKER: &str = "[REDACTED]";

/// Apply scrubbing to a free-form string.  Returns a new owned String.
///
/// Patterns scrubbed (closed v1.5) :
///   1. PEM blocks `-----BEGIN <TYPE>-----` (any kind)
///   2. Bearer tokens `Bearer <token>`
///   3. AWS-style keys (20-char `AKIA…` prefix)
///   4. GitHub tokens (`ghp_<40+>`, `gho_<40+>`, `ghs_<40+>`)
///   5. JWT-like `eyJ` 3-segment base64 strings
///   6. `--token <value>` CLI args
///   7. `password=...`, `secret=...`, `token=...` env-style assignments
#[must_use]
pub fn scrub(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        // PEM block — matches everything between BEGIN and END markers
        // of the same type.
        if input[i..].starts_with("-----BEGIN ") {
            if let Some(close) = input[i..].find("-----END ") {
                if let Some(close_end) = input[i + close..].find("-----") {
                    let block_end = i + close + close_end + 5;
                    out.push_str(MARKER);
                    i = block_end;
                    continue;
                }
            }
        }
        // Bearer tokens.
        if input[i..].to_ascii_lowercase().starts_with("bearer ") {
            out.push_str(MARKER);
            // Skip "bearer " + the token (alphanumeric + ._-).
            i += "bearer ".len();
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'_'
                    || bytes[i] == b'-')
            {
                i += 1;
            }
            continue;
        }
        // AWS / GitHub token prefixes.
        let lc = &input[i..].chars().take(4).collect::<String>().to_ascii_lowercase();
        if i + 16 <= bytes.len()
            && (lc.starts_with("akia") || lc.starts_with("ghp_") || lc.starts_with("gho_") || lc.starts_with("ghs_"))
        {
            // Replace the rest of the alphanumeric run.
            out.push_str(MARKER);
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            continue;
        }
        // JWT — eyJ + at least 2 dots in the next 100 chars.
        if input[i..].starts_with("eyJ") {
            let rest = &input[i..i + bytes.len().saturating_sub(i).min(512)];
            if rest.matches('.').count() >= 2 {
                out.push_str(MARKER);
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'.'
                        || bytes[i] == b'_'
                        || bytes[i] == b'-')
                {
                    i += 1;
                }
                continue;
            }
        }
        // Key=value pairs.
        for prefix in ["password=", "secret=", "token=", "api_key=", "apikey="] {
            if input[i..].to_ascii_lowercase().starts_with(prefix) {
                out.push_str(prefix);
                out.push_str(MARKER);
                i += prefix.len();
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'&'
                    && bytes[i] != b';'
                    && bytes[i] != b'"'
                {
                    i += 1;
                }
                continue;
            }
        }
        // --token <value> CLI arg.
        if input[i..].starts_with("--token ") {
            out.push_str("--token ");
            out.push_str(MARKER);
            i += "--token ".len();
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        // Default : pass through one byte.  Safe because all our patterns
        // are ASCII-prefixed.
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Apply scrubbing recursively to a JSON value (used to scrub the
/// `details` field of each [`crate::checks::CheckResult`]).
#[must_use]
pub fn scrub_json(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) => Value::String(scrub(s)),
        Value::Array(arr) => Value::Array(arr.iter().map(scrub_json).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                // Also scrub keys (defensive — unlikely but cheap).
                out.insert(scrub(k), scrub_json(v));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_pem_block() {
        let input = "before -----BEGIN PRIVATE KEY-----\nABCDEF\n-----END PRIVATE KEY----- after";
        let out = scrub(input);
        assert!(out.contains(MARKER));
        assert!(!out.contains("ABCDEF"));
        assert!(out.contains("before"));
        assert!(out.contains(" after"));
    }

    #[test]
    fn scrub_bearer_token() {
        let out = scrub("auth=Bearer abc123def456");
        assert!(out.contains(MARKER));
        assert!(!out.contains("abc123def456"));
    }

    #[test]
    fn scrub_aws_access_key() {
        let out = scrub("ek=AKIAIOSFODNN7EXAMPLE next");
        assert!(out.contains(MARKER));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(out.contains("next"));
    }

    #[test]
    fn scrub_github_token() {
        let out = scrub("ghp_1234567890abcdef1234567890abcdef1234");
        assert!(out.contains(MARKER));
        assert!(!out.contains("1234567890abcdef"));
    }

    #[test]
    fn scrub_jwt_three_segments() {
        let out = scrub("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signature");
        assert!(out.contains(MARKER));
        assert!(!out.contains("signature"));
    }

    #[test]
    fn scrub_password_assignment() {
        let out = scrub("DATABASE_URL=postgres://user:password=hunter2 other=here");
        assert!(out.contains("password=[REDACTED]"));
        assert!(!out.contains("hunter2"));
        assert!(out.contains("other=here"));
    }

    #[test]
    fn scrub_cli_token_flag() {
        let out = scrub("--token my-secret-value --foo bar");
        assert!(out.contains("--token [REDACTED]"));
        assert!(!out.contains("my-secret-value"));
        assert!(out.contains("--foo bar"));
    }

    #[test]
    fn scrub_plain_text_passes_through() {
        let s = "S1 boot : controller_ready=true participants_ready=3";
        assert_eq!(scrub(s), s);
    }

    #[test]
    fn scrub_json_recursive() {
        let v = serde_json::json!({
            "tokens": ["Bearer abc", "normal"],
            "kube": "AKIAIOSFODNN7EXAMPLE",
            "ok": 42
        });
        let s = scrub_json(&v);
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("AKIAIOSFODNN7"));
        assert!(!text.contains("abc"));
        assert!(text.contains("\"normal\""));
        assert!(text.contains("42"));
    }

    #[test]
    fn marker_is_redacted_string() {
        assert_eq!(MARKER, "[REDACTED]");
    }
}
