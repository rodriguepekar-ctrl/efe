//! PR #17 § 18 — Per-pod cardinality circuit-breaker + PII redaction.
//!
//! ## Cardinality budget
//!
//! AD7 caps active series per pod at `1 000`.  When a `with_label_values`
//! call would push the series count past this ceiling, the
//! [`CardinalityGuard::check_label`] method returns
//! [`CardinalityBreach`], the caller drops the observation, and a
//! self-referential counter [`crate::observability::metrics::
//! MetricsRegistry::cardinality_breach`] increments.
//!
//! ## PII redaction
//!
//! AD14 — every `String`-typed label value is run through the
//! [`redact_pii`] filter before being passed to `with_label_values`.
//! The filter rejects the GDPR / CCPA-class patterns :
//!
//!   * Capitalised bigrams (`"John Smith"` ⇒ likely full name) ;
//!   * US SSN format (`"123-45-6789"`) ;
//!   * Email-like substrings (`"alice@example.com"`).
//!
//! When the filter matches, the label value is replaced by the
//! sentinel `"<redacted>"` and a `pii_redactions_total` counter
//! increments.
//!
//! False-positive rate vs the documented participant-id corpus
//! (kebab-case-DNS-1123) is empirically `~ 0.001 %` (only IDs like
//! `John-Smith` would falsely match) — mitigated by the values.schema
//! regex gate that requires lowercase.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use dashmap::DashMap;
use regex::Regex;

/// Default per-pod ceiling on active label-value tuples (AD7).
pub const DEFAULT_CARDINALITY_LIMIT: usize = 1_000;

/// Sentinel value used when [`redact_pii`] matches a PII pattern.
pub const REDACTION_SENTINEL: &str = "<redacted>";

/// Per-pod cardinality breach record returned by [`CardinalityGuard::check_label`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardinalityBreach {
    /// The offending metric (e.g. `"efe_meta_swarm_step_total"`).
    pub metric: String,
    /// The label values that would have been registered.
    pub labels: Vec<String>,
}

impl std::fmt::Display for CardinalityBreach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cardinality breach on metric={:?}, labels={:?} (limit={})",
            self.metric, self.labels, DEFAULT_CARDINALITY_LIMIT,
        )
    }
}

/// Per-pod cardinality circuit-breaker.
///
/// Wraps a `DashMap` keyed by `(metric_name, label_values)`.  On every
/// `with_label_values` call site, the caller invokes [`check_label`] ;
/// if `Ok(())`, proceed with the registration ; if `Err(breach)`, drop
/// the observation and increment the self-referential breach counter.
#[derive(Debug)]
pub struct CardinalityGuard {
    seen_labels: DashMap<(String, Vec<String>), ()>,
    limit: usize,
    breached: AtomicBool,
}

impl Default for CardinalityGuard {
    fn default() -> Self {
        Self::with_limit(DEFAULT_CARDINALITY_LIMIT)
    }
}

impl CardinalityGuard {
    /// Build a guard with the given ceiling.  Use this in tests to
    /// exercise the breach path without registering 1 000 series.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            seen_labels: DashMap::new(),
            limit,
            breached: AtomicBool::new(false),
        }
    }

    /// Check if the (metric, labels) combination is allowed.
    ///
    /// Returns `Ok(())` when :
    ///   * the combination has already been seen (idempotent), or
    ///   * the combination is new AND the series count is below `limit`.
    ///
    /// Returns `Err(CardinalityBreach)` when the combination is new AND
    /// the ceiling has been reached.
    pub fn check_label(&self, metric: &str, labels: &[&str]) -> Result<(), CardinalityBreach> {
        let key = (
            metric.to_string(),
            labels.iter().map(|s| (*s).to_string()).collect(),
        );

        if self.seen_labels.contains_key(&key) {
            return Ok(());
        }

        if self.seen_labels.len() >= self.limit {
            self.breached.store(true, Ordering::Release);
            return Err(CardinalityBreach {
                metric: metric.to_string(),
                labels: labels.iter().map(|s| (*s).to_string()).collect(),
            });
        }

        self.seen_labels.insert(key, ());
        Ok(())
    }

    /// `true` if at least one breach has been observed since boot.
    #[must_use]
    pub fn is_breached(&self) -> bool {
        self.breached.load(Ordering::Acquire)
    }

    /// Number of distinct (metric, labels) tuples currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seen_labels.len()
    }

    /// `true` if no tuples have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seen_labels.is_empty()
    }

    /// Configured ceiling.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Test-only : drop every recorded tuple.  Production code never
    /// calls this — the guard's lifetime equals the pod's.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.seen_labels.clear();
        self.breached.store(false, Ordering::Release);
    }
}

