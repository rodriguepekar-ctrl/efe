//! **Sprint 8 — N14** : Swarm envelope payload type.
//!
//! See `docs/architecture/EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md`.
//!
//! # The envelope's role
//!
//! Every successfully-dispatched `SwarmRequest` produces ONE envelope
//! trace at the end.  The envelope is the audit-chain ANCHOR for the
//! swarm : it captures the request signature (the configuration fields
//! that wouldn't otherwise be on the chain) AND the aggregated outcome
//! (`overall_status`, `total_cost_cents`, `total_latency_ms`,
//! `op_status_counts`, plus cache / shadow / canary sub-event counts).
//!
//! # On-disk shape
//!
//! The envelope payload is stored as the envelope trace's `full_input`
//! AND `full_output` (the request portion in `full_input`, the outcome
//! portion in `full_output`).  At read time, callers reconstruct the
//! full `SwarmEnvelopePayload` from those two halves.
//!
//! # Forward compatibility
//!
//! The `envelope_format_version: u16` field lets Sprint 9+ evolve the
//! schema while preserving back-deserialisation of v1 envelopes.
//! `EnvelopeKind` is a discriminator that enables Sprint 9 streaming
//! updates without breaking the terminal-only Sprint 8 contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::traits::TraceId;
use crate::types::{SwarmStatus, TenantId};

// ─────────────────────────────────────────────────────────────────────
//  SwarmEnvelopePayload
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14** : Persisted body of the swarm envelope trace.
///
/// Round-trippable via serde.  Stored split across the envelope
/// trace's `full_input` (request signature) and `full_output`
/// (aggregated outcome) but logically one struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmEnvelopePayload {
    // ─── request signature ───
    /// Swarm request id (UUID v4).
    pub swarm_request_id: Uuid,

    /// Tenant scope.
    pub tenant_id: TenantId,

    /// Optional caller agent (e.g. "claude-sonnet-4-5").  Telemetry +
    /// lineage.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caller_agent_id: Option<String>,

    /// Customer-supplied dedup key (echoed back for forensics).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<String>,

    /// Customer-set audit tags.
    #[serde(default)]
    pub audit_tags: Vec<String>,

    /// Maximum concurrent ops the dispatcher was allowed to run.
    pub max_parallelism: u32,

    /// Wall-clock budget the swarm was given (ms).
    pub timeout_ms: u32,

    /// N7 swarm-wide cost cap, if set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cost_cap_cents: Option<f64>,

    /// Streaming flag (Sprint 9 wires actual streaming).
    #[serde(default)]
    pub streaming: bool,

    /// Number of ops in the original request.
    pub op_count: u32,

    /// Ordered op_ids — enables drill-down dashboards without scanning
    /// the chain.
    pub op_ids: Vec<String>,

    /// SHA-256 of the canonical-serialised SwarmRequest.  Tamper-
    /// evidence on the request bytes themselves.
    #[serde(with = "hex_array_32")]
    pub request_hash: [u8; 32],

    /// Wall-clock anchor at swarm start.
    pub started_at: DateTime<Utc>,

    // ─── aggregated outcome ───
    /// The dispatcher's single recorded truth about the swarm's overall
    /// status.  Closes "Gap 3" from the architecture doc.
    pub overall_status: SwarmStatus,

    /// Sum of all op costs (Sprint 8 includes cache-hit costs via N12).
    pub total_cost_cents: f64,

    /// Wall-clock from dispatch start to envelope write.
    pub total_latency_ms: u32,

    /// Wall-clock anchor at envelope write.
    pub completed_at: DateTime<Utc>,

    /// Per-status op count breakdown.
    pub op_status_counts: OpStatusCounts,

    /// Number of ops served from the N12 idempotency cache.
    pub cache_hit_count: u32,

    /// Number of shadow runs scheduled during this swarm (N13).
    pub shadow_run_count: u32,

    /// Number of shadow comparisons that returned Mismatch (N13).
    pub shadow_mismatch_count: u32,

    /// Number of ops dispatched to the canary version of the active
    /// rollout policy (N13).
    pub canary_routed_count: u32,

    // ─── lineage ───
    /// Optional parent envelope (set when this swarm is a child of a
    /// meta-swarm).  Sprint 8 : caller-set, no automatic wiring.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_envelope_trace_id: Option<TraceId>,

    // ─── versioning ───
    /// Schema version of the envelope payload.  Sprint 8 = 1.
    #[serde(default = "default_envelope_format_version")]
    pub envelope_format_version: u16,

    /// **Sprint 9 forward-compat** : kind of envelope.  Sprint 8
    /// always emits `Terminal`.  Sprint 9 may emit `Progress` /
    /// `Final` for streaming.
    #[serde(default)]
    pub envelope_kind: EnvelopeKind,
}

