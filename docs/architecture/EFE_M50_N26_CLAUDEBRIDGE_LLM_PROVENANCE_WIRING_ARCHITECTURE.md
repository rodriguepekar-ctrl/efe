# EFE M50 — N26 ClaudeBridge ↔ LlmProvenanceRecord Wiring Architecture

> **Sprint**          : 18 (Phase 9, day 86 – 90 of M50).
> **Status**          : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md`              (Sprint 9, shipped)
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md`           (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md`        (Sprint 11, shipped)
> * `EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md` (Sprint 13, shipped)
> * `EFE_M50_N22_HTTP_GATEWAY_EXPOSURE_ARCHITECTURE.md`        (Sprint 14, shipped)
> * `EFE_M50_N23_NANO_SWARM_ADAPTER_ARCHITECTURE.md`           (Sprint 15, shipped)
> * `EFE_M50_N24_WIRED_COMPLIANCE_RUNNER_ARCHITECTURE.md`      (Sprint 16, shipped)
>
> **Anchors** : N17 (Sprint 9) shipped the full LLM-provenance
> machinery — `LlmProvenanceRecord` data type, `LlmProvenanceCollector`
> trait, `DefaultLlmProvenanceCollector` impl, `call_with_provenance`
> glue helper, in-memory + (future) Postgres-backed stores, and the
> hashed query surface for replay (N16) and audit.  N21 (Sprint 13)
> shipped `ClaudeBridge` that wraps Anthropic's Messages API with
> a tool-use bundle protocol.  **The two never met.**  Every Claude
> dispatch since Sprint 13 has left ZERO forensic trace : no
> `LlmProvenanceRecord` rows in the store, no audit replay anchor,
> no cost reconciliation against the N17 hashed-prompt index.  The
> EU AI Act auditor who asked "show me every LLM call made by
> tenant X on date Y" gets an empty answer.
>
> **N26 fixes it.**  Sprint 18 wires the existing
> `LlmProvenanceCollector` into `ClaudeBridge` via the
> `call_with_provenance` glue.  Every two-turn Claude dispatch
> produces exactly one `LlmProvenanceRecord` row.  An additive
> `AgenticAuditContext` field on `AgenticTask` carries the
> caller-side provenance context (tenant + agent + template +
> publisher + gold-case) through to the bridge.  Backward-compat
> preserved : `AgenticTask` without `audit_context` continues to
> dispatch as before — without provenance emission, with a one-
> line warning log.

---

## Table of contents

0.  [TL;DR](#0-tldr)
1.  [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2.  [Twelve incident patterns N26 closes (P1–P12)](#2-twelve-incident-patterns-n26-closes-p1p12)
3.  [Commitments — what ships in Sprint 18 (G1–G15)](#3-commitments--what-ships-in-sprint-18-g1g15)
4.  [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5.  [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6.  [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7.  [Component architecture](#7-component-architecture)
8.  [Module hierarchy & data model](#8-module-hierarchy--data-model)
9.  [API surface (Rust traits + HTTP)](#9-api-surface-rust-traits--http)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N24)](#13-cross-cutting-alignment-n11--n24)
14. [Sprint 18 ship vs Sprint 19+ split](#14-sprint-18-ship-vs-sprint-19-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The forensic gap N26 closes.**

The EU AI Act §13 (Sprint-9 N17 architecture annex A1) requires :

> *"For every interaction with a high-risk LLM, the deployer SHALL
> record (a) the model identity + content hash, (b) the system +
> user prompts hashed, (c) token usage + cost accounting, (d) the
> provider response identifiers, (e) safety + completeness flags,
> (f) the publisher identity at dispatch time, all linked to the
> calling tenant and agent ; the record SHALL be retrievable for
> at least 5 years."*

N17 shipped (Sprint 9) the **shape** : the `LlmProvenanceRecord`
data type, the `LlmProvenanceCollector` trait + default impl, the
store, the query surface.  The expectation was that every
Mode-C nano agent + every direct LLM bridge would call
`call_with_provenance(...)` around their LLM dispatch.

N21 (Sprint 13) shipped **`ClaudeBridge`** for the agentic LLM
path.  It calls Anthropic's `/v1/messages` endpoint twice (tool-
use turn + tool_result turn).  But it does NOT use
`call_with_provenance`.  No `LlmProvenanceRecord` is ever emitted
on this path.

| What happens today | What should happen |
|---|---|
| `ClaudeBridge::call_with_context(bundle, task) → AgenticResponse` | Same surface, but produces a `LlmProvenanceRecord` as a side-effect |
| The audit chain has zero N17 rows for any Claude call | Every Claude dispatch produces exactly ONE row |
| The replay subsystem (N16) can't replay a Claude swarm | N16 replay reads N17 rows + re-dispatches |
| Cost reconciliation (FinOps SLO) is approximate (in-memory swarm cost) | Finance joins N17 rows by tenant for billing-grade reconciliation |
| The EU compliance officer asking "show me LLM calls for tenant X on date Y" gets an empty answer | Same query returns the full forensic chain |

### N26 ships five things

1.  **`AgenticAuditContext`** — additive optional field on
    `AgenticTask` carrying the pre-call provenance metadata
    (tenant_id, agent_id, agent_version, prompt_template_id +
    version, publisher_id + kyc_tier, gold_case_id, swarm_request_id,
    op_id, trace_id).  All callers that produce `AgenticTask` —
    the gateway's `agentic_call_handler`, the swarm's `summary-
    writer` Mode-C agent, the future MCP layer — populate it.

2.  **`ClaudeBridge::with_provenance(collector)`** — builder method
    that wires an `Arc<dyn LlmProvenanceCollector>` into the
    bridge.  When `Some`, every `call_with_context` invocation
    drives `call_with_provenance` ; when `None`, the bridge keeps
    its Sprint 13 behaviour (raw dispatch, no provenance) with a
    one-line warning log.

3.  **Hash + canonical-body computation inside ClaudeBridge** :
    the bridge computes `system_prompt_hash`, `user_prompt_hash`,
    `final_prompt_hash`, `prompt_inputs_hash`, `output_hash`
    using `efe_llm_provenance::hash::prompt_hashes_for`.  The
    `provider_request_id` is captured from the Anthropic response
    `id` field.

4.  **Two-turn collapse into one record** : the two Anthropic HTTP
    calls (turn 1 tool-use ask + turn 2 tool_result) produce ONE
    logical `LlmProvenanceRecord` with `provider_response_time_ms`
    = wall-clock sum, `token_usage` = sum of both turns,
    `tool_calls` carrying the recorded `tool_use` block from
    turn 1.

5.  **Gateway + smoke-binary + handler wiring** : the gateway's
    `AppState` gains
    `llm_provenance: Option<Arc<dyn LlmProvenanceCollector>>` ;
    the `agentic_call_handler` populates `AgenticAuditContext` ;
    the smoke binary `efe_claude_smoke` does the same.  A new
    `GET /v1/llm-provenance` operator-scoped endpoint exposes a
    paginated tenant-scoped view of recent records.

### What is NOT in scope (Sprint 19+)

- **Streaming SSE provenance** : Sprint 19 N27 wires the
  provider streaming token-event log into the record's
  `tool_calls` extension.
- **Postgres-backed `LlmProvenanceStore`** : Sprint 20 N28
  ships the durable store ; Sprint 18 uses the in-memory store.
- **N28 event-bus emission** : Sprint 20 publishes every
  provenance record on the workspace event bus for downstream
  dashboards.
- **Cross-bridge canonicalisation** : Sprint 22+ ships
  `OpenAIBridge`, `BedrockBridge`, etc., all reusing the same
  `LlmProvenanceCollector` injection pattern (the trait is
  bridge-agnostic).

### Why this matters NOW

Sprint 16 N24 closed the **catalogue-side** compliance gap : the
adapter dispatches the six built-in pre-analysis agents under
signature + gold + status gates and emits a
`nano_envelope_v1` event per dispatch.  But the agentic LLM
side — every `POST /v1/agentic/call` — still flies blind.
Sprint 18 N26 closes the **LLM-side** compliance gap so the EU
AI Act sign-off for Q3 2026 has both halves of the trail.

---

## 1. Context, motivation, sprint placement

### 1.1 Where N26 lands in the M50 timeline

```
Sprint 8  N11..N15  Firecracker pool / cache hardening / shadow-canary / envelope / fairness
Sprint 9  N16,N17   Multi-region replay / LLM provenance        ← N17 shipped (data + trait)
Sprint 10 N18       Publisher identity (Ed25519 + KYC + sanctions)
Sprint 11 N19       Gold-test gate Tier-1 promotion
Sprint 12 N20       Postgres real embeddings
Sprint 13 N21       Swarm orchestration + Agentic Bridge        ← ClaudeBridge shipped (no N17 wiring)
Sprint 14 N22       HTTP gateway exposure
Sprint 15 N23       NanoAgent ↔ SwarmAgent adapter
Sprint 16 N24       Wired compliance runner (sig + gold + status)
Sprint 17 N25       Multi-region nano federation
Sprint 18 N26  <<<  THIS — ClaudeBridge LLM provenance wiring
Sprint 19 N27       Streaming SSE provenance
Sprint 20 N28       Postgres LlmProvenanceStore + event bus
```

N26 is the **forensic-trail closure** sprint for the agentic
LLM path.  After Sprint 18, the EU AI Act §13 query
"show me every Claude call for tenant X on date Y" returns a
complete result — every record carries the publisher identity
(N18 anchor), the gold-case id (N19 anchor) if any, the
canonical prompt hashes (N16 replay anchor), and the cost
breakdown (FinOps anchor).

### 1.2 What N17 already shipped (recap)

| Component | Status | Sprint |
|-----------|--------|--------|
| `LlmProvenanceRecord` data type (44 fields, serde-stable) | ✅ shipped | 9 |
| `LlmProvenanceCollector` trait | ✅ shipped | 9 |
| `DefaultLlmProvenanceCollector` impl (writes to store + emits N17Metrics) | ✅ shipped | 9 |
| `call_with_provenance(collector, pre, hash, closure)` glue | ✅ shipped | 9 |
| `PreCallProvenance` + `PostCallProvenance` builders | ✅ shipped | 9 |
| `InMemoryLlmProvenanceStore` | ✅ shipped | 9 |
| `prompt_hashes_for` + BLAKE3 helpers | ✅ shipped | 9 |
| `ModelRegistry` trait + hash verification | ✅ shipped | 9 |
| `N17Metrics` (12 Prometheus surfaces) | ✅ shipped | 9 |
| `LlmProvenanceQueryApi` (5 read endpoints) | ✅ shipped | 9 |
| Cross-cutting `publisher_id` + `publisher_kyc_tier` snapshot | ✅ shipped | 10 (N18 follow-up) |
| Cross-cutting `gold_case_id` propagation | ✅ shipped | 11 (N19 follow-up) |

N26 adds **zero** new types to `efe-llm-provenance`.  The work
is **integration only** — plug existing pieces into existing
pieces, plus one additive struct on the `efe-agentic-bridge`
side (`AgenticAuditContext`).

### 1.3 What ClaudeBridge looks like today (recap)

```rust
// efe-agentic-bridge/src/claude.rs (Sprint 13 N21)
pub struct ClaudeBridge {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    anthropic_version: String,
    call_count: AtomicU64,
}

#[async_trait]
impl AgenticBridge for ClaudeBridge {
    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        // ... two-turn dispatch ...
    }
}
```

There is no `Arc<dyn LlmProvenanceCollector>` field.  There is no
`AgenticAuditContext` field on `AgenticTask`.  The bridge has no
visibility into the caller's tenant / agent / template metadata.

### 1.4 Stakeholders

| Role                       | Concern                                  | Mitigation                              |
|----------------------------|------------------------------------------|-----------------------------------------|
| EU AI Act auditor          | "Show me every LLM call for tenant X on date Y" | G1 + G3 + G13 ; admin endpoint                |
| FinOps                     | "Reconcile Anthropic invoice against the record store" | G5 ; cost breakdown per record                |
| Replay subsystem (N16)     | "Re-dispatch a 30-day-old Claude swarm bit-identically" | G4 + I7 ; prompt + output hashes pinned       |
| Customer (DPO)             | "Are PII redactions logged ?"            | G7 ; `pii_strategy_applied` + `redaction_token_set` |
| Customer (compliance)      | "Show me the publisher who authored the agent that made this call" | G8 ; `publisher_id` + `kyc_tier` snapshot     |
| Platform SRE               | "If the provenance store is slow, does the bridge hang ?" | F10 + I6 ; non-blocking emission              |
| Developer onboarding       | "Where do I plug a new LLM bridge ?"     | ADR-2 ; reusable injection pattern            |

### 1.5 Why a separate sprint (not a bug fix)

Three deliberate scope decisions justify a full sprint :

1.  **Trait surface change** — `AgenticTask` gains a new optional
    field ; all callers update.  Breaking changes are sprint
    deliverables, not bug fixes.
2.  **Cross-cutting plumbing** — gateway `AppState` extension,
    handler population, smoke binary update, OpenAPI spec.
3.  **Test surface** — 25+ acceptance tests, 8+ audit
    regressions, 10+ chaos tests, plus a live smoke that
    verifies a real Anthropic call produces a real record.

---

## 2. Twelve incident patterns N26 closes (P1–P12)

### P1 — EU audit query returned 0 rows

A Q2 2026 compliance dry-run for tenant Acme asked "list every
LLM call between 2026-04-01 and 2026-04-30".  The N17 store had
**zero** rows for Acme even though tooling logs showed > 12,000
Claude dispatches.  Auditor flagged it as a §13 violation.
**Fix** : G3 ; every Claude call produces exactly one record.

### P2 — Cost reconciliation off by > 30 %

Finance's monthly close compared the Anthropic invoice to the
swarm-side `total_cost_microcents` aggregate.  Variance was
+34 % — the swarm side under-counted because cached prompts
(N12) had zero `cost_microcents` but the Anthropic side still
billed for prompt-cache writes.  **Fix** : G5 ; record carries
the FULL `CostAccounting` (input + output + provider
microcents) so finance reconciles per-record, not per-swarm.

### P3 — Replay failed silently on a 28-day-old swarm

An N16 replay of swarm `s-2026-04-12-...` failed because the
provenance store had no entry for the Claude turn — the
replay subsystem had nothing to feed `call_with_provenance`
against.  **Fix** : G4 + I7 ; replay reads the N17 row to
reproduce the canonical prompt bytes + verify the model
identity matches.

### P4 — Publisher revoked but old LLM calls untraceable

A publisher was revoked (Sprint 10 N18 cascade).  The compliance
officer asked "show me every LLM call this publisher's agents
made between when their key was last rotated and revocation".
N17 had no rows ⇒ no answer.  **Fix** : G8 ; `publisher_id` +
`publisher_kyc_tier` are pinned on every record (the snapshot
captures the publisher state AT call time, not at query time).

### P5 — Gold-case-driven LLM call not attributable

A gold-test run (N19) failed on case `gold-case-xyz`.  The
forensic question : "what exactly did Claude return on that
case ?"  Without a record carrying `gold_case_id`, the answer
required scraping logs.  **Fix** : G9 ; `gold_case_id` flows
from the gold runner through the audit context to the record.

### P6 — Streaming change broke the protocol two-turn shape

A planned Sprint 19 N27 streaming change would split the two-
turn dispatch into smaller events.  Without a logical
"call record" anchor, every event needed its own row, blowing
up the store.  **Fix** : G6 + ADR-3 ; the record is the
**logical call**, not the HTTP turn.  Sprint 19 events extend
the same record via a new `tool_calls` entry pattern.

### P7 — PII redaction trail lost between PiiRedactor + Claude

The `PiiRedactor` (N21 preanalysis agent) replaces PII tokens.
The compliance officer asked "what tokens were redacted before
this Claude call ?"  Sprint 13 had no answer.  **Fix** : G7 ;
the bundle carries `redactions[]` and the record copies the
`redaction_token_set` from the bundle.

### P8 — A misconfigured collector silently dropped records

In a staging deployment the N17 collector pointed at a
non-existent backend.  The collector swallowed errors ; the
bridge dispatched anyway.  Records were lost.  **Fix** : ADR-5
+ I8 ; collector errors are LOGGED at WARN with the trace_id ;
the bridge surfaces a metric (`n26_provenance_emit_failed_total`)
so SREs see the silent drop.

### P9 — Provenance store back-pressure stalled the bridge

A slow store (e.g., Postgres under a long write transaction)
held the bridge call open for > 30 s after Claude returned.
Customer-visible latency exploded.  **Fix** : F10 + I6 + ADR-6 ;
the post-call emission is wrapped in
`tokio::time::timeout(500 ms)` ; timeout → log + metric ; the
bridge returns to the caller regardless.  Sprint 20 N28 ships
async-fire-and-forget via the event bus.

### P10 — Two-turn race recorded duplicate rows

A hand-rolled instrumentation attempt in Sprint 14 emitted one
row per turn — duplicates with the same trace_id confused the
audit query.  **Fix** : I3 ; the architecture's single-record-
per-logical-call discipline ; the helper invariant guarantees
exactly one `post_call` per `call_with_context`.

### P11 — `AgenticTask` audit context could be forged by the client

If the client supplies `audit_context.tenant_id`, a malicious
client could overwrite their own tenant id and impersonate
another tenant in the audit chain.  **Fix** : G11 + I12 ; the
gateway's `agentic_call_handler` OVERWRITES the
`audit_context.tenant_id` with the JWT claim's tenant_id ;
client-supplied value is ignored and a metric ticks.

### P12 — Smoke binary used the bridge without provenance

The `efe_claude_smoke` binary (Sprint 13) called the bridge
directly to validate "the bridge works against a live API key".
Live tests in CI did not produce N17 records, so the live
canary didn't verify the wiring.  **Fix** : G15 + Workflow C ;
the smoke binary wires a `DefaultLlmProvenanceCollector` and
asserts the record is written before exit.

---

## 3. Commitments — what ships in Sprint 18 (G1–G15)

### G1 — `AgenticAuditContext` struct on `AgenticTask`

```rust
// efe-agentic-bridge/src/task.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticAuditContext {
    pub trace_id: Uuid,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub tenant_id: TenantId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub pii_strategy_applied: PiiStrategy,
    pub redaction_token_set: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_region: Option<RegionCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<PublisherId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_kyc_tier: Option<KycTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_case_id: Option<GoldCaseId>,
}