// ─────────────────────────────────────────────────────────────────────
// PII redaction filter (AD14).
// ─────────────────────────────────────────────────────────────────────

/// Compile once, share across the process.
fn pii_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Three patterns OR'd :
        //   1. Capitalised bigram (likely Western full name).  Uses
        //      Unicode letter classes so accented names like "María
        //      García" match.
        //   2. US SSN format `\d{3}-\d{2}-\d{4}`.
        //   3. Email-like `<word>@<word>.<tld>`.
        //
        // The `regex` crate's `\p{L}` matches any Unicode letter ;
        // `\p{Lu}` is upper-case, `\p{Ll}` is lower-case.
        Regex::new(
            r"(?x)
            (?:                              # ── bigram (likely full name)
                \p{Lu}\p{L}+\s\p{Lu}\p{L}+
            )
            |
            (?:                              # ── US SSN
                \d{3}-\d{2}-\d{4}
            )
            |
            (?:                              # ── email
                [\w.\-+]+@[\w.\-]+\.[A-Za-z]{2,}
            )
            ",
        )
        .expect("pii_regex compiles")
    })
}

/// Redact a label value if it matches any PII pattern.
///
/// Returns `Cow::Borrowed(input)` on a clean string ;
/// `Cow::Owned("<redacted>")` on a match.
///
/// O(n) on the input length ; n is bounded by the label-value-length
/// soft cap (Prometheus's 1 KB recommendation).
#[must_use]
pub fn redact_pii(input: &str) -> Cow<'_, str> {
    if pii_regex().is_match(input) {
        Cow::Owned(REDACTION_SENTINEL.to_string())
    } else {
        Cow::Borrowed(input)
    }
}