fn default_envelope_format_version() -> u16 {
    1
}

impl SwarmEnvelopePayload {
    /// True iff `overall_status` reflects a successful outcome (all
    /// ops succeeded — no failures, no skips, no cancels).
    pub fn is_clean_success(&self) -> bool {
        matches!(self.overall_status, SwarmStatus::Success)
            && self.op_status_counts.failed == 0
            && self.op_status_counts.skipped == 0
            && self.op_status_counts.cancelled == 0
    }

    /// True iff this swarm involved any N13 audit-metadata activity
    /// (shadow or canary).
    pub fn had_rollout_activity(&self) -> bool {
        self.shadow_run_count > 0 || self.canary_routed_count > 0
    }
}

// ─────────────────────────────────────────────────────────────────────
//  OpStatusCounts
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14** : Counts of op outcomes by terminal status.
///
/// `success + failed + skipped + cancelled` MUST equal the swarm's
/// `op_count` (modulo a swarm that timed out with in-flight ops, where
/// some ops may be `Running` at envelope-write time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpStatusCounts {
    pub success: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cancelled: u32,
}

impl OpStatusCounts {
    pub fn total(&self) -> u32 {
        self.success + self.failed + self.skipped + self.cancelled
    }

    pub fn has_any_failure(&self) -> bool {
        self.failed > 0 || self.cancelled > 0
    }
}

// ─────────────────────────────────────────────────────────────────────
//  EnvelopeKind
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14** : Discriminator for envelope-emission kind.
/// Sprint 8 only emits `Terminal`.  Sprint 9 may add streaming variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// Single envelope, written at swarm completion.
    Terminal,

    /// Sprint 9 streaming intermediate update.
    Progress { update_seq: u32 },

    /// Sprint 9 streaming closing envelope (after `Progress` updates).
    Final { total_updates: u32 },
}

impl Default for EnvelopeKind {
    fn default() -> Self {
        Self::Terminal
    }
}

impl EnvelopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Progress { .. } => "progress",
            Self::Final { .. } => "final",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  hex_array_32 — serde helper for [u8; 32]
// ─────────────────────────────────────────────────────────────────────

