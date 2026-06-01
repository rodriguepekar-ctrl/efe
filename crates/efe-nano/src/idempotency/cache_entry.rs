//! **Sprint 8 — N12 day 2** : `CacheEntry` v2 — audit-grade payload.
//!
//! See `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` § 4.2 and § 6.
//!
//! # What changed vs Sprint 1 (`CacheEntry` v1)
//!
//! The v1 entry was a thin 3-field tuple (`output`, `cost_cents`,
//! `expires_at`).  It worked for a single-process cache that did not
//! participate in the audit chain — but the moment we add audit-grade
//! cache-hit traces (N12 G5), the entry needs to carry **provenance
//! metadata** so the cache-hit trace can record :
//!
//! 1. **Which version produced the cached output** (`resolved_version`).
//!    Without this, a cache hit served after a version bump would
//!    record the WRONG version in its trace.  Subtle — caught by audit.
//!
//! 2. **The original trace_id** (`original_trace_id`) so the cache-hit
//!    trace can declare it as its `parent_trace_ids[0]` — that is the
//!    N1 lineage link that lets the auditor walk back to the cold call.
//!
//! 3. **LLM-side metadata for Mode C** (`mode_c_metadata`) so cache-hit
//!    traces correctly report `llm_tokens_used` AND so the Mode C
//!    replay engine can match the original LLM call without re-querying
//!    the catalog.
//!
//! 4. **`size_bytes`** so the LRU evictor can apply a per-tenant memory
//!    cap.  Computed once at insert time — `CacheEntry` is immutable.
//!
//! 5. **`cached_at`** so the `flush_older_than` admin API works without
//!    a separate index.
//!
//! # Immutability invariant
//!
//! After [`CacheEntry::new`] returns, the entry is **immutable**.  No
//! field is mutated — even `expires_at` is set at construction.  Eviction
//! removes + reinserts a fresh entry rather than mutating in-place.  This
//! pattern :
//!
//! * removes the need for read-write locks on the entry,
//! * lets `Arc<CacheEntry>` be cheap-cloned for handoff to the trace
//!   writer (cache hit serves data + writes a trace concurrently),
//! * and matches the canonical-row-body invariant — the entry is just
//!   like a single trace row, frozen at write time.
//!
//! The single exception is the LRU "touch" — but that lives in the
//! `LruIndex`, NOT in this struct.  Sprint 9.

use chrono::{DateTime, Duration, Utc};
use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TenantId, TraceId};
use serde_json::Value;

// ─────────────────────────────────────────────────────────────────────
//  CacheEntry v2
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : Audit-grade cache payload.
///
/// Holds the cached output **plus** the provenance metadata that the
/// cache-hit trace writer needs to produce a chain-consistent audit row.
///
/// The `(tenant_id, agent_id, resolved_version, input_hash_resolved)`
/// quadruplet stored inline lets the flush API (§ 4.4) target entries
/// by criteria the opaque hex `CacheKey` itself cannot reveal.  Memory
/// overhead is ~78 bytes per entry — acceptable, and saves an order of
/// magnitude of complexity vs maintaining secondary indexes.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The cached output value as the runtime returned it.
    pub output: Value,

    /// Cost the producing call paid (cents).  Replayed on cache hit so
    /// the swarm cost cap sees the SAME cost for a cache hit as for a
    /// fresh call — preserving cost accounting under cache hits.
    pub cost_cents: f64,

    /// When this entry was inserted.  Used by `flush_older_than` and by
    /// the cache-hit trace for `cached_at` analytics.
    pub cached_at: DateTime<Utc>,

    /// Hard TTL boundary.  Past this instant, the entry is considered
    /// expired and a `get` returns `None` (and triggers eviction).
    pub expires_at: DateTime<Utc>,

    /// Tenant scope.  Stored so `flush_for_tenant` is an O(N) DashMap
    /// scan rather than an N×N collision-search.
    pub tenant_id: TenantId,

    /// Agent that produced this output.  Stored for `flush_for_agent`
    /// + cache-hit trace `nano_agent_id` projection without re-querying
    ///   the catalog.
    pub agent_id: NanoAgentId,

    /// Catalog-resolved agent version that produced this output.
    ///
    /// **Why store it ?** A cache hit served *after* a version bump
    /// must still record the **original** producing version on its
    /// trace — auditors need to know which code actually generated the
    /// bytes they're seeing.  Without this field the cache-hit trace
    /// would have to look up `agent.version` from the catalog at
    /// hit-time, picking up the new version.  Wrong.
    pub resolved_version: NanoAgentVersion,

    /// SHA-256 of the canonical input bytes (matches the trace's
    /// `input_hash_resolved`).  Stored so `flush_for_input_hash` can
    /// target a specific entry without iterating every key derivation.
    pub input_hash_resolved: [u8; 32],

    /// Trace id of the original (cold) call that populated this entry.
    ///
    /// The cache-hit trace declares this as its `parent_trace_ids[0]`
    /// — the N1 lineage link.  Auditors walking the chain see :
    ///   `... → cache_hit_trace.parent → cold_call_trace ...`
    /// and can replay the cold call deterministically.
    pub original_trace_id: TraceId,

    /// Optional LLM-side metadata for Mode C entries.  `None` for
    /// Mode A and Mode B.
    pub mode_c_metadata: Option<LlmCacheMetadata>,

    /// Approximate memory footprint of `output` (UTF-8 byte count of
    /// canonical JSON serialization).  Used by the LRU evictor to
    /// enforce a per-tenant byte budget.  Computed once at construction
    /// — never updated.
    pub size_bytes: u64,
}

