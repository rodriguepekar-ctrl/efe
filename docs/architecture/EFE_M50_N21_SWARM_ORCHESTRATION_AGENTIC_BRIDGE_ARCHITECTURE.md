# EFE M50 — N21 Swarm Orchestration + Agentic Bridge Architecture

> **Sprint** : 13 (Phase 5, day 51 – 55 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md` (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md` (Sprint 11, shipped)
> * `EFE_M50_N20_POSTGRES_REAL_EMBEDDINGS_ARCHITECTURE.md` (Sprint 12, shipped)
>
> **Anchors** : N11-N20 shipped a **compliance + audit layer** for
> isolated agent invocations.  N21 closes the **product positioning
> gap** identified in the Sprint 12 retro : EFE's actual product
> intent is a **multi-agent swarm pre-analyser** that pre-digests
> raw inputs so a downstream **agentic LLM** (Claude in tool-use
> mode) gets structured, redacted, bounded context instead of raw
> dumps.  N21 ships the **three crates** that make this real :
> `efe-blackboard`, `efe-swarm-orchestrator`, `efe-agentic-bridge`.
> The N11-N20 stack becomes an **opt-in compliance layer per agent**.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N21 closes (P1–P15)](#2-fifteen-incident-patterns-n21-closes-p1p15)
3. [Commitments — what ships in Sprint 13 (G1–G15)](#3-commitments--what-ships-in-sprint-13-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N20)](#13-cross-cutting-alignment-n11--n20)
14. [Sprint 13 ship vs Sprint 14/15 split](#14-sprint-13-ship-vs-sprint-1415-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The product intent EFE was always meant to serve.**

A customer uploads a 40-page PDF contract.  The naïve approach
hands the raw PDF to Claude in agentic mode :

| Cost dimension | Raw dump | Pre-analysed |
|---|---|---|
| Input tokens | 32 000 | 4 000 |
| Cost per call (Claude Sonnet 4.6) | $0.51 | $0.06 |
| Latency P95 | 12 s | 1.5 s |
| PII exposure to hosted LLM | YES (compliance break) | NO (redacted upstream) |
| Schema reliability | LLM-judged, brittle | Structurally validated upstream |
| Cross-language support | English-biased | FR/DE/IT/... pre-translated |
| Reproducibility | Non-deterministic | DAG + blackboard = replay-deterministic |

The **pre-analysis swarm** is the layer that does this work.  It
runs N small agents (Mode A pure Rust deterministic / Mode B
local model / Mode C lightweight LLM) **in a DAG of layers** that
can :

- run in **parallel** within a layer (NER + sentiment + language
  detection happen simultaneously) ;
- communicate via a **shared blackboard** (writeable JSON store
  versioned per agent write) ;
- pub/sub via an **EventBus** (NER agent publishes "found 3 PII
  entities", redaction agent subscribes) ;
- branch via **conditional edges** ("if language == FR, run
  translate_clauses ; else skip") ;
- aggregate via a **final Aggregator** that builds a
  `ContextBundle` for the downstream LLM ;
- ship the `ContextBundle` to the **AgenticBridge** that wraps
  Claude's Messages API in tool-use mode.

**Eight production capabilities ship in Sprint 13 N21** :

1. **`SwarmDag`** + `SwarmDagBuilder` — declarative DAG definition
   with cycle detection at build time, conditional edges,
   per-agent timeout, and an inviolable invariant that the
   Aggregator runs last.
2. **`Blackboard`** — `DashMap<JsonPointer, BlackboardValue>` with
   per-field versioning, observer pattern for change notifications,
   short-lived read guards (no `Ref` across `.await` — N20 audit
   lesson), and a 256 KB hard cap on the assembled `ContextBundle`.
3. **`EventBus`** — bounded `tokio::sync::broadcast` channel
   (capacity 256 messages per swarm run) with per-topic subscription,
   back-pressure when full (slow consumers drop, fast ones survive).
4. **`SwarmOrchestrator`** — runs the DAG layer-by-layer ; spawns
   tasks within a layer ; awaits all completions before advancing ;
   honors `tokio::time::timeout` per agent + global swarm timeout ;
   collects `AgentOutcome`s ; feeds to the Aggregator.
5. **`SwarmReport`** — per-agent + aggregate report : duration_ms,
   cost_microcents, blackboard_writes, events_published, errored.
   Total cost attributable per `(agent_id, swarm_id)`.
6. **`AgenticBridge`** trait + **`ClaudeBridge`** implementation —
   wraps Anthropic's Messages API with `tool_use` schema ; the
   `ContextBundle` is exposed to Claude as `tool_use` input ;
   supports streaming response.
7. **`ContextBundle`** SDK type — wire-frozen JSON bundle that the
   AgenticBridge consumes.  Schema includes : `entities[]`,
   `language`, `redactions[]`, `summary`, `classifications[]`,
   `metadata` (cost, swarm_id, agents_executed).
8. **Opt-in compliance** — `SwarmDag.compliance_policy` lets the
   caller turn on N18 signature verification + N19 gold-set
   binding per agent.  Default = off (low latency, low overhead).
   When on, each agent dispatch goes through the N11-N20 stack.

**Six reference agents ship in `efe-nano/src/agents/preanalysis/`** :
- `ner_extractor` (Mode A) — entity extraction via regex + spaCy
  port
- `sentiment_classifier` (Mode B) — local BERT-base fine-tuned
- `language_detector` (Mode A) — `whatlang` library
- `pii_redactor` (Mode A) — regex + `aho-corasick`
- `summary_writer` (Mode C) — Claude Haiku (cheap LLM)
- `schema_validator` (Mode A) — `jsonschema` strict mode

**Backward compatibility** :
- N11-N20 wire format and traits are 100 % unchanged.
- New crates depend on `efe-nano-sdk` re-exports ; no breaking
  edits there.
- Existing `SwarmRequest`/`SwarmEnvelopePayload` (N14) remain the
  default for single-invocation use cases ; `SwarmDag` is the new
  multi-agent extension.
- Existing `NanoAgent` invocation pipeline is unchanged.

**Out of scope (deferred Sprint 14/15)** :
- Real Anthropic Messages API streaming (Sprint 14 ; Sprint 13
  ships sync + stub streaming hooks).
- Cross-swarm caching of pre-analysis results (Sprint 14).
- Multi-region swarm dispatch (Sprint 15 — needs N16 cross-region
  envelope wiring).
- GPT / Mistral / Local-Llama bridges (Sprint 14 — Sprint 13
  ships `AgenticBridge` trait + `ClaudeBridge` ; the others impl
  the same trait).
- Vector-store-backed retrieval-augmented pre-analysis (Sprint 15).
- Swarm result diff visualisation in operator portal (Sprint 14).

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N21

**Incident I-2026-08-12 — Raw-PDF dump cost overrun.**
The Contracts Pilot team enabled Claude Sonnet on uploaded PDFs.
For 1 200 PDF/day × 32 k tokens × $0.015 / 1 k input = **$576 / day**.
Monthly bill projected at $17 280.  The same workload through a
swarm pre-analysis (NER + summary + language + redact + schema)
produces ~4 k token `ContextBundle` to Claude → $72 / day → $2 160
/ month.  **Savings : ~$15 000 / month / pilot.**  Without a swarm
orchestration layer, the cost is unsustainable past 5 pilots.

**Incident I-2026-08-18 — Mistranslated French clauses.**
A FR contract uploaded as English-named PDF.  Naïve Claude pipeline
assumed EN → mistranslated 11 clauses → customer rejected ALL
auto-generated summaries.  Root cause : no language-detection step
before LLM.  Swarm's `language_detector` (1 ms) would have caught
it → routed via `translate_clauses` (Claude Haiku, $0.001) →
correct summary.

**Incident I-2026-08-22 — PII leak to hosted LLM.**
A customer contract had 47 email addresses + 12 phone numbers
inline.  Raw PDF sent to OpenAI's GPT-4 endpoint without redaction.
GDPR Article 28 violation → compliance team P0.  Swarm's
`pii_redactor` (3 ms, Mode A regex + Aho-Corasick) would have
masked them before the LLM saw the doc.  Cost of incident :
4-week regulatory inquiry + customer escrow $200 k.

**Incident I-2026-08-25 — Race condition between unstructured agents.**
A homegrown async loop ran 8 pre-analysis tasks concurrently with
no DAG.  Two of them wrote to the same JSON path
`/document/summary` ; the last one wrote won.  The aggregator
read a partially-written summary → garbled output.  Root cause :
no DAG → no happens-before → no per-field versioning.

**Incident I-2026-08-28 — Duplicated NER work across agents.**
The summary agent and the classification agent each ran their own
NER pass.  Profile : 2 × 800 ms duplicated work.  No EventBus, no
shared blackboard, no inter-agent comm → wasted compute.  At
30 000 documents / day this is **$120 / day** of redundant work.

### 1.2 Why Sprint 13, immediately after N20

The N18-N20 sequence (publisher identity → gold test → real
embeddings + Postgres) finalised the **compliance layer**.  The
remaining product gap is the **value-creation layer** : how
customers actually use EFE.  Without N21, EFE is a sophisticated
audit chain attached to single agent invocations.  With N21, EFE
becomes the **swarm pre-analyser** that the original product
vision (per the Sprint 12 retro) always intended.

Sprint 13 ordering :

| Day | Deliverable | Depends on |
|---|---|---|
| 51 | `efe-blackboard` crate (Blackboard + EventBus + Observer) | none |
| 52 | `efe-swarm-orchestrator` crate (SwarmDag + Builder + cycle detect) | efe-blackboard |
| 53 | `SwarmOrchestrator::run` + layer scheduler + per-agent timeout | efe-blackboard, N15 fairness |
| 54 | `efe-agentic-bridge` crate (ClaudeBridge + ContextBundle + tool-use) | reqwest, N17 model identity |
| 55 | 6 reference pre-analysis agents + 15 acceptance tests + opt-in N18+N19 wiring | N11-N20 |

### 1.3 Who consumes N21 once it ships

| Consumer | Use case |
|---|---|
| **Product team** | Closes the product positioning gap : EFE is now a pre-analyser, not a compliance shell |
| **Customer success** | Pilots ship 10× more swarms now that latency is 1.5 s vs 12 s |
| **Cost team** | Per-swarm cost dashboard ; agent-level attribution |
| **Compliance team** | Opt-in N18+N19 per agent gives them the audit trail when they need it WITHOUT paying for it on every call |
| **Security team** | PII redactor running BEFORE hosted-LLM is the canonical defense pattern |
| **Sales team** | "EFE saves 90 % LLM cost via swarm pre-analysis" — the differentiator |
| **Anthropic partnership** | First-class `tool_use` integration with Claude Sonnet/Haiku |
| **N11 Firecracker pool** | Per-agent in microVM (optional) |
| **N12 cache** | LLM calls inside swarm cached by N12 key |
| **N15 fairness** | New `OperationClass::SwarmAgent` quota |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns clustered into 5 families :

* **Context bloat / cost** (P1, P2, P3) — 3 patterns
* **Domain leak** (P4, P5, P6) — 3 patterns
* **Inter-agent coordination** (P7, P8, P9) — 3 patterns
* **Failure isolation** (P10, P11, P12) — 3 patterns
* **Observability + reproducibility** (P13, P14, P15) — 3 patterns

---

## 2. Fifteen incident patterns N21 closes (P1–P15)

### Family A — Context bloat / cost

**P1. Raw input dumped wholesale to hosted LLM.**
32 k token PDF × $0.015 / 1 k = $0.48 per call (Incident I-2026-
08-12).  N21's swarm distills to ~4 k tokens.

**P2. Agentic LLM tier expensive on large inputs.**
Claude Sonnet is 5× more expensive than Haiku ; the latter is
fine for summarisation but inadequate for legal reasoning.  N21
routes summarisation through Haiku (cheap, small) and reserves
Sonnet for the final agentic step (expensive, smart) with already-
distilled context.

**P3. Latency linear in input size.**
12 s P95 on 40-page PDFs becomes a customer-experience blocker.
N21's swarm runs in parallel layers ; P95 drops to 1.5 s.

### Family B — Domain leak

**P4. PII exposed to hosted LLM (compliance break).**
Incident I-2026-08-22.  Swarm's `pii_redactor` (Mode A, 3 ms)
must run BEFORE any hosted-LLM call.

**P5. Wrong-language input misclassified by language-blind LLM.**
Incident I-2026-08-18.  Swarm's `language_detector` (Mode A, 1 ms,
whatlang) catches FR/DE/IT/JP at near-zero cost.

**P6. LLM output schema-violation undetected.**
LLM returns Markdown when JSON expected ; downstream parser
crashes.  Swarm's `schema_validator` (post-LLM) catches the drift
and triggers a retry with explicit `<json>` framing.

### Family C — Inter-agent coordination

**P7. No DAG → race conditions on shared output.**
Incident I-2026-08-25.  N21's blackboard has per-field versioning ;
two writers to the same JSON pointer become a CONFLICT, not a
silent last-write-wins.

**P8. No EventBus → duplicated work across agents.**
Incident I-2026-08-28.  Summary agent and classification agent
each redo NER.  N21's EventBus lets `ner_extractor` publish "found
entities" ; the others subscribe + reuse.

**P9. No conditional routing → agents fire for nothing.**
A `translate_clauses` agent runs on EN documents (no-op, wasted
$0.001/call × 30 k/day = $30/day).  N21's conditional edges
(`if blackboard.language != "en"`) skip the agent.

### Family D — Failure isolation

**P10. One agent crashes → whole swarm crashes.**
Naïve async-spawn-with-`?` fails the join handle ; the swarm
reports nothing.  N21's `AgentOutcome::Errored { kind, message }`
isolates one agent's failure ; the swarm continues with partial
context.

**P11. Agentic LLM down → swarm produces nothing useful.**
If Claude API is in outage, the swarm's pre-analysis is still
available — N21 returns the `ContextBundle` to the caller who can
defer the agentic step or substitute a local model.

**P12. Per-agent vs global timeout confusion.**
A single 30 s timeout on the swarm doesn't catch a 28 s hung
agent — by the time you notice, the budget is gone.  N21 has BOTH
per-agent timeouts (default 5 s) AND a global swarm timeout
(default 30 s).

### Family E — Observability + reproducibility

**P13. No per-agent trace in homemade swarms.**
N21's `SwarmReport.agents[i]` carries duration_ms, cost_microcents,
blackboard_writes, events_published, errored.  N14 envelope
shipping carries `swarm_id + agent_id_in_swarm` for full audit.

**P14. Cost attribution impossible across agents.**
N21's `SwarmReport.total_cost_microcents` plus per-agent
attribution.  Per-tenant cost dashboards become trivial.

**P15. DAG non-deterministic → replay impossible.**
Random ordering within a layer makes replay unstable.  N21's
layer scheduler uses a deterministic order (BTreeMap of agent_id
ASCII order) ; replay with the same blackboard inputs produces
the same final `ContextBundle`.

---

## 3. Commitments — what ships in Sprint 13 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### DAG & orchestration

**G1. `SwarmDag` declarative builder with cycle detect.**
`SwarmDagBuilder::add_agent`, `::add_dependency`, `::add_conditional`,
`::with_aggregator`.  Build returns `Err(SwarmDagError::Cycle)` if
any cycle is present.  Acceptance :
`phase_6_n21_swarm_dag_cycle_detected_at_build`.

**G2. Layer-by-layer execution.**
Layer N starts iff all layer-(N-1) deps complete.  Within a layer,
agents run concurrently via `tokio::spawn`.  Acceptance :
`phase_6_n21_layers_execute_in_order`.

**G3. Conditional edges via predicates.**
Closures `Fn(&Blackboard) -> bool` decide whether a dependency
fires.  Acceptance :
`phase_6_n21_conditional_skip_on_predicate_false`.

### Blackboard & EventBus

**G4. Blackboard with per-field versioning.**
Each write increments a per-pointer version ; readers see a
consistent snapshot.  Acceptance :
`phase_6_n21_blackboard_concurrent_writes_serialised`.

**G5. EventBus bounded broadcast.**
256-message capacity ; slow consumers drop, fast consumers survive.
Acceptance :
`phase_6_n21_eventbus_slow_consumer_drops_old`.

### Reporting

**G6. Per-agent + aggregate `SwarmReport`.**
duration_ms, cost_microcents, blackboard_writes, events_published.
Acceptance :
`phase_6_n21_swarm_report_attributes_cost_per_agent`.

### Agentic bridge

**G7. `AgenticBridge` trait + `ClaudeBridge` impl.**
Anthropic Messages API ; tool-use shape.  Acceptance :
`phase_6_n21_claude_bridge_tool_use_round_trip` (wiremock).

**G8. `ContextBundle` ≤ 256 KB hard cap.**
Aggregator refuses to emit a bundle exceeding the cap.  Acceptance :
`phase_6_n21_context_bundle_exceeds_256kb_rejected`.

### Failure isolation

**G9. Best-effort : 1 agent fails → swarm continues.**
The DAG completes with `AgentOutcome::Errored` for the failed
agent ; downstream agents that depend on it via the failed branch
skip ; the aggregator emits a partial bundle.  Acceptance :
`phase_6_n21_one_agent_failure_swarm_completes_partial`.

**G10. Per-agent + global swarm timeout.**
Per-agent default 5 s, configurable up to 60 s.  Global default
30 s.  Acceptance :
`phase_6_n21_per_agent_timeout_fires_independently`.

### Reference agents

**G11. Six pre-analysis agents ship.**
`ner_extractor`, `sentiment_classifier`, `language_detector`,
`pii_redactor`, `summary_writer`, `schema_validator`.  Acceptance :
`phase_6_n21_six_reference_agents_loadable`.

### Cross-cutting

**G12. N14 envelope carries swarm_id + agent_id_in_swarm.**
Acceptance : `phase_6_n21_envelope_traces_per_agent`.

**G13. Optional N18 + N19 per agent.**
`AgentNode.compliance: Compliance::N18Signed { ... } |
Compliance::N19GoldGated { ... } | Compliance::None`.  Acceptance :
`phase_6_n21_compliance_opt_in_per_agent_works`.

**G14. Replay determinism.**
Same DAG + same blackboard inputs + same RNG seed → identical
final ContextBundle byte-for-byte.  Acceptance :
`phase_6_n21_replay_produces_identical_bundle`.

### Operability

**G15. Prometheus surface.**
12 surfaces in `n21_*` namespace.  Acceptance :
`phase_6_n21_metrics_emit_all_12_surfaces`.

---

## 4. Invariants — non-negotiables (I1–I20)

**I1. DAG is acyclic.**  Builder validates at `build()` ; runtime
trusts.

**I2. Aggregator runs last + exactly once.**  Builder enforces that
exactly one node is tagged as Aggregator ; it depends transitively
on all other nodes.

**I3. Blackboard writes are atomic per JSON pointer.**  `DashMap`
shard-locked ; readers see committed state only.

**I4. EventBus capacity bounded at 256 messages.**  No infinite-
memory growth.

**I5. Conditional predicates are pure.**  No I/O, no async, no
RNG.  The runtime invokes them in the layer scheduler ; impure
predicates break replay (F10).

**I6. Per-agent timeout enforced via `tokio::time::timeout`.**  No
host-level kills ; in-process cancellation only.

**I7. Global timeout encompasses all agents + aggregator.**

**I8. Cost is `i64` microcents end-to-end.**  No float drift in
aggregations.

**I9. Failure isolation = best-effort.**  One agent's
`Result::Err` doesn't propagate to the swarm runner.

**I10. ContextBundle bounded ≤ 256 KB.**  Aggregator rejects if
exceeds.

**I11. ClaudeBridge uses Anthropic Messages API shape verbatim.**
Body format frozen with `serde` types matching `messages-2024-06-01`
+ `tool_use` block.

**I12. Tool-use shape carries `name = "swarm_context"` always.**
The downstream Claude call is told : "you have a tool that
returns the pre-analysed context".

**I13. Replay is deterministic.**  Same DAG + same input + same
seed = same bundle.  Sorting per-layer by ASCII agent_id is part
of the contract.

**I14. swarm_id is a v4 UUID, globally unique.**

**I15. N14 envelope opt-in.**  Default off ; turn on via
`SwarmRunConfig.emit_envelope`.

**I16. N18/N19 compliance opt-in per agent.**  Default off.

**I17. No DashMap `Ref` held across `.await`.**  Lesson from N20
audit B1.

**I18. Cardinality of metrics bounded.**  ≤ 200 distinct
`agent_id` labels per region.

**I19. Conditional skip propagates : if A is skipped, all of A's
descendants also skip.**

**I20. Aggregator failure = swarm failure.**  Per G9 best-effort,
agent failures are isolated ; aggregator failure is fatal because
without it, no ContextBundle exists.

---

## 5. Failure modes — what we PROTECT against (F1–F15)

### DAG construction

**F1. DAG cycle.**
* *Detection* : `SwarmDagBuilder::build()`.
* *Response* : `Err(SwarmDagError::Cycle { path: Vec<AgentId> })`.

**F2. Aggregator missing.**
* *Detection* : `build()`.
* *Response* : `Err(SwarmDagError::NoAggregator)`.

**F3. Aggregator depends on nothing.**
* *Detection* : `build()`.
* *Response* : `Err(SwarmDagError::AggregatorDisconnected)`.

### Runtime

**F4. Per-agent timeout.**
* *Detection* : `tokio::time::timeout` returns `Err(Elapsed)`.
* *Response* : `AgentOutcome::TimedOut { agent_id }`.
* *Recovery* : downstream conditionals branch on
  `Outcome::TimedOut`.

**F5. Global swarm timeout.**
* *Detection* : outer `timeout()` envelope on `SwarmOrchestrator::run`.
* *Response* : abort all in-flight tasks, return
  `SwarmReport::Aborted { reason: "global_timeout" }`.

**F6. Aggregator panics.**
* *Detection* : `JoinHandle::await` returns `Err(JoinError)`.
* *Response* : `SwarmReport::Failed`.

**F7. ClaudeBridge 429 rate limit.**
* *Detection* : HTTP 429 from Anthropic.
* *Response* : exponential backoff (1 s, 5 s, 30 s) up to 3
  retries ; eventual `BridgeError::RateLimited`.

**F8. Claude returns tool-use violation.**
* *Detection* : response body lacks expected `tool_use` block.
* *Response* : `BridgeError::ProtocolViolation`.

### Blackboard / EventBus

**F9. Blackboard write race.**
* *Detection* : per-field version mismatch on CAS-style write.
* *Response* : `BlackboardError::Conflict { pointer, expected_version, actual_version }`.

**F10. Conditional predicate diverges (non-pure).**
* *Detection* : test-time check via property test that the
  predicate is invariant under repeated evaluation.
* *Response* : not enforced at runtime ; documented as a contract.

**F11. EventBus capacity full.**
* *Detection* : `broadcast::Sender::send` returns `Err`.
* *Response* : drop oldest message ; metric
  `n21_eventbus_dropped_total`.

### Compliance opt-in

**F12. N18 signature invalid (when compliance ON).**
* *Detection* : N18 `SignatureVerifier` returns `Err`.
* *Response* : agent dispatch fails ; `AgentOutcome::Errored`.

**F13. N19 gold gate missing (when compliance ON).**
* *Detection* : N19 registry returns `None` for `(agent_id,
  version)`.
* *Response* : agent dispatch fails-closed.

### Bundle

**F14. ContextBundle exceeds 256 KB.**
* *Detection* : aggregator's `bundle.estimated_size()`.
* *Response* : aggregator returns `Err(AggregatorError::BundleTooLarge)`.

**F15. Replay drift (CI tests).**
* *Detection* : `phase_6_n21_replay_produces_identical_bundle`
  compares two runs ; fails if bytes differ.
* *Response* : CI red ; root cause is impure agent / predicate /
  RNG leak.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. SwarmDag as builder pattern + validate-at-build.

**Context.**  Where do DAG invariants get enforced ?

**Decision.**  `SwarmDagBuilder` is the only constructor.  `build()`
validates :
- DAG acyclic (Kahn's algorithm topological sort detecting cycles)
- Exactly one Aggregator
- Aggregator transitively depends on every other node
- All edges reference declared agents

**Rationale.**  Once built, the `SwarmDag` is immutable and the
runtime skips re-validation per run.  Errors caught at build are
caught at developer-time, not at production-time.

**Rejected alternatives.**  Runtime validation on every `run()` →
wasted CPU.  Implicit DAG inference from naming → fragile.

---

### ADR-2. Blackboard = `DashMap<JsonPointer, BlackboardValue>` with versioning.

**Context.**  Concurrent agents need to mutate shared state.

**Decision.**  Sharded `DashMap` keyed by JSON pointer (RFC 6901).
Each value is `BlackboardValue { value: serde_json::Value, version:
u64, written_by: AgentId, written_at: SystemTime }`.  CAS-style
writes : `write_if(pointer, expected_version, new_value)`.

**Rationale.**
- Fine-grained locking (per-shard) ; readers don't block.
- JSON pointer naming aligns with the bundle output schema.
- Versioning catches race conditions explicitly (F9).

**Rejected alternatives.**  Single Mutex<HashMap> → contention.
ConcurrentLinkedHashMap → no async support in Rust ecosystem.

---

### ADR-3. EventBus = bounded `tokio::sync::broadcast`.

**Context.**  Agents publish/subscribe to inter-agent events.

**Decision.**  `tokio::sync::broadcast::channel(256)` per swarm
run.  Topic filtering at subscriber side (cheap, in-process).
Slow consumers see `RecvError::Lagged`.

**Rationale.**  Battle-tested in tokio ecosystem ; back-pressure
behaviour is documented and explicit.

**Rejected alternatives.**  Custom MPMC → maintenance burden.
crossbeam-channel → no async story.

---

### ADR-4. Conditional edges = boxed closure `Fn(&Blackboard) -> bool`.

**Context.**  How does `if language == "fr"` get encoded ?

**Decision.**  `ConditionalEdge { from: AgentId, to: AgentId,
predicate: Box<dyn Fn(&Blackboard) -> bool + Send + Sync> }`.

**Rationale.**  Closure is type-erased ; the DAG can express any
boolean predicate over blackboard state without ad-hoc enum
variants.

**Rejected alternatives.**  Domain-specific predicate enum →
inflexible.  Rhai/Lua scripting → too much surface area for
Sprint 13.

---

### ADR-5. ClaudeBridge via `reqwest` + serde.

**Context.**  How to call Anthropic Messages API.

**Decision.**  `reqwest::Client` POST `/v1/messages` with body
matching `2023-06-01` API spec ; auth via `x-api-key` + `anthropic
-version` headers.

**Rationale.**  Identical pattern to N20 OpenAI/Cohere providers ;
already audited.

---

### ADR-6. Tool-use schema follows Anthropic Messages API.

**Context.**  How is ContextBundle passed to Claude ?

**Decision.**
- We do NOT pass the bundle in the user message (would waste tokens).
- Instead, the call sends `messages: [{role: user, content: "<task>"}]`
  PLUS `system: "You have a tool `swarm_context` that returns
  pre-analysed structured data..."` PLUS `tools: [{name:
  "swarm_context", input_schema: <json_schema_for_bundle>,
  description: "..."}]`.
- The first turn, Claude calls the tool ; we return the bundle as
  `tool_result`.
- The second turn, Claude has the context and produces the final
  answer.

**Rationale.**  Native to Anthropic's intended pattern ; lets
Claude decide which fields of the bundle to inspect (token-efficient).

**Rejected alternatives.**  Inline JSON in user message → 4× the
token cost.

---

### ADR-7. Per-agent timeout via `tokio::time::timeout`.

**Context.**  How are runaway agents bounded ?

**Decision.**  Each agent dispatch wrapped in `tokio::time::
timeout(Duration::from_millis(agent.timeout_ms), agent.run(...))`.
Default 5 000 ms ; max 60 000 ms.

**Rationale.**  In-process, no kernel kills.  Aligns with N19's
case-level timeout pattern.

---

### ADR-8. Failure isolation : `Result<AgentOutcome>`.

**Context.**  How does the runtime collect agent results ?

**Decision.**  Each spawned task returns `Result<AgentOutcome,
JoinError>`.  The runner collects all via `tokio::join!` /
`futures::future::join_all`.  Errored agents become
`AgentOutcome::Errored` ; the run still completes.

**Rationale.**  Best-effort is the product goal (G9 + F11).

---

### ADR-9. Cost in `i64` microcents end-to-end.

**Context.**  Aggregating cost across N agents safely.

**Decision.**  All cost fields are `i64` microcents (1 USD =
1 000 000 microcents).  N20 N19 same convention preserved.

**Rationale.**  No float drift ; safe sum of 1 000+ agents per
day.

---

### ADR-10. N14 envelope optional opt-in per swarm.

**Context.**  Should every swarm dispatch emit a N14
SwarmEnvelopePayload ?

**Decision.**  Opt-in via `SwarmRunConfig.emit_envelope: bool`.
Default false (low-latency mode).  When true, each agent dispatch
generates a child envelope rooted at the swarm_id.

**Rationale.**  Most production use cases don't need the audit
chain at the per-agent level ; opt-in keeps the hot path fast.
Compliance use cases turn it on globally.

---

## 7. Component architecture

### 7.1 The 8 components at a glance

```text
                       Customer request
                              │
                              ▼
            ┌──────────────────────────────────────┐
            │  efe-swarm-orchestrator              │
            │  ┌─────────────────────────────┐    │
            │  │ SwarmDagBuilder              │    │
            │  │ - add_agent                  │    │
            │  │ - add_dependency             │    │
            │  │ - add_conditional            │    │
            │  │ - with_aggregator            │    │
            │  └────────────┬─────────────────┘    │
            │               │ .build()             │
            │               ▼                      │
            │  ┌─────────────────────────────┐    │
            │  │ SwarmDag (immutable)         │    │
            │  │ - layers : Vec<Vec<AgentId>> │    │
            │  │ - conditional_edges          │    │
            │  │ - aggregator                 │    │
            │  └────────────┬─────────────────┘    │
            │               │                      │
            │               ▼                      │
            │  ┌─────────────────────────────┐    │
            │  │ SwarmOrchestrator::run       │    │
            │  │ - layer-by-layer dispatch    │    │
            │  │ - per-agent timeout          │    │
            │  │ - aggregator final           │    │
            │  └────────────┬─────────────────┘    │
            └───────────────┼──────────────────────┘
                            │
              uses           ▼            uses
   ┌──────────────────────────────────────────┐
   │  efe-blackboard                          │
   │  - Blackboard (DashMap, versioned)       │
   │  - EventBus (broadcast 256)              │
   │  - Observer trait                        │
   └────────────┬─────────────────────────────┘
                │ ContextBundle
                ▼
   ┌──────────────────────────────────────────┐
   │  efe-agentic-bridge                      │
   │  - AgenticBridge trait                   │
   │  - ClaudeBridge (reqwest, tool-use)      │
   │  - ContextBundle SDK type                │
   │  - ToolDefinition / ToolUseBlock         │
   └────────────┬─────────────────────────────┘
                │
                ▼
       Anthropic Claude Messages API
       /v1/messages with tool_use
```

### 7.2 `efe-blackboard` crate

```rust
// src/lib.rs

pub struct Blackboard {
    fields: DashMap<JsonPointer, BlackboardValue>,
    swarm_id: SwarmId,
    write_history: Mutex<Vec<BlackboardWrite>>,
}

pub struct BlackboardValue {
    pub value: serde_json::Value,
    pub version: u64,
    pub written_by: AgentId,
    pub written_at: SystemTime,
}

pub struct BlackboardWrite {
    pub pointer: JsonPointer,
    pub version: u64,
    pub agent_id: AgentId,
    pub at: SystemTime,
}

impl Blackboard {
    pub fn write(&self, pointer: JsonPointer, value: serde_json::Value, agent_id: AgentId)
        -> Result<u64, BlackboardError>;

    pub fn write_if(&self, pointer: JsonPointer, expected_version: u64,
                    new_value: serde_json::Value, agent_id: AgentId)
        -> Result<u64, BlackboardError>;

    pub fn read(&self, pointer: &JsonPointer) -> Option<BlackboardValue>;

    pub fn snapshot(&self) -> BlackboardSnapshot;

    pub fn estimated_size_bytes(&self) -> usize;
}

pub struct EventBus {
    tx: tokio::sync::broadcast::Sender<SwarmEvent>,
    capacity: usize,
    dropped_count: Arc<AtomicU64>,
}

pub struct SwarmEvent {
    pub swarm_id: SwarmId,
    pub published_by: AgentId,
    pub topic: String,
    pub payload: serde_json::Value,
    pub published_at: SystemTime,
}

impl EventBus {
    pub fn publish(&self, event: SwarmEvent) -> Result<(), EventBusError>;
    pub fn subscribe(&self) -> EventBusSubscription;
}

pub enum BlackboardError {
    Conflict { pointer: JsonPointer, expected_version: u64, actual_version: u64 },
    InvalidPointer { value: String },
    SizeLimitExceeded { current: usize, max: usize },
}
```

### 7.3 `efe-swarm-orchestrator` crate

```rust
// src/dag.rs

pub struct SwarmDagBuilder {
    nodes: HashMap<AgentId, AgentNode>,
    edges: Vec<DagEdge>,
    conditional_edges: Vec<ConditionalEdge>,
    aggregator: Option<AgentId>,
}

pub struct AgentNode {
    pub agent_id: AgentId,
    pub agent_ref: Arc<dyn SwarmAgent>,
    pub timeout_ms: u32,
    pub compliance: Compliance,
}

pub enum Compliance {
    None,
    N18Signed { publisher_id: PublisherId, key_id: KeyId },
    N19GoldGated { gold_set_id: GoldSetId },
}

pub struct DagEdge {
    pub from: AgentId,
    pub to: AgentId,
}

pub struct ConditionalEdge {
    pub from: AgentId,
    pub to: AgentId,
    pub predicate: Arc<dyn Fn(&Blackboard) -> bool + Send + Sync>,
    pub predicate_label: String,
}

impl SwarmDagBuilder {
    pub fn new() -> Self;
    pub fn add_agent(self, node: AgentNode) -> Self;
    pub fn add_dependency(self, from: AgentId, to: AgentId) -> Self;
    pub fn add_conditional<F>(self, from: AgentId, to: AgentId, label: String, predicate: F)
        -> Self where F: Fn(&Blackboard) -> bool + Send + Sync + 'static;
    pub fn with_aggregator(self, agent_id: AgentId) -> Self;
    pub fn build(self) -> Result<SwarmDag, SwarmDagError>;
}

pub struct SwarmDag {
    layers: Vec<Vec<AgentId>>,
    nodes: HashMap<AgentId, AgentNode>,
    conditional_edges: Vec<ConditionalEdge>,
    aggregator: AgentId,
}

pub enum SwarmDagError {
    Cycle { path: Vec<AgentId> },
    NoAggregator,
    AggregatorDisconnected,
    UnknownAgentInEdge { agent_id: AgentId },
    DuplicateAgent { agent_id: AgentId },
}

// src/agent.rs

#[async_trait]
pub trait SwarmAgent: Send + Sync + 'static {
    fn agent_id(&self) -> AgentId;
    async fn run(&self, ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError>;
}

pub struct SwarmAgentContext {
    pub swarm_id: SwarmId,
    pub blackboard: Arc<Blackboard>,
    pub eventbus: Arc<EventBus>,
    pub input: serde_json::Value,
}

pub enum AgentOutcome {
    Completed {
        cost_microcents: i64,
        duration_ms: u32,
        blackboard_writes: u32,
        events_published: u32,
    },
    Errored { kind: AgentErrorKind, message: String },
    TimedOut,
    Skipped { reason: String },
}

// src/runner.rs

pub struct SwarmOrchestrator {
    metrics: Arc<N21Metrics>,
    compliance_runner: Option<Arc<dyn ComplianceRunner>>,
}

impl SwarmOrchestrator {
    pub async fn run(&self, dag: &SwarmDag, input: serde_json::Value,
                     config: SwarmRunConfig) -> Result<SwarmReport, SwarmRunError>;
}

pub struct SwarmRunConfig {
    pub swarm_id: SwarmId,
    pub global_timeout_ms: u32,
    pub emit_envelope: bool,
    pub rng_seed: Option<u64>,
}

pub struct SwarmReport {
    pub swarm_id: SwarmId,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
    pub agents: Vec<AgentReport>,
    pub total_cost_microcents: i64,
    pub bundle: ContextBundle,
    pub overall: SwarmOverallVerdict,
}
```

### 7.4 `efe-agentic-bridge` crate

```rust
// src/lib.rs

pub struct ContextBundle {
    pub swarm_id: SwarmId,
    pub generated_at: SystemTime,
    pub entities: Vec<Entity>,
    pub language: Option<String>,
    pub redactions: Vec<Redaction>,
    pub summary: Option<String>,
    pub classifications: Vec<Classification>,
    pub extras: serde_json::Map<String, serde_json::Value>,
    pub metadata: ContextBundleMetadata,
}

pub struct ContextBundleMetadata {
    pub agents_executed: Vec<AgentId>,
    pub total_cost_microcents: i64,
    pub size_estimate_bytes: usize,
}

#[async_trait]
pub trait AgenticBridge: Send + Sync + 'static {
    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError>;
}

pub struct AgenticTask {
    pub system_prompt: String,
    pub user_message: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

pub struct AgenticResponse {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub model_identity: ModelIdentity,
    pub cost_microcents: i64,
}

pub struct ClaudeBridge {
    client: reqwest::Client,
    api_key: SecretString,
    base_url: String,
    default_model: String,
}

impl ClaudeBridge {
    pub fn new(api_key: SecretString) -> Self;
    pub fn with_endpoint(api_key: SecretString, base_url: String) -> Self;
    pub fn with_model(self, model: String) -> Self;
}

#[async_trait]
impl AgenticBridge for ClaudeBridge { /* ... */ }

pub enum BridgeError {
    RateLimited,
    ProtocolViolation { message: String },
    Backend { message: String },
    Authentication,
    BundleTooLarge { size_bytes: usize, max_bytes: usize },
}
```

### 7.5 `N21Metrics` (12 surfaces)

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n21_swarm_runs_total` | counter | `outcome` |
| 2 | `n21_swarm_run_duration_ms` | summary | `dag_layer_count` |
| 3 | `n21_swarm_run_cost_microcents_total` | counter | `tenant_tier` |
| 4 | `n21_agent_invocations_total` | counter | `agent_id`, `outcome` |
| 5 | `n21_agent_duration_ms` | summary | `agent_id` |
| 6 | `n21_agent_timeouts_total` | counter | `agent_id` |
| 7 | `n21_blackboard_writes_total` | counter | `agent_id` |
| 8 | `n21_blackboard_conflicts_total` | counter | `pointer_class` |
| 9 | `n21_eventbus_published_total` | counter | `topic` |
| 10 | `n21_eventbus_dropped_total` | counter | `reason` |
| 11 | `n21_claude_calls_total` | counter | `outcome` |
| 12 | `n21_context_bundle_size_bytes` | summary | `dag_id` |

---

## 8. Module hierarchy & data model

### 8.1 Crate layout

```text
crates/
├── efe-blackboard/                       (NEW)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── blackboard.rs
│   │   ├── eventbus.rs
│   │   ├── observer.rs
│   │   ├── snapshot.rs
│   │   └── error.rs
│   └── Cargo.toml
│
├── efe-swarm-orchestrator/                (NEW)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── dag/
│   │   │   ├── mod.rs
│   │   │   ├── builder.rs
│   │   │   ├── validation.rs
│   │   │   └── kahn.rs (topological sort)
│   │   ├── agent.rs
│   │   ├── runner/
│   │   │   ├── mod.rs
│   │   │   ├── layer.rs
│   │   │   └── aggregator.rs
│   │   ├── compliance.rs
│   │   ├── report.rs
│   │   └── metrics.rs
│   └── Cargo.toml
│
├── efe-agentic-bridge/                    (NEW)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── bundle.rs
│   │   ├── bridge.rs (trait)
│   │   ├── claude.rs (ClaudeBridge impl)
│   │   ├── tool_use.rs (Anthropic schema)
│   │   └── error.rs
│   └── Cargo.toml
│
└── efe-nano/
    └── src/
        └── agents/
            └── preanalysis/                (NEW — 6 reference agents)
                ├── mod.rs
                ├── ner_extractor.rs
                ├── sentiment_classifier.rs
                ├── language_detector.rs
                ├── pii_redactor.rs
                ├── summary_writer.rs
                └── schema_validator.rs
```

### 8.2 SDK types added to `efe-nano-sdk`

```rust
// efe-nano-sdk/src/types/swarm_dag.rs (NEW Sprint 13)

pub struct SwarmId(pub Uuid);
pub struct AgentId(pub String);

pub struct ContextBundle { /* exported here for cross-crate */ }
pub struct ContextBundleMetadata { /* exported */ }
pub struct Entity { /* exported */ }
pub struct Redaction { /* exported */ }
pub struct Classification { /* exported */ }
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints

```text
POST /v1/swarms/run
    Body: { dag_id: UUID, input: <json>, config: SwarmRunConfig }
    Auth: tenant JWT
    → 200 OK SwarmReport + ContextBundle
    → 408 if global_timeout_ms exceeded

POST /v1/swarms/dag
    Body: SwarmDagDescription (declarative JSON)
    → 201 Created { dag_id }
    → 400 if cycle / disconnected aggregator

GET /v1/swarms/{swarm_id}/report
    → 200 OK SwarmReport

POST /v1/swarms/{swarm_id}/agentic
    Body: AgenticTask { system_prompt, user_message, ... }
    → 200 OK AgenticResponse

GET /v1/agentic/bridge/status
    → 200 OK BridgeStatus { breaker_state, last_call_at, ... }
```

### 9.2 Internal Rust traits

`SwarmAgent`, `AgenticBridge`, `ComplianceRunner` (see § 7.3 / 7.4).

---

## 10. End-to-end workflows

### 10.1 Workflow A — PDF contract analyser

```
1. User POSTs PDF + asks for risk analysis.
2. API constructs SwarmDag :
     Layer 1 (parallel) :
       - extract_text (Mode A, PDF → text)
       - detect_language (Mode A)
       - extract_entities (Mode A, NER)
       - schema_check (Mode A)
     Layer 2 (parallel, depends Layer 1) :
       - if language != "en" → translate (Mode C, Haiku)
       - pii_redactor (Mode A)
       - classify_clauses (Mode B, local BERT)
       - compute_summary (Mode C, Haiku)
     Layer 3 (sequential, depends Layer 2) :
       - aggregate_risk_score (Mode A, deterministic rule)
     Layer 4 :
       - context_aggregator (the Aggregator)
3. SwarmOrchestrator.run(dag, input, config) :
   a. Layer 1 spawns 4 tasks parallel ; all write to blackboard.
   b. Layer 2 spawns 4 tasks parallel ; checks predicates ; some
      skip if blackboard says skip.
   c. Layer 3 runs aggregate_risk_score.
   d. Layer 4 runs context_aggregator → ContextBundle.
4. ClaudeBridge.call_with_context(bundle, task) :
   a. Sends to Claude /v1/messages with tool_use spec.
   b. Claude calls swarm_context tool → bridge returns bundle.
   c. Claude reasons + returns "Risk : 7/10. Clauses 4, 12, 17
      are high-risk indemnity ; recommend legal review."
5. API returns AgenticResponse to user.
```

### 10.2 Workflow B — Support ticket triage

```
1. New support ticket arrives.
2. Swarm :
     Layer 1 : language_detector + sentiment_classifier + extract_topic
     Layer 2 :
       - if sentiment.confidence < 0.7 → re-classify with bigger model
       - if pii_found → redact
       - lookup_kb (custom agent)
     Layer 3 : aggregator
3. Claude (agentic) decides : route to L1, L2, or auto-close.
```

### 10.3 Workflow C — Compliance opt-in dispatch

```
1. Operator builds DAG with compliance per agent :
   - ner_extractor : Compliance::None (low risk)
   - pii_redactor : Compliance::N18Signed { ... } (must verify pub)
   - claude_summary : Compliance::N19GoldGated { gold_set: ... }
2. Orchestrator routes through compliance_runner if compliance != None.
3. N18 verifies sig before dispatch.
4. N19 gold_run_id recorded.
5. Both audit chain rows committed.
```

### 10.4 Workflow D — Replay determinism

```
1. CI test : same DAG + same input + same seed → run twice.
2. Compare bundles byte-for-byte.
3. assert_eq!(bundle_1, bundle_2).
4. If diff → fail-test : a predicate or agent is impure.
```

### 10.5 Workflow E — Agent failure best-effort

```
1. Layer 2 has 4 agents.  classify_clauses crashes (panic, OOM).
2. AgentOutcome::Errored { kind: Crash, message: "panic" } recorded.
3. Layer 3 starts ; aggregate_risk_score depends on classify_clauses.
4. Since predicate edge says "if classifications is empty, skip
   risk_score", aggregate_risk_score skips.
5. Aggregator runs with partial blackboard ; bundle has
   `classifications: []`.
6. SwarmReport.overall = PartialSuccess.
```

---

## 11. Five-day implementation plan

### Day 51 — `efe-blackboard`
* Blackboard (DashMap + versioning) — ~400 LOC.
* EventBus (broadcast 256) — ~200 LOC.
* Observer trait — ~100 LOC.
* 35+ unit tests.

### Day 52 — `efe-swarm-orchestrator` DAG layer
* SwarmDagBuilder + cycle detection (Kahn) — ~500 LOC.
* SwarmDag immutable type — ~200 LOC.
* AgentNode + Compliance enum — ~150 LOC.
* 30+ unit tests.

### Day 53 — SwarmOrchestrator runtime
* Layer scheduler + tokio::spawn + per-agent timeout — ~600 LOC.
* AgentOutcome aggregation — ~150 LOC.
* N15 fairness wiring (`OperationClass::SwarmAgent`) — ~100 LOC.
* 25+ unit tests.

### Day 54 — `efe-agentic-bridge`
* ClaudeBridge impl + reqwest — ~400 LOC.
* ContextBundle type + 256 KB cap — ~250 LOC.
* Tool-use shape + Anthropic schema — ~300 LOC.
* 30+ unit tests (wiremock).

### Day 55 — 6 reference agents + 15 acceptance tests + metrics
* `ner_extractor` — ~200 LOC.
* `sentiment_classifier` — ~250 LOC.
* `language_detector` (whatlang) — ~80 LOC.
* `pii_redactor` (regex + aho-corasick) — ~300 LOC.
* `summary_writer` (Claude Haiku via bridge) — ~150 LOC.
* `schema_validator` (jsonschema) — ~120 LOC.
* `tests/phase_6_n21_swarm_orchestration.rs` — 15 acceptance tests.
* `N21Metrics` 12 surfaces.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (15)

```
phase_6_n21_swarm_dag_cycle_detected_at_build
phase_6_n21_layers_execute_in_order
phase_6_n21_conditional_skip_on_predicate_false
phase_6_n21_blackboard_concurrent_writes_serialised
phase_6_n21_eventbus_slow_consumer_drops_old
phase_6_n21_swarm_report_attributes_cost_per_agent
phase_6_n21_claude_bridge_tool_use_round_trip
phase_6_n21_context_bundle_exceeds_256kb_rejected
phase_6_n21_one_agent_failure_swarm_completes_partial
phase_6_n21_per_agent_timeout_fires_independently
phase_6_n21_six_reference_agents_loadable
phase_6_n21_envelope_traces_per_agent
phase_6_n21_compliance_opt_in_per_agent_works
phase_6_n21_replay_produces_identical_bundle
phase_6_n21_metrics_emit_all_12_surfaces
```

### 12.2 Property tests
* `proptest_dag_topological_sort_returns_valid_order`
* `proptest_blackboard_write_then_read_returns_same_value`
* `proptest_context_bundle_serde_round_trip`
* `proptest_predicate_purity` (predicates called 10× must return
  same value)

### 12.3 Chaos tests
* `chaos_agent_panic_isolated_from_swarm`
* `chaos_global_timeout_aborts_all`
* `chaos_eventbus_capacity_drops_slow_consumer`
* `chaos_claude_429_retries_then_errors`

### 12.4 Security tests
* `security_pii_redactor_runs_before_claude`
* `security_compliance_n18_invalid_sig_fails_closed`
* `security_blackboard_size_limit_prevents_DoS`

### 12.5 Cross-cutting smoke tests
* `cross_n11_n21_firecracker_microvm_per_agent`
* `cross_n14_n21_envelope_carries_swarm_id`
* `cross_n15_n21_fairness_quota_enforced`
* `cross_n17_n21_provenance_attributes_to_agent_in_swarm`
* `cross_n18_n21_signed_agent_verified_before_dispatch`
* `cross_n19_n21_gold_gated_agent_promoted_first`

---

## 13. Cross-cutting alignment (N11 → N20)

| N | Concern | N21 resolution |
|---|---|---|
| N11 | Per-agent in microVM (optional) | `Compliance::N18Signed` triggers VmPool acquire |
| N12 | LLM call inside swarm cached | cache key includes `(swarm_id, agent_id)` |
| N13 | New canary surface | `SwarmDag.dag_id` carries tier metadata |
| N14 | Envelope per agent | opt-in via `SwarmRunConfig.emit_envelope = true` |
| N15 | Fairness quota | new `OperationClass::SwarmAgent` |
| N16 | Cross-region swarm | deferred Sprint 15 |
| N17 | LLM provenance | every Claude call records `LlmProvenanceRecord` |
| N18 | Publisher sig verify | opt-in per agent via `Compliance::N18Signed` |
| N19 | Gold gate per agent | opt-in via `Compliance::N19GoldGated` |
| N20 | Postgres + multi-provider | the `ClaudeBridge` and stub providers coexist |

---

## 14. Sprint 13 ship vs Sprint 14/15 split

### Sprint 13 (this design)
* All 8 production capabilities.
* 15 acceptance + 4 property + 4 chaos + 3 security + 6 cross-cutting tests.
* 6 reference pre-analysis agents.

### Sprint 14 N22 — Multi-provider agentic + caching
* GPT-4 bridge + Mistral bridge + LocalLlama bridge (all implement
  `AgenticBridge`).
* Cross-swarm pre-analysis cache (skip re-running same agents on
  same input).
* Swarm result diff viewer in operator portal.

### Sprint 15 GA — Cross-region swarm + Sigstore release
* N16 cross-region envelope shipment of swarm runs.
* Vector-store-backed retrieval-augmented pre-analysis.
* v1.0 GA release with Sigstore signing.

---

## 15. Annexes

### A1 — Cargo dependencies added in Sprint 13

```toml
# efe-blackboard
dashmap = "6.1"
tokio = { workspace = true, features = ["sync"] }

# efe-swarm-orchestrator
futures = "0.3"
async-trait = "0.1"

# efe-agentic-bridge
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
secrecy = "0.10"

# efe-nano/preanalysis
whatlang = "0.16"
aho-corasick = "1.1"
jsonschema = "0.18"
```

### A2 — Anthropic Messages API call shape

```http
POST https://api.anthropic.com/v1/messages
Headers :
  x-api-key: <secret>
  anthropic-version: 2023-06-01
  content-type: application/json

Body :
{
  "model": "claude-sonnet-4-6",
  "max_tokens": 4096,
  "system": "You have a tool `swarm_context`...",
  "tools": [{
    "name": "swarm_context",
    "description": "Returns pre-analysed structured context",
    "input_schema": {"type": "object", "properties": {}}
  }],
  "messages": [
    {"role": "user", "content": "Analyse this contract for risk."}
  ]
}

Response (turn 1) :
{
  "content": [{"type": "tool_use", "id": "...", "name": "swarm_context", "input": {}}],
  "stop_reason": "tool_use"
}

Response (turn 2, after tool_result) :
{
  "content": [{"type": "text", "text": "Risk score 7/10..."}],
  "stop_reason": "end_turn"
}
```

### A3 — ContextBundle wire format

```json
{
  "swarm_id": "uuid",
  "generated_at": 1716300000,
  "entities": [
    {"type": "person", "value": "John Doe", "span": [120, 128]},
    {"type": "organisation", "value": "ACME Corp", "span": [200, 209]}
  ],
  "language": "en",
  "redactions": [
    {"type": "email", "original_span": [400, 420], "redaction_token": "<EMAIL_0>"}
  ],
  "summary": "Contract between ACME and XYZ for 12-month consulting...",
  "classifications": [
    {"label": "consulting", "confidence": 0.92}
  ],
  "extras": {"clauses": [...], "risk_score": 0.78},
  "metadata": {
    "agents_executed": ["ner_extractor", "language_detector", "pii_redactor"],
    "total_cost_microcents": 4280,
    "size_estimate_bytes": 1837
  }
}
```

### A4 — Operator runbook

```bash
# Run a swarm
efe-swarm run --dag-id pdf-analyser --input @contract.pdf

# Inspect a swarm report
efe-swarm report <swarm_id>

# Disable an agent globally (operator override)
efe-swarm disable-agent ner_extractor --reason "high false-positive"

# Inspect Claude bridge breaker
efe-swarm bridge status
```

### A5 — Comparison with related frameworks

| Framework | Their approach | N21 improvement |
|---|---|---|
| LangChain | sequential chains | + DAG, parallel layers, conditional |
| CrewAI / AutoGen | role-based multi-agent | + audit chain, compliance opt-in, no LLM-driven planning loop |
| Haystack | RAG-focused pipelines | + agentic bridge, swarm replay determinism |
| Anthropic Tool Use docs | single tool per call | + multi-agent pre-analyser feeding into tool_use |
| Inngest / Temporal | workflow engines | + LLM-native, Anthropic-aware |

---

**End of N21 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 13 (Phase 5, day 51 – 55).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 + N17 + N18 +
N19 + N20 → **N21** (this doc) → N22 multi-provider agentic + cache →
Sprint 15 GA v1.0 Sigstore.

**Last-mile checklist** (Sprint 13 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on 3 new crates.
- [ ] 6 reference pre-analysis agents shipped + tested.
- [ ] ClaudeBridge tool-use round-trip verified via wiremock.
- [ ] Blackboard 256 KB cap pen-tested with crafted inputs.
- [ ] Replay determinism CI test green on 10 distinct DAGs.
- [ ] N18 + N19 compliance opt-in verified per agent.
- [ ] Cross-region deferred to Sprint 15 (documented in § 14).
- [ ] Anthropic partnership team reviewed tool-use shape.
- [ ] Operator runbook A4 reviewed by ops team.
