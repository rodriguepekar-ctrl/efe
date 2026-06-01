//! In-process idempotency cache — **Sprint 8 N12 hardening**.
//!
//! See `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md`.
//!
//! # What N12 changed
//!
//! Sprint 1-4 shipped a 3-field `CacheEntry` keyed by a 4-component
//! SHA-256 (`tenant ‖ agent_id ‖ resolved_version ‖ canonical_input`).
//! That key shape silently collided across :
//!
//! 1. **Execution mode** — Mode A/B/C agents sharing an id at the same
//!    version (Phase 4 customer authoring).
//! 2. **Mode C provider/model** — same agent dispatched to Anthropic vs
//!    OpenAI returns different outputs but hit the same cache.
//! 3. **Prompt template id** — bumping the M21 registry template did NOT
//!    bust the cache.
//! 4. **PII strategy** — toggling `PiiStrategy` could serve a non-
//!    redacted era output to a redacted-era query (regulatory leak).
//!
//! And the cache hit path was **invisible to the HMAC audit chain** :
//! a cache hit returned the cached value without producing a trace, so
//! auditors saw the side effects (cost charges, swarm DAG continuations)
//! with no row explaining where the bytes came from.
//!
//! N12 ships :
//!
//! 1. [`cache_key`] — v2 key derivation with a 9-component discriminator
//!    and a versioned domain prefix (`"efe-nano-idempotency-v2"`) that
//!    enables one-shot mass invalidation by bump.
//! 2. [`cache_entry`] — v2 payload carrying `(tenant, agent_id,
//!    resolved_version, original_trace_id, input_hash, mode_c_metadata,
//!    size_bytes)` so the cache-hit trace writer (dispatcher) has full
//!    provenance without re-querying the catalog.
//! 3. **Flush API** — [`IdempotencyCache::flush_for_agent`],
//!    [`IdempotencyCache::flush_for_tenant`],
//!    [`IdempotencyCache::flush_for_input_hash`],
//!    [`IdempotencyCache::flush_older_than`] for operator-initiated
//!    invalidation (republish, GDPR cascade, drift detector).
//! 4. **Atomic counter metrics** — `hits_total`, `misses_total`,
//!    `evictions_total`, `flush_total`, `size_bytes`, `entries_count`
//!    — exposed via [`IdempotencyCache::metrics`] for the M40 metrics
//!    pipeline.
//! 5. **Dual-read v1 compat** — [`IdempotencyCache::derive_key_v1`]
//!    still produces a v1-shape key for the migration window (one
//!    deploy cycle).  Sprint 9 deprecates + removes.
//!
//! # Sprint 9 follow-ups
//!
//! * **LRU evictor** with per-tenant byte cap (§ 4.5 architecture doc).
//! * **Postgres + Redis backend** behind a `CacheStore` trait
//!   (§ Annexe B).
//! * **Remove v1 compat path** once telemetry confirms no v1 reads.

pub mod cache_entry;
pub mod cache_key;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use efe_nano_sdk::{NanoAgent, NanoAgentId, NanoAgentVersion, TenantId};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

pub use cache_entry::{CacheEntry, CachedOutput, LlmCacheMetadata};
pub use cache_key::{
    derive_key_v1_compat, derive_key_v2, CacheKey, CACHE_KEY_DOMAIN_PREFIX,
    CACHE_KEY_DOMAIN_PREFIX_V1,
};

// ─────────────────────────────────────────────────────────────────────
//  CacheMetrics
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12 § 4.6** : Atomic-counter metrics surfaced via
/// [`IdempotencyCache::metrics`].  Exposed to the M40 metrics pipeline ;
/// scraped into Prometheus by the platform-side adapter.
///
/// All counters use `Ordering::Relaxed` — the cache itself is the
/// source of truth (DashMap), counters are auxiliary observability and
/// strict ordering buys nothing.
#[derive(Debug, Default)]
pub struct CacheMetrics {
    /// Total cache hits served (incremented on `get` returning `Some`).
    pub hits_total: AtomicU64,

    /// Total cache misses (incremented on `get` returning `None`).
    pub misses_total: AtomicU64,

    /// Total entries evicted (TTL expiry on `get`, manual
    /// `evict_expired`, flush API).
    pub evictions_total: AtomicU64,