impl CacheEntry {
    /// **Sprint 8 — N12** : Build a fresh entry from a fresh runtime
    /// response.  TTL is taken from the caller (derived from
    /// [`efe_nano_sdk::IdempotencyPolicy`]).
    ///
    /// The `size_bytes` field is computed by canonical-serializing
    /// `output` — so it is invariant to JSON key ordering and matches
    /// the cache-key canonicalization used elsewhere.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        output: Value,
        cost_cents: f64,
        ttl_seconds: u32,
        tenant_id: TenantId,
        agent_id: NanoAgentId,
        resolved_version: NanoAgentVersion,
        input_hash_resolved: [u8; 32],
        original_trace_id: TraceId,
        mode_c_metadata: Option<LlmCacheMetadata>,
    ) -> Self {
        let now = Utc::now();
        let size_bytes = canonical_size_bytes(&output);
        Self {
            output,
            cost_cents,
            cached_at: now,
            expires_at: now + Duration::seconds(ttl_seconds as i64),
            tenant_id,
            agent_id,
            resolved_version,
            input_hash_resolved,
            original_trace_id,
            mode_c_metadata,
            size_bytes,
        }
    }

    /// Returns true if the entry's TTL has elapsed.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }

    /// **Sprint 8 — N12** : Construct a [`CachedOutput`] view for a
    /// cache hit.  Drops the audit metadata since the caller path only
    /// needs the output + cost ; the audit metadata is consumed by the
    /// cache-hit trace writer separately.
    pub fn to_cached_output(&self) -> CachedOutput {
        CachedOutput {
            output: self.output.clone(),
            cost_cents: self.cost_cents,
            resolved_version: self.resolved_version.clone(),
            original_trace_id: self.original_trace_id,
            mode_c_metadata: self.mode_c_metadata.clone(),
        }
    }
}

/// **Sprint 8 — N12** : Mode-C-specific metadata stored on the cache
/// entry so the cache-hit trace + the replay engine have everything
/// they need without re-querying the catalog or the prompt registry.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmCacheMetadata {
    pub provider: String,
    pub model: String,
    pub prompt_template_id: String,

    /// Numeric version of the prompt template at the time of the cold
    /// call.  Lets the cache-hit trace surface "we served output from
    /// template vX" even after the registry bumped to vX+1.  Defaults
    /// to 0 when the registry isn't wired.
    pub prompt_template_version: u32,

    /// How many tokens the cold call consumed.  Used on cache hit to
    /// record the SAME `llm_tokens_used` in the cache-hit trace — this
    /// preserves billing accuracy across cache hits.
    pub tokens_used: u32,

    /// LLM-side cost component (subset of `cost_cents` — useful for
    /// breaking out LLM cost vs orchestration cost in analytics).
    pub llm_cost_cents: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  CachedOutput (caller-facing view)
// ─────────────────────────────────────────────────────────────────────

