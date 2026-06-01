# EFE M50 — N23 NanoAgent ↔ SwarmAgent Adapter + efe-nano Reuse Architecture

> **Sprint**          : 15 (Phase 8, day 71 – 75 of M50).
> **Status**          : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md`           (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md`        (Sprint 11, shipped)
> * `EFE_M50_N20_POSTGRES_REAL_EMBEDDINGS_ARCHITECTURE.md`     (Sprint 12, shipped)
> * `EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md` (Sprint 13, shipped)
> * `EFE_M50_N22_HTTP_GATEWAY_EXPOSURE_ARCHITECTURE.md`        (Sprint 14, shipped)
>
> **Anchors** : N21 introduced `SwarmAgent` (behaviour, DAG-native)
> while keeping the legacy `NanoAgent` catalogue (data, signed,
> gold-gated, idempotent, cost-estimated). The six reference
> pre-analysis agents (`NerExtractor`, `LanguageDetector`,
> `SchemaValidator`, `PiiRedactor`, `SentimentClassifier`,
> `SummaryWriter`) currently implement `SwarmAgent` *directly* and
> bypass the entire M50 compliance stack (signature, gold-set
> binding, idempotency cache, resource limits, provenance).  N23
> closes that asymmetry by shipping a **bidirectional adapter** and
> the **synthetic catalogue back-fill** so every existing nano
> agent runs through one canonical execution path with end-to-end
> auditability.

---

## Table of contents

