//! Audit trace writer.
//!
//! Phase 1 ships [`stub::InMemoryAuditWriter`] + [`stub::ConsoleAuditWriter`].
//! Phase 2 Sprint 6 adds the **HMAC chain** layer :
//! - [`hmac_chain::HmacChainAuditWriter`] : main impl.
//! - [`hmac_chain::InMemoryHmacChainAuditWriter`] : convenience wrapper
//!   that also stores full input/output bodies for replay.
//! - [`chain_store::ChainStore`] : storage backend trait.
//! - [`chain_store::InMemoryChainStore`] : Sprint 6 default impl.
//! - [`verify::verify_chain`] : integrity verifier.
//! - [`hkdf::HmacKeySource`] : per-tenant key derivation.

pub mod chain_store;
/// **Sprint 8 — N13** : sliding drift window + promotion gate evaluator.
pub mod drift;
pub mod hkdf;
pub mod hmac_chain;
/// **Sprint 7 — N9** : Merkle epoch layer on top of the linear HMAC
/// chain.  Gives O(log N) inclusion proofs and O(N / epoch_size)
/// chain verification — vs the pre-Sprint-7 O(N) HMAC walk.
pub mod merkle;
pub mod payload;
pub mod stub;
pub mod verify;

pub use chain_store::{ChainEntry, ChainStore, InMemoryChainStore};
pub use drift::{DriftWindow, PromotionGateEvaluator};
pub use hkdf::HmacKeySource;
pub use hmac_chain::{HmacChainAuditWriter, InMemoryHmacChainAuditWriter};
pub use merkle::{
    build_root, build_tree, commitment_hash, leaf_hash, node_hash, verify_inclusion,
    verify_trace_in_epoch, EpochRoot, EpochRootStore, EpochSealer, EpochSealerConfig,
    InMemoryEpochRootStore, InMemoryNotary, InclusionProof, MerkleError, MerkleTree, Notary,
    NotaryError, NotaryReceipt, SealError,
};
pub use payload::{canonical_row_body, compute_hmac, compute_payload_hash};
pub use verify::{verify_chain, verify_subchain, ChainIntegrityReport};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TenantId, TraceId};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// One nano agent call trace.
///
/// Phase 1 records this to a stub writer.  Sprint 6 chains these into
/// the per-tenant HMAC chain via [`HmacChainAuditWriter`].
///
/// **Sprint 7 — N1 / N6** changes :
/// - `parent_trace_id: Option<TraceId>` → `parent_trace_ids: Vec<TraceId>`
///   so a `Merge` op with multiple upstream refs correctly records all
///   ancestors in the audit DAG (was always-`None` pre-Sprint-7).
/// - `input_hash: [u8;32]` → `input_hash_source` + `input_hash_resolved`
///   so forensics can recover BOTH "what did the caller send literally"
///   (source) AND "what value did the runtime see after `Ref`/`Merge`
///   resolution" (resolved).
///
/// **Sprint 7 — N10** addition :
/// - `tombstone: Option<TombstoneMetadata>` so an erased trace can be
///   marked GDPR-tombstoned without destroying the HMAC chain integrity.
///   See [`TombstoneMetadata`] for the contract.  The `tombstone` field
///   is **NOT** included in [`payload::canonical_row_body`] : adding a
///   tombstone preserves the row's `payload_hash` and therefore the
///   chain HMAC.  Auditors see the tombstone as operational metadata
///   alongside the cryptographically-frozen row.
#[derive(Debug, Clone)]
pub struct NanoAgentTrace {
    pub trace_id: TraceId,

    /// **Sprint 7 — N1** : DAG parents (one per `Ref` / `Merge` source).
    /// Empty `Vec` ≡ layer-0 op (no upstream dependency).
    pub parent_trace_ids: Vec<TraceId>,

    pub swarm_request_id: Uuid,
    pub operation_id: String,
    pub tenant_id: TenantId,
    pub caller_agent_id: Option<String>,
    pub nano_agent_id: NanoAgentId,
    pub nano_agent_version: NanoAgentVersion,
    pub execution_mode: String,

    /// **Sprint 7 — N6** : SHA-256 of the canonical-serialized
    /// `OperationInput` AST (Literal / Ref / Merge tree, sources
    /// unsubstituted).  Captures "what did the caller send".
    pub input_hash_source: [u8; 32],

    /// **Sprint 7 — N6** (previously named `input_hash`) : SHA-256 of
    /// the resolved JSON value the runtime actually saw (Ref / Merge
    /// substituted).  Captures "what value did the agent receive".
    /// For `Literal` inputs, `input_hash_source == input_hash_resolved`.
    pub input_hash_resolved: [u8; 32],

    pub output_hash: Option<[u8; 32]>,
    pub error_kind: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u32>,
    pub cost_cents: f64,
    pub llm_tokens_used: Option<u32>,