/// What [`crate::idempotency::IdempotencyCache::get`] returns on a hit.
///
/// **Sprint 8 — N12** : extended with `resolved_version`,
/// `original_trace_id`, and `mode_c_metadata` so the dispatcher can
/// write a chain-consistent cache-hit trace without re-querying the
/// catalog.
#[derive(Debug, Clone)]
pub struct CachedOutput {
    pub output: Value,
    pub cost_cents: f64,
    pub resolved_version: NanoAgentVersion,
    pub original_trace_id: TraceId,
    pub mode_c_metadata: Option<LlmCacheMetadata>,
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

/// Canonicalize `v` (sort object keys recursively) and return the byte
/// length.  This matches [`crate::idempotency::canonicalize_json`] —
/// using a wrapper keeps the dependency contained and the function
/// testable in isolation.
pub(crate) fn canonical_size_bytes(v: &Value) -> u64 {
    super::canonicalize_json(v).len() as u64
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::TraceId;
    use serde_json::json;

    fn v(s: &str) -> NanoAgentVersion {
        NanoAgentVersion::new(s).unwrap()
    }
    fn aid(s: &str) -> NanoAgentId {
        NanoAgentId::new(s).unwrap()
    }
    fn make(output: Value, ttl: u32, mode_c: Option<LlmCacheMetadata>) -> CacheEntry {
        CacheEntry::new(
            output,
            0.005,
            ttl,
            TenantId::system(),
            aid("test_agent"),
            v("1.0.0"),
            [0u8; 32],
            TraceId::new(),
            mode_c,
        )
    }

    #[test]
    fn new_round_trip_carries_fields() {
        let trace_id = TraceId::new();
        let tenant = TenantId::system();
        let e = CacheEntry::new(
            json!({"ok": true}),
            0.005,
            60,
            tenant,
            aid("test_agent"),
            v("1.0.0"),
            [1u8; 32],
            trace_id,
            None,
        );
        assert_eq!(e.output, json!({"ok": true}));
        assert!((e.cost_cents - 0.005).abs() < 1e-9);
        assert_eq!(e.tenant_id, tenant);
        assert_eq!(e.agent_id, aid("test_agent"));
        assert_eq!(e.resolved_version, v("1.0.0"));
        assert_eq!(e.input_hash_resolved, [1u8; 32]);
        assert_eq!(e.original_trace_id, trace_id);
        assert!(e.mode_c_metadata.is_none());
    }

    #[test]
    fn expires_at_is_cached_at_plus_ttl() {
        let e = make(json!({}), 60, None);
        let dt = e.expires_at - e.cached_at;
        assert_eq!(dt.num_seconds(), 60);
    }

    #[test]
    fn is_expired_reflects_expires_at() {
        let e = make(json!({}), 60, None);
        let before_expiry = e.expires_at - Duration::seconds(1);
        let after_expiry = e.expires_at + Duration::seconds(1);
        assert!(!e.is_expired(before_expiry));
        assert!(e.is_expired(after_expiry));
        // Boundary : exactly at expires_at counts as expired (≤ semantics).
        assert!(e.is_expired(e.expires_at));
    }

    #[test]
    fn size_bytes_counts_canonical_form() {
        let e = make(json!({"k": "v"}), 60, None);
        // canonical form is `{"k":"v"}` → 9 bytes.
        assert_eq!(e.size_bytes, 9);
    }

    #[test]
    fn size_bytes_invariant_under_object_key_order() {
        let e1 = make(json!({"a": 1, "b": 2}), 60, None);
        let e2 = make(json!({"b": 2, "a": 1}), 60, None);
        assert_eq!(e1.size_bytes, e2.size_bytes);
    }

    #[test]
    fn to_cached_output_preserves_audit_metadata() {
        let e = make(
            json!({"x": 1}),
            60,
            Some(LlmCacheMetadata {
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                prompt_template_id: "m50.test.v1".into(),
                prompt_template_version: 1,
                tokens_used: 1234,
                llm_cost_cents: 0.003,
            }),
        );
        let co = e.to_cached_output();
        assert_eq!(co.output, e.output);
        assert!((co.cost_cents - e.cost_cents).abs() < 1e-9);
        assert_eq!(co.resolved_version, e.resolved_version);
        assert_eq!(co.original_trace_id, e.original_trace_id);
        assert_eq!(co.mode_c_metadata, e.mode_c_metadata);
    }

    #[test]
    fn llm_metadata_round_trip_clone() {
        let m = LlmCacheMetadata {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_template_id: "m50.test.v1".into(),
            prompt_template_version: 7,
            tokens_used: 42,
            llm_cost_cents: 0.001,
        };
        let c = m.clone();
        assert_eq!(m, c);
    }

    #[test]
    fn entry_is_clonable() {
        // Sanity : Arc<CacheEntry> usage requires cheap clones.
        let e = make(json!({"big": "x".repeat(1000)}), 60, None);
        let e2 = e.clone();
        assert_eq!(e.size_bytes, e2.size_bytes);
        assert_eq!(e.original_trace_id, e2.original_trace_id);
        assert_eq!(e.tenant_id, e2.tenant_id);
    }

    #[test]
    fn ttl_zero_constructs_entry_already_expired() {
        let e = make(json!({}), 0, None);
        assert!(e.is_expired(e.cached_at));
    }

    #[test]
    fn distinct_tenants_are_distinguishable() {
        let e1 = CacheEntry::new(
            json!({}),
            0.0,
            60,
            TenantId::new(),
            aid("a"),
            v("1.0.0"),
            [0u8; 32],
            TraceId::new(),
            None,
        );
        let e2 = CacheEntry::new(
            json!({}),
            0.0,
            60,
            TenantId::new(),
            aid("a"),
            v("1.0.0"),
            [0u8; 32],
            TraceId::new(),
            None,
        );
        // Random TenantIds — should differ.
        assert_ne!(e1.tenant_id, e2.tenant_id);
    }
}