pub struct AgenticTask {
    pub user_message: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// **N26** : optional provenance context.  `None` for backward
    /// compatibility ; `Some` enables full N17 emission.
    pub audit_context: Option<AgenticAuditContext>,
}
```

Existing callers compile unchanged because `audit_context`
defaults to `None`.

### G2 — `ClaudeBridge::with_provenance(collector)` builder

```rust
impl ClaudeBridge {
    pub fn with_provenance(
        mut self,
        collector: Arc<dyn LlmProvenanceCollector>,
    ) -> Self {
        self.provenance = Some(collector);
        self
    }
}
```

The collector is held as `Option<Arc<dyn LlmProvenanceCollector>>`.
When `None`, the bridge skips emission with a single WARN log.

### G3 — Every Claude dispatch produces exactly one record

When `audit_context.is_some() && self.provenance.is_some()`, the
bridge wraps the two-turn dispatch in `call_with_provenance(...)`
producing one logical record.  When either is `None`, no record
is produced and a metric ticks
(`n26_provenance_skipped_total{reason}` :
`no_audit_context` | `no_collector`).

### G4 — Replay-ready hashes pinned

`system_prompt_hash`, `user_prompt_hash`, `final_prompt_hash`,
`prompt_inputs_hash` (the canonical bundle hash) are all computed
inside the bridge via `efe_llm_provenance::hash::prompt_hashes_for`
+ `hash_bytes(bundle_json)`.  `output_hash` is computed from the
final assistant text.  All four are BLAKE3 and stable across
runs given the same inputs (I7).

### G5 — Full cost breakdown in `CostAccounting`

The bridge translates Anthropic's `usage` into N17's
`CostAccounting` struct :

```rust
let cost_accounting = CostAccounting {
    input_microcents: per_input_token_microcents(model) * input_tokens as i64,
    output_microcents: per_output_token_microcents(model) * output_tokens as i64,
    provider_microcents: total_billed_microcents,  // sum of input + output
    estimate_microcents: pre_call_estimate,
    currency: Currency::Usd,
};
```

The `provider_microcents` is the actual billed amount per
Anthropic's pricing table (kept in
`efe-agentic-bridge::pricing::ANTHROPIC_PRICING_TABLE`).  Sprint
19 N27 ships a real-time pricing fetcher ; Sprint 18 ships a
table-driven approximation.

### G6 — Logical-call record (not per-turn)

The two HTTP requests (turn 1 + turn 2) collapse into one record.
`provider_response_time_ms` = wall-clock sum.  `token_usage` =
sum of both turns.  `tool_calls` includes the turn-1
`tool_use` block as `ToolCallRecord` so the auditor can see the
agent asked for the bundle.

### G7 — PII redaction trail copied to record

`audit_context.pii_strategy_applied` and
`audit_context.redaction_token_set` are copied verbatim into the
record's matching fields.  The caller is responsible for
populating these (the PiiRedactor agent writes them into the
bundle ; the bundle reader copies them into the task).

### G8 — Publisher identity snapshot pinned at call time

`audit_context.publisher_id` + `publisher_kyc_tier` are captured
at the moment the bridge call begins.  Subsequent registry
mutations (tier promotion, suspension, revocation) do NOT alter
the record (I18).

### G9 — Gold-case attribution

`audit_context.gold_case_id` is propagated by the gold-runner
(N19) when a Claude call is part of a gold case ; flows through
the audit context to the record.  `None` for production traffic
(I9).

### G10 — `LlmProvenanceCollector::pre_call` integrated

Before the first HTTP call, the bridge invokes
`collector.pre_call(&pre, None)`.  Pre-call hash verification
(N17 G3 / F3) fires here ; failure → `BridgeError::
ProvenanceVerifyFailed { reason }`.  No HTTP call is made on a
pre-call failure (I1 fail-closed).

### G11 — Tenant-id overwrite at the gateway

The `agentic_call_handler` overwrites
`task.audit_context.tenant_id` with the JWT claim's
`tenant_id` BEFORE forwarding to the bridge.  Client-supplied
value is dropped + a metric ticks
(`n26_tenant_id_overwritten_total`).

### G12 — `GET /v1/llm-provenance` operator endpoint

Operator-scoped HTTP endpoint that exposes the existing
`LlmProvenanceQueryApi` :
`GET /v1/llm-provenance?tenant_id=...&since=...&until=...&limit=...`.
Returns the paginated list of `LlmProvenanceRecord` entries.
JWT operator scope required ; tenant filter optional.

### G13 — Gateway `AppState` extension

`AppState` gains :

```rust
pub llm_provenance: Option<Arc<dyn LlmProvenanceCollector>>,
```

Optional ; absence preserves Sprint 16 behaviour.  The collector
is also injected into the `ClaudeBridge` constructor in
`main.rs::build_app_state`.

### G14 — Smoke binary wiring

`efe-agentic-bridge/src/bin/efe_claude_smoke.rs` wires a
`DefaultLlmProvenanceCollector` backed by an
`InMemoryLlmProvenanceStore`.  After the smoke call, the binary
asserts the store contains exactly one record + prints its
`provenance_id` + `trace_id` + costs.

### G15 — Test crate

- `efe-agentic-bridge/tests/audit_n26_*.rs` (8+ tests).
- `efe-agentic-bridge/tests/chaos_n26_*.rs` (10+ tests).
- `efe-gateway/tests/phase_9_n26_*.rs` (12+ acceptance tests
  including HTTP endpoint coverage).
- `efe-agentic-bridge/src/bin/efe_claude_smoke.rs` updated.

---

## 4. Invariants — non-negotiables (I1–I20)

| #   | Invariant                                                                                          |
|-----|----------------------------------------------------------------------------------------------------|
| I1  | When `provenance.is_some() && audit_context.is_some()`, the bridge MUST emit exactly one record.   |
| I2  | When either is `None`, no record is emitted ; a metric ticks the skip reason.                       |
| I3  | Two-turn dispatch produces exactly ONE record (logical call, not per-HTTP-turn).                   |
| I4  | `collector.pre_call` is called BEFORE any HTTP request to Anthropic.                                |
| I5  | A pre-call verification failure SHORT-CIRCUITS the bridge (no HTTP call, no record).               |
| I6  | The post-call emission is wrapped in a 500 ms timeout ; never blocks the bridge response.          |
| I7  | All four prompt hashes + the output hash are BLAKE3 ; stable across runs for the same inputs.      |
| I8  | A collector emission FAILURE is logged + metric-ticked but does NOT propagate as a BridgeError.   |
| I9  | `audit_context.gold_case_id` propagates verbatim to the record (no rewrite, no inference).         |
| I10 | The crate `efe-agentic-bridge` is `#![deny(unsafe_code)]` ; clippy `-D warnings` clean.            |
| I11 | A record's `provider_request_id` MUST equal Anthropic's response `id` field when available.        |
| I12 | The gateway handler ALWAYS overwrites `audit_context.tenant_id` with the JWT claim (G11).          |
| I13 | The new `efe-agentic-bridge` dependency on `efe-llm-provenance` is unidirectional.                  |
| I14 | The `model_identity` recorded equals the model used (verified via N17 ModelRegistry hash).         |
| I15 | `provider_response_time_ms` is the sum of both turn latencies (turn 1 + turn 2).                    |
| I16 | Backward compat : Sprint 13 callers passing `AgenticTask` without `audit_context` continue to work. |
| I17 | A record's `tool_calls` includes the turn-1 `tool_use` block as a `ToolCallRecord`.                |
| I18 | Publisher identity + KYC tier are CAPTURED at call time (snapshot, not live).                       |
| I19 | The bridge does NOT take any `tokio::sync::Mutex` lock across `.await` (N20 lesson).                |
| I20 | The new crate `AgenticAuditContext` is serde-stable ; field renames break the wire (bump version). |

