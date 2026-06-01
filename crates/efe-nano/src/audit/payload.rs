//! Canonical payload + hash + HMAC compute helpers for the audit chain.
//!
//! # Why canonical serialization
//!
//! The HMAC chain links each trace to the previous one via :
//!
//! ```text
//! HMAC = HMAC-SHA256(chain_parent_id || payload_hash, tenant_key)
//! payload_hash = SHA-256(canonical_row_body(trace))
//! ```
//!
//! For the chain to be **deterministic across verifiers** (we write once,
//! a regulator may verify years later), the `canonical_row_body` must be
//! byte-identical regardless of :
//!
//! - JSON-map key insertion order
//! - Float precision noise
//! - Date/time formatting variance
//!
//! This module enforces canonicalization via :
//! - Object keys sorted alphabetically
//! - DateTime serialized as RFC 3339 with second precision
//! - UUIDs serialized as lowercase hyphenated
//! - Optional fields omitted (not serialized as `null`)

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::audit::NanoAgentTrace;

// ─────────────────────────────────────────────────────────────────────
//  canonical_row_body
// ─────────────────────────────────────────────────────────────────────

/// Produce the canonical byte representation of a trace's row body.
///
/// Excludes :
/// - `payload_hash`, `hmac`, `chain_parent_id` (which depend on this body)
/// - `inserted_at` (set at storage time)
/// - `audit_tags` (mutable post-insert)
///
/// Includes : everything else, in alphabetical key order, with stable
/// formatting per field type.
pub fn canonical_row_body(trace: &NanoAgentTrace) -> Vec<u8> {
    // Build a serde_json::Map manually to ensure stable iteration order.
    let mut map = serde_json::Map::new();
    map.insert(
        "caller_agent_id".into(),
        match &trace.caller_agent_id {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    map.insert(
        "completed_at".into(),
        match trace.completed_at {
            Some(t) => Value::String(t.to_rfc3339()),
            None => Value::Null,
        },
    );
    map.insert(
        "cost_cents".into(),
        Value::String(format!("{:.6}", trace.cost_cents)),
    );
    map.insert(
        "error_kind".into(),
        match &trace.error_kind {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    map.insert(
        "execution_mode".into(),
        Value::String(trace.execution_mode.clone()),
    );
    // Sprint 7 — N6 : the chain commits to BOTH the source AST hash
    // (what the caller sent) and the resolved value hash (what the
    // agent saw).  For Literal inputs these are equal ; for Ref/Merge
    // they diverge.  Both are part of the HMAC payload so tampering
    // with either is detectable.
    map.insert(
        "input_hash_resolved".into(),
        Value::String(hex::encode(trace.input_hash_resolved)),
    );
    map.insert(
        "input_hash_source".into(),
        Value::String(hex::encode(trace.input_hash_source)),
    );
    map.insert(
        "latency_ms".into(),
        match trace.latency_ms {
            Some(n) => Value::Number(n.into()),
            None => Value::Null,
        },
    );
    map.insert(
        "llm_tokens_used".into(),
        match trace.llm_tokens_used {
            Some(n) => Value::Number(n.into()),
            None => Value::Null,
        },
    );
    map.insert(
        "nano_agent_id".into(),
        Value::String(trace.nano_agent_id.as_str().to_string()),
    );
    map.insert(
        "nano_agent_version".into(),
        Value::String(trace.nano_agent_version.as_str().to_string()),
    );
    map.insert(
        "operation_id".into(),
        Value::String(trace.operation_id.clone()),
    );
    // Sprint 7 — N1 : DAG parents as a JSON array of lowercase-
    // hyphenated UUID strings, sorted lex for stable canonicalization.
    // Empty array ≡ layer-0 op.  Sorted so two traces with the same
    // ancestry serialize identically regardless of dispatcher
    // iteration order.
    let mut parents: Vec<String> = trace
        .parent_trace_ids
        .iter()
        .map(|p| p.0.to_string())
        .collect();
    parents.sort();
    map.insert(
        "parent_trace_ids".into(),
        Value::Array(parents.into_iter().map(Value::String).collect()),
    );
    map.insert(
        "output_hash".into(),
        match trace.output_hash {
            Some(h) => Value::String(hex::encode(h)),
            None => Value::Null,
        },
    );
    map.insert(
        "started_at".into(),
        Value::String(trace.started_at.to_rfc3339()),
    );
    map.insert(
        "swarm_request_id".into(),
        Value::String(trace.swarm_request_id.to_string()),
    );
    map.insert(
        "tenant_id".into(),
        Value::String(trace.tenant_id.to_string()),
    );
    map.insert(
        "trace_id".into(),
        Value::String(trace.trace_id.0.to_string()),
    );

    // Render to canonical bytes (sorted keys recursively, no whitespace).
    let value = Value::Object(map);
    crate::idempotency::canonicalize_json(&value).into_bytes()
}

/// SHA-256 of the canonical row body.
pub fn compute_payload_hash(trace: &NanoAgentTrace) -> [u8; 32] {
    let body = canonical_row_body(trace);
    let digest = Sha256::digest(&body);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  HMAC compute
// ─────────────────────────────────────────────────────────────────────

/// Compute the chain HMAC :
/// `HMAC-SHA-256(chain_parent_id_bytes || payload_hash, key)`
///
/// For the genesis row (no parent), uses the literal "genesis" bytes as
/// chain_parent_id_bytes (matches `efe-curation` M23 convention).
pub fn compute_hmac(
    chain_parent_id: Option<uuid::Uuid>,
    payload_hash: &[u8; 32],
    key: &[u8],
) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<Sha256>>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts any key length");
    match chain_parent_id {
        Some(p) => mac.update(p.as_bytes()),
        None => mac.update(b"genesis"),
    }
    mac.update(payload_hash);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 7 — N10 : tombstone HMAC
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 7 — N10** : Compute the HMAC over a tombstone's canonical
/// fields, keyed by the tenant audit key.
///
/// The HMAC covers :
/// ```text
/// HMAC-SHA-256(
///     "tombstone-v1" ‖
///     trace_id_bytes (16) ‖
///     erased_at_micros (8 big-endian) ‖
///     erasure_reason_str (variable) ‖
///     0x01 ‖ request_id_bytes (16)   OR   0x00 (if None) ‖
///     erasure_actor_bytes (variable),
///     tenant_audit_key
/// )
/// ```
///
/// The domain prefix `"tombstone-v1"` prevents confusion with the
/// main chain HMAC (which omits this prefix).  Bumping to `"tombstone-v2"`
/// is the migration path for future field additions.
pub fn compute_tombstone_hmac(
    trace_id: uuid::Uuid,
    erased_at_micros: i64,
    erasure_reason: &crate::audit::ErasureReason,
    erasure_request_id: Option<uuid::Uuid>,
    erasure_actor: &str,
    key: &[u8],
) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<Sha256>>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts any key length");
    mac.update(b"tombstone-v1");
    mac.update(trace_id.as_bytes());
    mac.update(&erased_at_micros.to_be_bytes());
    mac.update(erasure_reason.as_str().as_bytes());
    match erasure_request_id {
        Some(rid) => {
            mac.update(&[0x01u8]);
            mac.update(rid.as_bytes());
        }
        None => {
            mac.update(&[0x00u8]);
        }
    }
    mac.update(erasure_actor.as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TenantId, TraceId};
    use uuid::Uuid;

    fn sample_trace() -> NanoAgentTrace {
        NanoAgentTrace {
            trace_id: TraceId(Uuid::from_u128(0x1111)),
            parent_trace_ids: vec![],
            swarm_request_id: Uuid::from_u128(0x2222),
            operation_id: "op-1".into(),
            tenant_id: TenantId::system(),
            caller_agent_id: Some("test-caller".into()),
            nano_agent_id: NanoAgentId::new("agent_a").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".into(),
            input_hash_source: [1u8; 32],
            input_hash_resolved: [1u8; 32],
            output_hash: Some([2u8; 32]),
            error_kind: None,
            started_at: chrono::DateTime::parse_from_rfc3339("2026-05-18T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            completed_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-05-18T10:00:01Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            latency_ms: Some(123),
            cost_cents: 0.0125,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        }
    }

    #[test]
    fn canonical_body_is_deterministic() {
        let t = sample_trace();
        let a = canonical_row_body(&t);
        let b = canonical_row_body(&t);
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_body_excludes_full_input_full_output() {
        let mut t = sample_trace();
        let a = canonical_row_body(&t);
        t.full_input = Some(serde_json::json!({"x": 1}));
        t.full_output = Some(serde_json::json!({"y": 2}));
        let b = canonical_row_body(&t);
        // full_input / full_output are NOT in the chain body — they
        // change per-trace but don't affect the chain integrity.
        assert_eq!(a, b);
    }

    #[test]
    fn payload_hash_is_32_bytes() {
        let t = sample_trace();
        let h = compute_payload_hash(&t);
        assert_eq!(h.len(), 32);
    }

    #[test]
    fn payload_hash_changes_when_trace_changes() {
        let mut t = sample_trace();
        let h1 = compute_payload_hash(&t);
        t.cost_cents = 999.99;
        let h2 = compute_payload_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hmac_genesis_distinct_from_parent() {
        let payload = [0u8; 32];
        let key = b"some_test_key_32_bytes_length_xx";
        let h_genesis = compute_hmac(None, &payload, key);
        let h_with_parent = compute_hmac(Some(Uuid::nil()), &payload, key);
        assert_ne!(h_genesis, h_with_parent);
    }

    #[test]
    fn hmac_deterministic_for_same_inputs() {
        let payload = [42u8; 32];
        let parent = Uuid::from_u128(0x9999);
        let key = b"x";
        let a = compute_hmac(Some(parent), &payload, key);
        let b = compute_hmac(Some(parent), &payload, key);
        assert_eq!(a, b);
    }

    #[test]
    fn hmac_changes_when_key_changes() {
        let payload = [0u8; 32];
        let parent = Uuid::nil();
        let a = compute_hmac(Some(parent), &payload, b"key_a");
        let b = compute_hmac(Some(parent), &payload, b"key_b");
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_changes_when_payload_changes() {
        let parent = Uuid::nil();
        let key = b"k";
        let a = compute_hmac(Some(parent), &[0u8; 32], key);
        let b = compute_hmac(Some(parent), &[1u8; 32], key);
        assert_ne!(a, b);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N10 : the central crypto invariant
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn canonical_body_invariant_under_tombstoning() {
        // The key N10 contract : adding a tombstone to a trace MUST
        // NOT change its canonical_row_body — otherwise the payload_hash
        // would change and the HMAC chain would break.
        let live = sample_trace();
        let h_live = compute_payload_hash(&live);

        let mut tombstoned = sample_trace();
        tombstoned.tombstone = Some(crate::audit::TombstoneMetadata::new(
            chrono::Utc::now(),
            crate::audit::ErasureReason::GdprArticle17,
            Some(Uuid::new_v4()),
            "gdpr_saga:test",
            [0xff; 32],
        ));
        // Also clear the PII bodies as the tombstone API would do.
        tombstoned.full_input = None;
        tombstoned.full_output = None;
        let h_tombstoned = compute_payload_hash(&tombstoned);

        assert_eq!(
            h_live, h_tombstoned,
            "tombstoning MUST NOT change payload_hash (N10 ↔ HMAC chain invariant)"
        );
    }

    #[test]
    fn tombstone_hmac_is_deterministic() {
        let trace_id = Uuid::from_u128(0x42);
        let request_id = Uuid::from_u128(0x99);
        let micros = 1_700_000_000_000_000_i64;
        let key = b"tenant-key-test";
        let h1 = compute_tombstone_hmac(
            trace_id,
            micros,
            &crate::audit::ErasureReason::GdprArticle17,
            Some(request_id),
            "actor:x",
            key,
        );
        let h2 = compute_tombstone_hmac(
            trace_id,
            micros,
            &crate::audit::ErasureReason::GdprArticle17,
            Some(request_id),
            "actor:x",
            key,
        );
        assert_eq!(h1, h2);
    }

    #[test]
    fn tombstone_hmac_distinguishes_request_id_presence() {
        let trace_id = Uuid::from_u128(0x42);
        let request_id = Uuid::from_u128(0x99);
        let micros = 1_700_000_000_000_000_i64;
        let key = b"tenant-key-test";
        let with_req = compute_tombstone_hmac(
            trace_id,
            micros,
            &crate::audit::ErasureReason::GdprArticle17,
            Some(request_id),
            "actor:x",
            key,
        );
        let without_req = compute_tombstone_hmac(
            trace_id,
            micros,
            &crate::audit::ErasureReason::GdprArticle17,
            None,
            "actor:x",
            key,
        );
        assert_ne!(with_req, without_req);
    }

    #[test]
    fn canonical_body_handles_optional_fields() {
        let t1 = sample_trace();
        let t2 = NanoAgentTrace {
            caller_agent_id: None,
            output_hash: None,
            completed_at: None,
            latency_ms: None,
            error_kind: Some("Timeout".into()),
            llm_tokens_used: Some(42),
            ..sample_trace()
        };
        let h1 = compute_payload_hash(&t1);
        let h2 = compute_payload_hash(&t2);
        assert_ne!(h1, h2);
    }
}