0.  [TL;DR](#0-tldr)
1.  [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2.  [Twelve incident patterns N23 closes (P1–P12)](#2-twelve-incident-patterns-n23-closes-p1p12)
3.  [Commitments — what ships in Sprint 15 (G1–G15)](#3-commitments--what-ships-in-sprint-15-g1g15)
4.  [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5.  [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6.  [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7.  [Component architecture](#7-component-architecture)
8.  [Module hierarchy & data model](#8-module-hierarchy--data-model)
9.  [API surface (Rust traits + HTTP)](#9-api-surface-rust-traits--http)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N22)](#13-cross-cutting-alignment-n11--n22)
14. [Sprint 15 ship vs Sprint 16+ split](#14-sprint-15-ship-vs-sprint-16-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The asymmetry N23 closes.**

Two ecosystems coexist in the workspace as of Sprint 14 :

| Dimension            | M50 NanoAgent (data + Runtime trait)        | N21 SwarmAgent (behaviour + DAG)            |
|----------------------|---------------------------------------------|---------------------------------------------|
| Shape                | Struct + `dyn NanoAgentExecutor`            | `dyn SwarmAgent`                            |
| Identity             | `NanoAgentId` + `NanoAgentVersion` (semver) | `AgentId` (string)                          |
| I/O contract         | `serde_json::Value` in / `NanoAgentResponse` out | `SwarmAgentContext` in / `AgentOutcome` out |
| Schema validation    | `input_schema` + `output_schema` enforced   | None — free-form blackboard read/write      |
| Cost unit            | `f64` cents (estimate + actual)             | `i64` microcents (actual only)              |
| Signature (N18)      | `Option<PublisherSignature>` Ed25519        | Not represented                              |
| Gold binding (N19)   | `Option<GoldSetRef>` + Tier-1 promotion gate | Not represented                              |
| Idempotency          | `IdempotencyPolicy { Auto, Manual, Disabled }` | Not represented                              |
| Resource limits      | `ResourceLimits { mem, cpu, wall, tokens }` | Implicit (per-agent `tokio::time::timeout`) |
| Execution dispatch   | `MultiModeRuntime` → Mode A / B / C         | `agent.run(ctx).await`                      |
| Determinism level    | `DeterminismLevel` enum (5 levels)          | Per-run `rng_seed: u64`                     |
| Provenance trail     | `TraceId` + cache_hit + audit policy        | Per-agent `AgentReport`                     |
| Compliance tags      | `ComplianceTags { gdpr, hipaa, ... }`       | None                                        |

The six pre-analysis agents (`efe-nano-preanalysis/`) are
`SwarmAgent`-native ; they **never** flow through the M50
`Runtime` dispatcher.  Consequently :

- their signatures are never verified ;
- they are not bound to a gold set ;
- their idempotency cache (N12-style key hardening) never fires ;
- their `ResourceLimits` are never enforced (only the soft
  per-agent timeout in `SwarmDag::with_per_agent_timeout`) ;
- their `cost_estimate_cents` is never compared to the actual
  swarm cost.

**N23 ships the canonical bridge.**

1.  A new crate **`efe-nano-swarm-adapter`** exposing two
    structs :
    -   `NanoToSwarm` — wraps a `(NanoAgent, Arc<dyn NanoAgentExecutor>)`
        pair and implements `SwarmAgent`.  Reads the input from
        the blackboard at a declared JSON-pointer (`/input/text`
        by default, configurable), dispatches via the executor,
        writes the response under a declared output pointer
        (`/nano/<agent_id>` by default), then translates
        `NanoAgentResponse` → `AgentOutcome::Completed` with
        correct microcents conversion (`(cents * 10_000) as i64`).
    -   `SwarmToNano` — wraps a `dyn SwarmAgent` and implements
        `NanoAgentExecutor` for callers (CLI / FFI / MCP) that
        speak the Mode A/B/C runtime contract but want to drive
        a swarm node directly.  Synthesises a one-agent DAG on
        the fly and runs it through `SwarmOrchestrator`.
2.  A **synthetic catalogue back-fill** : every existing
    pre-analysis agent gets a published `NanoAgent` descriptor
    in `efe-nano-preanalysis/src/catalog.rs`.  Six catalogue
    entries land, each with its real `execution_mode`,
    `input_schema`, `output_schema`, `ResourceLimits`,
    `cost_estimate_cents`, `determinism`, `idempotency`,
    `compliance` flags, and a deterministic `NanoAgentId` (e.g.
    `ner-extractor-rs.v1.0.0`).  The pre-analysis crate re-
    exports a `default_catalogue()` returning `Vec<NanoAgent>`.
3.  An updated **`dag_registry`** in `efe-gateway` that no
    longer wires the six structs directly into the DAG ; instead
    it pulls them from the catalogue and wraps each with
    `NanoToSwarm`.  The same DAG topology survives ; the
    dispatch path now flows through `MultiModeRuntime` so every
    N18 / N19 / N12 invariant fires.
4.  A **policy layer** on the adapter : `NanoToSwarmPolicy {
    verify_signature: bool, require_gold_binding: bool,
    enforce_limits: bool, idempotency_cache: Option<Arc<dyn
    IdempotencyCache>> }`.  Defaults : all on for Tier-1
    agents, all off (fast path) for experimental.
5.  **Cost reconciliation** : the adapter writes both the
    Mode-level `cost_cents` (f64) and the swarm-level
    `cost_microcents` (i64) to the blackboard under
    `/nano/<agent_id>/cost`.  The aggregator surfaces the
    reconciled total in `SwarmReport.total_cost_microcents` plus
    a new `cost_estimate_microcents` field built from the
    catalogue entries.
6.  **Determinism replay** : the adapter forwards
    `SwarmAgentContext.rng_seed` to a thread-local RNG honoured
    by Mode A pure-Rust agents (and ignored by stateless agents).
    `DeterminismLevel::FullDeterministic` agents that read RNG
    re-execute bit-identically under the same seed (I13 anchor).
7.  **Audit envelope** : every dispatch emits one
    `nano_envelope_v1` event on the EventBus (publishing
    `agent_id`, `swarm_id`, `mode`, `cost_microcents`,
    `cache_hit`, `signature_verified`, `gold_bound`,
    `trace_id`).  The N14 swarm envelope subsystem consumes
    these envelopes and stamps the Sprint 8 trace.
8.  **HTTP exposure** : the N22 gateway gains one new endpoint
    `GET /v1/nano-catalogue` returning the merged catalogue
    (efe-nano-preanalysis + any tenant-published nano).  No new
    write paths in Sprint 15 ; publication continues to flow
    through N18 (`POST /v1/publishers/<id>/.../nano-publish`).

**What it unlocks downstream.**

- **Compliance** : every pre-analysis dispatch is signed,
  optionally gold-gated, idempotently cached.  Tenants on the
  EU GDPR plan see the same enforcement as bespoke nanos they
  publish themselves.
- **Cost predictability** : `SwarmReport` now carries both
  estimate and actual cost ; the variance is the customer-
  facing SLO.  Drift > 20 % triggers an alert (Sprint 16+).
- **Replay** : Sprint 9's N16 multi-region replay can re-execute
  a swarm bit-identically using the catalogue snapshot +
  blackboard journal — Sprint 15 ships the catalogue snapshot
  side ; the journal lands in Sprint 16 N24.
- **Single execution path** : the meta-swarm proto
  (`efe-meta-swarm-proto`) and the MCP tool layer (M16+) can
  now build orchestrations on **one** trait (`NanoAgent` data
  + `NanoAgentExecutor` dispatch) instead of forking by
  ecosystem.

**Five reference outcomes ship in Sprint 15 N23.**

1. `NanoToSwarm` wraps any `NanoAgent` as a `SwarmAgent` (with
   policy gating, blackboard I/O, cost translation).
2. `SwarmToNano` exposes any `SwarmAgent` through the
   `NanoAgentExecutor` trait.
3. Six pre-analysis catalogue entries published, signed by the
   built-in EFE publisher.
4. `dag_registry.default_dag()` rebuilt on top of the
   catalogue + adapter (no behavioural change for callers, full
   compliance audit on the wire).
5. `GET /v1/nano-catalogue` endpoint live, OpenAPI 3.1 updated,
   audit_n23 + phase_8_n23 tests green (target : 30+
   acceptance, 50+ unit).

---

## 1. Context, motivation, sprint placement

### 1.1 Where N23 lands in the M50 timeline

```
Sprint 8  N11..N15  Firecracker pool / cache hardening / shadow-canary
Sprint 9  N16,N17   Multi-region replay / LLM provenance
Sprint 10 N18       Publisher identity (Ed25519 + KYC + sanctions)
Sprint 11 N19       Gold-test gate Tier-1 promotion
Sprint 12 N20       Postgres real embeddings (M33 pivot)
Sprint 13 N21       Swarm orchestration + Agentic Bridge (Claude)
Sprint 14 N22       HTTP gateway exposure (axum 0.7, RFC 7807)
Sprint 15 N23  <<<  THIS — NanoAgent ↔ SwarmAgent adapter + reuse
Sprint 16 N24       DAG registry + blackboard journal (replay anchor)
Sprint 17 N25       Multi-region nano federation
```

N23 is the **final convergence sprint** for the Phase-7-to-8
transition.  It re-aligns the two ecosystems that diverged when
N21 introduced `SwarmAgent` without back-filling the N18-N20
compliance commitments onto the swarm dispatch path.  Every
subsequent sprint (N24+) assumes one execution path : the
adapter is the contract.

### 1.2 What the Sprint 14 retro called out

The N22 chirurgical audit closed by listing four open items :

1. **Compliance gap** — `POST /v1/swarms/run` returns a `SwarmReport`
   that the EU compliance officer cannot sign off on : no signed
   provenance per agent, no gold binding proof, no idempotency
   cache hits, no resource-limit witness.
2. **Cost gap** — `total_cost_microcents` is post-hoc ; there is
   no `total_cost_estimate_microcents` for the customer-facing
   SLO and no per-agent variance.
3. **Determinism gap** — `rng_seed` flows into
   `SwarmAgentContext` but the pre-analysis agents do not honour
   it (they are stateless or use `rand::thread_rng()` ad-hoc).
4. **Two-path drift** — onboarding a new pre-analysis agent
   requires writing **two** files : a `SwarmAgent` struct and
   (separately) a `NanoAgent` catalogue entry.  Forking is
   inevitable.

N23 addresses all four in one converged sprint.

### 1.3 Why the adapter is the right shape (vs the alternative)

Three alternatives were rejected during the design freeze :

- **A1 — Make `SwarmAgent` carry NanoAgent metadata.**  Rejected.
  It pollutes the swarm trait with N18/N19 concerns that 80 %
  of swarm agents (the agentic ones, not pre-analysis) do not
  need.  Pushes complexity to every implementer.
- **A2 — Make `NanoAgent` carry blackboard awareness.**
  Rejected.  Pulls the swarm runtime into the SDK layer.
  Couples the catalogue (intended to be portable + signable)
  to a specific orchestrator.
- **A3 — Drop `SwarmAgent` and run everything through
  `NanoAgentExecutor`.**  Rejected (for now).  The DAG needs a
  context-rich trait (blackboard, event bus).  The Runtime
  trait is intentionally context-poor.

The adapter approach (the canonical solution in production
distributed systems : think *gRPC ↔ JSON-RPC bridges*) keeps
each trait single-purpose and pushes the translation to one
crate (`efe-nano-swarm-adapter`) audited end-to-end.

### 1.4 Stakeholders

| Role                       | Concern                                    | Mitigation                          |
|----------------------------|--------------------------------------------|-------------------------------------|
| Customer (compliance)      | "Was this dispatch signed + gold-bound ?"  | G2, G4 ; per-agent envelope event   |
| Customer (finance)         | "What did this swarm actually cost vs the quote ?" | G6 ; cost reconciliation field      |
| Platform SRE               | "Can I rerun a 3-week-old swarm and get the same bytes ?" | G7 ; deterministic seed forwarding  |
| Developer (onboarding new agent) | "Where do I put the schema + the run() ?" | G3 ; single file per agent          |
| Auditor                    | "Show me the per-agent dispatch trail."    | G8 ; nano_envelope_v1 events        |
| Tenant (publisher of nanos)| "Can I publish a nano and have it run in any swarm ?" | G1, G3 ; one canonical path         |

---

## 2. Twelve incident patterns N23 closes (P1–P12)

These are not hypothetical : each refers to a real incident
class observed during the Sprint 12-14 chaos / audit passes.

### P1 — Unsigned pre-analysis dispatch shipped to prod

During an EU compliance dry-run on 2026-05-11 the auditor asked
for the Ed25519 signature of the `NerExtractor` invocation that
redacted a SAR.  The N21 swarm code path never verified one ;
the customer had to be told that pre-analysis runs outside the
N18 control plane.  **Fix** : adapter G2 verifies the
catalogue signature on every dispatch when policy demands it.

### P2 — Gold-set drift undetected

A regression in the `SentimentClassifier` (a Mode-B local
model) shipped to Tier-1 because the swarm dispatch path
bypassed N19.  Caught only by a customer escalation 7 days
later.  **Fix** : adapter G4 enforces the gold binding on every
dispatch tier-gated as Tier-1+.

### P3 — Idempotency cache cold for nano-style work

The N12 cache (Mode A pure-Rust) saw 0 hits for pre-analysis
agents because they never flowed through `MultiModeRuntime`.
**Fix** : adapter G5 routes through the executor, which routes
through the cache.

### P4 — Resource limit violations only surfaced as timeouts

A buggy pre-analysis agent allocated 8 GB ; the swarm's per-
agent `tokio::time::timeout` fired, but the system was already
OOM-pressured.  **Fix** : adapter G9 forwards
`ResourceLimits` to the executor where Mode A's thread pool
enforces a soft memory ceiling.

### P5 — Catalogue-vs-runtime version drift

Tenant-published nano `v1.4.2` was registered in the
catalogue ; the swarm DAG referenced a hard-coded `v1.3.0`.
The two co-existed silently.  **Fix** : G10 makes the DAG
reference the catalogue by `NanoAgentId + version` and resolves
once at build time ; subsequent catalogue rotations require
explicit DAG re-publication.

### P6 — Two-path drift on new agent onboarding

A junior added a new pre-analysis agent (`LegalCitationParser`)
only as a `SwarmAgent` ; it never reached the catalogue, never
got signed, and could not be invoked by tenants via the MCP
layer.  **Fix** : G3 — single canonical addition pattern.

### P7 — Cost estimate vs actual diverged silently

A Mode-C agent burned $40 of Claude tokens against a
`cost_estimate_cents = 0.50`.  No one noticed until the
month-end finance reconciliation.  **Fix** : G6 — the
adapter publishes both numbers, the aggregator carries the
variance, the gateway exposes both in `SwarmReport`.

### P8 — Non-deterministic replay broke regression suite

A multi-region replay failed because `NerExtractor` used
`rand::thread_rng()` for tie-breaking ; the seed forwarded by
the swarm context was ignored.  **Fix** : G7 — adapter exposes
`rng_seed` to Mode A handlers via a thread-local context,
documented in the migration guide.

### P9 — Audit envelope missing on pre-analysis runs

The N14 swarm envelope subsystem tagged only the agentic
bridge call, not the upstream pre-analysis chain.  Auditors
saw a partial trace.  **Fix** : G8 — every adapter dispatch
emits a `nano_envelope_v1` event consumed by N14.

### P10 — MCP server could not expose pre-analysis agents

The M16 MCP layer exposes `NanoAgent` entries as MCP tools.
Pre-analysis agents were invisible.  **Fix** : G1 + G3 ; the
catalogue back-fill makes them MCP-discoverable.

### P11 — Tenant could not run a published nano in a swarm

A tenant published a nano via N18 ; calling `POST
/v1/swarms/run` with `dag_id: "<custom>"` failed because no
adapter could wrap a `NanoAgent` into a `SwarmAgent`.  **Fix** :
G1 — `NanoToSwarm` makes any catalogue entry runnable.

### P12 — CLI / FFI callers wanted Runtime, not Swarm

The CLI tool `efe nano run <id>` speaks the `Runtime` trait
but the user wanted to invoke `SummaryWriter` (a `SwarmAgent`).
**Fix** : G11 — `SwarmToNano` reverse adapter.

---

## 3. Commitments — what ships in Sprint 15 (G1–G15)

### G1 — `NanoToSwarm` adapter

Wraps `(Arc<NanoAgent>, Arc<dyn NanoAgentExecutor>)` and
implements `SwarmAgent`.  Configurable input pointer
(default `/input/text`), output pointer (default
`/nano/<agent_id>`), and policy (signature / gold / limits /
idempotency cache).

### G2 — Signature verification on dispatch

If `policy.verify_signature == true` and
`agent.signature.is_some()`, the adapter verifies the Ed25519
signature against the canonical body **before** dispatch.
Failure → `AgentOutcome::Errored { kind: SignatureInvalid, .. }`.

### G3 — Catalogue-first onboarding

A new pre-analysis agent is added by :
1. Writing one file `crates/efe-nano-preanalysis/src/<name>.rs`
   that ships a `pub fn nano_descriptor() -> NanoAgent` *and* a
   `pub fn nano_runner() -> Arc<dyn NanoAgentExecutor>` (or
   re-uses `MultiModeRuntime`'s default dispatch).
2. Adding one line to `default_catalogue()`.
The adapter does the rest.  No new `SwarmAgent` impl per agent.

### G4 — Gold binding enforcement

If `policy.require_gold_binding == true` and the catalogue
entry has `gold_set_ref: Some(_)`, the adapter calls
`GoldApi::lookup_gold_set` and checks status is `Approved`.
Failure → `AgentOutcome::Errored { kind: GoldBindingMissing, .. }`.

### G5 — Idempotency cache integration

If `policy.idempotency_cache.is_some()`, the adapter hashes
`(agent_id, version, input)` (N12 hardened key), consults the
cache, returns the cached `NanoAgentResponse` on hit, otherwise
dispatches and stores the result with the configured TTL.
Cache hit → `AgentOutcome::Completed { ..., cache_hit: true }`
exposed via blackboard.

### G6 — Cost reconciliation

The adapter writes both `cost_estimate_microcents` (from the
catalogue) and `cost_actual_microcents` (from the dispatch) to
the blackboard under `/nano/<agent_id>/cost`.  The aggregator
surfaces both in `SwarmReport` and a per-agent variance field.

### G7 — Deterministic seed forwarding

`SwarmAgentContext.rng_seed` flows into a thread-local visible
to Mode-A handlers via `efe_nano::runtime::context::current_seed()`.
Mode-B / Mode-C handlers receive it via an opaque `CallContext`
field (Sprint 16+ honours it ; Sprint 15 documents only).

### G8 — `nano_envelope_v1` event emission

Every dispatch emits one event on the swarm EventBus with the
topic `nano.envelope.v1` carrying : `agent_id`, `agent_version`,
`swarm_id`, `mode`, `cost_actual_microcents`,
`cost_estimate_microcents`, `cache_hit`, `signature_verified`,
`gold_bound`, `trace_id`, `latency_ms`.  Consumed by N14.

### G9 — Resource limit forwarding

The adapter passes `agent.limits` to the `NanoAgentExecutor`
which Mode A enforces via thread pool budgets + `tokio::time::
timeout(agent.limits.wall_clock_ms)`.  Sprint 15 ships the
wiring ; the memory budget enforcement is best-effort.

### G10 — Catalogue → DAG resolution at build time

`SwarmDagBuilder::add_nano(catalogue_entry: Arc<NanoAgent>)`
introduces the adapter automatically.  The DAG records the
catalogue `NanoAgentId + version` so replay can fail loudly if
the catalogue rotates.

### G11 — `SwarmToNano` reverse adapter

Implements `NanoAgentExecutor` over `Arc<dyn SwarmAgent>` by
spinning a single-node DAG and running it through
`SwarmOrchestrator`.  Used by CLI / MCP / FFI callers.

### G12 — Six pre-analysis catalogue entries

`ner-extractor.v1.0.0`, `language-detector.v1.0.0`,
`schema-validator.v1.0.0`, `pii-redactor.v1.0.0`,
`sentiment-classifier.v1.0.0`, `summary-writer.v1.0.0`.
Each signed by the built-in EFE publisher.  Schemas extracted
from each agent's current internals.

### G13 — `GET /v1/nano-catalogue` HTTP endpoint

Returns the merged catalogue (built-in + tenant-published).
Tenant entries scoped by `jwt.tenant_id`.  OpenAPI 3.1
descriptor updated.  Response is a single JSON document, max
1 MB.  Sprint 16+ paginates.

### G14 — Adapter test crate

`efe-nano-swarm-adapter/tests/audit_n23_*.rs`,
`phase_8_n23_*.rs`, `chaos_n23_*.rs` (style sister N18 / N19 /
N22).  Target : 30 acceptance + 50 unit + 12 chaos.

### G15 — Migration guide

`docs/migration/N23_NANO_SWARM_ADAPTER.md` documents :
- How `dag_registry` changes (the wiring is now catalogue-
  driven).
- How a tenant adds a new pre-analysis agent.
- How to opt out of signature verification (tier-experimental
  agents).
- How to honour `rng_seed` in a Mode-A handler.

---

## 4. Invariants — non-negotiables (I1–I20)

Every commit on this branch is rejected by CI if any invariant
below would be violated.  Each invariant has a corresponding
acceptance test in `audit_n23_*.rs`.

| #   | Invariant                                                                                          |
|-----|----------------------------------------------------------------------------------------------------|
| I1  | One canonical execution path per dispatch : `NanoToSwarm::run → executor.execute_with_context`.    |
| I2  | The adapter NEVER bypasses the executor — no inline shortcut to a stateful agent.                  |
| I3  | The catalogue `NanoAgent` is canonical hashed (Sprint 10 N18) ; the hash is part of the envelope. |
| I4  | Signature verification fails closed : missing signature when policy demands → `Errored`.           |
| I5  | Gold binding fails closed : missing approved gold set when policy demands → `Errored`.             |
| I6  | Cost translation is exact : `(cents * 10_000.0).round() as i64` ; no precision drift.              |
| I7  | The blackboard write under `/nano/<agent_id>` is **versioned** ; replay reads version 1 only.      |
| I8  | The `nano_envelope_v1` event is **always** emitted, even on `Errored` outcomes.                    |
| I9  | The seed forwarded to `current_seed()` is the `SwarmAgentContext.rng_seed`, never random.          |
| I10 | The adapter does NOT take any `tokio::sync::Mutex` lock across `.await` (N20 audit lesson).        |
| I11 | The catalogue is loaded once at `AppState` build time ; later mutations are atomic swaps.          |
| I12 | A DAG referencing a catalogue entry by `(id, version)` is frozen ; rotating the catalogue does NOT mutate the DAG. |
| I13 | `SwarmToNano` synthesises a fresh `SwarmId` per call ; no orchestrator state leaks between calls.  |
| I14 | The HTTP `/v1/nano-catalogue` response is tenant-scoped : the built-in `efe` namespace is always returned ; the `tenant/<uuid>` namespace only when the JWT `tenant_id` matches. |
| I15 | The adapter does NOT add a new Tokio runtime — it is `tokio`-agnostic at the trait surface.        |
| I16 | The adapter is `#![deny(unsafe_code)]` ; the entire crate compiles under `cargo clippy --all-targets --all-features -- -D warnings`. |
| I17 | The adapter does NOT depend on `efe-gateway` ; the dependency arrow is `gateway → adapter`.        |
| I18 | The adapter publishes ONE public type per direction (`NanoToSwarm`, `SwarmToNano`) — no leak of internals. |
| I19 | Every dispatch produces a `trace_id` ; if not provided by the caller, the adapter generates a fresh `Uuid::new_v4()` and writes it to the blackboard. |
| I20 | The synthetic catalogue back-fill is **deterministic** : `default_catalogue()` returns the same bytes across runs. |

---

## 5. Failure modes — what we PROTECT against (F1–F15)

| #   | Failure                                                                  | Mitigation                                |
|-----|--------------------------------------------------------------------------|-------------------------------------------|
| F1  | Catalogue entry signature forged → unauthorised dispatch                 | G2 ; Ed25519 verify on every dispatch     |
| F2  | Gold-set status flipped from `Approved` to `Rejected` mid-flight         | G4 ; live lookup, no cached approval      |
| F3  | Adapter holds the blackboard lock across an LLM call (deadlock)          | I10 ; read+drop pattern                   |
| F4  | Cache poisoning : a malicious response stored under a colliding key      | G5 ; N12 hardened key includes version + canonical hash |
| F5  | Catalogue rotation invalidates an in-flight swarm                        | I12 ; DAG snapshots the `(id, version)`   |
| F6  | A Mode-C handler ignores the wall-clock budget and spends 5 min          | G9 ; outer `tokio::time::timeout`         |
| F7  | A tenant's nano shadows a built-in nano of the same name                 | G13 + I14 ; namespaces (`efe` vs `tenant/<uuid>`) |
| F8  | The reverse adapter (`SwarmToNano`) spawns a new orchestrator per call → resource leak | I13 ; single-shot DAG, drop after report  |
| F9  | The `nano_envelope_v1` event is dropped under EventBus back-pressure     | G8 + I8 ; envelope written to blackboard if EventBus full |
| F10 | A non-deterministic agent silently breaks replay                         | G7 + I9 + I20 ; seed forwarded + catalogue deterministic |
| F11 | The OpenAPI 3.1 spec drifts from the live HTTP endpoint                  | G13 ; spec generated from the same handler |
| F12 | The adapter accidentally calls a panicking handler                       | The N22 CatchPanicLayer wraps it ; the adapter does NOT swallow panics |
| F13 | A renamed `AgentId` breaks an in-flight DAG                              | The adapter derives `AgentId` from `NanoAgentId` ; the derivation is a pure function |
| F14 | Cost overflow on a long swarm (`i64` microcents)                         | Saturating add ; > i64::MAX/2 logs an error and caps |
| F15 | The migration guide is wrong / outdated                                  | The guide is a CI artefact built from a Rust example in `examples/n23_migration_smoke.rs` |

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1 — Crate-level placement

**Decision** : ship `efe-nano-swarm-adapter` as a new
workspace crate, depending on `efe-nano-sdk`,
`efe-swarm-orchestrator`, `efe-blackboard`,
`efe-publisher-identity`, `efe-gold-test-gate`.

**Rejected alternatives** :
- Place the adapter inside `efe-swarm-orchestrator`.
  Rejected : couples the orchestrator to publisher identity +
  gold gate, two heavy dependencies most swarm users do not
  need.
- Place it inside `efe-nano-sdk`. Rejected : pollutes the
  signed-portable SDK with a Tokio runtime + blackboard
  dependency.

**Consequence** : one extra crate, one extra workspace entry,
one extra clippy target.  Worth it for the dependency hygiene.

### ADR-2 — Trait direction of the primary adapter

**Decision** : the primary direction is `NanoAgent → SwarmAgent`
(forward).  The reverse direction (`SwarmAgent → NanoAgent`) is
secondary and ships for CLI / FFI ergonomics.

**Rationale** : the catalogue is the source of truth ; behaviour
is derived.  The swarm orchestrator is a runtime concern that
should consume catalogue entries, not produce them.

### ADR-3 — Policy as an argument, not a global

**Decision** : `NanoToSwarmPolicy` is passed at adapter
construction time, never read from a global or env var inside
the adapter.

**Rationale** : testability + multi-tenancy.  Different tenants
must be able to opt different agents into different policy
modes within the same process.

### ADR-4 — Cost translation rounding

**Decision** : the adapter uses banker's rounding on the
`(cents * 10_000)` conversion :
`((cents * 10_000.0).round() as i64)`.

**Rejected** : truncation (silent loss on long swarms) and
ceiling (overbills customers).

### ADR-5 — Blackboard pointer namespacing

**Decision** : the adapter writes under
`/nano/<agent_id>/<field>` with `field ∈ {output, cost, trace,
mode, cache_hit, signature_verified, gold_bound, latency_ms}`.

**Rejected** : flat keys (`/<agent_id>_output`) — collide with
non-adapter SwarmAgents.  Tree-shape (`/nano/<agent_id>`
holding a single JSON object) — defeats versioned write
semantics.

### ADR-6 — Identifier derivation

**Decision** : `AgentId::from_nano(&nano: NanoAgent) -> AgentId`
is `AgentId::new(format!("{}.{}", nano.id, nano.version)).unwrap()`.
The version is mandatory ; two versions of the same nano in
the same DAG must be addressable.

### ADR-7 — One executor per adapter, not shared

**Decision** : each `NanoToSwarm` owns an `Arc<dyn
NanoAgentExecutor>`.  Multiple adapters in the same DAG share
the same Arc (cheap clone, single dispatch path).

### ADR-8 — Catalogue freezing semantics

**Decision** : `default_catalogue()` returns a `Vec<NanoAgent>`
that is signed at build time using `build.rs` and a baked-in
EFE publisher key.  The signature is checked at `AppState`
construction time ; a tampered binary fails to start.

**Rationale** : protects against supply-chain tampering of the
shipped binary.

### ADR-9 — HTTP catalogue endpoint shape

**Decision** : `GET /v1/nano-catalogue` returns a single JSON
document of shape :

```json
{
  "version": "1.0",
  "generated_at": "<RFC3339>",
  "tenant_id": "<uuid|null>",
  "entries": [ {NanoAgent}, ... ]
}
```

Sprint 16+ adds `?since=<ts>&limit=<n>` pagination.

### ADR-10 — Reverse adapter scope

**Decision** : `SwarmToNano` does NOT expose the full swarm
context to the wrapped agent ; it provides a single-input
single-output bridge with a fresh blackboard scoped to the
call.  Multi-agent compositions must use the forward direction.

**Rationale** : the reverse adapter is for ergonomics, not for
production traffic.  Multi-agent compositions belong in a real
DAG.

---

## 7. Component architecture

### 7.1 Crate dependency graph

```
                ┌─────────────────────────────────┐
                │  efe-gateway (N22)              │
                │  - dag_registry                 │
                │  - GET /v1/nano-catalogue       │
                └────────────┬────────────────────┘
                             │ depends-on
                             ▼
        ┌────────────────────────────────────────────┐
        │  efe-nano-swarm-adapter (N23, NEW)          │
        │                                             │
        │   ┌──────────────┐    ┌───────────────┐    │
        │   │ NanoToSwarm  │    │  SwarmToNano  │    │
        │   └──────┬───────┘    └──────┬────────┘    │
        │          │                    │             │
        │     impl SwarmAgent    impl NanoAgentExecutor │
        └──────────┼────────────────────┼─────────────┘
                   │                    │
   ┌───────────────┼──────────┐    ┌───┼──────────────────┐
   ▼               ▼          ▼    ▼   ▼                  ▼
┌─────────┐ ┌──────────────┐ ┌───────────┐  ┌────────────────┐
│efe-nano-│ │efe-swarm-    │ │efe-       │  │efe-publisher-  │
│sdk      │ │orchestrator  │ │blackboard │  │identity (N18)  │
│(catalog)│ │(SwarmAgent)  │ │(I/O)      │  │(verify)        │
└─────────┘ └──────────────┘ └───────────┘  └────────────────┘
                                              │
                                              ▼
                                      ┌────────────────┐
                                      │efe-gold-test-  │
                                      │gate (N19)      │
                                      └────────────────┘

PLUS, consumers:
   efe-nano-preanalysis  (catalog + runners back-fill)
   efe-nano-inference-daemon (Mode B executor)
   efe-meta-swarm-proto (uses adapter to compose meta-swarms)
   efe-cli  (uses SwarmToNano for `efe nano run <id>`)
   efe-ffi  (re-exports SwarmToNano for foreign hosts)
```

### 7.2 Public crate surface (`efe-nano-swarm-adapter`)

```rust
// SPDX-License-Identifier: UNLICENSED
//! Sprint 15 — N23 : NanoAgent ↔ SwarmAgent bridge.
#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod forward;          // NanoToSwarm
pub mod reverse;          // SwarmToNano
pub mod policy;           // NanoToSwarmPolicy + IdempotencyCache
pub mod cost;             // cents ↔ microcents conversion
pub mod envelope;         // nano_envelope_v1
pub mod error;            // AdapterError taxonomy
pub mod determinism;      // current_seed() thread-local

pub use forward::NanoToSwarm;
pub use reverse::SwarmToNano;
pub use policy::{NanoToSwarmPolicy, IdempotencyCache, InMemoryIdempotencyCache};
pub use cost::{cents_to_microcents, microcents_to_cents};
pub use envelope::NanoEnvelopeV1;
pub use error::AdapterError;
```

### 7.3 Component diagram — single dispatch end-to-end

```
                   ┌─────────────────────┐
                   │  SwarmOrchestrator  │
                   │  .run(dag, input)   │
                   └──────────┬──────────┘
                              │ for each agent in layer
                              ▼
                ┌────────────────────────────────┐
                │  NanoToSwarm::run(ctx)         │
                │  ───────────────────────────── │
                │  1. read input from blackboard │
                │  2. policy.verify_signature?   │
                │     → publisher_api.verify     │
                │  3. policy.require_gold?       │
                │     → gold_api.lookup_status   │
                │  4. policy.idempotency_cache?  │
                │     → cache.get(hardened_key)  │
                │  5. set current_seed(ctx.rng)  │
                │  6. executor.execute_with_ctx  │
                │     ↳ MultiModeRuntime         │
                │       ↳ Mode A | Mode B | Mode C│
                │  7. cents → microcents         │
                │  8. write outputs + cost +     │
                │     metadata to blackboard     │
                │  9. emit nano_envelope_v1      │
                │ 10. return AgentOutcome        │
                └────────────────────────────────┘
                              │
                              ▼
                   ┌─────────────────────┐
                   │ Blackboard          │
                   │  /nano/<id>/output  │
                   │  /nano/<id>/cost    │
                   │  /nano/<id>/trace   │
                   │  /nano/<id>/...     │
                   └─────────────────────┘
                              │
                              ▼
                   ┌─────────────────────┐
                   │ EventBus            │
                   │  nano.envelope.v1   │
                   └─────────────────────┘
```

### 7.4 Reverse adapter — `SwarmToNano`

```
                ┌──────────────────────────────────┐
                │  SwarmToNano::execute_with_ctx   │
                │  ───────────────────────────────│
                │  1. construct single-node DAG    │
                │     with self.wrapped agent      │
                │  2. construct fresh Blackboard,  │
                │     fresh EventBus               │
                │  3. write input @ /input/text    │
                │  4. orchestrator.run(dag, ...)   │
                │  5. read /nano/<id>/output       │
                │  6. translate microcents → cents │
                │  7. return NanoAgentResponse     │
                └──────────────────────────────────┘
```

---

## 8. Module hierarchy & data model

### 8.1 Source tree (post-N23)

```
crates/
├─ efe-nano-swarm-adapter/                              # NEW
│  ├─ Cargo.toml
│  └─ src/
│     ├─ lib.rs
│     ├─ forward/
│     │  ├─ mod.rs
│     │  ├─ nano_to_swarm.rs              # NanoToSwarm impl SwarmAgent
│     │  ├─ output.rs                     # blackboard write helpers
│     │  └─ input.rs                      # blackboard read helpers
│     ├─ reverse/
│     │  ├─ mod.rs
│     │  └─ swarm_to_nano.rs              # SwarmToNano impl NanoAgentExecutor
│     ├─ policy/
│     │  ├─ mod.rs
│     │  ├─ policy_struct.rs              # NanoToSwarmPolicy
│     │  ├─ idempotency.rs                # IdempotencyCache trait + in-mem impl
│     │  ├─ signature_gate.rs             # SignatureGate
│     │  └─ gold_gate.rs                  # GoldBindingGate
│     ├─ cost.rs                          # f64 ↔ i64 microcents
│     ├─ envelope.rs                      # NanoEnvelopeV1 (serde)
│     ├─ determinism.rs                   # thread_local!{ seed }
│     └─ error.rs                         # AdapterError enum
│  ├─ tests/
│  │  ├─ phase_8_n23_nano_to_swarm.rs     # 16 tests
│  │  ├─ phase_8_n23_swarm_to_nano.rs     # 6 tests
│  │  ├─ phase_8_n23_policy.rs            # 8 tests
│  │  ├─ audit_n23_nano_swarm_adapter.rs  # 12 regression tests
│  │  └─ chaos_n23_nano_swarm_adapter.rs  # 8 chaos tests
│  └─ examples/
│     └─ n23_migration_smoke.rs           # used by docs + CI
│
├─ efe-nano-preanalysis/                                # MODIFIED
│  └─ src/
│     ├─ catalog.rs                       # NEW : default_catalogue()
│     ├─ ner_extractor.rs                 # +nano_descriptor()+nano_runner()
│     ├─ language_detector.rs             # idem
│     ├─ schema_validator.rs              # idem
│     ├─ pii_redactor.rs                  # idem
│     ├─ sentiment_classifier.rs          # idem
│     ├─ summary_writer.rs                # idem
│     └─ lib.rs                           # re-export default_catalogue
│
├─ efe-gateway/                                         # MODIFIED
│  └─ src/
│     ├─ dag_registry.rs                  # rewritten on top of adapter
│     ├─ handlers/
│     │  └─ nano_catalogue.rs             # NEW : GET /v1/nano-catalogue
│     └─ router.rs                        # +route
│
└─ docs/migration/
   └─ N23_NANO_SWARM_ADAPTER.md           # NEW
```

### 8.2 Key types — verbatim signatures

```rust
// crates/efe-nano-swarm-adapter/src/forward/nano_to_swarm.rs

use std::sync::Arc;
use async_trait::async_trait;
use efe_blackboard::Blackboard;
use efe_nano_sdk::types::catalog::NanoAgent;
use efe_nano_sdk::traits::runtime::{NanoAgentExecutor, CallContext};
use efe_swarm_orchestrator::agent::{
    SwarmAgent, SwarmAgentContext, AgentOutcome, AgentErrorKind, SwarmAgentError, AgentId,
};

use crate::policy::NanoToSwarmPolicy;
use crate::envelope::NanoEnvelopeV1;
use crate::cost::cents_to_microcents;
use crate::error::AdapterError;

pub struct NanoToSwarm {
    /// The catalogue entry (signed, gold-bound, schema-defined).
    pub nano: Arc<NanoAgent>,
    /// The executor that knows how to dispatch by ExecutionMode.
    pub executor: Arc<dyn NanoAgentExecutor>,
    /// Policy gates (signature / gold / limits / cache).
    pub policy: NanoToSwarmPolicy,
    /// Blackboard input pointer (default "/input/text").
    pub input_pointer: String,
    /// Optional override for the blackboard output prefix.
    /// Default: "/nano/<agent_id>".
    pub output_prefix: Option<String>,
}

impl NanoToSwarm {
    pub fn new(
        nano: Arc<NanoAgent>,
        executor: Arc<dyn NanoAgentExecutor>,
    ) -> Self { /* ... */ }

    pub fn with_policy(mut self, p: NanoToSwarmPolicy) -> Self { /* ... */ }
    pub fn with_input_pointer<S: Into<String>>(mut self, s: S) -> Self { /* ... */ }
    pub fn with_output_prefix<S: Into<String>>(mut self, s: S) -> Self { /* ... */ }

    fn derived_agent_id(&self) -> AgentId { /* AgentId::new(format!("{}.{}", id, version)) */ }
}

#[async_trait]
impl SwarmAgent for NanoToSwarm {
    fn agent_id(&self) -> AgentId {
        self.derived_agent_id()
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        // (see workflows §10.1 for the full body)
    }
}
```

```rust
// crates/efe-nano-swarm-adapter/src/policy/policy_struct.rs

#[derive(Debug, Clone)]
pub struct NanoToSwarmPolicy {
    pub verify_signature: bool,
    pub require_gold_binding: bool,
    pub enforce_limits: bool,
    pub idempotency_cache: Option<Arc<dyn IdempotencyCache>>,
    pub signature_verifier: Option<Arc<dyn PublisherApi>>,
    pub gold_api: Option<Arc<dyn GoldApi>>,
}

impl Default for NanoToSwarmPolicy {
    fn default() -> Self {
        Self {
            verify_signature: false,
            require_gold_binding: false,
            enforce_limits: true,
            idempotency_cache: None,
            signature_verifier: None,
            gold_api: None,
        }
    }
}

impl NanoToSwarmPolicy {
    pub fn tier1_strict(
        verifier: Arc<dyn PublisherApi>,
        gold_api: Arc<dyn GoldApi>,
        cache: Arc<dyn IdempotencyCache>,
    ) -> Self {
        Self {
            verify_signature: true,
            require_gold_binding: true,
            enforce_limits: true,
            idempotency_cache: Some(cache),
            signature_verifier: Some(verifier),
            gold_api: Some(gold_api),
        }
    }

    pub fn experimental_fast() -> Self {
        Self::default()
    }
}
```

```rust
// crates/efe-nano-swarm-adapter/src/policy/idempotency.rs

#[async_trait]
pub trait IdempotencyCache: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Option<NanoAgentResponse>;
    async fn put(&self, key: &str, value: NanoAgentResponse, ttl: Duration);
}

pub struct InMemoryIdempotencyCache {
    map: DashMap<String, (NanoAgentResponse, SystemTime)>,
}
```

```rust
// crates/efe-nano-swarm-adapter/src/envelope.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoEnvelopeV1 {
    pub schema: &'static str,         // "nano.envelope.v1"
    pub agent_id: String,             // derived AgentId
    pub nano_id: String,              // NanoAgentId
    pub nano_version: String,         // semver
    pub swarm_id: Uuid,
    pub trace_id: String,
    pub mode: &'static str,           // "pure-rust" | "local-model" | "llm-augmented"
    pub cost_estimate_microcents: i64,
    pub cost_actual_microcents: i64,
    pub cost_variance_microcents: i64, // actual - estimate
    pub cache_hit: bool,
    pub signature_verified: bool,
    pub gold_bound: bool,
    pub latency_ms: u32,
    pub outcome: NanoEnvelopeOutcome,  // "completed" | "errored" | "timed_out"
    pub emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NanoEnvelopeOutcome {
    Completed,
    Errored,
    TimedOut,
    Skipped,
}
```

```rust
// crates/efe-nano-swarm-adapter/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("blackboard read failed at pointer {0}")]
    BlackboardReadMissing(String),

    #[error("signature verification failed: {0}")]
    SignatureInvalid(String),

    #[error("gold binding missing or not approved: {0}")]
    GoldBindingMissing(String),

    #[error("idempotency cache failure: {0}")]
    CacheError(String),

    #[error("executor dispatch failed: {0}")]
    DispatchFailed(String),

    #[error("blackboard write failed: {0}")]
    BlackboardWriteFailed(String),

    #[error("cost translation overflow: {0}")]
    CostOverflow(String),
}

impl From<AdapterError> for SwarmAgentError {
    fn from(e: AdapterError) -> Self {
        SwarmAgentError::AgentInternal(e.to_string())
    }
}
```

```rust
// crates/efe-nano-swarm-adapter/src/cost.rs

/// Exact, banker's-rounded conversion.
/// `i64` saturates at i64::MAX/2 to leave headroom for swarm
/// aggregation (F14).
pub fn cents_to_microcents(cents: f64) -> i64 {
    let raw = (cents * 10_000.0).round();
    if raw.is_finite() {
        let cap = (i64::MAX / 2) as f64;
        let bounded = raw.clamp(-cap, cap);
        bounded as i64
    } else {
        0
    }
}

pub fn microcents_to_cents(microcents: i64) -> f64 {
    (microcents as f64) / 10_000.0
}
```

```rust
// crates/efe-nano-swarm-adapter/src/determinism.rs

use std::cell::Cell;

thread_local! {
    static SEED: Cell<Option<u64>> = const { Cell::new(None) };
}

pub fn set_seed(s: u64) {
    SEED.with(|c| c.set(Some(s)));
}

pub fn clear_seed() {
    SEED.with(|c| c.set(None));
}

pub fn current_seed() -> Option<u64> {
    SEED.with(|c| c.get())
}

/// RAII guard — restores prior seed on drop.
pub struct SeedGuard {
    prev: Option<u64>,
}

impl SeedGuard {
    pub fn new(seed: u64) -> Self {
        let prev = current_seed();
        set_seed(seed);
        Self { prev }
    }
}

impl Drop for SeedGuard {
    fn drop(&mut self) {
        SEED.with(|c| c.set(self.prev));
    }
}
```

### 8.3 Blackboard layout under `/nano/<agent_id>`

```
/nano/<agent_id>/output                  ← NanoAgentResponse.output (Value)
/nano/<agent_id>/cost                    ← { estimate_microcents, actual_microcents, variance_microcents }
/nano/<agent_id>/trace                   ← TraceId
/nano/<agent_id>/mode                    ← "pure-rust" | "local-model" | "llm-augmented"
/nano/<agent_id>/cache_hit               ← bool
/nano/<agent_id>/signature_verified      ← bool
/nano/<agent_id>/gold_bound              ← bool
/nano/<agent_id>/latency_ms              ← u32
/nano/<agent_id>/envelope                ← full NanoEnvelopeV1 (only if EventBus saturated, F9)
```

---

## 9. API surface (Rust traits + HTTP)

### 9.1 New public Rust API (`efe-nano-swarm-adapter`)

| Item                                  | Where                                | Stability |
|---------------------------------------|--------------------------------------|-----------|
| `struct NanoToSwarm`                  | `forward::nano_to_swarm`             | stable    |
| `impl SwarmAgent for NanoToSwarm`     | idem                                 | stable    |
| `struct SwarmToNano`                  | `reverse::swarm_to_nano`             | stable    |
| `impl NanoAgentExecutor for SwarmToNano` | idem                              | stable    |
| `struct NanoToSwarmPolicy`            | `policy::policy_struct`              | stable    |
| `trait IdempotencyCache`              | `policy::idempotency`                | stable    |
| `struct InMemoryIdempotencyCache`     | idem                                 | stable    |
| `struct NanoEnvelopeV1`               | `envelope`                           | wire-stable |
| `enum NanoEnvelopeOutcome`            | idem                                 | wire-stable |
| `enum AdapterError`                   | `error`                              | stable    |
| `fn cents_to_microcents(f64) -> i64`  | `cost`                               | stable    |
| `fn microcents_to_cents(i64) -> f64`  | idem                                 | stable    |
| `struct SeedGuard`                    | `determinism`                        | stable    |
| `fn current_seed() -> Option<u64>`    | idem                                 | stable    |

### 9.2 New methods on `SwarmDagBuilder` (`efe-swarm-orchestrator`)

```rust
impl SwarmDagBuilder {
    /// Wraps `nano` in a `NanoToSwarm` adapter and adds it as a layer node.
    pub fn add_nano(
        &mut self,
        nano: Arc<NanoAgent>,
        executor: Arc<dyn NanoAgentExecutor>,
        policy: NanoToSwarmPolicy,
    ) -> &mut Self { /* ... */ }

    pub fn add_nano_with_pointers(
        &mut self,
        nano: Arc<NanoAgent>,
        executor: Arc<dyn NanoAgentExecutor>,
        policy: NanoToSwarmPolicy,
        input_pointer: String,
        output_prefix: String,
    ) -> &mut Self { /* ... */ }
}
```

### 9.3 New HTTP endpoint

```
GET /v1/nano-catalogue
    ▸ auth        : Bearer JWT
    ▸ tenant scope: returns "efe" namespace always + "tenant/<tenant_id>"
                    entries when the JWT tenant_id is non-nil
    ▸ response    : 200 application/json
                    body : { version, generated_at, tenant_id, entries[] }
    ▸ errors      : 401 missing/invalid auth
                    503 catalogue snapshot unavailable
```

OpenAPI 3.1 fragment shipped in
`crates/efe-gateway/src/openapi/nano_catalogue.rs`.

### 9.4 New events on the EventBus

| Topic                   | Payload                       | Subscribers      |
|-------------------------|-------------------------------|------------------|
| `nano.envelope.v1`      | `NanoEnvelopeV1`              | N14 envelope, N17 provenance, audit log |
| `nano.cost.variance.v1` | `{ agent_id, variance_microcents, threshold }` | Cost reconciliation alerting (Sprint 16+) |

### 9.5 New catalogue back-fill API (`efe-nano-preanalysis`)

```rust
/// Returns the six built-in pre-analysis agents as NanoAgent
/// descriptors plus their runners.  Deterministic across runs
/// (I20).
pub fn default_catalogue() -> Vec<(Arc<NanoAgent>, Arc<dyn NanoAgentExecutor>)>;

/// Single-agent accessors for tenants that want only one.
pub mod descriptors {
    pub fn ner_extractor() -> Arc<NanoAgent>;
    pub fn language_detector() -> Arc<NanoAgent>;
    pub fn schema_validator() -> Arc<NanoAgent>;
    pub fn pii_redactor() -> Arc<NanoAgent>;
    pub fn sentiment_classifier() -> Arc<NanoAgent>;
    pub fn summary_writer() -> Arc<NanoAgent>;
}

pub mod runners {
    pub fn ner_extractor() -> Arc<dyn NanoAgentExecutor>;
    // ... etc
}
```

---

## 10. End-to-end workflows

### 10.1 Workflow A — `NanoToSwarm::run` body (annotated)

```rust
async fn run(
    &self,
    ctx: SwarmAgentContext,
) -> Result<AgentOutcome, SwarmAgentError> {
    let started = std::time::Instant::now();
    let trace_id = generate_or_inherit_trace_id(&ctx);
    let agent_id_str = self.derived_agent_id().to_string();
    let prefix = self
        .output_prefix
        .clone()
        .unwrap_or_else(|| format!("/nano/{agent_id_str}"));

    // (1) Read input from blackboard.
    let input_value = ctx
        .blackboard
        .read(&self.input_pointer)
        .map(|bv| bv.value)
        .ok_or_else(|| {
            AdapterError::BlackboardReadMissing(self.input_pointer.clone())
        })?;

    // (2) Signature verification (G2 / I4).
    let mut signature_verified = false;
    if self.policy.verify_signature {
        match (&self.nano.signature, &self.policy.signature_verifier) {
            (Some(sig), Some(verifier)) => {
                verifier
                    .verify_nano_signature(&self.nano, sig)
                    .await
                    .map_err(|e| AdapterError::SignatureInvalid(e.to_string()))?;
                signature_verified = true;
            }
            _ => {
                return Ok(errored_outcome(
                    AgentErrorKind::PreconditionFailed,
                    "signature required but missing or verifier not configured",
                    started,
                ));
            }
        }
    }

    // (3) Gold binding (G4 / I5).
    let mut gold_bound = false;
    if self.policy.require_gold_binding {
        match (&self.nano.gold_set_ref, &self.policy.gold_api) {
            (Some(g_ref), Some(api)) => {
                let status = api
                    .lookup_gold_set_status(g_ref)
                    .await
                    .map_err(|e| AdapterError::GoldBindingMissing(e.to_string()))?;
                if status != GoldSetStatus::Approved {
                    return Ok(errored_outcome(
                        AgentErrorKind::PreconditionFailed,
                        "gold binding not approved",
                        started,
                    ));
                }
                gold_bound = true;
            }
            _ => {
                return Ok(errored_outcome(
                    AgentErrorKind::PreconditionFailed,
                    "gold binding required but missing or api not configured",
                    started,
                ));
            }
        }
    }

    // (4) Idempotency cache lookup (G5).
    let cache_key = build_cache_key(&self.nano, &input_value);
    let mut cache_hit = false;
    let nano_response = if let Some(cache) = &self.policy.idempotency_cache {
        if let Some(cached) = cache.get(&cache_key).await {
            cache_hit = true;
            cached
        } else {
            // (5) Set seed for Mode A determinism (G7 / I9).
            let _guard = crate::determinism::SeedGuard::new(ctx.rng_seed);
            let call_ctx = CallContext::default()
                .with_trace_id(trace_id.clone())
                .with_tenant_id(ctx.swarm_id.into_uuid());
            // (6) Dispatch.
            let resp = self
                .executor
                .execute_with_context(&self.nano, input_value.clone(), &call_ctx)
                .await
                .map_err(|e| AdapterError::DispatchFailed(e.to_string()))?;
            cache.put(&cache_key, resp.clone(), Duration::from_secs(ttl_for(&self.nano))).await;
            resp
        }
    } else {
        let _guard = crate::determinism::SeedGuard::new(ctx.rng_seed);
        let call_ctx = CallContext::default()
            .with_trace_id(trace_id.clone())
            .with_tenant_id(ctx.swarm_id.into_uuid());
        self
            .executor
            .execute_with_context(&self.nano, input_value.clone(), &call_ctx)
            .await
            .map_err(|e| AdapterError::DispatchFailed(e.to_string()))?
    };

    let latency_ms = started.elapsed().as_millis() as u32;

    // (7) Cost translation (G6 / I6).
    let estimate_microcents = cents_to_microcents(self.nano.cost_estimate_cents);
    let actual_microcents = cents_to_microcents(nano_response.cost_cents);
    let variance = actual_microcents.saturating_sub(estimate_microcents);

    // (8) Blackboard writes (I7).
    let writes = write_outputs_to_blackboard(
        &ctx.blackboard,
        &prefix,
        &ctx.agent_id,
        &nano_response,
        estimate_microcents,
        actual_microcents,
        variance,
        cache_hit,
        signature_verified,
        gold_bound,
        latency_ms,
        &trace_id,
    )?;

    // (9) Envelope emission (G8 / I8 / F9).
    let envelope = NanoEnvelopeV1 {
        schema: "nano.envelope.v1",
        agent_id: ctx.agent_id.to_string(),
        nano_id: self.nano.id.to_string(),
        nano_version: self.nano.version.to_string(),
        swarm_id: ctx.swarm_id.into_uuid(),
        trace_id: trace_id.clone(),
        mode: mode_label(&self.nano.execution_mode),
        cost_estimate_microcents: estimate_microcents,
        cost_actual_microcents: actual_microcents,
        cost_variance_microcents: variance,
        cache_hit,
        signature_verified,
        gold_bound,
        latency_ms,
        outcome: NanoEnvelopeOutcome::Completed,
        emitted_at: chrono::Utc::now(),
    };
    let _published = ctx.eventbus.publish("nano.envelope.v1", &envelope);
    // F9 : if publish returns NoSubscribers OR Lagged, the envelope
    // is also persisted to /nano/<id>/envelope.

    // (10) Return outcome.
    Ok(AgentOutcome::Completed {
        cost_microcents: actual_microcents,
        duration_ms: latency_ms,
        blackboard_writes: writes,
        events_published: 1,
    })
}
```

### 10.2 Workflow B — `SwarmToNano::execute_with_context` body

```rust
async fn execute_with_context(
    &self,
    _agent: &NanoAgent,
    input: serde_json::Value,
    ctx: &CallContext,
) -> Result<NanoAgentResponse, NanoAgentError> {
    let started = std::time::Instant::now();

    // (1) Build a single-node DAG containing self.wrapped.
    let mut builder = SwarmDagBuilder::new();
    builder.add_agent(AgentNode::new(self.wrapped.clone()));
    builder.with_aggregator(NoOpAggregator::id());
    builder.add_agent(AgentNode::new(Arc::new(NoOpAggregator)));
    builder.add_edge(DagEdge {
        from: self.wrapped.agent_id(),
        to: NoOpAggregator::id(),
    });
    let dag = builder.build()
        .map_err(|e| NanoAgentError::Backend(format!("dag build : {e}")))?;

    // (2) Fresh swarm run config — single agent, no envelope.
    let config = SwarmRunConfig {
        global_timeout_ms: 30_000,
        emit_envelope: false,
        rng_seed: ctx.rng_seed.unwrap_or_else(|| rand::random()),
        tenant_tier_label: "swarm-to-nano".into(),
        swarm_id: SwarmId::new(),
        dag_id_label: "swarm-to-nano-single".into(),
    };

    // (3) Run the orchestrator.
    let report = self
        .orchestrator
        .run(&dag, input, config)
        .await
        .map_err(|e| NanoAgentError::Backend(format!("swarm run : {e}")))?;

    // (4) Extract the output from the blackboard.
    let agent_id_str = self.wrapped.agent_id().to_string();
    let output = report
        .blackboard_view
        .read(&format!("/nano/{agent_id_str}/output"))
        .map(|bv| bv.value)
        .unwrap_or(serde_json::Value::Null);

    // (5) Build the NanoAgentResponse.
    Ok(NanoAgentResponse {
        output,
        latency_ms: started.elapsed().as_millis() as u32,
        cost_cents: microcents_to_cents(report.total_cost_microcents),
        trace_id: ctx.trace_id.clone().unwrap_or_default(),
        cache_hit: false,
    })
}
```

### 10.3 Workflow C — Gateway `GET /v1/nano-catalogue`

```
1.  jwt extractor (N22) → Claims { tenant_id }
2.  handler reads:
       a. state.catalogue.efe_builtin()  (the 6 pre-analysis + future)
       b. state.catalogue.tenant(tenant_id) when tenant_id ≠ nil
3.  merge into NanoCatalogueResponse {
       version: "1.0",
       generated_at: chrono::Utc::now().to_rfc3339(),
       tenant_id: Option<Uuid>,
       entries: Vec<NanoAgent>,
    }
4.  serialize → JSON, set Content-Type
5.  return 200
6.  failure : 503 with problem-details (NanoCatalogueSnapshotUnavailable)
```

### 10.4 Workflow D — Migration of `dag_registry.default_dag()`

```diff
- builder.add_agent(AgentNode::new(Arc::new(NerExtractor::new())));
- builder.add_agent(AgentNode::new(Arc::new(LanguageDetector::new())));
- builder.add_agent(AgentNode::new(Arc::new(SchemaValidator::new(...))));
- builder.add_agent(AgentNode::new(Arc::new(SentimentClassifier::new())));
- builder.add_agent(AgentNode::new(Arc::new(PiiRedactor::new())));
- builder.add_agent(AgentNode::new(Arc::new(SummaryWriter::new())));
+ let policy = NanoToSwarmPolicy::tier1_strict(
+     state.publisher_api.clone(),
+     state.gold_api.clone(),
+     state.idempotency_cache.clone(),
+ );
+ for (nano, runner) in efe_nano_preanalysis::default_catalogue() {
+     builder.add_nano(nano, runner, policy.clone());
+ }
```

The DAG topology (layers + edges + aggregator) is unchanged.
The dispatch path now flows through `MultiModeRuntime` ; N18 /
N19 / N12 / N14 / N17 all fire.

### 10.5 Workflow E — Tenant publishes a new nano and runs it in a swarm

```
1. Tenant signs nano body with N18 publisher key.
2. POST /v1/publishers/<id>/publish-nano (Sprint 15+ endpoint, deferred)
   ◦ N18 verifies signature + KYC + sanctions.
   ◦ Tenant nano lands in tenant catalogue.
3. Tenant calls GET /v1/nano-catalogue → sees their nano.
4. Tenant constructs a DAG (POST /v1/dags, Sprint 16 N24) referencing
   the new NanoAgentId.
5. POST /v1/swarms/run with dag_id → adapter wraps the published
   nano + runs it ; signature + gold binding fire ; envelope emitted.
```

---

## 11. Five-day implementation plan

### Day 1 — Crate skeleton + types + tests harness

- Scaffold `crates/efe-nano-swarm-adapter/` :
  - `Cargo.toml` (deps : tokio, async-trait, serde, thiserror,
    dashmap, uuid, chrono, tracing, efe-nano-sdk, efe-swarm-
    orchestrator, efe-blackboard, efe-publisher-identity, efe-
    gold-test-gate).
  - `src/lib.rs` with module declarations + `#![deny(unsafe_code)]`.
  - `src/cost.rs` + tests (round-trip, overflow, NaN).
  - `src/determinism.rs` + tests (thread-local, RAII guard).
  - `src/error.rs` + tests (`From<AdapterError> for SwarmAgentError`).
  - `src/envelope.rs` + tests (serde round-trip, schema field).
  - `src/policy/idempotency.rs` + `InMemoryIdempotencyCache` +
    tests (get/put/TTL expiry).
- Add the crate to `Cargo.toml` workspace members.
- Run `cargo build -p efe-nano-swarm-adapter`.
- Run `cargo clippy -p efe-nano-swarm-adapter --all-targets -- -D warnings`.
- **Exit criteria** : crate compiles + 18 unit tests pass + clippy
  strict clean.

### Day 2 — `NanoToSwarm` forward adapter

- `src/forward/input.rs` — blackboard read helper with typed errors.
- `src/forward/output.rs` — multi-pointer write helper, idempotent
  re-writes return new version each (I7).
- `src/policy/signature_gate.rs` — wraps `PublisherApi::verify_nano_signature`
  (Sprint 15 adds the method to N18 trait if missing — minor patch).
- `src/policy/gold_gate.rs` — wraps `GoldApi::lookup_gold_set_status`.
- `src/forward/nano_to_swarm.rs` — the full `impl SwarmAgent for
  NanoToSwarm` per §10.1.
- Tests :
  - `phase_8_n23_nano_to_swarm.rs` (16 tests) :
    1. happy-path Mode A dispatch
    2. signature verification on
    3. signature verification fails closed when policy on but
       agent unsigned
    4. gold binding on + approved
    5. gold binding required but missing → Errored
    6. idempotency cache miss → put
    7. idempotency cache hit → return cached, no dispatch
    8. cost translation exact for typical f64
    9. cost translation overflow saturates
    10. blackboard write at custom output_prefix
    11. blackboard read at custom input_pointer
    12. seed forwarding visible via current_seed()
    13. envelope emitted on success
    14. envelope emitted on errored
    15. envelope persisted to blackboard when EventBus full
    16. concurrent dispatch (50 parallel) — no shared lock held
        across .await
- **Exit criteria** : 16 phase_8 tests pass + clippy strict clean.

### Day 3 — `SwarmToNano` reverse adapter + catalogue back-fill

- `src/reverse/swarm_to_nano.rs` — per §10.2.
- Tests `phase_8_n23_swarm_to_nano.rs` (6 tests) :
  1. wrap a stateless SwarmAgent → execute → response
  2. cost reverse translation (microcents → cents)
  3. trace_id propagated via CallContext
  4. fresh blackboard per call (no leak)
  5. fresh swarm_id per call
  6. orchestrator drop after report (no leaked task)
- Catalogue back-fill in `efe-nano-preanalysis` :
  - `src/catalog.rs` — `default_catalogue()` builds the 6
    entries.  Each `NanoAgent` signed by a baked EFE-built-in
    publisher key (sprint 15 ships the dev key ; prod key
    rotates at GA).
  - `src/<each>.rs` gets `pub fn nano_descriptor()` +
    `pub fn nano_runner()`.
  - Determinism : `default_catalogue()` returns the same bytes
    across runs (I20 — checked by a hash test).
- **Exit criteria** : 6 reverse-adapter tests + 6 catalogue
  back-fill tests pass.

### Day 4 — Gateway integration + HTTP endpoint

- `efe-gateway/Cargo.toml` : add `efe-nano-swarm-adapter` +
  re-export `default_catalogue` from `efe-nano-preanalysis`.
- `efe-gateway/src/state.rs` : add `catalogue: Arc<NanoCatalogue>`
  and `idempotency_cache: Arc<dyn IdempotencyCache>` fields ;
  builder methods.
- `efe-gateway/src/dag_registry.rs` : rewrite per §10.4. The
  6 hard-coded `add_agent` calls become a loop over
  `default_catalogue()` wrapped through `NanoToSwarm`.
- `efe-gateway/src/handlers/nano_catalogue.rs` : new handler
  for `GET /v1/nano-catalogue`.
- `efe-gateway/src/router.rs` : `.route("/v1/nano-catalogue",
  get(handlers::nano_catalogue_handler))`.
- `efe-gateway/src/openapi/...` : add path entry.
- Tests `phase_8_n23_catalogue_http.rs` (5 tests) :
  1. GET /v1/nano-catalogue without JWT → 401
  2. GET with valid JWT → 200 + six entries
  3. response shape matches OpenAPI schema
  4. tenant_id from JWT shows in response
  5. ContextBundle from a default DAG run mentions all six
     adapted agents
- **Exit criteria** : gateway compiles + workspace tests green +
  N22 acceptance tests still pass + 5 new endpoint tests pass.

### Day 5 — Audit + chaos + migration guide + smoke

- `audit_n23_nano_swarm_adapter.rs` (12 tests pinning each
  invariant I1..I20 that's externally observable) :
  - I3 catalogue canonical hash stable
  - I6 cost translation precision (f64::EPSILON)
  - I7 blackboard write version monotonic
  - I8 envelope emitted on Errored
  - I10 no Mutex across .await (compile + test-time async fence)
  - I12 DAG snapshot includes (id, version)
  - I13 SwarmToNano fresh swarm_id
  - I14 catalogue tenant scoping
  - I16 clippy -D warnings clean
  - I18 only two public adapter types
  - I20 default_catalogue determinism (hash)
  - meta-invariant : every adapter dispatch ends with a
    Completed | Errored | TimedOut outcome — never panics.
- `chaos_n23_nano_swarm_adapter.rs` (8 tests) :
  - F1 forged signature rejected
  - F2 gold set flipped Approved → Rejected mid-flight rejected
  - F3 simulated lock contention does not deadlock
  - F4 cache-key collision attempt detected
  - F6 wall-clock budget exceeded → AgentOutcome::TimedOut
  - F8 SwarmToNano under 100 parallel calls — no orchestrator
    leak (verified via `tokio::task::yield_now` + memory snapshot)
  - F9 EventBus full → envelope still persisted to blackboard
  - F14 cost overflow saturates without panic
- `docs/migration/N23_NANO_SWARM_ADAPTER.md` :
  - "What changed" section.
  - "How to add a new pre-analysis agent" walkthrough.
  - "How to opt out of strict policy" snippet.
  - "How to honour rng_seed in your Mode-A handler" code sample
    using `current_seed()`.
  - "Compatibility window" : the old direct-SwarmAgent path is
    deprecated, removed in Sprint 16 N24.
- `examples/n23_migration_smoke.rs` (CI-built example) :
  - constructs a fake catalogue
  - wraps each into `NanoToSwarm`
  - runs a 3-agent DAG
  - asserts the envelope events appear on the bus
  - asserts the cost reconciliation is non-zero
- **Exit criteria** :
  - `cargo test --workspace` green (target : prior 2513 PASSED
    + new tests, expected 2570+ PASSED, 0 FAILED).
  - `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - Live smoke against built gateway : `curl ${BASE}/v1/nano-
    catalogue -H 'Authorization: Bearer …'` returns 200 with 6
    entries.

---

## 12. Test strategy

### 12.1 Test pyramid

```
                     ▲
                    /│\
                   / │ \                Chaos (8)
                  /  │  \              Audit (12)
                 / Adp│ter\           Phase 8 acceptance (33)
                /────┼────\          Unit (38)
               /  Cross─crate \       (catalogue back-fill,
              /─────────┼──────\       gateway, env)
             /                  \
            /────────────────────\
```

- **Unit (38)** — `cost`, `determinism`, `envelope` serde,
  `error` From impls, `policy::idempotency` in-mem TTL.
- **Acceptance (33)** — `phase_8_n23_*.rs` exercise
  `NanoToSwarm`, `SwarmToNano`, gateway endpoint, catalogue
  back-fill, dag_registry rewrite end-to-end.
- **Audit (12)** — `audit_n23_*.rs` pin each externally-
  observable invariant.
- **Chaos (8)** — `chaos_n23_*.rs` fuzz signature/gold/limit/
  cache failure modes.
- **Cross-crate smoke (1)** — `examples/n23_migration_smoke.rs`
  built by CI as a regression.

### 12.2 Property tests

- `cost`: `(x: f64) → microcents_to_cents(cents_to_microcents(x)) ≈ x`
  within `f64::EPSILON * 10_000.0` for `|x| < 1_000_000.0`.
- `envelope`: serde round-trip exact for any random
  `NanoEnvelopeV1`.

### 12.3 Mutation testing surface

`cargo mutants -p efe-nano-swarm-adapter --in-place` target :

- mutate `cents_to_microcents` (assert any sign/scale change
  caught by tests).
- mutate the policy short-circuits (assert signature-off path
  still tests negative case).
- mutate the envelope outcome enum (assert serde stays stable).

### 12.4 Coverage budget

- `cost.rs` — 100 % branch.
- `determinism.rs` — 100 % branch.
- `forward/nano_to_swarm.rs` — ≥ 95 % branch (the 4 policy gates × 2 outcomes × cache hit/miss = 16 paths).
- `reverse/swarm_to_nano.rs` — ≥ 90 % branch.

---

## 13. Cross-cutting alignment (N11 → N22)

| Sprint | Subsystem                         | How N23 uses it                                     |
|--------|-----------------------------------|------------------------------------------------------|
| N11    | Firecracker pool (Mode B)         | `MultiModeRuntime::mode_b` reached unchanged ; the adapter forwards through `execute_with_context` ; thread-local seed is read on the host. |
| N12    | Cache-key hardening               | The hardened key shape `(agent_id, version, canonical_hash, input_hash)` is reused for `IdempotencyCache::get/put`. |
| N13    | Shadow-canary catalog             | Catalogue back-fill (G12) lands six entries in the canary path automatically ; canary verdicts surface in the envelope. |
| N14    | Swarm envelope trace              | `nano_envelope_v1` is a sibling schema ; N14 subscribes and stamps the parent `trace_id`. |
| N15    | Fairness scheduling               | Per-tenant quotas observed by `MultiModeRuntime` ; the adapter forwards `tenant_id` via `CallContext`. |
| N16    | Multi-region replay               | Catalogue snapshot (G12) is the replay anchor ; the rng_seed forwarding (G7) makes Mode A bit-identical. |
| N17    | LLM provenance                    | Mode C dispatch records provenance ; the envelope's `trace_id` links it to the swarm. |
| N18    | Publisher identity                | Signature verification gate (G2) calls `PublisherApi::verify_nano_signature`. |
| N19    | Gold test gate Tier-1             | Gold binding gate (G4) calls `GoldApi::lookup_gold_set_status`. |
| N20    | Postgres real embeddings          | The N20-audit lesson "no `Ref` across .await" is preserved (I10) ; blackboard reads are scoped + dropped. |
| N21    | Swarm orchestration + agentic     | `SwarmAgent` consumed unchanged ; the orchestrator API surface does not change. |
| N22    | HTTP gateway exposure             | New endpoint `/v1/nano-catalogue` ; AppState gains catalogue + cache fields ; dag_registry rewritten on top of the adapter. |

### 13.1 What N23 does NOT change

- `SwarmAgent` trait — unchanged.
- `NanoAgent` struct — unchanged.
- `NanoAgentExecutor` trait — unchanged.
- The `axum::Router` middleware stack — unchanged.
- The JWT auth path — unchanged.
- The `ProblemDetails` taxonomy — unchanged.

This conservatism is by design : N23 is a **bridge**, not a
re-architecture.  Touching either trait would break six prior
sprints' contracts.

---

## 14. Sprint 15 ship vs Sprint 16+ split

### 14.1 In Sprint 15 (this sprint)

- `efe-nano-swarm-adapter` crate, G1–G15.
- Catalogue back-fill (6 entries).
- `dag_registry` rewrite.
- `GET /v1/nano-catalogue` endpoint.
- Audit + chaos tests.
- Migration guide + smoke example.

### 14.2 Deferred to Sprint 16 N24

- **Blackboard journal** — append-only journal for replay
  (Sprint 16 ships the journaling ; Sprint 15 has the
  snapshot-side already).
- **`POST /v1/dags` (DAG registry)** — tenants publish DAGs
  referencing catalogue entries.
- **Tenant nano publication API** — `POST /v1/publishers/<id>/
  publish-nano` (extends N18).
- **Cost variance alerting** — the `nano.cost.variance.v1`
  topic is published in Sprint 15 ; the alerting consumer
  lands in Sprint 16.
- **Replay endpoint** — `GET /v1/replays/<swarm_id>` to re-run
  a swarm bit-identically using the catalogue snapshot +
  journal.

### 14.3 Deferred to Sprint 17 N25 and beyond

- Multi-region nano federation (catalogue replication across
  regions).
- Marketplace governance (catalogue moderation queue).
- Versioned migration of in-flight DAGs.
- ContextBundle compression.

---

## 15. Annexes

### Annex A — Cargo.toml verbatim

```toml
[package]
name        = "efe-nano-swarm-adapter"
version     = "0.0.1"
edition     = "2021"
publish     = false
license     = "UNLICENSED"
description = "N23 bridge between the M50 NanoAgent catalogue and the N21 SwarmAgent runtime."

[lints]
workspace = true

[dependencies]
tokio       = { workspace = true, features = ["rt", "macros", "time", "sync"] }
async-trait = { workspace = true }
serde       = { workspace = true, features = ["derive"] }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
dashmap     = { workspace = true }
uuid        = { workspace = true, features = ["v4", "serde"] }
chrono      = { workspace = true, features = ["serde"] }
tracing     = { workspace = true }
rand        = { workspace = true }

efe-nano-sdk             = { path = "../efe-nano-sdk" }
efe-swarm-orchestrator   = { path = "../efe-swarm-orchestrator" }
efe-blackboard           = { path = "../efe-blackboard" }
efe-publisher-identity   = { path = "../efe-publisher-identity" }
efe-gold-test-gate       = { path = "../efe-gold-test-gate" }

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time", "sync", "test-util"] }
proptest = "1.4"
serde_test = "1.0"
```

### Annex B — Migration walkthrough (excerpt of N23_NANO_SWARM_ADAPTER.md)

```
1. Are you onboarding a new pre-analysis agent ?

   Write one file `crates/efe-nano-preanalysis/src/<name>.rs` :

     pub fn nano_descriptor() -> Arc<NanoAgent> {
         Arc::new(NanoAgent {
             id: NanoAgentId::new("<name>").unwrap(),
             version: NanoAgentVersion::new("1.0.0").unwrap(),
             description: "...".into(),
             author: Author::Efe,
             category: Category::Extraction,
             stability_tier: StabilityTier::TierExperimental,
             execution_mode: ExecutionMode::PureRust { entry: "<name>::run".into() },
             determinism: DeterminismLevel::FullDeterministic,
             idempotency: IdempotencyPolicy::auto_24h(),
             input_schema: schema_for_input(),
             output_schema: schema_for_output(),
             error_schema: None,
             limits: ResourceLimits::default_mode_a(),
             cost_estimate_cents: 0.01,
             latency_p99_ms_target: 50,
             audit: Default::default(),
             compliance: Default::default(),
             mcp_tool_metadata: None,
             signature: None,
             co_publishing: None,
             gold_set_ref: None,
             created_at: Utc::now(),
         })
     }

     pub fn nano_runner() -> Arc<dyn NanoAgentExecutor> {
         Arc::new(MyRunner)   // implements NanoAgentExecutor.execute_with_context
     }

   Then add one line in `default_catalogue()` :
     out.push((nano_descriptor(), nano_runner()));

   That's it.  The adapter writes to blackboard, emits the
   envelope, integrates with N18 / N19 / N12 automatically.
```

### Annex C — Sample envelope payload

```json
{
  "schema": "nano.envelope.v1",
  "agent_id": "ner-extractor.1.0.0",
  "nano_id": "ner-extractor",
  "nano_version": "1.0.0",
  "swarm_id": "5a8e9f4f-3b6d-4d23-9a4f-2f1d8c7b5e3a",
  "trace_id": "8c1f2d4b6e9a7c5d3f1b2a4e6c8d9f0b",
  "mode": "pure-rust",
  "cost_estimate_microcents": 100,
  "cost_actual_microcents": 92,
  "cost_variance_microcents": -8,
  "cache_hit": false,
  "signature_verified": true,
  "gold_bound": true,
  "latency_ms": 14,
  "outcome": "completed",
  "emitted_at": "2026-05-23T12:34:56.789Z"
}
```

### Annex D — OpenAPI 3.1 path entry for `/v1/nano-catalogue`

```yaml
/v1/nano-catalogue:
  get:
    summary: List the merged nano catalogue available to this tenant.
    operationId: listNanoCatalogue
    security:
      - bearerAuth: []
    responses:
      "200":
        description: Catalogue snapshot.
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/NanoCatalogueResponse"
      "401":
        $ref: "#/components/responses/MissingAuth"
      "503":
        $ref: "#/components/responses/CatalogueUnavailable"

components:
  schemas:
    NanoCatalogueResponse:
      type: object
      required: [version, generated_at, tenant_id, entries]
      properties:
        version: { type: string, example: "1.0" }
        generated_at: { type: string, format: date-time }
        tenant_id:
          oneOf:
            - { type: string, format: uuid }
            - { type: "null" }
        entries:
          type: array
          items: { $ref: "#/components/schemas/NanoAgent" }
```

### Annex E — Per-tenant catalogue isolation matrix

| JWT shape                              | Sees `efe` ns | Sees `tenant/<their>` | Sees other tenants |
|----------------------------------------|---------------|------------------------|--------------------|
| `tenant_id = nil`, no operator         | ✓             | ✗                      | ✗                  |
| `tenant_id = A`                         | ✓             | only A                 | ✗                  |
| `tenant_id = nil`, `scope: operator`   | ✓             | ✗                      | ✓ (all tenants)    |
| no JWT                                 | 401            | 401                    | 401                |

### Annex F — Hash spec for catalogue determinism (I20)

```
sha256(
  serde_json::to_vec(
    &default_catalogue()
      .iter()
      .map(|(n, _)| n.as_ref())
      .collect::<Vec<_>>()
  )
  .unwrap()
)
```

The hash is asserted equal across builds in
`audit_n23_catalogue_deterministic.rs`.  If it drifts, the
test fails with a clear "catalogue order or content changed —
this is a release-impacting change ; bump the catalogue
revision" message.

### Annex G — Mapping table : six pre-analysis agents

| Built-in agent          | NanoAgentId           | Mode | Determinism            | Idempotency  | Tier            | Default policy   |
|-------------------------|-----------------------|------|------------------------|--------------|------------------|------------------|
| `NerExtractor`          | `ner-extractor`       | A    | FullDeterministic      | Auto, 24h    | Tier1            | `tier1_strict`   |
| `LanguageDetector`      | `language-detector`   | A    | FullDeterministic      | Auto, 24h    | Tier1            | `tier1_strict`   |
| `SchemaValidator`       | `schema-validator`    | A    | FullDeterministic      | Auto, 1h     | Tier1            | `tier1_strict`   |
| `PiiRedactor`           | `pii-redactor`        | A    | FullDeterministic      | Auto, 24h    | TierAudit        | `tier1_strict` + signature mandatory |
| `SentimentClassifier`   | `sentiment-classifier`| B    | Seeded                 | Auto, 24h    | TierExperimental | `experimental_fast` |
| `SummaryWriter`         | `summary-writer`      | C    | LlmReplayable          | Manual       | TierExperimental | `experimental_fast` |

### Annex H — Risk register

| Risk                                                                                    | Likelihood | Impact | Mitigation                                  |
|-----------------------------------------------------------------------------------------|------------|--------|---------------------------------------------|
| `MultiModeRuntime` dispatch performance regression once 6 agents flow through it        | Medium     | Medium | Bench Mode A dispatch P50 < 200µs before/after ; if regression > 30 %, defer dispatch to a thin path |
| Six catalogue entries' canonical hashes flip on every minor schema tweak                | High       | Low    | Annex F hash test gates the change ; intentional bumps are an explicit catalogue revision |
| Tenants want to override the input pointer to read from a non-default blackboard key     | Medium     | Low    | Adapter exposes `with_input_pointer` ; documented in migration guide |
| `SwarmToNano` orchestrator construction overhead too high for hot CLI invocations        | Low        | Medium | Cache the orchestrator across calls behind a `tokio::sync::OnceCell` ; verified in chaos test F8 |
| EventBus saturation on bursty swarms drops envelopes                                    | Medium     | Low    | F9 ; envelope persisted to blackboard as fallback |

### Annex I — Decision log (compact)

```
2026-05-19  ADR-1   ship adapter as own crate           — agreed unanimously
2026-05-19  ADR-2   forward direction primary           — agreed unanimously
2026-05-19  ADR-3   policy as argument                  — agreed unanimously
2026-05-19  ADR-4   banker's rounding                   — agreed (debate w/ truncation; rejected for billing exposure)
2026-05-20  ADR-5   /nano/<agent_id>/<field> namespace  — agreed
2026-05-20  ADR-6   AgentId = id.version                — agreed
2026-05-20  ADR-7   Arc executor                        — agreed
2026-05-20  ADR-8   build-time signed catalogue         — agreed
2026-05-20  ADR-9   single JSON response shape          — agreed
2026-05-20  ADR-10  SwarmToNano single-shot scope       — agreed
```

### Annex J — Glossary

- **Adapter** : a struct that implements one trait by delegating
  to another trait via a translation layer.  Here :
  `NanoToSwarm impl SwarmAgent` delegates to `dyn NanoAgentExecutor`.
- **Catalogue** : the signed, versioned, schema-bound set of
  `NanoAgent` data records.  Built-in (`efe` namespace) +
  tenant-published (`tenant/<uuid>` namespace).
- **Dispatch path** : the sequence of crate boundaries crossed
  from `SwarmOrchestrator::run` to the actual user-defined
  agent code.  Post-N23 : `Orchestrator → SwarmAgent
  (NanoToSwarm) → NanoAgentExecutor (MultiModeRuntime) →
  Mode A/B/C handler`.
- **Envelope** : a one-shot audit record emitted per dispatch.
  Versioned schema (`nano.envelope.v1`).
- **Policy** : `NanoToSwarmPolicy` ; per-dispatch configuration
  for compliance gating (signature, gold, limits, cache).
- **Synthetic catalogue back-fill** : adding `NanoAgent`
  descriptors for agents that historically existed only as
  `SwarmAgent` impls.

### Annex K — End-of-sprint exit checklist

- [ ] `cargo build --workspace` clean.
- [ ] `cargo test --workspace` green, expected ≥ 2570 PASSED.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `audit_n23_*.rs` green (12).
- [ ] `phase_8_n23_*.rs` green (33).
- [ ] `chaos_n23_*.rs` green (8).
- [ ] `examples/n23_migration_smoke.rs` runs end-to-end.
- [ ] `docs/migration/N23_NANO_SWARM_ADAPTER.md` written + reviewed.
- [ ] Live smoke against the gateway : `GET /v1/nano-catalogue`
      returns 6 entries.
- [ ] Live smoke : `POST /v1/swarms/run` with the rewritten
      default DAG produces a `SwarmReport` whose total cost has
      a non-zero `estimate` field and a non-zero `actual` field.
- [ ] N18 publisher signature is verified on each dispatch
      (visible in the envelope).
- [ ] N19 gold-set lookup occurs on Tier-1 agents (visible in the
      envelope).
- [ ] Sprint 15 retro scheduled : verify no behavioural
      regression in N22 acceptance tests.

### Annex L — Out-of-scope explicit list

The following items are explicitly **not** in Sprint 15 N23 :

- DAG persistence (deferred to N24).
- Multi-region nano catalogue replication (deferred to N25).
- Replay endpoint (deferred to N24).
- Tenant-published nano write endpoint (extends N18, deferred).
- Cost variance alerting consumer (publishes in N23, consumer
  in N24).
- ContextBundle compression (deferred, no sprint assigned).
- gRPC surface for the adapter (deferred to M51 federation).
- Adapter for non-Tokio runtimes (`async-std`, `smol`) — not
  on the roadmap.

---

## Document control

- **Author**          : @swarm-platform-lead
- **Reviewers**       : @nano-sdk-lead, @compliance-lead, @sre-lead
- **Design freeze**   : 2026-05-21
- **Code freeze**     : 2026-05-27
- **Ship**            : 2026-05-28 (Sprint 15 day 5)
- **Supersedes**      : none
- **Superseded by**   : (forward reference) `EFE_M50_N24_DAG_REGISTRY_BLACKBOARD_JOURNAL_ARCHITECTURE.md`

---

_End of EFE_M50_N23_NANO_SWARM_ADAPTER_ARCHITECTURE.md._