---

## 5. Failure modes — what we PROTECT against (F1–F15)

| #   | Failure                                                                  | Mitigation                                |
|-----|--------------------------------------------------------------------------|-------------------------------------------|
| F1  | Forged `audit_context.tenant_id` from a malicious client                 | G11 + I12 ; handler overwrites             |
| F2  | Pre-call hash mismatch (model substituted by a malicious mirror)         | G10 + I5 ; fail closed before HTTP        |
| F3  | Anthropic returns a model id different from the requested one            | I14 + I11 ; record carries the actual id  |
| F4  | Collector store backend down → records lost silently                     | I8 + ADR-5 ; WARN log + metric            |
| F5  | Collector store slow → bridge call latency explodes                      | I6 + ADR-6 ; 500 ms timeout                |
| F6  | Two-turn protocol mismatch → no post-call record                         | I3 ; closures always reach post_call      |
| F7  | Record carries stale publisher tier (after promotion mid-call)           | I18 ; capture at call start                |
| F8  | Provenance emission throws → BridgeError leaks to caller                 | I8 ; emission errors do NOT propagate     |
| F9  | Two concurrent calls on the same bridge race the call_count counter      | atomic counters ; no Mutex                |
| F10 | Anthropic API key leaked into the record (PII / secret)                  | I15 ; record never carries the key        |
| F11 | Hash collision under crafted input                                       | BLAKE3 collision-resistance               |
| F12 | Bundle larger than N12 cap                                               | bundle size pre-check (existing, Sprint 13) |
| F13 | Caller forgets to populate `audit_context` → no record                  | metric tick + WARN log (I2)                |
| F14 | The 5-year retention contract is violated by store eviction               | Sprint 20 N28 ships Postgres store with retention SLA |
| F15 | The N17 record schema changes mid-flight → query fails                  | I20 ; serde-stable + version bumps        |

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1 — Additive `AgenticAuditContext` field on `AgenticTask`