    /// Total flush API invocations (sum of `flush_for_*` calls).
    pub flush_total: AtomicU64,

    /// Total entries removed by flush API (sum across all `flush_for_*`
    /// return values).  Distinguishes flush calls from flush volume.
    pub flush_entries_total: AtomicU64,

    /// Live entries count.  Increments on insert (new key), decrements
    /// on every removal (expiry, flush, manual).
    pub entries_count: AtomicU64,

    /// Live total `size_bytes` of every entry held.  Maintained by
    /// add-on-insert / sub-on-remove.
    pub size_bytes: AtomicU64,
}

impl CacheMetrics {
    /// Snapshot the counters into a plain struct for downstream
    /// serialization or assertion.  Cheap — 7 relaxed loads.
    pub fn snapshot(&self) -> CacheMetricsSnapshot {
        CacheMetricsSnapshot {
            hits_total: self.hits_total.load(Ordering::Relaxed),
            misses_total: self.misses_total.load(Ordering::Relaxed),
            evictions_total: self.evictions_total.load(Ordering::Relaxed),
            flush_total: self.flush_total.load(Ordering::Relaxed),
            flush_entries_total: self.flush_entries_total.load(Ordering::Relaxed),
            entries_count: self.entries_count.load(Ordering::Relaxed),
            size_bytes: self.size_bytes.load(Ordering::Relaxed),
        }
    }
}

/// Plain-data snapshot of [`CacheMetrics`], suitable for assertion +
/// JSON-serialization downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMetricsSnapshot {
    pub hits_total: u64,
    pub misses_total: u64,
    pub evictions_total: u64,
    pub flush_total: u64,
    pub flush_entries_total: u64,
    pub entries_count: u64,
    pub size_bytes: u64,
}

// ─────────────────────────────────────────────────────────────────────
//  IdempotencyCache
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : in-process TTL cache, hardened to v2.
///
/// Storage is `DashMap<String, CacheEntry>` where the key is the
/// 64-char hex SHA-256 produced by [`derive_key_v2`].  Stored as
/// `String` (not `CacheKey`) so the DashMap can use the fast borrowed-
/// `str` lookup path.
#[derive(Debug, Default)]
pub struct IdempotencyCache {
    entries: DashMap<String, CacheEntry>,
    metrics: CacheMetrics,
}

