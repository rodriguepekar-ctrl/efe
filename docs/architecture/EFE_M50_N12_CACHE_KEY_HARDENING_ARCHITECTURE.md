# EFE M50 — Sprint 8 — N12 Cache Key Hardening & Cache-Hit Traceability

**Status**            : DRAFT v1
**Date drafted**      : 2026-05-20
**Owner**             : Rodrigue Pekar (founding engineer) + Senior Platform Engineer #1
**Sprint**            : Sprint 8 (Phase 3 of M50) — sibling to N11
**Predecessor**       : Sprint 7 (N1-N10 closed) + Sprint 8 N11 (Firecracker pool)
**Companion docs**    :
- [`EFE_M50_AGENTIC_ARCHITECTURE.md`](EFE_M50_AGENTIC_ARCHITECTURE.md) (M50 master)
- [`EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md`](EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md) (Sprint 8 sibling)
- [Sprint 7 N1 audit](../audit/Sprint7-DAG-Ancestry.md) (parent_trace_ids — referenced for cache-hit linking)
- [Sprint 7 N4 audit](../audit/Sprint7-Negative-Traces.md) (preflight traces — referenced for cache-hit traces)

**Contrat additif** : This work touches **only** `crates/efe-nano/src/idempotency/` + a 5-line change in `crates/efe-nano/src/dispatcher/executor.rs` to wire the cache-hit trace.  No SDK changes ; no breaking changes to `VmPool`, `Dispatcher`, `ReplayEngine`, `EpochSealer`, or any external trait.  The Sprint 7 925 tests stay green.

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Problem statement — what we discovered](#1-problem-statement--what-we-discovered)
   - 1.1 What is already correct (resolved-version discrimination)
   - 1.2 What is still broken (5 real defects)
2. [Goals & non-goals](#2-goals--non-goals)
3. [Architecture overview](#3-architecture-overview)
4. [Component breakdown — 6 components](#4-component-breakdown--6-components)
5. [The CacheKey v2 algorithm](#5-the-cachekey-v2-algorithm)
6. [CacheEntry v2 — payload with audit metadata](#6-cacheentry-v2--payload-with-audit-metadata)
7. [Cache-hit trace integration](#7-cache-hit-trace-integration--audit-chain-completeness)
8. [Cache invalidation strategies](#8-cache-invalidation-strategies)
9. [LRU eviction + memory cap](#9-lru-eviction--memory-cap)
10. [Mode C special cases](#10-mode-c-special-cases)
11. [Tenant-class partitioning](#11-tenant-class-partitioning)
12. [Concurrency model](#12-concurrency-model)
13. [Failure modes & recovery — 8 paths](#13-failure-modes--recovery--8-paths)
14. [Migration plan — 4-step backward-compatible upgrade](#14-migration-plan--4-step-backward-compatible-upgrade)
15. [Testing strategy — 4 tiers](#15-testing-strategy--4-tiers)
16. [Performance budget](#16-performance-budget)
17. [Sprint 8 day-by-day plan (5 days)](#17-sprint-8-day-by-day-plan-5-days)
18. [Open questions / risks register](#18-open-questions--risks-register)
19. [Decisions log (5 ADRs)](#19-decisions-log-5-adrs)
20. [Annexe A — CacheKey reference impl](#annexe-a--cachekey-reference-impl)
21. [Annexe B — CacheEntry SQL schema (Phase 4)](#annexe-b--cacheentry-sql-schema-phase-4)

---

## 0. Executive summary

EFE Nano's idempotency cache (Sprint 1-4 ship) is a per-process `DashMap<CacheKey, CacheEntry>` with TTL eviction.  Cache hits return cached outputs without re-invoking the runtime — sub-millisecond response for repeat queries with identical inputs.

**Surprise from the architecture audit** : the long-standing concern "the cache keys on `version_req` not on resolved version" turns out to be **false** — the code already passes `&Arc<NanoAgent>.version` (the catalog-resolved version) into `derive_key`.  See § 1.1 for the live evidence.

**But the audit surfaced 5 OTHER defects** that compromise Sprint 8's audit-grade promise (§ 1.2) :

1. **No execution-mode discrimination** — Mode A vs. Mode B agents with the same `(id, version)` collide in the cache.  Today this can't happen because Mode A and Mode B agents have different ids in the starter pack, but it's a latent bug for Phase 4 customer authoring.
2. **No Mode C provider/model discrimination** — same agent v1.0.0 routed to Anthropic vs OpenAI produces different outputs, but the cache key doesn't see the routing decision.
3. **No prompt-template-version discrimination** — bumping the prompt template (a versioned Mode C resource, M21) silently keeps stale cache entries.
4. **No PII-strategy discrimination** — flipping `PiiStrategy::None` ↔ `PreLlmRedact` changes what the LLM sees, but doesn't bust the cache.
5. **Cache hits are INVISIBLE to the audit chain** — no `NanoAgentTrace` is written when the cache short-circuits the runtime, leaving regulatory blind-spots.

**N12 ships a complete cache-key v2 design** : versioned domain prefix, 9-component discriminator chain, audit-grade `CacheEntry` payload (carries `resolved_version` + `original_trace_id` + `cached_at`), cache-hit trace writer that hooks into the Sprint 6 HMAC chain, and an LRU+TTL hybrid evictor.

**Sprint 8 commitment** (5 days, founder + senior platform eng) :

| # | Deliverable | Verification |
|---|-------------|--------------|
| C1 | CacheKey v2 with 9-component discriminator | `cache_key_v2_distinguishes_all_dimensions` (10 tests) GREEN |
| C2 | CacheEntry v2 carrying `resolved_version` + `original_trace_id` | `cache_entry_v2_round_trip` GREEN |
| C3 | Cache-hit trace written to HMAC chain | `cache_hit_writes_audit_trace_linked_to_origin` GREEN |
| C4 | LRU+TTL eviction, configurable cap | `lru_evicts_oldest_under_pressure` GREEN |
| C5 | Mode C provider/model/template discrimination | `mode_c_cache_invalidates_on_provider_swap` GREEN |
| C6 | Backward-compat migration : v1 hash misses cleanly | `v1_keys_miss_v2_cache_after_rotation` GREEN |
| C7 | Cache invalidation API : `flush_for_agent`, `flush_for_tenant` | `flush_api_invalidates_targeted_entries` GREEN |
| C8 | Metrics + render : `cache_hits_total`, `cache_misses_total`, `cache_evictions_total`, `cache_size_bytes` | Grafana dashboard JSON validated |

---

## 1. Problem statement — what we discovered

### 1.1 What is already correct (resolved-version discrimination)

The original N12 hypothesis from the Sprint 7 audit was :

> "The cache keys on `NanoAgentRef.version_req` (e.g. `^1.0`), not on the resolved version (e.g. `1.2.7`).  Publishing `1.2.8` silently serves the cached `1.2.7` output."

**This hypothesis is false** as of Sprint 7 (verified 2026-05-20).

The Sprint 4 `IdempotencyCache::derive_key` signature is :

```rust
pub fn derive_key(
    &self,
    tenant: TenantId,
    agent_id: &NanoAgentId,
    version: &NanoAgentVersion,   // ← takes a VERSION, not a VersionReq
    input: &Value,
) -> String { ... }
```

The dispatcher executor (in `dispatcher/executor.rs`) calls it with :

```rust
let cache_key = idempotency.derive_key(
    tenant_id,
    &handle.agent.id,
    &handle.agent.version,   // ← `handle.agent` is the catalog-resolved Arc<NanoAgent>
    &input,
);
```

Because `handle.agent` is the resolved `Arc<NanoAgent>` (produced by `Catalog::get` which already applies the `version_req` semver matching), the third argument is **already the resolved version**.

**Conclusion** : publishing v1.2.8 of an agent **does** naturally invalidate the cache for v1.2.7 requests, because the cache key on the new request resolves to a different version → different hash → cache miss.

This is good news for our regulatory promise.  But the audit dug deeper and found 5 OTHER cracks below.

### 1.2 What is still broken (5 real defects)

#### Defect 1 : no execution-mode discrimination

Today the cache key is `(tenant, agent_id, version, input)`.  Two different agents at different `execution_mode`s (Mode A `json_validate` and a hypothetical Mode B `json_validate_via_ml`) **happen to** have different `agent_id` strings — so no collision in practice.

But in Phase 4 (customer authoring path, M50 N18 publisher identity), customers can publish their own `json_validate` agents at any execution mode.  If tenant A publishes `json_validate` v1.0.0 Mode A and tenant B publishes `json_validate` v1.0.0 Mode B with the same input schema, the cross-tenant cache (currently per-tenant only) would be fine — but **within a tenant**, an `evolve_catalog` operation that swaps Mode A → Mode B for the same `(id, version)` (semver patch bump with implementation change) would let the old Mode A cached output mask the new Mode B behavior.

**Probability today** : low (operators don't do this).  **Probability Phase 4** : high (customer-authored agents will swap modes).

#### Defect 2 : no Mode C provider/model discrimination

A Mode C agent's `ExecutionMode::LlmAugmented { provider, model, ... }` carries which LLM provider + model the agent expects.  If the operator updates the agent's `model` from `claude-sonnet-4-5` to `claude-haiku-4-5` **without bumping the agent version** (which would be a violation of best-practice but technically allowed by the SDK), the cache returns stale outputs from the Sonnet era.

Even more subtly : the **M21 LLM router** can dynamically reroute to a fallback provider on circuit-breaker trip.  Same agent, same version, but different provider hit the LLM.  Cache hit on subsequent identical call serves whichever provider ran the cold call — could be misleading.

#### Defect 3 : no prompt-template-version discrimination

Mode C agents reference a prompt template by id (e.g. `"m50.summarize_strict.v1"`).  M21 (prompt registry) maintains template versions.  When operators bump a template (`v1 → v2`) for A/B testing, the cache key doesn't see this — same input + same agent + same template_id → cache hit on stale v1 output.

This is **the most insidious** of the 5 defects because prompt-template bumps are FREQUENT (weekly in active development) and the regression is silent : auditors check that traces match the template version, but the cached entries pre-date the bump and serve "before" content from the "after" trace lineage.

#### Defect 4 : no PII-strategy discrimination

`ComplianceTags.pii_strategy` controls whether Mode C input is redacted before the LLM call.  Toggling `None → PreLlmRedact` :
- Changes the effective LLM input (redacted form).
- Changes the LLM output (different completion).
- Should bust the cache.

Today the cache key doesn't include `pii_strategy`, so toggling redaction policy mid-day silently serves un-redacted-era outputs to redacted-era queries.  Worse : the cached output may contain PII that the redacted-era policy explicitly tried to suppress.  **Regulatory violation risk**.

#### Defect 5 : cache hits are invisible to the audit chain

This is the most architecturally significant.  When the cache hits :

```rust
if let Some(cached) = idempotency.get(&cache_key).await {
    outcome.status = OpStatus::Success;
    outcome.output = Some(cached.output);
    outcome.cost_cents = cached.cost_cents;
    outcome.cache_hit = true;
    outcome.latency_ms = 0;
    return outcome;     // ← RETURNS WITHOUT WRITING A TRACE
}
```

The runtime is **never invoked** → no `NanoAgentTrace` written → **the HMAC chain has no record of this call**.

Consequences :
- **Audit gap** : the regulator who replays Tenant T's chain sees N traces.  The customer reports N+M calls served on that day (M of them were cache hits).  Chain integrity is fine, but **provenance is missing**.
- **Replay impossible** : `ReplayEngine` can't verify cache hits because they aren't in the chain.  The replay report shows "N total ops" but should show "N+M".
- **Cost reconciliation broken** : billing meters count cache hits separately (M21 cache layer), but they don't reconcile against the chain → operator can't trust "trace count == metered call count".
- **GDPR scope ambiguous** : a tenant requesting Art. 17 erasure on a date range : the chain has N traces to tombstone.  The cache has M PII-bearing entries that ALSO need erasure.  No link.

**N12 fixes this** by writing a "cache-hit trace" for every cache hit, linked via `parent_trace_ids` to the trace_id of the original cold call.  The chain stays the audit-grade source of truth.

---

## 2. Goals & non-goals

### 2.1 Sprint 8 N12 goals (commitments)

| # | Goal | Acceptance |
|---|------|------------|
| G1 | CacheKey v2 with 9-component discriminator | 10 unit tests for each discriminator dimension |
| G2 | Domain prefix versioning (`v2`) for one-shot rotation | `cache_v1_to_v2_migration_clean_miss` GREEN |
| G3 | CacheEntry v2 carries `resolved_version`, `original_trace_id`, `cached_at`, `mode_c_metadata` | round-trip test GREEN |
| G4 | Cache-hit trace written to HMAC chain | `cache_hit_writes_trace_with_parent_links` GREEN |
| G5 | Cache hit trace's `parent_trace_ids` contains the original trace_id (Sprint 7 N1 invariant) | test GREEN |
| G6 | LRU eviction with configurable per-tenant cap | `lru_evicts_lru_under_pressure` GREEN |
| G7 | Cache invalidation API : `flush_for_agent`, `flush_for_tenant`, `flush_for_input_hash` | tests GREEN |
| G8 | Cache hits respect GDPR tombstones from N10 | integration test : tombstone → cache flushed |
| G9 | 4 Prometheus metrics : `cache_hits_total`, `cache_misses_total`, `cache_evictions_total`, `cache_size_bytes` | exposed via `/metrics` |
| G10 | Backward-compat : v1 cache entries lazily age out (24h TTL) ; v2 cohabits | tested via mixed-version stress |

### 2.2 Sprint 8 N12 non-goals (deferred)

| # | Non-goal | When |
|---|----------|------|
| NG1 | Redis-backed distributed cache (production scale-out) | Phase 4 (M27 wiring) |
| NG2 | Postgres-backed cache mirror for durability | Phase 4 (M27 wiring) |
| NG3 | Cross-region cache replication | Phase 5 |
| NG4 | Cache warming on agent publish (predictive pre-load) | Phase 5 |
| NG5 | Cache compression (zstd of `output` blobs) | Phase 4 |
| NG6 | Probabilistic cache (Bloom filter sidecar) | Phase 5 |
| NG7 | Cross-tenant deduplication (shared output hash across tenants) | OUT OF SCOPE — violates tenant isolation |
| NG8 | Semantic similarity cache (hit on "near-identical" inputs) | OUT OF SCOPE — violates idempotency contract |

---

## 3. Architecture overview

### 3.1 The 3-layer diagram

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Dispatcher::run_one_op                                                   │
│                                                                          │
│   1. derive_key_v2(tenant, agent, mode-c-context, canonical_input)       │
│      ↓                                                                   │
│   2. idempotency.get(key) ── Hit ?                                       │
│      ├── Yes : write_cache_hit_trace(original_trace_id, ...)  ───┐       │
│      │        return cached_output                              │       │
│      └── No  : runtime.execute_with_context(...) ──── trace ────┤       │
│                idempotency.put(key, entry_v2)                   │       │
│                                                                  │       │
│                                                                  ▼       │
└─────────────────────────────────────────────────────────  ┌────────────────┐
                                                            │ HMAC audit chain│
                                                            │  + N9 Merkle    │
                                                            │  + N10 tombstone│
                                                            └────────────────┘
                                                                  ▲
                                                                  │
┌──────────────────────────────────────────────────────────────────┘
│ IdempotencyCache (N12 v2)
│
│  ┌──────────────────────────────────────────────────────────────────┐
│  │ Layer 1 :  CacheKey derivation (9-component)                     │
│  │   SHA256(                                                        │
│  │     DOMAIN_PREFIX                                                │
│  │     ‖ tenant_id (16 B)                                           │
│  │     ‖ agent_id (variable)                                        │
│  │     ‖ resolved_version (variable)                                │
│  │     ‖ execution_mode_discriminant ("pure_rust"|"local_model"|... │
│  │     ‖ mode_c_provider_model (if Mode C, else "")                 │
│  │     ‖ mode_c_prompt_template_id_v (if Mode C, else "")           │
│  │     ‖ pii_strategy_discriminant                                  │
│  │     ‖ canonical_input_json                                       │
│  │   )                                                              │
│  └─────────────────────┬────────────────────────────────────────────┘
│                        ↓
│  ┌──────────────────────────────────────────────────────────────────┐
│  │ Layer 2 :  CacheEntry v2 payload                                 │
│  │   output                                                         │
│  │   cost_cents                                                     │
│  │   cached_at                                                      │
│  │   expires_at                                                     │
│  │   resolved_version           ← NEW (N12)                         │
│  │   original_trace_id          ← NEW (N12, links to chain)         │
│  │   mode_c_metadata (provider, model, tokens, template_version)    │
│  │   size_bytes                 ← NEW (for LRU)                     │
│  └─────────────────────┬────────────────────────────────────────────┘
│                        ↓
│  ┌──────────────────────────────────────────────────────────────────┐
│  │ Layer 3 :  Eviction policy (LRU + TTL hybrid)                    │
│  │   DashMap<CacheKey, CacheEntry>                                  │
│  │   + lru_index : VecDeque<(CacheKey, last_access_at)>             │
│  │   + tenant_size_cap : HashMap<TenantId, max_bytes>               │
│  │   on insert : if over cap → evict LRU                            │
│  │   on get    : touch last_access                                  │
│  │   background TTL sweep every 60s                                 │
│  └──────────────────────────────────────────────────────────────────┘
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Component breakdown — 6 components

### 4.1 `CacheKey` derivation (`idempotency/cache_key.rs`)

**Responsibility** : produce a stable, collision-resistant hash from
the dispatch context.  Pure function, no I/O, no allocations beyond
the hash buffer.

**Domain prefix** :

```rust
pub const CACHE_KEY_DOMAIN_PREFIX: &[u8] = b"efe-nano-idempotency-v2";
```

The `v2` suffix enables one-shot rotation : bumping to `v3` mass-invalidates everything.  Used during incident response or breaking schema changes.

**Function shape** :

```rust
pub fn derive_key_v2(
    tenant: TenantId,
    agent: &NanoAgent,          // resolves to id + version + mode + compliance
    canonical_input: &[u8],     // pre-canonicalized JSON bytes
) -> CacheKey {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_KEY_DOMAIN_PREFIX);
    hasher.update(b"\x00");                                       // domain terminator
    hasher.update(tenant.0.as_bytes());                           // 16 bytes UUID
    hasher.update(agent.id.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(agent.version.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(agent.execution_mode.mode_str().as_bytes());    // "pure_rust" | "local_model" | "llm_augmented"
    hasher.update(b"\x00");
    if let ExecutionMode::LlmAugmented { provider, model, prompt_template_id, .. } = &agent.execution_mode {
        hasher.update(provider.as_bytes());
        hasher.update(b":");
        hasher.update(model.as_bytes());
        hasher.update(b"\x00");
        hasher.update(prompt_template_id.as_bytes());
    }
    hasher.update(b"\x00");
    hasher.update(agent.compliance.pii_strategy.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(canonical_input);
    CacheKey(hex_encode(hasher.finalize().into()))
}
```

**Why null-byte terminators between components** : prevents collision-by-concatenation (where two distinct `(id, version)` pairs happen to share a flattened byte sequence with another two).  Same defense as the HMAC chain's domain separation in N9 Merkle.

**Why hex-encode the final digest** : the key is used as a `HashMap` key (cloned + compared by string).  Hex is ASCII-safe, comparable, and avoids the `[u8; 32]` borrow-checker friction.

### 4.2 `CacheEntry` v2 (`idempotency/cache_entry.rs`)

```rust
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub output: serde_json::Value,
    pub cost_cents: f64,
    pub cached_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,

    // ── N12 NEW FIELDS ──
    /// The agent version that produced this output.  Returned on cache
    /// hit so the cache-hit trace records correct provenance.
    pub resolved_version: NanoAgentVersion,

    /// The trace_id of the original (cold) call.  The cache-hit trace
    /// puts this in `parent_trace_ids` so the audit chain can reconstruct
    /// the cache hit's lineage back to the source.
    pub original_trace_id: TraceId,

    /// LLM-side metadata for Mode C entries (Some), None for Mode A/B.
    pub mode_c_metadata: Option<LlmCacheMetadata>,

    /// Approximate memory footprint (UTF-8 byte count of `output`).
    /// Used by the LRU evictor — recomputed on each insert, never
    /// updated after.
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct LlmCacheMetadata {
    pub provider: String,
    pub model: String,
    pub prompt_template_id: String,
    pub prompt_template_version: u32,
    pub tokens_used: u32,
    pub llm_cost_cents: f64,
}
```

### 4.3 Cache-hit trace writer

A new internal helper invoked by the dispatcher on cache hit :

```rust
async fn write_cache_hit_trace(
    audit: &Arc<dyn AuditWriter>,
    tenant_id: TenantId,
    swarm_request_id: Uuid,
    op_id: &str,
    caller_agent_id: Option<String>,
    agent: &NanoAgent,
    canonical_input: &[u8],
    entry: &CacheEntry,
) {
    let now = Utc::now();
    let trace = NanoAgentTrace {
        trace_id: TraceId::new(),
        // N1 invariant : link via parent_trace_ids
        parent_trace_ids: vec![entry.original_trace_id],
        swarm_request_id,
        operation_id: op_id.to_string(),
        tenant_id,
        caller_agent_id,
        nano_agent_id: agent.id.clone(),
        nano_agent_version: entry.resolved_version.clone(),
        execution_mode: agent.execution_mode.mode_str().to_string(),
        // N6 invariant : both source and resolved hashes
        input_hash_source: hash_bytes(canonical_input),
        input_hash_resolved: hash_bytes(canonical_input),
        output_hash: Some(hash_json(&entry.output)),
        // N12 NEW : a dedicated sentinel error_kind marks this as cache-hit
        error_kind: Some("CacheHit".to_string()),
        started_at: now,
        completed_at: Some(now),
        latency_ms: Some(0),
        cost_cents: entry.cost_cents,
        llm_tokens_used: entry.mode_c_metadata.as_ref().map(|m| m.tokens_used),
        full_input: None,    // already on the original trace
        full_output: None,   // already on the original trace
        tombstone: None,
    };
    audit.record(trace).await;
}
```

**Three subtle invariants of this trace** :

1. **`error_kind: Some("CacheHit")`** — not actually an error, but the sentinel value is the unambiguous discriminator for the auditor.  Sprint 9 may promote this to a dedicated `trace_kind: TraceKind::CacheHit | TraceKind::Fresh | ...` enum.

2. **`full_input` / `full_output` = `None`** — the cache hit trace is **pointer-shaped**, not data-shaped.  The full bodies live on the original trace (linked via `parent_trace_ids`).  This prevents PII duplication and reduces chain size.

3. **`parent_trace_ids` = `[original_trace_id]`** — the chain link.  Auditor walking the chain sees the cache hit, follows the parent, finds the cold-call trace, can replay the cold call.

### 4.4 Cache invalidation API

```rust
impl IdempotencyCache {
    /// Flush all entries for a specific agent (id + any version) within
    /// a tenant.  Used when operators republish or rollback an agent.
    pub async fn flush_for_agent(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
    ) -> usize {
        // ... walks DashMap, retains only entries whose key doesn't
        // match (tenant, agent_id, _, _).  Returns count evicted.
    }

    /// Flush all entries for a tenant.  Used on tenant offboarding +
    /// GDPR cascade.
    pub async fn flush_for_tenant(&self, tenant: TenantId) -> usize { ... }

    /// Flush a specific (tenant, agent, version, input_hash) entry.
    /// Used by the drift detector (M24) when a specific cached output
    /// is suspected of being staleness-poisoned.
    pub async fn flush_for_input_hash(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version: &NanoAgentVersion,
        input_hash: &[u8; 32],
    ) -> usize { ... }

    /// Flush everything older than a threshold.  Used during cache
    /// version rotation (v2 → v3).
    pub async fn flush_older_than(&self, threshold: DateTime<Utc>) -> usize { ... }
}
```

### 4.5 LRU evictor

```rust
struct LruIndex {
    /// Per-tenant LRU queue, ordered oldest-first.
    queues: HashMap<TenantId, VecDeque<(CacheKey, Instant)>>,
    /// Per-tenant total size in bytes.
    sizes: HashMap<TenantId, u64>,
    /// Per-tenant cap.  None ⇒ no cap.
    caps: HashMap<TenantId, u64>,
    /// Default cap applied when no explicit cap is set.
    default_cap_bytes: u64,
}

impl LruIndex {
    fn record_insert(&mut self, tenant: TenantId, key: &CacheKey, size: u64) -> Vec<CacheKey> {
        // Push to queue, increment size.  If over cap, pop LRU until
        // under cap.  Return evicted keys for the caller to delete
        // from the DashMap.
    }

    fn record_get(&mut self, tenant: TenantId, key: &CacheKey) {
        // Move to back of queue (touch).
    }
}
```

The LRU index is protected by a `Mutex<LruIndex>` — write-only on insert + touch on get.  The hot read path (cache hit on already-touched entry) only updates a `last_access_at` Instant inside the entry, not the LRU queue, until the TTL sweep visits it.

### 4.6 Metrics

```rust
pub struct CacheMetrics {
    pub hits_total: Counter,
    pub misses_total: Counter,
    pub evictions_total: Counter,
    pub size_bytes: Gauge,         // current
    pub entries_count: Gauge,      // current
    pub put_duration: Histogram,
    pub get_duration: Histogram,
    pub flush_total: Counter,
    pub flush_entries_total: Counter,
}
```

Per-tenant labels via `LabelledMetrics<TenantClass>`.

---

## 5. The CacheKey v2 algorithm

### 5.1 Wire format

The key is a hex-encoded SHA-256 digest of the byte sequence :

```text
"efe-nano-idempotency-v2"
\x00
<tenant_id : 16 bytes raw UUID>
<agent_id : UTF-8 bytes, no null inside (NanoAgentId validator enforces)>
\x00
<resolved_version : UTF-8 bytes, no null inside (semver validator enforces)>
\x00
<execution_mode : "pure_rust" | "local_model" | "llm_augmented">
\x00
{ if Mode C : <provider>:<model>\x00<prompt_template_id> } else <empty>
\x00
<pii_strategy : "none" | "pre_llm_redact" | "strict">
\x00
<canonical_input_json bytes>
```

Output : 64-char lowercase hex string.

### 5.2 Canonicalization rules

Input JSON is canonicalized via the existing `crate::idempotency::canonicalize_json` :
- Object keys sorted alphabetically (recursive).
- Whitespace removed.
- Numbers normalized (e.g. `1.0` → `1`).
- Booleans, strings, null preserved as-is.

This guarantees that `{"a":1, "b":2}` and `{"b":2, "a":1}` produce the **same** cache key.

### 5.3 Collision analysis

For two distinct dispatch contexts to collide, all 9 components must agree.  Conservative collision probability with SHA-256 (256-bit output) at 10⁹ cache entries per tenant per year :

```
P(collision) ≈ N² / 2^257
           ≈ (10⁹)² / 2^257
           ≈ 10^18 / 10^77
           ≈ 10^-59
```

Universe-age scale.  **Not a practical concern.**

### 5.4 Why not include `cost_estimate_cents` ?

Because cost estimate is a property of the call, not its identity.  Two calls with same input but different operator-set cost ceilings should produce the same output (the cost cap might fire mid-execution, but the input + agent define the deterministic outcome).

### 5.5 Why include `pii_strategy` but not `audit.include_input` ?

`pii_strategy` changes what the LLM sees (redacted vs. original) → different outputs.  `audit.include_input` only changes what's stored in the trace, not what the agent receives → same output.

---

## 6. CacheEntry v2 — payload with audit metadata

### 6.1 Serialization

`CacheEntry` is **NOT** persisted to disk in Sprint 8 (in-process DashMap only).  Phase 4 wires Postgres + Redis with JSON serialization (see Annexe B).

### 6.2 Memory layout

For a 64-bit platform, the struct's stack size is approximately :

```
output         : Box<Value>          → 8 bytes pointer + heap-allocated tree
cost_cents     : f64                 → 8 bytes
cached_at      : DateTime<Utc>       → 12 bytes
expires_at     : DateTime<Utc>       → 12 bytes
resolved_version : NanoAgentVersion  → 24 bytes (String inside)
original_trace_id : TraceId          → 16 bytes (Uuid)
mode_c_metadata : Option<...>        → 1 byte tag + 96 bytes when Some
size_bytes     : u64                 → 8 bytes
                                       ──────
                                       ~ 185 bytes + output heap
```

At 1 KB average output size, 1 million entries × 1.2 KB total ≈ **1.2 GB**.  This is the basis for the LRU cap.

### 6.3 Update semantics

`CacheEntry` is **immutable after insert**.  No "update in place" — TTL expiry and LRU eviction remove + insert if needed.  This simplifies concurrency (no read-modify-write races).

Exception : `last_access_at` is updated on `get` for LRU bookkeeping.  This field is held in the `LruIndex`, not in `CacheEntry`, precisely to keep `CacheEntry` immutable.

---

## 7. Cache-hit trace integration — audit chain completeness

### 7.1 The lifecycle of a cache hit

```text
T0 :  Op A executes cold.
      ↓
      runtime.execute_with_context → trace_A in HMAC chain.
      ↓
      idempotency.put(key, entry { original_trace_id: trace_A, ... })

T1 :  Op B with identical (tenant, agent, version, ..., input) submitted.
      ↓
      idempotency.get(key) hits.
      ↓
      write_cache_hit_trace(parent_trace_ids: [trace_A], error_kind: "CacheHit")
      ↓
      trace_B appended to chain, links back to trace_A.
      ↓
      return entry.output to caller.
```

### 7.2 The auditor's view

A regulator walks the tenant's chain and sees :

```
trace_1   nano=json_validate@1.0  caller=claude-sonnet  cost=0.001  parent=[]
trace_2   nano=json_validate@1.0  caller=claude-sonnet  cost=0.001  parent=[trace_1]  error_kind=CacheHit
trace_3   nano=ner_clinical@1.0   caller=gpt-4o         cost=0.040  parent=[]
trace_4   nano=ner_clinical@1.0   caller=gpt-4o         cost=0.040  parent=[trace_3]  error_kind=CacheHit
trace_5   nano=ner_clinical@1.0   caller=gpt-4o         cost=0.040  parent=[trace_3]  error_kind=CacheHit
...
```

The auditor immediately understands : trace_2 was a cache hit linking back to trace_1.  Full provenance.

### 7.3 Replay implications

`ReplayEngine` (Sprint 6 ship + Sprint 7 hardening) walks the chain.  When it encounters a cache-hit trace (`error_kind == "CacheHit"`), it has two options :

1. **Verbatim replay** : skip the runtime execute, instead consult the original trace (via `parent_trace_ids[0]`) and use its `output_hash`.  Fast.
2. **Recomputation replay** : ignore the cache, re-execute the agent on the input.  Slow but exercises determinism.

Sprint 8 ships option 1 (verbatim).  Sprint 9 may add an `--exhaustive` flag to the replay CLI for option 2.

### 7.4 GDPR tombstone interaction (N10)

If the original trace gets tombstoned (e.g. tenant requests Art. 17 erasure on the underlying input), all cache-hit traces that link to it should ALSO be tombstoned.  Sprint 8 N12 ships this cascade :

```rust
async fn tombstone_cascade_for_origin(
    chain_store: &dyn ChainStore,
    cache: &IdempotencyCache,
    tenant: TenantId,
    origin_trace_id: TraceId,
    metadata: TombstoneMetadata,
) -> Result<usize, ChainStoreError> {
    // 1. Find all cache-hit traces with origin_trace_id in parent_trace_ids.
    // 2. Tombstone each (preserves chain integrity per N10).
    // 3. Flush corresponding cache entries (so future Art. 17 audit shows no residual).
    // Returns total tombstones applied.
}
```

---

## 8. Cache invalidation strategies

| Trigger | Strategy | API |
|---------|----------|-----|
| TTL expiry | Background sweep every 60s | (automatic) |
| Memory pressure | LRU eviction on insert | (automatic) |
| Agent republish (new version) | Natural via cache key change | (no action needed) |
| Agent rollback (revert to v_old) | Lazy : v_old's old entries expire by TTL ; new calls with v_old generate fresh entries | (no action needed) |
| Operator-initiated flush | Explicit | `flush_for_agent`, `flush_for_tenant` |
| Drift detected (M24) | Targeted flush of suspected entry | `flush_for_input_hash` |
| GDPR Art. 17 erasure (N10) | Cascade from chain to cache | `tombstone_cascade_for_origin` |
| Cache schema migration | Mass invalidation via DOMAIN_PREFIX bump | (deploy with new prefix) |

---

## 9. LRU eviction + memory cap

### 9.1 Per-tenant cap

Default cap : **256 MiB per tenant**.  Configurable via env var or per-tenant feature flag :

```yaml
# values.yaml
cache:
  defaultTenantCapBytes: 268435456    # 256 MiB
  perTenantCapOverrides:
    tier-audit-tenant-1: 1073741824   # 1 GiB
    experimental-tenant-7: 67108864   # 64 MiB
```

### 9.2 Eviction algorithm

On insert :
1. Compute `entry.size_bytes`.
2. Acquire LRU mutex.
3. If `current_size + entry.size_bytes > cap` : pop LRU keys until under cap, push each to `evicted_keys` list.
4. Push new entry to back of LRU.
5. Release mutex.
6. For each evicted key : DashMap.remove + metric inc.

### 9.3 Complexity analysis

- Insert : O(log N) amortized (DashMap shard + LRU queue push).
- Get : O(1) amortized (DashMap shard + LRU touch O(1) with linked-list backing).
- TTL sweep : O(N) per pass, every 60s — acceptable for ≤ 10⁶ entries.

For larger scales, Phase 4 swaps the in-process DashMap for Redis with `EXPIRE` (TTL handled by Redis) + `MAXMEMORY allkeys-lru` (LRU handled by Redis).

---

## 10. Mode C special cases

### 10.1 The replay-via-cache pipeline (N3 interaction)

Sprint 7 N3 introduced `DeterminismLevel::LlmReplayable` — Mode C agents whose LLM responses are cached for replay.  This **is the same cache** as the idempotency cache (N12).

Therefore N12's cache key MUST include all Mode C discriminators (provider, model, prompt_template, pii_strategy) — otherwise N3's replay-via-cache breaks : a cached response from `claude-sonnet` would mistakenly satisfy a replay attempt against `claude-haiku`.

### 10.2 Temperature ≠ 0 caveat

If a Mode C agent has `temperature > 0`, LLM outputs are non-deterministic — caching them violates the determinism contract.  Sprint 8 N12 emits a warning at cache write time if `agent.execution_mode.temperature > 0.0` :

```rust
if let ExecutionMode::LlmAugmented { temperature, .. } = &agent.execution_mode {
    if *temperature > 0.001 {
        tracing::warn!(
            agent_id = %agent.id,
            temperature = %temperature,
            "caching a non-deterministic Mode C output (temperature > 0) ; \
             determinism contract is best-effort, not guaranteed"
        );
        metrics.non_deterministic_cache_writes.inc();
    }
}
```

### 10.3 Streaming responses (NOT supported)

Mode C agents that stream tokens (M21 `ExecuteStreaming` RPC) cannot be cached in their streaming form — caching is per-complete-response only.  Sprint 8 N12 panics if a streaming response reaches `put()` (the dispatcher must not call put for streams).

---

## 11. Tenant-class partitioning

The cache is **per-tenant** by construction (tenant_id in the key).  N11 introduced `TenantClass` for VM pool partitioning ; should the cache use it too ?

**Decision** : NO.  The cache is read-mostly, latency-critical, and small per-entry.  Per-tenant-class partitioning would 3x the memory footprint with no SLA benefit.  Per-tenant caps (§ 9) already prevent noisy-neighbor.

---

## 12. Concurrency model

### 12.1 Lock hierarchy

1. `DashMap<CacheKey, CacheEntry>` shards : sharded locks (no global).
2. `LruIndex` : `Mutex<LruIndex>` — held only during insert/touch, microsecond critical sections.
3. Background TTL sweep : holds DashMap shard locks one at a time.

### 12.2 The get-and-touch race

If two threads simultaneously `get(key)` the same entry, the LRU's "move to back" must serialize.  The Mutex acquisition is the bottleneck.

Mitigation : `tokio::sync::Mutex<LruIndex>` (async-friendly, doesn't block the runtime).  Touch can be deferred via a `mpsc::UnboundedSender<TouchEvent>` channel batched every 100 ms.  Sprint 8 ships eager touch ; Phase 4 may add batched touch if needed.

### 12.3 Cache-hit trace write race

The dispatcher reads `cache.get(key)`, then `audit.record(trace)`.  These are independent — no race between cache and trace.  BUT : if cache-hit trace write fails (e.g. HMAC chain key issue), the customer receives the cached output without an audit trace.  This is acceptable behavior (chain writer is fail-open per Sprint 6) but should be metered : `cache_hit_trace_write_failure_total`.

---

## 13. Failure modes & recovery — 8 paths

| # | Failure | Detection | Recovery | User-visible |
|---|---------|-----------|----------|--------------|
| F1 | DashMap insert fails (allocator OOM) | `try_insert` returns Err | Skip cache, complete the call normally ; log + metric | (silent) |
| F2 | LRU mutex poisoned (panic mid-update) | `lock()` returns Err | Reset LRU index, mass-flush cache | (silent) |
| F3 | Cache-hit trace write fails | `audit.record` returns Err (Sprint 6 fail-open : actually never errors at this layer, but the AuditChainError propagates as a log) | Customer still gets cached output ; audit gap logged | (silent) |
| F4 | TTL sweep panics | tokio task JoinError | Restart sweep task ; log SEV-2 | (silent) |
| F5 | Tenant cap exceeded by single huge output (> cap) | Insert evicts everything else and STILL fails | Skip cache, complete normally ; log SEV-3 | (silent) |
| F6 | Cross-tenant cache leak (impossible by design) | N/A | N/A | (impossible) |
| F7 | Memory cap evicts an entry referenced by an in-flight replay | LRU is unaware of replay engine | Replay sees ReplayWindowExpired (Sprint 7 N3) | `ReplayOpStatus::ReplayWindowExpired` |
| F8 | Domain prefix rotation mid-deployment | Mixed v1+v2 keys cohabit during rollout | v1 keys naturally age out by TTL ; v2 path takes over | (silent — clean cutover) |

---

## 14. Migration plan — 4-step backward-compatible upgrade

### Step 1 (day 1) — ship CacheKey v2 alongside v1

Both `derive_key_v1` (old, kept for read) and `derive_key_v2` (new, used for write) are wired.  Dispatcher writes only v2 keys ; reads check v2 first, fall back to v1 lookup (so old entries still serve hits).

### Step 2 (day 2) — ship CacheEntry v2

`CacheEntry::resolved_version`, `original_trace_id`, etc. fields added.  Old v1 entries (without these fields) are wrapped in `Option<CacheEntry>` and surface `None` for new fields — cache-hit trace path uses sensible defaults (`resolved_version: agent.version`, `original_trace_id: TraceId::nil()`) which the auditor sees as "pre-N12 legacy".

### Step 3 (day 3-4) — ship LRU + invalidation API

LRU index added.  Flush APIs published.  Per-tenant caps configured via Helm values.

### Step 4 (day 5) — wire cache-hit trace

The dispatcher calls `write_cache_hit_trace` on every cache hit.  This is the regulatory closure : audit chain completeness restored.

### Rollback

`EFE_NANO_CACHE_VERSION=v1` env var disables v2 entirely.  Customer rollback path under 30 seconds.

---

## 15. Testing strategy — 4 tiers

### Tier 1 — Unit tests (pure logic, no I/O)

- `cache_key_v2_distinguishes_resolved_version` : v1.0.0 vs v1.0.1 → different keys.
- `cache_key_v2_distinguishes_execution_mode` : Mode A vs Mode B → different keys.
- `cache_key_v2_distinguishes_provider_model` : Anthropic vs OpenAI → different keys.
- `cache_key_v2_distinguishes_prompt_template_id` : v1 vs v2 → different keys.
- `cache_key_v2_distinguishes_pii_strategy` : None vs PreLlmRedact → different keys.
- `cache_key_v2_canonicalizes_input_order` : `{"a":1,"b":2}` ≡ `{"b":2,"a":1}` → same key.
- `cache_key_v2_distinguishes_tenant` : tenant A vs tenant B → different keys.
- `cache_key_v2_domain_prefix_rotates_all` : v2 prefix vs v3 prefix → different keys.
- `cache_entry_v2_carries_resolved_version`
- `cache_entry_v2_round_trip`

### Tier 2 — Integration tests (with dispatcher + mock chain)

- `cache_hit_writes_trace_to_audit_chain` : verifies the trace appears in the chain.
- `cache_hit_trace_links_to_origin_via_parent_trace_ids` : verifies N1 invariant.
- `cache_hit_trace_has_error_kind_CacheHit` : verifies discriminator.
- `cache_hit_returns_same_cost_as_origin` : billing consistency.
- `cache_miss_after_agent_republish_resolves_to_new_version` : the "non-bug" verification.
- `flush_for_agent_evicts_only_targeted_tenant_agent`
- `flush_for_tenant_evicts_all`
- `tombstone_cascade_invalidates_cache`

### Tier 3 — Stress tests

- `lru_evicts_lru_under_pressure` : fill cap, verify oldest pop.
- `concurrent_get_and_put_no_corruption` : 16 threads × 10k ops.
- `concurrent_lru_touch_doesnt_deadlock` : multi-threaded get on same key.

### Tier 4 — End-to-end (dispatcher + chain + replay)

- `cache_hit_then_replay_classifies_as_match` : Sprint 7 N3 verification — `LlmReplayable` agent's replay through the cache.
- `cache_invalidation_propagates_to_replay` : flush + replay → MissingFullInput.

---

## 16. Performance budget

### 16.1 Sprint 8 SLO targets

| SLO | Target | Verification |
|-----|--------|--------------|
| `cache.get(key)` p50 | ≤ 5 µs | bench |
| `cache.get(key)` p99 | ≤ 50 µs | bench |
| `cache.put(key, entry)` p99 | ≤ 100 µs | bench |
| `derive_key_v2` p99 | ≤ 10 µs (SHA-256 of ~ 1 KB input) | bench |
| Cache-hit overhead (vs Sprint 4 baseline) | ≤ + 20 µs (trace write) | bench comparison |
| Cache size at 1M entries | ≤ 200 MB | memprofile |
| TTL sweep duration at 1M entries | ≤ 1 s p99 | bench |
| LRU eviction p99 (1 entry) | ≤ 50 µs | bench |

### 16.2 Cache hit ratio

Target steady-state hit ratio per tenant : **30-70 %** for typical Mode A agents, **5-15 %** for Mode C (LLM outputs are unique inputs more often).

Below 10 % hit ratio Mode A : either input distribution is too varied OR cache cap too small.  Operator alert.

---

## 17. Sprint 8 day-by-day plan (5 days)

| Day | Owner | Deliverable | Verification |
|-----|-------|-------------|--------------|
| 1 | founder | `cache_key.rs` v2 derivation + 10 unit tests | Tier 1 GREEN |
| 2 | senior eng | `cache_entry.rs` v2 + LruIndex + invalidation API | Tier 1 GREEN |
| 3 | senior eng | Migrate `idempotency/mod.rs` to use v2 + dual-read v1 | Tier 1+2 GREEN |
| 4 | founder | Wire `write_cache_hit_trace` in dispatcher + cascade with N10 | Tier 2 GREEN |
| 5 | both | Tier 3 stress + Tier 4 E2E + soak | full suite GREEN |

---

## 18. Open questions / risks register

| # | Risk | Likely | Impact | Mitigation | Owner |
|---|------|--------|--------|------------|-------|
| R1 | Customer relies on Sprint 4 cache key shape (unlikely : internal API) | Low | Medium | Dual-read v1 for 1 sprint | founder |
| R2 | LRU mutex contention at high RPS | Medium | Medium | Async mutex + batched touch (Phase 4 ready) | senior eng |
| R3 | Cache-hit trace write overhead exceeds 20µs budget | Low | Medium | Move to fire-and-forget via channel (Sprint 9) | founder |
| R4 | Tenant cap eviction thrashes under realistic workload | Medium | Medium | Per-tenant tunable cap + ops dashboards | SRE lead |
| R5 | GDPR tombstone cascade fails | Low | High | Idempotent cascade + retry queue | compliance lead |
| R6 | Cache breaks N3 LlmReplayable replay | Low | Critical | Tier 4 E2E test covers this explicitly | founder |
| R7 | Domain prefix bump on hot-deploy causes 100% miss spike | Medium | Medium | Documented in runbook ; deploy during off-peak | SRE lead |

---

## 19. Decisions log (5 ADRs)

### ADR-N12-001 : One cache instance or per-tenant ?

**Context** : Sprint 4 ships one DashMap shared across tenants ; per-tenant isolation is via the key (tenant_id in hash).

**Decision** : **Keep one shared DashMap** with per-tenant LRU partitioning.

**Consequences** :
- ✅ Memory locality good (hot keys share L2/L3).
- ✅ Single mutex on LRU index — no per-tenant lock proliferation.
- ⚠️ Noisy neighbor risk — mitigated by per-tenant cap.

### ADR-N12-002 : Cache-hit trace vs. metric-only

**Context** : Sprint 4 records cache hits via a metric (`cache_hits_total`).  Adding a trace per hit doubles chain volume for the steady-state case where hits dominate.

**Decision** : **Write a trace for every cache hit**.

**Consequences** :
- ✅ Audit chain becomes complete : every customer call has a trace.
- ✅ Replay can reconstruct the cache hit's lineage.
- ⚠️ Chain volume up by 2-3× for high-hit-ratio tenants — mitigated by `full_input/output = None` on cache-hit traces (no body bloat).

**Alternatives rejected** :
- Metric-only : audit gap remains.
- Compact "delta" trace (only diff vs origin) : complex, low value.

### ADR-N12-003 : Domain prefix in key vs. sidecar version table

**Context** : Cache schema rotation needs a way to mass-invalidate without enumerating entries.

**Decision** : **Embed version in domain prefix** (`"efe-nano-idempotency-v2"`).

**Consequences** :
- ✅ Bumping the prefix → all keys differ → natural mass-miss.
- ✅ Backward compat : v1 entries can be read via dual-derive during migration.
- ⚠️ Hot-deploy spike : 100% miss for ~ 60s.  Documented + scheduled.

### ADR-N12-004 : Memory cap default

**Context** : What's a "reasonable" default per-tenant cache cap ?

**Decision** : **256 MiB per tenant**, configurable.

**Reasoning** :
- Mid-tier customer : 100k unique cached outputs × 2 KiB avg = 200 MiB.
- TIER-AUDIT customers needing 1M entries get explicit override.
- Holds at ≤ 10 % of typical EFE Nano pod RAM (8 GiB cap).

### ADR-N12-005 : Mode C non-deterministic caching (temperature > 0)

**Context** : Mode C agents with `temperature > 0` produce non-deterministic outputs.  Should we cache them ?

**Decision** : **Yes, but with a `non_deterministic_cache_writes` metric and a warning log**.

**Reasoning** :
- Some operators want even non-deterministic outputs cached for cost reasons (LLM cost > inconsistency).
- Customers who want strict determinism set `temperature = 0.0` → cache is deterministic.
- The warning + metric let ops detect the regression : "hey, you're caching a stochastic model — outputs may differ across replays".

---

## Annexe A — CacheKey reference impl

```rust
// crates/efe-nano/src/idempotency/cache_key.rs

use efe_nano_sdk::{NanoAgent, ExecutionMode, TenantId, NanoAgentVersion};
use sha2::{Digest, Sha256};

pub const CACHE_KEY_DOMAIN_PREFIX: &[u8] = b"efe-nano-idempotency-v2";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(pub String);

impl CacheKey {
    pub fn as_str(&self) -> &str { &self.0 }
}

pub fn derive_key_v2(
    tenant: TenantId,
    agent: &NanoAgent,
    canonical_input: &[u8],
) -> CacheKey {
    let mut h = Sha256::new();
    h.update(CACHE_KEY_DOMAIN_PREFIX);
    h.update(b"\x00");
    h.update(tenant.0.as_bytes());
    h.update(agent.id.as_str().as_bytes());
    h.update(b"\x00");
    h.update(agent.version.as_str().as_bytes());
    h.update(b"\x00");
    h.update(agent.execution_mode.mode_str().as_bytes());
    h.update(b"\x00");
    if let ExecutionMode::LlmAugmented {
        provider,
        model,
        prompt_template_id,
        ..
    } = &agent.execution_mode {
        h.update(provider.as_bytes());
        h.update(b":");
        h.update(model.as_bytes());
        h.update(b"\x00");
        h.update(prompt_template_id.as_bytes());
    }
    h.update(b"\x00");
    h.update(agent.compliance.pii_strategy.as_str().as_bytes());
    h.update(b"\x00");
    h.update(canonical_input);

    let digest = h.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").unwrap();
    }
    CacheKey(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_a() -> NanoAgent { /* ... */ }
    fn sample_agent_b_different_version() -> NanoAgent { /* ... */ }

    #[test]
    fn distinguishes_resolved_version() {
        let t = TenantId::system();
        let i = b"{}";
        let k1 = derive_key_v2(t, &sample_agent_a(), i);
        let k2 = derive_key_v2(t, &sample_agent_b_different_version(), i);
        assert_ne!(k1, k2);
    }
    // ... 9 more discrimination tests
}
```

---

## Annexe B — CacheEntry SQL schema (Phase 4)

```sql
-- Phase 4 deployment (Redis primary, Postgres mirror for durability).
-- Sprint 8 N12 ships in-process only ; this schema is for Phase 4 wiring.

CREATE TABLE idempotency_cache (
    cache_key            TEXT         NOT NULL,          -- 64-char hex SHA-256
    tenant_id            UUID         NOT NULL,
    agent_id             TEXT         NOT NULL,
    resolved_version     TEXT         NOT NULL,
    output_json          JSONB        NOT NULL,
    cost_cents           DOUBLE PRECISION NOT NULL,
    cached_at            TIMESTAMPTZ  NOT NULL,
    expires_at           TIMESTAMPTZ  NOT NULL,
    original_trace_id    UUID         NOT NULL,
    mode_c_metadata      JSONB        NULL,
    size_bytes           BIGINT       NOT NULL,
    PRIMARY KEY (tenant_id, cache_key)
);

CREATE INDEX idempotency_cache_expires_at ON idempotency_cache (expires_at);
CREATE INDEX idempotency_cache_tenant_agent ON idempotency_cache (tenant_id, agent_id);

-- RLS for tenant isolation.
ALTER TABLE idempotency_cache ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON idempotency_cache
  USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));
```

---

## Sign-off blockers

- [ ] Founder review (rodrigue.pekar)
- [ ] Senior platform eng review
- [ ] Audit/compliance review (cache-hit trace + Art. 17 cascade)
- [ ] SRE review (cap defaults + rotation runbook)

**Sprint 8 N12 success criteria** : 10 goals (G1-G10) GREEN + Tier 4 E2E passes against the live FirecrackerVmPool from N11.

**Sprint 9 success criteria for N12** : 7-day soak with cache hit ratio ≥ 30 % on Mode A traffic ; zero `cache_hit_trace_write_failure_total` events ; zero N10 tombstone cascade misses.