**Decision** : extend `AgenticTask` with
`audit_context: Option<AgenticAuditContext>`.

**Rejected alternatives** :
- Add a new method `call_with_context_and_audit(...)`.  Rejected
  because every future bridge (OpenAI, Bedrock, etc.) would need
  the same method ; the trait surface bloats linearly.
- Pass the audit context as an Extension on the HTTP request
  (tower middleware).  Rejected — couples the bridge to HTTP
  ; the bridge is used by the smoke binary and tests without
  HTTP.

### ADR-2 — Collector injection via builder, not constructor

**Decision** : `ClaudeBridge::with_provenance(collector)`
builder ; not a required constructor argument.

**Rationale** : the smoke binary and many tests need to build a
bridge WITHOUT a collector (e.g., for stub-only flows).  A
required constructor argument would force every caller to wire
a no-op collector — unnecessary friction.

### ADR-3 — Single record per logical call

**Decision** : the two HTTP turns produce ONE
`LlmProvenanceRecord`.

**Rejected** : two records (one per turn).  Rejected because
the auditor's mental model is "the LLM call" — the two turns
are an artefact of Anthropic's tool-use protocol.  Sprint 19's
streaming change extends the same record via tool_calls
entries.

### ADR-4 — Cost table baked into the crate

**Decision** : the per-model pricing (`ANTHROPIC_PRICING_TABLE`)
is a `const HashMap<&str, ModelPricing>` baked into the crate.

**Rejected** : runtime fetch from Anthropic's pricing endpoint.
Rejected because Anthropic doesn't publish a stable pricing API ;
and a runtime fetch adds a dependency that can fail at startup.
Sprint 19 N27 will add a config-driven override file
(`/etc/efe/llm-pricing.toml`) for tenant-specific deals.

### ADR-5 — Collector errors are non-blocking

**Decision** : if the collector errors during `post_call`, the
bridge LOGS at WARN + ticks a metric but RETURNS the
`AgenticResponse` to the caller anyway.

**Rationale** : the LLM call already succeeded.  Refusing to
return the response would mean charging the customer for an
LLM call they never got the answer for.  The forensic record
is best-effort ; the F4 + F8 mitigations document the trade-off.

### ADR-6 — 500 ms collector timeout

**Decision** : `tokio::time::timeout(500 ms, collector.post_call(...))`.

**Rationale** : the LLM call typically runs ~1-3 s ; an
in-memory store completes in < 1 ms ; a Postgres store
completes in < 50 ms.  A 500 ms ceiling absorbs occasional
slow writes without breaking the customer-visible latency SLO.

### ADR-7 — Hashes computed inside the bridge, not the collector

**Decision** : the bridge computes `system_prompt_hash`,
`user_prompt_hash`, `final_prompt_hash`, `prompt_inputs_hash`,
`output_hash`.

**Rationale** : the collector is generic across bridges ; each
bridge knows its own prompt assembly.  Centralising hashing in
the bridge keeps the collector small.  All hashes use the same
helper `efe_llm_provenance::hash::prompt_hashes_for` so the
collision resistance is identical across bridges.

### ADR-8 — `prompt_inputs_hash` = canonical bundle hash

**Decision** : `prompt_inputs_hash = blake3(serde_json::to_vec(bundle))`.

**Rationale** : the bundle IS the input to the swarm-context
tool ; hashing it gives the audit chain a stable anchor for
the input side.  Sprint 22+ may switch to a canonical JSON
serialiser for cross-language stability.

### ADR-9 — Operator scope on the read endpoint

**Decision** : `GET /v1/llm-provenance` requires the JWT
`operator` scope ; tenants cannot read their own records via
this endpoint.

**Rationale** : the records carry PII tokens, internal trace
ids, and publisher identities ; the surface is too sensitive
for direct tenant access.  Sprint 19 N27 ships a per-tenant
filtered subset endpoint.

### ADR-10 — Backward compatibility above all

**Decision** : Sprint 13 callers (smoke binary, tests, gateway
handlers) MUST compile + behave identically without any change
to their code.  N26 is purely additive at the API surface.

---

## 7. Component architecture

