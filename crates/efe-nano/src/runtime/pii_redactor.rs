//! PII redaction adapter for Mode C nano agents.
//!
//! # Architecture
//!
//! Sprint 5 introduces a `PiiRedactor` that walks the JSON input tree
//! before passing it to the LLM.  Each string value is run through a
//! `PiiBackend` (trait) which returns the input with PII spans replaced
//! by tokens (e.g. `[EMAIL_1]`, `[PHONE_2]`).
//!
//! # Sprint 5 ship vs production wiring
//!
//! Sprint 5 ships two backends :
//!
//! 1. **`RegexPiiBackend`** — pure Rust regex-based detection for
//!    EMAIL / PHONE / SSN / CREDIT_CARD / IPV4 / IBAN.  This is the
//!    Sprint 5 default backend : zero external dependencies, fast
//!    (~ 1-5 ms per KB), deterministic.
//! 2. **`MockPiiBackend`** — test fixture that records every redact call
//!    and returns a configurable response.
//!
//! Phase 4 swaps in a `PresidioPiiBackend` that proxies to the
//! `efe-pii` (M20) crate's Microsoft Presidio integration, adding NER-
//! based detection for PERSON / LOCATION / MEDICAL_LICENSE / etc.
//!
//! The trait surface is **stable** : Phase 4 changes the impl, not the
//! contract.

use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::PiiError;

// ─────────────────────────────────────────────────────────────────────
//  PiiBackend trait
// ─────────────────────────────────────────────────────────────────────

/// Backend that performs the actual PII detection + replacement on a
/// single string.
#[async_trait]
pub trait PiiBackend: Send + Sync + 'static {
    /// Redact PII from `input`, returning the redacted string.
    /// Tokens like `[EMAIL_n]` should be inserted in place of detected
    /// spans.  The mapping `n → original` is **not** persisted at this
    /// trait level (Phase 5+ may add cross-call PII correlation).
    async fn redact(&self, input: &str) -> Result<String, PiiError>;
}

/// Type alias for an owned redactor backend.
pub type SharedPiiBackend = Arc<dyn PiiBackend>;

// ─────────────────────────────────────────────────────────────────────
//  PiiRedactor — walks JSON tree
// ─────────────────────────────────────────────────────────────────────

/// Walks a JSON value tree, applying `PiiBackend::redact` to every
/// string field.  Object keys are NOT redacted (they're structural).
pub struct PiiRedactor {
    backend: SharedPiiBackend,
}

impl PiiRedactor {
    pub fn new(backend: SharedPiiBackend) -> Self {
        Self { backend }
    }

    /// Convenience : default backend is `RegexPiiBackend`.
    pub fn with_default_regex_backend() -> Self {
        Self::new(Arc::new(RegexPiiBackend::new()))
    }

    /// Walk a JSON value, redacting every string field.
    pub async fn redact_json(
        &self,
        value: serde_json::Value,
    ) -> Result<serde_json::Value, PiiError> {
        self.walk(value).await
    }

    fn walk(
        &self,
        v: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, PiiError>> + Send + '_>> {
        Box::pin(async move {
            match v {
                serde_json::Value::String(s) => {
                    let red = self.backend.redact(&s).await?;
                    Ok(serde_json::Value::String(red))
                }
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        out.push(self.walk(v).await?);
                    }
                    Ok(serde_json::Value::Array(out))
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, v) in map {
                        // Keys are NOT redacted ; only values.
                        out.insert(k, self.walk(v).await?);
                    }
                    Ok(serde_json::Value::Object(out))
                }
                primitive => Ok(primitive),
            }
        })
    }
}

impl std::fmt::Debug for PiiRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiiRedactor").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  RegexPiiBackend — Sprint 5 default
// ─────────────────────────────────────────────────────────────────────

/// Detects + replaces common PII categories using regex.
///
/// **NOT a substitute for Presidio NER** in production : this backend
/// only catches structured patterns (emails, phones, IDs).  Free-form
/// names and locations are NOT detected.  For HIPAA / GDPR Article 9
/// workloads, swap in `PresidioPiiBackend` (Phase 4 Sprint 12 — full
/// `efe-pii` integration).
pub struct RegexPiiBackend {
    /// Track per-pattern hit count for observability.
    counters: Mutex<HashMap<&'static str, u64>>,
}

impl RegexPiiBackend {
    pub fn new() -> Self {
        Self {
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub fn hit_count(&self, pattern: &str) -> u64 {
        let counters = self.counters.lock().unwrap();
        *counters.get(pattern).unwrap_or(&0)
    }

    pub fn total_hits(&self) -> u64 {
        let counters = self.counters.lock().unwrap();
        counters.values().sum()
    }
}

impl Default for RegexPiiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RegexPiiBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegexPiiBackend")
            .field("total_hits", &self.total_hits())
            .finish_non_exhaustive()
    }
}

