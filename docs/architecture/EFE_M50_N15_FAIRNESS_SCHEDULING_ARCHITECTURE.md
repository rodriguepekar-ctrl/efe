# EFE M50 — N15 Fairness Scheduling Architecture

> **Status** — DRAFT (Sprint 8 final, target ship cycle T+14d)
> **Owners** — founder (architecture), senior eng (implementation)
> **Companion docs** —
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8 N11)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8 N12)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8 N13)
> * `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md` (Sprint 8 N14)
> **Sister deliverable** — `EFE_M50_N16_QUOTA_BURST_BORROW_ARCHITECTURE.md` (Sprint 9, planned — work-conserving cross-tenant borrowing)

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Problem statement — the noisy neighbor in EFE Nano](#1-problem-statement--the-noisy-neighbor-in-efe-nano)
2. [Goals & non-goals](#2-goals--non-goals)
3. [Architecture overview](#3-architecture-overview)
4. [Component breakdown — 8 components](#4-component-breakdown--8-components)
5. [`TenantTier` + `QuotaPolicy` — the data model](#5-tenanttier--quotapolicy--the-data-model)
6. [DRR concurrency scheduler — slot-counted resources](#6-drr-concurrency-scheduler--slot-counted-resources)
7. [Token bucket — rate-limited resources](#7-token-bucket--rate-limited-resources)
8. [WFQ swarm intake — bounded-queue resources (Sprint 9 horizon)](#8-wfq-swarm-intake--bounded-queue-resources-sprint-9-horizon)
9. [`AdmissionGate` — the single entry point](#9-admissiongate--the-single-entry-point)
10. [Per-tenant inflight tracker](#10-per-tenant-inflight-tracker)
11. [Executor integration — replaces the global semaphore](#11-executor-integration--replaces-the-global-semaphore)
12. [N11 Firecracker pool integration](#12-n11-firecracker-pool-integration)
13. [N13 shadow scheduler integration](#13-n13-shadow-scheduler-integration)
14. [Audit chain integration](#14-audit-chain-integration)
15. [Backpressure protocol — error surface](#15-backpressure-protocol--error-surface)
16. [Failure modes — F1 to F16](#16-failure-modes--f1-to-f16)
17. [Invariants — I1 to I20](#17-invariants--i1-to-i20)
18. [Metrics — 12 Prometheus surfaces](#18-metrics--12-prometheus-surfaces)
19. [Implementation plan — 5-day breakdown](#19-implementation-plan--5-day-breakdown)
20. [Test matrix](#20-test-matrix)
21. [Migration path](#21-migration-path)
22. [Open questions / ADRs](#22-open-questions--adrs)
23. [Annexe A — full type definitions](#23-annexe-a--full-type-definitions)
24. [Annexe B — Postgres schema (Phase 4)](#24-annexe-b--postgres-schema-phase-4)
25. [Annexe C — runtime config examples](#25-annexe-c--runtime-config-examples)
26. [Annexe D — incident playbook](#26-annexe-d--incident-playbook)

---

## 0. Executive summary

EFE Nano (Sprint 1–8 ship) treats all tenants as equal at the resource layer.  The dispatcher's `max_parallelism` cap is **per-swarm**, the cost cap (N7) is **per-swarm**, the cache (N12) is segregated by tenant but unbounded in throughput, the Firecracker VM pool (N11) allocates first-come-first-served, the LLM client (Mode C) is rate-limited only at the provider level — not at the tenant level.

The result : **classic noisy-neighbor symptoms**.  A single Phase-4 customer running a 10 000-op batch can saturate the Firecracker pool for 10 minutes ; a Mode-C-heavy tenant can exhaust the Anthropic provider quota for a region ; a retry-storm from a misbehaving tenant can drown the audit-chain mutex.  Every other tenant pays the latency tax.

**N15 ships a complete multi-resource fair scheduler** sitting between the API gateway and the dispatcher.  Eight commitments :

1. **`TenantTier` enum** — `Free`, `Tier1`, `Tier2`, `Tier3`, `Enterprise` — drives weight, quotas, and priority across every resource.
2. **`QuotaPolicy`** — per-tier configuration of concurrency caps, LLM rates, queue depths, audit QPS, VM pool share.
3. **`DrrScheduler<TenantId>`** — Deficit Round Robin for slot-counted resources (CPU semaphore, VM pool, in-flight ops).  Weighted, work-conserving, anti-starvation.
4. **`TokenBucket<(TenantId, ProviderId)>`** — per-tenant per-provider rate limiter for LLM tokens-per-second.  Refill = tenant weight × global rate.
5. **`AdmissionGate`** — single entry point in front of `Dispatcher::dispatch` that decides if a swarm can be admitted ; returns `BackpressureError { kind, retry_after_ms }` otherwise.
6. **`InflightTracker`** — per-tenant authoritative inflight count + queue depth.  Source of truth for metrics + admission decisions.
7. **`BackpressureError`** SDK-level error type with HTTP 429 + `Retry-After`-mapped response so clients can implement exponential backoff.
8. **12 Prometheus metric surfaces** — per-tenant admissions / denials / queue depth / inflight / wait time / bucket levels.

**Integration story with the just-shipped neighbours** :

- **N11 Firecracker pool** — the `KindPool`'s `acquire()` now goes through the `DrrScheduler` so VM allocation is weighted by tenant.
- **N12 idempotency cache** — cache hits release their admission ticket immediately (sub-ms) so a 100 % cache-hit traffic burst doesn't even register against the tenant's slot quota.
- **N13 shadow scheduler** — shadow runs use a SEPARATE admission lane (`Platform`-attributed) so shadow doesn't compete with prod for tenant slots.
- **N14 swarm envelope** — the envelope's `op_status_counts` gains a `denied_by_admission` field (Sprint 9 schema bump) so per-swarm fairness telemetry is auditable.

Verification gate (T+14d) :

| ID | Commitment | Test |
|---|---|---|
| C1 | TenantTier + QuotaPolicy round-trip via serde | `quota_policy_serde` GREEN |
| C2 | DRR scheduler — weighted fair share over 1000 acquires | `drr_weighted_fair_share_within_5pct` GREEN |
| C3 | Token bucket — refill rate ± 5 % of nominal | `token_bucket_refill_within_5pct` GREEN |
| C4 | AdmissionGate denies on full inflight | `admission_denies_when_inflight_full` GREEN |
| C5 | AdmissionGate denies on full queue | `admission_denies_when_queue_full` GREEN |
| C6 | BackpressureError carries retry_after_ms | `backpressure_carries_retry_hint` GREEN |
| C7 | Anti-starvation — tenant with deficit gets a slot within K iterations | `anti_starvation_floor_within_k` GREEN |
| C8 | Tier upgrade rebalances immediately | `tier_upgrade_rebalances` GREEN |
| C9 | N11 Firecracker pool : tenant share enforced | `firecracker_pool_respects_tenant_quota` GREEN |
| C10 | N12 cache hits don't consume slots | `cache_hits_release_admission_immediately` GREEN |
| C11 | N13 shadow runs use Platform lane | `shadow_uses_platform_admission_lane` GREEN |
| C12 | 12 Prometheus surfaces exposed via `/metrics` | metric exposure test |

---

## 1. Problem statement — the noisy neighbor in EFE Nano

### 1.1 Six observed incident patterns

**P1 — The batch monopolist.**  Customer T submits a `SwarmRequest` with 10 000 ops.  Dispatcher's `max_parallelism = 100`.  T's swarm runs for ~10 minutes occupying 100 concurrent slots.  Other tenants' swarms wait behind T in `tokio::JoinSet` queue or get starved on the shared `tokio` executor.

**P2 — The LLM provider hog.**  Customer T sends 5 000 Mode C requests in 60 seconds.  Anthropic provider rate-limits the platform's API key.  Mode C requests for other tenants get HTTP 429 from Anthropic for ~5 minutes.

**P3 — The Firecracker pool squatter.**  N11 ships 64 microVM slots across all tenants.  Customer T's batch-NER workload monopolises all 64 slots for 8 minutes.  Other tenants' Mode B requests sit in `acquire()` queue.

**P4 — The retry storm.**  Customer T's swarm has `retry_policy.max_attempts = 5`.  20 % of T's ops fail and retry, each retry consuming a fresh slot.  Effective concurrency multiplies 1.6× for T.  Other tenants see less than their fair share.

**P5 — The audit-chain floods.**  Sprint 6 ship serializes per-tenant audit writes via `Mutex<chain_tip>`.  Customer T's swarm produces 10 000 trace appends in 30 seconds.  Other tenants writing to the chain wait behind T's mutex queue.

**P6 — The cache-poisoning DOS.**  Customer T crafts 1 000 inputs that produce cache misses (N12 9-component discriminator).  Each miss pays the full dispatch cost.  T's idempotency-cache evictions thrash the DashMap.  Other tenants see their cache hit rate degrade.

### 1.2 What pre-N15 mitigations look like (insufficient)

* **Per-swarm `max_parallelism`** — but T can submit 100 swarms in parallel, each at the cap.
* **N7 swarm-wide `cost_cap_cents`** — economic bound, not resource bound.  T can still saturate VM pool even while staying under cost cap.
* **Per-provider HTTP rate-limiting** — global, not per-tenant.  T gets the slowdown but so does everyone else.
* **Per-tenant cache (`flush_for_tenant`)** — defensive ; doesn't prevent the flood.

None of these address the FAIRNESS dimension : "no tenant should be able to cause sustained degradation of another tenant's latency / availability".

### 1.3 Why now ?

Three concurrent drivers :

1. **Phase 4 customer authoring** (M50.4) — tenants run their own agents at their own volumes ; we cannot trust the input distribution.
2. **Mode C cost scale** — Mode C reaches 90 % of compute cost ; LLM provider quotas are FINITE shared resources.
3. **Enterprise SLA contracts** — Sprint 9 GTM commits "Enterprise tier guaranteed 99.9 % availability + < 1 s p99 for prod ops".  Without isolation, an SLA violation by one tenant breaks the contract for all.

### 1.4 Three resource classes

The fix must distinguish between three classes of shared resources :

| Class | Examples | Scheduling primitive |
|---|---|---|
| **Slot-counted** | CPU semaphore, microVM pool, in-flight op count, audit-write mutex | **DRR** (deficit round-robin) |
| **Rate-limited** | LLM provider RPM, audit-write QPS, M28 event-bus emit rate | **Token bucket** per (tenant, resource) |
| **Queue-bounded** | Incoming swarm queue, retry queue | **WFQ** (Sprint 9) — Sprint 8 ships fixed-depth + reject |

Different algorithms because the natural semantics differ.

---

## 2. Goals & non-goals

### 2.1 Sprint 8 N15 commitments — the 12 we ship

| ID | Goal | Surface | Verification |
|---|---|---|---|
| G1 | `TenantTier` enum + `QuotaPolicy` serde round-trip | `efe_nano_sdk::TenantTier` / `QuotaPolicy` | round-trip test |
| G2 | `DrrScheduler<TenantId>` — weighted fair share + work-conserving | `efe_nano::scheduler::DrrScheduler` | 1k-acquire fairness test |
| G3 | `TokenBucket<(TenantId, ProviderId)>` — refill rate accurate ± 5 % | `efe_nano::scheduler::TokenBucket` | timing test |
| G4 | `AdmissionGate::try_admit(tenant, request)` returns `AdmissionTicket` or `BackpressureError` | `efe_nano::scheduler::AdmissionGate` | unit + integration |
| G5 | `AdmissionTicket` RAII — releases on drop | (Drop impl) | leak test |
| G6 | `InflightTracker` per-tenant authoritative counters | `efe_nano::scheduler::InflightTracker` | concurrency test |
| G7 | `BackpressureError { kind, retry_after_ms, advice }` — SDK error type | `efe_nano_sdk::BackpressureError` | error-shape test |
| G8 | Anti-starvation : tenant with pending work gets a slot within `min(max_starvation_iterations, K)` | DRR algorithm | invariant test |
| G9 | N11 Firecracker pool — `acquire()` routed through DRR | `efe_nano::runtime::pool::firecracker::pool` | integration test |
| G10 | N12 cache hits release admission ticket immediately | `dispatcher/executor.rs` | timing test |
| G11 | N13 shadow runs use Platform-attributed lane | `dispatcher/shadow.rs` | integration test |
| G12 | 12 Prometheus metric surfaces | `efe_nano::scheduler::metrics` | exposure test |

### 2.2 Sprint 8 N15 non-goals (deferred)

| ID | Deferred capability | Sprint to ship |
|---|---|---|
| N1 | **WFQ swarm intake queue** — Sprint 8 ships fixed-depth FIFO with reject | Sprint 9 |
| N2 | **Cross-tenant work-conserving token borrow** — Tier 1 cannot borrow from Tier 3 idle bucket | Sprint 9 (N16) |
| N3 | **Hierarchical scheduling** — organization-level above tenant | Sprint 10 |
| N4 | **Dynamic tier promotion** — auto-upgrade based on usage history | Sprint 10 |
| N5 | **Cross-region fairness** — Sprint 8 is single-region | Phase 4 deployment |
| N6 | **Multi-armed-bandit weight tuning** | Sprint 12+ |
| N7 | **Tenant suspension UI** | Phase 4 admin surface |
| N8 | **Token bucket persistence** — Sprint 8 in-memory ; loses state on restart | Sprint 9 Redis backend |

---

## 3. Architecture overview

### 3.1 The 5-layer diagram

```
┌────────────────────────────────────────────────────────────────────────┐
│                       Client / API Gateway                              │
│                                                                         │
│    SwarmRequest                                                         │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Layer 1 :  AdmissionGate                                        │    │
│  │                                                                 │    │
│  │    gate.try_admit(tenant_id, &request)                          │    │
│  │      ├── consult policy : per-tenant queue depth check          │    │
│  │      ├── consult inflight tracker : per-tenant slot count       │    │
│  │      ├── consult global cap : platform-wide concurrency         │    │
│  │      └── on success → AdmissionTicket (RAII)                    │    │
│  │      └── on denial → BackpressureError { retry_after_ms, ... }  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│         │                                                               │
│         ▼ (ticket held)                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Layer 2 :  DrrScheduler — per-op slot acquisition                │    │
│  │                                                                 │    │
│  │    For each op in the swarm :                                   │    │
│  │      drr.acquire(tenant_id) → DrrPermit                         │    │
│  │      ├── if tenant deficit ≥ 1 : grant immediately              │    │
│  │      ├── else : block on cohort scheduler tick                  │    │
│  │      └── weighted by TenantTier                                 │    │
│  │                                                                 │    │
│  │    Anti-starvation : tenant deficit increases per round          │    │
│  │    by quantum × weight ; oldest tenant always gets next pick.   │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│         │                                                               │
│         ▼ (slot held)                                                   │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Layer 3 :  TokenBucket — Mode C LLM rate limiting               │    │
│  │                                                                 │    │
│  │    bucket.acquire(tenant_id, provider_id, tokens_needed)        │    │
│  │      ├── if bucket has enough : decrement + grant               │    │
│  │      ├── else : compute wait_ms, return error or sleep          │    │
│  │      └── refill rate = tier_weight × global_rate / Σ weights    │    │
│  │                                                                 │    │
│  │    Per (tenant, provider) bucket.  Global bucket above (Sprint  │    │
│  │    9 work-conserving cross-tenant borrow).                      │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Layer 4 :  Dispatcher — existing N1-N14 path                    │    │
│  │                                                                 │    │
│  │    Dispatcher::dispatch(request)                                │    │
│  │      ├── planner                                                │    │
│  │      ├── executor.execute_plan_with_extras_n14                  │    │
│  │      │     └── per-op : reads admission ticket + DRR permit     │    │
│  │      ├── shadow scheduler (N13)                                 │    │
│  │      └── envelope writer (N14)                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Layer 5 :  Metrics + observability                              │    │
│  │                                                                 │    │
│  │    - per_tenant_admissions_total                                │    │
│  │    - per_tenant_admissions_denied_total                         │    │
│  │    - per_tenant_inflight_current                                │    │
│  │    - per_tenant_queue_depth_current                             │    │
│  │    - per_tenant_token_bucket_level_current                      │    │
│  │    - per_tenant_drr_deficit_current                             │    │
│  │    - admission_wait_ms_histogram                                │    │
│  │    - ... (12 surfaces total)                                    │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────────────────┘
```

### 3.2 The shape of the change

| Layer | Today (pre-N15) | After N15 |
|---|---|---|
| API gateway | Calls `Dispatcher::dispatch` directly | Calls `AdmissionGate::try_admit` first ; if Ok, dispatches with ticket |
| Dispatcher | Uses `tokio::Semaphore::new(max_parallel)` globally | Uses `DrrScheduler<TenantId>` ; ticket pinned to swarm lifetime |
| Mode C client | Per-provider HTTP rate limit only | Per-tenant `TokenBucket` AND per-provider HTTP rate limit |
| N11 Firecracker pool | FIFO acquire | Tenant-weighted via `DrrScheduler` adapter |
| Audit chain | Per-tenant mutex | Same (Sprint 8 ship) ; Sprint 9 may add per-tier queue priority |
| Error surface | `DispatcherError { ... }` only | New `BackpressureError { kind, retry_after_ms, advice }` |
| Metrics | 14 metric surfaces (N12 + N13 + N14) | + 12 N15 surfaces = 26 |
| SwarmResponse | Unchanged | Sprint 9 adds optional `fairness_telemetry: Option<FairnessTelemetry>` field |

---

## 4. Component breakdown — 8 components

### 4.1 `TenantTier` enum (SDK)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantTier {
    /// Trial / free tier.  Hardest quotas.  May be deprioritized
    /// during platform load.
    Free,
    /// Paid tier 1 — low volume.  Weight 1.
    Tier1,
    /// Paid tier 2 — medium volume.  Weight 5.
    Tier2,
    /// Paid tier 3 — high volume.  Weight 20.
    Tier3,
    /// Enterprise contract — custom SLA.  Weight 100.
    Enterprise,
    /// Platform / operator-internal tenant.  Highest priority
    /// (above Enterprise).  Used for system tasks (catalog
    /// maintenance, audit chain verification jobs).
    Platform,
}
```

The `Platform` variant is reserved : it's the tenant used by EFE's own internal services (e.g. the M40 metrics scraper, the M24 GDPR saga).  Its quota is unbounded.

### 4.2 `QuotaPolicy` (SDK)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaPolicy {
    /// Tier this policy applies to.
    pub tier: TenantTier,

    /// Weight used by DRR scheduler.  Higher = larger share.
    pub drr_weight: u32,

    /// Max concurrent ops per tenant across all swarms.
    pub max_inflight_ops: u32,

    /// Max concurrent swarms per tenant.
    pub max_inflight_swarms: u32,

    /// Max pending requests in the swarm intake queue.
    pub swarm_queue_depth: u32,

    /// Per-tenant LLM tokens-per-second for each provider.
    pub llm_tokens_per_second: f64,

    /// Per-tenant audit-write QPS limit.
    pub audit_writes_per_second: f64,

    /// Per-tenant Firecracker pool share (basis points : 1 bp =
    /// 0.01 % of platform pool).
    pub firecracker_pool_share_bps: u16,

    /// Anti-starvation floor : the maximum number of DRR rounds a
    /// tenant with pending work waits before being granted at
    /// least one slot.
    pub max_starvation_rounds: u32,
}

impl QuotaPolicy {
    pub fn free() -> Self { ... }
    pub fn tier1() -> Self { ... }
    pub fn tier2() -> Self { ... }
    pub fn tier3() -> Self { ... }
    pub fn enterprise() -> Self { ... }
    pub fn platform() -> Self { ... }
}
```

Default presets per tier :

| Tier | drr_weight | max_inflight_ops | max_inflight_swarms | swarm_queue_depth | llm_tokens_per_second |
|---|---:|---:|---:|---:|---:|
| Free | 1 | 5 | 1 | 4 | 50 |
| Tier1 | 1 | 20 | 5 | 16 | 200 |
| Tier2 | 5 | 100 | 25 | 64 | 1 000 |
| Tier3 | 20 | 500 | 100 | 256 | 5 000 |
| Enterprise | 100 | 2 500 | 500 | 1 024 | 25 000 |
| Platform | ∞ | ∞ | ∞ | ∞ | ∞ |

These numbers are tunable per deployment (env var override Sprint 9).

### 4.3 `DrrScheduler<TenantId>` (efe-nano)

```rust
pub struct DrrScheduler {
    /// Per-tenant state, lazily created on first acquire.
    state: DashMap<TenantId, TenantDrrState>,
    /// Global concurrency cap : the maximum number of slots
    /// granted at once across all tenants.
    global_capacity: u32,
    /// Quantum added to deficit per round.
    quantum: u32,
    /// Currently-held slots (across all tenants).
    inflight: AtomicU32,
    /// Notifier for waiting acquirers.
    wake: Notify,
    /// Per-tenant quota lookup (TenantTier → QuotaPolicy).
    policy_lookup: Arc<dyn TenantPolicyLookup>,
}

struct TenantDrrState {
    deficit: i64,                 // signed for fairness math
    last_round_active: u64,       // round counter for anti-starvation
    inflight: u32,                // slots currently held by this tenant
    weight: u32,                  // copied from QuotaPolicy.drr_weight
    max_inflight: u32,            // copied from QuotaPolicy.max_inflight_ops
}
```

`DrrScheduler::acquire(tenant) -> DrrPermit` :

1. Look up tenant state (lazy-init from policy).
2. If `inflight >= max_inflight` for this tenant : return `Err(BackpressureError::TenantInflightFull)`.
3. If `state.inflight >= state.max_inflight` : return error.
4. If `global.inflight >= global.capacity` : block via `Notify` ; wake on release.
5. Once admitted : increment `state.inflight` + `global.inflight`, decrement `deficit` by 1.
6. Return `DrrPermit` (RAII drop releases the slot).

`DrrPermit::drop()` :
1. Decrement `state.inflight` + `global.inflight`.
2. Notify the next waiter (oldest-deficit-first).

Per-round bookkeeping :
- Every K acquires (e.g. K = `global.capacity`), advance a round.
- For each tenant with `inflight > 0` OR pending waiters : add `weight × quantum` to deficit.
- Reset round counter, wake oldest waiter.

### 4.4 `TokenBucket<(TenantId, ProviderId)>` (efe-nano)

```rust
pub struct TokenBucket {
    /// (tenant, provider) → state.
    buckets: DashMap<(TenantId, ProviderId), BucketState>,
    /// Default per-tenant refill rate (tokens/sec) — lookup by tier.
    policy_lookup: Arc<dyn TenantPolicyLookup>,
    /// Global rate limiter shared by all tenants (provider RPM).
    global_buckets: DashMap<ProviderId, BucketState>,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
    capacity: f64,         // bucket max
    refill_per_sec: f64,
}
```

`TokenBucket::acquire(tenant, provider, count) -> Result<(), BackpressureError>` :

1. Refill the (tenant, provider) bucket to current wall-clock.
2. If `tokens >= count` : decrement + grant.
3. Else : compute wait_ms = (count - tokens) / refill_per_sec × 1000.
4. Return `Err(BackpressureError::LlmRateLimited { retry_after_ms: wait_ms as u32 })`.

Optionally, the caller can choose to `acquire_blocking(...)` and sleep instead — Sprint 9 deliverable.

### 4.5 `AdmissionGate` (efe-nano)

The single entry point for `SwarmRequest` admission.

```rust
pub struct AdmissionGate {
    drr: Arc<DrrScheduler>,
    tracker: Arc<InflightTracker>,
    policy_lookup: Arc<dyn TenantPolicyLookup>,
    global_cap: AdmissionGlobalCap,
}

pub struct AdmissionGlobalCap {
    pub max_concurrent_swarms: u32,
    pub max_pending_swarms: u32,
}

impl AdmissionGate {
    pub async fn try_admit(
        &self,
        tenant: TenantId,
        request: &SwarmRequest,
    ) -> Result<AdmissionTicket, BackpressureError> { ... }
}
```

`try_admit` decision tree :

1. **Inflight swarms cap** : if `tracker.inflight_swarms(tenant) >= policy.max_inflight_swarms` → `BackpressureError::SwarmInflightFull`.
2. **Queue depth** : if `tracker.queue_depth(tenant) >= policy.swarm_queue_depth` → `BackpressureError::QueueFull`.
3. **Global capacity** : if `tracker.global_inflight_swarms() >= global_cap.max_concurrent_swarms` → `BackpressureError::PlatformBusy`.
4. **Tenant inflight ops** : if `request.op_count > policy.max_inflight_ops - tracker.inflight_ops(tenant)` → `BackpressureError::OpInflightFull` (the swarm wouldn't fit).
5. **Admit** : increment tracker counters ; return `AdmissionTicket` (RAII).

`AdmissionTicket::drop()` decrements counters.

### 4.6 `InflightTracker` (efe-nano)

```rust
pub struct InflightTracker {
    per_tenant: DashMap<TenantId, TenantInflight>,
    global_inflight_swarms: AtomicU32,
    global_inflight_ops: AtomicU32,
}

struct TenantInflight {
    inflight_swarms: AtomicU32,
    inflight_ops: AtomicU32,
    queue_depth: AtomicU32,        // pending swarms not yet started
    cumulative_admissions: AtomicU64,
    cumulative_denials: AtomicU64,
}
```

Source of truth for :
- Admission decisions
- Metric reporting
- M0 dashboard "tenant snapshot" queries

### 4.7 `BackpressureError` (SDK)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("backpressure : {kind:?} (retry after {retry_after_ms} ms)")]
pub struct BackpressureError {
    pub kind: BackpressureKind,
    pub retry_after_ms: u32,
    pub tenant_id: TenantId,
    /// Operator-visible advice on what to do.  Surfaces in
    /// HTTP response body so client developers see actionable
    /// text (not just an opaque error code).
    pub advice: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressureKind {
    /// Tenant's max concurrent swarms reached.
    SwarmInflightFull,
    /// Tenant's pending swarm queue is full.
    QueueFull,
    /// Tenant's max concurrent ops reached.
    OpInflightFull,
    /// Platform-wide capacity reached.  All tenants affected.
    PlatformBusy,
    /// LLM provider quota exhausted for tenant.
    LlmRateLimited,
    /// Audit-write quota exhausted for tenant.
    AuditRateLimited,
    /// Firecracker microVM pool share exhausted for tenant.
    FirecrackerPoolFull,
    /// Tenant is suspended (operator action).
    TenantSuspended,
}
```

### 4.8 `metrics` module (efe-nano)

```rust
pub struct SchedulerMetrics {
    pub per_tenant_admissions_total: Counter,         // labels : tenant_tier
    pub per_tenant_admissions_denied_total: Counter,  // labels : tenant_tier, kind
    pub per_tenant_inflight_swarms_current: Gauge,    // labels : tenant_id
    pub per_tenant_inflight_ops_current: Gauge,       // labels : tenant_id
    pub per_tenant_queue_depth_current: Gauge,        // labels : tenant_id
    pub per_tenant_token_bucket_level_current: Gauge, // labels : tenant_id, provider_id
    pub per_tenant_drr_deficit_current: Gauge,        // labels : tenant_id
    pub admission_wait_ms_histogram: Histogram,
    pub drr_acquire_wait_ms_histogram: Histogram,
    pub token_bucket_acquire_wait_ms_histogram: Histogram,
    pub global_inflight_swarms_current: Gauge,
    pub global_inflight_ops_current: Gauge,
}
```

12 surfaces total.  Per-tenant labels are bounded by `tenant_id` cardinality (max 100k in Phase 4).

---

## 5. `TenantTier` + `QuotaPolicy` — the data model

### 5.1 Why tier-based, not per-tenant ?

A naive approach would be a per-tenant `QuotaPolicy` row.  But :

1. **Operational cost** — 100k tenants × custom policies = ops nightmare.
2. **Pricing signal** — tier matches the billing model.  Tier change is the lever.
3. **Sprint 9 hierarchy** — organization above tenant can override at org level (Sprint 10).

Per-tenant overrides ARE supported via the `tenant_policy_override` table (Sprint 9 deliverable, see Annexe B).  Sprint 8 ships tier-only.

### 5.2 Where is `TenantTier` stored ?

`TenantTier` lives in the M2 billing service (out of scope for this doc).  EFE Nano consults it via the `TenantPolicyLookup` trait :

```rust
#[async_trait]
pub trait TenantPolicyLookup: Send + Sync + 'static {
    async fn lookup(&self, tenant: TenantId) -> QuotaPolicy;
}
```

Sprint 8 ships :
- `InMemoryTenantPolicyLookup` — `HashMap<TenantId, TenantTier>` + defaults.
- `BillingServiceLookup` (Sprint 9) — HTTP call to M2.

Tier changes are pushed to the lookup via M28 event-bus events ; cache TTL is 60 seconds.

### 5.3 Tier weight rationale

Weights are deliberately chosen with **5×, 4×, 5× ratios** between successive tiers :

* Free → Tier1 : 1× — Free is essentially same weight as Tier1 (just lower caps).
* Tier1 → Tier2 : 5× — meaningful "pay more, get more" jump.
* Tier2 → Tier3 : 4× — gradual.
* Tier3 → Enterprise : 5× — final jump.

The total weight ratio Enterprise / Free = 100×.  Under contention, an Enterprise tenant gets 100× the slot share of a Free tenant.  At low load, every tenant gets the same throughput (work-conserving — no slot wasted).

### 5.4 What changes the policy ?

* **Tier upgrade** (M0 dashboard) — sets new TenantTier in M2 ; propagates via event bus ; takes effect within 60 s.
* **Operator override** (Phase 4) — direct policy table entry overrides tier defaults.
* **Suspension** (admin tool) — sets `tier = Suspended` (an internal-only variant ; quota = 0 across the board).

---

## 6. DRR concurrency scheduler — slot-counted resources

### 6.1 Why DRR (not WFQ) for concurrency ?

Three reasons :

1. **Constant-time per acquire** — DRR is O(1) per dequeue (vs WFQ's O(log N)).  Hot path.
2. **Simple to reason about** — deficit counter + quantum + weight ; no virtual finish times.
3. **Work-conserving by construction** — idle tenants accumulate deficit ; the next active tenant inherits the slot.

WFQ is theoretically better for FAIRNESS GUARANTEES (provable share within a constant of perfect).  DRR's bound is `quantum × max_packet_size`.  For our use case (each "packet" is a uniform slot acquisition), the bound is exactly `quantum`.

### 6.2 Algorithm

```
init:
  for each tenant t:
    deficit[t] = 0
    inflight[t] = 0
  active_ring = []   # round-robin list of tenants with pending acquires

on acquire(tenant t):
  if inflight[t] >= policy[t].max_inflight: return Err(TenantInflightFull)
  if global_inflight >= global_capacity:
    enqueue (t, waker)
    block until woken
  inflight[t] += 1
  global_inflight += 1
  deficit[t] -= 1
  return Ok(DrrPermit { tenant: t })

on release(tenant t):
  inflight[t] -= 1
  global_inflight -= 1
  schedule_next()

schedule_next():
  # round-robin through tenants with pending waiters
  if no pending waiters: return
  loop:
    pick t = active_ring.next()
    if t has pending waiter AND deficit[t] >= 1:
      wake one waiter for t
      break
    # tenant skipped because deficit too low.  Add quantum.
    deficit[t] += policy[t].drr_weight × quantum
    # Anti-starvation : if t's age in queue > max_starvation_rounds, force-grant.
    if waiter_age_rounds(t) > policy[t].max_starvation_rounds:
      wake one waiter for t
      deficit[t] -= 1  # account for forced grant
      break
```

### 6.3 Anti-starvation guarantee

By design, after `max_starvation_rounds × global_capacity` acquires, EVERY tenant with a pending waiter will have been served at least once.

Proof : in the worst case, a low-weight tenant accumulates `quantum × weight` deficit per round.  After `max_starvation_rounds` rounds, the tenant has accumulated `weight × quantum × rounds` deficit.  Since `weight ≥ 1` and `quantum ≥ 1`, the deficit reaches at least `1` after `max_starvation_rounds` rounds, granting at least one acquire.

The forced-grant branch (`waiter_age_rounds > max_starvation_rounds`) is a SAFETY NET — in normal operation, the deficit mechanism alone is sufficient.

### 6.4 Hot path complexity

* `acquire` (success path) : O(1) atomic increment + DashMap lookup ≈ 50 ns.
* `acquire` (block path) : O(1) enqueue + park ≈ 100 ns + sleep.
* `release` : O(1) atomic decrement + Notify wake ≈ 100 ns.

For 10k acquires/sec across all tenants, scheduler overhead is < 1 % of wall-clock.

### 6.5 Why a single global scheduler instead of per-Dispatcher ?

**Decision** : one `DrrScheduler` per platform deployment, shared across all `Dispatcher` instances.

Alternative : per-Dispatcher scheduler with each request hitting one specific instance.

Rationale : tenant fairness must hold across the entire platform, not per dispatcher pod.  Sprint 8 ships single-pod ; Sprint 10 distributed deploys may shard but with a global coordination layer.

---

## 7. Token bucket — rate-limited resources

### 7.1 Why token bucket

For per-second rate limits (LLM provider RPM, audit-write QPS), the token bucket is the textbook primitive :
- **Burst tolerance** : capacity > refill rate allows short bursts to drain the bucket then catch up.
- **Smooth average** : long-term rate ≤ refill rate.
- **Cheap** : O(1) per acquire.

### 7.2 Algorithm

```
struct Bucket:
  tokens: f64
  last_refill: Instant
  capacity: f64
  refill_per_sec: f64

acquire(tokens_needed: f64) -> Result<(), wait_ms>:
  refill_to_now()
  if tokens >= tokens_needed:
    tokens -= tokens_needed
    return Ok(())
  shortfall = tokens_needed - tokens
  wait_secs = shortfall / refill_per_sec
  return Err(wait_secs × 1000)

refill_to_now():
  now = Instant::now()
  elapsed = now - last_refill
  tokens = min(capacity, tokens + elapsed.as_secs_f64() × refill_per_sec)
  last_refill = now
```

### 7.3 Per-tenant bucket sizing

```
capacity = refill_per_sec × burst_seconds
```

Default `burst_seconds = 10` (sustained 10s burst tolerated above the rate).  Configurable per tier.

### 7.4 Mode C integration

`crates/efe-nano/src/runtime/llm_client.rs` wraps each LLM request :

```rust
async fn call_llm(...) -> Result<Response, NanoAgentError> {
    let tokens_needed = estimate_tokens(&request);
    token_bucket
        .acquire(tenant, provider, tokens_needed)
        .map_err(|wait_ms| NanoAgentError::Backpressure {
            retry_after_ms: wait_ms,
        })?;
    // ... actual LLM call ...
}
```

The acquire fails with `BackpressureError::LlmRateLimited` — the dispatcher catches and surfaces.

### 7.5 Why per-tenant per-provider (and not just per-tenant) ?

A tenant might use Anthropic AND OpenAI simultaneously.  Each provider has its own rate limit.  Per-(tenant, provider) bucket isolates them : Anthropic provider outage doesn't penalize the tenant's OpenAI quota.

### 7.6 Global bucket — Sprint 8 stub, Sprint 9 redistribution

Sprint 8 ships per-(tenant, provider) buckets ONLY.  No global bucket above.  This is conservative : no tenant can borrow from another tenant's idle bucket.

Sprint 9 N16 adds a HIERARCHICAL bucket : platform-wide bucket above tenant buckets, with work-conserving borrow.  If tenant A is idle, tenant B can temporarily exceed their share up to the platform cap.

---

## 8. WFQ swarm intake — bounded-queue resources (Sprint 9 horizon)

### 8.1 Sprint 8 ship : fixed-depth FIFO with reject

Per-tenant swarm queue (`tracker.queue_depth(tenant)`) is bounded by `policy.swarm_queue_depth`.  When full → `BackpressureError::QueueFull`.  No reordering — first-come-first-served.

### 8.2 Sprint 9 deliverable : WFQ

Sprint 9 introduces a virtual-finish-time per swarm in queue.  Dequeue smallest-VFT-first.  This achieves :
- Two tenants with equal weight take turns (no Tetris artifacts).
- Higher-weight tenant gets more frequent dequeues.
- Caller_agent_id-aware priority (e.g. `caller="claude-sonnet-4-5"` gets +X weight if the tenant's caller-agent contract specifies).

For Sprint 8 ship, FIFO + queue depth cap is sufficient.  Documented as a known suboptimality.

---

## 9. `AdmissionGate` — the single entry point

### 9.1 The contract

```rust
impl AdmissionGate {
    pub async fn try_admit(
        &self,
        tenant: TenantId,
        request: &SwarmRequest,
    ) -> Result<AdmissionTicket, BackpressureError>;
}
```

* **Idempotent** : multiple calls with the same `request.request_id` return the same admission decision (Sprint 9 adds idempotency cache).
* **Non-blocking** : `try_admit` returns immediately ; no internal sleep.
* **Resource-honest** : consumes resources (counters) on Ok ; releases on Drop (RAII).

### 9.2 When is `try_admit` called ?

In `Dispatcher::dispatch` at the very start, BEFORE planning :

```rust
pub async fn dispatch(&self, request: SwarmRequest) -> Result<SwarmResponse, DispatcherError> {
    let ticket = self.admission.try_admit(request.tenant_id, &request).await
        .map_err(DispatcherError::Backpressure)?;
    // ticket is held for the entire dispatch lifetime ; released on Drop.

    // ... existing N1-N14 dispatch logic ...
}
```

### 9.3 What if there's no `AdmissionGate` ?

When the Dispatcher's `admission_gate` field is `None`, `dispatch` skips admission and proceeds as today.  This preserves backward compatibility for harnesses / unit tests that don't wire fairness scheduling.

Production deploys MUST install an `AdmissionGate` — enforced by the Sprint 9 deployment validator.

### 9.4 Why admit at swarm-entry, not per-op ?

Two-stage admission :
1. **Swarm-entry** (`AdmissionGate`) — coarse-grained, checks tenant queue + inflight swarms.
2. **Per-op** (`DrrScheduler`) — fine-grained, weighted slot allocation.

If we only admitted per-op, a tenant could occupy 100 swarms with 1 op each (queue exhaustion).  If we only admitted per-swarm, a 10 000-op swarm would consume only 1 slot while monopolising 10 000 worker tasks.  Both stages are needed.

### 9.5 What about `AdmissionTicket.release()`?

`AdmissionTicket` is RAII : its `Drop` impl decrements counters.  No manual release needed.  The Dispatcher holds the ticket until `dispatch()` returns ; the ticket then drops naturally.

If the Dispatcher panics mid-dispatch, the ticket still drops correctly (Rust's panic unwinding runs Drop).

---

## 10. Per-tenant inflight tracker

### 10.1 The struct

```rust
pub struct InflightTracker {
    per_tenant: DashMap<TenantId, TenantInflight>,
    global_inflight_swarms: AtomicU32,
    global_inflight_ops: AtomicU32,
}

struct TenantInflight {
    inflight_swarms: AtomicU32,
    inflight_ops: AtomicU32,
    queue_depth: AtomicU32,
    cumulative_admissions: AtomicU64,
    cumulative_denials: AtomicU64,
    last_activity: AtomicI64,        // unix-epoch micros, for LRU eviction
}
```

### 10.2 Cardinality + memory

Each tenant entry ≈ 64 bytes.  For 100k tenants → 6.4 MB.  Bounded.

LRU eviction kicks in when DashMap.len() > 100 000 (Sprint 9 hard cap) — drops entries with `inflight = 0` AND `queue_depth = 0` AND `last_activity > 24h ago`.

### 10.3 Consistency : counters vs metrics

The tracker IS the source of truth.  The metrics surface (§ 18) reads atomically from the tracker.  No double-count : both the admission gate and the metric exporter point at the same `AtomicU32`.

### 10.4 Edge case : counter underflow

If the AdmissionTicket's Drop runs BEFORE the increment (TOCTOU race), the decrement could underflow `inflight_ops` from 0 to u32::MAX.  Mitigation : `fetch_sub` with saturating behaviour OR a debug-build assertion that catches the bug.

Sprint 8 implementation uses `fetch_sub` and explicit max-with-zero before reads.

---

## 11. Executor integration — replaces the global semaphore

### 11.1 Current code (pre-N15)

```rust
let semaphore = Arc::new(Semaphore::new(max_parallel));

for handle in &runnable {
    joinset.spawn(async move {
        let _permit = semaphore.acquire_owned().await.expect("semaphore is never closed");
        run_one_op(...).await
    });
}
```

### 11.2 After N15

```rust
// `admission_ticket` is held by the dispatcher across the whole dispatch.
// `drr` is the per-tenant DRR scheduler.

for handle in &runnable {
    let drr = Arc::clone(&drr);
    let tenant_id = request.tenant_id;
    joinset.spawn(async move {
        let _permit = drr.acquire(tenant_id).await
            .expect("DRR acquire never returns Err for accepted tenant");
        run_one_op(...).await
    });
}
```

The previous `tokio::Semaphore` is gone — DRR is the global concurrency cap.

### 11.3 Cache hit short-circuit

When N12 reports a cache hit, the slot is held only for the duration of the cache lookup + cache-hit trace write (< 1 ms).  The DRR permit is acquired + released within a few hundred microseconds — minimal effective consumption of the tenant's slot share.

Documented invariant : cache hits MUST NOT block other tenants for more than 1 ms.

### 11.4 Shadow run separate lane

N13's `ShadowScheduler::schedule` spawns a `tokio::spawn` task.  Inside, the shadow acquires a DRR permit using `TenantId::system()` (or a dedicated `ShadowTenantId`).  So the shadow doesn't compete with the production tenant's prod slots — it goes through the Platform lane.

### 11.5 Per-tenant batch budget

A tenant's `max_inflight_ops` is the cap.  When the swarm submits 1000 ops but the tenant's cap is 500, the executor's JoinSet processes only 500 concurrently ; the next 500 wait on DRR acquire.  Total wall-clock time doubles vs the pre-N15 "fast and unfair" path — but the tenant doesn't exceed their fair share of platform CPU.

---

## 12. N11 Firecracker pool integration

### 12.1 The wrapping

N11's `KindPool::acquire(kind)` is replaced with a tenant-aware version :

```rust
impl KindPool {
    pub async fn acquire_for(
        &self,
        kind: PoolKind,
        tenant: TenantId,
        drr: &DrrScheduler,
    ) -> Result<WarmVm, VmPoolError> {
        let _drr_permit = drr.acquire(tenant).await?;
        self.acquire(kind).await
    }
}
```

When the platform pool is at 64 VMs and tenant A has policy `firecracker_pool_share_bps = 100` (1 %), tenant A's effective cap is 0.64 ≈ 1 VM at a time.  Anything above queues on DRR.

### 12.2 Tenant share enforcement

The `firecracker_pool_share_bps` translates to a DRR weight :
- weight = `tier.drr_weight × pool_share_bps / 10000`

This couples the Firecracker pool share to the global DRR weight, ensuring consistent priority across resources.

### 12.3 Existing N11 tests adapt

N11 tests that allocate VMs without a tenant context now use `TenantId::system()` (Platform tier, unbounded).  Tests that exercise tenant fairness use specific tenants.

---

## 13. N13 shadow scheduler integration

### 13.1 Shadow uses Platform lane

N13's `ShadowScheduler::schedule` spawns a `tokio::spawn`.  Inside the spawned future, before invoking the shadow runtime, it calls :

```rust
let _drr_permit = drr.acquire(TenantId::system()).await?;
```

So shadow consumption goes through the Platform tier (unbounded) — never against the customer tenant's slot share.

This matches the architecture commitment (N13 § 9) : "shadow doesn't compete with prod for tenant slots".

### 13.2 Cost attribution still works

N13's `CostAttribution::{Platform, Tenant}` is orthogonal to N15 admission.  Cost goes to whoever's budget ; admission goes through Platform DRR.  The cost router and DRR are separate concerns.

### 13.3 Shadow concurrency cap

Sprint 8 ship : shadow concurrency is bounded only by Platform DRR weight (= ∞ for Platform tier).  No per-tenant shadow cap.

Sprint 9 may add `policy.max_concurrent_shadows` to bound shadow blast radius.

---

## 14. Audit chain integration

### 14.1 Current (pre-N15)

`InMemoryHmacChainAuditWriter::record` acquires the per-tenant chain Mutex.  Under heavy write load, the mutex queue serializes per-tenant writes.  No fairness across tenants — the tenant in the queue first wins.

### 14.2 Sprint 8 ship : audit-rate token bucket

Each tenant has a `audit_writes_per_second` quota (per tier).  Before the mutex acquire, a token bucket is consulted :

```rust
async fn record(&self, trace: NanoAgentTrace) {
    let _audit_token = self.audit_bucket
        .acquire(trace.tenant_id, AUDIT_PROVIDER_ID, 1.0)
        .await;
    // ... proceed to mutex acquire + write ...
}
```

If the tenant exceeds `audit_writes_per_second`, the bucket returns `BackpressureError::AuditRateLimited`.  The audit writer is FIRE-AND-FORGET RESILIENT (N6 contract) : on backpressure, log warn + drop the trace.  Not ideal — losing audit rows breaks the chain.

**Sprint 9 fix** : audit-rate backpressure must NOT drop traces ; instead, it should sleep with bounded backoff or fail the dispatch.  Sprint 8 documents this as a known limitation.

### 14.3 Why audit per-tier limits ?

A Free-tier tenant can submit 10 000 ops in a Mode A swarm.  Each op produces an audit trace (+ envelope = 10 001 traces).  At 10 000 traces / 30 seconds = 333 writes/sec for ONE tenant.  Multiply by N tenants → audit chain saturates.

Audit-rate limit per tenant : 50 writes/sec for Free, scaling to 25 000/sec for Enterprise.  Sustains realistic per-tier traffic.

---

## 15. Backpressure protocol — error surface

### 15.1 The `BackpressureError` shape

```rust
pub struct BackpressureError {
    pub kind: BackpressureKind,
    pub retry_after_ms: u32,
    pub tenant_id: TenantId,
    pub advice: String,
}
```

The `advice` field is human-readable and explains the action :

| Kind | Sample advice |
|---|---|
| SwarmInflightFull | "You have 5/5 concurrent swarms in flight.  Wait for one to finish, or upgrade your tier." |
| QueueFull | "Your pending-swarm queue is full (16/16).  Retry in 250 ms or upgrade your tier." |
| OpInflightFull | "Your tenant ops cap is 20 ; this swarm has 30 ops.  Split into smaller swarms or upgrade." |
| PlatformBusy | "Platform-wide capacity reached.  Retry in 500 ms (transient, no action needed)." |
| LlmRateLimited | "Your Anthropic rate limit (200 tokens/sec) is exhausted.  Retry in 150 ms." |
| AuditRateLimited | "Your audit-write rate limit is exhausted.  Reduce trace volume or upgrade tier." |
| FirecrackerPoolFull | "Mode B microVM pool share exhausted.  Retry in 400 ms." |
| TenantSuspended | "Your account is suspended.  Contact support." |

### 15.2 HTTP mapping

The API gateway maps `BackpressureError` to HTTP **429 Too Many Requests** with :

```http
HTTP/1.1 429 Too Many Requests
Retry-After: 1
Content-Type: application/json

{
  "error": "backpressure",
  "kind": "queue_full",
  "retry_after_ms": 250,
  "tenant_id": "...",
  "advice": "Your pending-swarm queue is full (16/16).  Retry in 250 ms or upgrade your tier."
}
```

The HTTP `Retry-After` header carries the integer ceiling of seconds.  The body carries the precise milliseconds.

### 15.3 Client SDK auto-retry

The Sprint 9 SDK client wraps requests with exponential backoff retry on 429.  Sprint 8 ship is server-side only ; clients pass-through.

### 15.4 Dispatcher integration

```rust
pub enum DispatcherError {
    ...
    /// **N15** : Tenant-or-platform backpressure.  Caller should
    /// retry after `retry_after_ms`.
    #[error("backpressure : {0}")]
    Backpressure(BackpressureError),
}
```

The dispatcher's `dispatch()` returns `Err(DispatcherError::Backpressure(...))` when admission fails.

---

## 16. Failure modes — F1 to F16

### F1 — DashMap contention on tracker
Many tenants concurrently incrementing.  DashMap shards locks ; contention bounded.  Worst-case ~100 ns per increment.

### F2 — Counter underflow on early Drop
AdmissionTicket Drop runs before Acquire fully published the increment.  Mitigation : `fetch_sub` + saturating max-with-zero on read.

### F3 — Tier change mid-dispatch
Tenant tier changes via M2 push event WHILE the dispatcher holds the ticket.  Old policy used for this dispatch ; next dispatch uses new policy.  Acceptable.

### F4 — Policy lookup latency
`TenantPolicyLookup::lookup` takes 10 ms.  Admission gate latency = 10 ms.  Mitigation : aggressive cache (60 s TTL).

### F5 — Forever-blocked acquire
A bug in DRR allows a tenant's waiter to never be woken.  Anti-starvation force-grant prevents this : after `max_starvation_rounds`, the waiter is force-served.

### F6 — Notify lost wake
`tokio::Notify::notify_one` semantics : a wake before a wait is buffered.  Multiple wakes are deduplicated.  Safe.

### F7 — Token bucket clock skew
System clock jumps backward.  `elapsed = now - last_refill` becomes negative.  Mitigation : saturating subtraction (clamp to zero).

### F8 — Cache hit storm
Tenant A submits 10k identical requests in 1 second.  Cache serves all from N12.  Admission grants each → tenant_inflight_ops counter reaches `max_inflight_ops`.  Reaches cap → subsequent requests get `OpInflightFull`.  Expected — operator should increase cap or split traffic.

### F9 — Inflight tracker DashMap eviction
A tenant's entry evicted at the wrong moment (between Acquire and Release).  Mitigation : eviction policy explicitly checks `inflight == 0 AND queue_depth == 0`.  Safe.

### F10 — Sub-millisecond timing precision
On Linux, `Instant::now()` has < 1 ns precision.  Token bucket refill math is accurate.  No issue.

### F11 — Suspended tenant during in-flight swarm
Tenant suspended (operator action) while dispatch in progress.  Admission ticket held ; dispatch completes.  Subsequent dispatches denied.  Acceptable.

### F12 — Platform capacity miscalculation
`global_capacity` set to 100 ; actual CPU only handles 50 concurrent.  Platform overloaded.  Mitigation : ops monitoring + adaptive capacity (Sprint 10 deliverable).

### F13 — Audit chain backpressure dropped trace
Audit rate-limit exhausted → trace dropped → chain has a gap.  Hard correctness break.  Sprint 9 fixes (block instead of drop) ; Sprint 8 logs warn.

### F14 — DRR weight overflow
Tenant tier has `drr_weight = 100` ; multiplied by `quantum × max_starvation_rounds` in extreme cases.  Overflow check : use `i64` for deficit math.

### F15 — Concurrent tier change
Two operators upgrade the same tenant simultaneously.  Last-write-wins on M2 event bus.  Tracker re-reads next admission.  Safe.

### F16 — Cold start latency
First-ever request from a tenant : DashMap inserts a new entry.  ~200 ns overhead.  Acceptable.

---

## 17. Invariants — I1 to I20

| ID | Invariant | Verification |
|---|---|---|
| I1 | One ticket per admitted swarm | RAII enforces |
| I2 | tracker.inflight_ops(t) ≤ policy(t).max_inflight_ops | admission check |
| I3 | tracker.inflight_swarms(t) ≤ policy(t).max_inflight_swarms | admission check |
| I4 | tracker.queue_depth(t) ≤ policy(t).swarm_queue_depth | admission check |
| I5 | Σ tracker.inflight_swarms(t) ≤ global_cap.max_concurrent_swarms | admission check |
| I6 | DRR : tenant with deficit ≥ 1 gets next slot when global capacity available | test invariant |
| I7 | Anti-starvation : tenant with pending acquire gets served within `max_starvation_rounds` | force-grant branch |
| I8 | Work-conserving : if any tenant has pending work AND global capacity is free, work proceeds | test invariant |
| I9 | Token bucket : long-term tokens granted ≤ refill_rate × elapsed_secs | timing test |
| I10 | Token bucket : burst grants ≤ capacity | acquire test |
| I11 | Counters never go negative (saturating) | underflow test |
| I12 | AdmissionTicket Drop releases counters exactly once | RAII test |
| I13 | Tier change rebalances DRR weights on next round | rebalance test |
| I14 | N11 Firecracker `acquire_for` enforces tenant share | integration test |
| I15 | N13 shadow uses Platform tenant for DRR | integration test |
| I16 | Cache hit holds DRR permit < 1 ms (p99) | timing test |
| I17 | BackpressureError carries non-zero retry_after_ms | error-shape test |
| I18 | Platform tier has unlimited quotas | policy test |
| I19 | Suspended tenant : all admissions denied | suspension test |
| I20 | Metrics atomic counters never lag the tracker by > 1 op | snapshot test |

---

## 18. Metrics — 12 Prometheus surfaces

```rust
pub struct SchedulerMetrics {
    // Counters
    pub per_tenant_admissions_total: AtomicU64,         // labels : tier
    pub per_tenant_admissions_denied_total: AtomicU64,  // labels : tier, kind

    // Gauges (snapshots from tracker)
    pub per_tenant_inflight_swarms_current: GaugeMap<TenantId>,
    pub per_tenant_inflight_ops_current: GaugeMap<TenantId>,
    pub per_tenant_queue_depth_current: GaugeMap<TenantId>,
    pub per_tenant_token_bucket_level_current: GaugeMap<(TenantId, ProviderId)>,
    pub per_tenant_drr_deficit_current: GaugeMap<TenantId>,
    pub global_inflight_swarms_current: AtomicU32,
    pub global_inflight_ops_current: AtomicU32,

    // Histograms
    pub admission_wait_ms_histogram_sum: AtomicU64,
    pub admission_wait_ms_histogram_count: AtomicU64,
    pub drr_acquire_wait_ms_histogram_sum: AtomicU64,
    pub drr_acquire_wait_ms_histogram_count: AtomicU64,
    pub token_bucket_acquire_wait_ms_histogram_sum: AtomicU64,
    pub token_bucket_acquire_wait_ms_histogram_count: AtomicU64,
}
```

Total : 12 metric surfaces (counters + gauges + histogram approximations).

### 18.1 M0 dashboard panels

| Panel | Query |
|---|---|
| Per-tier admission rate | `rate(per_tenant_admissions_total[1m])` |
| Per-tier denial rate | `rate(per_tenant_admissions_denied_total[1m])` |
| Top 10 noisy tenants | sort_desc(per_tenant_inflight_ops_current)[:10] |
| Platform inflight saturation | global_inflight_swarms_current / global_cap |
| P99 admission wait | histogram_quantile(0.99, admission_wait_ms_histogram) |
| LLM bucket levels by provider | per_tenant_token_bucket_level_current grouped by provider_id |

### 18.2 Alert rules

* `rate(per_tenant_admissions_denied_total{kind="platform_busy"}[5m]) > 10` — platform capacity needs scaling.
* `histogram_quantile(0.99, admission_wait_ms_histogram) > 500` — admission gate is slow.
* `per_tenant_inflight_ops_current{tenant_tier="enterprise"} == 0 for 5m` — Enterprise tenant inactive (potential SLA concern).
* `per_tenant_admissions_denied_total{kind="tenant_suspended"} > 0` — suspended tenants are still trying.

---

## 19. Implementation plan — 5-day breakdown

### 19.1 Day 1 — SDK types + policy

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano-sdk/src/types/scheduling.rs` with `TenantTier`, `QuotaPolicy`, `BackpressureError`, `BackpressureKind` | Tier 1 GREEN |
| founder | `TenantPolicyLookup` trait | trait test |
| founder | Tier presets (`QuotaPolicy::free()` / `tier1()` / ... / `platform()`) | preset tests |
| founder | `InMemoryTenantPolicyLookup` impl | unit tests |

### 19.2 Day 2 — DRR scheduler

| Owner | Deliverable | Verification |
|---|---|---|
| senior eng | `efe-nano/src/scheduler/drr.rs` with `DrrScheduler` + `DrrPermit` (RAII) | Tier 1 GREEN |
| senior eng | Anti-starvation force-grant logic | Tier 1 GREEN |
| senior eng | Weighted fair share — 1000-acquire test : Tier1 vs Tier3 share within 5 % | timing test |
| senior eng | Concurrency test : 100 spawned tasks, asserting fairness | tokio integration |

### 19.3 Day 3 — Token bucket + admission gate

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano/src/scheduler/token_bucket.rs` with `TokenBucket` | Tier 1 GREEN |
| founder | `efe-nano/src/scheduler/admission.rs` with `AdmissionGate` + `AdmissionTicket` (RAII) | Tier 1 GREEN |
| founder | `efe-nano/src/scheduler/tracker.rs` with `InflightTracker` | Tier 1 GREEN |

### 19.4 Day 4 — Executor + N11/N13 integration

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `Dispatcher::with_admission_gate(...)` builder | smoke test |
| founder | `executor::execute_plan_with_extras_n14` accepts optional `Arc<DrrScheduler>` | wire-up test |
| founder | N11 `KindPool::acquire_for(tenant, drr)` | integration test |
| founder | N13 `ShadowScheduler` uses Platform DRR | shadow test |
| founder | N12 cache-hit fast path : sub-1ms permit hold | timing test |

### 19.5 Day 5 — Metrics + integration tests + verification

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano/src/scheduler/metrics.rs` with 12 surfaces | exposure test |
| founder | `tests/phase_2_n15_fairness_scheduling.rs` — 30+ tests | full GREEN |
| both | Final clippy strict + cargo test full suite | gate-by-gate |

### 19.6 Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| DRR weight starvation under extreme load | low | Force-grant branch ensures min progress |
| Tracker DashMap hot-spot | medium | Sharded locks ; 100k tenant cap |
| Cache-hit slot-hold blocks fairness | low | Sub-1ms drop ; documented |
| Token bucket clock skew | low | Saturating math |
| Sprint 9 cross-region rollout breaks | low | Sprint 8 single-region only |
| Dispatcher tests break without AdmissionGate | medium | Optional `with_admission_gate` ; legacy path preserved |

---

## 20. Test matrix

### 20.1 Tier 1 — unit tests (per module)

| Test | Description |
|---|---|
| `tenant_tier_serde_round_trip` | Each tier round-trips via JSON |
| `quota_policy_presets` | `free()` / `tier1()` / ... return monotonic weights |
| `inflight_tracker_increments_on_admit` | Admission gate increments counters |
| `inflight_tracker_decrements_on_drop` | RAII ticket drop decrements |
| `admission_gate_denies_swarm_inflight_full` | Tenant hits `max_inflight_swarms` → SwarmInflightFull |
| `admission_gate_denies_queue_full` | Tenant hits `swarm_queue_depth` → QueueFull |
| `admission_gate_denies_op_inflight_full` | Request's op_count + current inflight > max → OpInflightFull |
| `drr_scheduler_acquire_release_roundtrip` | Acquire + Drop is symmetric |
| `drr_scheduler_weighted_fair_share` | 1000 acquires across 2 tiers, share matches weight ratio ± 5 % |
| `drr_scheduler_work_conserving` | Single active tenant uses full global capacity |
| `drr_scheduler_anti_starvation` | Low-weight tenant gets ≥ 1 slot per max_starvation_rounds |
| `token_bucket_acquire_within_capacity` | Acquire ≤ capacity → instant Ok |
| `token_bucket_refill_to_capacity_after_idle` | Idle period → bucket refills to capacity |
| `token_bucket_refill_rate_accurate` | Sustained acquire at refill rate → no exhaustion |
| `backpressure_error_carries_retry_after_ms` | Error includes non-zero hint |
| `backpressure_error_serde_round_trip` | Error JSON-round-trips |

### 20.2 Tier 2 — integration tests

| Test | Description |
|---|---|
| `dispatcher_admits_under_quota` | Tenant under quota → dispatch succeeds |
| `dispatcher_denies_over_quota` | Tenant over quota → BackpressureError returned |
| `dispatcher_releases_slot_on_completion` | After dispatch, tracker decrements |
| `n11_firecracker_pool_respects_tenant_share` | Tier1 tenant gets ~5 % of pool |
| `n13_shadow_uses_platform_drr_lane` | Shadow runs don't consume customer tenant slot |
| `n12_cache_hit_fast_release` | Cache hit holds DRR permit < 1 ms |
| `n14_envelope_records_admission_kind` | Sprint 9 envelope captures admission outcome |

### 20.3 Tier 3 — stress

| Test | Description |
|---|---|
| `stress_1000_acquires_fair_share` | 1k acquires across 4 tenants of different tiers |
| `stress_concurrent_tier_change` | 100 acquires while tier changes mid-flight |
| `stress_token_bucket_under_burst` | 5× burst then sustained rate |
| `stress_inflight_tracker_under_100k_tenants` | DashMap behaviour under cardinality |

### 20.4 Tier 4 — E2E soak

| Test | Description |
|---|---|
| `e2e_24h_mixed_tier_workload` | 4 tenants of different tiers, 24h simulated load |
| `e2e_tenant_suspension_during_load` | Suspend mid-flight → in-flight completes, new denied |

---

## 21. Migration path

### 21.1 Pre-N15 state

Dispatcher uses `tokio::Semaphore` globally.  No tenant fairness.

### 21.2 N15 deploy (Sprint 8)

1. **Code deployed** — `AdmissionGate` available as a builder method on `Dispatcher`.  Default : NOT installed.
2. **Phase A** : install `AdmissionGate` with `InMemoryTenantPolicyLookup` configured to give EVERY tenant Enterprise tier (effectively no fairness, but the machinery is in place).
3. **Phase B** : roll out tier assignments via M2 push, starting with Free tier for known-abusive tenants.
4. **Phase C** : gradually tighten quotas as data shows actual load profiles.

### 21.3 Backward compat

* Test harnesses that don't install `AdmissionGate` continue to work (legacy path).
* Existing integration tests pass unmodified.
* SDK clients receive `BackpressureError` as a new variant ; older clients see HTTP 429 and retry.

---

## 22. Open questions / ADRs

### ADR-N15-001 : DRR over WFQ for concurrency

**Decision** : DRR for slot-counted resources.

**Rationale** : O(1) per acquire ; simple to reason about ; bound is exactly `quantum × weight`.  WFQ is theoretically tighter but O(log N) per dequeue and more complex to implement correctly.

### ADR-N15-002 : Token bucket per (tenant, provider)

**Decision** : One bucket per (tenant, provider).

**Alternatives** : One bucket per tenant (sum across providers), one bucket per provider (global).

**Rationale** : Per-(tenant, provider) isolates providers — Anthropic outage doesn't penalize tenant's OpenAI quota.

### ADR-N15-003 : Tier-based, not per-tenant policy

**Decision** : `QuotaPolicy` is per-tier ; per-tenant overrides are Sprint 9 deferred.

**Rationale** : Operational simplicity.  100k tenants × custom policy = ops nightmare.

### ADR-N15-004 : Admission at swarm-entry, not per-op

**Decision** : Two-stage admission — `AdmissionGate` at swarm-entry, `DrrScheduler` per-op.

**Rationale** : Captures both queue depth AND fine-grained slot fairness.

### ADR-N15-005 : Audit-rate backpressure drops in Sprint 8

**Decision** : Sprint 8 ship logs warn + drops trace on audit rate-limit hit.

**Trade-off** : Audit chain gap.  Sprint 9 must fix.

### ADR-N15-006 : Anti-starvation force-grant

**Decision** : After `max_starvation_rounds`, force-serve the waiter.

**Rationale** : Prevents pathological deficits ; bounded latency for low-tier tenants.

### ADR-N15-007 : `Platform` tier for internal services

**Decision** : Reserve `TenantTier::Platform` for EFE-internal services with unbounded quotas.

**Rationale** : N13 shadows + M28 events + M40 metrics need to bypass quotas.

### ADR-N15-008 : No global LLM token redistribution in Sprint 8

**Decision** : Sprint 8 has only per-(tenant, provider) buckets.  No work-conserving global bucket.

**Rationale** : Sprint 9 N16 introduces hierarchical buckets with borrow.  Sprint 8 keeps it simple.

---

## 23. Annexe A — full type definitions

```rust
// crates/efe-nano-sdk/src/types/scheduling.rs

use serde::{Deserialize, Serialize};
use thiserror::Error;
use crate::types::TenantId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantTier {
    Free,
    Tier1,
    Tier2,
    Tier3,
    Enterprise,
    Platform,
}

impl TenantTier {
    pub fn drr_weight(self) -> u32 {
        match self {
            Self::Free => 1,
            Self::Tier1 => 1,
            Self::Tier2 => 5,
            Self::Tier3 => 20,
            Self::Enterprise => 100,
            Self::Platform => u32::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaPolicy {
    pub tier: TenantTier,
    pub drr_weight: u32,
    pub max_inflight_ops: u32,
    pub max_inflight_swarms: u32,
    pub swarm_queue_depth: u32,
    pub llm_tokens_per_second: f64,
    pub audit_writes_per_second: f64,
    pub firecracker_pool_share_bps: u16,
    pub max_starvation_rounds: u32,
}

impl QuotaPolicy {
    pub fn free() -> Self { ... }
    pub fn tier1() -> Self { ... }
    pub fn tier2() -> Self { ... }
    pub fn tier3() -> Self { ... }
    pub fn enterprise() -> Self { ... }
    pub fn platform() -> Self { ... }

    pub fn for_tier(tier: TenantTier) -> Self {
        match tier {
            TenantTier::Free => Self::free(),
            TenantTier::Tier1 => Self::tier1(),
            TenantTier::Tier2 => Self::tier2(),
            TenantTier::Tier3 => Self::tier3(),
            TenantTier::Enterprise => Self::enterprise(),
            TenantTier::Platform => Self::platform(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("backpressure : {kind:?} (retry after {retry_after_ms} ms)")]
pub struct BackpressureError {
    pub kind: BackpressureKind,
    pub retry_after_ms: u32,
    pub tenant_id: TenantId,
    pub advice: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressureKind {
    SwarmInflightFull,
    QueueFull,
    OpInflightFull,
    PlatformBusy,
    LlmRateLimited,
    AuditRateLimited,
    FirecrackerPoolFull,
    TenantSuspended,
}
```

```rust
// crates/efe-nano/src/scheduler/mod.rs

pub mod admission;
pub mod drr;
pub mod metrics;
pub mod policy_lookup;
pub mod token_bucket;
pub mod tracker;

pub use admission::{AdmissionGate, AdmissionTicket};
pub use drr::{DrrPermit, DrrScheduler};
pub use metrics::SchedulerMetrics;
pub use policy_lookup::{InMemoryTenantPolicyLookup, TenantPolicyLookup};
pub use token_bucket::TokenBucket;
pub use tracker::InflightTracker;
```

```rust
// crates/efe-nano/src/scheduler/drr.rs

pub struct DrrScheduler {
    state: DashMap<TenantId, Mutex<TenantDrrState>>,
    global_capacity: u32,
    quantum: u32,
    inflight: AtomicU32,
    wake: Notify,
    policy_lookup: Arc<dyn TenantPolicyLookup>,
}

pub struct DrrPermit {
    tenant: TenantId,
    scheduler: Arc<DrrScheduler>,
}

impl Drop for DrrPermit {
    fn drop(&mut self) {
        self.scheduler.release(self.tenant);
    }
}
```

---

## 24. Annexe B — Postgres schema (Phase 4)

```sql
-- Per-tenant policy table.  Tier defaults applied if absent.
CREATE TABLE tenant_quota_policy (
    tenant_id              UUID PRIMARY KEY,
    tier                   TEXT NOT NULL,
    drr_weight             INT NOT NULL,
    max_inflight_ops       INT NOT NULL,
    max_inflight_swarms    INT NOT NULL,
    swarm_queue_depth      INT NOT NULL,
    llm_tokens_per_second  NUMERIC(12, 4) NOT NULL,
    audit_writes_per_second NUMERIC(12, 4) NOT NULL,
    firecracker_pool_share_bps INT NOT NULL,
    max_starvation_rounds  INT NOT NULL,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by             TEXT NOT NULL
);

-- Audit log of policy changes.
CREATE TABLE tenant_quota_policy_history (
    history_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id     UUID NOT NULL,
    from_policy   JSONB,
    to_policy     JSONB NOT NULL,
    changed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    changed_by    TEXT NOT NULL,
    reason        TEXT
);

-- Inflight snapshot table for failover (Sprint 9).
CREATE TABLE inflight_snapshot (
    snapshot_at        TIMESTAMPTZ NOT NULL,
    tenant_id          UUID NOT NULL,
    inflight_swarms    INT NOT NULL,
    inflight_ops       INT NOT NULL,
    queue_depth        INT NOT NULL,
    PRIMARY KEY (snapshot_at, tenant_id)
);

CREATE INDEX idx_inflight_snapshot_tenant ON inflight_snapshot(tenant_id);
```

---

## 25. Annexe C — runtime config examples

### 25.1 Operator-set environment variables

```bash
# Global concurrency cap.
EFE_NANO_GLOBAL_CONCURRENT_SWARMS=1000
EFE_NANO_GLOBAL_CONCURRENT_OPS=10000

# DRR scheduler config.
EFE_NANO_DRR_QUANTUM=1
EFE_NANO_DRR_MAX_STARVATION_ROUNDS=10

# Default per-tier policies.
EFE_NANO_TIER_FREE_MAX_INFLIGHT_OPS=5
EFE_NANO_TIER_TIER1_MAX_INFLIGHT_OPS=20
EFE_NANO_TIER_TIER2_MAX_INFLIGHT_OPS=100
EFE_NANO_TIER_TIER3_MAX_INFLIGHT_OPS=500
EFE_NANO_TIER_ENTERPRISE_MAX_INFLIGHT_OPS=2500

# LLM provider rate limits per tier.
EFE_NANO_LLM_TIER_FREE_TOKENS_PER_SECOND=50
EFE_NANO_LLM_TIER_TIER1_TOKENS_PER_SECOND=200
EFE_NANO_LLM_TIER_TIER2_TOKENS_PER_SECOND=1000

# Audit-write quota per tier.
EFE_NANO_AUDIT_TIER_FREE_QPS=50
EFE_NANO_AUDIT_TIER_ENTERPRISE_QPS=25000
```

### 25.2 Test fixture

```rust
let policy_lookup = Arc::new(InMemoryTenantPolicyLookup::with_tier_defaults());
policy_lookup.set_tier(t1, TenantTier::Tier1);
policy_lookup.set_tier(t2, TenantTier::Tier3);

let drr = Arc::new(DrrScheduler::new(global_capacity: 100, quantum: 1, policy_lookup));
let tracker = Arc::new(InflightTracker::new());
let gate = Arc::new(AdmissionGate::new(drr.clone(), tracker, policy_lookup));
let dispatcher = Dispatcher::new(...).with_admission_gate(gate);
```

---

## 26. Annexe D — incident playbook

### Scenario : "Free tenant flooding"

T+0 : Free tenant T0 submits 1000 SwarmRequests in 30 seconds.  Tier policy : `max_inflight_swarms = 1, swarm_queue_depth = 4`.

T+0 to T+5s : 5 requests admitted ; queue + inflight = 5.  Remaining 995 return `BackpressureError::QueueFull` with `retry_after_ms = 250`.

T+5s onwards : tenant client backs off ; submits at ~1 request/sec.  Admissions sustain at fair share.

Other tenants continue undisturbed.  Operator dashboard shows `per_tenant_admissions_denied_total{kind="queue_full", tier="free"}` spike but no impact on other tiers.

### Scenario : "Enterprise SLA breach claim"

T+0 : Enterprise tenant T_ent reports "request took 30 seconds, you committed < 1 s p99".

Investigation : query `histogram_quantile(0.99, admission_wait_ms_histogram{tenant_id="T_ent"}[1h])` → 50 ms (well under SLA).  Then query `histogram_quantile(0.99, drr_acquire_wait_ms_histogram{tenant_id="T_ent"}[1h])` → 200 ms (still OK).  Then the dispatcher latency itself — found the issue : a 30 s Mode C call due to provider slowness.

Conclusion : NOT a fairness issue.  N15 metrics correctly show that the platform's scheduling stayed within SLA bounds.  The issue was downstream.

### Scenario : "Tier upgrade verification"

T_2 was on Tier1.  Operator upgrades to Tier3 (via M2 push).  Within 60 s, the policy lookup cache refreshes ; next admission uses Tier3 weights.  No traffic impact.

Verification : `per_tenant_drr_deficit_current{tenant_id="T_2"}` jumps from low values to high values (more deficit accrual per round).

### Scenario : "Anthropic outage"

T+0 : Anthropic returns 429s globally.  Every tenant's token bucket for Anthropic fills (refilling normally since LOCAL rate limit is intact).  But actual LLM calls fail.  Dispatcher's `llm_client` returns errors ; N13 doesn't shadow ; dispatches fail.

N15 behaviour : bucket levels show as full (no consumption).  The denial counter for `LlmRateLimited` stays low.  The issue is observable in N12 dispatcher error rates, NOT in N15 metrics.

Action : ops switches to OpenAI failover (Sprint 9 multi-provider fallback).

---

**End of EFE M50 — N15 Fairness Scheduling Architecture (Sprint 8).**