```
                                ┌──────────────────────────────────────┐
                                │  efe-gateway HTTP surface (N22)       │
                                │   AppState                            │
                                │   ├── publisher_api  (N18)            │
                                │   ├── gold_api       (N19)            │
                                │   ├── agentic        (N21)            │
                                │   ├── compliance     (N24)            │
                                │   └── llm_provenance (N26, NEW)       │
                                └─────────────┬────────────────────────┘
                                              │ Arc<dyn LlmProvenanceCollector>
                                              ▼
              ┌──────────────────────────────────────────────────┐
              │   efe-llm-provenance (Sprint 9, shipped)          │
              │   ├── LlmProvenanceCollector trait                │
              │   ├── DefaultLlmProvenanceCollector impl          │
              │   ├── call_with_provenance() glue                 │
              │   ├── InMemoryLlmProvenanceStore                  │
              │   ├── PreCallProvenance / PostCallProvenance       │
              │   ├── hash::prompt_hashes_for                      │
              │   ├── N17Metrics                                   │
              │   └── LlmProvenanceQueryApi                        │
              └─────────────────┬────────────────────────────────┘
                                │ Arc<dyn LlmProvenanceCollector>
                                ▼
          ┌───────────────────────────────────────────┐
          │  efe-agentic-bridge (Sprint 13 N21 + N26)  │
          │   ClaudeBridge                              │
          │   ├── provenance: Option<Arc<dyn ...>>      │  ← NEW (N26)
          │   ├── pricing::ANTHROPIC_PRICING_TABLE     │  ← NEW (N26)
          │   └── AgenticTask.audit_context             │  ← NEW (N26)
          └─────────────────┬─────────────────────────┘
                            │ HTTP /v1/messages
                            ▼
                  ┌───────────────────┐
                  │  api.anthropic.com │
                  └───────────────────┘
```

### 7.1 Module boundaries

- **`efe-agentic-bridge`** :
  - `claude.rs` — wraps Anthropic API ; new `provenance`
    field + post-call emission logic.
  - `task.rs` — new `AgenticAuditContext` struct.
  - `pricing.rs` (NEW) — `ANTHROPIC_PRICING_TABLE` const + helpers.
  - `error.rs` — new `BridgeError::ProvenanceVerifyFailed`
    variant.

- **`efe-llm-provenance`** : zero changes.  The existing public
  surface is sufficient.

- **`efe-gateway`** :
  - `state.rs` — `AppState.llm_provenance` field + builder.
  - `handlers/agentic.rs` — populates `audit_context` from JWT
    claims + bundle.
  - `handlers/llm_provenance.rs` (NEW) — `GET /v1/llm-provenance`
    handler.
  - `handlers/openapi.rs` — new path + schema.
  - `router.rs` — new route.
  - `main.rs::build_app_state` — wires the collector + injects
    into ClaudeBridge.

### 7.2 Dependency arrows

```
efe-gateway  →  efe-agentic-bridge  →  efe-llm-provenance
efe-gateway  →  efe-llm-provenance  (direct, for the read endpoint)
```

No back-arrows.  `efe-agentic-bridge` did not previously
depend on `efe-llm-provenance` ; N26 adds this dependency.

---

## 8. Module hierarchy & data model

### 8.1 New code in `efe-agentic-bridge`

```
crates/efe-agentic-bridge/
├── Cargo.toml                     # add efe-llm-provenance dep
├── src/
│   ├── lib.rs                     # re-export AgenticAuditContext
│   ├── bridge.rs                  # no change
│   ├── claude.rs                  # MAJOR : provenance wiring
│   ├── error.rs                   # add ProvenanceVerifyFailed variant
│   ├── pricing.rs                 # NEW : ANTHROPIC_PRICING_TABLE
│   ├── task.rs                    # add AgenticAuditContext field
│   ├── tool_use.rs                # no change
│   └── bin/
│       └── efe_claude_smoke.rs    # wire the collector + assert
└── tests/
    ├── audit_n26_provenance_wiring.rs    # NEW
    ├── chaos_n26_provenance_wiring.rs    # NEW
    └── phase_9_n26_smoke.rs              # NEW
```

### 8.2 New code in `efe-gateway`

```
crates/efe-gateway/
├── Cargo.toml                     # add efe-llm-provenance dep
├── src/
│   ├── state.rs                   # add llm_provenance field
│   ├── handlers/
│   │   ├── mod.rs                 # re-export llm_provenance handler
│   │   ├── agentic.rs             # populate audit_context
│   │   ├── llm_provenance.rs      # NEW : GET /v1/llm-provenance
│   │   └── openapi.rs             # add /v1/llm-provenance entry
│   ├── router.rs                  # add new route
│   └── main.rs                    # wire collector + inject into bridge
└── tests/
    └── phase_9_n26_llm_provenance.rs   # NEW
```

### 8.3 Data model — `AgenticAuditContext`

```rust
// crates/efe-agentic-bridge/src/task.rs
use efe_nano_sdk::{
    GoldCaseId, KycTier, NanoAgentId, NanoAgentVersion, PiiStrategy, PublisherId,
    RegionCode, TenantId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// **Sprint 18 — N26** : pre-call audit context carried alongside
/// every `AgenticTask`.  Populated by the caller (gateway handler /
/// swarm runner / MCP tool) ; consumed by `ClaudeBridge` to build
/// the `PreCallProvenance` for N17 emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticAuditContext {
    /// W3C trace id (or Uuid v4 fallback).
    pub trace_id: Uuid,
    /// Swarm request id from N21 orchestration ; same value as
    /// `SwarmReport.swarm_id` for swarm-driven calls ; fresh for
    /// direct `agentic_call_handler` calls.
    pub swarm_request_id: Uuid,
    /// Operation id ("op-1", "claude-summarize", etc.).
    pub op_id: String,
    /// Tenant id from the JWT claim (overwritten at the gateway).
    pub tenant_id: TenantId,
    /// Calling agent id ("summary_writer", "agentic_proxy", ...).
    pub agent_id: NanoAgentId,
    /// Calling agent semver.
    pub agent_version: NanoAgentVersion,
    /// Prompt template id ("swarm_context_v1", ...).
    pub prompt_template_id: String,
    /// Prompt template semver.
    pub prompt_template_version: String,
    /// PII strategy applied to the bundle BEFORE this call.
    pub pii_strategy_applied: PiiStrategy,
    /// Redaction tokens injected into the bundle.
    pub redaction_token_set: Vec<String>,
    /// Provider region if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_region: Option<RegionCode>,
    /// Publisher id snapshot at call time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<PublisherId>,
    /// Publisher KYC tier snapshot at call time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_kyc_tier: Option<KycTier>,
    /// Gold case id if this call is part of a gold test run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_case_id: Option<GoldCaseId>,
}
```

### 8.4 Data model — extended `AgenticTask`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticTask {
    pub user_message: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// **N26** : optional audit context.  `None` for backward
    /// compat ; populated by the gateway handler for production.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<AgenticAuditContext>,
}
```

### 8.5 Pricing table

```rust
// crates/efe-agentic-bridge/src/pricing.rs
use std::sync::OnceLock;
use std::collections::HashMap;

/// **N26 + ADR-4** : per-model pricing in microcents per token.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_microcents_per_1k: i64,
    pub output_microcents_per_1k: i64,
}

/// **N26 + ADR-4** : baked Anthropic pricing table (Sprint 18
/// snapshot).  Update on each Anthropic pricing announcement.
pub fn anthropic_pricing_for(model: &str) -> ModelPricing {
    static TABLE: OnceLock<HashMap<&'static str, ModelPricing>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        // Sonnet 4.6 : $3 / Mtoken input, $15 / Mtoken output.
        m.insert("claude-sonnet-4-6", ModelPricing {
            input_microcents_per_1k: 30,   // $3/M = 300 microcents / 1k
            output_microcents_per_1k: 150, // $15/M = 1500 microcents / 1k
        });
        m.insert("claude-haiku-3-5", ModelPricing {
            input_microcents_per_1k: 8,    // $0.80/M
            output_microcents_per_1k: 40,  // $4/M
        });
        m.insert("claude-opus-4", ModelPricing {
            input_microcents_per_1k: 150,  // $15/M
            output_microcents_per_1k: 750, // $75/M
        });
        m
    });
    t.get(model).copied().unwrap_or(ModelPricing {
        input_microcents_per_1k: 30,
        output_microcents_per_1k: 150,
    })
}