// Compiled regex caches — initialized on first use.
static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
static PHONE_RE: OnceLock<Regex> = OnceLock::new();
static SSN_RE: OnceLock<Regex> = OnceLock::new();
static CREDIT_CARD_RE: OnceLock<Regex> = OnceLock::new();
static IPV4_RE: OnceLock<Regex> = OnceLock::new();
static IBAN_RE: OnceLock<Regex> = OnceLock::new();

fn email_re() -> &'static Regex {
    EMAIL_RE.get_or_init(|| {
        Regex::new(r"[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+")
            .expect("email regex valid")
    })
}

fn phone_re() -> &'static Regex {
    // Matches international + national phone formats.  Conservative :
    // requires at least 7 digits + optional spaces/dashes.
    PHONE_RE.get_or_init(|| {
        Regex::new(r"\+?\d[\d \-]{6,}\d").expect("phone regex valid")
    })
}

fn ssn_re() -> &'static Regex {
    // US SSN pattern : NNN-NN-NNNN.
    SSN_RE.get_or_init(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("ssn regex valid"))
}

fn credit_card_re() -> &'static Regex {
    // 13-19 digit card numbers with optional separators.
    CREDIT_CARD_RE.get_or_init(|| {
        Regex::new(r"\b(?:\d{4}[-\s]?){3,4}\d{1,4}\b").expect("cc regex valid")
    })
}

fn ipv4_re() -> &'static Regex {
    IPV4_RE.get_or_init(|| {
        Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("ipv4 regex valid")
    })
}

fn iban_re() -> &'static Regex {
    // IBAN : 2 letters + 2 digits + 11-30 alphanumeric chars.
    IBAN_RE.get_or_init(|| {
        Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").expect("iban regex valid")
    })
}