    // ─── Sprint 6 : optional full input/output for replay ───
    /// Full input value, if `agent.audit.include_input` is true.
    /// NOT included in the canonical row body (so it can be stored
    /// separately or excluded from the chain).
    pub full_input: Option<serde_json::Value>,
    /// Full output value, if `agent.audit.include_output` is true.
    pub full_output: Option<serde_json::Value>,

    /// **Sprint 7 — N10** : GDPR / retention tombstone marker.
    /// `None` ⇒ the trace is live ; `Some(...)` ⇒ the trace's
    /// PII-bearing fields (`full_input`, `full_output`, body store
    /// entries) have been erased per GDPR Art. 17 or a retention
    /// policy.  See [`TombstoneMetadata`] for the contract.
    ///
    /// **NOT** part of [`payload::canonical_row_body`] — this preserves
    /// the row's `payload_hash` and the HMAC chain integrity.
    pub tombstone: Option<TombstoneMetadata>,

    /// **Sprint 8 — N14** : Pointer to the swarm's envelope trace.
    ///
    /// - `None` for the envelope trace itself (it IS the envelope).
    /// - `None` for `RolloutPolicyChanged` virtual-swarm rows (N13).
    /// - `None` for traces written outside a swarm context (stand-
    ///   alone runtime calls, audit chain bootstrap, etc.).
    /// - `Some(envelope_id)` for every op / cache-hit / shadow /
    ///   comparison trace written within a swarm dispatch.
    ///
    /// **NOT** part of [`payload::canonical_row_body`] — same rationale
    /// as `tombstone` : operational metadata, preserves HMAC
    /// verification of pre-N14 rows.  Sprint 9 may promote to canonical
    /// via a `body_version` schema bump.
    pub swarm_envelope_trace_id: Option<TraceId>,
}

impl NanoAgentTrace {
    /// **Sprint 7 — N6** : Backward-compat accessor returning the resolved
    /// input hash (was the pre-Sprint-7 `input_hash` field).  Use this
    /// when comparing against a hash computed from a JSON value (e.g.
    /// replay determinism check on the materialised input).
    pub fn input_hash(&self) -> [u8; 32] {
        self.input_hash_resolved
    }

    /// **Sprint 8 — N14** : true iff this trace IS the swarm envelope
    /// (vs an op trace within a swarm).
    pub fn is_swarm_envelope(&self) -> bool {
        self.error_kind.as_deref() == Some("SwarmEnvelope")
    }

    /// **Sprint 7 — N10** : true iff this trace has been GDPR-tombstoned.
    /// Tombstoned traces preserve all crypto-relevant fields (hashes +
    /// chain HMAC) but have their PII bodies erased.
    pub fn is_tombstoned(&self) -> bool {
        self.tombstone.is_some()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 7 — N10 : Tombstone metadata
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 7 — N10** : Why a trace was tombstoned.
///
/// The pre-Sprint-7 audit chain had no erasure mechanism — GDPR Art. 17
/// erasure requests had to either (a) destroy the chain (breaking
/// tamper-evidence) or (b) refuse erasure (illegal under EU law).
/// Sprint 7 — N10 introduces the **tombstone** : a structured erasure
/// of PII-bearing fields while cryptographically-frozen fields
/// (hashes + HMAC) remain intact.  This satisfies BOTH :
///
/// 1. **GDPR Art. 17** : the PII content is no longer retrievable
///    (`full_input`, `full_output`, side-store body entries erased).
/// 2. **Audit-grade integrity** : the chain HMAC verification still
///    passes because the `payload_hash` was computed on the original
///    canonical body, and `canonical_row_body` is invariant under
///    tombstoning (the tombstone field is NOT part of it).
///
/// **Residual PII risk** : `input_hash_source` / `input_hash_resolved`
/// / `output_hash` remain.  GDPR considers SHA-256 hashes as
/// pseudonymized when the corresponding plaintext is destroyed
/// (which it is, by tombstone construction).  An attacker who
/// already holds the plaintext can re-hash and confirm
/// participation — but they already had the plaintext anyway, so no
/// new information leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureReason {
    /// User invoked GDPR Article 17 (right to erasure / right to be
    /// forgotten).  The reason MUST be propagated to the M28 events
    /// bus as `efe.tenant.{slug}.gdpr.trace_tombstoned`.
    GdprArticle17,

    /// Retention policy expired (e.g. "delete traces older than 90 days
    /// for tenant X").  No GDPR request involved.
    RetentionPolicy,

    /// Operator-initiated, e.g. "tenant T migrated off-platform".
    /// Not GDPR but still legitimate.
    OperatorRequest,

    /// Trace's underlying tenant was deleted ; cascade erasure.
    TenantDeleted,
}

impl ErasureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GdprArticle17 => "gdpr_article_17",
            Self::RetentionPolicy => "retention_policy",
            Self::OperatorRequest => "operator_request",
            Self::TenantDeleted => "tenant_deleted",
        }
    }
}