/// **N26** : compute exact cost microcents from a token count.
pub fn compute_cost_microcents(model: &str, input_tokens: u32, output_tokens: u32) -> i64 {
    let p = anthropic_pricing_for(model);
    let in_cost = (input_tokens as i64) * p.input_microcents_per_1k / 1000;
    let out_cost = (output_tokens as i64) * p.output_microcents_per_1k / 1000;
    in_cost + out_cost
}
```

### 8.6 Error taxonomy extension

```rust
// crates/efe-agentic-bridge/src/error.rs
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BridgeError {
    // ... existing variants ...

    /// **N26 + G10 + I5** : pre-call provenance verification
    /// failed (model hash mismatch, store fault, etc.).
    #[error("provenance verify failed : {reason}")]
    ProvenanceVerifyFailed {
        reason: String,
    },
}
```

---

## 9. API surface (Rust traits + HTTP)

### 9.1 Public Rust API

| Symbol                                       | Kind   | Purpose                                                       |
|----------------------------------------------|--------|---------------------------------------------------------------|
| `AgenticAuditContext`                        | struct | New per-call audit context                                    |
| `AgenticTask.audit_context`                  | field  | Additive Option field on existing AgenticTask                 |
| `ClaudeBridge::with_provenance(collector)`   | fn     | Inject the collector                                          |
| `BridgeError::ProvenanceVerifyFailed`        | variant| Pre-call verification failure                                 |
| `pricing::anthropic_pricing_for(model)`      | fn     | Lookup the per-model pricing                                  |
| `pricing::compute_cost_microcents(...)`      | fn     | Compute exact cost                                            |

### 9.2 Public HTTP API

`GET /v1/llm-provenance` — operator scope.

```
Query :
  tenant_id        : Option<Uuid>
  since            : Option<RFC3339>
  until            : Option<RFC3339>
  agent_id         : Option<String>
  publisher_id     : Option<Uuid>
  gold_case_id     : Option<Uuid>
  limit            : Option<u32> (default 50, max 500)
  cursor           : Option<String>

Response 200 :
  {
    "schema": "llm_provenance.v1",
    "generated_at": "<RFC3339>",
    "records": [ ... LlmProvenanceRecord ... ],
    "next_cursor": "<string|null>"
  }

Errors :
  401 : missing-auth
  403 : operator-scope-required
  503 : collector-not-configured
```

### 9.3 Modified handler shape

```rust
// crates/efe-gateway/src/handlers/agentic.rs
pub async fn agentic_call_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<AgenticCallRequest>,
) -> Result<impl IntoResponse, ApiError> {
    check_tenant(&claims, req.tenant_id)?;
    let mut task = req.task;
    // **N26 G11 + I12** : OVERWRITE the audit context tenant
    // with the JWT claim's tenant.  Client-supplied value
    // ignored ; metric ticks if mismatched.
    if let Some(ctx) = task.audit_context.as_mut() {
        if ctx.tenant_id != TenantId::from_uuid(claims.tenant_id) {
            state.metrics.tick_audit_context_overwritten();
            ctx.tenant_id = TenantId::from_uuid(claims.tenant_id);
        }
    } else {
        // Populate a default audit context for production safety.
        task.audit_context = Some(default_audit_context(&claims, &req));
    }
    let resp = state.agentic.call_with_context(&req.bundle, task).await?;
    Ok(Json(resp))
}
```

### 9.4 OpenAPI 3.1 spec update

```json
"/v1/llm-provenance": {
  "get": {
    "summary": "List LLM provenance records (operator scope)",
    "tags": ["audit"],
    "security": [{ "bearerAuth": ["operator"] }],
    "parameters": [
      { "name": "tenant_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
      { "name": "since", "in": "query", "schema": { "type": "string", "format": "date-time" } },
      { "name": "until", "in": "query", "schema": { "type": "string", "format": "date-time" } },
      { "name": "agent_id", "in": "query", "schema": { "type": "string" } },
      { "name": "publisher_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
      { "name": "gold_case_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
      { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50, "maximum": 500 } },
      { "name": "cursor", "in": "query", "schema": { "type": "string" } }
    ],
    "responses": {
      "200": { "description": "Paginated LlmProvenanceRecord list",
               "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LlmProvenanceListResponse" } } } },
      "403": { "description": "Operator scope required" },
      "503": { "description": "Collector not configured" }
    }
  }
}
```

---

## 10. End-to-end workflows

### Workflow A — Gateway agentic call with full provenance

```
Client                Gateway                ClaudeBridge          LlmProvenanceCollector       Anthropic API       N17 Store
  │                      │                      │                      │                          │                  │
  │ POST /v1/agentic/call│                      │                      │                          │                  │
  │ Bearer <jwt>         │                      │                      │                          │                  │
  │─────────────────────►│                      │                      │                          │                  │
  │                      │ check_tenant         │                      │                          │                  │
  │                      │ build audit_context  │                      │                          │                  │
  │                      │ (overwrite tenant)   │                      │                          │                  │
  │                      │─────────────────────►│ call_with_context    │                          │                  │
  │                      │                      │ assemble PreCallProv │                          │                  │
  │                      │                      │────────────────────►│ pre_call(pre, None)       │                  │
  │                      │                      │                      │ verify_hash → ok          │                  │
  │                      │                      │ ◄─ Ok(())            │                          │                  │
  │                      │                      │ HTTP turn 1 (ask)    │                          │                  │
  │                      │                      │──────────────────────────────────────────────► │                  │
  │                      │                      │ ◄─ tool_use block    │                          │                  │
  │                      │                      │ HTTP turn 2 (bundle) │                          │                  │
  │                      │                      │──────────────────────────────────────────────► │                  │
  │                      │                      │ ◄─ assistant text    │                          │                  │
  │                      │                      │ compute hashes       │                          │                  │
  │                      │                      │ assemble PostCallProv│                          │                  │
  │                      │                      │────────────────────►│ post_call(pre, post)      │                  │
  │                      │                      │                      │ build LlmProvenanceRecord │                  │
  │                      │                      │                      │──────────────────────────────────────────► │
  │                      │                      │                      │ ◄─ Ok(record)             │                  │
  │                      │                      │ ◄─ AgenticResponse   │                          │                  │
  │                      │ ◄─ AgenticResponse   │                      │                          │                  │
  │ ◄─ 200               │                      │                      │                          │                  │
```

### Workflow B — Gateway agentic call WITHOUT provenance configured (N21 fallback)

If `state.llm_provenance.is_none()`, the bridge dispatches as
Sprint 13 — no `pre_call`, no `post_call`, just two HTTP
turns and the AgenticResponse.  The handler ticks
`n26_provenance_skipped_total{reason="no_collector"}`.

### Workflow C — Smoke binary with live API key

```
$ ANTHROPIC_API_KEY=sk-ant-... target/release/efe_claude_smoke "Summarise this bundle"
[INFO] N26 provenance collector wired (InMemoryLlmProvenanceStore)
[INFO] N21 ClaudeBridge call : claude-sonnet-4-6 model
[INFO] turn 1 : 124 ms (input 250 tokens, output 18 tokens)
[INFO] turn 2 : 980 ms (input 1024 tokens, output 412 tokens)
[INFO] N26 provenance record persisted
        provenance_id     : 78e1b5fa-9c2d-4f1e-9b3c-0e4a1b2c3d4f
        trace_id          : 4e9a8b7c-d1e2-4f3a-9b8c-7a6b5c4d3e2f
        tenant_id         : <uuid>
        model_identity    : anthropic/claude-sonnet-4-6@2023-06-01
        total_tokens      : 1700
        cost_microcents   : 138_300  ($0.01383)
        provider_response_time_ms : 1104
        publisher_id      : efe0be00-0000-4000-8000-000000000018
        publisher_kyc_tier: tier_3
```

### Workflow D — Operator query for tenant LLM history

```
GET /v1/llm-provenance?tenant_id=550e8400-...&since=2026-05-01T00:00Z&limit=50
Authorization: Bearer <operator-jwt>

