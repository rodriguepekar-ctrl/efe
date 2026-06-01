# EFE M50 — N13 Shadow / Canary Catalog Architecture

> **Status** — DRAFT (Sprint 8, target ship cycle T+10d)
> **Owners** — founder (architecture), senior eng (implementation)
> **Companion docs** —
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8 N11)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8 N12, just shipped)
> * `EFE_M50_PHASE_2_ARCHITECTURE.md` (Phase 2 panorama)
> **Sister deliverable** — `EFE_M50_N14_CONTINUOUS_DRIFT_AUDIT_ARCHITECTURE.md` (Sprint 9, planned)

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Problem statement — what we're solving](#1-problem-statement--what-were-solving)
2. [Goals & non-goals](#2-goals--non-goals)
3. [Architecture overview](#3-architecture-overview)
4. [Component breakdown — 6 components](#4-component-breakdown--6-components)
5. [`RolloutPolicy` — the on-disk source of truth](#5-rolloutpolicy--the-on-disk-source-of-truth)
6. [Sticky routing algorithm](#6-sticky-routing-algorithm)
7. [Output comparison strategies](#7-output-comparison-strategies)
8. [Shadow + canary trace lineage](#8-shadow--canary-trace-lineage)
9. [Cost accounting model](#9-cost-accounting-model)
10. [Promotion workflow](#10-promotion-workflow)
11. [Rollback workflow — manual + automatic](#11-rollback-workflow--manual--automatic)
12. [Idempotency cache interaction (N12 alignment)](#12-idempotency-cache-interaction-n12-alignment)
13. [Replay engine interaction](#13-replay-engine-interaction)
14. [GDPR tombstone cascade (N10 alignment)](#14-gdpr-tombstone-cascade-n10-alignment)
15. [Mode A / B / C specifics](#15-mode-a--b--c-specifics)
16. [Customer authoring (Phase 4)](#16-customer-authoring-phase-4)
17. [Tenant isolation + RBAC](#17-tenant-isolation--rbac)
18. [Failure modes — F1 to F12](#18-failure-modes--f1-to-f12)
19. [Invariants — I1 to I16](#19-invariants--i1-to-i16)
20. [Metrics — 9 Prometheus surfaces](#20-metrics--9-prometheus-surfaces)
21. [Implementation plan — 5-day breakdown](#21-implementation-plan--5-day-breakdown)
22. [Test matrix](#22-test-matrix)
23. [Migration path](#23-migration-path)
24. [Open questions / ADRs](#24-open-questions--adrs)
25. [Annexe A — full type definitions](#25-annexe-a--full-type-definitions)
26. [Annexe B — Postgres schema (Phase 4)](#26-annexe-b--postgres-schema-phase-4)
27. [Annexe C — customer-authoring MCP tools](#27-annexe-c--customer-authoring-mcp-tools)
28. [Annexe D — sample drift incident playbook](#28-annexe-d--sample-drift-incident-playbook)

---

## 0. Executive summary

EFE Nano's catalog (Sprint 1-7 ship) treats agent versioning as a flat operator-driven cut-over : publish v_new, and 100 % of traffic for `(tenant, agent_id)` instantly switches.  That model works for handler-only changes inside the founder's own runtime — it does **not** scale to :

1. **Mode B / Mode C** rollouts, where new model weights or new prompt templates can subtly regress only on edge-case inputs.
2. **Phase 4 customer-authored agents** (M50.4), where tenants publish their own versions — they MUST be able to deploy a v2 progressively and roll back if it misbehaves, without operator intervention.
3. **Cross-mode migrations** (e.g. an agent moving from Mode A to Mode B), where the cache key segregates via N12 but no quality gate compares output parity.

**N13 ships a complete shadow + canary rollout layer**, sitting between the catalog and the dispatcher.  The five capabilities :

1. **`RolloutPolicy`** — typed enum (`Stable | Canary | Shadow`) stored per `(tenant, agent_id)` in the catalog, atomically updatable, with audit row on every change.
2. **Sticky routing** — deterministic `sticky_sample(tenant, agent_id, input_hash) → bps ∈ [0, 10 000)` so the same input always hits the same variant.  Same input → same cache key → idempotent.
3. **Async shadow scheduler** — Mode C–safe : production response returned unblocked, shadow run scheduled `tokio::spawn`-style, output compared in the background, never blocking the caller.
4. **DriftWindow + promotion gates** — bounded sliding window of shadow outcomes per `(tenant, agent_id)`, evaluated against operator-configured `PromotionGates` (sample_count, drift_bps, error_rate, latency, cost delta).  Promotion blocks if gates fail ; rollback triggers automatically when drift > threshold for ≥ min_sample_count samples.
5. **Cost attribution** — every shadow + canary cost cent routes to a configurable budget (`Platform` or `Tenant`), with hard caps enforced at dispatch time.  Shadow cost-cap exhaustion drops the shadow but never breaks production.

The integration story with the just-shipped neighbours :

- **N12 cache key v2** already discriminates on `resolved_version`, so the canary and prod cache entries naturally segregate — no cache-side work required beyond a `flush_for_agent` call on promotion.
- **N10 GDPR tombstone** cascades from a prod trace to its `ShadowRun` + `ShadowComparison` children via the existing `parent_trace_ids` lineage (which N1 added in Sprint 7).
- **N7 swarm cost cap** integrates with the new `cost_attribution` flag : a canary op's cost still counts against the swarm cap, but a shadow's cost is bookkept against a separate budget.

Verification gate (T+10d) :

| ID | Commitment | Test |
|---|---|---|
| C1 | RolloutPolicy round-trip through catalog | `rollout_policy_set_get_round_trip` GREEN |
| C2 | Sticky routing uniformity over 10 000 samples | `sticky_routing_uniform_within_1pct` GREEN |
| C3 | Async shadow doesn't block prod | `shadow_does_not_add_to_prod_latency` GREEN |
| C4 | 4 comparison strategies on Mode A / B / C | `comparison_strategy_matrix` GREEN |
| C5 | DriftWindow + promotion gate evaluator | `promotion_gates_block_on_drift` GREEN |
| C6 | Cost attribution routes shadow cost to chosen budget | `shadow_cost_routes_to_platform_budget` GREEN |
| C7 | N10 cascade — tombstone prod ⇒ tombstone shadow + comparison | `tombstone_cascade_includes_shadow_children` GREEN |
| C8 | N12 cache segregation — canary + prod coexist | `canary_and_prod_cache_entries_coexist` GREEN |
| C9 | Replay filters shadow traces (same pattern as `CacheHit`) | `replay_filters_shadow_run_traces` GREEN |

---

## 1. Problem statement — what we're solving

### 1.1 The naive deployment pattern (today, pre-N13)

```
operator → catalog.publish(v2)            ← atomic catalog write
        → all traffic for (tenant, agent_id) instantly resolves to v2
        → no comparison, no rollback gate, no shadow window
```

Risks materialised :

- **Mode A regression** — a Rust impl change that passes unit tests but produces subtly different output on a `null` field the unit tests didn't cover.
- **Mode B silent drift** — a new model fine-tune that degrades on edge-case inputs not in the validation set.
- **Mode C prompt template churn** — bumping `prompt_template_id` from v3 to v4 silently changes the JSON shape (adds a field, renames another) — caller-side parsers break.
- **Customer regression (Phase 4)** — tenant T publishes `my_agent v2.0.0` ; v2 has a bug ; T's own dispatch path breaks ; T has no platform-level mitigation.

### 1.2 What other rollout platforms do

For platform-level reference :

- **Slack** internal feature gates : 1 % / 5 % / 25 % / 100 % rollout, sticky per user, with auto-pause on error threshold.
- **Stripe** dashboard : shadow traffic against the prior version on every deploy, parity score surfaced to the engineer.
- **Google's Diff testing** (internal) : run two versions concurrently, compare outputs, surface drift to the SRE.
- **AWS Lambda alias weights** : assign 5 % of traffic to a new alias, rollback by editing weight to 0 %.

All four share three primitives :

1. **Sticky cohort selection** — same caller (or same input) always hits the same variant.
2. **Output comparison** — drift score surfaced.
3. **Rollback gate** — automated or operator-pulled.

N13 brings these primitives to EFE Nano, adapted for agent dispatch (input-hash-sticky, not session-sticky).

### 1.3 Why Phase 4 makes this urgent

The Phase 4 (M50.4) commitment is **customer-authored agents** — a tenant publishes their own nano agent code, and the platform dispatches it on their behalf.  The blast radius :

- One tenant's bad canary should NOT impact other tenants' agents.
- One tenant's bad canary should NOT take down their OWN traffic — they need a safe ramp.
- Operators must NOT need to be in the loop for every customer rollout (otherwise it doesn't scale).

Without N13, the only safe customer-authoring path is "publish to staging environment, manual test, copy-paste to prod" — which is exactly what the platform was supposed to eliminate.

### 1.4 What pre-N13 attempts looked like (and why they failed)

Three patterns tried earlier (all rejected) :

- **Multi-version catalog entries with caller-side selection** — caller passes `version: "1.2.x"` and gets the latest patch.  Fails because (a) catalog can't FORCE a canary on a caller, (b) leaks across tenants (one tenant's "latest" is another's "broken").
- **Coin-flip routing per request** — caller-agnostic random split.  Breaks idempotency cache (same input → different variants → cache thrash) and breaks replay determinism.
- **A/B cookie tagged on the SwarmRequest** — caller-side `experiment_group` field.  Requires every caller to participate ; platform can't enforce.

The right primitive : **deterministic sticky routing keyed on `input_hash`**, OWNED by the platform's catalog, INVISIBLE to the caller, and bound to the idempotency cache key (N12) for free.

---

## 2. Goals & non-goals

### 2.1 Sprint 8 N13 commitments — the 9 we ship

| ID | Goal | Surface | Verification |
|---|---|---|---|
| G1 | `RolloutPolicy` enum + catalog persistence (Postgres-ready trait) | `Catalog::set_rollout_policy / get_rollout_policy` | round-trip test |
| G2 | Sticky routing — deterministic, uniform per (tenant, agent, input) | `dispatcher::routing::sticky_sample` | 10 000-sample histogram test |
| G3 | Async shadow scheduler — prod-path-unaffected | `dispatcher::shadow::Scheduler` | timing test prod ≤ 5 % overhead |
| G4 | 4 comparison strategies (Exact, Structural, Semantic, None) | `dispatcher::shadow::ComparisonStrategy` | per-mode matrix |
| G5 | DriftWindow + promotion + rollback gates | `audit::drift::DriftWindow + PromotionGateEvaluator` | gate-evaluator GREEN |
| G6 | Cost attribution (Platform vs Tenant) + budget enforcement | `runtime::cost_attribution::CostRouter` | budget-cap test |
| G7 | N10 tombstone cascade : shadow + comparison children erased with parent | `audit::chain_store::tombstone_trace_cascade` | cascade test |
| G8 | N12 cache segregation : canary + prod coexist ; promotion flushes prod | reused N12 API | coexistence test |
| G9 | 9 Prometheus metrics : runs / matches / mismatches / cost / canary_routed / canary_traffic_bps / promotions / rollbacks / drift_rate_bps | scraped via `/metrics` | metric-exposure test |

### 2.2 Sprint 8 N13 non-goals (deferred)

| ID | Deferred capability | Sprint to ship |
|---|---|---|
| N1 | **Sync shadow** (block-until-comparison) — only async in Sprint 8 | Sprint 9 (under cost-cap-aware feature flag) |
| N2 | **Multi-version canary** (3+ variants simultaneously) — only 2-way | Sprint 10 |
| N3 | **Cross-tenant cohort sharing** ("5 % of all tenants' traffic") | Sprint 11 |
| N4 | **Per-region rollout segmentation** | Phase 4 deployment |
| N5 | **Promotion automation** (auto-promote when gates pass) — only auto-**rollback** is automated | Sprint 9 |
| N6 | **Postgres-backed `RolloutPolicy` storage** — in-memory only in Sprint 8 | Sprint 9 |
| N7 | **`ComparisonStrategy::Semantic` recursive comparator agents** that are themselves shadowed | Sprint 10 (cycle-detection) |
| N8 | **Multi-armed bandit** dynamic canary_bps tuning | Sprint 12+ |

---

## 3. Architecture overview

### 3.1 The 3-layer diagram

```
┌──────────────────────────────────────────────────────────────────────┐
│                       EFE Nano Dispatcher (M50)                      │
│                                                                      │
│    incoming SwarmRequest                                             │
│         │                                                            │
│         ▼                                                            │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ Layer 1 :  Catalog rollout-policy resolution                   │  │
│  │                                                                │  │
│  │   ResolvedAgent = catalog.resolve_for_dispatch(                │  │
│  │       tenant,                                                  │  │
│  │       agent_id,                                                │  │
│  │       version_req,    ← from OperationInput.agent.version      │  │
│  │       input_hash,     ← N6 source-hash on the canonical AST    │  │
│  │   )                                                            │  │
│  │                                                                │  │
│  │   internal steps :                                             │  │
│  │     a. catalog.get(agent_id, version_req) → base candidates    │  │
│  │     b. policy ← catalog.get_rollout_policy(tenant, agent_id)   │  │
│  │     c. match policy :                                          │  │
│  │          Stable(v)         ⇒ Single(v)                         │  │
│  │          Canary(v_p, v_c)  ⇒ if sticky_bps < canary_bps        │  │
│  │                                  Single(v_c)                   │  │
│  │                                else                            │  │
│  │                                  Single(v_p)                   │  │
│  │          Shadow(v_p, v_s)  ⇒ ShadowPair(v_p, v_s, compare)     │  │
│  │     d. honour version-pin opt-out (=X.Y.Z bypasses policy)     │  │
│  │     e. return ResolvedAgent                                    │  │
│  └────────────────────────────────────────────────────────────────┘  │
│         │                                                            │
│         ▼                                                            │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ Layer 2 :  Dispatch                                            │  │
│  │                                                                │  │
│  │    match ResolvedAgent :                                       │  │
│  │      Single(agent) :                                           │  │
│  │          run_one_op as today (unchanged)                       │  │
│  │      ShadowPair(prod, shadow, comparison, cost_attribution) :  │  │
│  │          1. tokio::spawn shadow run (audit + cost-attributed)  │  │
│  │          2. run_one_op(prod) synchronously                     │  │
│  │          3. return prod outcome to caller                      │  │
│  │          4. (in background) compare prod_output vs shadow      │  │
│  │             output via `compare_outputs(strategy, p, s)`       │  │
│  │          5. emit ShadowComparison trace                        │  │
│  │          6. DriftWindow.record(outcome)                        │  │
│  └────────────────────────────────────────────────────────────────┘  │
│         │                                                            │
│         ▼                                                            │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ Layer 3 :  Audit + drift gates                                 │  │
│  │                                                                │  │
│  │    Trace lineage :                                             │  │
│  │      prod_trace        : error_kind = None     (cold or hit)   │  │
│  │      shadow_trace      : error_kind = "ShadowRun"              │  │
│  │                          parent_trace_ids = [prod_trace.id]    │  │
│  │      comparison_trace  : error_kind = "ShadowComparison"       │  │
│  │                          parent_trace_ids = [prod, shadow]     │  │
│  │                                                                │  │
│  │    DriftWindow per (tenant, agent_id) :                        │  │
│  │      ring buffer of last K samples                             │  │
│  │      mismatch_rate_bps                                         │  │
│  │      error_rate_bps                                            │  │
│  │      p99_latency_delta_pct                                     │  │
│  │      cost_delta_pct                                            │  │
│  │                                                                │  │
│  │    PromotionGateEvaluator :                                    │  │
│  │      if drift_rate > gates.max_drift_bps                       │  │
│  │         AND samples ≥ gates.min_sample_count :                 │  │
│  │           auto-rollback (atomic catalog swap)                  │  │
│  └────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
```

### 3.2 The shape of the change

| Layer | Today (pre-N13) | After N13 |
|---|---|---|
| Catalog | `get(NanoAgentRef)` returns ONE agent | extends with `resolve_for_dispatch(...)` returning `Single` or `ShadowPair` |
| Dispatcher | `run_one_op` runs the resolved agent | branches on `ResolvedAgent` ; shadow-pair spawns background |
| Audit | Single trace per op | Up to 3 traces per op : prod + shadow + comparison |
| Replay | Walks all traces | **NEW** : filters out `ShadowRun` + `ShadowComparison` traces |
| GDPR | Tombstone single trace | **NEW** : cascade tombstone shadow + comparison children |
| Cost cap | Per-swarm `cost_cap_cents` (N7) | **NEW** : independent `shadow_budget` for Platform-attributed shadow cost |
| Cache key | N12 v2 (9-component) | unchanged — `resolved_version` already segregates prod / canary |
| Metrics | 4 cache metrics (N12) | **NEW** : +9 rollout metrics |

---

## 4. Component breakdown — 6 components

### 4.1 `catalog::rollout` (NEW module)

```rust
// crates/efe-nano-sdk/src/types/rollout.rs

/// **Sprint 8 — N13** : Per-(tenant, agent_id) deployment policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RolloutPolicy {
    /// Steady state : a single version serves 100 % of traffic.
    Stable {
        version: NanoAgentVersion,
    },
    /// New version runs side-by-side ; production response served,
    /// shadow output compared but never returned to the caller.
    Shadow {
        prod_version: NanoAgentVersion,
        shadow_version: NanoAgentVersion,
        started_at: DateTime<Utc>,
        comparison: ComparisonStrategy,
        cost_attribution: CostAttribution,
        shadow_budget_cents: Option<f64>,
    },
    /// New version serves `canary_bps` basis-points (0-10 000) of
    /// traffic, sticky-routed by input hash.
    Canary {
        prod_version: NanoAgentVersion,
        canary_version: NanoAgentVersion,
        canary_bps: u16,
        started_at: DateTime<Utc>,
        promotion_gates: PromotionGates,
        cost_attribution: CostAttribution,
    },
}
```

Rationale for the structure :

* **Tagged with `kind`** so JSON serialization is forward-compatible (a new `MultiArm` variant in Sprint 10 won't break old deserializers if we add `#[serde(other)]` on a `Future` variant).
* **`PartialEq` derive** so the dispatcher can detect "policy unchanged since last snapshot" cheaply.
* **`shadow_budget_cents: Option<f64>`** — `None` means "use the platform default budget" (`Platform` attribution) or "tenant budget" (`Tenant` attribution).  Explicit Some(0) means "shadow off in cost-emergency".

### 4.2 `catalog` API extension

```rust
#[async_trait]
pub trait Catalog: Send + Sync + 'static {
    // ── existing pre-N13 surface preserved ──
    async fn get(&self, r: &NanoAgentRef, tenant: TenantId) -> CatalogResult<Arc<NanoAgent>>;
    async fn list(&self, tenant: TenantId, filter: ListFilter) -> CatalogResult<Vec<Arc<NanoAgent>>>;
    async fn search(&self, tenant: TenantId, query: &str) -> CatalogResult<Vec<Arc<NanoAgent>>>;
    async fn publish(&self, tenant: TenantId, agent: NanoAgent) -> CatalogResult<NanoAgentId>;
    async fn unpublish(&self, tenant: TenantId, id: &NanoAgentId, version: &NanoAgentVersion)
        -> CatalogResult<()>;
    async fn promote(&self, id: &NanoAgentId, version: &NanoAgentVersion, to: StabilityTier)
        -> CatalogResult<()>;

    // ── NEW Sprint 8 N13 ──

    /// Resolve an agent for dispatch, applying any active rollout policy.
    /// Returns `Single` for stable / canary / version-pin opt-out, and
    /// `ShadowPair` for the shadow path.
    async fn resolve_for_dispatch(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version_req: &VersionReq,
        input_hash: &[u8; 32],
    ) -> CatalogResult<ResolvedAgent>;

    /// Set or update the rollout policy for an agent.  Atomic vs
    /// concurrent reads via Mutex<Policy> ; the previous policy is
    /// returned for an audit row.  Validates that both versions exist
    /// in the catalog and that the policy transition is legal (e.g.
    /// can't go from Stable(v1) directly to Stable(v2) — must go
    /// through Canary).
    async fn set_rollout_policy(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        policy: RolloutPolicy,
    ) -> CatalogResult<RolloutPolicy>;  // returns prior policy

    /// Get the current rollout policy.  Returns Stable(latest_version)
    /// if no explicit policy was ever set.
    async fn get_rollout_policy(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
    ) -> CatalogResult<RolloutPolicy>;
}

pub enum ResolvedAgent {
    Single(Arc<NanoAgent>),
    ShadowPair {
        prod: Arc<NanoAgent>,
        shadow: Arc<NanoAgent>,
        comparison: ComparisonStrategy,
        cost_attribution: CostAttribution,
        shadow_budget_cents: Option<f64>,
    },
}
```

The `resolve_for_dispatch` method is the **single integration point** the dispatcher consults.  This keeps the dispatcher rollout-policy-agnostic — it just sees either a single agent or a pair.

### 4.3 `dispatcher::routing` (NEW module)

```rust
// crates/efe-nano/src/dispatcher/routing.rs

/// **Sprint 8 — N13** : deterministic input-hash-sticky cohort sampler.
///
/// Returns a `u16` in `[0, 10 000)` — basis-points — that the catalog's
/// canary resolver compares against `canary_bps`.  Same `(tenant,
/// agent_id, input_hash)` triple → same sample, every invocation, every
/// machine, every restart.
pub fn sticky_sample(
    tenant: TenantId,
    agent_id: &NanoAgentId,
    input_hash: &[u8; 32],
) -> u16 {
    // SHA-256 over a versioned salt + the discriminator triple.
    let mut h = Sha256::new();
    h.update(STICKY_ROUTING_SALT);             // b"efe-nano-n13-routing-v1"
    h.update([0u8]);
    h.update(tenant.0.as_bytes());             // 16 bytes
    h.update(agent_id.as_str().as_bytes());
    h.update([0u8]);
    h.update(input_hash);                      // 32 bytes
    let digest = h.finalize();

    // Take first 4 bytes as a big-endian u32 ; modulo 10_000 introduces
    // at most 0.018 % bias (4 294 967 295 / 10 000 ≈ 429 496) — well
    // below the smallest discriminable canary cohort (1 bp = 0.01 %).
    let raw = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (raw % 10_000) as u16
}
```

Three design choices documented here :

* **Versioned salt** — `STICKY_ROUTING_SALT = b"efe-nano-n13-routing-v1"`.  Bumping to `-v2` re-shuffles the cohort selection globally without re-deploying agents.  Used as escape hatch for "we observed that our 5 % cohort is over-representing power users — re-shuffle".
* **u32 % 10 000 not u16 % 10 000** — the smaller modulus reduces the worst-case sub-cohort bias from 8.4 % relative (u16) to 0.018 % relative (u32).  Cheap and clean.
* **No tenant / agent_id separator** beyond the NUL byte — both have validators that reject NUL, so the boundaries are unambiguous.

### 4.4 `dispatcher::shadow` (NEW module)

```rust
// crates/efe-nano/src/dispatcher/shadow.rs

/// **Sprint 8 — N13** : shadow execution scheduler.
///
/// Spawned at dispatch time when `ResolvedAgent::ShadowPair` is
/// returned.  Runs the shadow agent against the same input
/// asynchronously, never blocking the production path, and emits two
/// chain-linked audit traces (ShadowRun + ShadowComparison).
pub struct ShadowScheduler {
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    drift_window: Arc<DriftWindow>,
    cost_router: Arc<CostRouter>,
}

impl ShadowScheduler {
    pub fn schedule(
        self: &Arc<Self>,
        tenant: TenantId,
        swarm_request_id: Uuid,
        prod_trace_id: TraceId,
        prod_outcome: ProdOutcomeSnapshot,
        shadow_agent: Arc<NanoAgent>,
        input: serde_json::Value,
        input_hash: [u8; 32],
        comparison: ComparisonStrategy,
        cost_attribution: CostAttribution,
        shadow_budget_cents: Option<f64>,
    );
}

/// What the production op produced — captured at the moment of return.
pub struct ProdOutcomeSnapshot {
    pub agent_version: NanoAgentVersion,
    pub output: serde_json::Value,
    pub output_hash: [u8; 32],
    pub cost_cents: f64,
    pub latency_ms: u32,
}
```

Three subtle design rules :

1. **Snapshot the prod outcome by value, not by reference**.  The shadow comparison may run minutes after the production response was returned — we MUST NOT rely on any handle that could be invalidated.
2. **`Arc<ShadowScheduler>`** — cloned cheaply into the spawned future, so the scheduler can be dropped without leaking shadows.  All internal state is `Arc<...>`.
3. **`schedule` is fire-and-forget** — returns `()`, not a `JoinHandle`.  We don't track the spawn task ; if the runtime is shutting down, `tokio::spawn`'d futures are cancelled and the shadow is dropped (with a `tracing::warn!` in the cancel-detect path).

### 4.5 `audit::drift` (NEW module)

```rust
// crates/efe-nano/src/audit/drift.rs

/// **Sprint 8 — N13** : per-(tenant, agent_id) sliding window of
/// shadow comparison outcomes.
pub struct DriftWindow {
    /// `(tenant, agent_id)` → ring buffer of last K outcomes.
    windows: DashMap<(TenantId, NanoAgentId), Mutex<VecDeque<ShadowOutcome>>>,
    max_samples: usize,
}

impl DriftWindow {
    pub fn record(&self, tenant: TenantId, agent_id: NanoAgentId, outcome: ShadowOutcome);
    pub fn stats(&self, tenant: TenantId, agent_id: &NanoAgentId) -> Option<DriftStats>;
    pub fn clear(&self, tenant: TenantId, agent_id: &NanoAgentId);
}

#[derive(Debug, Clone, Copy)]
pub struct DriftStats {
    pub sample_count: u32,
    pub match_count: u32,
    pub mismatch_count: u32,
    pub failed_shadow_count: u32,
    pub comparator_unavailable_count: u32,
    pub mismatch_rate_bps: u16,
    pub p99_latency_delta_pct: i32,
    pub cost_delta_pct: i32,
}
```

### 4.6 `runtime::cost_attribution` (NEW module)

```rust
// crates/efe-nano/src/runtime/cost_attribution.rs

/// **Sprint 8 — N13** : routes the cost of a shadow / canary call to
/// the right budget bucket.
pub struct CostRouter {
    /// Platform-wide shadow budget — global, refilled by ops process.
    platform_shadow_budget_cents: AtomicF64,
    /// Per-tenant shadow budget — refilled by tenant billing.
    tenant_shadow_budgets: DashMap<TenantId, AtomicF64>,
}

impl CostRouter {
    /// Reserve `cents` against the right budget.  Returns Err if
    /// budget exhausted ; the caller drops the shadow.
    pub fn reserve(
        &self,
        tenant: TenantId,
        cents: f64,
        attribution: CostAttribution,
    ) -> Result<CostReservation, CostBudgetError>;

    /// Commit the reservation after the shadow / canary completes
    /// with the actual cost.  Refunds if actual < reserved.
    pub fn commit(&self, reservation: CostReservation, actual_cents: f64);
}
```

---

## 5. `RolloutPolicy` — the on-disk source of truth

### 5.1 Variant semantics

```rust
pub enum RolloutPolicy {
    Stable { version },
    Shadow { prod_version, shadow_version, started_at, comparison, cost_attribution, shadow_budget_cents },
    Canary { prod_version, canary_version, canary_bps, started_at, promotion_gates, cost_attribution },
}
```

| Variant | When to use | Caller-visible behaviour |
|---|---|---|
| `Stable` | Default for every fresh agent.  Steady state after promotion completes. | 100 % traffic on `version`. |
| `Shadow` | Pre-canary quality gate.  Want to compare outputs before any caller sees v_shadow. | 100 % traffic on `prod_version` ; v_shadow runs async on EVERY input, compared, never returned. |
| `Canary` | Gradual rollout.  Sticky-route `canary_bps` of traffic to v_canary. | 99 % (or whatever) on prod, 1 % on canary ; same input always same variant. |

### 5.2 Legal transitions

```
Stable(v1)  ──set_canary(v2, X%)──▶  Canary(v1, v2, X%)
Stable(v1)  ──set_shadow(v2)─────▶  Shadow(v1, v2)
Shadow(v1, v2)  ──promote_to_canary(Y%)──▶  Canary(v1, v2, Y%)
Shadow(v1, v2)  ──drop_shadow()──▶  Stable(v1)
Canary(v1, v2, X%)  ──ramp(Y%)──▶  Canary(v1, v2, Y%)         (Y > X or Y < X both legal)
Canary(v1, v2, 100%)  ──promote()──▶  Stable(v2)
Canary(v1, v2, X%)  ──rollback()─▶  Stable(v1)
Canary(v1, v2, X%)  ──set_canary(v3, _)──▶  REJECTED          (must rollback or promote first)
Stable(v1)  ──set_stable(v2)────▶  REJECTED                   (no shortcut ; must canary or shadow first)
```

The "no shortcut" rule is enforced by `set_rollout_policy` :

```rust
fn validate_transition(prev: &RolloutPolicy, next: &RolloutPolicy)
    -> Result<(), InvalidTransition>
{
    match (prev, next) {
        // Stable → anything-else is fine.
        (RolloutPolicy::Stable { .. }, _) => Ok(()),

        // Shadow → Stable(prod_version) is the "drop shadow" path.
        (RolloutPolicy::Shadow { prod_version, .. },
         RolloutPolicy::Stable { version: v }) if v == prod_version => Ok(()),

        // Shadow → Canary(same prod, same shadow) is "promote shadow to canary".
        (RolloutPolicy::Shadow { prod_version, shadow_version, .. },
         RolloutPolicy::Canary { prod_version: pp, canary_version: cv, .. })
            if pp == prod_version && cv == shadow_version => Ok(()),

        // Canary → Stable(prod) = rollback ; Canary → Stable(canary) = promote.
        (RolloutPolicy::Canary { prod_version, canary_version, .. },
         RolloutPolicy::Stable { version: v }) if v == prod_version || v == canary_version
            => Ok(()),

        // Canary → Canary(same versions, different bps) is ramping.
        (RolloutPolicy::Canary { prod_version, canary_version, .. },
         RolloutPolicy::Canary { prod_version: pp, canary_version: cv, .. })
            if pp == prod_version && cv == canary_version => Ok(()),

        _ => Err(InvalidTransition::IllegalTransition),
    }
}
```

### 5.3 Why no `disabled` variant ?

A "rollout disabled" state would be redundant with `Stable(latest)`.  Removing the policy entirely is conceptually equivalent.  Sprint 10 may add a `Pinned { version, frozen_at }` variant for "this version is end-of-life, do not even allow operator override" — out of scope for N13.

### 5.4 The `started_at` field

Both `Shadow` and `Canary` carry `started_at`.  Used for :

- **Audit observability** — operators see "this canary has been running 14 days, time to promote or rollback".
- **Auto-expiry** (Sprint 10) — operator can configure `expires_at` and auto-rollback if not promoted by then.
- **Drift window scoping** — drift stats are accumulated since `started_at` and reset on every policy change.

Not used as part of cache-key invariant (would defeat caching across multiple canary windows).

### 5.5 Promotion gates spec

```rust
pub struct PromotionGates {
    /// Minimum total samples (sum of match + mismatch) before any gate
    /// evaluation is meaningful.  Default 100.
    pub min_sample_count: u32,

    /// Maximum allowed mismatch rate (basis-points = 1/10 000).
    /// Default 100 bps = 1 %.
    pub max_drift_bps: u16,

    /// Maximum allowed error rate (canary call failed entirely).
    /// Default 50 bps = 0.5 %.
    pub max_error_rate_bps: u16,

    /// Maximum allowed P99 latency delta — canary may not be more
    /// than X % slower than prod.  Default 50 % (i.e. canary p99
    /// ≤ 1.5× prod p99).  Signed : negative permits canary FASTER.
    pub max_latency_p99_delta_pct: u16,

    /// Maximum allowed cost delta.  Default 100 % (canary may cost
    /// up to 2× prod ; useful for Mode A → Mode C swap).
    pub max_cost_delta_pct: u16,
}
```

The five gates are evaluated **in conjunction** (all must pass for promotion to succeed).  Auto-rollback triggers when **any** of `max_drift_bps` or `max_error_rate_bps` is breached for ≥ `min_sample_count` samples.  Latency + cost are advisory at rollback time but blocking at promotion time.

---

## 6. Sticky routing algorithm

### 6.1 Why sticky ?

Three reasons :

1. **Idempotency holds** — N12 cache keys segregate by `resolved_version` ; if the same input toggled between prod and canary across calls, every call would miss the cache.
2. **Replay determinism** — replay of a swarm dispatched during canary MUST land the same op on the same variant as the original ; sticky routing makes this automatic.
3. **Caller-visible reproducibility** — when a customer hits a Mode C agent and sees a specific output, calling again with the same input should not flicker between two prompt templates.

### 6.2 Algorithm

```
sticky_bps = SHA-256(
    "efe-nano-n13-routing-v1"
    \x00
    tenant_id  (16 bytes raw UUID)
    agent_id   (UTF-8, NUL-terminated)
    input_hash (32 bytes, the N6 source hash)
)[0..4] interpreted as big-endian u32
sticky_bps %= 10_000
```

If `sticky_bps < canary_bps` → canary variant ; else → prod variant.

### 6.3 Uniformity proof

For SHA-256, the first 4 bytes are uniformly distributed over `[0, 2^32)`.  Modulo 10 000 :

- Worst-case bias : `(2^32 % 10_000) / 2^32 = 7 296 / 4 294 967 296 ≈ 1.7 × 10⁻⁶`.
- For a 1 % cohort (`canary_bps = 100`), expected sample frequency is 100 / 10 000 = 1.0 %.  Bias-adjusted maximum deviation per 10 000 samples : `100 ± 1.7 × 10⁻⁴` — well below detection threshold.

This is verified by the `sticky_routing_uniform_within_1pct` test : draw 10 000 samples uniformly across input hashes, count how many land in each bucket of width 100 bps, assert each bucket count is within ±20 % of expected (Chebyshev-loose check accounting for SHA-256 variance + sample-size noise).

### 6.4 Version-pin opt-out

The caller's `OperationInput.agent.version: VersionReq` can pin an exact version (`=1.2.3`).  In that case, the rollout policy is bypassed :

```rust
if version_req.is_exact_pin() {
    // Caller explicitly chose a version — honour it, ignore policy.
    return ResolvedAgent::Single(catalog.get_exact(agent_id, &pinned_version)?);
}
```

This is the escape hatch for callers who need reproducibility (e.g. for compliance audits replaying historical decisions).  Audit row : the trace records the actual `nano_agent_version` served, so even policy bypass is auditable.

### 6.5 Salt rotation

Bumping `STICKY_ROUTING_SALT` from `-v1` to `-v2` re-shuffles every (tenant, agent, input) → cohort assignment.  Use cases :

- **Cohort exhaustion** — the "5 % canary" cohort happened to be all power users ; results are biased.  Re-shuffle.
- **Migration to bigger cohort space** — Sprint 10 moves from `u16 % 10_000` to `u32 % 10_000_000` (sub-basis-points).  Bumping the salt + the modulus is one atomic change.

Salt rotation has a side-effect : every canary cache entry gets a "natural" expiry, because the cohort selection changed.  In practice we couple a salt bump with `idempotency.flush_for_agent(...)` on every active canary policy.  Sprint 9 documents this in an ops runbook.

---

## 7. Output comparison strategies

```rust
pub enum ComparisonStrategy {
    Exact,
    Structural,
    Semantic { comparator_agent_id: NanoAgentId },
    None,
}
```

### 7.1 `Exact`

```rust
fn compare_exact(prod_hash: [u8; 32], shadow_hash: [u8; 32]) -> ShadowOutcome {
    if prod_hash == shadow_hash {
        ShadowOutcome::Match { similarity: 1.0, ... }
    } else {
        ShadowOutcome::Mismatch { similarity: 0.0, diff_summary: "hash mismatch".into(), ... }
    }
}
```

* **Default for Mode A FullDeterministic.**
* **Default for Mode B Seeded when seeds match.**
* Cost : O(1).  Just a hash compare.
* Limitation : doesn't tolerate ANY whitespace, key-order, or numeric-representation difference.  Use `Structural` if those are acceptable.

### 7.2 `Structural`

```rust
fn compare_structural(prod: &Value, shadow: &Value) -> ShadowOutcome {
    // 1. Same JSON shape : same key set at every level, same array lengths.
    // 2. Same value TYPES (string, number, null, bool, array, object).
    // 3. Value differences allowed within type — score by % of leaf
    //    paths that are byte-identical.
    let (shape_ok, similarity) = walk(prod, shadow);
    if !shape_ok {
        return ShadowOutcome::Mismatch { similarity: 0.0, diff_summary: shape_diff(...) };
    }
    if similarity >= 0.95 {
        ShadowOutcome::Match { similarity, ... }
    } else {
        ShadowOutcome::Mismatch { similarity, diff_summary: leaf_diff(...) }
    }
}
```

* **Default for Mode C LlmReplayable.**
* Catches "the new prompt template added/removed a JSON key" without thrashing on harmless wording changes inside fields.
* Tunable threshold (default 0.95) configurable per `RolloutPolicy::Shadow.comparison_threshold` (Sprint 9 — out of scope for Sprint 8 ship, hard-coded 0.95 for now).
* Limitation : doesn't detect SEMANTIC drift inside fields (e.g. v1 said "Apple Inc." and v2 said "AAPL" — structurally identical, semantically different).

### 7.3 `Semantic { comparator_agent_id }`

```rust
fn compare_semantic(prod: &Value, shadow: &Value, comparator: &NanoAgent, runtime: &dyn NanoAgentExecutor)
    -> ShadowOutcome
{
    // Build the comparator's input.
    let comparator_input = json!({
        "prod_output": prod,
        "shadow_output": shadow,
        "comparison_kind": "semantic_similarity",
    });
    // Run the comparator agent (typically Mode A, sometimes Mode C).
    let resp = runtime.execute(comparator, comparator_input).await;
    // The comparator MUST return { "similarity": f, "reasoning": "..." }.
    let similarity: f32 = resp.output["similarity"].as_f64().unwrap_or(0.0) as f32;
    if similarity >= 0.95 { ShadowOutcome::Match { similarity, ... } }
    else { ShadowOutcome::Mismatch { similarity, diff_summary: resp.output["reasoning"]... } }
}
```

* **Opt-in** — only when the policy specifies a `comparator_agent_id`.
* The comparator is an ordinary nano agent — usually a tenant-specific Mode A or a starter-pack Mode C "diff_judge".
* **Cycle risk** — if the comparator itself is shadowed, we'd recurse forever.  Sprint 8 forbids comparator self-shadow with a catalog invariant ; Sprint 10 adds cycle detection.

### 7.4 `None`

* Used when comparison is intentionally skipped — e.g. Mode C LlmEphemeral, where every call is genuinely unique and comparison is meaningless.
* The shadow still RUNS — for cost analytics, latency comparison, and parity-of-shape — but the per-call comparison is skipped.
* DriftWindow ignores `None`-strategy outcomes for the drift rate calculation, but counts them toward `sample_count` for latency/cost gates.

### 7.5 Default-by-mode matrix

```
┌─────────────────────────┬─────────────────────────────────┐
│   Agent.determinism     │   Default ComparisonStrategy     │
├─────────────────────────┼─────────────────────────────────┤
│ FullDeterministic       │ Exact                            │
│ Seeded                  │ Exact (assuming seed matches)    │
│ LlmReplayable           │ Structural                       │
│ LlmEphemeral            │ None                             │
│ SideEffect              │ ⛔ shadow PROHIBITED              │
└─────────────────────────┴─────────────────────────────────┘
```

SideEffect agents (e.g. one that writes to a DB or sends an email) MUST NOT be shadowed — running the shadow would cause a double-write.  The catalog's `set_rollout_policy` rejects `Shadow` policies for `DeterminismLevel::SideEffect` agents.

---

## 8. Shadow + canary trace lineage

### 8.1 The three new trace shapes

Building on the N12 cache-hit trace pattern (`error_kind = Some("CacheHit")`), N13 introduces two new sentinels :

```
Production trace        : NanoAgentTrace { error_kind: None, ... }            (existing)
CacheHit trace          : NanoAgentTrace { error_kind: Some("CacheHit"), ... } (N12)
ShadowRun trace         : NanoAgentTrace { error_kind: Some("ShadowRun"),
                                            parent_trace_ids: [prod_id], ... } (NEW N13)
ShadowComparison trace  : NanoAgentTrace { error_kind: Some("ShadowComparison"),
                                            parent_trace_ids: [prod_id, shadow_id],
                                            output_hash: Some(similarity_hash), ... } (NEW N13)
```

**A canary call does NOT emit a special trace** — it emits a standard production trace, but the trace's `nano_agent_version` is the canary version.  The fact that the call was a canary is inferable post-hoc from comparing `nano_agent_version` to the prod_version on the policy at the trace's time.  Sprint 9 may add a `rollout_kind: Option<RolloutKind>` enum field, but Sprint 8 keeps the trace schema unchanged for safety.

### 8.2 Lifecycle

```
T0  : caller dispatches op O
T1  : catalog.resolve_for_dispatch returns ShadowPair(prod_v, shadow_v)
T2  : dispatcher pre-allocates prod_trace_id + shadow_trace_id (N1 invariant)
T3  : ShadowScheduler.schedule(...) — spawns the background task
T4  : prod runtime.execute(prod_agent) — emits prod_trace
T5  : dispatcher returns ProdOutcome to caller, op closed
                                                              ── caller sees result here ──
T6  : (background) shadow runtime.execute(shadow_agent) — emits shadow_trace
        (parent_trace_ids = [prod_trace.id])
T7  : (background) compare_outputs(strategy, prod_out, shadow_out) → ShadowOutcome
T8  : (background) emit ShadowComparison trace
        (parent_trace_ids = [prod_trace.id, shadow_trace.id])
T9  : (background) DriftWindow.record(outcome)
T10 : (background) PromotionGateEvaluator.check_rollback(tenant, agent_id)
        if breach detected ⇒ atomic catalog swap to Stable(prod_version)
```

### 8.3 Audit chain integrity

The HMAC chain (Sprint 6 ship) verifies as before — every trace, including the shadow / comparison ones, gets its own chain row with payload_hash and HMAC.  The chain's parent pointer is `chain_parent_id`, which is set to the IMMEDIATE PREDECESSOR in the per-tenant chain (NOT the same as `parent_trace_ids` which is the DAG / lineage parent).

So if a swarm has 3 ops, the chain might look like (per tenant) :

```
Index   trace.id    error_kind        parent_trace_ids (DAG)         chain_parent_id (HMAC)
1       p1          None              []                              None (epoch root)
2       p2          None              [p1]                            1
3       p3          None              [p2]                            2
4       s2          ShadowRun         [p2]                            3
5       s3          ShadowRun         [p3]                            4
6       c2          ShadowComparison  [p2, s2]                        5
7       c3          ShadowComparison  [p3, s3]                        6
```

The HMAC chain remains linear and tamper-evident ; the lineage tree (DAG of `parent_trace_ids`) is orthogonal.

### 8.4 Replay implications

The replay engine (`crates/efe-nano/src/replay/mod.rs`) was extended in N12 to filter out `CacheHit` traces.  N13 extends the filter to also exclude `ShadowRun` and `ShadowComparison` traces :

```rust
let traces: Vec<_> = all_traces
    .into_iter()
    .filter(|entry| !matches!(
        entry.trace.error_kind.as_deref(),
        Some("CacheHit") | Some("ShadowRun") | Some("ShadowComparison")
    ))
    .collect();
```

This keeps the replay invariant — "replay sees only the K ops that were actually served, regardless of how many shadow / comparison rows were appended".

### 8.5 Cache-hit interaction

When a cache hit occurs on a Shadow-policy agent :

- N12 writes the CacheHit trace as before.
- The shadow is STILL scheduled — because the shadow output may have evolved since the cache entry was first populated.  This is configurable per policy via `shadow_on_cache_hit: bool` (default true for `Shadow`, default false for `Canary`).  We want full shadow coverage during a shadow window ; we don't want to amplify canary cost.

When a cache hit occurs on a Canary-policy agent :

- The cache key (N12) is keyed on `resolved_version`, so canary cache hits are confined to the canary cohort.
- No shadow is scheduled (canary policies don't shadow).

---

## 9. Cost accounting model

### 9.1 The dual ledger

Every dispatched op now produces TWO ledger entries when shadowed :

| Entry | Subject | Amount | Attribution |
|---|---|---|---|
| Production cost | tenant budget (N7 swarm cap) | `prod.cost_cents` | Tenant (always) |
| Shadow cost | per `CostAttribution` | `shadow.cost_cents` | Platform OR Tenant |

```rust
pub enum CostAttribution {
    /// Platform-wide budget absorbs the cost.  Default for operator-
    /// initiated shadow policies on EFE-shipped agents.
    Platform,
    /// Tenant pays — only ethical when the tenant explicitly opted
    /// in via `efe_nano_publish_canary { cost_attribution: "tenant" }`.
    Tenant,
}
```

### 9.2 Budget enforcement

```
shadow_cost_estimate = shadow_agent.cost_estimate_cents
if CostRouter.reserve(tenant, shadow_cost_estimate, attribution).is_err() {
    // Budget exhausted — drop the shadow, log a warning, emit a metric.
    tracing::warn!(?tenant, ?agent_id, "shadow dropped : budget exhausted");
    metric_shadow_dropped_due_to_budget_total.inc();
    return;   // Production response still served unchanged.
}
```

**Budget exhaustion is silent to the caller** — production succeeds.  This is the critical guarantee : a budget squeeze never breaks customer requests.

### 9.3 Canary cost handling

Canary calls are REAL production — they go through the standard N7 swarm cost cap, charged to the tenant.  No separate budget.  Operators can still configure `cost_attribution: Platform` on the Canary policy if they want the canary cohort cost subsidized — but this is unusual (typically only used for "we want to test a 10× more expensive Mode C version against Mode A and absorb the extra cost while we evaluate").

### 9.4 Default budgets (Sprint 8 ship)

| Budget | Default | Configurable via |
|---|---|---|
| Platform shadow budget | $50 / day | `EFE_NANO_PLATFORM_SHADOW_BUDGET_USD_PER_DAY` env |
| Per-tenant shadow budget | unconfigured (= unlimited until set) | `efe_admin_tenant_set_shadow_budget` MCP tool (Sprint 9) |

In dev / test, both budgets are effectively unlimited (`f64::INFINITY`).  Production deploys override via env.

### 9.5 Metering integration

The `crates/efe-nano/src/metering/` module already aggregates per-tenant cost.  N13 extends it with :

- `cost_cents_shadow` — cumulative cost of shadow runs, by tenant and attribution.
- `cost_cents_canary` — cumulative cost of canary-cohort production runs.
- These are pivoted in the M0 dashboard alongside the existing prod cost metric.

---

## 10. Promotion workflow

### 10.1 Operator-initiated promotion (Sprint 8 ship)

```
operator → MCP tool : efe_nano_promote_canary {
    tenant_id: <uuid>,
    agent_id: "json_validate",
}

dispatcher service (admin role check) →
    1. catalog.get_rollout_policy(tenant, agent_id)
       → Canary { prod_v: 1.2.3, canary_v: 1.3.0, canary_bps: 5000, gates, ... }
    2. drift_window.stats(tenant, agent_id) → DriftStats
    3. PromotionGateEvaluator.evaluate(stats, gates) → Pass | Block(reasons)
    4. if Pass :
         catalog.set_rollout_policy(tenant, agent_id, Stable { version: 1.3.0 })
         idempotency.flush_for_agent(tenant, &agent_id)   ← N12 cache invalidation
         drift_window.clear(tenant, &agent_id)
         emit audit row : RolloutPolicyChanged { from: Canary, to: Stable(1.3.0) }
         return PromotionReceipt { promoted_at, ... }
    5. if Block(reasons) :
         return PromotionBlocked { reasons }
```

### 10.2 Gate evaluation logic

```rust
impl PromotionGateEvaluator {
    pub fn evaluate(&self, stats: &DriftStats, gates: &PromotionGates) -> GateOutcome {
        let mut blocks = Vec::new();
        if stats.sample_count < gates.min_sample_count {
            blocks.push(format!("insufficient samples : {} < {}",
                stats.sample_count, gates.min_sample_count));
        }
        if stats.mismatch_rate_bps > gates.max_drift_bps {
            blocks.push(format!("drift {}.{:02} % > {}.{:02} % threshold",
                stats.mismatch_rate_bps / 100, stats.mismatch_rate_bps % 100,
                gates.max_drift_bps / 100, gates.max_drift_bps % 100));
        }
        // ... error rate / latency / cost gates similarly ...
        if blocks.is_empty() { GateOutcome::Pass }
        else { GateOutcome::Block { reasons: blocks } }
    }
}
```

### 10.3 Atomicity guarantee

`catalog.set_rollout_policy` is the atomic boundary :

- Uses `Mutex<HashMap<(TenantId, NanoAgentId), RolloutPolicy>>` in the in-memory impl.
- Uses a `SELECT ... FOR UPDATE` + `UPDATE` in the Postgres impl (Sprint 9).

In-flight dispatches that already captured the old policy (via `resolve_for_dispatch`) complete with the old policy.  New dispatches see the new policy.  No mid-swarm "split brain".

### 10.4 Audit row on promotion

Promotion emits a `RolloutPolicyChanged` event via the existing M28 event bus + a special audit-chain trace :

```
NanoAgentTrace {
    error_kind: Some("RolloutPolicyChanged"),
    nano_agent_id: agent_id,
    nano_agent_version: old_prod_version,    // we record what was REPLACED
    operation_id: "rollout_policy_change",
    parent_trace_ids: [],
    swarm_request_id: ROLLOUT_VIRTUAL_SWARM_UUID,
    full_input: Some(json!({ "from": old_policy, "to": new_policy, "actor": "operator:<id>" })),
    ...
}
```

This is **a non-customer-facing virtual swarm trace** — it doesn't correspond to a real DAG op, but it's part of the HMAC chain so that promotion / rollback is itself tamper-evident.

---

## 11. Rollback workflow — manual + automatic

### 11.1 Manual rollback (MCP tool)

```
operator → MCP : efe_nano_rollback_canary {
    tenant_id: <uuid>,
    agent_id: "json_validate",
}

→ catalog.set_rollout_policy(tenant, agent_id, Stable { version: prod_version_from_canary })
→ idempotency.flush_for_input_hash(<each cached canary entry>)   ← targeted, not flush_for_agent
→ drift_window.clear(tenant, &agent_id)
→ emit audit row : RolloutPolicyChanged { from: Canary, to: Stable(prod_v) }
```

Note the cache flush is **targeted** : only canary entries are evicted, prod entries stay warm.  Sprint 8 implementation uses `flush_for_input_hash` on each canary entry ; Sprint 9 may add a `flush_for_version` shortcut.

### 11.2 Automatic rollback (drift breach)

After every shadow comparison or canary error :

```rust
let stats = drift_window.stats(tenant, agent_id);
if stats.sample_count >= gates.min_sample_count
   && stats.mismatch_rate_bps > gates.max_drift_bps {
    catalog.set_rollout_policy(tenant, agent_id,
        RolloutPolicy::Stable { version: policy.prod_version() })
    .await?;
    // ... cache flush + drift clear + audit row ...
    metric_auto_rollback_total.inc();
}
```

The check is performed by the `ShadowScheduler` after every comparison — it's a cheap O(1) atomic-counter read on the drift window, gated by a `last_eval_at` cooldown (default 60 seconds) so we don't hammer the catalog.

### 11.3 Why auto-rollback but not auto-promote ?

Asymmetric defaults :

- **Auto-rollback** : low-risk action (returns to known-good state).  Worth automating to bound blast radius.
- **Auto-promote** : high-risk action (introduces an unverified state to 100 % of traffic).  Operator-only by default.  Sprint 9 may add `auto_promote: AutoPromoteAfter { duration, no_breach_window }`.

### 11.4 Rollback rate limiting

To prevent thrashing under flaky drift signals :

- After an auto-rollback, the policy is locked at `Stable` for at least 1 hour.  Attempts to set a new policy on the same agent during the lockout return `RolloutPolicyError::RollbackCooldown`.
- Manual operator-initiated rollback always succeeds (no cooldown — operators override).
- Cooldown is held in-memory in `DriftWindow` (per `(tenant, agent_id)` last-rollback timestamp).  Lost on process restart — acceptable trade-off in Sprint 8 ; Sprint 9 persists.

---

## 12. Idempotency cache interaction (N12 alignment)

### 12.1 Canary and prod cache entries coexist naturally

The N12 cache key derivation (`derive_key_v2`) embeds the **resolved_version** as a key component :

```
key = SHA-256(
    "efe-nano-idempotency-v2"
    \x00 tenant_id
    \x00 agent_id
    \x00 resolved_version    ← v_prod vs v_canary differ here
    ...
)
```

So a request that lands on `v_canary` gets a different cache key than the same input on `v_prod`.  No special N13 work needed — the entries coexist, segregated.

### 12.2 Promotion-time cache flush

After promotion `Canary(v1, v2, X%) → Stable(v2)` :

```rust
idempotency.flush_for_agent(tenant, &agent_id).await;
```

This drops BOTH `v_prod` and `v_canary` cache entries.  The next call cold-fetches under the new policy (now `Stable(v_canary)`).  This is correct because the v_canary entries were CANARY-cohort entries — after promotion the cohort is "everyone" not "5 %", so the prior canary entries' sticky-routing invariant may no longer hold.

### 12.3 Rollback-time cache flush

After rollback `Canary(v1, v2, X%) → Stable(v1)` :

```rust
// Only flush v_canary entries — keep v_prod cache warm.
for input_hash in drift_window.observed_canary_input_hashes(tenant, &agent_id) {
    idempotency.flush_for_input_hash(
        tenant, &agent_id, &v2 /* canary version */, &input_hash
    ).await;
}
```

Sprint 8 implementation : keep a per-agent `observed_canary_input_hashes` set inside the `DriftWindow`, bounded by `max_samples`.  On rollback, iterate.  Sprint 9 simplifies via a `flush_for_version(tenant, agent_id, version)` shortcut.

### 12.4 Shadow cache pollution risk

Shadow policies cache the SHADOW output under the shadow version's cache key.  Two implications :

* **Pro** : a 2nd identical request doesn't re-shadow — the cache serves the prior shadow output (and a `CacheHit` trace is emitted for the shadow).
* **Con** : if the shadow is buggy and produces wrong output, the bad output sits in cache for `idempotency.ttl_sec`.  Mitigation : drop_shadow workflow also calls `flush_for_input_hash` on observed shadow entries.

### 12.5 N12 cache-hit trace lineage compatibility

When a shadow op hits cache (shadow served from prior shadow run) :

- N12 writes a CacheHit trace with `parent_trace_ids = [original_shadow_trace_id]`.
- The downstream ShadowComparison trace then has `parent_trace_ids = [prod_id, cache_hit_shadow_id]`.
- This 3-level lineage stays consistent with the chain HMAC integrity.

The replay engine's existing `CacheHit` filter handles this without modification.

---

## 13. Replay engine interaction

### 13.1 The "shadow is not a real op" invariant

The replay engine's job is to verify that, given the same inputs, the agent produces the same outputs (deterministically) and the audit chain hasn't been tampered.  Shadow runs are NOT "real" ops — they're a quality measurement.  Replaying them would be a category error.

Sprint 8 extends the existing CacheHit filter (already in `replay/mod.rs:101-108`) :

```rust
let traces: Vec<_> = all_traces
    .into_iter()
    .filter(|entry| !matches!(
        entry.trace.error_kind.as_deref(),
        Some("CacheHit") | Some("ShadowRun") | Some("ShadowComparison") | Some("RolloutPolicyChanged")
    ))
    .collect();
```

This means replay always sees the K real ops, regardless of how many shadow / comparison rows were appended.

### 13.2 Canary replay

Canary calls ARE real ops — they served real production traffic.  The trace records `nano_agent_version = v_canary`.  Replay must use the SAME version (the trace's recorded version), per the existing `replay::lookup_determinism` logic that pins to the original version.  No new code needed — the existing pinning works.

### 13.3 Edge case : a canary that was rolled back

Scenario : swarm S dispatched at T1 with `Canary(v1, v2, 100%)` — so 100 % went to v2.  At T2 the canary was rolled back to `Stable(v1)`.  At T3 we replay S.

- Replay loads the chain entries for S — they all record `nano_agent_version = v2`.
- Replay reconstructs the SwarmRequest with the original inputs.
- Replay calls `catalog.resolve_for_dispatch(...)` — current policy is `Stable(v1)`, so it returns v1.
- **The replay would run v1, but the original served v2 — versions don't match → ReplayMismatch.**

The fix : replay's pinning logic must call `catalog.get(NanoAgentRef { id, version: =v2 })` instead of `resolve_for_dispatch`.  This is the existing Sprint 7 N5 behaviour — replay pins to the exact version recorded on the trace.  N13 doesn't need to change this ; we just need to make sure `Catalog::get(NanoAgentRef)` still resolves a specific version even after rollout policy changes.

Test : `replay_after_canary_rollback_uses_original_version` (G9-derived).

---

## 14. GDPR tombstone cascade (N10 alignment)

### 14.1 The cascade contract

When a tenant invokes GDPR Art. 17 erasure on a trace `prod_t1`, every dependent trace must also be tombstoned :

```
prod_t1  (the user-facing op trace)
  ├── shadow_t1     (parent_trace_ids = [prod_t1.id])
  └── comparison_t1 (parent_trace_ids = [prod_t1.id, shadow_t1.id])
```

If `prod_t1` is tombstoned but `comparison_t1` survives with its `full_input` populated, we have a GDPR leak : the comparison's `full_input` may contain the original PII passed through the prod call.

### 14.2 Implementation : extend `tombstone_trace` to cascade

The existing N10 `ChainStore::tombstone_trace(tenant, trace_id, metadata)` operates on a SINGLE trace.  N13 extends it with a new method :

```rust
async fn tombstone_trace_cascade(
    &self,
    tenant: TenantId,
    root_trace_id: Uuid,
    metadata: TombstoneMetadata,
) -> Result<TombstoneOutcome, ChainStoreError>;
```

Implementation logic :

1. Tombstone `root_trace_id` (the regular `tombstone_trace` path).
2. Scan the chain for entries where `parent_trace_ids` contains `root_trace_id`.
3. For each : if `error_kind ∈ { ShadowRun, ShadowComparison, CacheHit }`, tombstone too.
4. Cascade recursively (a ShadowComparison's parent_trace_ids contains shadow_t1.id ; tombstoning prod_t1 cascades to shadow_t1, then to comparison_t1).

Performance : O(N) over the chain.  For the in-memory store, acceptable.  For Postgres (Sprint 9), an index on `parent_trace_ids` (GIN) makes this O(log N) per ancestor.

### 14.3 Cache cascade too

Already covered by N12 + N13's `flush_for_original_trace` :

```rust
idempotency.flush_for_original_trace(prod_trace_id).await;
// AND
idempotency.flush_for_original_trace(shadow_trace_id).await;  // the shadow's cached output
```

The GDPR saga (M24 Phase 4 component) is responsible for sequencing these two calls after the chain store cascade.

---

## 15. Mode A / B / C specifics

### 15.1 Mode A (PureRust)

| Aspect | Recommendation |
|---|---|
| Default comparison | `Exact` |
| Cost | Equal between v1 and v2 (pure Rust both) |
| Latency overhead | Minimal — shadow runs in the same process, no microVM cost |
| Risk | Lowest — pure Rust agents are FullDeterministic by definition |
| Promotion gate defaults | `max_drift_bps: 0` (Mode A must be byte-deterministic) |

Use case : refactoring a starter-pack Mode A handler.  Set up `Shadow(v1, v2)` for a few days to catch any byte-level drift before promotion.

### 15.2 Mode B (LocalModel)

| Aspect | Recommendation |
|---|---|
| Default comparison | `Exact` (Seeded) or `Structural` (when seeds differ) |
| Cost | Equal (both consume the local model microVM pool) |
| Latency overhead | One extra VM acquisition — bounded by the N11 pool's `max_parallelism` |
| Risk | Medium — model fine-tunes can produce different numeric outputs on edge cases |
| Promotion gate defaults | `max_drift_bps: 50` (0.5 % drift) |

Use case : new model weights for `ner_clinical`.  Shadow window of 1k samples ; auto-rollback if `max_drift_bps` breached.

### 15.3 Mode C (LlmAugmented)

| Aspect | Recommendation |
|---|---|
| Default comparison | `Structural` (LlmReplayable) or `None` (LlmEphemeral) |
| Cost | Doubles — every call now hits the LLM provider twice |
| Latency overhead | Substantial — second LLM call adds 100ms-2s ; mitigated by `async` shadow |
| Risk | Highest — prompt template version changes can subtly shift JSON shape |
| Promotion gate defaults | `max_drift_bps: 100` (1 %) + `max_cost_delta_pct: 50` |
| Cost attribution default | `Platform` (operator absorbs the 2x cost during the shadow window) |

Use case : bump `summarize_strict.prompt_template_id` from v3 to v4.  Shadow for 24h with N=10k samples ; promote only if structural match rate > 99 %.

**LlmEphemeral special case** : these agents have no Match expectation (every call returns a genuinely different output).  Shadow is still useful for :

- Cost comparison (is the new model cheaper ?).
- Latency comparison.
- Schema validation (does the new model output conform to the same JSON schema ?).

But the per-call comparison strategy MUST be `None` ; using `Exact` or `Structural` would produce a 100 % mismatch rate and trigger spurious rollbacks.

### 15.4 SideEffect mode (M50.6)

Shadow PROHIBITED.  Catalog rejects the policy.  Canary allowed (it's still real traffic, just cohort-routed).

### 15.5 Mixed-mode canary (Mode A → Mode B)

When `canary_version` is a different mode than `prod_version` :

- Cache keys segregate via N12 (different `execution_mode` discriminator).
- Cost may differ by 100× (Mode A ~0.0001¢ vs Mode B ~0.01¢).
- `max_cost_delta_pct` gate must be set high enough to permit the promotion (e.g. 10 000 % = 100×).
- The Structural comparison may need a custom comparator if Mode A and Mode B return DIFFERENT JSON shapes (Mode A returns `{"valid": true}` whereas Mode B returns `{"is_valid": true, "confidence": 0.97}`).

This is documented as an advanced use case in the customer-authoring guide.

---

## 16. Customer authoring (Phase 4)

### 16.1 The two MCP tools

```typescript
// Tool : efe_nano_publish_canary
//   Tenant publishes a canary on their OWN agent.
// Author : Tenant-owned only.  EFE-shipped agents reject this tool.

efe_nano_publish_canary({
  agent_id: "customer.summarize_legal_doc",
  prod_version: "1.2.3",
  canary_version: "1.3.0-rc1",
  canary_bps: 500,                              // 5 %
  comparison: { kind: "structural" },
  promotion_gates: {
    min_sample_count: 1000,
    max_drift_bps: 100,
    max_error_rate_bps: 50,
    max_latency_p99_delta_pct: 50,
    max_cost_delta_pct: 50,
  },
  cost_attribution: { kind: "tenant" },
})

// Tool : efe_nano_promote_canary
efe_nano_promote_canary({
  agent_id: "customer.summarize_legal_doc",
})

// Tool : efe_nano_rollback_canary
efe_nano_rollback_canary({
  agent_id: "customer.summarize_legal_doc",
})

// Tool : efe_nano_observe_canary
//   Returns DriftStats + gate evaluation, doesn't change state.
efe_nano_observe_canary({ agent_id: "customer.summarize_legal_doc" })
```

### 16.2 RBAC

| Tool | Required role |
|---|---|
| `efe_nano_publish_canary` | Tenant admin OR (Phase 4) tenant developer with `agents:publish` permission |
| `efe_nano_promote_canary` | Tenant admin only |
| `efe_nano_rollback_canary` | Tenant admin OR operator (escalation) |
| `efe_nano_observe_canary` | Anyone with `agents:read` |

EFE-shipped agents (Author::Efe) reject all four tools when called by a customer ; only EFE operators can rollout-policy those.

### 16.3 Cross-tenant safety

A tenant can only set policies on agents it OWNS (Author::Tenant matches their tenant_id).  Catalog enforces :

```rust
async fn set_rollout_policy(&self, tenant, agent_id, policy) -> CatalogResult<...> {
    let agent = self.get(&NanoAgentRef::new(agent_id.clone()), tenant).await?;
    match agent.author {
        Author::Tenant { tenant_id } if tenant_id == tenant => Ok(()),
        Author::Efe => Err(CatalogError::Unauthorized("cannot policy EFE-shipped agent")),
        _ => Err(CatalogError::Unauthorized("cannot policy another tenant's agent")),
    }
}
```

### 16.4 Quota limits per tenant

To prevent a tenant from running 100 canaries simultaneously :

- Default : max 3 active rollout policies per tenant (Sprint 8 ship).
- Configurable per tenant tier (`tier1: 3 ; tier2: 10 ; tier3: unlimited`).
- Enforced at `set_rollout_policy` time.

---

## 17. Tenant isolation + RBAC

### 17.1 Storage isolation

Rollout policies are stored keyed on `(tenant_id, agent_id)`.  No cross-tenant visibility :

```rust
struct InMemoryCatalog {
    agents: DashMap<(TenantId, NanoAgentId, NanoAgentVersion), Arc<NanoAgent>>,
    policies: DashMap<(TenantId, NanoAgentId), RolloutPolicy>,    // NEW
}
```

Phase 4 Postgres schema (Annexe B) uses `tenant_id` as a partition key.

### 17.2 RBAC summary

| Action | Operator (EFE) | Tenant Admin | Tenant Dev | End User |
|---|---|---|---|---|
| Read EFE-shipped policy | ✅ | ✅ | ✅ | ❌ |
| Read own-tenant policy | ✅ | ✅ | ✅ | ❌ |
| Read other-tenant policy | ❌ (cross-tenant breach) | ❌ | ❌ | ❌ |
| Set EFE-shipped policy | ✅ | ❌ | ❌ | ❌ |
| Set own-tenant policy | ✅ (override) | ✅ | with permission | ❌ |
| Manual rollback own-tenant | ✅ | ✅ | with permission | ❌ |
| Auto-rollback (system) | n/a | n/a | n/a | n/a |

### 17.3 Audit trail per RBAC action

Every policy-change action emits a `RolloutPolicyChanged` audit row carrying :

- `actor_kind: enum { Operator(operator_id), TenantAdmin(user_id), AutoRollback, ... }`
- `from`, `to` (full policy structs)
- `reason: Option<String>` (operator's optional explanation)
- `timestamp` (microseconds)

This row IS part of the HMAC chain (it's a NanoAgentTrace with `error_kind = Some("RolloutPolicyChanged")`).  Tenants can replay their own policy history.

---

## 18. Failure modes — F1 to F12

### 18.1 F1 : Shadow runtime fails (e.g. microVM crash)

- Production succeeds, ProdOutcome returned to caller.
- ShadowScheduler captures the error.
- ShadowRun trace recorded with `error_kind = Some("ShadowRun")` and `error_kind` extended (`Some("ShadowRunFailed:ConnectionRefused")` — sentinel-encoded).
- ShadowComparison trace : `ShadowOutcome::ShadowFailed`.
- DriftWindow records as `failed_shadow_count++`.
- Promotion gate : excessive `failed_shadow_count` → block promotion.
- No auto-rollback (it's the shadow that failed, not the canary).

### 18.2 F2 : Comparator agent fails

- Production + shadow both succeed.
- `compare_outputs` calls the comparator agent ; agent errors.
- ShadowComparison trace : `ShadowOutcome::ComparatorUnavailable`.
- DriftWindow records as `comparator_unavailable_count++`.
- Promotion gate : `sample_count` only counts conclusive outcomes (Match / Mismatch).  ComparatorUnavailable doesn't advance the gate.
- Operator gets alerted via the M0 dashboard if comparator failure rate > 50 %.

### 18.3 F3 : Canary version errors out repeatedly

- Each canary error is recorded as a production trace with `error_kind = Some("AgentError:<kind>")`.
- DriftWindow's `error_rate_bps` ticks up.
- Auto-rollback gate : `error_rate_bps > max_error_rate_bps AND sample_count ≥ min_sample_count` → auto-rollback.
- Cache : N12 entries for the canary version naturally evict (errors don't cache).

### 18.4 F4 : Catalog policy mid-update during dispatch

- Dispatcher calls `catalog.resolve_for_dispatch(...)` at swarm planning time.
- The resolution returns ONE snapshot of the policy.
- All ops in the swarm use the same snapshot (held in the DagPlan).
- A concurrent `set_rollout_policy` updates the catalog ; the next swarm sees the new policy.
- No mid-swarm split brain.

### 18.5 F5 : Shadow cost budget exhausted

- `CostRouter.reserve` returns `Err(BudgetExhausted)`.
- ShadowScheduler logs a warn, increments `shadow_dropped_due_to_budget_total`, returns immediately without spawning.
- No shadow trace is emitted (nothing happened).
- Production unaffected.

### 18.6 F6 : Drift window memory pressure

- DriftWindow holds `max_samples = 1000` per `(tenant, agent_id)`.
- For 10 000 tenants × 100 agents each = 1M windows × 1000 samples × ~200 bytes/sample = ~200 GB.
- That's too much.  Mitigation : DriftWindow's `max_samples` is configurable per tenant tier, default 1000 for tier1 ; 100 for free-tier.
- Cardinality cap : max 10 000 active windows globally.  When exceeded, LRU-evict the least-recently-touched window (the policy for that agent still works, just without drift stats).

### 18.7 F7 : Concurrent `set_rollout_policy` from two operators

- Atomic Mutex on the policy slot.
- Last-write-wins.
- Both writes emit RolloutPolicyChanged audit rows ; the chain shows the sequence.
- No undefined behaviour.

### 18.8 F8 : Promotion when canary version was deleted

- `catalog.get(canary_version)` fails with `VersionNotFound`.
- Promotion pre-flight catches and returns `PromotionError::CanaryVersionDeleted`.
- Policy stays as Canary (with a now-dangling reference) until operator either (a) rollback to prod, or (b) re-publishes the canary version.

### 18.9 F9 : Tenant deleted while policy exists

- M24 GDPR saga cascades : delete agents → delete policies → delete drift windows.
- Implementation : `catalog.delete_tenant(tenant_id)` recursively drops all keyed-by-tenant state.
- Sprint 8 ship : in-memory removal ; Sprint 9 Postgres cascade with FK ON DELETE CASCADE.

### 18.10 F10 : Sticky routing reveals a power-user bias

- Symptom : the 5 % canary cohort happens to be 80 % power users (high-volume tenants).
- Detection : drift stats look fine but production-vs-canary user behaviour analytics show skew.
- Mitigation : bump `STICKY_ROUTING_SALT` from `-v1` to `-v2`.  Re-shuffles every cohort.  Flush idempotency cache for affected agents.

### 18.11 F11 : Comparator agent is itself shadowed

- Sprint 8 : forbidden at policy-set time.  `set_rollout_policy` rejects a Shadow / Canary policy whose `comparison.comparator_agent_id` matches another agent that ALREADY has an active Shadow / Canary policy.
- Sprint 10 : cycle detection allows it, with stricter recursion limit.

### 18.12 F12 : Replay engine encounters a `RolloutPolicyChanged` trace

- The trace is a virtual-swarm one — doesn't belong to a real op.
- Replay engine's filter (§ 13.1) excludes it.
- Replay report doesn't include policy changes ; auditors query the chain directly for those.

---

## 19. Invariants — I1 to I16

The system MUST maintain these properties at all times.

| ID | Invariant | Verification |
|---|---|---|
| I1 | Sticky routing is deterministic — same (tenant, agent, input_hash) → same variant | unit + integration tests |
| I2 | Sticky routing is uniform within ±20 % over 10k samples per bps bucket | histogram test |
| I3 | Tenant isolation — sticky sample for tenant_A doesn't leak to tenant_B | cross-tenant test |
| I4 | Cache key segregates prod from canary (via N12 resolved_version) | already covered by N12 G3 test |
| I5 | Policy snapshot atomicity — a single swarm sees ONE consistent policy | snapshot test |
| I6 | Shadow doesn't affect prod response — async, non-blocking | timing test |
| I7 | Shadow cost ≤ shadow_budget OR shadow is dropped | budget test |
| I8 | Audit chain — shadow + comparison traces are valid chain entries with HMAC | chain verify test |
| I9 | Replay consistency — replay uses the version that originally served | replay-after-rollout test |
| I10 | GDPR cascade — tombstoning a prod trace cascades to all shadow + comparison children | tombstone cascade test |
| I11 | Auto-rollback gate — drift > threshold AND samples ≥ min → rollback | drift detector test |
| I12 | Version-pin opt-out — explicit "=X.Y.Z" bypasses policy | pin-opt-out test |
| I13 | Manual override — operator-initiated rollback always succeeds | rate-limit-bypass test |
| I14 | Comparator self-shadow forbidden | policy-set rejection test |
| I15 | SideEffect agent shadow forbidden | policy-set rejection test |
| I16 | Multi-version legal-transition matrix enforced (§ 5.2) | transition validator test |

---

## 20. Metrics — 9 Prometheus surfaces

```rust
pub struct N13Metrics {
    /// Total shadow runs scheduled (whether or not they completed).
    pub shadow_runs_total: Counter,         // labels : tenant_tier, agent_id

    /// Total shadow runs that produced a Match outcome.
    pub shadow_matches_total: Counter,      // labels : tenant_tier, agent_id

    /// Total shadow runs that produced a Mismatch outcome.
    pub shadow_mismatches_total: Counter,   // labels : tenant_tier, agent_id

    /// Cumulative shadow cost in cents, partitioned by attribution.
    pub shadow_cost_cents_total: Counter,   // labels : attribution=platform|tenant

    /// Total canary-cohort traffic routed (a real prod call on canary version).
    pub canary_routed_total: Counter,       // labels : tenant_tier, agent_id

    /// Gauge : current canary_bps for each active policy.
    pub canary_traffic_bps: Gauge,          // labels : tenant_id, agent_id

    /// Total promotions executed (manual + future-auto).
    pub promotion_total: Counter,           // labels : kind=manual|auto, outcome=pass|block

    /// Total rollbacks executed (manual + auto).
    pub rollback_total: Counter,            // labels : kind=manual|auto

    /// Gauge : current drift_rate_bps for each agent with an active policy.
    pub drift_rate_bps: Gauge,              // labels : tenant_id, agent_id
}
```

### 20.1 Scrape interval + cardinality

- Scrape interval : 30 seconds.
- Cardinality cap : `agent_id` is bounded to ~100 EFE-shipped + ~50 tenant-authored = ~150 ; `tenant_id` is unbounded.
- Per-tenant labels are NOT emitted for high-cardinality counters — instead, per-tenant breakdowns query the metering DB.

### 20.2 Alerts

The M0 ops team gets alerted on :

- `rate(shadow_mismatches_total[5m]) > 0.1 * rate(shadow_matches_total[5m])` — 10 % drift over 5 min.
- `rate(rollback_total{kind="auto"}[15m]) > 5` — more than 5 auto-rollbacks in 15 min suggests a systemic regression.
- `shadow_cost_cents_total{attribution="platform"} > daily_budget * 0.8` — platform shadow budget approaching exhaustion.
- `canary_traffic_bps` stuck > 8000 for > 24h — operator forgot to promote a near-finished canary.

---

## 21. Implementation plan — 5-day breakdown

### 21.1 Day 1 — Types + sticky routing

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano-sdk/src/types/rollout.rs` with `RolloutPolicy` + `ComparisonStrategy` + `CostAttribution` + `PromotionGates` | Tier 1 GREEN |
| founder | `efe-nano/src/dispatcher/routing.rs` with `sticky_sample` + 15 unit tests (uniformity, determinism, tenant isolation) | Tier 1 GREEN |
| founder | `efe-nano-sdk/src/types/rollout.rs` serde round-trip tests | Tier 1 GREEN |

### 21.2 Day 2 — Catalog API extension

| Owner | Deliverable | Verification |
|---|---|---|
| senior eng | `Catalog` trait extension : `resolve_for_dispatch / set_rollout_policy / get_rollout_policy` | Tier 1 GREEN |
| senior eng | `InMemoryCatalog` impl + transition validator + RBAC checks | Tier 1 GREEN |
| senior eng | `tombstone_trace_cascade` extension to `ChainStore` | Tier 1+2 GREEN |

### 21.3 Day 3 — Dispatcher integration

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `dispatcher/shadow.rs` ShadowScheduler + ProdOutcomeSnapshot + spawn semantics | Tier 1+2 GREEN |
| founder | Dispatcher `run_one_op` branching on `ResolvedAgent::ShadowPair` | Tier 1+2 GREEN |
| founder | `runtime::cost_attribution::CostRouter` | Tier 1 GREEN |

### 21.4 Day 4 — Drift + promotion + audit

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `audit::drift::DriftWindow` + `DriftStats` | Tier 1 GREEN |
| founder | `PromotionGateEvaluator` + auto-rollback hook | Tier 2 GREEN |
| founder | `compare_outputs` impl with 4 strategies (Exact, Structural, Semantic, None) | Tier 1+2 GREEN |
| founder | ShadowRun + ShadowComparison + RolloutPolicyChanged trace emission | Tier 2 GREEN |

### 21.5 Day 5 — Integration tests + verification

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `tests/phase_2_n13_shadow_canary.rs` — Tier 3 integration + Tier 4 stress | full suite GREEN |
| both | Customer-authoring MCP tools : `efe_nano_publish_canary` + `promote_canary` + `rollback_canary` + `observe_canary` | M28 tool-bus test |
| both | Final clippy strict + cargo build + 9 prometheus metrics exposed | gate-by-gate |

### 21.6 Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Sticky routing uniformity fails CI on small samples | medium | use Chebyshev-loose bound (±20 % per bucket) ; 10k samples |
| Shadow doubles total platform cost in dev | low | shadow defaults OFF ; only enabled when explicit policy set |
| Drift window OOM on high-cardinality tenants | medium | LRU cap at 10 000 windows globally |
| `Author::Tenant` matching breaks existing tests | low | extensive test coverage of RBAC paths |
| Comparator self-shadow loop | low | catalog rejects at policy-set time |

---

## 22. Test matrix

### 22.1 Tier 1 — unit tests (per module)

| Module | Tests |
|---|---|
| `types/rollout.rs` | Serde round-trip ; PartialEq ; PromotionGates default values |
| `dispatcher/routing.rs` | Sticky determinism ; uniformity ; tenant isolation ; salt versioning |
| `catalog::resolve_for_dispatch` | Stable → Single ; Canary → sample-routed Single ; Shadow → ShadowPair ; version-pin opt-out |
| `catalog::set_rollout_policy` | Legal transitions ; illegal transitions rejected ; RBAC ; cross-tenant rejection |
| `dispatcher/shadow.rs` | ShadowScheduler spawn ; ProdOutcomeSnapshot capture ; cost-router reserve+commit |
| `audit::drift` | DriftWindow record + LRU eviction ; DriftStats computation ; PromotionGateEvaluator |
| `compare_outputs` | Exact match + mismatch ; Structural shape diff ; Semantic comparator ; None skips |

### 22.2 Tier 2 — integration tests

| Test | Description |
|---|---|
| `canary_5pct_routes_exactly_5pct` | 1000 dispatches with `canary_bps=500` ; assert 50 ± 20 land on canary |
| `shadow_does_not_block_prod` | Time prod-only vs shadow-on dispatch ; assert delta < 5 % |
| `promotion_gates_block_on_drift` | Inject mismatches ; gate evaluator returns Block |
| `auto_rollback_on_drift_breach` | Inject drift > threshold ; catalog auto-swaps to Stable(prod) |
| `tombstone_cascade_includes_shadow_children` | Tombstone prod trace ; assert shadow + comparison are tombstoned too |
| `canary_and_prod_cache_entries_coexist` | Dispatch identical input twice in canary cohort + prod cohort ; assert 2 cache entries |
| `replay_filters_shadow_run_traces` | Replay swarm that had shadow ; assert replay's total_ops equals original op count |
| `cost_router_drops_shadow_when_budget_exhausted` | Set budget to 0 ; assert prod succeeds and shadow not spawned |
| `policy_change_cascade_flushes_cache_correctly` | Promote canary ; assert idempotency.flush_for_agent called |

### 22.3 Tier 3 — stress

| Test | Description |
|---|---|
| `stress_5000_canary_dispatches_metrics_match` | 5000 dispatches under `canary_bps=2000` ; assert 1000 ± 50 canary, metrics integrity |
| `stress_concurrent_policy_changes_atomic` | 100 concurrent `set_rollout_policy` ; last-write-wins, no partial state |
| `stress_drift_window_lru_under_cardinality` | Set max_windows = 10 ; touch 50 windows ; assert LRU eviction |

### 22.4 Tier 4 — E2E soak

| Test | Description |
|---|---|
| `e2e_24h_canary_promotion_workflow` | Simulate 24h of dispatches, drift staying below threshold, operator promotes successfully |
| `e2e_24h_canary_auto_rollback` | Simulate 24h with drift > threshold ; assert auto-rollback fires at the right sample count |

---

## 23. Migration path

### 23.1 Pre-N13 state (today)

Catalog stores only `(tenant_id, agent_id, version) → NanoAgent`.  No policies.  Dispatcher uses `catalog.get(NanoAgentRef)` which resolves the version_req to the latest matching version.

### 23.2 N13 day 1 (deploy)

- Catalog binary deployed with new `RolloutPolicy` support.
- Default policy for every agent : `Stable { version: <latest_matching> }` — implicitly created by `get_rollout_policy` if none exists.
- Dispatcher's `resolve_for_dispatch` always returns `Single(latest_matching)` — behaviour is identical to pre-N13.

### 23.3 N13 day 2-3 (first canary)

- An operator (or, Phase 4, a tenant) calls `efe_nano_publish_canary` for ONE agent.
- That agent's policy transitions to `Canary(...)`.  All others remain `Stable(...)`.
- The dispatcher's new code path activates ONLY for that one agent.  Every other agent dispatches as before.

### 23.4 No big-bang migration

The architecture is intentionally **opt-in per agent**.  Existing dispatch paths are not rewritten.  N13 introduces NEW code paths and the catalog routes between old and new.  Rollback : disabling N13 means setting every agent's policy back to `Stable`.

---

## 24. Open questions / ADRs

### ADR-N13-001 : Async shadow as default

**Decision** : shadow runs are scheduled via `tokio::spawn`, not awaited inline.

**Rationale** :

- Mode C shadow can add 100ms-2s latency ; inline shadow doubles caller-visible latency.
- For high-throughput Mode A agents, inline doubles the CPU bill on the hot path.
- Async shadow is acceptable because drift detection is a quality signal, not a hard correctness gate.

**Trade-off** : the comparison result is eventually consistent.  Sprint 9 adds an opt-in `sync_shadow: bool` on the Shadow policy for cost-cap-aware use cases (LLM provider budgets where we want hard same-call comparison before billing).

### ADR-N13-002 : Sticky routing by `input_hash` (not session or random)

**Decision** : the cohort is determined by `sha256(salt || tenant || agent_id || input_hash) % 10_000 < canary_bps`.

**Alternatives considered** :

- **Random** — breaks idempotency cache.
- **Caller session id** — requires every caller to pass an opaque session ; not all callers have sessions.
- **Tenant-uniform** — `tenant_id % 100 < canary_pct` — but then a tenant is either ALL canary or ALL prod, not 5 % of their traffic.
- **Time-based** — `unix_seconds % 100 < canary_pct` — non-deterministic, breaks replay.

`input_hash` is the only choice that preserves idempotency + replay + caller-agnostic + per-tenant uniformity.

### ADR-N13-003 : Cost attribution defaults to `Platform`

**Decision** : for EFE-shipped agents, shadow cost defaults to `Platform` ; for Tenant-authored, defaults to `Tenant`.

**Rationale** : EFE encourages safe rollouts on its own agents by absorbing the cost.  Tenants writing their own agents are responsible for their own bills.

### ADR-N13-004 : Drift window in-memory (Sprint 8) → Postgres (Sprint 9)

**Decision** : Sprint 8 ships in-memory drift window only.  Sprint 9 adds Postgres-backed durability.

**Rationale** : the drift window is a sliding observability signal, not authoritative.  Losing it on process restart means the next K samples are needed before auto-rollback can fire — acceptable graceful degradation.

### ADR-N13-005 : Comparison strategy per agent (not per dispatch)

**Decision** : `ComparisonStrategy` lives on the `RolloutPolicy::Shadow`, not on the SwarmRequest.

**Rationale** : callers don't know which agents are shadowed.  Comparison strategy is an operator concern.

### ADR-N13-006 : `STICKY_ROUTING_SALT` versioning

**Decision** : embed a salt version in the constant (`-v1` today, `-v2` if we ever rotate).

**Rationale** : the cohort selection is the most invasive thing N13 controls.  Bumping the salt is the escape hatch.

### ADR-N13-007 : RolloutPolicyChanged virtual-swarm trace

**Decision** : every policy change emits a `NanoAgentTrace { error_kind: Some("RolloutPolicyChanged"), ... }` that is a real chain entry.

**Rationale** : promotion + rollback are themselves auditable actions ; storing them in the same chain as op-level traces gives tenants a unified audit log.

---

## 25. Annexe A — full type definitions

```rust
// crates/efe-nano-sdk/src/types/rollout.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::types::{NanoAgentId, NanoAgentVersion};

/// **Sprint 8 — N13** : Per-(tenant, agent_id) deployment policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RolloutPolicy {
    Stable {
        version: NanoAgentVersion,
    },
    Shadow {
        prod_version: NanoAgentVersion,
        shadow_version: NanoAgentVersion,
        started_at: DateTime<Utc>,
        comparison: ComparisonStrategy,
        cost_attribution: CostAttribution,
        #[serde(default)]
        shadow_budget_cents: Option<f64>,
        #[serde(default = "shadow_on_cache_hit_default")]
        shadow_on_cache_hit: bool,
    },
    Canary {
        prod_version: NanoAgentVersion,
        canary_version: NanoAgentVersion,
        canary_bps: u16,
        started_at: DateTime<Utc>,
        promotion_gates: PromotionGates,
        cost_attribution: CostAttribution,
    },
}

fn shadow_on_cache_hit_default() -> bool { true }

impl RolloutPolicy {
    pub fn prod_version(&self) -> &NanoAgentVersion {
        match self {
            Self::Stable { version } => version,
            Self::Shadow { prod_version, .. } => prod_version,
            Self::Canary { prod_version, .. } => prod_version,
        }
    }
    pub fn second_version(&self) -> Option<&NanoAgentVersion> {
        match self {
            Self::Stable { .. } => None,
            Self::Shadow { shadow_version, .. } => Some(shadow_version),
            Self::Canary { canary_version, .. } => Some(canary_version),
        }
    }
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Stable { .. } => "stable",
            Self::Shadow { .. } => "shadow",
            Self::Canary { .. } => "canary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComparisonStrategy {
    Exact,
    Structural,
    Semantic { comparator_agent_id: NanoAgentId },
    None,
}

impl ComparisonStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Structural => "structural",
            Self::Semantic { .. } => "semantic",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostAttribution {
    Platform,
    Tenant,
}

impl CostAttribution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Tenant => "tenant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionGates {
    pub min_sample_count: u32,
    pub max_drift_bps: u16,
    pub max_error_rate_bps: u16,
    pub max_latency_p99_delta_pct: u16,
    pub max_cost_delta_pct: u16,
}

impl Default for PromotionGates {
    fn default() -> Self {
        Self {
            min_sample_count: 100,
            max_drift_bps: 100,
            max_error_rate_bps: 50,
            max_latency_p99_delta_pct: 50,
            max_cost_delta_pct: 100,
        }
    }
}

// ─── ShadowOutcome / DriftStats / GateOutcome ───

#[derive(Debug, Clone)]
pub enum ShadowOutcome {
    Match {
        compared_at: DateTime<Utc>,
        similarity: f32,
        prod_latency_ms: u32,
        shadow_latency_ms: u32,
        prod_cost_cents: f64,
        shadow_cost_cents: f64,
    },
    Mismatch {
        compared_at: DateTime<Utc>,
        similarity: f32,
        diff_summary: String,
        prod_latency_ms: u32,
        shadow_latency_ms: u32,
        prod_cost_cents: f64,
        shadow_cost_cents: f64,
    },
    ShadowFailed {
        compared_at: DateTime<Utc>,
        error: String,
    },
    ComparatorUnavailable {
        compared_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct DriftStats {
    pub sample_count: u32,
    pub match_count: u32,
    pub mismatch_count: u32,
    pub failed_shadow_count: u32,
    pub comparator_unavailable_count: u32,
    pub mismatch_rate_bps: u16,
    pub error_rate_bps: u16,
    pub p99_latency_delta_pct: i32,
    pub cost_delta_pct: i32,
}

#[derive(Debug, Clone)]
pub enum GateOutcome {
    Pass,
    Block { reasons: Vec<String> },
}
```

```rust
// crates/efe-nano/src/dispatcher/routing.rs

const STICKY_ROUTING_SALT: &[u8] = b"efe-nano-n13-routing-v1";

pub fn sticky_sample(
    tenant: TenantId,
    agent_id: &NanoAgentId,
    input_hash: &[u8; 32],
) -> u16 {
    let mut h = Sha256::new();
    h.update(STICKY_ROUTING_SALT);
    h.update([0u8]);
    h.update(tenant.0.as_bytes());
    h.update(agent_id.as_str().as_bytes());
    h.update([0u8]);
    h.update(input_hash);
    let digest = h.finalize();
    let raw = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (raw % 10_000) as u16
}
```

---

## 26. Annexe B — Postgres schema (Phase 4)

```sql
-- crates/efe-nano/migrations/v010_rollout_policy.sql

CREATE TABLE rollout_policy (
    tenant_id        UUID NOT NULL,
    agent_id         TEXT NOT NULL,
    policy_json      JSONB NOT NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by       TEXT NOT NULL,         -- actor encoding (operator:<id>, tenant_admin:<id>, ...)
    PRIMARY KEY (tenant_id, agent_id)
);

CREATE INDEX idx_rollout_policy_tenant ON rollout_policy(tenant_id);
CREATE INDEX idx_rollout_policy_kind ON rollout_policy((policy_json->>'kind'));

-- Audit table : every policy change adds a row here, never UPDATEs.
CREATE TABLE rollout_policy_history (
    history_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID NOT NULL,
    agent_id         TEXT NOT NULL,
    from_policy_json JSONB,                  -- NULL on first set
    to_policy_json   JSONB NOT NULL,
    changed_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    changed_by       TEXT NOT NULL,
    reason           TEXT,
    FOREIGN KEY (tenant_id) REFERENCES tenant(tenant_id) ON DELETE CASCADE
);

CREATE INDEX idx_rollout_history_agent ON rollout_policy_history(tenant_id, agent_id, changed_at DESC);

-- Drift window storage (Sprint 9).
CREATE TABLE drift_outcome (
    outcome_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID NOT NULL,
    agent_id         TEXT NOT NULL,
    recorded_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    kind             TEXT NOT NULL,          -- 'match' | 'mismatch' | 'shadow_failed' | 'comparator_unavailable'
    similarity       REAL,
    prod_latency_ms  INT,
    shadow_latency_ms INT,
    prod_cost_cents  NUMERIC(10, 6),
    shadow_cost_cents NUMERIC(10, 6),
    diff_summary     TEXT,
    FOREIGN KEY (tenant_id) REFERENCES tenant(tenant_id) ON DELETE CASCADE
);

CREATE INDEX idx_drift_outcome_tenant_agent_time
    ON drift_outcome(tenant_id, agent_id, recorded_at DESC);

-- Retention : drift outcomes auto-deleted after 30 days.
SELECT cron.schedule(
    'drift_outcome_retention',
    '0 3 * * *',
    'DELETE FROM drift_outcome WHERE recorded_at < NOW() - INTERVAL ''30 days'';'
);
```

---

## 27. Annexe C — customer-authoring MCP tools

### 27.1 `efe_nano_publish_canary`

```json
{
  "name": "efe_nano_publish_canary",
  "description": "Set a Canary rollout policy on an agent you own. Routes a configurable percentage of traffic to the new version.",
  "inputSchema": {
    "type": "object",
    "required": ["agent_id", "prod_version", "canary_version", "canary_bps"],
    "properties": {
      "agent_id": { "type": "string", "description": "Your agent's id" },
      "prod_version": { "type": "string", "pattern": "^\\d+\\.\\d+\\.\\d+", "description": "The currently-live version" },
      "canary_version": { "type": "string", "pattern": "^\\d+\\.\\d+\\.\\d+", "description": "The new version to canary" },
      "canary_bps": { "type": "integer", "minimum": 0, "maximum": 10000, "description": "Percentage in basis-points (100 = 1%)" },
      "comparison": {
        "type": "object",
        "default": { "kind": "structural" },
        "properties": {
          "kind": { "type": "string", "enum": ["exact", "structural", "semantic", "none"] },
          "comparator_agent_id": { "type": "string", "description": "Required when kind=semantic" }
        }
      },
      "promotion_gates": {
        "type": "object",
        "default": {
          "min_sample_count": 100,
          "max_drift_bps": 100,
          "max_error_rate_bps": 50,
          "max_latency_p99_delta_pct": 50,
          "max_cost_delta_pct": 100
        }
      },
      "cost_attribution": { "type": "string", "enum": ["platform", "tenant"], "default": "tenant" }
    }
  }
}
```

### 27.2 `efe_nano_promote_canary`

```json
{
  "name": "efe_nano_promote_canary",
  "description": "Promote the canary version to 100% (replacing prod). Evaluates promotion gates first ; fails with diagnostic if gates block.",
  "inputSchema": {
    "type": "object",
    "required": ["agent_id"],
    "properties": {
      "agent_id": { "type": "string" },
      "force": { "type": "boolean", "default": false, "description": "Bypass gate evaluation. Operator-only." }
    }
  }
}
```

### 27.3 `efe_nano_rollback_canary`

```json
{
  "name": "efe_nano_rollback_canary",
  "description": "Roll back the canary, restoring 100% prod traffic. Always succeeds (no gate evaluation).",
  "inputSchema": {
    "type": "object",
    "required": ["agent_id"],
    "properties": {
      "agent_id": { "type": "string" },
      "reason": { "type": "string", "description": "Optional explanation logged in audit history" }
    }
  }
}
```

### 27.4 `efe_nano_observe_canary`

```json
{
  "name": "efe_nano_observe_canary",
  "description": "Read-only inspection of the current rollout policy + drift stats + gate evaluation.",
  "inputSchema": {
    "type": "object",
    "required": ["agent_id"],
    "properties": {
      "agent_id": { "type": "string" }
    }
  },
  "outputSchema": {
    "type": "object",
    "properties": {
      "policy": { "$ref": "#/definitions/RolloutPolicy" },
      "drift_stats": { "$ref": "#/definitions/DriftStats" },
      "gate_outcome": { "$ref": "#/definitions/GateOutcome" },
      "auto_rollback_armed": { "type": "boolean" }
    }
  }
}
```

---

## 28. Annexe D — sample drift incident playbook

### Scenario

T+0 : Operator publishes a canary for `summarize_strict` at 5 % (`canary_bps=500`).

T+1h : Drift stats show 230 samples, mismatch rate climbing past 200 bps (above the 100 bps threshold).

T+1h05 : Auto-rollback fires.  Policy returns to `Stable(prod_version)`.  Cache flushed.

T+1h06 : Operator gets alerted via the M0 dashboard ; queries `efe_nano_observe_canary` to confirm the rollback happened.

### Investigation

```
operator → MCP : efe_nano_observe_canary { agent_id: "summarize_strict" }
        → {
             policy: { kind: "stable", version: "1.2.3" },
             drift_stats: {
                 sample_count: 230,
                 match_count: 187,
                 mismatch_count: 41,
                 failed_shadow_count: 2,
                 comparator_unavailable_count: 0,
                 mismatch_rate_bps: 1783,   ← 17.83 %, well above threshold
                 ...
             },
             gate_outcome: { kind: "block", reasons: ["drift 17.83 % > 1.00 % threshold"] },
             auto_rollback_armed: false  ← already fired
          }
```

Operator pulls the per-sample drift detail from the chain :

```
SELECT to_policy_json, changed_at, reason
  FROM rollout_policy_history
 WHERE tenant_id = $TENANT AND agent_id = 'summarize_strict'
ORDER BY changed_at DESC LIMIT 5;
```

Returns the auto-rollback audit row with reason `"auto-rollback : drift 17.83 % > 1.00 % threshold @ 230 samples"`.

Operator inspects the ShadowComparison traces for the mismatched samples (chain query by `error_kind = 'ShadowComparison' AND output_hash = mismatch_hash_X`) and sees that the new prompt template added a `urgency_level` JSON field that didn't exist in v1.2.3.

### Recovery

Operator decides the new template's `urgency_level` field is a desired addition, but the comparison strategy (`Structural`) was too strict for this kind of additive change.  They re-publish the canary with :

```
efe_nano_publish_canary {
  ...
  comparison: { kind: "semantic", comparator_agent_id: "starter.summary_diff_judge" },
  promotion_gates: { max_drift_bps: 200, ... },  // more permissive
}
```

The semantic comparator handles "v2 added a key that doesn't break callers" intelligently.

Lockout : the 1-hour post-rollback cooldown applies.  The new canary is queued until T+2h05.

### Lessons codified

After the incident, the operator team adds a tenant-tier-specific default :

- **Tier 1 (production-critical)** : `comparison: Structural`, `max_drift_bps: 50`.
- **Tier 2 (standard)** : `comparison: Structural`, `max_drift_bps: 200`.
- **Tier 3 (experimental)** : `comparison: None`, gates relaxed.

These defaults are surfaced in the MCP tool's `default` JSON schema fields.

---

**End of EFE M50 — N13 Shadow / Canary Catalog Architecture (Sprint 8).**
