//! Sprint **D13-bis** — span-attribute forbidden-key filter.
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md` § 9.
//!
//! D13 documented an attribute whitelist + forbidden prefixes
//! (`spans::FORBIDDEN_ATTR_PREFIXES`) but didn't actually run a
//! filter at OTel-export time. Defense in depth : even if a
//! handler dev forgets and writes
//! `tracing::info_span!("...", email = %user.email)`, the OTel
//! pipeline drops the field before it leaves the process. PII
//! never reaches Tempo.
//!
//! # Where this is invoked
//!
//! Three strategic places, all defense-in-depth :
//!
//!   1. **Resource construction** (`resource::build_resource`) —
//!      we don't pull from arbitrary env vars, so this is
//!      defensive only.
//!   2. **MeasuringSpanExporter** (`measuring_exporter.rs`) — the
//!      exporter wrapper calls [`filter_span_batch`] on every
//!      `SpanData` batch BEFORE handing it to the OTLP exporter.
//!      This is the **authoritative gate** : forbidden attributes
//!      never leave the process. Audit fix § A15.
//!   3. **Direct callers** — handlers that build attribute lists
//!      programmatically may call [`filter_attributes`] for
//!      explicit filtering at construction time.
//!
//! # Performance — audit fix A16
//!
//! [`is_forbidden_attribute`] is on the hot path : every span
//! attribute, every event attribute, every link attribute, on
//! every export batch. The original implementation called
//! `key.to_ascii_lowercase()` which allocates a `String` per
//! check. The current implementation does a byte-level
//! case-insensitive substring scan — **zero allocations** on the
//! hot path.
//!
//! # Pin test
//!
//! `scripts/tests/test_d13bis_otel_sdk.py` checks that this
//! module references `FORBIDDEN_ATTR_PREFIXES` and that the
//! `MeasuringSpanExporter` invokes `filter_span_batch` so a
//! future refactor that removes the link surfaces in CI.

use opentelemetry::KeyValue;
use opentelemetry_sdk::export::trace::SpanData;

use crate::spans::FORBIDDEN_ATTR_PREFIXES;

/// Return `true` iff this attribute key is forbidden by the
/// architecture-doc whitelist (`spans.rs::FORBIDDEN_ATTR_PREFIXES`).
///
/// The check is **substring** (not prefix-only) because the
/// underlying constant is named `*_PREFIXES` for historical
/// reasons but architecturally we want to catch any occurrence.
/// A future tightening (D13-ter) can change to startswith only.
///
/// # Audit fix A16
///
/// Uses byte-level case-insensitive substring scan to avoid
/// allocating a `String` per call. Hot-path safe.
#[must_use]
pub fn is_forbidden_attribute(key: &str) -> bool {
    let key_bytes = key.as_bytes();
    FORBIDDEN_ATTR_PREFIXES
        .iter()
        .any(|forbidden| contains_ignore_ascii_case(key_bytes, forbidden.as_bytes()))
}

/// Allocation-free case-insensitive substring search. Returns
/// `true` iff `needle` appears anywhere inside `haystack`,
/// ignoring ASCII case differences.
///
/// Implementation : naive sliding window — fine for small
/// `FORBIDDEN_ATTR_PREFIXES` (< 30 entries, all < 32 bytes).
/// A `memchr`-based search would be faster but the current
/// inputs don't warrant the dep.
#[inline]
fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// Filter a slice of attributes, returning only the allowed ones.
/// Forbidden attributes produce a single `tracing::warn!` line per
/// call (not per attribute) so the warning rate stays bounded.
#[must_use]
pub fn filter_attributes(attrs: Vec<KeyValue>) -> Vec<KeyValue> {
    let mut out = Vec::with_capacity(attrs.len());
    let mut dropped: Vec<String> = Vec::new();
    for kv in attrs {
        if is_forbidden_attribute(kv.key.as_str()) {
            dropped.push(kv.key.as_str().to_string());
            continue;
        }
        out.push(kv);
    }
    if !dropped.is_empty() {
        crate::metrics::d13bis_instruments::otel_attributes_filtered_total()
            .inc_by(dropped.len() as u64);
        tracing::warn!(
            target: "efe.otel.filter",
            dropped_keys = ?dropped,
            "OTel attribute filter dropped forbidden keys (PII/secret prevention)"
        );
    }
    out
}

/// **Audit fix § A15** — walk a span-export batch and strip every
/// forbidden attribute, on the span itself, on its events, and on
/// its links.
///
/// This closes the gap that pre-audit D13-bis had : the filter
/// existed but was never invoked, so PII still reached Tempo.
///
/// Mutates the batch in place rather than allocating a new
/// `Vec<SpanData>` ; allocation only happens when filtering
/// produces a change to one of the inner attribute vectors.
///
/// Returns the total count of attributes stripped across the
/// whole batch (used by [`MeasuringSpanExporter`] to bump the
/// `efe_otel_attributes_filtered_total` counter).
pub fn filter_span_batch(batch: &mut [SpanData]) -> u64 {
    let mut total_stripped: u64 = 0;
    for span in batch.iter_mut() {
        // 1. Span-level attributes.
        let before = span.attributes.len();
        span.attributes.retain(|kv| !is_forbidden_attribute(kv.key.as_str()));
        let stripped = (before - span.attributes.len()) as u64;
        if stripped > 0 {
            span.dropped_attributes_count =
                span.dropped_attributes_count.saturating_add(stripped as u32);
            total_stripped += stripped;
        }

        // 2. Event-level attributes.
        for event in span.events.events.iter_mut() {
            let before = event.attributes.len();
            event
                .attributes
                .retain(|kv| !is_forbidden_attribute(kv.key.as_str()));
            let stripped = (before - event.attributes.len()) as u64;
            if stripped > 0 {
                event.dropped_attributes_count = event
                    .dropped_attributes_count
                    .saturating_add(stripped as u32);
                total_stripped += stripped;
            }
        }

        // 3. Link-level attributes.
        for link in span.links.links.iter_mut() {
            let before = link.attributes.len();
            link.attributes
                .retain(|kv| !is_forbidden_attribute(kv.key.as_str()));
            let stripped = (before - link.attributes.len()) as u64;
            if stripped > 0 {
                link.dropped_attributes_count = link
                    .dropped_attributes_count
                    .saturating_add(stripped as u32);
                total_stripped += stripped;
            }
        }
    }
    if total_stripped > 0 {
        crate::metrics::d13bis_instruments::otel_attributes_filtered_total()
            .inc_by(total_stripped);
    }
    total_stripped
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_canonical_attrs() {
        for k in ["tenant", "catalog_key", "n_classes", "round_idx"] {
            assert!(!is_forbidden_attribute(k), "{k:?} should be allowed");
        }
    }

    #[test]
    fn forbids_pii_keys() {
        for k in [
            "email",
            "user_email",
            "customer_email",
            "phone",
            "address",
            "raw_input",
            "input",
            "text",
        ] {
            assert!(is_forbidden_attribute(k), "{k:?} should be forbidden");
        }
    }

    #[test]
    fn forbids_secret_keys() {
        for k in ["jwt", "token", "api_key", "password", "secret"] {
            assert!(is_forbidden_attribute(k), "{k:?} should be forbidden");
        }
    }

    #[test]
    fn forbids_ids_with_unbounded_cardinality() {
        for k in ["request_id", "trace_id", "span_id"] {
            assert!(is_forbidden_attribute(k), "{k:?} should be forbidden");
        }
    }

    #[test]
    fn case_insensitive_match() {
        assert!(is_forbidden_attribute("EMAIL"));
        assert!(is_forbidden_attribute("Email"));
        assert!(is_forbidden_attribute("eMaIl"));
    }

    #[test]
    fn substring_match_rejects_prefixed_keys() {
        // Operator might write `"user_email_hash"` thinking it's safe.
        // We refuse — the keyword `email` is a tripwire.
        assert!(is_forbidden_attribute("user_email_hash"));
        assert!(is_forbidden_attribute("hashed_password"));
    }

    #[test]
    fn filter_preserves_allowed_attrs() {
        let attrs = vec![
            KeyValue::new("tenant", "acme"),
            KeyValue::new("n_classes", 5_i64),
        ];
        let kept = filter_attributes(attrs);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn filter_drops_forbidden_attrs() {
        let attrs = vec![
            KeyValue::new("tenant", "acme"),
            KeyValue::new("email", "alice@example.com"),
            KeyValue::new("password", "hunter2"),
            KeyValue::new("n_classes", 5_i64),
        ];
        let kept = filter_attributes(attrs);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|kv| kv.key.as_str() == "tenant"));
        assert!(kept.iter().any(|kv| kv.key.as_str() == "n_classes"));
        assert!(!kept.iter().any(|kv| kv.key.as_str() == "email"));
        assert!(!kept.iter().any(|kv| kv.key.as_str() == "password"));
    }

    #[test]
    fn filter_handles_empty_input() {
        let kept = filter_attributes(vec![]);
        assert!(kept.is_empty());
    }

    // =================================================================
    // Audit-fix A16 regression : zero-allocation case-insensitive scan
    // =================================================================

    #[test]
    fn contains_ignore_ascii_case_basic() {
        assert!(contains_ignore_ascii_case(b"USER_EMAIL", b"email"));
        assert!(contains_ignore_ascii_case(b"PassWord123", b"password"));
        assert!(!contains_ignore_ascii_case(b"tenant", b"email"));
    }

    #[test]
    fn contains_ignore_ascii_case_edge_cases() {
        // Empty needle matches anything.
        assert!(contains_ignore_ascii_case(b"foo", b""));
        // Empty haystack, non-empty needle.
        assert!(!contains_ignore_ascii_case(b"", b"x"));
        // Needle longer than haystack.
        assert!(!contains_ignore_ascii_case(b"hi", b"hello"));
        // Exact match.
        assert!(contains_ignore_ascii_case(b"email", b"EMAIL"));
    }

    // =================================================================
    // Audit-fix A15 regression : filter_span_batch wires PII protection
    // =================================================================

    use opentelemetry::trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState};
    use opentelemetry::InstrumentationScope;
    use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};
    use std::borrow::Cow;
    use std::time::SystemTime;

    fn make_span_data(attrs: Vec<KeyValue>) -> SpanData {
        SpanData {
            span_context: SpanContext::new(
                TraceId::from_bytes([1u8; 16]),
                SpanId::from_bytes([1u8; 8]),
                TraceFlags::default(),
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            span_kind: SpanKind::Server,
            name: Cow::Borrowed("test"),
            start_time: SystemTime::now(),
            end_time: SystemTime::now(),
            attributes: attrs,
            dropped_attributes_count: 0,
            events: SpanEvents::default(),
            links: SpanLinks::default(),
            status: Status::Unset,
            instrumentation_scope: InstrumentationScope::builder("efe").build(),
        }
    }

    #[test]
    fn filter_span_batch_strips_pii_from_span_attrs() {
        let mut batch = vec![make_span_data(vec![
            KeyValue::new("tenant", "acme"),
            KeyValue::new("email", "alice@example.com"),
            KeyValue::new("password", "hunter2"),
            KeyValue::new("n_classes", 5_i64),
        ])];
        let stripped = filter_span_batch(&mut batch);
        assert_eq!(stripped, 2, "should strip 2 forbidden keys");
        assert_eq!(batch[0].attributes.len(), 2);
        assert!(batch[0]
            .attributes
            .iter()
            .all(|kv| kv.key.as_str() != "email" && kv.key.as_str() != "password"));
        // dropped_attributes_count must be bumped so OTel collectors
        // can report on filtering.
        assert_eq!(batch[0].dropped_attributes_count, 2);
    }

    #[test]
    fn filter_span_batch_empty_is_zero() {
        let mut batch: Vec<SpanData> = vec![];
        assert_eq!(filter_span_batch(&mut batch), 0);
    }

    #[test]
    fn filter_span_batch_no_pii_is_zero() {
        let mut batch = vec![make_span_data(vec![
            KeyValue::new("tenant", "acme"),
            KeyValue::new("n_classes", 5_i64),
        ])];
        assert_eq!(filter_span_batch(&mut batch), 0);
        assert_eq!(batch[0].dropped_attributes_count, 0);
    }

    #[test]
    fn filter_span_batch_multiple_spans() {
        let mut batch = vec![
            make_span_data(vec![KeyValue::new("email", "a")]),
            make_span_data(vec![KeyValue::new("tenant", "acme")]),
            make_span_data(vec![KeyValue::new("jwt", "x.y.z")]),
        ];
        let stripped = filter_span_batch(&mut batch);
        assert_eq!(stripped, 2);
        assert_eq!(batch[0].attributes.len(), 0);
        assert_eq!(batch[1].attributes.len(), 1);
        assert_eq!(batch[2].attributes.len(), 0);
    }
}