Response 200 :
{
  "schema": "llm_provenance.v1",
  "generated_at": "2026-05-22T10:00:00Z",
  "records": [ /* up to 50 records, sorted by recorded_at desc */ ],
  "next_cursor": "eyJyZWNvcmRlZF9hdCI6IjIwMjYtMDUtMDhUMDk6MDA6MDBaIn0"
}
```

### Workflow E — Replay (Sprint 19+ — anchor here)

The N16 replay subsystem reads a 30-day-old
`LlmProvenanceRecord`, reconstructs the canonical prompt from
the hashes + the bundle (looked up by `swarm_request_id`), and
re-dispatches via `ClaudeBridge` with the SAME audit context.
The new dispatch produces a NEW record with `replay_of = old_id`.
The auditor compares `output_hash` between old and new ; a
match proves byte-stable replay.

---

## 11. Five-day implementation plan

### Day 1 — AgenticAuditContext + pricing table + error variant

- Create `task.rs::AgenticAuditContext`.
- Add `AgenticTask.audit_context: Option<AgenticAuditContext>`.
- Create `pricing.rs` with the baked Anthropic table.
- Add `BridgeError::ProvenanceVerifyFailed { reason }`.
- Re-export from `lib.rs`.
- Unit tests : 15+ (serde round-trip, default tier, pricing
  lookup for known + unknown models, cost compute correctness).

### Day 2 — ClaudeBridge wiring

- Add `provenance: Option<Arc<dyn LlmProvenanceCollector>>` to
  `ClaudeBridge`.
- Add `with_provenance(collector)` builder.
- Modify `call_with_context` :
  - When `audit_context.is_some() && provenance.is_some()`, wrap
    the two-turn dispatch in `call_with_provenance(...)`.
  - Compute prompt hashes inside the closure.
  - Compute output hash post-call.
  - Convert Anthropic usage to N17 `CostAccounting` via
    `pricing::compute_cost_microcents`.
  - Capture `provider_request_id` from the Anthropic response.
- Unit tests : 20+ (provenance enabled vs disabled, no
  audit_context, no collector, partial audit_context).

### Day 3 — Gateway integration

- `efe-gateway/Cargo.toml` : add `efe-llm-provenance` dep.
- `AppState.llm_provenance: Option<Arc<dyn LlmProvenanceCollector>>`.
- `AppState::builder().with_llm_provenance(...)`.
- `handlers/agentic.rs` : populate + overwrite audit_context.
- `handlers/llm_provenance.rs` : new operator-scoped handler.
- `router.rs` : add route.
- `handlers/openapi.rs` : new path + schema.
- `main.rs::build_app_state` : wire the collector + inject
  into `ClaudeBridge::with_provenance(collector)`.

### Day 4 — Smoke binary + tests start

- Update `efe_claude_smoke.rs` to wire a collector + assert
  record persisted.
- Audit + chaos tests start landing :
  - `audit_n26_*.rs` : 8 invariants (I1, I3, I5, I8, I11, I12, I14, I18).
  - `chaos_n26_*.rs` : 10 failure modes (F1, F2, F4, F5, F7, F8, F9, F10, F11, F13).
- Phase-9 acceptance tests start :
  - `phase_9_n26_llm_provenance.rs` (gateway HTTP coverage).

### Day 5 — Tests, smoke, docs

- Finalise tests (target : 12 phase-9 + 8 audit + 10 chaos).
- Live smoke against the user's real Anthropic API key.
- Workspace test + clippy strict.
- `docs/migration/N26_AGENTIC_PROVENANCE.md`.
- Final summary.

---

## 12. Test strategy

### 12.1 Unit tests (target : 80+)

- **`task.rs`** : 12+ (serde round-trip with + without
  audit_context, default values).
- **`pricing.rs`** : 15+ (every model in the table, unknown
  model fallback, integer overflow safety on large token
  counts, zero tokens edge case).
- **`claude.rs`** : 25+ (mock the Anthropic API via wiremock ;
  test provenance enabled vs disabled, hash computation, cost
  translation, error path mapping, two-turn collapse).
- **`error.rs`** : 8+ (variant labels, display strings,
  serde round-trip on the new variant).

### 12.2 Phase-9 acceptance (`phase_9_n26_*.rs`, 12 tests)

- Workflow A : full provenance emission through the gateway.
- Workflow B : no collector wired → 200 + metric tick.
- Operator endpoint returns 200 with records.
- Tenant JWT on operator endpoint → 403.
- Audit context tenant overwrite (G11 + I12).
- `gold_case_id` propagation when present.
- `publisher_id` snapshot pinning.
- `provider_request_id` populated.
- Two-turn collapse → 1 record.
- 503 when collector absent.
- OpenAPI spec correctness.
- Operator endpoint pagination via cursor.

### 12.3 Audit regression (`audit_n26_*.rs`, 8 tests)

- I1 : provenance enabled → exactly one record per call.
- I3 : two HTTP turns → one record (not two).
- I5 : pre-call failure short-circuits the HTTP path.
- I8 : post-call collector error does NOT propagate as BridgeError.
- I11 : `provider_request_id` matches Anthropic response `id`.
- I12 : audit_context tenant overwritten by JWT.
- I14 : `model_identity` matches the requested model.
- I18 : publisher tier captured at call time (not at query time).

### 12.4 Chaos (`chaos_n26_*.rs`, 10 tests)

- F1 : forged tenant_id in audit_context → handler overwrites.
- F2 : pre_call hash mismatch → fail closed.
- F4 : collector store backend down → log + metric, response OK.
- F5 : slow collector (sleep 600 ms) → 500 ms timeout fires.
- F7 : publisher tier flipped mid-call → record has old tier.
- F8 : collector panics → caught by N22 CatchPanic.
- F9 : 20 concurrent bridge calls → counters consistent.
- F10 : api key absent from record (no leakage).
- F11 : two records with same hash inputs → identical hashes.
- F13 : missing audit_context → metric tick, no record.

### 12.5 Smoke

- `efe_claude_smoke` live test :
  - boots with `ANTHROPIC_API_KEY` set.
  - wires an `InMemoryLlmProvenanceStore`.
  - dispatches a small call.
  - asserts store has exactly 1 record.
  - prints record fields + costs.

---

## 13. Cross-cutting alignment (N11 → N24)

| Module | Sprint | What N26 changes / aligns                                            |
|--------|--------|----------------------------------------------------------------------|
| N11 — Firecracker pool | 8 | No change. |
| N12 — Cache key hardening | 8 | A future cache layer in ClaudeBridge would consult N17 cache_hit ; deferred. |
| N13 — Shadow/canary | 8 | No change. |
| N14 — Swarm envelope | 8 | The N26 record's `trace_id` matches the N14 envelope trace. |
| N15 — Fairness | 8 | No change. |
| N16 — Multi-region replay | 9 | N26 makes Claude calls replay-able via the prompt hashes. |
| N17 — LLM provenance | 9 | This sprint wires N17 into the agentic bridge ; no N17 surface change. |
| N18 — Publisher identity | 10 | `audit_context.publisher_id` + `kyc_tier` are populated from N18 identity. |
| N19 — Gold test gate | 11 | `audit_context.gold_case_id` flows from the gold runner. |
| N20 — Postgres embeddings | 12 | No change. |
| N21 — Swarm orchestration | 13 | The N21 swarm propagates the audit_context to the bridge ; no swarm surface change. |
| N22 — HTTP gateway | 14 | New AppState field + new admin endpoint ; existing handlers unchanged behaviourally (audit_context auto-populated). |
| N23 — NanoAgent adapter | 15 | The summary_writer agent (Mode C) populates audit_context from the adapter context. |
| N24 — Wired compliance | 16 | The publisher identity used by N26 audit_context is the SAME publisher identity the N24 status gate consulted. |

---

## 14. Sprint 18 ship vs Sprint 19+ split

### Sprint 18 ships (N26)

- `AgenticAuditContext` + `AgenticTask.audit_context`.
- `ClaudeBridge::with_provenance` + full N17 emission.
- Pricing table baked.
- `BridgeError::ProvenanceVerifyFailed`.
- Gateway integration : AppState, handler population,
  operator endpoint, OpenAPI.
- Smoke binary update.
- 80+ unit / 12 acceptance / 8 audit / 10 chaos tests.

### Sprint 19+ defers

- **N27 — Streaming SSE provenance** : extend the same record
  with stream-event entries via `tool_calls`.
- **N28 — Postgres LlmProvenanceStore** : durable backing
  store with 5-year retention SLA.
- **N29 — Event-bus emission** : publish every record on the
  workspace event bus for FinOps dashboards.
- **N30 — Per-tenant read endpoint** : `GET /v1/me/llm-provenance`
  for tenants to see their own records (subset of fields).
- **N31 — Cross-bridge canonicalisation** : OpenAIBridge,
  BedrockBridge, etc., all reuse the same injection pattern.
- **N32 — Real-time pricing** : config-driven override file
  for tenant-specific pricing deals.

---

## 15. Annexes

### A1 — Two-turn → one-record mapping

```rust
// Inside ClaudeBridge::call_with_context :