/// **Sprint 7 — N10** : Structured marker on a tombstoned trace.
///
/// Contains the operational metadata of the erasure (who, when, why).
/// Stored alongside the trace but NOT part of `canonical_row_body` so
/// adding a tombstone preserves the chain HMAC.
///
/// The `tombstone_hmac` provides **tamper-evidence on the tombstone
/// itself** : an attacker who tries to forge a tombstone (or modify
/// an existing one) without the tenant audit key fails the check.
/// This is independent of the main chain HMAC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TombstoneMetadata {
    /// When the tombstone was applied.  Microsecond resolution.
    pub erased_at: chrono::DateTime<chrono::Utc>,

    /// Why the trace was erased.
    pub erasure_reason: ErasureReason,

    /// Optional link to a higher-level erasure request id (e.g. the
    /// `efe_request_gdpr_erasure` MCP tool's request_id).  Lets
    /// auditors trace "which user GDPR ask led to this tombstone".
    pub erasure_request_id: Option<Uuid>,

    /// Free-form identifier of the actor that triggered the tombstone :
    /// - `"gdpr_saga:request:{uuid}"` for the M24 GDPR saga path
    /// - `"retention_policy:90d"` for time-based purges
    /// - `"operator:{username}"` for manual ops
    pub erasure_actor: String,

    /// HMAC-SHA256 over `(trace_id ‖ erased_at_micros ‖ erasure_reason_str
    /// ‖ erasure_request_id_bytes ‖ erasure_actor_bytes)` keyed by the
    /// tenant audit key.  Detects tampering with any tombstone field.
    pub tombstone_hmac: [u8; 32],
}

impl TombstoneMetadata {
    /// Construct (the caller is responsible for computing
    /// `tombstone_hmac` — see [`payload::compute_tombstone_hmac`]).
    pub fn new(
        erased_at: chrono::DateTime<chrono::Utc>,
        erasure_reason: ErasureReason,
        erasure_request_id: Option<Uuid>,
        erasure_actor: impl Into<String>,
        tombstone_hmac: [u8; 32],
    ) -> Self {
        Self {
            erased_at,
            erasure_reason,
            erasure_request_id,
            erasure_actor: erasure_actor.into(),
            tombstone_hmac,
        }
    }
}

/// Trait for recording nano agent traces.
#[async_trait]
pub trait AuditWriter: Send + Sync + 'static {
    /// Record a single trace.  Implementations should be resilient :
    /// audit failures must not crash the runtime (cf
    /// [`HmacChainAuditWriter::record`] which logs + drops on failure).
    async fn record(&self, trace: NanoAgentTrace);
}

/// Compute the SHA-256 hash of a JSON value for audit purposes.
///
/// Sprint 6 upgrade : real SHA-256 (was a 64-bit `DefaultHasher` placeholder
/// in Phase 1).  This affects the `input_hash` + `output_hash` fields of
/// every trace from Sprint 6 onwards.
pub fn hash_json(value: &serde_json::Value) -> [u8; 32] {
    // Use canonical serialization so that semantically-equivalent inputs
    // (different key orders, etc.) hash to the same value.  This matches
    // the idempotency cache + the chain payload canonicalization.
    let canonical = crate::idempotency::canonicalize_json(value);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hash_json_deterministic() {
        let v = json!({"a": 1, "b": "x"});
        let h1 = hash_json(&v);
        let h2 = hash_json(&v);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_json_invariant_to_key_order() {
        let h1 = hash_json(&json!({"a": 1, "b": 2}));
        let h2 = hash_json(&json!({"b": 2, "a": 1}));
        // Both should hash to the same value because canonicalization
        // sorts keys.
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_json_distinguishes_values() {
        let h1 = hash_json(&json!({"a": 1}));
        let h2 = hash_json(&json!({"a": 2}));
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_json_produces_32_bytes() {
        let h = hash_json(&json!({}));
        assert_eq!(h.len(), 32);
    }

    #[test]
    fn hash_json_not_all_zeros_for_real_input() {
        let h = hash_json(&json!({"x": 1, "y": "test"}));
        assert_ne!(h, [0u8; 32]);
    }

    #[test]
    fn trace_construction() {
        let trace = NanoAgentTrace {
            trace_id: TraceId::new(),
            parent_trace_ids: vec![],
            swarm_request_id: Uuid::new_v4(),
            operation_id: "op-1".to_string(),
            tenant_id: TenantId::new(),
            caller_agent_id: Some("test".to_string()),
            nano_agent_id: NanoAgentId::new("test_agent").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".to_string(),
            input_hash_source: [0; 32],
            input_hash_resolved: [0; 32],
            output_hash: Some([1; 32]),
            error_kind: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            latency_ms: Some(5),
            cost_cents: 0.0001,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        };
        assert_eq!(trace.operation_id, "op-1");
        assert!(trace.full_input.is_none());
        assert!(trace.full_output.is_none());
        assert!(trace.parent_trace_ids.is_empty());
        assert_eq!(trace.input_hash(), [0u8; 32]);
    }
}
