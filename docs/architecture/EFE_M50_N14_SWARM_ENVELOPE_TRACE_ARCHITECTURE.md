# EFE M50 — N14 Swarm Envelope Trace Architecture

> **Status** — DRAFT (Sprint 8 finalisation, target ship cycle T+12d)
> **Owners** — founder (architecture), senior eng (implementation)
> **Companion docs** —
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8 N12, shipped)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8 N13, shipped)
> * `EFE_M50_PHASE_2_ARCHITECTURE.md` (Phase 2 panorama)
> **Sister deliverable** — `EFE_M50_N15_META_SWARM_LINEAGE_ARCHITECTURE.md` (Sprint 9, planned — child swarm chaining)

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Problem statement — what we're solving](#1-problem-statement--what-were-solving)
2. [Goals & non-goals](#2-goals--non-goals)
3. [Architecture overview](#3-architecture-overview)
4. [Component breakdown — 8 components](#4-component-breakdown--8-components)
5. [`SwarmEnvelopePayload` — the on-disk source of truth](#5-swarmenvelopepayload--the-on-disk-source-of-truth)
6. [Envelope lifecycle](#6-envelope-lifecycle)
7. [Per-op trace linkage via `swarm_envelope_trace_id`](#7-per-op-trace-linkage-via-swarm_envelope_trace_id)
8. [Audit chain integration (HMAC + canonical body)](#8-audit-chain-integration-hmac--canonical-body)
9. [Replay engine integration](#9-replay-engine-integration)
10. [GDPR tombstone cascade extension](#10-gdpr-tombstone-cascade-extension)
11. [N11 / N12 / N13 alignment](#11-n11--n12--n13-alignment)
12. [Meta-swarm lineage (forward-compatible)](#12-meta-swarm-lineage-forward-compatible)
13. [Streaming sub-results — Sprint 9 horizon](#13-streaming-sub-results--sprint-9-horizon)
14. [Failure modes — F1 to F14](#14-failure-modes--f1-to-f14)
15. [Invariants — I1 to I18](#15-invariants--i1-to-i18)
16. [Metrics — 8 Prometheus surfaces](#16-metrics--8-prometheus-surfaces)
17. [Implementation plan — 5-day breakdown](#17-implementation-plan--5-day-breakdown)
18. [Test matrix](#18-test-matrix)
19. [Migration path](#19-migration-path)
20. [Open questions / ADRs](#20-open-questions--adrs)
21. [Annexe A — full type definitions](#21-annexe-a--full-type-definitions)
22. [Annexe B — Postgres schema (Phase 4)](#22-annexe-b--postgres-schema-phase-4)
23. [Annexe C — sample envelope JSON](#23-annexe-c--sample-envelope-json)
24. [Annexe D — forensic query playbook](#24-annexe-d--forensic-query-playbook)

---

## 0. Executive summary

EFE Nano's chain (Sprint 6 ship) records **one trace per op**.  A `SwarmRequest` with K ops produces K rows ; each row tells you about one nano-agent invocation.  But the **swarm itself** — the top-level batch with its `caller_agent_id`, `max_parallelism`, `audit_tags`, `cost_cap_cents`, `timeout_ms`, `idempotency_key`, and its aggregated outcome — has **no row of its own**.

That gap creates seven systematic forensic problems documented in § 1.  N14 closes them by introducing the **Swarm Envelope trace** : a single chain-linked row written at swarm completion that anchors the swarm in the audit chain, captures the full request signature, and records the aggregated outcome.

**Five capabilities ship in Sprint 8 N14** :

1. **`SwarmEnvelopePayload`** — typed structure stored as the envelope trace's `full_input` (request fields) + `full_output` (aggregated outcome).  Round-trippable.
2. **`swarm_envelope_trace_id: Option<TraceId>`** — new operational-metadata field on `NanoAgentTrace`, populated on every op trace within a swarm, pointing at the envelope.  **Excluded from canonical_row_body** so adding it preserves Sprint 6 HMAC chain integrity for pre-N14 rows.
3. **`error_kind = Some("SwarmEnvelope")`** — sentinel discriminator, filtered by the replay engine just like `CacheHit` / `ShadowRun` (N12 / N13).
4. **`ReplayEngine::envelope_for_swarm(...)`** — read API : given `(tenant, swarm_request_id)`, returns the envelope payload — the entry point for forensic investigations.
5. **GDPR cascade extension** — `tombstone_swarm_envelope(envelope_trace_id)` cascades to every op trace of the same `swarm_request_id`, ensuring an "erase this batch" operation is atomic.

The integration with the just-shipped neighbours :

- **N12 cache** — cache-hit traces inside a swarm carry `swarm_envelope_trace_id`.  The envelope's `op_status_counts` treats cache hits as `Success`.
- **N13 shadow + canary** — `ShadowRun` / `ShadowComparison` traces also carry `swarm_envelope_trace_id` so the envelope captures the COMPLETE audit-side activity of the swarm.  Replay filter (already excluding 4 sentinels) gains a 5th : `SwarmEnvelope`.
- **N10 GDPR tombstone** — the cascade now extends from envelope → all op traces of the swarm in one atomic erase ; symmetric to "tombstone the prod trace cascades to its shadow children" (N13 § 14).
- **N9 Merkle epoch** — every envelope is a regular chain entry, gets included in the current epoch's Merkle root, gets a notary receipt.  Sprint 9 may add a per-envelope inclusion proof shortcut.

Verification gate (T+12d) :

| ID | Commitment | Test |
|---|---|---|
| C1 | Envelope round-trip : write + read = identity | `envelope_round_trip` GREEN |
| C2 | Every op trace within a swarm carries the envelope id | `op_traces_carry_envelope_id` GREEN |
| C3 | Envelope captures all SwarmRequest fields verbatim | `envelope_captures_request_signature` GREEN |
| C4 | Envelope captures aggregated outcome (status, cost, latency, op counts) | `envelope_captures_outcome` GREEN |
| C5 | HMAC chain still verifies pre-N14 rows (no canonical body change) | `pre_n14_chain_still_verifies` GREEN |
| C6 | Replay filter excludes `SwarmEnvelope` | `replay_filters_swarm_envelope` GREEN |
| C7 | GDPR tombstone cascade from envelope erases all op traces | `tombstone_envelope_cascades_ops` GREEN |
| C8 | Cache-hit / shadow / comparison traces carry envelope id | `audit_metadata_traces_carry_envelope_id` GREEN |
| C9 | Meta-swarm parent_envelope_trace_id propagated correctly | `child_swarm_envelope_references_parent` GREEN |

---

## 1. Problem statement — what we're solving

### 1.1 The seven systematic gaps

Today, without an envelope :

**Gap 1 — Request signature is reconstructed, not stored.**  The original `SwarmRequest`'s top-level fields (`max_parallelism`, `audit_tags`, `cost_cap_cents`, `timeout_ms`, `idempotency_key`, `streaming`) are visible in the SwarmResponse return path but vanish from the chain.  An auditor 6 months later can only reconstruct them by inspecting per-op traces — and even then, they only see what each op SAW, not what the swarm was CONFIGURED with.

**Gap 2 — Preflight failures have no chain entry.**  If a `SwarmRequest` fails at planning time (invalid DAG, cycle detection, unauthorized agent, malformed input), no op trace is ever written and the chain has no record that the attempt happened.  An attacker who triggers preflight failures can probe the platform without leaving any audit footprint.

**Gap 3 — Aggregated status is an opinion, not a fact.**  The dispatcher decides "Success" vs "PartialFailure" vs "TimedOut" vs "Cancelled" by walking op outcomes.  That decision lives in the dispatcher's code path — NOT in the chain.  Two different dispatchers (older vs newer code) could classify the same outcomes differently.  Forensics demand a single recorded truth.

**Gap 4 — Forensic queries need an entry point.**  Investigating an incident starts from a `swarm_request_id` (e.g., "tenant T reported a wrong output at 14:32 UTC, swarm_id = X").  The auditor's first query : "give me the swarm row" — and there is none.  They have to query "all traces for swarm_request_id = X", aggregate manually, and reconstruct the swarm shape.  Workflow friction every time.

**Gap 5 — Streaming + async semantics have no anchor.**  Sprint 9 plans streaming partial results back to the caller as ops complete.  Without an envelope, there's no row to attach intermediate updates to.

**Gap 6 — Cross-swarm dependencies are invisible.**  Meta-swarms (Sprint 9 N15) spawn child swarms.  The lineage "swarm A's op_5 invoked swarm B" requires a typed parent-envelope reference somewhere.  Per-op traces' `parent_trace_ids` carry DAG lineage WITHIN a swarm — they can't naturally cross swarm boundaries.

**Gap 7 — Billing accuracy requires envelope-level totals.**  Per-op `cost_cents` are tracked, but the swarm's TOTAL cost (with cost-cap-induced abort logic) is computed by the aggregator and never written.  An audit-grade billing reconciliation needs the swarm-level totals to match the per-op sum — and currently the chain only stores the latter.

### 1.2 The cost of the gap

Real incidents we've observed (pre-Phase 4 Sprint 0) :

* A customer claimed they were billed for a swarm they never sent.  Investigation : the swarm_request_id matched per-op rows in the chain.  We confirmed the swarm DID run.  But the customer's API client logs claimed timeout — they retried with the same idempotency_key.  Without an envelope row tagging "this swarm's idempotency_key was K and overall_status was Success", proving the dedup-collision path took 4 days.

* A drift in `total_cost_cents` between the SwarmResponse payload and the sum of per-op `cost_cents` in the chain.  Cause : the cost cap (N7) fired mid-swarm ; some ops were CostCapReached'd without recording cost.  But the SwarmResponse had a smaller total than the chain sum.  Without an envelope, reconciling that drift required custom queries against both audit and billing logs.

* A Phase 4 customer canary regression (N13) was rolled back via auto-rollback at T+1h.  The RolloutPolicyChanged trace was recorded.  Operators wanted to see "which 47 swarms in the canary window had the broken output".  Without per-swarm envelope rows, this query became "scan all op traces for the version, aggregate by swarm_request_id, count distinct" — slow + brittle.

### 1.3 Why now ?

Three drivers converge :

1. **N13 (just shipped)** introduces the canary / shadow workflow.  Operators need to query swarms by version + outcome (canary rollback forensics) — envelope makes this trivial.
2. **Phase 4 customer authoring** requires customer-visible audit logs.  Customers want their "list of swarms run this week" surfaced via SDK — the envelope IS that row.
3. **Sprint 9 streaming + meta-swarm** need an anchor to attach intermediate updates and parent-child references.  Sprint 8 lays the foundation now so Sprint 9 work fits without breaking changes.

---

## 2. Goals & non-goals

### 2.1 Sprint 8 N14 commitments — the 9 we ship

| ID | Goal | Surface | Verification |
|---|---|---|---|
| G1 | `SwarmEnvelopePayload` SDK type + serde round-trip | `efe_nano_sdk::types::SwarmEnvelopePayload` | round-trip test |
| G2 | `swarm_envelope_trace_id: Option<TraceId>` field on `NanoAgentTrace`, excluded from canonical_row_body | `efe_nano::audit::NanoAgentTrace.swarm_envelope_trace_id` | HMAC verification still works on pre-N14 rows |
| G3 | `error_kind = Some("SwarmEnvelope")` sentinel + replay filter extension | `crate::replay::mod.rs` filter chain | `replay_filters_swarm_envelope` |
| G4 | `Dispatcher::dispatch` writes the envelope at completion (success, failure, timeout, plan-failure) | `Dispatcher::dispatch` | 4 status-path tests |
| G5 | Per-op traces carry `swarm_envelope_trace_id` (incl. CacheHit / ShadowRun / ShadowComparison) | `CallContext.swarm_envelope_trace_id` plumbed through runtime | integration tests |
| G6 | `ReplayEngine::envelope_for_swarm(tenant, swarm_id)` | read API | `envelope_query_round_trip` |
| G7 | `ChainStore::tombstone_swarm_envelope(envelope_id)` cascades to all op traces | extend N10 cascade | cascade test |
| G8 | `SwarmResponse.swarm_envelope_trace_id: Option<TraceId>` surfaced to callers | SDK SwarmResponse field | E2E test |
| G9 | 8 Prometheus metrics : `envelopes_written_total`, `envelopes_write_failed_total`, `envelope_latency_ms_histogram`, `envelope_op_count_histogram`, `envelope_total_cost_cents_histogram`, `envelope_tombstones_total`, `envelopes_queried_total`, `envelope_size_bytes` | scraped via `/metrics` | metric-exposure test |

### 2.2 Sprint 8 N14 non-goals (deferred)

| ID | Deferred capability | Sprint to ship |
|---|---|---|
| N1 | **Streaming sub-result envelopes** — multi-update envelope rows during long-running swarms | Sprint 9 |
| N2 | **`parent_envelope_trace_id`** wired automatically in meta-swarms — field exists but caller sets it manually | Sprint 9 N15 |
| N3 | **`swarm_envelope_trace_id` in canonical_row_body** — promotes the field to part of HMAC payload via body_version bump | Sprint 9 |
| N4 | **Postgres `swarm_envelopes` projection table** — for fast tenant-dashboard queries | Sprint 9 |
| N5 | **Negative envelope on planner crash** — currently if dispatcher itself crashes mid-dispatch, no envelope is written | Sprint 9 (stale-sweep job) |
| N6 | **Customer-facing MCP tool** `efe_nano_list_swarms` returning envelope summaries | Sprint 9 |
| N7 | **Aggregated swarm-level metrics derived from envelope** (e.g. `swarm_p99_latency_ms_by_tenant`) | Sprint 10 |
| N8 | **Envelope-level retention policies** (e.g. "tombstone all envelopes > 90 days") | Sprint 10 |

---

## 3. Architecture overview

### 3.1 The 3-layer diagram

```
┌────────────────────────────────────────────────────────────────────────┐
│                        EFE Nano Dispatcher (M50)                       │
│                                                                        │
│    incoming SwarmRequest                                               │
│         │                                                              │
│         ▼                                                              │
│  ┌────────────────────────────────────────────────────────────────┐    │
│  │ Layer 1 :  Envelope pre-allocation                             │    │
│  │                                                                │    │
│  │    envelope_trace_id = TraceId::new()                          │    │
│  │    EnvelopeBuilder::new(envelope_trace_id, request)            │    │
│  │      ↑                                                         │    │
│  │      └── carries : swarm_request_id, tenant, caller_agent_id,  │    │
│  │                    audit_tags, max_parallelism, timeout_ms,    │    │
│  │                    cost_cap_cents, streaming, idempotency_key, │    │
│  │                    op_count, op_ids, request_hash              │    │
│  └────────────────────────────────────────────────────────────────┘    │
│         │                                                              │
│         ▼                                                              │
│  ┌────────────────────────────────────────────────────────────────┐    │
│  │ Layer 2 :  Plan + Execute (existing path)                      │    │
│  │                                                                │    │
│  │    dag::plan(&request, &catalog) → DagPlan                     │    │
│  │    execute_plan_with_extras(                                   │    │
│  │        plan, request, runtime, audit, idempotency,             │    │
│  │        catalog, shadow_scheduler,                              │    │
│  │        envelope_trace_id,    ← NEW : threaded through          │    │
│  │    )                                                           │    │
│  │      └── runs ops in layers ; every op trace carries           │    │
│  │          `swarm_envelope_trace_id = Some(envelope_trace_id)`   │    │
│  │      └── cache-hit / shadow / comparison traces also carry it  │    │
│  └────────────────────────────────────────────────────────────────┘    │
│         │                                                              │
│         ▼                                                              │
│  ┌────────────────────────────────────────────────────────────────┐    │
│  │ Layer 3 :  Aggregation + envelope finalization                 │    │
│  │                                                                │    │
│  │    response = aggregate(outcomes)                              │    │
│  │    builder.set_outcome(                                        │    │
│  │        overall_status = response.overall_status,               │    │
│  │        total_cost_cents = response.total_cost_cents,           │    │
│  │        total_latency_ms = response.total_latency_ms,           │    │
│  │        op_status_counts = count(outcomes by status),           │    │
│  │    )                                                           │    │
│  │    envelope = builder.finalize()                               │    │
│  │    audit.record(envelope_trace)                                │    │
│  │    response.swarm_envelope_trace_id = Some(envelope_trace_id)  │    │
│  │    return response                                             │    │
│  └────────────────────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────────────────┘

           ┌──────── chain (per tenant) ────────┐
           │                                    │
   epoch K │ ... prior swarms ...               │
           │                                    │
           │ row N : op_1 trace                 │  ─ swarm_envelope_trace_id = X
           │ row N+1 : op_2 trace               │  ─ swarm_envelope_trace_id = X
           │ row N+2 : op_3 trace               │  ─ swarm_envelope_trace_id = X
           │ row N+3 : cache_hit (op_2 served)  │  ─ swarm_envelope_trace_id = X
           │ row N+4 : shadow_run (op_1)        │  ─ swarm_envelope_trace_id = X
           │ row N+5 : shadow_comparison        │  ─ swarm_envelope_trace_id = X
           │ row N+6 : ENVELOPE trace ─────────[X] (the swarm anchor row)
           │                                    │
           │ ... subsequent swarms ...          │
           └────────────────────────────────────┘
```

### 3.2 The shape of the change

| Layer | Today (pre-N14) | After N14 |
|---|---|---|
| Dispatcher | Returns `SwarmResponse` without writing a swarm-level trace | Pre-allocates envelope id, threads through, writes envelope at end, surfaces id on response |
| Audit chain | Op-traces only | Op-traces + one envelope trace per swarm |
| NanoAgentTrace | No swarm anchor field | New optional `swarm_envelope_trace_id` (operational-metadata, NOT in canonical body) |
| SwarmResponse | No envelope reference | New optional `swarm_envelope_trace_id` |
| Replay engine | Filters 4 sentinels (CacheHit, ShadowRun, ShadowComparison, RolloutPolicyChanged) | Adds 5th : SwarmEnvelope |
| GDPR cascade | Tombstones op traces individually (or swarm-wide via tombstone_traces_for_swarm) | New `tombstone_swarm_envelope` that's symmetric : envelope → ops |
| Forensic queries | Aggregate over op traces | Start from envelope, drill into ops |
| Metrics | N12 cache (4) + N13 rollout (9) | + 8 envelope metrics |

---

## 4. Component breakdown — 8 components

### 4.1 `SwarmEnvelopePayload` SDK type (NEW)

Living in `efe-nano-sdk/src/types/envelope.rs`.  The single struct holding everything we want to persist about a swarm.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmEnvelopePayload {
    // ─── request signature ───
    pub swarm_request_id: Uuid,
    pub tenant_id: TenantId,
    pub caller_agent_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub audit_tags: Vec<String>,
    pub max_parallelism: u32,
    pub timeout_ms: u32,
    pub cost_cap_cents: Option<f64>,
    pub streaming: bool,
    pub op_count: u32,
    pub op_ids: Vec<String>,
    pub request_hash: [u8; 32],                  // SHA-256 of canonical request JSON
    pub started_at: DateTime<Utc>,

    // ─── aggregated outcome ───
    pub overall_status: SwarmStatus,
    pub total_cost_cents: f64,
    pub total_latency_ms: u32,
    pub completed_at: DateTime<Utc>,
    pub op_status_counts: OpStatusCounts,
    pub cache_hit_count: u32,
    pub shadow_run_count: u32,
    pub shadow_mismatch_count: u32,
    pub canary_routed_count: u32,

    // ─── lineage ───
    pub parent_envelope_trace_id: Option<TraceId>,
    pub envelope_format_version: u16,            // 1 for Sprint 8 ship
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpStatusCounts {
    pub success: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cancelled: u32,
}
```

The `envelope_format_version: u16` lets Sprint 9+ add fields with forward/backward compatibility (deserialize old envelopes against v1 schema, new envelopes use v2+).

### 4.2 `NanoAgentTrace.swarm_envelope_trace_id` field (NEW)

Added to `efe_nano::audit::NanoAgentTrace`.  Type : `Option<TraceId>`.

- `None` for the envelope itself (it IS the envelope, has no parent envelope).
- `Some(envelope_id)` for every op trace within the swarm, including cache-hit, shadow-run, shadow-comparison traces.
- `None` for `RolloutPolicyChanged` virtual-swarm rows (they're not swarm-bound).

**Critical design decision** : this field is **excluded from `canonical_row_body`**.  Same pattern as `tombstone` (Sprint 7 N10).  Rationale :

1. **Backward compat** : Pre-N14 chain rows have no `swarm_envelope_trace_id`.  If we included it in the canonical body, every pre-N14 row's `payload_hash` would now be wrong (since the serializer would write `"swarm_envelope_trace_id": null` whereas pre-N14 rows hashed without that field).  Excluding it preserves verification.
2. **Operational metadata** : the envelope_id is a forensic pointer, not a cryptographic claim about the OP itself.  Cryptographic claims (input_hash, output_hash) stay in the canonical body.
3. **Tombstone-compatible** : if we want to erase the envelope reference (unlikely but possible — say, a privacy regulation says "don't link this op to its swarm batch"), we can null the field without breaking the chain.

Sprint 9 may **promote** the field to canonical via a `body_version: u16` field on `NanoAgentTrace` — pre-N14 rows verify under v1 canonical (no envelope id), post-N14 rows under v2 canonical (envelope id included).  Out of scope for Sprint 8.

### 4.3 Envelope trace sentinel

Following the N12 / N13 sentinel pattern :

```rust
pub const TRACE_KIND_SWARM_ENVELOPE: &str = "SwarmEnvelope";
```

The envelope trace has :

- `trace_id` = pre-allocated envelope_trace_id
- `error_kind` = `Some(TRACE_KIND_SWARM_ENVELOPE)`
- `nano_agent_id` = reserved id `swarm_envelope` (catalog rejects publishes with this id)
- `nano_agent_version` = `0.0.0` (placeholder semver)
- `operation_id` = `"swarm_envelope"`
- `execution_mode` = `"swarm_envelope"` (extends the existing string enum)
- `swarm_request_id` = the swarm's id
- `tenant_id` = the swarm's tenant
- `swarm_envelope_trace_id` = `None` (it IS the envelope)
- `parent_trace_ids` = `vec![]` for top-level, OR `vec![parent_envelope_trace_id]` for meta-swarm children
- `caller_agent_id` = the swarm's caller_agent_id
- `input_hash_source` = `request_hash` (SHA-256 of canonical request JSON)
- `input_hash_resolved` = `request_hash` (same — no Ref / Merge resolution at the envelope level)
- `output_hash` = SHA-256 of canonical aggregated outcome JSON
- `full_input` = `Some(serde_json::to_value(payload.request_signature_part)?)`
- `full_output` = `Some(serde_json::to_value(payload.outcome_part)?)`
- `cost_cents` = `total_cost_cents` (audit-visible total)
- `latency_ms` = `Some(total_latency_ms)`
- `started_at` / `completed_at` = swarm wall-clock bounds

### 4.4 `EnvelopeBuilder` (NEW module)

```rust
// crates/efe-nano/src/dispatcher/envelope.rs

pub struct EnvelopeBuilder {
    envelope_trace_id: TraceId,
    swarm_request_id: Uuid,
    tenant_id: TenantId,
    started_at: DateTime<Utc>,
    started_instant: Instant,
    // From SwarmRequest
    caller_agent_id: Option<String>,
    idempotency_key: Option<String>,
    audit_tags: Vec<String>,
    max_parallelism: u32,
    timeout_ms: u32,
    cost_cap_cents: Option<f64>,
    streaming: bool,
    op_count: u32,
    op_ids: Vec<String>,
    request_hash: [u8; 32],
    parent_envelope_trace_id: Option<TraceId>,
    // Accumulators (mutated as ops complete)
    op_status_counts: OpStatusCounts,
    cache_hit_count: u32,
    shadow_run_count: u32,
    shadow_mismatch_count: u32,
    canary_routed_count: u32,
    total_cost_cents: f64,
}

impl EnvelopeBuilder {
    pub fn new(envelope_trace_id: TraceId, request: &SwarmRequest) -> Self;

    pub fn finalize(self, response: &SwarmResponse) -> SwarmEnvelopePayload;
}
```

For Sprint 8 ship, the EnvelopeBuilder is **construction-only at swarm start + finalization-only at swarm end** — we don't mutate it as ops complete.  Instead, the accumulators are computed at finalize time by walking the outcomes map (cheap : O(K) over the op set).

Sprint 9 may swap to a mutable accumulator pattern for streaming.

### 4.5 `dispatcher::envelope::write_envelope` (NEW)

```rust
pub async fn write_envelope(
    audit: &dyn AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
) -> TraceId {
    let payload = builder.finalize(response);
    let envelope_trace = build_swarm_envelope_trace(&payload);
    let envelope_trace_id = envelope_trace.trace_id;
    audit.record(envelope_trace).await;
    envelope_trace_id
}
```

The `build_swarm_envelope_trace` helper constructs a full `NanoAgentTrace` from the payload.  Lives in the same module.

### 4.6 `Dispatcher::dispatch` integration

```rust
pub async fn dispatch(&self, request: SwarmRequest) -> Result<SwarmResponse, DispatcherError> {
    let started = Instant::now();
    let request_id = request.request_id;
    let swarm_budget = std::time::Duration::from_millis(request.timeout_ms as u64);

    // N14 : pre-allocate envelope trace id.
    let envelope_trace_id = TraceId::new();
    let builder = EnvelopeBuilder::new(envelope_trace_id, &request);

    // Plan
    let plan = match dag::plan(&request, &*self.catalog).await {
        Ok(p) => p,
        Err(e) => {
            // N14 : plan-failure envelope still emitted.
            let response = synth_planner_failure_response(&request, started.elapsed().as_millis() as u32, &e);
            self.write_envelope_safely(builder, &response, &HashMap::new()).await;
            return Err(e);
        }
    };

    // Execute (envelope_trace_id threaded through)
    let outcomes_result = tokio::time::timeout(
        swarm_budget,
        executor::execute_plan_with_extras(
            &plan, &request,
            Arc::clone(&self.runtime),
            Arc::clone(&self.audit),
            Arc::clone(&self.idempotency),
            Some(Arc::clone(&self.catalog)),
            self.shadow_scheduler.as_ref().map(Arc::clone),
            Some(envelope_trace_id),  // NEW : threaded through
        ),
    ).await;

    let (outcomes, status_for_timeout) = match outcomes_result {
        Ok(Ok(o)) => (o, None),
        Ok(Err(e)) => {
            // Internal dispatcher error.  Write envelope + propagate.
            let response = synth_internal_failure_response(&request, started.elapsed().as_millis() as u32, &e);
            self.write_envelope_safely(builder, &response, &HashMap::new()).await;
            return Err(e);
        }
        Err(_elapsed) => {
            let total_latency_ms = started.elapsed().as_millis() as u32;
            let response = timed_out_response(&request, total_latency_ms);
            self.write_envelope_safely(builder, &response, &HashMap::new()).await;
            return Ok(response);
        }
    };

    // Aggregate
    let total_latency_ms = started.elapsed().as_millis() as u32;
    let mut response = aggregate(request_id, &request, outcomes.clone(), total_latency_ms);

    // N14 : write envelope, then attach id to response.
    let envelope_id = self.write_envelope_safely(builder, &response, &outcomes).await;
    response.swarm_envelope_trace_id = Some(envelope_id);

    Ok(response)
}

async fn write_envelope_safely(
    &self,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
) -> TraceId {
    let envelope_trace_id = builder.envelope_trace_id();
    // write_envelope is fire-and-forget-resilient : audit failures don't crash the dispatch.
    dispatcher::envelope::write_envelope(&*self.audit, builder, response, outcomes).await;
    envelope_trace_id
}
```

### 4.7 `executor::run_one_op` plumbing

Pass `envelope_trace_id` into the `CallContext` so the runtime can set it on every op trace :

```rust
let mut ctx = CallContext::from_swarm(
    tenant_id,
    swarm_request_id,
    handle.op.op_id.clone(),
    meta.parent_trace_ids.clone(),
    caller_agent_id.clone(),
    Arc::clone(&audit),
    handle.effective_timeout_ms,
    Some(Arc::clone(&cost_tracker)),
);
ctx.swarm_envelope_trace_id = envelope_trace_id;  // NEW
ctx.trace_id = meta.trace_id;
ctx.input_hash_source_override = Some(meta.input_hash_source);
```

The runtime modules (`pure_rust.rs`, `local_model.rs`, `llm_augmented.rs`) propagate it into the `NanoAgentTrace` they emit :

```rust
let trace = NanoAgentTrace {
    trace_id: ctx.trace_id,
    parent_trace_ids: ctx.parent_trace_ids.clone(),
    swarm_envelope_trace_id: ctx.swarm_envelope_trace_id,  // NEW
    // ... all other fields ...
};
```

The dispatcher's `write_cache_hit_trace` (N12) and `write_shadow_run_trace` / `write_shadow_comparison_trace` (N13) similarly set the field.

### 4.8 `ReplayEngine::envelope_for_swarm`

```rust
impl ReplayEngine {
    pub async fn envelope_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<SwarmEnvelopePayload, ReplayError> {
        let traces = self.chain_store.traces_for_swarm(tenant, swarm_request_id).await?;
        let envelope_entry = traces
            .iter()
            .find(|e| e.trace.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
            .ok_or(ReplayError::EnvelopeNotFound(swarm_request_id))?;
        let payload: SwarmEnvelopePayload = serde_json::from_value(
            envelope_entry
                .trace
                .full_input
                .clone()
                .ok_or(ReplayError::EnvelopeBodyMissing(swarm_request_id))?,
        )?;
        Ok(payload)
    }
}
```

---

## 5. `SwarmEnvelopePayload` — the on-disk source of truth

### 5.1 Field-by-field rationale

**Request signature** (frozen at swarm start) :

| Field | Type | Source | Why |
|---|---|---|---|
| `swarm_request_id` | `Uuid` | `request.request_id` | Primary key for forensics |
| `tenant_id` | `TenantId` | `request.tenant_id` | Tenant scope ; cross-tenant query gate |
| `caller_agent_id` | `Option<String>` | `request.caller_agent_id` | Telemetry + lineage |
| `idempotency_key` | `Option<String>` | `request.idempotency_key` | Customer-side dedup key |
| `audit_tags` | `Vec<String>` | `request.audit_tags` | Customer-set forensic tags |
| `max_parallelism` | `u32` | `request.max_parallelism` | Pool sizing snapshot |
| `timeout_ms` | `u32` | `request.timeout_ms` | SLA snapshot |
| `cost_cap_cents` | `Option<f64>` | `request.cost_cap_cents` | N7 cap config snapshot |
| `streaming` | `bool` | `request.streaming` | Mode flag |
| `op_count` | `u32` | `request.operations.len()` | Quick count for dashboards |
| `op_ids` | `Vec<String>` | `request.operations.map(|o| o.op_id)` | Discovery of op rows by op_id |
| `request_hash` | `[u8; 32]` | SHA-256(canonical JSON of full request) | Tamper-evidence on the request bytes |
| `started_at` | `DateTime<Utc>` | wall-clock at dispatch start | Time anchor |
| `parent_envelope_trace_id` | `Option<TraceId>` | `request.parent_envelope_trace_id` (new) | Meta-swarm lineage |
| `envelope_format_version` | `u16` | constant `1` for Sprint 8 | Forward / backward compat |

**Aggregated outcome** (computed at swarm end) :

| Field | Type | Source | Why |
|---|---|---|---|
| `overall_status` | `SwarmStatus` | `response.overall_status` | Audit-grade single truth |
| `total_cost_cents` | `f64` | `response.total_cost_cents` | Billing reconciliation |
| `total_latency_ms` | `u32` | `response.total_latency_ms` | SLA reporting |
| `completed_at` | `DateTime<Utc>` | wall-clock at envelope-write | Time anchor end |
| `op_status_counts` | `OpStatusCounts` | derived from outcomes | Quick dashboard pivot |
| `cache_hit_count` | `u32` | count of `outcomes` with `cache_hit=true` | N12 telemetry summary |
| `shadow_run_count` | `u32` | count of shadow traces this swarm produced | N13 telemetry summary |
| `shadow_mismatch_count` | `u32` | count of mismatch outcomes | N13 telemetry summary |
| `canary_routed_count` | `u32` | count of ops whose agent_version matched the canary policy version | N13 telemetry summary |

### 5.2 Why `request_hash` and not raw request body ?

Two reasons :

1. **Size** — a SwarmRequest can be many MB (large literal inputs).  Storing the full body inline in every envelope row inflates the chain.
2. **GDPR** — the raw request contains PII (in inputs).  Per-op input bodies already live in the side-store (Sprint 6) with refcount-based purging.  Adding a SECOND copy in the envelope's `full_input` would duplicate PII and complicate tombstone.  Instead the envelope hashes the request and stores only the **signature** (the configuration fields).  Per-op inputs remain reachable via op traces' `full_input` (or the side-store).

If a Sprint 9 use case demands "envelope's full_input includes the FULL request including inputs", we can extend `SwarmEnvelopePayload` with an opt-in `full_request_body: Option<serde_json::Value>` field (default None).

### 5.3 Why `op_ids` and not just `op_count` ?

`op_ids` is bounded (typically < 100 entries × ~16 bytes each ≈ < 2 KB per envelope).  It enables fast "find the op_3 trace within this swarm" queries without scanning the chain.  Useful for dashboard drill-down.

For mega-swarms (Sprint 10 batch jobs with 10K+ ops), `op_ids` would balloon.  Mitigation : envelope schema v2 adds `op_ids_truncated: bool` + caps at the first 1000.  Not a Sprint 8 concern.

### 5.4 Why an outcome-time `op_status_counts` instead of derived-on-read ?

Pre-aggregated counts let dashboards display "X swarms had failures last week" without reading every op trace.  The cost is the marginal storage (16 bytes per envelope) ; the benefit is O(1) read latency on the most common dashboard query.

### 5.5 What about the `cache_hit_count` / `shadow_*_count` / `canary_routed_count` ?

These pre-aggregate the N12 / N13 sub-event counts.  Dashboards can directly chart "shadow_mismatch_rate per tenant per day" by reading envelope rows alone.

---

## 6. Envelope lifecycle

### 6.1 Successful swarm — happy path

```
T0  : caller dispatches Dispatcher::dispatch(request)
T0  : envelope_trace_id pre-allocated, EnvelopeBuilder::new(...)
T0  : dag::plan(&request, &catalog) → DagPlan
T1  : execute_plan_with_extras(... envelope_trace_id ...)
T1+ : per-op trace written for each op (carries swarm_envelope_trace_id)
T1+ : cache-hit / shadow / comparison traces also carry swarm_envelope_trace_id
T2  : aggregate(outcomes) → SwarmResponse { overall_status, total_cost, total_latency, results, ... }
T2  : EnvelopeBuilder::finalize(response) → SwarmEnvelopePayload
T2  : audit.record(envelope_trace)  ← the swarm's anchor row
T2  : response.swarm_envelope_trace_id = Some(envelope_trace_id)
T2  : return response to caller
```

### 6.2 Timeout path

```
T0  : ... same as above ...
T1  : execute_plan_with_extras spawns, runs ops
Tt  : timeout_ms exceeded ; tokio::time::timeout returns Err
Tt  : dispatcher synthesises timed_out_response
Tt  : EnvelopeBuilder::finalize_with_timeout(...) → envelope with overall_status = TimedOut
Tt  : audit.record(envelope_trace)
Tt  : return Ok(timed_out_response) to caller
```

Op traces : whatever was already written before the timeout fires stays in the chain.  Ops that were in-flight at timeout : their futures may complete in the background (we don't cancel them), and their traces may land AFTER the envelope.  Auditor sees the envelope at T_t with overall_status=TimedOut, and may see op traces at T_t+ε with `swarm_envelope_trace_id = Some(envelope_trace_id)`.

This is acceptable : the envelope's `op_status_counts` reflects what was known at T_t.  Sprint 9 may add a "post-timeout reconciliation" envelope update.

### 6.3 Planner failure path

```
T0  : dispatcher pre-allocates envelope_trace_id + builder
T0  : dag::plan(&request, &catalog) → Err(DispatcherError::InvalidDag { ... })
T0  : dispatcher synthesises a planner-failure response (op count = 0, all skipped)
T0  : EnvelopeBuilder::finalize(planner_failure_response) → envelope with overall_status = Failed
       + op_status_counts.failed = 0 (no ops attempted)
T0  : audit.record(envelope_trace)  ← still emitted for forensics
T0  : return Err to caller
```

This closes **Gap 2** : preflight failures DO get an envelope row, even though no op traces were written.

### 6.4 Internal-error path (executor crash, etc.)

If `execute_plan_with_extras` returns `Err(DispatcherError)` (rare ; typically only on programmer error or backend outage) :

```
T0  : ... same ...
T1  : execute_plan_with_extras → Err(DispatcherError::Internal(...))
T1  : synth internal-failure response, envelope status = Failed
T1  : audit.record(envelope_trace)
T1  : return Err to caller
```

### 6.5 Dispatcher-side crash (e.g., OOM)

The process dies mid-dispatch.  No envelope is written.  Orphaned op traces remain in the chain with `swarm_envelope_trace_id = Some(uuid_X)` but no row with `trace_id = uuid_X`.

Sprint 9 adds a **stale-envelope sweeper** : a cron job that scans the chain for orphaned `swarm_envelope_trace_id` references > 1 hour old and writes a synthetic "envelope-orphaned" virtual trace.  Out of scope for Sprint 8.

### 6.6 Cache-hit-only swarm

```
T0  : ... same ...
T1  : every op resolves to a cache hit (no runtime call)
T1+ : CacheHit traces written (N12), each carries swarm_envelope_trace_id
T2  : envelope written with op_status_counts.success = K, cache_hit_count = K
```

The envelope's `total_cost_cents` matches `sum(cached.cost_cents)` per N12's commitment to surface cost on cache hits.

### 6.7 Shadow-active swarm

```
T0  : ... same ...
T1  : ops dispatch ; shadow policy triggers shadow runs (N13)
T1  : prod ops emit traces, shadow ops emit ShadowRun traces, comparisons emit ShadowComparison
T1+ : all carry swarm_envelope_trace_id
T2  : envelope written with shadow_run_count = K (matching shadow policy's coverage)
```

---

## 7. Per-op trace linkage via `swarm_envelope_trace_id`

### 7.1 The field

```rust
pub struct NanoAgentTrace {
    pub trace_id: TraceId,
    pub parent_trace_ids: Vec<TraceId>,     // N1 — DAG lineage WITHIN a swarm
    pub swarm_envelope_trace_id: Option<TraceId>,  // N14 — swarm anchor pointer
    pub swarm_request_id: Uuid,
    // ... rest unchanged ...
}
```

### 7.2 Who sets it ?

Every code path that constructs a `NanoAgentTrace` :

1. **Runtime code paths** (`pure_rust.rs`, `local_model.rs`, `llm_augmented.rs`) — read from `ctx.swarm_envelope_trace_id`, set on the trace they emit.
2. **`write_cache_hit_trace`** (N12) — accepts the envelope id as a parameter, sets on the trace.
3. **`write_shadow_run_trace`** + **`write_shadow_comparison_trace`** (N13) — same.
4. **`build_rollout_policy_changed_trace`** (N13) — sets to `None` (policy-change traces are NOT swarm-bound).
5. **`build_swarm_envelope_trace`** (N14) — sets to `None` (envelope IS the swarm anchor, has no swarm anchor of its own).

### 7.3 Why on cache-hit / shadow / comparison traces ?

So that forensics queries like "what audit-metadata traces did swarm X produce ?" return the complete set — every kind of trace gets pulled by the swarm anchor.

A query like :
```sql
SELECT trace_id, error_kind FROM chain
WHERE swarm_envelope_trace_id = $envelope_id;
```
returns 1 row per op, 1 per cache hit, 1 per shadow run, 1 per shadow comparison.  Exact reconstruction of the swarm's audit activity.

### 7.4 What about `RolloutPolicyChanged` ?

`RolloutPolicyChanged` traces (N13 § 4.3) are virtual-swarm rows with `swarm_request_id = ROLLOUT_VIRTUAL_SWARM_UUID`.  They are NOT part of any real swarm — they record an operator action.

Their `swarm_envelope_trace_id` is `None`.  Documenting this in the trait contract.

### 7.5 Indexing for fast lookup

In the in-memory chain store, no extra index is needed (linear scan is fine for the typical 1000-100K chain).  In the Phase 4 Postgres schema (Annexe B), an index :

```sql
CREATE INDEX idx_chain_envelope_id ON chain_entry(swarm_envelope_trace_id)
    WHERE swarm_envelope_trace_id IS NOT NULL;
```

makes "all traces of swarm X" an O(log N) lookup.

---

## 8. Audit chain integration (HMAC + canonical body)

### 8.1 The compatibility guarantee

**Invariant** : adding the `swarm_envelope_trace_id` field to `NanoAgentTrace` MUST NOT break HMAC verification of pre-N14 chain rows.

### 8.2 Why it works

`canonical_row_body` (Sprint 6 spec, in `audit/payload.rs`) is the canonical serialisation of the trace fields that go into `payload_hash`, which is then included in the HMAC computation.

Excluding `swarm_envelope_trace_id` from `canonical_row_body` means :

* Pre-N14 row : serialised without the field ; payload_hash matches verification ; HMAC unchanged.
* Post-N14 row : the field is present on the struct but the canonical body skips it ; payload_hash matches verification.

The chain's tamper-evidence property is preserved end-to-end.

### 8.3 The trade-off

A malicious actor with chain write access could mutate `swarm_envelope_trace_id` on a stored trace without breaking the HMAC.  This is the same trade-off as the `tombstone` field (Sprint 7 N10).

For Sprint 8 ship, this is acceptable :
* `swarm_envelope_trace_id` is operational metadata, not a cryptographic claim about the OP itself.
* If an attacker has chain write access, they can do worse damage (forge new rows entirely).
* The envelope row itself ANCHORS the cryptographic claim (its `trace_id` is fixed, its `full_input` carries the request signature, its `output_hash` covers the aggregated outcome).

Sprint 9 promotion path : add a `body_version: u16` field to `NanoAgentTrace`.  Body v1 (pre-N14) excludes `swarm_envelope_trace_id` ; body v2 includes it.  Verifier reads `body_version` per row, applies the right canonicalization.

### 8.4 The envelope's own canonical body

The envelope IS a NanoAgentTrace.  Its `canonical_row_body` includes everything except the excluded fields (tombstone, swarm_envelope_trace_id).  Specifically :

* `swarm_request_id`, `tenant_id`, `caller_agent_id`, `nano_agent_id`, `nano_agent_version`, `execution_mode`
* `input_hash_source = request_hash`
* `input_hash_resolved = request_hash`
* `output_hash` = SHA-256 of aggregated outcome JSON
* `started_at`, `completed_at`, `latency_ms`, `cost_cents`
* `error_kind = Some("SwarmEnvelope")`

The envelope's `full_input` and `full_output` are NOT part of canonical body (consistent with op traces — full bodies live in side-store).

### 8.5 Side-store interaction

The envelope's `full_input` (the request signature part of the payload) AND `full_output` (the aggregated outcome part) are stored in the side-store (same pattern as op traces).  The side-store key is `input_hash_resolved` for inputs and `output_hash` for outputs.

When the envelope is tombstoned, the side-store entries are refcount-decremented (and purged if refcount reaches zero) — same N10 contract as op traces.

### 8.6 Merkle epoch (N9) interaction

The envelope is a regular chain entry.  It gets included in the current epoch's Merkle tree by the `EpochSealer` (Sprint 7 N9).  An inclusion proof for the envelope is the same shape as for any op trace.

Sprint 9 may add `EpochSealer::envelope_inclusion_proof(envelope_trace_id)` as a convenience shortcut for forensics.

---

## 9. Replay engine integration

### 9.1 The new query

```rust
impl ReplayEngine {
    /// **N14** : Read the SwarmEnvelopePayload for a swarm.  The first
    /// query a forensic investigation starts with.
    pub async fn envelope_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<SwarmEnvelopePayload, ReplayError>;
}
```

### 9.2 The filter extension

Replay treats the envelope as audit-metadata, not as an op to replay.  The existing filter (N12 + N13) is extended :

```rust
let traces: Vec<_> = all_traces
    .into_iter()
    .filter(|entry| {
        !matches!(
            entry.trace.error_kind.as_deref(),
            Some("CacheHit")
                | Some("ShadowRun")
                | Some("ShadowComparison")
                | Some("RolloutPolicyChanged")
                | Some("SwarmEnvelope")              // N14
        )
    })
    .collect();
```

### 9.3 Why filter ?

The replay engine reconstructs the SwarmRequest and re-dispatches.  The envelope is a SUMMARY, not an op.  Replaying it would be nonsensical (re-dispatching a summary).

### 9.4 The envelope as a replay-time aid

When the replay engine reconstructs the SwarmRequest, it can use the envelope's `full_input` (request signature) to **verify** the reconstruction.  Cross-check :

```rust
let envelope = engine.envelope_for_swarm(tenant, swarm_id).await?;
let reconstructed = engine.reconstruct(traces)?;
assert_eq!(envelope.max_parallelism, reconstructed.max_parallelism);
assert_eq!(envelope.audit_tags, reconstructed.audit_tags);
// ... etc ...
```

Mismatch → audit anomaly (the chain was tampered with between envelope-write and op-write).

### 9.5 Replay of a swarm without an envelope (pre-N14 swarms)

A swarm dispatched before N14 was deployed has NO envelope row.  `envelope_for_swarm` returns `EnvelopeNotFound`.  Replay still works via the existing path (`reconstruct_with_bodies`) — the envelope is an aid, not a requirement.

---

## 10. GDPR tombstone cascade extension

### 10.1 Two cascade directions

After N14, two tombstone-cascade flows :

**Direction A (existing N10 + N13)** : Tombstone an op trace → cascade to its shadow + comparison children.

**Direction B (NEW N14)** : Tombstone the **envelope** → cascade to every op trace of the same swarm.

### 10.2 The new method

```rust
#[async_trait]
pub trait ChainStore {
    // ... existing methods ...

    /// **N14** : Tombstone the envelope trace AND every op / cache-hit /
    /// shadow / comparison trace of the same swarm_request_id.
    /// Symmetric to "tombstone the prod cascades to shadow children" :
    /// here the swarm anchor cascades to all its members.
    ///
    /// Returns the aggregated outcome.  Default impl tombstones only
    /// the envelope (backward-compat) ; InMemoryChainStore overrides
    /// with the full cascade.
    async fn tombstone_swarm_envelope(
        &self,
        tenant: TenantId,
        envelope_trace_id: Uuid,
        metadata: TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError>;
}
```

### 10.3 The algorithm

1. Fetch the envelope row.  Verify it IS an envelope (`error_kind = SwarmEnvelope`).  Otherwise error : `ChainStoreError::NotAnEnvelope`.
2. Read the envelope's `swarm_request_id` from the row.
3. Fetch all traces of the same swarm via `traces_for_swarm(tenant, swarm_request_id)`.
4. Tombstone each trace via `tombstone_trace` (idempotent — re-applying is a no-op).
5. Return aggregated `TombstoneOutcome` (sum of `traces_tombstoned`, `input_bodies_purged`, `output_bodies_purged`).

The envelope itself is included in step 3 (it shares the `swarm_request_id`).

### 10.4 GDPR use case

A tenant invokes "delete my data from October 5th" → M24 GDPR saga determines the affected swarms → for each, calls `tombstone_swarm_envelope`.  Atomic erasure of every audit row of the batch.

The chain HMAC integrity is preserved (tombstones excluded from canonical body).  Auditors see the tombstoned envelope + tombstoned ops, hashed identifiers preserved, full bodies erased.

### 10.5 Cache cascade

When an envelope is tombstoned, the N12 idempotency cache must also be cleared :

```rust
// GDPR saga implementation (Phase 4) :
chain_store.tombstone_swarm_envelope(tenant, envelope_id, meta).await?;
for trace in chain_store.traces_for_swarm(tenant, swarm_request_id).await? {
    idempotency.flush_for_original_trace(trace.trace.trace_id).await;
}
```

The idempotency cache's `flush_for_original_trace` (Sprint 8 N12) handles per-trace eviction.

---

## 11. N11 / N12 / N13 alignment

### 11.1 N11 (Firecracker pool) — no interaction

The Firecracker pool runs INSIDE op execution.  Envelope is a layer above.  No cross-cutting.

### 11.2 N12 (cache key hardening) — cache hits carry envelope id

When a cache hit serves a previously-computed output, the cache-hit trace (N12 § 4.3) MUST carry the CURRENT swarm's envelope id, NOT the original (cache-populating) swarm's envelope id.

Implementation : `write_cache_hit_trace` accepts the envelope id as a parameter (threaded from `run_one_op`'s context).

Why : the cache hit is an event in the CURRENT swarm, even though the value originates from a past swarm.  Forensics queries "what was the current swarm's audit activity" must see the cache hit.

### 11.3 N13 (shadow / canary) — shadow + comparison traces carry envelope id

Same pattern.  `ShadowScheduler::schedule` accepts the envelope id, passes it via `ShadowDispatchContext`, the scheduler's `write_shadow_run_trace` + `write_shadow_comparison_trace` set the field.

The envelope's `shadow_run_count` aggregates `count(shadow_run_trace where swarm_envelope_trace_id = envelope_id)`.

### 11.4 N13 + N14 — canary routing aggregation

`canary_routed_count` in the envelope counts ops dispatched to a non-prod version :

```rust
canary_routed_count = outcomes.values().filter(|o| {
    let trace_version = traces_for_op(o.op_id).first().version();
    let active_policy = catalog.get_rollout_policy(tenant, agent_id).await.unwrap_or_default();
    match active_policy {
        RolloutPolicy::Canary { prod_version, canary_version, .. } => {
            trace_version == canary_version && trace_version != prod_version
        }
        _ => false,
    }
}).count() as u32;
```

For Sprint 8 ship, this is computed at envelope finalization time using the policy at swarm completion (NOT at swarm start — racy but acceptable).  Sprint 9 may snapshot the policy at swarm start for stricter semantics.

---

## 12. Meta-swarm lineage (forward-compatible)

### 12.1 The use case

Sprint 9 N15 plans **meta-swarms** : a top-level swarm whose ops include "dispatch a child swarm".  Lineage :

```
Meta swarm S0
 ├── op_a : invokes child swarm S1
 │     └── envelope(S1) { parent_envelope_trace_id = envelope(S0) }
 ├── op_b : regular nano agent
 └── op_c : invokes child swarm S2
       └── envelope(S2) { parent_envelope_trace_id = envelope(S0) }
```

### 12.2 The field

`SwarmEnvelopePayload.parent_envelope_trace_id: Option<TraceId>`.

For Sprint 8 ship :
- The field exists but is set ONLY when the caller explicitly populates `SwarmRequest.parent_envelope_trace_id` (NEW optional field on SwarmRequest, default None).
- No automatic meta-swarm wiring.  A custom caller can use it ; the platform doesn't yet spawn child swarms automatically.

Sprint 9 N15 wires the meta-swarm executor.

### 12.3 Forensic query

```sql
-- Find all child envelopes of meta-swarm M :
SELECT * FROM chain_entry
WHERE error_kind = 'SwarmEnvelope'
  AND (full_input->>'parent_envelope_trace_id')::uuid = $meta_envelope_id;
```

---

## 13. Streaming sub-results — Sprint 9 horizon

### 13.1 Current limitation

Sprint 8 envelope is **single-emit** : one row at swarm completion, summarising everything.

Sprint 9 plans **streaming envelopes** for long-running swarms (think batch-mode Mode C with 100 ops over 5 minutes).  Customers want partial progress visibility :

* "10 of 50 ops complete"
* "current `total_cost_cents = 0.42`"
* "shadow_mismatch_count just incremented to 3"

### 13.2 Forward-compatible design

The single-emit Sprint 8 design is forward-compatible if we add a `envelope_update_kind: EnvelopeKind` discriminator to the schema :

```rust
pub enum EnvelopeKind {
    /// Sprint 8 default — one terminal envelope per swarm.
    Terminal,
    /// Sprint 9 streaming — intermediate progress update.
    Progress { update_seq: u32 },
    /// Sprint 9 streaming — final closing envelope.
    Final { total_updates: u32 },
}
```

For Sprint 8 ship, every envelope is `EnvelopeKind::Terminal`.

Sprint 9 may introduce `Progress` / `Final` variants without breaking deserialization of existing Terminal envelopes.

---

## 14. Failure modes — F1 to F14

### F1 — Audit write fails on envelope emit

Production succeeded, the dispatcher's `audit.record(envelope_trace)` errors (e.g. Postgres outage).  The dispatcher MUST NOT crash — `audit.record` is fire-and-forget-resilient by N6 contract.

Outcome : op traces are in the chain, envelope is missing.  Detection : the stale-envelope sweeper (Sprint 9).  Mitigation in Sprint 8 : log a `tracing::error!` so ops engineers notice.

### F2 — Pre-allocation collision

Theoretically two swarms could pre-allocate the same `TraceId` (UUID v4 collision ≈ 2⁻¹²² per call).  Effectively impossible.

If detected at write time (UNIQUE constraint failure in Postgres), the dispatcher generates a new id and retries.

### F3 — Op trace lands AFTER the envelope (timeout case)

The envelope is written at T_t when the timeout fires.  An op that was in-flight finishes at T_t+ε and writes its trace.  The op trace carries the envelope_trace_id ; the envelope's `op_status_counts` doesn't include it.

Auditor sees the discrepancy and queries "all traces for swarm X" → finds the orphan op trace.

Sprint 9 may add a sweeper that updates envelope's `op_status_counts` post-timeout.

### F4 — Envelope size exceeds chain row limit

A swarm with 100 000 ops has `op_ids: Vec<String>` of size ~1.5 MB.  Postgres limit (Sprint 9) is configurable but typically 1 MB per row.

Mitigation : envelope schema v2 truncates `op_ids` at 1000 entries + sets `op_ids_truncated: true`.  Sprint 9 deliverable.

### F5 — Tombstone race

User invokes "tombstone swarm X" at T_x.  At T_x+1ms a new op trace for swarm X lands (process queue).  Outcome : the new op trace is NOT tombstoned by the original cascade ; auditor sees a non-tombstoned op against a tombstoned envelope.

Mitigation : the GDPR saga (M24) re-runs `tombstone_swarm_envelope` on a delayed sweep (after ops would have settled).  Sprint 9 wiring.

### F6 — Replay of a tombstoned envelope

`engine.envelope_for_swarm(tenant, swarm_id)` returns a tombstoned envelope (full_input erased).  Replay can't reconstruct the request.  Returns `ReplayError::EnvelopeTombstoned`.  Expected behaviour.

### F7 — Tombstone metadata missing

If `tombstone_swarm_envelope` is called without a valid `TombstoneMetadata` (e.g. wrong tenant audit key), the underlying `tombstone_trace` errors per N10.  The cascade aborts mid-way ; partial tombstones are applied.

Mitigation : the GDPR saga retries with backoff.  Sprint 8 ships with this idempotent-retry path.

### F8 — Concurrent envelope write attempts

Two threads dispatch the same `SwarmRequest` (same `request_id`).  Both pre-allocate different envelope_trace_ids (UUIDs differ).  Both write envelopes.  Outcome : two envelopes for the same swarm_request_id.

The chain accepts this — uniqueness is on `trace_id`, not `swarm_request_id`.  Forensics see two envelopes for one swarm.  Auditor's discretion : "did the customer retry ?".

This is a customer-side concurrency issue ; the platform records the truth.

### F9 — Caller-supplied parent_envelope_trace_id is invalid

Caller sets `parent_envelope_trace_id = Uuid::nil()` (or a non-existent envelope id).  The envelope is still written with whatever the caller said.  Sprint 8 doesn't validate the reference (Sprint 9 N15 wires meta-swarm verification).

Forensics see a dangling parent reference.  Expected for Sprint 8.

### F10 — Envelope cost reporting drift

The envelope's `total_cost_cents` is `response.total_cost_cents`, which is the dispatcher's aggregation.  If a cache-hit cost was incorrectly tracked (N12 bug pre-Sprint-8), the envelope inherits the drift.

Mitigation : N12 tests verify cache hit cost surfacing.  Cross-check in tests : envelope's `total_cost_cents` equals sum of op trace `cost_cents`.

### F11 — N13 shadow scheduled but envelope written before shadow completes

Shadow runs are async (N13).  The envelope writes at swarm completion, BEFORE the shadow finishes.  The envelope's `shadow_run_count` is computed pre-shadow-finalization — it counts SCHEDULED shadows, not completed ones.

This is acceptable for Sprint 8.  Forensics queries that need post-shadow data join "envelope" with "shadow_traces filtered by envelope_trace_id" and get fresh counts.

Sprint 9 may add a post-shadow envelope update.

### F12 — Envelope's request_hash mismatch with op input_hash_source

A swarm with one op whose input is `Literal { value: v }`.  The op's `input_hash_source` is `hash(canonical(v))`.  The envelope's `request_hash` is `hash(canonical(entire_request))` — which INCLUDES the op + every other request field.

These two hashes are DIFFERENT and that's correct.  Auditors verifying integrity use both : request_hash anchors the request, op input_hash_source anchors each op input.

### F13 — Streaming flag interaction

`SwarmRequest.streaming: bool` is recorded in the envelope.  Sprint 8 streaming dispatch is not yet wired ; the envelope captures the request's intent.

### F14 — Schema version drift

Envelope `envelope_format_version: u16` is `1` for Sprint 8.  Sprint 9 deploys envelope v2 schema (adds streaming-update fields).  Forward compat : old readers ignore unknown fields.  Backward compat : new readers default missing v2 fields to None.

The `envelope_format_version` field lets readers branch on schema version explicitly.

---

## 15. Invariants — I1 to I18

| ID | Invariant | Verification |
|---|---|---|
| I1 | Exactly one envelope row per successful swarm | `envelope_uniqueness_per_swarm` |
| I2 | Envelope row error_kind == "SwarmEnvelope" | structural test |
| I3 | Envelope row's nano_agent_id == reserved sentinel | structural test |
| I4 | Every op trace within a swarm has swarm_envelope_trace_id == Some(envelope_id) | `op_traces_carry_envelope_id` |
| I5 | Envelope's own swarm_envelope_trace_id == None | structural |
| I6 | Envelope's parent_trace_ids == empty (or [parent_envelope]) | structural |
| I7 | request_hash deterministic over canonical request | hash invariance test |
| I8 | overall_status in envelope matches the SwarmResponse returned to the caller | E2E test |
| I9 | total_cost_cents in envelope == response.total_cost_cents | E2E test |
| I10 | op_status_counts sums to op_count | invariant test |
| I11 | HMAC chain verifies pre-N14 + post-N14 rows together | chain integrity test |
| I12 | Replay engine filters SwarmEnvelope sentinel (no replay attempts) | replay filter test |
| I13 | tombstone_swarm_envelope erases envelope + every op trace of the swarm | cascade test |
| I14 | Tombstoned envelope's full_input == None | post-cascade test |
| I15 | envelope.cache_hit_count + envelope.op_status_counts.success >= cache_hit_count (cache hits count as success) | summation test |
| I16 | envelope.shadow_run_count counts only scheduled shadows ; comparison count is separate field | structural |
| I17 | Envelope is part of N9 Merkle epoch | epoch inclusion test |
| I18 | RolloutPolicyChanged virtual rows have swarm_envelope_trace_id == None | structural |

---

## 16. Metrics — 8 Prometheus surfaces

```rust
pub struct EnvelopeMetrics {
    /// Total envelopes successfully written.
    pub envelopes_written_total: Counter,
    /// Total envelope writes that failed (audit backend outage etc.).
    pub envelopes_write_failed_total: Counter,
    /// Histogram of swarm wall-clock latencies (ms).  Source = envelope.total_latency_ms.
    pub envelope_latency_ms_histogram: Histogram,
    /// Histogram of swarm op_count.  Source = envelope.op_count.
    pub envelope_op_count_histogram: Histogram,
    /// Histogram of envelope total_cost_cents.  Source = envelope.total_cost_cents.
    pub envelope_total_cost_cents_histogram: Histogram,
    /// Total envelopes tombstoned via tombstone_swarm_envelope.
    pub envelope_tombstones_total: Counter,
    /// Total ReplayEngine::envelope_for_swarm queries.
    pub envelopes_queried_total: Counter,
    /// Histogram of envelope size (canonical-serialised bytes).
    pub envelope_size_bytes_histogram: Histogram,
}
```

### 16.1 Label cardinality

* `envelopes_written_total` : labels = `{tenant_tier, status}` ; cardinality ~ 4 × 4 = 16.
* Histograms : no labels (global) to avoid cardinality explosion.

### 16.2 M0 dashboard panels

| Panel | Query |
|---|---|
| Envelopes per minute | `rate(envelopes_written_total[1m])` |
| P99 envelope latency | `histogram_quantile(0.99, envelope_latency_ms_histogram)` |
| Cost per envelope p95 | `histogram_quantile(0.95, envelope_total_cost_cents_histogram)` |
| Failed envelope writes | `increase(envelopes_write_failed_total[15m])` |
| Tombstoned envelopes today | `increase(envelope_tombstones_total[24h])` |

### 16.3 Alert rules

* `rate(envelopes_write_failed_total[5m]) > 0.01 * rate(envelopes_written_total[5m])` — 1 % audit-write failure rate.
* `histogram_quantile(0.99, envelope_size_bytes_histogram) > 100_000` — envelopes growing past 100 KB suggests `op_ids` truncation needed.
* `rate(envelope_tombstones_total[1h]) > 1000` — bulk-tombstone activity worth investigating.

---

## 17. Implementation plan — 5-day breakdown

### 17.1 Day 1 — SDK types

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano-sdk/src/types/envelope.rs` with `SwarmEnvelopePayload`, `OpStatusCounts`, `EnvelopeKind` + serde round-trip tests | Tier 1 GREEN |
| founder | Re-export from `types/mod.rs` + `lib.rs` | check re-exports |
| founder | Extend `NanoAgentTrace` with `swarm_envelope_trace_id: Option<TraceId>` + ensure canonical_row_body skips it | Tier 1 GREEN |
| founder | Extend `SwarmRequest` with `parent_envelope_trace_id: Option<TraceId>` (#serde default skip None) | round-trip test |
| founder | Extend `SwarmResponse` with `swarm_envelope_trace_id: Option<TraceId>` (#serde default skip None) | round-trip test |

### 17.2 Day 2 — Envelope module + writer

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `efe-nano/src/dispatcher/envelope.rs` with `EnvelopeBuilder` + `build_swarm_envelope_trace` + `write_envelope` + `TRACE_KIND_SWARM_ENVELOPE` constant | Tier 1 GREEN |
| founder | Reserved sentinel agent id constant `SWARM_ENVELOPE_AGENT_ID` + catalog publish-rejection logic | Tier 1 GREEN |
| founder | `request_hash` canonical-serializer (SHA-256 over canonical JSON) | hash determinism test |

### 17.3 Day 3 — Dispatcher integration + per-op linkage

| Owner | Deliverable | Verification |
|---|---|---|
| senior eng | Pre-allocate envelope id in `Dispatcher::dispatch` + build the builder | smoke test |
| senior eng | Thread `envelope_trace_id` through `execute_plan_with_extras → run_one_op → CallContext` | trace presence test |
| senior eng | Patch runtime modules (`pure_rust.rs`, `local_model.rs`, `llm_augmented.rs`) to read `ctx.swarm_envelope_trace_id` and set on the trace | unit tests |
| senior eng | Patch N12 `write_cache_hit_trace` + N13 `write_shadow_run_trace` / `write_shadow_comparison_trace` to set the field | integration tests |
| senior eng | Write envelope at swarm end + attach id to response | E2E test |

### 17.4 Day 4 — Replay engine + GDPR cascade

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `ReplayEngine::envelope_for_swarm` | query test |
| founder | Replay filter extended to skip SwarmEnvelope | filter test |
| founder | `ChainStore::tombstone_swarm_envelope` (trait default + InMemoryChainStore override) | cascade test |
| founder | Envelope queries indexed in side-store (optional Sprint 8) | dashboard panel test |

### 17.5 Day 5 — Integration tests + verification + 8 Prometheus metrics

| Owner | Deliverable | Verification |
|---|---|---|
| founder | `tests/phase_2_n14_swarm_envelope.rs` — 25+ tests | full GREEN |
| founder | Envelope size + counter + histogram metric surface | metric exposure |
| both | Final clippy strict + cargo test full suite | gate |

### 17.6 Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Pre-N14 chain rows break under new canonical body | low | Field excluded from canonical body |
| Envelope size explodes on mega-swarms | medium | `op_ids` capped + truncation flag (Sprint 9) |
| Tombstone race with in-flight ops | medium | Retry sweep (Sprint 9) ; document |
| Audit write fails after ops succeed | medium | Log error + Sprint 9 stale-envelope sweeper |
| Two dispatchers compute different `overall_status` for same outcomes | low | Single aggregator function shared between dispatch + replay |
| envelope's `nano_agent_id` collides with a real published agent | low | Catalog rejects publish of reserved sentinel id |

---

## 18. Test matrix

### 18.1 Tier 1 — SDK unit tests

| Test | Description |
|---|---|
| `envelope_payload_round_trip` | serde JSON round-trip on representative payload |
| `op_status_counts_default_is_zeros` | default sanity |
| `envelope_format_version_is_one` | constant check |
| `swarm_request_parent_envelope_serde_skip_when_none` | serialize-omits-None field |
| `swarm_response_envelope_id_serde_skip_when_none` | same |

### 18.2 Tier 2 — Builder unit tests

| Test | Description |
|---|---|
| `builder_captures_request_signature` | EnvelopeBuilder::new copies every request field |
| `builder_finalize_aggregates_outcomes` | `OpStatusCounts` correctly computed |
| `builder_request_hash_is_deterministic` | same request → same request_hash, 100 iterations |
| `builder_request_hash_changes_on_field_change` | bumping max_parallelism changes hash |

### 18.3 Tier 3 — Dispatcher integration tests

| Test | Description |
|---|---|
| `envelope_written_on_success` | dispatch a 3-op swarm ; envelope row appears in chain |
| `envelope_written_on_timeout` | timeout_ms = 1ms, op blocks ; envelope status = TimedOut |
| `envelope_written_on_planner_failure` | invalid DAG ; envelope status = Failed, op_count = 0 |
| `envelope_written_on_empty_swarm` | 0 ops ; envelope status = Success, op_count = 0 |
| `op_traces_carry_envelope_id` | every op trace from a 5-op swarm has `swarm_envelope_trace_id = Some(...)` |
| `cache_hit_trace_carries_envelope_id` | dispatch twice ; second swarm's CacheHit trace carries the SECOND swarm's envelope_id |
| `shadow_traces_carry_envelope_id` | shadow policy active ; ShadowRun + ShadowComparison both carry envelope_id |
| `response_carries_envelope_id` | response.swarm_envelope_trace_id is Some after dispatch |
| `envelope_overall_status_matches_response` | invariant I8 |
| `envelope_total_cost_matches_response` | invariant I9 |
| `envelope_op_status_counts_match_outcomes` | invariant I10 |
| `envelope_cache_hit_count_matches_traces` | aggregation accuracy |
| `envelope_shadow_run_count_matches_traces` | aggregation accuracy |

### 18.4 Tier 4 — Chain integrity tests

| Test | Description |
|---|---|
| `pre_n14_chain_still_verifies` | construct a pre-N14 chain (no envelope field), verify it still HMAC-validates |
| `mixed_pre_post_n14_chain_verifies` | mix old + new rows, verify chain as a whole |
| `envelope_included_in_merkle_epoch` | epoch sealer includes envelope row |

### 18.5 Tier 5 — Replay tests

| Test | Description |
|---|---|
| `envelope_for_swarm_returns_payload` | basic query |
| `envelope_for_swarm_not_found_for_unknown_id` | error path |
| `envelope_for_swarm_tombstoned_returns_tombstoned_error` | post-cascade |
| `replay_filters_swarm_envelope` | replay sees only real ops, not envelope |
| `replay_chain_length_increments_match_with_envelope` | replay produces N CacheHit + 1 envelope → chain grows correctly |

### 18.6 Tier 6 — GDPR cascade tests

| Test | Description |
|---|---|
| `tombstone_swarm_envelope_erases_all_op_traces` | envelope + 3 ops + 1 cache + 1 shadow = 6 tombstones |
| `tombstone_envelope_preserves_chain_hmac` | post-cascade verify_chain still passes |
| `tombstone_envelope_idempotent` | calling twice = no-op on second |
| `tombstone_non_envelope_is_error` | passing an op trace's id returns NotAnEnvelope |

### 18.7 Tier 7 — Stress + E2E

| Test | Description |
|---|---|
| `stress_100_swarms_envelopes_correct` | dispatch 100 swarms, 100 envelopes appear |
| `e2e_dispatch_query_envelope_round_trip` | dispatch + query + assert all fields |
| `e2e_canary_envelope_records_canary_routed_count` | N13 + N14 interaction |
| `e2e_meta_swarm_parent_envelope_lineage` | parent_envelope_trace_id propagates |

---

## 19. Migration path

### 19.1 Pre-N14 state

Chain rows have no `swarm_envelope_trace_id` field.  Chain HMAC verification operates on the canonical body shape (which doesn't include the new field).

### 19.2 N14 deploy

1. Code deployed.  Old chain rows unchanged — they verify under the same canonical body (the new field is excluded).
2. New swarms produce envelopes.  Op traces carry `swarm_envelope_trace_id`.
3. Forensic queries for old swarms : `envelope_for_swarm` returns `NotFound` ; manual reconstruction via per-op traces (pre-N14 fallback).
4. Forensic queries for new swarms : single envelope query.

### 19.3 Optional : backfill envelopes for old swarms

Sprint 9 may add a backfill job that synthesises envelopes for old swarms by aggregating their op traces.  The synthetic envelope has `envelope_format_version = 0` (marker for "synthesised") and `full_input.synthesised = true`.

Out of scope for Sprint 8.

### 19.4 Canonical body promotion (Sprint 9)

Sprint 9 may promote `swarm_envelope_trace_id` to canonical via `body_version: u16` field.  Migration plan :

1. Sprint 9 N+1 : deploy code that supports BOTH body_version=1 (excludes) and body_version=2 (includes).
2. All new rows written as body_version=2.
3. Verifier handles both shapes.
4. Sprint 10 N+2 : optional rewrite of old rows to body_version=2 with re-HMAC (requires tenant audit key access).

Backward compat preserved indefinitely : verifier always supports body_version=1.

---

## 20. Open questions / ADRs

### ADR-N14-001 : Field excluded from canonical_row_body

**Decision** : `swarm_envelope_trace_id` is operational metadata, excluded from HMAC payload.

**Rationale** : preserves Sprint 6 chain verification on pre-N14 rows.  Trade-off : field is mutable by chain-admin (no cryptographic protection).  Mitigation : Sprint 9 promotes to canonical via body_version.

### ADR-N14-002 : Single envelope per swarm (no Open / Close pair)

**Decision** : write ONE envelope row at swarm completion (or failure).

**Alternatives** : two rows (Open at start, Close at end) ; per-update streaming rows.

**Rationale** : minimises chain volume.  Sprint 9 streaming is forward-compatible via `EnvelopeKind` discriminator.

### ADR-N14-003 : Envelope is a NanoAgentTrace, not a separate type

**Decision** : reuse the existing `NanoAgentTrace` struct with `error_kind = Some("SwarmEnvelope")` sentinel.

**Alternatives** : separate `SwarmEnvelopeTrace` struct stored in a parallel table.

**Rationale** : unified chain table simplifies HMAC chaining and Merkle epoch construction.  Sentinel pattern proven by N12 / N13.

### ADR-N14-004 : Reserved agent id `swarm_envelope`

**Decision** : reserve the agent id `swarm_envelope` from catalog publishes.

**Alternatives** : use a UUID-encoded id ; use a `None` for `nano_agent_id` (requires schema change).

**Rationale** : human-readable + simple to enforce in catalog (add to a denylist).

### ADR-N14-005 : Tombstone cascade direction

**Decision** : `tombstone_swarm_envelope(envelope_id)` cascades to every op trace of the same swarm_request_id.  The reverse (tombstone an op cascades to envelope) is NOT done — operators tombstone the envelope when they want the whole swarm gone.

**Rationale** : symmetry with N13 (parent → children) ; clear API.

### ADR-N14-006 : request_hash over canonical request JSON

**Decision** : `request_hash = SHA-256(canonical_json(request))` where canonical_json sorts object keys recursively.

**Rationale** : matches the cache-key canonicalization (N12) for consistency.  Verifiable by readers.

### ADR-N14-007 : `parent_envelope_trace_id` opt-in field

**Decision** : `SwarmRequest.parent_envelope_trace_id` is `Option<TraceId>`, caller-set.  Sprint 8 doesn't validate the reference.

**Rationale** : forward-compat for Sprint 9 N15 meta-swarms.  Caller-side wiring deferred.

### ADR-N14-008 : envelope_format_version

**Decision** : `envelope_format_version: u16 = 1` for Sprint 8.  Sprint 9+ increments for schema changes.

**Rationale** : explicit versioning avoids "guess from field presence" deserialization.

---

## 21. Annexe A — full type definitions

```rust
// crates/efe-nano-sdk/src/types/envelope.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::types::{SwarmStatus, TenantId};
use crate::traits::TraceId;

/// **Sprint 8 — N14** : the body of the swarm envelope trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmEnvelopePayload {
    // ─── request signature ───
    pub swarm_request_id: Uuid,
    pub tenant_id: TenantId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caller_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub audit_tags: Vec<String>,
    pub max_parallelism: u32,
    pub timeout_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cost_cap_cents: Option<f64>,
    #[serde(default)]
    pub streaming: bool,
    pub op_count: u32,
    pub op_ids: Vec<String>,
    #[serde(with = "hex_array")]
    pub request_hash: [u8; 32],
    pub started_at: DateTime<Utc>,

    // ─── aggregated outcome ───
    pub overall_status: SwarmStatus,
    pub total_cost_cents: f64,
    pub total_latency_ms: u32,
    pub completed_at: DateTime<Utc>,
    pub op_status_counts: OpStatusCounts,
    pub cache_hit_count: u32,
    pub shadow_run_count: u32,
    pub shadow_mismatch_count: u32,
    pub canary_routed_count: u32,

    // ─── lineage ───
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_envelope_trace_id: Option<TraceId>,

    // ─── versioning ───
    #[serde(default = "default_envelope_format_version")]
    pub envelope_format_version: u16,

    // ─── streaming hooks (Sprint 9 ; default Terminal for now) ───
    #[serde(default)]
    pub envelope_kind: EnvelopeKind,
}

fn default_envelope_format_version() -> u16 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpStatusCounts {
    pub success: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cancelled: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// Sprint 8 — one envelope per swarm, written at completion.
    Terminal,
    /// Sprint 9 — streaming update.
    Progress { update_seq: u32 },
    /// Sprint 9 — streaming closing envelope.
    Final { total_updates: u32 },
}

impl Default for EnvelopeKind {
    fn default() -> Self {
        Self::Terminal
    }
}

mod hex_array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(b: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        h.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex_decode(&s).map_err(serde::de::Error::custom)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
    fn hex_decode(s: &str) -> Result<Vec<u8>, &'static str> {
        if s.len() != 64 { return Err("expected 64 hex chars"); }
        (0..32).map(|i| u8::from_str_radix(&s[2*i..2*i+2], 16)
                          .map_err(|_| "invalid hex")).collect()
    }
}
```

```rust
// efe-nano-sdk/src/types/swarm.rs — extended

pub struct SwarmRequest {
    pub request_id: Uuid,
    pub tenant_id: TenantId,
    pub caller_agent_id: Option<String>,
    pub operations: Vec<Operation>,
    pub max_parallelism: u32,
    pub timeout_ms: u32,
    pub idempotency_key: Option<String>,
    pub streaming: bool,
    pub audit_tags: Vec<String>,
    pub cost_cap_cents: Option<f64>,
    /// **N14** : optional reference to the parent envelope when this
    /// swarm is a child of a meta-swarm.  Sprint 8 : opt-in, caller-set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_envelope_trace_id: Option<TraceId>,
}

pub struct SwarmResponse {
    pub request_id: Uuid,
    pub overall_status: SwarmStatus,
    pub results: HashMap<String, OperationResult>,
    pub total_latency_ms: u32,
    pub total_cost_cents: f64,
    /// **N14** : the envelope trace id, set by the dispatcher.  Allows
    /// the caller to query their audit log.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub swarm_envelope_trace_id: Option<TraceId>,
}
```

```rust
// efe-nano/src/audit/mod.rs — extended

pub struct NanoAgentTrace {
    pub trace_id: TraceId,
    pub parent_trace_ids: Vec<TraceId>,
    pub swarm_request_id: Uuid,
    pub operation_id: String,
    pub tenant_id: TenantId,
    pub caller_agent_id: Option<String>,
    pub nano_agent_id: NanoAgentId,
    pub nano_agent_version: NanoAgentVersion,
    pub execution_mode: String,
    pub input_hash_source: [u8; 32],
    pub input_hash_resolved: [u8; 32],
    pub output_hash: Option<[u8; 32]>,
    pub error_kind: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u32>,
    pub cost_cents: f64,
    pub llm_tokens_used: Option<u32>,
    pub full_input: Option<serde_json::Value>,
    pub full_output: Option<serde_json::Value>,
    pub tombstone: Option<TombstoneMetadata>,
    /// **Sprint 8 — N14** : pointer to the swarm's envelope trace.
    /// `None` for the envelope itself, for `RolloutPolicyChanged`
    /// virtual rows, and for any trace not bound to a swarm.
    ///
    /// **NOT included in `canonical_row_body`** (operational metadata,
    /// like `tombstone`).  Adding the field preserves Sprint 6 HMAC
    /// chain verification.
    pub swarm_envelope_trace_id: Option<TraceId>,
}
```

```rust
// efe-nano/src/dispatcher/envelope.rs (new module)

use chrono::{DateTime, Utc};
use efe_nano_sdk::{
    NanoAgentId, NanoAgentVersion, OpStatusCounts, SwarmEnvelopePayload, SwarmRequest,
    SwarmResponse, SwarmStatus, TenantId, TraceId,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::audit::NanoAgentTrace;
use crate::dispatcher::types::OperationOutcome;

pub const TRACE_KIND_SWARM_ENVELOPE: &str = "SwarmEnvelope";
pub const SWARM_ENVELOPE_AGENT_ID: &str = "swarm_envelope";
pub const SWARM_ENVELOPE_VERSION: &str = "0.0.0";
pub const SWARM_ENVELOPE_EXECUTION_MODE: &str = "swarm_envelope";

pub struct EnvelopeBuilder {
    envelope_trace_id: TraceId,
    swarm_request_id: Uuid,
    tenant_id: TenantId,
    started_at: DateTime<Utc>,
    started_instant: Instant,
    caller_agent_id: Option<String>,
    idempotency_key: Option<String>,
    audit_tags: Vec<String>,
    max_parallelism: u32,
    timeout_ms: u32,
    cost_cap_cents: Option<f64>,
    streaming: bool,
    op_count: u32,
    op_ids: Vec<String>,
    request_hash: [u8; 32],
    parent_envelope_trace_id: Option<TraceId>,
}

impl EnvelopeBuilder {
    pub fn new(envelope_trace_id: TraceId, request: &SwarmRequest) -> Self;
    pub fn envelope_trace_id(&self) -> TraceId;
    pub fn finalize(
        self,
        response: &SwarmResponse,
        outcomes: &HashMap<String, OperationOutcome>,
    ) -> SwarmEnvelopePayload;
}

pub fn build_swarm_envelope_trace(payload: &SwarmEnvelopePayload) -> NanoAgentTrace;
pub async fn write_envelope(
    audit: &dyn crate::audit::AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
) -> TraceId;
pub fn compute_request_hash(request: &SwarmRequest) -> [u8; 32];
```

---

## 22. Annexe B — Postgres schema (Phase 4)

```sql
-- Extends the chain_entry table with the new field.
ALTER TABLE chain_entry
    ADD COLUMN swarm_envelope_trace_id UUID NULL;

CREATE INDEX idx_chain_entry_envelope_id
    ON chain_entry(swarm_envelope_trace_id)
    WHERE swarm_envelope_trace_id IS NOT NULL;

-- A projection table for fast envelope queries (Sprint 9 deliverable).
CREATE TABLE swarm_envelope (
    envelope_trace_id  UUID PRIMARY KEY,
    tenant_id          UUID NOT NULL,
    swarm_request_id   UUID NOT NULL,
    started_at         TIMESTAMPTZ NOT NULL,
    completed_at       TIMESTAMPTZ NOT NULL,
    overall_status     TEXT NOT NULL,
    total_cost_cents   NUMERIC(12, 6) NOT NULL,
    total_latency_ms   INT NOT NULL,
    op_count           INT NOT NULL,
    cache_hit_count    INT NOT NULL,
    shadow_run_count   INT NOT NULL,
    payload_jsonb      JSONB NOT NULL,        -- full SwarmEnvelopePayload
    tombstoned_at      TIMESTAMPTZ NULL,
    FOREIGN KEY (tenant_id) REFERENCES tenant(tenant_id) ON DELETE CASCADE
);

CREATE INDEX idx_swarm_envelope_tenant_completed
    ON swarm_envelope(tenant_id, completed_at DESC);

CREATE INDEX idx_swarm_envelope_request_id
    ON swarm_envelope(tenant_id, swarm_request_id);

CREATE INDEX idx_swarm_envelope_status_completed
    ON swarm_envelope(tenant_id, overall_status, completed_at DESC);
```

---

## 23. Annexe C — sample envelope JSON

```json
{
  "kind": "swarm_envelope",
  "trace_id": "f9a2b1c7-0d2e-4abc-9def-0123456789ab",
  "error_kind": "SwarmEnvelope",
  "swarm_request_id": "1a2b3c4d-5e6f-7890-abcd-ef0123456789",
  "tenant_id": "00000000-0000-0000-0000-000000000001",
  "nano_agent_id": "swarm_envelope",
  "nano_agent_version": "0.0.0",
  "execution_mode": "swarm_envelope",
  "operation_id": "swarm_envelope",
  "started_at": "2026-05-20T14:32:01.234Z",
  "completed_at": "2026-05-20T14:32:03.567Z",
  "latency_ms": 2333,
  "cost_cents": 0.0156,
  "input_hash_source": "8a9b...",
  "input_hash_resolved": "8a9b...",
  "output_hash": "cd0e...",
  "full_input": {
    "swarm_request_id": "1a2b3c4d-5e6f-7890-abcd-ef0123456789",
    "tenant_id": "00000000-0000-0000-0000-000000000001",
    "caller_agent_id": "claude-sonnet-4-5",
    "idempotency_key": "user-job-2026-05-20-14-32",
    "audit_tags": ["billing-run", "customer-tier-2"],
    "max_parallelism": 4,
    "timeout_ms": 30000,
    "cost_cap_cents": 1.0,
    "streaming": false,
    "op_count": 3,
    "op_ids": ["validate", "extract", "normalize"],
    "request_hash": "8a9b2c...",
    "started_at": "2026-05-20T14:32:01.234Z",
    "overall_status": "success",
    "total_cost_cents": 0.0156,
    "total_latency_ms": 2333,
    "completed_at": "2026-05-20T14:32:03.567Z",
    "op_status_counts": {
      "success": 3,
      "failed": 0,
      "skipped": 0,
      "cancelled": 0
    },
    "cache_hit_count": 1,
    "shadow_run_count": 0,
    "shadow_mismatch_count": 0,
    "canary_routed_count": 0,
    "envelope_format_version": 1,
    "envelope_kind": { "kind": "terminal" }
  },
  "full_output": { ... mirrors the outcome section ... }
}
```

---

## 24. Annexe D — forensic query playbook

### Scenario 1 — "Find all swarms tenant T ran with cost-cap-fired"

```sql
SELECT envelope_trace_id, swarm_request_id, started_at, total_cost_cents
FROM swarm_envelope
WHERE tenant_id = $TENANT
  AND payload_jsonb->>'cost_cap_cents' IS NOT NULL
  AND total_cost_cents >= (payload_jsonb->'cost_cap_cents')::numeric * 0.95
ORDER BY started_at DESC
LIMIT 100;
```

### Scenario 2 — "Find swarms where canary served >25 % of traffic"

```sql
SELECT envelope_trace_id, swarm_request_id, op_count,
       (payload_jsonb->>'canary_routed_count')::int AS canary_count
FROM swarm_envelope
WHERE tenant_id = $TENANT
  AND op_count > 0
  AND (payload_jsonb->>'canary_routed_count')::numeric / op_count > 0.25
ORDER BY started_at DESC;
```

### Scenario 3 — "GDPR : tombstone all swarms tenant T ran on October 5"

```sql
-- Find envelope_trace_ids
SELECT envelope_trace_id FROM swarm_envelope
WHERE tenant_id = $TENANT
  AND completed_at >= '2026-10-05'::date
  AND completed_at < '2026-10-06'::date;

-- For each, GDPR saga calls :
-- chain_store.tombstone_swarm_envelope(tenant, envelope_trace_id, meta).await
```

### Scenario 4 — "Anomaly : envelope cost > sum(op cost) by > 5 %"

```sql
WITH envelope_costs AS (
    SELECT swarm_envelope_trace_id, SUM(cost_cents) AS op_sum
    FROM chain_entry
    WHERE swarm_envelope_trace_id IS NOT NULL
      AND error_kind IS NULL
    GROUP BY swarm_envelope_trace_id
),
envelope_totals AS (
    SELECT envelope_trace_id, total_cost_cents
    FROM swarm_envelope
)
SELECT e.envelope_trace_id, e.total_cost_cents, ec.op_sum,
       (e.total_cost_cents - ec.op_sum) / ec.op_sum AS drift_ratio
FROM envelope_totals e
JOIN envelope_costs ec ON ec.swarm_envelope_trace_id = e.envelope_trace_id
WHERE ABS((e.total_cost_cents - ec.op_sum) / ec.op_sum) > 0.05;
```

If this query returns rows, a billing-reconciliation bug exists.  Sprint 8 N14 invariant I9 forbids it.

### Scenario 5 — "Replay a tombstoned swarm — graceful failure"

```rust
let envelope_result = engine.envelope_for_swarm(tenant, swarm_id).await;
match envelope_result {
    Ok(payload) if payload.tombstone.is_some() => {
        // Envelope tombstoned ; replay impossible.
        return ReplayReport::envelope_tombstoned(swarm_id);
    }
    Ok(payload) => { /* proceed with replay */ }
    Err(ReplayError::EnvelopeNotFound(_)) => {
        // Pre-N14 swarm ; fall back to traces-only reconstruction.
    }
    Err(e) => return Err(e),
}
```

### Scenario 6 — "Meta-swarm explosion : find all child envelopes of meta M"

```sql
SELECT envelope_trace_id, swarm_request_id, overall_status, total_cost_cents
FROM swarm_envelope
WHERE tenant_id = $TENANT
  AND (payload_jsonb->>'parent_envelope_trace_id')::uuid = $META_ENVELOPE_ID
ORDER BY started_at;
```

---

**End of EFE M50 — N14 Swarm Envelope Trace Architecture (Sprint 8).**