#[async_trait]
impl PiiBackend for RegexPiiBackend {
    async fn redact(&self, input: &str) -> Result<String, PiiError> {
        let mut out = input.to_string();
        let mut counter: u32 = 0;

        // Order matters : redact most-specific patterns first to avoid
        // overlapping matches.  Credit-card before IBAN before generic
        // phone (which has the most permissive pattern).
        let patterns: &[(&str, &Regex, &str)] = &[
            ("EMAIL", email_re(), "[EMAIL_"),
            ("CC", credit_card_re(), "[CC_"),
            ("SSN", ssn_re(), "[SSN_"),
            ("IBAN", iban_re(), "[IBAN_"),
            ("IPV4", ipv4_re(), "[IPV4_"),
            ("PHONE", phone_re(), "[PHONE_"),
        ];

        for (label, re, token_prefix) in patterns {
            let mut replaced = String::with_capacity(out.len());
            let mut last_end = 0;
            let mut hits = 0u32;
            for m in re.find_iter(&out) {
                replaced.push_str(&out[last_end..m.start()]);
                hits += 1;
                counter += 1;
                replaced.push_str(token_prefix);
                replaced.push_str(&counter.to_string());
                replaced.push(']');
                last_end = m.end();
            }
            replaced.push_str(&out[last_end..]);
            out = replaced;

            if hits > 0 {
                let mut counters = self.counters.lock().unwrap();
                *counters.entry(label).or_insert(0) += hits as u64;
            }
        }

        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  MockPiiBackend — for tests
// ─────────────────────────────────────────────────────────────────────

/// Test-only PII backend.  Records every redact call + returns either
/// the input unchanged or a configurable replacement.
pub struct MockPiiBackend {
    /// If `Some`, return this string on every call (regardless of input).
    fixed_output: Mutex<Option<String>>,
    /// Number of `redact` calls observed.
    call_count: std::sync::atomic::AtomicU32,
    /// Capture every input seen.
    captures: Mutex<Vec<String>>,
}

impl MockPiiBackend {
    pub fn passthrough() -> Self {
        Self {
            fixed_output: Mutex::new(None),
            call_count: std::sync::atomic::AtomicU32::new(0),
            captures: Mutex::new(Vec::new()),
        }
    }

    pub fn returning(fixed: impl Into<String>) -> Self {
        Self {
            fixed_output: Mutex::new(Some(fixed.into())),
            call_count: std::sync::atomic::AtomicU32::new(0),
            captures: Mutex::new(Vec::new()),
        }
    }

    pub fn set_output(&self, output: impl Into<String>) {
        *self.fixed_output.lock().unwrap() = Some(output.into());
    }

    pub fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn captures(&self) -> Vec<String> {
        self.captures.lock().unwrap().clone()
    }
}

impl Default for MockPiiBackend {
    fn default() -> Self {
        Self::passthrough()
    }
}

impl std::fmt::Debug for MockPiiBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockPiiBackend")
            .field("call_count", &self.call_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PiiBackend for MockPiiBackend {
    async fn redact(&self, input: &str) -> Result<String, PiiError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.captures.lock().unwrap().push(input.to_string());
        let fixed = self.fixed_output.lock().unwrap().clone();
        Ok(fixed.unwrap_or_else(|| input.to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Anti-leakage check (PiiStrategy::Strict)
// ─────────────────────────────────────────────────────────────────────

/// Scan an output value for any redaction tokens.  Used by
/// `PiiStrategy::Strict` to reject LLM responses that echo redacted
/// inputs back.
///
/// Tokens patterns : `[EMAIL_N]`, `[PHONE_N]`, `[SSN_N]`, `[CC_N]`,
/// `[IBAN_N]`, `[IPV4_N]`.
pub fn contains_redaction_tokens(value: &serde_json::Value) -> Option<String> {
    fn scan_string(s: &str) -> Option<String> {
        const PREFIXES: &[&str] = &[
            "[EMAIL_", "[PHONE_", "[SSN_", "[CC_", "[IBAN_", "[IPV4_",
        ];
        for prefix in PREFIXES {
            if let Some(idx) = s.find(prefix) {
                if let Some(end) = s[idx..].find(']') {
                    return Some(s[idx..idx + end + 1].to_string());
                }
            }
        }
        None
    }

    match value {
        serde_json::Value::String(s) => scan_string(s),
        serde_json::Value::Array(arr) => arr.iter().find_map(contains_redaction_tokens),
        serde_json::Value::Object(map) => map.values().find_map(contains_redaction_tokens),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── RegexPiiBackend ──────────────────────────────────────────────

    #[tokio::test]
    async fn regex_redacts_email() {
        let b = RegexPiiBackend::new();
        let out = b.redact("contact john@example.com today").await.unwrap();
        assert!(out.contains("[EMAIL_"));
        assert!(!out.contains("john@example.com"));
    }

    #[tokio::test]
    async fn regex_redacts_phone() {
        let b = RegexPiiBackend::new();
        let out = b.redact("call +1-555-1234567 now").await.unwrap();
        assert!(out.contains("[PHONE_"));
    }

    #[tokio::test]
    async fn regex_redacts_ssn() {
        let b = RegexPiiBackend::new();
        let out = b.redact("SSN: 123-45-6789").await.unwrap();
        assert!(out.contains("[SSN_"));
        assert!(!out.contains("123-45-6789"));
    }

    #[tokio::test]
    async fn regex_redacts_credit_card() {
        let b = RegexPiiBackend::new();
        let out = b.redact("Card: 4111-1111-1111-1111").await.unwrap();
        assert!(out.contains("[CC_"));
        assert!(!out.contains("4111-1111-1111-1111"));
    }

    #[tokio::test]
    async fn regex_redacts_iban() {
        let b = RegexPiiBackend::new();
        let out = b.redact("IBAN GB29NWBK60161331926819").await.unwrap();
        assert!(out.contains("[IBAN_"));
    }

    #[tokio::test]
    async fn regex_redacts_ipv4() {
        let b = RegexPiiBackend::new();
        let out = b.redact("Visited from 192.168.1.42 today").await.unwrap();
        assert!(out.contains("[IPV4_"));
    }

    #[tokio::test]
    async fn regex_redacts_multiple_emails_with_distinct_tokens() {
        let b = RegexPiiBackend::new();
        let out = b
            .redact("email a@x.com and b@y.com")
            .await
            .unwrap();
        assert!(out.contains("[EMAIL_1]"));
        assert!(out.contains("[EMAIL_2]"));
    }

    #[tokio::test]
    async fn regex_no_pii_passes_through() {
        let b = RegexPiiBackend::new();
        let out = b.redact("ordinary text with no PII").await.unwrap();
        assert_eq!(out, "ordinary text with no PII");
    }

    #[tokio::test]
    async fn regex_empty_string_returns_empty() {
        let b = RegexPiiBackend::new();
        let out = b.redact("").await.unwrap();
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn regex_hit_count_tracks_per_category() {
        let b = RegexPiiBackend::new();
        let _ = b
            .redact("a@x.com and b@y.com plus +1-555-1234567")
            .await
            .unwrap();
        assert_eq!(b.hit_count("EMAIL"), 2);
        assert_eq!(b.hit_count("PHONE"), 1);
        assert!(b.total_hits() >= 3);
    }

    // ── PiiRedactor JSON walk ────────────────────────────────────────

    #[tokio::test]
    async fn redactor_walks_object_string_field() {
        let r = PiiRedactor::with_default_regex_backend();
        let input = json!({"email": "user@host.com", "age": 30});
        let out = r.redact_json(input).await.unwrap();
        assert!(out["email"].as_str().unwrap().contains("[EMAIL_"));
        assert_eq!(out["age"], 30);    // numbers untouched
    }

    #[tokio::test]
    async fn redactor_walks_nested_object() {
        let r = PiiRedactor::with_default_regex_backend();
        let input = json!({
            "user": {
                "contact": {
                    "email": "deep@nested.com"
                }
            }
        });
        let out = r.redact_json(input).await.unwrap();
        assert!(out["user"]["contact"]["email"]
            .as_str()
            .unwrap()
            .contains("[EMAIL_"));
    }

    #[tokio::test]
    async fn redactor_walks_array() {
        let r = PiiRedactor::with_default_regex_backend();
        let input = json!(["a@x.com", "b@y.com", 42, "no pii"]);
        let out = r.redact_json(input).await.unwrap();
        let arr = out.as_array().unwrap();
        assert!(arr[0].as_str().unwrap().contains("[EMAIL_"));
        assert!(arr[1].as_str().unwrap().contains("[EMAIL_"));
        assert_eq!(arr[2], 42);
        assert_eq!(arr[3], "no pii");
    }

    #[tokio::test]
    async fn redactor_preserves_object_keys() {
        // Even if a key looks like an email, we don't redact it (structural).
        let r = PiiRedactor::with_default_regex_backend();
        let input = json!({"alice@x.com": "the value here"});
        let out = r.redact_json(input).await.unwrap();
        // Key stays intact ; value stays intact (no PII).
        assert!(out.as_object().unwrap().contains_key("alice@x.com"));
    }

    #[tokio::test]
    async fn redactor_handles_empty_object() {
        let r = PiiRedactor::with_default_regex_backend();
        let out = r.redact_json(json!({})).await.unwrap();
        assert_eq!(out, json!({}));
    }

    #[tokio::test]
    async fn redactor_handles_null_and_bool() {
        let r = PiiRedactor::with_default_regex_backend();
        let input = json!({"a": null, "b": true, "c": 1.5});
        let out = r.redact_json(input.clone()).await.unwrap();
        assert_eq!(out, input);
    }

    // ── MockPiiBackend ───────────────────────────────────────────────

    #[tokio::test]
    async fn mock_passthrough_returns_input() {
        let b = MockPiiBackend::passthrough();
        let out = b.redact("hello world").await.unwrap();
        assert_eq!(out, "hello world");
    }

    #[tokio::test]
    async fn mock_returning_replaces() {
        let b = MockPiiBackend::returning("REDACTED");
        let out = b.redact("anything").await.unwrap();
        assert_eq!(out, "REDACTED");
    }

    #[tokio::test]
    async fn mock_tracks_call_count_and_captures() {
        let b = MockPiiBackend::passthrough();
        let _ = b.redact("x").await.unwrap();
        let _ = b.redact("y").await.unwrap();
        let _ = b.redact("z").await.unwrap();
        assert_eq!(b.call_count(), 3);
        assert_eq!(b.captures(), vec!["x", "y", "z"]);
    }

    // ── contains_redaction_tokens ────────────────────────────────────

    #[test]
    fn redaction_token_detected_in_string() {
        let v = json!("data was [EMAIL_1] sent");
        let found = contains_redaction_tokens(&v).unwrap();
        assert_eq!(found, "[EMAIL_1]");
    }

    #[test]
    fn redaction_token_detected_in_array() {
        let v = json!(["clean", "leaked [PHONE_3] here"]);
        let found = contains_redaction_tokens(&v).unwrap();
        assert!(found.contains("[PHONE_"));
    }

    #[test]
    fn redaction_token_detected_in_nested_object() {
        let v = json!({
            "user": {"summary": "Wrote to [EMAIL_2]"},
        });
        let found = contains_redaction_tokens(&v).unwrap();
        assert!(found.contains("[EMAIL_"));
    }

    #[test]
    fn redaction_token_not_detected_in_clean_output() {
        let v = json!({"answer": "all good here", "count": 3});
        assert!(contains_redaction_tokens(&v).is_none());
    }

    #[test]
    fn redaction_token_handles_all_supported_categories() {
        for token in &["[EMAIL_1]", "[PHONE_2]", "[SSN_3]", "[CC_4]", "[IBAN_5]", "[IPV4_6]"] {
            let v = json!(format!("X{token}Y"));
            assert!(contains_redaction_tokens(&v).is_some(), "missed {token}");
        }
    }
}