mod hex_array_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        let mut hex = String::with_capacity(64);
        for byte in b {
            use std::fmt::Write;
            write!(hex, "{byte:02x}").expect("write to String never fails");
        }
        s.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "expected 64-char hex string, got {}",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|e| serde::de::Error::custom(format!("invalid hex at {i}: {e}")))?;
        }
        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    fn sample_payload() -> SwarmEnvelopePayload {
        SwarmEnvelopePayload {
            swarm_request_id: Uuid::new_v4(),
            tenant_id: TenantId::system(),
            caller_agent_id: Some("claude-sonnet-4-5".into()),
            idempotency_key: Some("user-2026-05-20".into()),
            audit_tags: vec!["billing".into(), "tier-2".into()],
            max_parallelism: 4,
            timeout_ms: 30_000,
            cost_cap_cents: Some(1.0),
            streaming: false,
            op_count: 3,
            op_ids: vec!["a".into(), "b".into(), "c".into()],
            request_hash: [0x42u8; 32],
            started_at: Utc::now(),
            overall_status: SwarmStatus::Success,
            total_cost_cents: 0.0156,
            total_latency_ms: 2333,
            completed_at: Utc::now(),
            op_status_counts: OpStatusCounts {
                success: 3,
                failed: 0,
                skipped: 0,
                cancelled: 0,
            },
            cache_hit_count: 1,
            shadow_run_count: 0,
            shadow_mismatch_count: 0,
            canary_routed_count: 0,
            parent_envelope_trace_id: None,
            envelope_format_version: 1,
            envelope_kind: EnvelopeKind::Terminal,
        }
    }

    #[test]
    fn payload_round_trip() {
        let p = sample_payload();
        let s = serde_json::to_string(&p).unwrap();
        let r: SwarmEnvelopePayload = serde_json::from_str(&s).unwrap();
        assert_eq!(p, r);
    }

    #[test]
    fn payload_default_kind_is_terminal() {
        let json = serde_json::json!({
            "swarm_request_id": "00000000-0000-0000-0000-000000000001",
            "tenant_id": "00000000-0000-0000-0000-000000000000",
            "max_parallelism": 4,
            "timeout_ms": 30000,
            "op_count": 0,
            "op_ids": [],
            "request_hash": "00".repeat(32).chars().take(64).collect::<String>(),
            "started_at": "2026-05-20T00:00:00Z",
            "overall_status": "success",
            "total_cost_cents": 0.0,
            "total_latency_ms": 0,
            "completed_at": "2026-05-20T00:00:01Z",
            "op_status_counts": {"success": 0, "failed": 0, "skipped": 0, "cancelled": 0},
            "cache_hit_count": 0,
            "shadow_run_count": 0,
            "shadow_mismatch_count": 0,
            "canary_routed_count": 0
        });
        let p: SwarmEnvelopePayload = serde_json::from_value(json).unwrap();
        assert_eq!(p.envelope_kind, EnvelopeKind::Terminal);
        assert_eq!(p.envelope_format_version, 1);
    }

    #[test]
    fn op_status_counts_total() {
        let c = OpStatusCounts { success: 3, failed: 1, skipped: 2, cancelled: 1 };
        assert_eq!(c.total(), 7);
    }

    #[test]
    fn op_status_counts_has_any_failure_predicate() {
        let clean = OpStatusCounts { success: 5, ..Default::default() };
        assert!(!clean.has_any_failure());
        let failed = OpStatusCounts { failed: 1, ..Default::default() };
        assert!(failed.has_any_failure());
        let cancelled = OpStatusCounts { cancelled: 1, ..Default::default() };
        assert!(cancelled.has_any_failure());
        // Skipped does NOT count as a failure (skips are dependency
        // propagation, not actual failures).
        let skipped = OpStatusCounts { skipped: 1, ..Default::default() };
        assert!(!skipped.has_any_failure());
    }

    #[test]
    fn envelope_kind_progress_serde() {
        let k = EnvelopeKind::Progress { update_seq: 7 };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"kind\":\"progress\""));
        assert!(s.contains("\"update_seq\":7"));
        let r: EnvelopeKind = serde_json::from_str(&s).unwrap();
        assert_eq!(k, r);
    }

    #[test]
    fn envelope_kind_final_serde() {
        let k = EnvelopeKind::Final { total_updates: 12 };
        let s = serde_json::to_string(&k).unwrap();
        let r: EnvelopeKind = serde_json::from_str(&s).unwrap();
        assert_eq!(k, r);
    }

    #[test]
    fn envelope_kind_as_str() {
        assert_eq!(EnvelopeKind::Terminal.as_str(), "terminal");
        assert_eq!(EnvelopeKind::Progress { update_seq: 0 }.as_str(), "progress");
        assert_eq!(EnvelopeKind::Final { total_updates: 0 }.as_str(), "final");
    }

    #[test]
    fn request_hash_hex_round_trip() {
        let p = SwarmEnvelopePayload {
            request_hash: [
                0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33,
                0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
                0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04,
                0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
            ],
            ..sample_payload()
        };
        let s = serde_json::to_string(&p).unwrap();
        // The serialized hex must contain the known prefix.
        assert!(s.contains("deadbeef"));
        let r: SwarmEnvelopePayload = serde_json::from_str(&s).unwrap();
        assert_eq!(p.request_hash, r.request_hash);
    }

    #[test]
    fn request_hash_invalid_length_rejected() {
        let mut v = serde_json::to_value(sample_payload()).unwrap();
        v["request_hash"] = serde_json::json!("deadbeef"); // too short
        let r: Result<SwarmEnvelopePayload, _> = serde_json::from_value(v);
        assert!(r.is_err());
    }

    #[test]
    fn is_clean_success_strict() {
        let mut p = sample_payload();
        assert!(p.is_clean_success());
        p.op_status_counts.failed = 1;
        assert!(!p.is_clean_success());
    }

    #[test]
    fn had_rollout_activity_predicate() {
        let mut p = sample_payload();
        assert!(!p.had_rollout_activity());
        p.shadow_run_count = 5;
        assert!(p.had_rollout_activity());
        p.shadow_run_count = 0;
        p.canary_routed_count = 3;
        assert!(p.had_rollout_activity());
    }

    #[test]
    fn audit_tags_default_to_empty_on_deser() {
        let json = r#"{
            "swarm_request_id": "00000000-0000-0000-0000-000000000001",
            "tenant_id": "00000000-0000-0000-0000-000000000000",
            "max_parallelism": 4,
            "timeout_ms": 30000,
            "op_count": 0,
            "op_ids": [],
            "request_hash": "0000000000000000000000000000000000000000000000000000000000000000",
            "started_at": "2026-05-20T00:00:00Z",
            "overall_status": "success",
            "total_cost_cents": 0.0,
            "total_latency_ms": 0,
            "completed_at": "2026-05-20T00:00:01Z",
            "op_status_counts": {"success": 0, "failed": 0, "skipped": 0, "cancelled": 0},
            "cache_hit_count": 0,
            "shadow_run_count": 0,
            "shadow_mismatch_count": 0,
            "canary_routed_count": 0
        }"#;
        let p: SwarmEnvelopePayload = serde_json::from_str(json).unwrap();
        assert!(p.audit_tags.is_empty());
    }
}