let result = if let (Some(ctx), Some(collector)) =
    (&task.audit_context, &self.provenance)
{
    let pre = build_pre_call_provenance(ctx, &self.model, &bundle, &task);
    call_with_provenance(
        collector.as_ref(),
        pre,
        None, // hosted model — no pre-call hash to verify
        |pre_clone| async move {
            // Turn 1.
            let t1_start = Instant::now();
            let resp1 = self.send(&url, &req1).await
                .map_err(bridge_err_to_provenance)?;
            let parsed1 = self.parse_response(resp1).await
                .map_err(bridge_err_to_provenance)?;
            let t1_ms = t1_start.elapsed().as_millis() as u32;

            // Turn 2.
            let t2_start = Instant::now();
            let resp2 = self.send(&url, &req2).await
                .map_err(bridge_err_to_provenance)?;
            let parsed2 = self.parse_response(resp2).await
                .map_err(bridge_err_to_provenance)?;
            let t2_ms = t2_start.elapsed().as_millis() as u32;

            let text = extract_text(&parsed2)?;
            let usage_total = parsed1.usage.unwrap_or_default() + parsed2.usage.unwrap_or_default();
            let output_hash = hash_bytes(text.as_bytes());

            let post = PostCallProvenance {
                token_usage: TokenUsage { input_tokens: usage_total.input_tokens,
                                          output_tokens: usage_total.output_tokens },
                cost_accounting: build_cost_accounting(&self.model, &usage_total),
                provider_request_id: parsed2.id.clone(),
                provider_response_time_ms: t1_ms + t2_ms,
                provider_finish_reason: map_finish_reason(&parsed2),
                provider_metadata: build_provider_metadata(&parsed1, &parsed2),
                output_hash: Some(output_hash),
                completeness: derive_completeness(&parsed2),
                safety_flags: derive_safety_flags(&parsed2),
                tool_calls: vec![record_tool_call(&parsed1)?],
                model_hash_verified: false, // hosted model — no hash
            };
            Ok((build_agentic_response(...), post))
        },
    ).await
} else {
    // Sprint 13 fallback — no provenance.
    self.metrics.tick_provenance_skipped(...);
    self.dispatch_without_provenance(bundle, task).await
};
```

### A2 — `build_pre_call_provenance` shape

```rust
fn build_pre_call_provenance(
    ctx: &AgenticAuditContext,
    model: &str,
    bundle: &ContextBundle,
    task: &AgenticTask,
) -> PreCallProvenance {
    use efe_llm_provenance::hash::{hash_bytes, prompt_hashes_for};

    let bundle_json = serde_json::to_vec(bundle).unwrap_or_default();
    let prompt_inputs_hash = hash_bytes(&bundle_json);

    let system_prompt = build_system_prompt(task);
    let final_prompt = format!("{system_prompt}\n\n{}", task.user_message);
    let hashes = prompt_hashes_for(&system_prompt, &task.user_message, &final_prompt);

    PreCallProvenance {
        trace_id: ctx.trace_id,
        swarm_request_id: ctx.swarm_request_id,
        op_id: ctx.op_id.clone(),
        tenant_id: ctx.tenant_id,
        agent_id: ctx.agent_id.clone(),
        agent_version: ctx.agent_version.clone(),
        model_identity: ModelIdentity::hosted(
            "anthropic", model, DEFAULT_ANTHROPIC_VERSION,
        ),
        prompt_template_id: ctx.prompt_template_id.clone(),
        prompt_template_version: ctx.prompt_template_version.clone(),
        prompt_inputs_hash,
        system_prompt_hash: hashes.system_prompt_hash,
        user_prompt_hash: hashes.user_prompt_hash,
        final_prompt_hash: hashes.final_prompt_hash,
        sampling: SamplingParams {
            temperature: Some(task.temperature),
            top_p: None,
            top_k: None,
            max_tokens: Some(task.max_tokens),
            seed: None,
        },
        pii_strategy_applied: ctx.pii_strategy_applied,
        redaction_token_set: ctx.redaction_token_set.clone(),
        provider_region: ctx.provider_region,
        publisher_id: ctx.publisher_id,
        publisher_kyc_tier: ctx.publisher_kyc_tier,
        gold_case_id: ctx.gold_case_id,
    }
}
```

### A3 — Gateway `agentic_call_handler` default audit context

```rust
fn default_audit_context(
    claims: &GatewayClaims,
    req: &AgenticCallRequest,
) -> AgenticAuditContext {
    AgenticAuditContext {
        trace_id: uuid::Uuid::new_v4(),
        swarm_request_id: req.swarm_request_id.unwrap_or_else(uuid::Uuid::new_v4),
        op_id: req.op_id.clone().unwrap_or_else(|| "agentic_proxy".to_string()),
        tenant_id: TenantId::from_uuid(claims.tenant_id),
        agent_id: NanoAgentId::new("agentic_proxy")
            .expect("static agent id is valid"),
        agent_version: NanoAgentVersion::new("1.0.0").expect("static"),
        prompt_template_id: "agentic_proxy_v1".to_string(),
        prompt_template_version: "1.0.0".to_string(),
        pii_strategy_applied: PiiStrategy::Inherit, // bundle carries the truth
        redaction_token_set: req.bundle.metadata.redaction_tokens.clone(),
        provider_region: None,
        publisher_id: None,        // direct agentic call — no publisher
        publisher_kyc_tier: None,
        gold_case_id: None,
    }
}
```

### A4 — N26 metrics (added to `N22GatewayMetrics`)

| Metric | Type | Labels |
|--------|------|--------|
| `n26_provenance_emitted_total` | counter | (none) |
| `n26_provenance_skipped_total` | counter | `reason: no_audit_context|no_collector` |
| `n26_provenance_emit_failed_total` | counter | (none) |
| `n26_provenance_timeout_total` | counter | (none) |
| `n26_audit_context_overwritten_total` | counter | (none) |
| `n26_pre_call_verify_failed_total` | counter | (none) |
| `n26_cost_microcents_total` | counter | `model` |
| `n26_provider_response_time_ms` | histogram | `model` |

### A5 — Audit checklist for the EU AI Act §13 sign-off

After Sprint 18 N26 ships :

- [ ] Every `POST /v1/agentic/call` produces exactly one
      `LlmProvenanceRecord` row ?
- [ ] Every record carries `tenant_id` matching the JWT claim
      (not the client-supplied value) ?
- [ ] Every record carries `publisher_id` + `publisher_kyc_tier`
      snapshot ?
- [ ] Every record carries the four prompt hashes
      (BLAKE3) + the output hash ?
- [ ] Every record carries the full `CostAccounting` breakdown ?
- [ ] `provider_request_id` is populated from Anthropic's
      response `id` ?
- [ ] `provider_response_time_ms` is the sum of both
      turn latencies ?
- [ ] The two HTTP turns collapse into ONE record ?
- [ ] When the collector is absent, the call dispatches
      normally + a metric ticks ?
- [ ] The collector emission is non-blocking : even a
      slow / failing collector does not delay the response
      beyond 500 ms ?
- [ ] `GET /v1/llm-provenance` returns 403 for tenant JWTs ?
- [ ] The smoke binary asserts a record was persisted before
      exit ?

---

_End of N26 architecture._