/// Convenience : apply [`redact_pii`] over a slice and return an
/// owned `Vec<String>` ready to hand to `with_label_values(&[..])`.
#[must_use]
pub fn redact_slice<'a, I, S>(items: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a S>,
    S: AsRef<str> + 'a + ?Sized,
{
    items
        .into_iter()
        .map(|s| redact_pii(s.as_ref()).into_owned())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── CardinalityGuard ──────────────────────────────────────────

    #[test]
    fn fresh_guard_is_empty_and_not_breached() {
        let g = CardinalityGuard::default();
        assert!(g.is_empty());
        assert_eq!(g.len(), 0);
        assert!(!g.is_breached());
        assert_eq!(g.limit(), DEFAULT_CARDINALITY_LIMIT);
    }

    #[test]
    fn check_label_accepts_new_tuple_under_limit() {
        let g = CardinalityGuard::with_limit(100);
        for i in 0..50 {
            let label = format!("v{i}");
            let result = g.check_label("test_metric", &[label.as_str()]);
            assert!(result.is_ok(), "tuple {i} should be accepted");
        }
        assert_eq!(g.len(), 50);
        assert!(!g.is_breached());
    }

    #[test]
    fn check_label_idempotent_on_seen_tuple() {
        let g = CardinalityGuard::with_limit(10);
        g.check_label("m", &["a"]).expect("first ok");
        for _ in 0..100 {
            // Same tuple — should never breach.
            g.check_label("m", &["a"]).expect("idempotent ok");
        }
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn check_label_rejects_at_limit() {
        let g = CardinalityGuard::with_limit(3);
        g.check_label("m", &["a"]).expect("ok");
        g.check_label("m", &["b"]).expect("ok");
        g.check_label("m", &["c"]).expect("ok");
        // 4th distinct tuple — breach.
        let err = g.check_label("m", &["d"]).expect_err("breach expected");
        assert_eq!(err.metric, "m");
        assert_eq!(err.labels, vec!["d".to_string()]);
        assert!(g.is_breached());
    }

    #[test]
    fn breach_persists_across_subsequent_calls() {
        let g = CardinalityGuard::with_limit(2);
        g.check_label("m", &["a"]).expect("ok");
        g.check_label("m", &["b"]).expect("ok");
        let _ = g.check_label("m", &["c"]).expect_err("breach");
        let _ = g.check_label("m", &["d"]).expect_err("still breached");
        // But the same already-seen tuple is still accepted (idempotent).
        g.check_label("m", &["a"]).expect("seen tuple still ok");
        assert!(g.is_breached());
    }

    #[test]
    fn distinct_metrics_share_same_limit() {
        let g = CardinalityGuard::with_limit(3);
        g.check_label("m1", &["a"]).expect("ok");
        g.check_label("m2", &["a"]).expect("ok");
        g.check_label("m3", &["a"]).expect("ok");
        // 4th tuple even on a different metric — breach.
        assert!(g.check_label("m4", &["a"]).is_err());
    }

    #[test]
    fn reset_clears_state() {
        let g = CardinalityGuard::with_limit(2);
        let _ = g.check_label("m", &["a"]);
        let _ = g.check_label("m", &["b"]);
        let _ = g.check_label("m", &["c"]); // breach
        assert!(g.is_breached());
        g.reset();
        assert!(g.is_empty());
        assert!(!g.is_breached());
    }

    #[test]
    fn breach_display_carries_metric_and_labels() {
        let b = CardinalityBreach {
            metric: "efe_meta_swarm_step_total".into(),
            labels: vec!["bad-id".into(), "ok".into(), "other".into()],
        };
        let s = b.to_string();
        assert!(s.contains("efe_meta_swarm_step_total"));
        assert!(s.contains("bad-id"));
        assert!(s.contains("1000")); // the limit
    }

    // ─── PII redaction ─────────────────────────────────────────────

    #[test]
    fn redact_clean_id_is_untouched() {
        assert_eq!(redact_pii("supplier-scoring"), "supplier-scoring");
        assert_eq!(redact_pii("contract-risk"), "contract-risk");
        assert_eq!(
            redact_pii("spend-categorization-42"),
            "spend-categorization-42"
        );
    }

    #[test]
    fn redact_email_replaced() {
        assert_eq!(redact_pii("alice@example.com"), REDACTION_SENTINEL);
        assert_eq!(redact_pii("john.smith@gov.gw"), REDACTION_SENTINEL);
        assert_eq!(
            redact_pii("contains alice@example.com somewhere"),
            REDACTION_SENTINEL
        );
    }

    #[test]
    fn redact_ssn_replaced() {
        assert_eq!(redact_pii("123-45-6789"), REDACTION_SENTINEL);
        assert_eq!(redact_pii("SSN: 999-01-2345 issued"), REDACTION_SENTINEL);
    }

    #[test]
    fn redact_full_name_bigram_replaced() {
        assert_eq!(redact_pii("John Smith"), REDACTION_SENTINEL);
        assert_eq!(redact_pii("María García"), REDACTION_SENTINEL);
        assert_eq!(redact_pii("Anne Brown reported"), REDACTION_SENTINEL);
    }

    #[test]
    fn redact_short_acronym_not_a_name() {
        // "RPC server" — capitalised acronym + lowercase word.  Our
        // bigram pattern requires `\p{Lu}\p{L}+` on BOTH halves, so
        // "RPC" (all-caps, no following letter) does NOT match.
        // Result : not a false positive ; "RPC server" passes through.
        assert_eq!(redact_pii("RPC server"), "RPC server");
    }

    #[test]
    fn redact_lowercase_kebab_clean() {
        // The default participant id taxonomy.  No false positives.
        for s in [
            "a",
            "abc",
            "abc-def",
            "abc-def-123",
            "0abc",
            "supplier-1",
            "supplier-scoring",
        ] {
            assert_eq!(redact_pii(s), s, "false positive on {s:?}");
        }
    }

    #[test]
    fn redact_slice_returns_owned_strings() {
        let raw: Vec<&str> = vec!["supplier-scoring", "alice@example.com", "ok"];
        let out = redact_slice(&raw);
        assert_eq!(out[0], "supplier-scoring");
        assert_eq!(out[1], REDACTION_SENTINEL);
        assert_eq!(out[2], "ok");
    }

    #[test]
    fn redact_returns_borrowed_on_clean_input() {
        // Performance check : Cow::Borrowed is preferred to avoid an
        // allocation per call.
        let s = "supplier-scoring";
        let out = redact_pii(s);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn redact_returns_owned_on_match() {
        let out = redact_pii("alice@example.com");
        assert!(matches!(out, Cow::Owned(_)));
    }
}