impl IdempotencyCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// **Number of stored entries.**  Source-of-truth is DashMap.len() —
    /// the `entries_count` metric counter mirrors this but can briefly
    /// disagree under racing inserts/removes (Relaxed Ordering).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// **Borrow the metrics** for the M40 metrics pipeline.
    pub fn metrics(&self) -> &CacheMetrics {
        &self.metrics
    }

    // ─────────────────────────────────────────────────────────────────
    //  Key derivation
    // ─────────────────────────────────────────────────────────────────

    /// **Sprint 8 — N12** : derive a v2 cache key from the full
    /// [`NanoAgent`] context.  This is the modern API ; pre-N12 code
    /// using the v1 4-component API should call [`Self::derive_key_v1`]
    /// during the migration window.
    pub fn derive_key(&self, tenant: TenantId, agent: &NanoAgent, input: &Value) -> CacheKey {
        let canonical = canonicalize_json(input);
        derive_key_v2(tenant, agent, canonical.as_bytes())
    }

    /// **Pre-N12** key derivation, kept for migration only.  Produces
    /// the same hex SHA-256 the Sprint 4 cache wrote, so v1 entries
    /// migrated forward still serve their original hits.
    ///
    /// Sprint 9 will deprecate this method, log a `WARN` for every
    /// call (so platform engineers can spot stragglers), then remove
    /// it after one quiet release cycle.
    pub fn derive_key_v1(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version: &NanoAgentVersion,
        input: &Value,
    ) -> CacheKey {
        let canonical = canonicalize_json(input);
        derive_key_v1_compat(tenant, agent_id, version, canonical.as_bytes())
    }

    // ─────────────────────────────────────────────────────────────────
    //  Core get/put — v2 API
    // ─────────────────────────────────────────────────────────────────

    /// Look up an entry by its v2 key.  Returns the full [`CachedOutput`]
    /// view (audit metadata included) so the dispatcher can write a
    /// chain-consistent cache-hit trace.
    ///
    /// Expired entries are removed lazily.  Counts a hit/miss in
    /// [`CacheMetrics`].
    pub async fn get(&self, key: &CacheKey) -> Option<CachedOutput> {
        self.get_by_str(key.as_str()).await
    }

    /// Internal variant : look up by raw hex string.  Used by both the
    /// v2 path and (during migration) any v1 reads.
    async fn get_by_str(&self, key_str: &str) -> Option<CachedOutput> {
        let mut should_remove_expired = false;
        let mut hit: Option<CachedOutput> = None;

        // Phase 1 : peek under shared lock.
        if let Some(entry) = self.entries.get(key_str) {
            if !entry.is_expired(Utc::now()) {
                hit = Some(entry.to_cached_output());
            } else {
                should_remove_expired = true;
            }
        }

        // Phase 2 : remove if expired (drops shared lock first).
        if should_remove_expired {
            if let Some((_, removed)) = self.entries.remove(key_str) {
                self.metrics
                    .evictions_total
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .size_bytes
                    .fetch_sub(removed.size_bytes, Ordering::Relaxed);
                self.metrics
                    .entries_count
                    .fetch_sub(1, Ordering::Relaxed);
            }
        }

        if hit.is_some() {
            self.metrics.hits_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.metrics.misses_total.fetch_add(1, Ordering::Relaxed);
        }
        hit
    }

    /// Insert a v2 entry.  TTL of 0 is a no-op (entries with no TTL
    /// shouldn't reach the cache).
    pub async fn put(&self, key: CacheKey, entry: CacheEntry) {
        if entry.expires_at <= entry.cached_at {
            // Already-expired or TTL-0 — skip insert.
            return;
        }
        let new_size = entry.size_bytes;
        if let Some(prev) = self.entries.insert(key.into_string(), entry) {
            // Overwrite : subtract previous size, count stays same.
            self.metrics
                .size_bytes
                .fetch_sub(prev.size_bytes, Ordering::Relaxed);
            self.metrics
                .size_bytes
                .fetch_add(new_size, Ordering::Relaxed);
        } else {
            // New insert : bump count + add size.
            self.metrics
                .entries_count
                .fetch_add(1, Ordering::Relaxed);
            self.metrics
                .size_bytes
                .fetch_add(new_size, Ordering::Relaxed);
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Eviction
    // ─────────────────────────────────────────────────────────────────

    /// Sweep every entry, dropping the expired ones.  O(N) — cheap for
    /// in-process caches under normal load ; Sprint 9 LRU evictor adds
    /// a per-tenant amortized fast path.  Returns the eviction count.
    pub async fn evict_expired(&self) -> usize {
        let now = Utc::now();
        let mut evicted_size: u64 = 0;
        let mut evicted_count: u64 = 0;
        self.entries.retain(|_, entry| {
            if entry.is_expired(now) {
                evicted_size = evicted_size.saturating_add(entry.size_bytes);
                evicted_count += 1;
                false
            } else {
                true
            }
        });
        if evicted_count > 0 {
            self.metrics
                .evictions_total
                .fetch_add(evicted_count, Ordering::Relaxed);
            self.metrics
                .size_bytes
                .fetch_sub(evicted_size, Ordering::Relaxed);
            self.metrics
                .entries_count
                .fetch_sub(evicted_count, Ordering::Relaxed);
        }
        evicted_count as usize
    }

    // ─────────────────────────────────────────────────────────────────
    //  Flush API — N12 § 4.4
    // ─────────────────────────────────────────────────────────────────

    /// **N12 § 4.4** : Flush every entry for `(tenant, agent_id, *, *)`
    /// — used when operators republish or rollback an agent and want to
    /// drop every cached output that named the prior version, mode,
    /// provider, or template.
    ///
    /// Returns the count of entries removed.  Increments
    /// `flush_total` (by 1) and `flush_entries_total` (by the count).
    pub async fn flush_for_agent(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
    ) -> usize {
        self.flush_matching(|entry| entry.tenant_id == tenant && &entry.agent_id == agent_id)
            .await
    }

    /// **N12 § 4.4** : Flush every entry for `(tenant, *, *, *)`.  Used
    /// on :
    /// * Tenant offboarding (M0 customer success workflow)
    /// * GDPR cascade (N10 tombstone trigger : when a tenant invokes
    ///   right-to-erasure on bulk data, every cached output derived
    ///   from that tenant's inputs must go too — otherwise a cache hit
    ///   on a still-warm entry would resurface erased PII).
    pub async fn flush_for_tenant(&self, tenant: TenantId) -> usize {
        self.flush_matching(|entry| entry.tenant_id == tenant).await
    }

    /// **N12 § 4.4** : Flush a specific
    /// `(tenant, agent_id, version, input_hash)` entry.
    ///
    /// Used by the M24 drift detector when a specific cached output is
    /// suspected of being staleness-poisoned (e.g. the upstream data
    /// source changed but the cache hasn't expired yet).
    pub async fn flush_for_input_hash(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version: &NanoAgentVersion,
        input_hash: &[u8; 32],
    ) -> usize {
        self.flush_matching(|entry| {
            entry.tenant_id == tenant
                && &entry.agent_id == agent_id
                && &entry.resolved_version == version
                && &entry.input_hash_resolved == input_hash
        })
        .await
    }

    /// **N12 § 4.4** : Flush everything inserted before `threshold`.
    /// Used during the cache schema rotation (v2 → v3) so stragglers
    /// don't linger past the TTL.
    pub async fn flush_older_than(&self, threshold: DateTime<Utc>) -> usize {
        self.flush_matching(|entry| entry.cached_at < threshold).await
    }

    /// **N12 + N10 GDPR cascade** : Flush every entry whose
    /// `original_trace_id` matches `trace_id`.
    ///
    /// **The cascade contract.**  When the GDPR saga (M24) tombstones
    /// a trace via [`crate::audit::chain_store::ChainStore::tombstone_trace`],
    /// the trace's PII bodies are erased from the chain.  But the cache
    /// may still hold the same output for a cache hit served before the
    /// tombstone — and serving that cache hit would resurface erased
    /// PII (the tombstone is then ineffective).
    ///
    /// To prevent that, the saga MUST call this method **immediately
    /// after** the chain tombstone succeeds.  The cache then evicts
    /// every entry produced by the tombstoned trace.  Subsequent calls
    /// re-derive the output through the runtime — which is what we
    /// want, because the catalog/code/PII-strategy may have evolved.
    ///
    /// Returns the number of evicted entries.  In normal operation
    /// (one cache entry per trace) this is 0 or 1.  A retry-storm cold
    /// call that produced multiple trace_ids for the same (tenant,
    /// agent, input) tuple would be cleaned together — that's correct.
    pub async fn flush_for_original_trace(
        &self,
        trace_id: efe_nano_sdk::TraceId,
    ) -> usize {
        self.flush_matching(|entry| entry.original_trace_id == trace_id)
            .await
    }

    /// **Generic flush** : retain entries where the predicate returns
    /// `false`.  Internal helper for the flush API.  All public
    /// flush methods route here for consistent metric accounting.
    async fn flush_matching<F>(&self, predicate: F) -> usize
    where
        F: Fn(&CacheEntry) -> bool,
    {
        self.metrics.flush_total.fetch_add(1, Ordering::Relaxed);

        let mut removed_size: u64 = 0;
        let mut removed_count: u64 = 0;

        // DashMap::retain takes a `FnMut(&K, &mut V) -> bool` returning
        // true to KEEP.  We negate the predicate.
        self.entries.retain(|_, entry| {
            if predicate(entry) {
                removed_size = removed_size.saturating_add(entry.size_bytes);
                removed_count += 1;
                false // remove
            } else {
                true // keep
            }
        });

        if removed_count > 0 {
            self.metrics
                .flush_entries_total
                .fetch_add(removed_count, Ordering::Relaxed);
            self.metrics
                .size_bytes
                .fetch_sub(removed_size, Ordering::Relaxed);
            self.metrics
                .entries_count
                .fetch_sub(removed_count, Ordering::Relaxed);
        }

        removed_count as usize
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Canonical JSON serializer (unchanged from Sprint 1)
// ─────────────────────────────────────────────────────────────────────

/// Render a JSON value with keys sorted recursively so logically-equal
/// objects produce identical strings (and hence identical cache keys).
pub fn canonicalize_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(obj) => {
            // Sort keys alphabetically.
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                write_canonical(&obj[*k], out);
            }
            out.push('}');
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
        IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy,
        ResourceLimits, StabilityTier, TraceId,
    };
    use serde_json::json;
    use std::sync::Arc;

    // ─────────────────────────────────────────────────────────────────
    //  Fixtures
    // ─────────────────────────────────────────────────────────────────

    fn aid(s: &str) -> NanoAgentId {
        NanoAgentId::new(s).unwrap()
    }
    fn v(s: &str) -> NanoAgentVersion {
        NanoAgentVersion::new(s).unwrap()
    }
    fn agent(id: &str, ver: &str, mode: ExecutionMode) -> NanoAgent {
        NanoAgent {
            id: aid(id),
            version: v(ver),
            description: "t".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: mode,
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            error_schema: None,
            limits: ResourceLimits::default(),
            cost_estimate_cents: 0.0001,
            latency_p99_ms_target: 100,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        }
    }
    fn pure_agent(id: &str, ver: &str) -> NanoAgent {
        agent(
            id,
            ver,
            ExecutionMode::PureRust {
                entry: "x".into(),
            },
        )
    }
    fn fresh_entry(t: TenantId, a: &NanoAgent, input: &Value) -> CacheEntry {
        let canon = canonicalize_json(input);
        let mut h = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut h, canon.as_bytes());
        let digest = sha2::Digest::finalize(h);
        let mut input_hash = [0u8; 32];
        input_hash.copy_from_slice(&digest);
        CacheEntry::new(
            json!({"out": true}),
            0.005,
            60,
            t,
            a.id.clone(),
            a.version.clone(),
            input_hash,
            TraceId::new(),
            None,
        )
    }

    // ─────────────────────────────────────────────────────────────────
    //  Basic put/get
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn put_then_get_returns_value() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("test_agent", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({"x": 1}));
        let entry = fresh_entry(t, &a, &json!({"x": 1}));
        cache.put(key.clone(), entry).await;
        let hit = cache.get(&key).await.unwrap();
        assert_eq!(hit.output, json!({"out": true}));
        assert!((hit.cost_cents - 0.005).abs() < 1e-9);
        assert_eq!(hit.resolved_version, v("1.0.0"));
    }

    #[tokio::test]
    async fn miss_returns_none_and_counts_miss() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("test_agent", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({}));
        assert!(cache.get(&key).await.is_none());
        let snap = cache.metrics().snapshot();
        assert_eq!(snap.misses_total, 1);
        assert_eq!(snap.hits_total, 0);
    }

    #[tokio::test]
    async fn hit_increments_hits_counter() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("test_agent", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({}));
        cache.put(key.clone(), fresh_entry(t, &a, &json!({}))).await;
        let _ = cache.get(&key).await.unwrap();
        let _ = cache.get(&key).await.unwrap();
        let snap = cache.metrics().snapshot();
        assert_eq!(snap.hits_total, 2);
        assert_eq!(snap.misses_total, 0);
    }

    #[tokio::test]
    async fn ttl_zero_skips_insert() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("test_agent", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({}));
        let mut e = fresh_entry(t, &a, &json!({}));
        // Force already-expired.
        e.expires_at = e.cached_at;
        cache.put(key.clone(), e).await;
        assert!(cache.get(&key).await.is_none());
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn expired_entry_is_evicted_on_get() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("test_agent", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({}));
        let mut e = fresh_entry(t, &a, &json!({}));
        // Insert as live, then mutate the stored entry to be expired.
        cache.put(key.clone(), e.clone()).await;
        cache
            .entries
            .alter(key.as_str(), |_, mut entry| {
                entry.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
                entry
            });
        assert!(cache.get(&key).await.is_none());
        assert_eq!(cache.len(), 0);
        let snap = cache.metrics().snapshot();
        assert_eq!(snap.evictions_total, 1);
        // entries_count was incremented on put then decremented on evict.
        assert_eq!(snap.entries_count, 0);
        // Touch e to silence the unused-mut analysis on the temp binding.
        e.cost_cents = 0.0;
    }

    // ─────────────────────────────────────────────────────────────────
    //  Key derivation (v2 vs v1 compat)
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn key_invariant_on_object_key_order() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let k1 = cache.derive_key(t, &a, &json!({"a": 1, "b": 2}));
        let k2 = cache.derive_key(t, &a, &json!({"b": 2, "a": 1}));
        assert_eq!(k1, k2);
    }

    #[tokio::test]
    async fn v1_and_v2_keys_differ() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let v2 = cache.derive_key(t, &a, &json!({"x": 1}));
        let v1 = cache.derive_key_v1(t, &a.id, &a.version, &json!({"x": 1}));
        assert_ne!(v2, v1);
    }

    #[tokio::test]
    async fn v2_key_distinguishes_mode() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let mut a = pure_agent("x", "1.0.0");
        let mut b = pure_agent("x", "1.0.0");
        b.execution_mode = ExecutionMode::LocalModel {
            model_id: "m".into(),
            model_version: "1.0".into(),
        };
        let ka = cache.derive_key(t, &a, &json!({}));
        let kb = cache.derive_key(t, &b, &json!({}));
        assert_ne!(ka, kb);
        // Mutate a to confirm it doesn't ripple.
        a.description = "z".into();
        assert_eq!(ka, cache.derive_key(t, &a, &json!({})));
    }

    #[tokio::test]
    async fn v2_key_distinguishes_pii_strategy() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let mut a = pure_agent("x", "1.0.0");
        let mut b = pure_agent("x", "1.0.0");
        a.compliance = ComplianceTags {
            pii_redaction_strategy: PiiStrategy::None,
            ..ComplianceTags::default()
        };
        b.compliance = ComplianceTags {
            pii_redaction_strategy: PiiStrategy::Strict,
            ..ComplianceTags::default()
        };
        assert_ne!(
            cache.derive_key(t, &a, &json!({})),
            cache.derive_key(t, &b, &json!({}))
        );
    }

    // ─────────────────────────────────────────────────────────────────
    //  Flush API
    // ─────────────────────────────────────────────────────────────────

    async fn populate_three(cache: &IdempotencyCache) -> (TenantId, TenantId, NanoAgent, NanoAgent) {
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let agent_a = pure_agent("agent_a", "1.0.0");
        let agent_b = pure_agent("agent_b", "1.0.0");
        // 3 entries : (t1, a, in1), (t1, b, in2), (t2, a, in1)
        let k1 = cache.derive_key(t1, &agent_a, &json!({"i": 1}));
        let k2 = cache.derive_key(t1, &agent_b, &json!({"i": 2}));
        let k3 = cache.derive_key(t2, &agent_a, &json!({"i": 1}));
        cache
            .put(k1, fresh_entry(t1, &agent_a, &json!({"i": 1})))
            .await;
        cache
            .put(k2, fresh_entry(t1, &agent_b, &json!({"i": 2})))
            .await;
        cache
            .put(k3, fresh_entry(t2, &agent_a, &json!({"i": 1})))
            .await;
        (t1, t2, agent_a, agent_b)
    }

    #[tokio::test]
    async fn flush_for_tenant_drops_only_that_tenant() {
        let cache = IdempotencyCache::new();
        let (t1, t2, _, _) = populate_three(&cache).await;
        assert_eq!(cache.len(), 3);
        let evicted = cache.flush_for_tenant(t1).await;
        assert_eq!(evicted, 2);
        assert_eq!(cache.len(), 1);
        let snap = cache.metrics().snapshot();
        assert_eq!(snap.flush_total, 1);
        assert_eq!(snap.flush_entries_total, 2);
        // t2 entry survives.
        let _ = t2;
    }

    #[tokio::test]
    async fn flush_for_agent_drops_only_that_agent() {
        let cache = IdempotencyCache::new();
        let (t1, _, agent_a, _) = populate_three(&cache).await;
        let evicted = cache.flush_for_agent(t1, &agent_a.id).await;
        assert_eq!(evicted, 1); // (t1, agent_a, in1) gone ; (t1, agent_b, in2), (t2, agent_a, in1) survive
        assert_eq!(cache.len(), 2);
    }

    #[tokio::test]
    async fn flush_for_input_hash_drops_only_that_entry() {
        let cache = IdempotencyCache::new();
        let t = TenantId::new();
        let a = pure_agent("agent_a", "1.0.0");
        let k1 = cache.derive_key(t, &a, &json!({"i": 1}));
        let k2 = cache.derive_key(t, &a, &json!({"i": 2}));
        let e1 = fresh_entry(t, &a, &json!({"i": 1}));
        let e2 = fresh_entry(t, &a, &json!({"i": 2}));
        let input_hash_1 = e1.input_hash_resolved;
        cache.put(k1, e1).await;
        cache.put(k2, e2).await;
        assert_eq!(cache.len(), 2);
        let evicted = cache
            .flush_for_input_hash(t, &a.id, &a.version, &input_hash_1)
            .await;
        assert_eq!(evicted, 1);
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn flush_older_than_uses_cached_at() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let k = cache.derive_key(t, &a, &json!({"x": 1}));
        cache.put(k, fresh_entry(t, &a, &json!({"x": 1}))).await;
        // Now insert another after a synthetic time skip — we cannot
        // sleep in unit tests reliably, so directly mutate the stored
        // entry's `cached_at` to simulate an older write.
        let earlier = Utc::now() - chrono::Duration::seconds(120);
        let k2 = cache.derive_key(t, &a, &json!({"x": 2}));
        cache.put(k2.clone(), fresh_entry(t, &a, &json!({"x": 2}))).await;
        cache.entries.alter(k2.as_str(), |_, mut e| {
            e.cached_at = earlier;
            e
        });
        let threshold = Utc::now() - chrono::Duration::seconds(60);
        let evicted = cache.flush_older_than(threshold).await;
        assert_eq!(evicted, 1);
    }

    #[tokio::test]
    async fn flush_when_empty_returns_zero_and_counts_call() {
        let cache = IdempotencyCache::new();
        let evicted = cache.flush_for_tenant(TenantId::new()).await;
        assert_eq!(evicted, 0);
        let snap = cache.metrics().snapshot();
        assert_eq!(snap.flush_total, 1);
        assert_eq!(snap.flush_entries_total, 0);
    }

    #[tokio::test]
    async fn flush_for_original_trace_evicts_matching_entries() {
        // Sprint 8 — N12 + N10 GDPR cascade test.
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        // Two entries with different inputs → different trace_ids.
        let k1 = cache.derive_key(t, &a, &json!({"i": 1}));
        let k2 = cache.derive_key(t, &a, &json!({"i": 2}));
        let e1 = fresh_entry(t, &a, &json!({"i": 1}));
        let target_trace = e1.original_trace_id;
        let e2 = fresh_entry(t, &a, &json!({"i": 2}));
        let bystander_trace = e2.original_trace_id;
        assert_ne!(target_trace, bystander_trace);
        cache.put(k1, e1).await;
        cache.put(k2.clone(), e2).await;
        assert_eq!(cache.len(), 2);
        // Cascade triggered : tombstone target_trace.
        let n = cache.flush_for_original_trace(target_trace).await;
        assert_eq!(n, 1);
        assert_eq!(cache.len(), 1);
        // The bystander survives.
        assert!(cache.get(&k2).await.is_some());
    }

    #[tokio::test]
    async fn flush_for_original_trace_unknown_id_is_zero() {
        let cache = IdempotencyCache::new();
        let n = cache
            .flush_for_original_trace(TraceId::new())
            .await;
        assert_eq!(n, 0);
        // But the flush call IS counted.
        assert_eq!(cache.metrics().snapshot().flush_total, 1);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Metrics integrity
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn metrics_size_bytes_decrements_on_remove() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({"x": 1}));
        let entry = fresh_entry(t, &a, &json!({"x": 1}));
        let size = entry.size_bytes;
        cache.put(key.clone(), entry).await;
        assert_eq!(cache.metrics().snapshot().size_bytes, size);
        cache.flush_for_tenant(t).await;
        assert_eq!(cache.metrics().snapshot().size_bytes, 0);
    }

    #[tokio::test]
    async fn metrics_overwrite_replaces_size() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let key = cache.derive_key(t, &a, &json!({"x": 1}));
        // First entry : output {"out": true}  → 13 bytes canonical
        let e1 = fresh_entry(t, &a, &json!({"x": 1}));
        let size1 = e1.size_bytes;
        cache.put(key.clone(), e1).await;
        assert_eq!(cache.metrics().snapshot().size_bytes, size1);
        // Second entry : same key, bigger output
        let mut e2 = fresh_entry(t, &a, &json!({"x": 1}));
        e2.output = json!({"out": true, "extra": "padding-".repeat(100)});
        // Recompute size_bytes manually (CacheEntry::new would, but we
        // mutated the field directly).
        e2.size_bytes = cache_entry::canonical_size_bytes(&e2.output);
        let size2 = e2.size_bytes;
        cache.put(key, e2).await;
        // Size jumps to size2 — overwrite path subtracts size1 then adds size2.
        assert_eq!(cache.metrics().snapshot().size_bytes, size2);
        // Entries count stays at 1.
        assert_eq!(cache.metrics().snapshot().entries_count, 1);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Canonicalizer invariants (Sprint 1 carry-over)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn canonical_handles_nested_object_keys() {
        let v = json!({
            "z": {"b": 1, "a": 2},
            "a": [3, {"y": 4, "x": 5}]
        });
        let c = canonicalize_json(&v);
        assert!(c.starts_with("{\"a\":[3,{\"x\":5,\"y\":4}],\"z\":{\"a\":2,\"b\":1}}"));
    }

    #[test]
    fn canonical_escapes_special_chars() {
        let v = json!({"k": "a\"b\\c\nd"});
        let c = canonicalize_json(&v);
        assert!(c.contains("\\\""));
        assert!(c.contains("\\\\"));
        assert!(c.contains("\\n"));
    }

    #[test]
    fn canonical_array_preserves_order() {
        let c1 = canonicalize_json(&json!([1, 2, 3]));
        let c2 = canonicalize_json(&json!([3, 2, 1]));
        assert_ne!(c1, c2);
    }

    #[test]
    fn canonical_handles_unicode_strings() {
        let v = json!({"name": "Élève — Café ☕"});
        let c = canonicalize_json(&v);
        assert!(c.contains("Élève"));
        assert!(c.contains("☕"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Concurrency
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrent_writes_consistent_with_metrics() {
        let cache = Arc::new(IdempotencyCache::new());
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let mut handles = Vec::new();
        for i in 0..50 {
            let cache = Arc::clone(&cache);
            let a = a.clone();
            handles.push(tokio::spawn(async move {
                let key = cache.derive_key(t, &a, &json!({"i": i}));
                let entry = fresh_entry(t, &a, &json!({"i": i}));
                cache.put(key, entry).await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(cache.len(), 50);
        // Metrics : entries_count tracks DashMap.len.
        assert_eq!(cache.metrics().snapshot().entries_count, 50);
    }

    #[tokio::test]
    async fn derive_key_produces_64_hex_chars() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let k = cache.derive_key(t, &a, &json!({}));
        assert_eq!(k.as_str().len(), 64);
        assert!(k.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn evict_expired_removes_only_expired() {
        let cache = IdempotencyCache::new();
        let t = TenantId::system();
        let a = pure_agent("x", "1.0.0");
        let alive_k = cache.derive_key(t, &a, &json!({"x": "alive"}));
        let dead_k = cache.derive_key(t, &a, &json!({"x": "dead"}));
        cache
            .put(alive_k.clone(), fresh_entry(t, &a, &json!({"x": "alive"})))
            .await;
        cache
            .put(dead_k.clone(), fresh_entry(t, &a, &json!({"x": "dead"})))
            .await;
        // Force-expire the "dead" key by direct mutation.
        cache.entries.alter(dead_k.as_str(), |_, mut e| {
            e.expires_at = Utc::now() - chrono::Duration::seconds(1);
            e
        });
        let evicted = cache.evict_expired().await;
        assert_eq!(evicted, 1);
        assert!(cache.get(&alive_k).await.is_some());
        assert!(cache.get(&dead_k).await.is_none());
        assert_eq!(cache.metrics().snapshot().entries_count, 1);
    }
}
