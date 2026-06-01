# EFE M50 — N19 Gold-Test Gate TIER-1 Architecture

> **Sprint** : 11 (Phase 4, day 41 – 45 of M50).
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
>
> **Anchors** : Sprint 1 catalog (`StabilityTier::Tier1` was a marker
> with no enforcement gate).  N13 (Sprint 8) wired shadow / canary
> rollout policy.  N18 (Sprint 10) wired Ed25519 signatures on
> publishes.  **N19 closes the loop** : a published & signed agent now
> only reaches `Tier1` after a *Gold-test gate* run passes
> end-to-end against a curated, operator-approved, audit-chained
> reference test suite.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N19 closes (P1–P15)](#2-fifteen-incident-patterns-n19-closes-p1p15)
3. [Commitments — what ships in Sprint 11 (G1–G15)](#3-commitments--what-ships-in-sprint-11-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N12 → N18)](#13-cross-cutting-alignment-n12--n18)
14. [Sprint 11 ship vs Sprint 12/13 split](#14-sprint-11-ship-vs-sprint-1213-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The problem.**  `StabilityTier::Tier1` is the wall between "the
publisher just shipped a thing" (`TierExperimental`) and "EFE
operationally trusts this thing enough to canary-route real customer
traffic" (`Tier1`).  Today, the boundary is *enforced by an SQL UPDATE
ran by an operator* :

- **No reference test suite** : a Tier 1 promotion is approved by eyeballing
  a Slack thread and clicking a button.
- **No regression detection** : a publisher can ship a "fix" that
  silently changes the agent's output distribution and nobody finds out
  until a customer complains.
- **No equivalence model for non-determinism** : Mode A (pure Rust)
  needs byte-equality, Mode B (local model) needs ε-tolerance, Mode C
  (LLM) needs semantic similarity — and we have zero infrastructure for
  any of these.
- **No audit trail of the promotion event** : there is a row in the
  N6 chain saying "Tier1 set" but no proof that a quality bar was met.
- **No replay anchoring** : a Tier 1 agent published Q1 has no
  observable reference behaviour we can re-run Q3 to detect drift.
- **No bypass model** : when a Tier 1 agent has a sev-1 bug and we
  need a hot-fix in 15 minutes, the only path today is "skip the
  reviewer entirely" — which we've done twice in the last two months
  (Incidents I-2026-06-28, I-2026-07-02) and which destroys the entire
  audit story.

Five real incidents documented in the Sprint 10 retro (§ 1.1) motivate
making the Tier 1 gate a *cryptographically auditable, semantically
typed, replay-anchored, operator-overridable* mechanism — the
**Gold-test gate**.

**The fix.**  N19 introduces the **`GoldSet`** (an immutable,
versioned, operator-approved bundle of `GoldCase`s : input + expected
output + assertion strategy) and the **`GoldGate`** (a runner +
verdict + audit + metrics pipeline that runs the GoldSet against a
candidate version, in hermetic isolation, and emits a non-repudiable
report).  Eleven strict guarantees ship :

1. **`GoldCase` SDK type** with three first-class assertion
   strategies — `StrictEquality` (Mode A), `NumericTolerance` (Mode B,
   explicit ε per field), `SemanticSimilarity` (Mode C, fixed
   embedding model + cosine threshold).
2. **`GoldSet` immutable + versioned** — each agent version pins
   exactly one `GoldSet` version.  Modifications produce a NEW gold
   set version (never an in-place edit).
3. **Operator approval ceremony** — a `GoldSet` only becomes
   *executable as a gate* once 2-of-3 EFE engineers + a ticket
   reference sign it.
4. **`GoldGate` runs in hermetic Firecracker microVMs** (reuses N11
   pool) — no network, no clock skew, no host filesystem access.
5. **Mandatory at Tier 1 promote** — `RolloutPolicy::promote_to(
   Tier1)` calls `GoldGate::run(agent, gold_set)` and rejects if any
   case fails.  No SQL backdoor : the N18 publish ledger's
   `AgentPromotedToTier1` entry includes the `gold_run_id`.
6. **Cost-bounded** — each gold run has a configurable cost cap
   (default 100 USD ; Tier 3 publishers can raise to 1 000 USD per
   ceremony) ; LLM calls are cache-aware (N12) and replay-anchored
   (N16) to drive marginal cost to near zero on re-runs.
7. **HMAC-chained gold-run ledger** — every `GoldGateRun` is a row in
   a dedicated chain (mirrors N18 publish ledger), independently
   verifiable by `efe-gold-verify`.
8. **Replay-anchored** — a `GoldCase` MAY reference an N16 trace as
   its expected output, in which case the gold run re-executes the
   trace via the N16 replay engine and compares.
9. **Drift detection** — a scheduled re-run (every 30 days) re-runs
   every Tier 1 agent against its gold set.  Failure demotes the
   agent to `TierExperimental` + emits a `GoldDriftDetected` audit
   row.
10. **Emergency bypass workflow** — a `BypassCertificate` (2-of-3
    EFE eng sigs + ticket) lets a promote skip the gate.  Every bypass
    is a separate audit row tagged with the operator team's
    rationale ; bypassed agents are flagged in the catalog.
11. **Cross-cutting with N17 + N18** — every gold case execution
    emits an `LlmProvenanceRecord` (if Mode C) AND the `gold_run_id`
    surfaces in the N18 publish ledger entry that records the Tier 1
    promotion.

**Six capabilities ship in Sprint 11 N19** :

1. **`GoldCase`** + **`GoldSet`** + **`GoldSetId`** + **`GoldAssertion
   Strategy`** + **`GoldVerdict`** + **`GoldReport`** + **`BypassCertif
   icate`** SDK types.
2. **`GoldRunner`** trait + `FirecrackerGoldRunner` impl (uses N11
   pool) + `InMemoryGoldRunner` impl (tests).
3. **`GoldSetRegistry`** trait + `InMemoryGoldSetRegistry` impl —
   register, approve, lookup, list-for-agent, archive.
4. **`GoldRunLedger`** trait + `InMemoryGoldRunLedger` impl — HMAC-
   chained ledger of gold runs + bypasses.
5. **`GoldGate`** orchestrator — wires registry + runner + ledger +
   metrics into the `promote_to_tier_1` flow.
6. **`AssertionEngine`** with three strategies + `EmbeddingProvider`
   trait + `StubEmbeddingProvider` (Sprint 12 wires OpenAI / Cohere).

**Out of scope (deferred to Sprint 12 N20 / Sprint 13 N21)** :

- Real embedding providers (OpenAI text-embedding-3-small, Cohere
  embed-multilingual-v3, sentence-transformers/all-MiniLM-L6-v2).
  Sprint 11 ships the `EmbeddingProvider` trait + a `Stub` that
  returns deterministic hashed vectors so the wire format is frozen.
- Automated *gold case discovery* — Sprint 11 requires publishers to
  hand-curate cases.  Sprint 13 N21 introduces "shadow capture" : the
  N13 shadow rollout proposes cases for operator review.
- *Cross-version diff* — comparing two gold runs (v1 vs v2) and
  flagging behavioural deltas — Sprint 12.
- *GoldCase mutation testing* — Sprint 13.
- *Gold gate parallelism budget tuning* — Sprint 12 wires N15
  fairness scheduler tier-aware queuing for gold runs.

**Backward compatibility.**  N19 changes are strictly additive :

- `NanoAgent.gold_set_ref: Option<GoldSetRef>` field added (None for
  pre-N19 ; the gate accepts None as `gold_run.cases_executed = 0` +
  emits a `GoldSetMissing` warning).
- `StabilityTier::Tier1` semantics : promotion to Tier 1 now requires
  a passing gold run OR a bypass.  Pre-N19 Tier 1 agents are
  grandfathered for 90 days (`grandfathered_until: SystemTime`) ; after
  that, the 30-day drift sweep demotes them.
- New `OperationErrorKind::GoldGateFailed { run_id, failed_cases }` +
  `GoldSetMissing` + `BypassRequired` variants.
- N18 publish ledger gains a new entry kind
  `PublishLedgerEntry::AgentPromotedToTier1 { agent_id, version,
  gold_run_id, bypass: Option<BypassCertificate> }`.
- N17 LlmProvenanceRecord gains an optional
  `gold_case_id: Option<GoldCaseId>` so gold-run-driven LLM calls are
  attributable to the case that triggered them.

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N19

**Incident I-2026-05-30 — Silent prompt-template regression.**
A Tier 1 publisher shipped v2.1.0 of `summarize_strict` after
refactoring their prompt template.  The output schema didn't change,
the smoke test passed, but downstream consumers' classifiers (which
fed `summarize_strict`'s output into a topic classifier) saw a 12 %
distribution shift over the next 48 h.  Root cause : the new prompt
caused the model to emit slightly more verbose summaries, breaking a
downstream length-based heuristic.  Detection took 3 days ; no
mechanism existed to compare v2.1.0 against v2.0.x on a canonical
reference set.  Fix took 6 hours of rollback + comms.

**Incident I-2026-06-15 — Numeric drift on Mode B local model.**
A Phase 4 partner pinned a fine-tuned BERT classifier (Mode B, local
weights) as `Tier1`.  Sprint 9 deployment of the same model on a new
GPU SKU caused float ordering nondeterminism that changed about
0.4 % of predictions (from `positive` to `neutral`).  N17 provenance
showed identical model hash + identical prompts → "nothing changed",
yet outputs changed.  No tolerance band was specified anywhere.

**Incident I-2026-06-22 — Tier 1 promotion via SQL UPDATE.**
A new EFE ops engineer promoted a community-published agent to Tier 1
by running `UPDATE catalog.entries SET stability_tier='Tier1' WHERE
id='extract_table_v3'`.  The agent had passed exactly zero objective
checks ; the engineer assumed the publisher's own README test cases
were sufficient.  Discovered 11 days later when a customer's pipeline
returned mangled tables.  Audit chain showed the update but no
quality evidence.

**Incident I-2026-06-28 — LLM provider migration broke gold-like
behaviour without anyone noticing.**
Anthropic silently migrated `claude-3-haiku` to `claude-3-haiku
-20240307` and back over the course of a week (provider rollout
incident on their side).  Three Tier 1 EFE agents that used Haiku saw
their refusal rates spike from 0.2 % to 1.7 % during the migration
window.  We had N17 provenance recording the version mismatch ; we did
NOT have a reference test set that would have flagged the behavioural
delta automatically.

**Incident I-2026-07-02 — Hot-fix bypassed all checks.**
A Sev-1 prod incident required pushing a hot-fix to a Tier 1 agent
within 30 minutes.  Operator skipped the entire informal review
process to ship fast.  Customers got the fix in 25 min ; *but the
audit chain has no record of which engineer authorised the bypass
and on what evidence*.  Post-mortem flagged this as the single
biggest forensic gap of Sprint 10.

### 1.2 Why Sprint 11, immediately after N18

N18 (Publisher Identity) gave us cryptographic non-repudiation of
"who" published a version.  N19 gives us cryptographic non-repudiation
of "did this version pass a quality bar".  Together they close the
forensic loop : `EU regulator → trace_id → agent_id → publisher_id →
gold_run_id → operator_approval`.

Sprint 11 ordering :

| Day | Deliverable | Depends on |
|---|---|---|
| 41 | SDK types + GoldSet registry | N18 OperatorSignature, N11 ResourceLimits |
| 42 | GoldRunner trait + Firecracker impl | N11 pool, N18 SignatureVerifier (for hermetic verification of agent body) |
| 43 | AssertionEngine + 3 strategies | N17 LlmProvenanceRecord, N16 ReplayEngine |
| 44 | GoldGate orchestrator + N18 ledger integration | N18 PublishLedger |
| 45 | Integration tests + metrics + drift sweep | N15 fairness scheduler |

### 1.3 Who consumes N19 once it ships

| Consumer | Use case |
|---|---|
| **Compliance team** | Demonstrate to auditors that every Tier 1 agent passed a documented quality bar with operator approval |
| **Ops team** | Promote-to-Tier-1 portal shows gold-run report inline ; reject button if fail |
| **Publishers** | Authoring portal for gold sets ; preview-run before submitting for approval |
| **Customer success** | "Why was your agent demoted?" → point at drift sweep report |
| **Security team** | Bypass log review (must be < 10 / month per policy) |
| **N13 canary** | Will only route customer traffic to a Tier 1 agent whose latest gold run passed within drift window |
| **N16 replay** | A replay over Tier 1 traffic can cross-check against the agent's gold-run outputs |
| **N17 provenance** | Gold-case-id surfaces in `LlmProvenanceRecord.gold_case_id` |
| **N18 publish ledger** | `AgentPromotedToTier1` entry references the gold run |
| **M31 billing** | Tier 1 publishers can be billed for gold-run compute (Sprint 13) |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns clustered into 5 families :

* **Regression detection** (P1, P2, P3, P4) — 4 patterns
* **Quality bar definition** (P5, P6, P7) — 3 patterns
* **Equivalence model** (P8, P9, P10) — 3 patterns
* **Lifecycle & drift** (P11, P12, P13) — 3 patterns
* **Forensic & override** (P14, P15) — 2 patterns

---

## 2. Fifteen incident patterns N19 closes (P1–P15)

> **Methodology** : every P-row sources from (a) a real incident with
> ticket reference, (b) a regulator-level requirement, or (c) a
> Sprint 10 retro-flagged near-miss.  No speculative rows.

### Family A — Regression detection

**P1. Silent semantic regression on prompt-template refactor.**
Publisher refactors a Mode C agent's prompt template ; output schema
is unchanged but distribution shifts (Incident I-2026-05-30).
Existing CI is unit-test scoped — no end-to-end reference test fires.
N19's `SemanticSimilarity` strategy on a frozen GoldSet catches the
shift : the cosine similarity between v2.0.x and v2.1.0 outputs on
the same gold inputs falls below the threshold → promote rejected.

**P2. Output-schema-conformant but values-wrong regression.**
A `extract_table_v3` agent returns valid JSON that matches the
schema (so the schema-validation post-condition passes) but the
extracted *values* changed — column 3 became "N/A" instead of the
actual number.  Schema validation is necessary but not sufficient.
N19's `StrictEquality` (Mode A) or `NumericTolerance` (Mode B)
catches this case by binding expected exact / ε-bounded values.

**P3. Numeric drift on Mode B model migration.**
A pinned local model behaves differently on a new GPU SKU — float
ordering changes 0.4 % of decisions (Incident I-2026-06-15).  N19's
`NumericTolerance { epsilon: 0.01 }` applied per output field rejects
the migration if drift exceeds the band.

**P4. LLM provider silent migration drift.**
Provider migrates `claude-3-haiku → claude-3-haiku-20240307` mid-flow
(Incident I-2026-06-28).  N19's gold set, re-run on a 30-day drift
sweep, detects that the cosine similarity to the original outputs
fell from 0.98 to 0.86 → demotion fires + alert.

### Family B — Quality bar definition

**P5. No reference test set exists.**
Tier 1 is an operator click (Incident I-2026-06-22).  N19 makes a
GoldSet a *prerequisite* for the promote endpoint to even accept the
request.  No GoldSet → `BadRequest("Tier 1 promotion requires a
GoldSet")`.

**P6. Inconsistent promotion criteria across publishers.**
Some publishers had 10 internal tests, others had 0.  Tier 1 meant
different things for different agents.  N19 sets a *floor* : ≥ 5
gold cases, ≥ 1 case per declared output mode, ≥ 1 PII-sensitive
case if the agent compliance tags say so.

**P7. Ad-hoc gold sets that drift from agent semantics.**
A publisher hand-wrote 20 test cases for v1.0.  By v2.5, half are
stale (input schema changed, etc.) and silently skipped by ad-hoc
runners.  N19's `GoldSet.agent_input_schema_hash` is checked at run
time ; if it doesn't match the current agent's input schema, the
gold run fails-closed with `StaleGoldSet`.

### Family C — Equivalence model

**P8. Mode A / B / C have incompatible equality models.**
Mode A demands byte-for-byte equality (deterministic).  Mode B
demands ε-tolerance (float reordering on GPU).  Mode C demands
semantic similarity (LLM nondeterminism).  N19 has three first-class
strategies, each with its own SDK type, picked per `GoldCase`.

**P9. Fuzzy-match false positives kill operator trust.**
A naive semantic-similarity threshold at 0.85 marks every clean Mode
C output as "passing" because the noise floor is 0.83.  N19's
`SemanticSimilarity` has a *paired* threshold : the threshold MUST be
above the noise floor measured on the same agent over N parallel
runs of the same case (replay-anchored).  Sprint 11 ships a stub
that requires manual threshold setting ; Sprint 12 auto-computes.

**P10. Cost overrun on LLM gold tests.**
A Mode C agent with 50 gold cases × 3 retries each × $0.50 / call =
$75 per gold run.  At one run per promote + 30-day drift × 200
agents = $30k / month.  N19's `cost_cap_usd` per `GoldSet` rejects
the run if exceeded ; cache (N12) + replay (N16) drive the marginal
cost on re-runs to near zero (cached calls don't bill).

### Family D — Lifecycle & drift

**P11. Gold set staleness over agent lifetime.**
A GoldSet approved for v1.0.0 keeps being run against v1.5.0.  The
input schema added a field ; the gold cases don't supply it ; the
agent uses a default ; outputs match.  *Effectively, the new
behaviour path is never tested.*  N19's `agent_input_schema_hash`
binding rejects this.

**P12. Gold set drift over time even with unchanged agent.**
An LLM-backed Tier 1 agent's behaviour shifts as the provider's
model is silently updated (P4).  N19 schedules a re-run every 30
days.  Failed re-run = demote + alert + force re-approval.

**P13. Emergency bypass with no audit trail.**
Sev-1 hot-fix bypasses review (Incident I-2026-07-02).  N19's
`BypassCertificate` is signed by 2-of-3 EFE engineers, carries a
ticket ID, and lives forever in the gold-run ledger.  Catalog UI
flags bypassed agents in red until the next clean run.

### Family E — Forensic & override

**P14. No audit row of "the gold gate ran".**
Today there's no row at all.  N19 emits a `GoldGateRun` ledger entry
per gold run, with `verdict`, `cases_executed`, `cases_passed`,
`cost_usd_actual`, `total_duration_ms`, and a vec of per-case
mini-reports.

**P15. Operator override without two-person rule.**
A single operator could approve a bypass.  N19 enforces 2-of-3 EFE
engineers on `BypassCertificate.approvers : Vec<OperatorSignature>` ;
verified at gate-time.

---

## 3. Commitments — what ships in Sprint 11 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### Identity & integrity

**G1. GoldSet immutable + content-addressed.**
A `GoldSet` is identified by `GoldSetId(Uuid)` ; its body is hashed
(BLAKE3-256) into `gold_set_hash: Sha256Hash` (reuses N18's hash
newtype).  Any edit produces a NEW `GoldSet` ; the old one stays
forever.  Acceptance : `phase_4_n19_gold_set_immutable_after_approve`.

**G2. GoldSet operator-approval ceremony.**
A `GoldSet` is `Draft` until `2-of-3 EFE engineers` sign it via
`OperatorSignature`s (reuses N18 type).  Acceptance :
`phase_4_n19_gold_set_requires_two_approvers`.

**G3. Gold-run ledger HMAC-chained.**
Every gold run + bypass emits an entry in a dedicated HMAC chain
(mirrors N18 `PublishLedger`).  Acceptance :
`phase_4_n19_gold_run_ledger_chain_verifies`.

### Equivalence

**G4. Three first-class assertion strategies.**
`StrictEquality` (Mode A), `NumericTolerance { epsilon, fields }`
(Mode B), `SemanticSimilarity { embedding_model, threshold_cosine }`
(Mode C).  Acceptance :
`phase_4_n19_three_strategies_decide_pass_fail`.

**G5. JSON schema validation on every gold case.**
Even strict-equality cases run the agent output through the agent's
declared `output_schema` first — schema-conformance is mandatory.
Acceptance : `phase_4_n19_output_schema_violation_fails_case`.

### Gate enforcement

**G6. Promote-to-Tier-1 mandatory gate.**
`RolloutPolicy::promote_to(Tier1)` calls `GoldGate::run(...)` and
rejects on failure.  No SQL backdoor : N18 publish ledger entry
`AgentPromotedToTier1` requires a `gold_run_id`.  Acceptance :
`phase_4_n19_promote_tier_1_requires_passing_gold_run`.

**G7. Hermetic isolation via N11 microVMs.**
Gold cases run in Firecracker microVMs from the N11 pool — no
network, no clock skew, no host FS access.  Acceptance :
`phase_4_n19_gold_run_isolated_in_microvm`.

### Cost / cache

**G8. Cost-bounded with hard cap.**
`GoldSet.cost_cap_usd: f64` ; runner stops + emits
`CostCapExceeded` if cumulative cost > cap.  Default 100 USD ;
Tier 3 publishers may raise via operator override.  Acceptance :
`phase_4_n19_cost_cap_aborts_run`.

**G9. Cache- and replay-aware.**
LLM calls go through N12 cache + N16 replay-anchored execution.
Cached calls don't bill.  Acceptance :
`phase_4_n19_cached_calls_drive_marginal_cost_to_zero`.

### Lifecycle

**G10. 30-day drift sweep.**
Scheduled job re-runs every Tier 1 agent's gold set every 30 days.
Failure → demote to `TierExperimental` + audit row.  Acceptance :
`phase_4_n19_drift_sweep_demotes_on_fail`.

**G11. Operator bypass with two-of-three rule.**
`BypassCertificate { approvers: Vec<OperatorSignature>, ticket_id,
rationale }` must have ≥ 2 approvers.  Bypassed agents are flagged
in the catalog (`bypassed_until: SystemTime`).  Acceptance :
`phase_4_n19_bypass_requires_two_of_three_approvals`.

### Cross-cutting

**G12. Gold case → N17 provenance link.**
A gold-run-driven LLM call carries `gold_case_id` on its
`LlmProvenanceRecord`.  Acceptance :
`phase_4_n19_llm_provenance_carries_gold_case_id`.

**G13. Gold run → N18 publish ledger link.**
`PublishLedgerEntry::AgentPromotedToTier1` includes `gold_run_id`.
Acceptance :
`phase_4_n19_n18_ledger_entry_references_gold_run`.

### Operability

**G14. Gold-run report query API.**
`GET /v1/gold-runs/{id}` + `GET /v1/agents/{id}/{version}/gold-runs`
endpoints.  Acceptance :
`phase_4_n19_gold_run_query_api_returns_report`.

**G15. Prometheus surface.**
12 surfaces in `n19_*` namespace (§ 7.8).  Acceptance :
`phase_4_n19_metrics_emit_all_12_surfaces`.

---

## 4. Invariants — non-negotiables (I1–I20)

### Immutability & versioning

**I1. GoldSet body is immutable after approval.**  Edits create a
new `GoldSetId` ; approval signatures are over the body hash.

**I2. GoldCase is identified by `GoldCaseId(Uuid)`.**  Reused across
re-runs of the same case — the `gold_case_id` is the stable join key
between gold-run ledger, N17 provenance, and N6 audit chain.

**I3. The 2-of-3 approval threshold is invariant.**  Lowering the
threshold requires an architecture-doc bump.

**I4. GoldSet has a content hash binding (`gold_set_hash`).**  The
hash covers : the ordered list of cases, the agent_id, the agent
input/output schema hashes captured at approval time, and the
approval-ceremony timestamp.

**I5. A passing gold run is identified by `gold_run_id`.**  Stable
forever ; referenced by N18 publish ledger + N6 audit.

### Coverage & quality

**I6. ≥ 5 gold cases per GoldSet.**  Below 5, registration rejects.

**I7. ≥ 1 case per declared `ExecutionMode` of the agent.**  A
multi-mode agent (Mode A + Mode C) must have at least one case per
mode.

**I8. PII-sensitive agents require ≥ 1 PII-redaction case.**  An
agent whose `ComplianceTags.pii_categories` is non-empty must include
at least one case asserting that the redaction was applied (input
has email → output must not contain it ; assertion strategy =
`StrictEquality` on the redaction sentinel).

**I9. Side-effect agents (`DeterminismLevel::SideEffect`) cannot be
gated by gold tests** — they must be `TierExperimental` forever.
This is a deliberate exclusion : gold tests run in hermetic
isolation and cannot reproduce production side-effects.

**I10. Input schema hash must match agent at run time.**  If the
gold set's `agent_input_schema_hash` ≠ the agent's current schema
hash, the run fails-closed with `StaleGoldSet`.

### Hermeticity & determinism

**I11. Gold cases run with no network access.**  Firecracker microVM
has the network namespace disabled.  An agent that tries to call
out fails with `RuntimeError::NetworkDisabled`.

**I12. Gold cases run with a frozen wall clock.**  The microVM's
clock is pinned to `GoldRun.frozen_at_utc`.  Used by agents that
read `Utc::now()`.

**I13. Gold cases see no host filesystem.**  Only the gold case's
declared `input` is visible.

**I14. Gold cases have a per-case timeout.**  `case.timeout_seconds`
default 30 s, max 300 s.  Timeout → case failed.

### Integrity & audit

**I15. Gold-run ledger HMAC-chains every entry.**  Reuses Sprint 6
chain primitives + N18 ledger pattern.

**I16. Gold-run ledger entries reference the publish-ledger entry
ID.**  Bidirectional traceability with N18.

**I17. Bypass certificate is non-repudiable.**  Stored alongside
the gold-run-ledger row that records the bypass usage ; queryable by
operator team for the bi-weekly bypass review meeting.

**I18. A bypassed Tier 1 agent is flagged in the catalog.**  UI shows
a "BYPASSED" badge ; flag clears on the next clean gold run.

### Performance & cardinality

**I19. Gold run P99 latency ≤ 5 min for ≤ 50 cases.**  SLA breach
fires PagerDuty.

**I20. Gold metric cardinality ≤ 5 000 lines.**  Labels are bounded
to `tier` (4 values) × `outcome` (5 values) × `agent_id` (≤ 250
production agents at Sprint 11 capacity).

---

## 5. Failure modes — what we PROTECT against (F1–F15)

> **Read convention** : `F-N : cause / detection / response / recovery`.

### Runtime faults

**F1. Gold runner panics.**
* *Cause* : bug in Firecracker microVM startup, Mode B model OOMs.
* *Detection* : runner returns `Err(RuntimeError::Panic)`.
* *Response* : the case is marked `Errored` (distinct from `Failed`) ;
  if ≥ 50 % of cases error, the entire run is `Errored` (not a
  reliable failure signal — operator must investigate).
* *Recovery* : ops investigates ; rerun once issue fixed.

**F2. Network leak from gold case (hermeticity violated).**
* *Cause* : misconfigured microVM ; bug in N11 pool isolation.
* *Detection* : eBPF probe on the microVM's tap interface during the
  run.
* *Response* : run aborts with `HermeticityBroken` ; page security
  on-call immediately.
* *Recovery* : Sprint 11 ships the detection ; remediation is N11's
  responsibility (Sprint 11 hardening pass).

**F3. LLM provider down during gold run.**
* *Cause* : OpenAI / Anthropic outage.
* *Detection* : `LlmProvenanceRecord.completeness = Errored` from
  the underlying call.
* *Response* : case retries up to 3 times (1 s, 5 s, 30 s
  backoff) ; if all 3 fail, case marked `Errored` (not failed).
* *Recovery* : automatic on provider recovery + re-run.

### Cost / budget

**F4. Cost cap exceeded mid-run.**
* *Cause* : a Mode C case spawned many tool calls or unbounded
  retries.
* *Detection* : running cost counter > cap.
* *Response* : run aborts with `CostCapExceeded { actual, cap }`.
  Cases already completed are recorded ; remaining marked `Skipped`.
* *Recovery* : raise cap (operator override) or trim cases.

**F5. Embedding provider quota exceeded.**
* *Cause* : SemanticSimilarity strategy hammers the embedding API.
* *Detection* : `EmbeddingProvider` returns `RateLimited`.
* *Response* : exponential backoff up to 60 s ; eventual case
  `Errored` if persistent.
* *Recovery* : Sprint 12 wires multi-provider failover.

### Integrity

**F6. Gold-set hash mismatch on lookup.**
* *Cause* : storage corruption or tampering.
* *Detection* : computed hash ≠ stored `gold_set_hash`.
* *Response* : reject the run with `GoldSetHashMismatch` ; alert
  security ops.
* *Recovery* : restore from backup ; re-approve the set.

**F7. Bypass signature invalid.**
* *Cause* : forged or revoked operator key.
* *Detection* : N18 SignatureVerifier rejects the
  `OperatorSignature`.
* *Response* : reject the bypass ; emit `BypassRejected` audit row.
* *Recovery* : valid operators re-issue.

### Schema & versioning

**F8. Gold set schema-hash mismatch.**
* *Cause* : agent's input schema changed after gold set was approved.
* *Detection* : `gold_set.agent_input_schema_hash ≠ agent.input
  _schema_hash()`.
* *Response* : run aborts with `StaleGoldSet`.
* *Recovery* : publisher submits a new gold set keyed to the new
  schema ; ops re-approves.

**F9. Replay anchor expired.**
* *Cause* : a gold case references an N16 trace that's outside the
  N16 retention window.
* *Detection* : N16 returns `TraceNotFound`.
* *Response* : case marked `Errored` ; report flags the stale anchor.
* *Recovery* : publisher re-anchors the case to a fresh trace.

**F10. Embedding model migrated mid-run.**
* *Cause* : Sprint 12 wires a real embedding provider that has its
  own versioning ; if the provider silently bumps mid-run, similarity
  comparisons are apples-to-oranges.
* *Detection* : N17-style model identity check on each embedding
  call ; mismatch detected.
* *Response* : run aborts with `EmbeddingModelDrift`.
* *Recovery* : re-pin embedding model version + re-run.

### Coverage

**F11. Gold set count below minimum.**
* *Cause* : publisher submitted 3 cases (< 5 floor).
* *Detection* : `GoldSet::validate()` at registration.
* *Response* : reject registration with `GoldSetTooSmall`.
* *Recovery* : publisher adds cases.

**F12. PII-sensitive agent without PII case.**
* *Cause* : agent has `ComplianceTags { pii_categories: [Email] }`
  but no gold case asserts redaction.
* *Detection* : `GoldSet::validate()`.
* *Response* : reject with `MissingPiiAssertionCase`.
* *Recovery* : publisher adds a redaction-checking case.

### Lifecycle

**F13. Stale gold set (> 180d since approval).**
* *Cause* : tier 1 agent unchanged for 6 months ; gold set is from
  pre-history.
* *Detection* : drift sweep computes `now - gold_set.approved_at`.
* *Response* : warning emitted ; not blocking, but flagged in the
  ops dashboard.
* *Recovery* : publisher re-approves an updated set.

**F14. Gold gate latency > 5 min (SLA breach).**
* *Cause* : 50 Mode C cases × 6 s each = 5 min, or unexpected
  provider latency.
* *Detection* : `n19_gold_run_duration_seconds_p99` > 300.
* *Response* : PagerDuty alert ; doesn't block the run (it eventually
  finishes), but ops investigates.
* *Recovery* : raise parallelism (N15 quota for gold) or trim cases.

**F15. Concurrent gold runs on the same `(agent_id, version)`.**
* *Cause* : two operators clicked "promote" simultaneously.
* *Detection* : `GoldGate` checks an in-progress flag on the agent.
* *Response* : the second request returns
  `GoldRunAlreadyInProgress { run_id }` ; caller can poll.
* *Recovery* : automatic — when the first run completes, both
  callers see the verdict via the query API.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. Multi-strategy assertions instead of a single equality.

**Context.**  Mode A demands byte-equality ; Mode B demands ε-
tolerance ; Mode C demands semantic similarity.  A single equality
model can't cover all three.

**Decision.**  Three first-class strategies, each a distinct SDK
type with explicit parameters :
* `StrictEquality` (Mode A)
* `NumericTolerance { fields: HashMap<JsonPointer, f64> }` (Mode B)
* `SemanticSimilarity { embedding_model, cosine_threshold }` (Mode C)

A fourth (`SchemaValidationOnly`) is a degenerate case for stub
fixtures — never sufficient as the sole assertion on a real GoldSet.

**Rationale.**
* **Type safety** : the SDK can refuse "wrong-mode" strategies at
  compile time via `#[serde(tag = "kind")]` taxonomy.
* **Explicit ε** : `NumericTolerance` forces the author to *think* —
  no implicit ε = "I don't know".
* **Embedding pinning** : `SemanticSimilarity` carries
  `embedding_model: ModelIdentity` (reuses N17 type), pinning the
  comparator across re-runs.

**Consequences.**
* SDK gains `GoldAssertionStrategy` enum.
* `AssertionEngine` dispatches by strategy variant.

**Rejected alternatives.**
* **Single universal "similarity" metric** : too coarse for Mode A
  (false negatives on JSON ordering).
* **Per-publisher custom Wasm assertion** : too much attack surface ;
  Sprint 13 may revisit.

---

### ADR-2. GoldSet storage : Postgres + content-addressed Blake3.

**Context.**  Where gold sets live.

**Decision.**  Production : `gold.gold_set` Postgres table keyed by
`gold_set_id: Uuid` + `gold_set_hash: Sha256Hash` (content
hash).  Cases stored as JSONB ; large payloads (e.g. embedded
binary blobs) go to S3 with a content-addressed key.

**Rationale.**
* **Postgres FK** to `publisher.publisher_registry` (gold set
  ownership) + `catalog.entries` (target agent).
* **Content-addressed** : the hash is the cryptographic identity ;
  storage path is `s3://efe-gold/<hash[..2]>/<hash[2..4]>/<full
  hash>`.

**Consequences.**
* Sprint 11 ships in-memory ; Sprint 12 wires Postgres + S3.

**Rejected alternatives.**
* **Git LFS** — opaque to the rest of the platform.
* **Pure S3 / no DB** — can't enforce FKs to the publisher
  registry.

---

### ADR-3. LLM semantic similarity via embedding cosine + fixed model.

**Context.**  How to compare LLM outputs for "similar enough".

**Decision.**  Embed both expected and actual output via a
*pinned* embedding model.  Use cosine similarity ; compare against
a per-case threshold.  Sprint 11 ships a *stub* embedding provider
that hashes the input deterministically (cosine of two stubs =
strict equality, with metadata about why it would have agreed) ;
Sprint 12 wires real providers.

**Rationale.**
* **Industry standard** : Anthropic eval pipelines, OpenAI evals,
  and academic benchmarks all use cosine.
* **Pinning is mandatory** : silent embedding-model migration would
  destroy comparability (F10).
* **Threshold is explicit per case** : authors think about noise
  floors.

**Consequences.**
* `EmbeddingProvider` trait, `StubEmbeddingProvider` impl.
* `SemanticSimilarity.embedding_model: ModelIdentity`.

**Rejected alternative.**
* **BLEU/ROUGE** : too brittle for free-form output.
* **LLM-as-judge** : non-deterministic ; would need another gold
  set to validate the judge.  Sprint 13 may pilot.

---

### ADR-4. "Frozen Tier 1" = passing gold run within last 30 days.

**Context.**  How long is a passing gold run valid ?

**Decision.**  A gold run's verdict is valid for 30 days.  Past
that, a fresh run is required to retain Tier 1 status.  The 30-day
drift sweep does this automatically.

**Rationale.**
* **Provider drift** : Claude / GPT update silently.  30 days is the
  half-life Anthropic publishes.
* **Operationally cheap** : on cached runs, marginal cost is near
  zero.

**Consequences.**
* `n19_gold_run_age_days` gauge per agent.
* Scheduled job ; alert if >30d since last run AND tier == Tier1.

**Rejected alternative.**
* **No expiry** — fails P4 / P12.

---

### ADR-5. Bypass = 2-of-3 EFE engineers + ticket.

**Context.**  Sev-1 hot-fixes need a fast path.

**Decision.**  `BypassCertificate { approvers: Vec<Operator
Signature>, ticket_id: String, rationale: String }`.  ≥ 2 approvers ;
ticket_id format `INC-<n>` or `SEC-<n>` ; rationale ≥ 64 chars.

**Rationale.**
* **Two-person rule** : industry standard for emergency overrides.
* **Ticket binding** : ties the bypass to an incident — post-mortem
  reviewers can find the rationale.

**Consequences.**
* Bypass uses N18's `OperatorSignature` (same scheme).
* Bypassed agents flagged in the catalog UI until next clean run.

**Rejected alternative.**
* **Single-engineer bypass** — fails I-2026-07-02 root cause.

---

### ADR-6. Gold case selection : hybrid manual + property-based.

**Context.**  How are gold cases authored ?

**Decision.**  Sprint 11 : hand-curated by publishers, ≥ 5 cases,
covering :
* the happy path of the input schema (≥ 1 case),
* each edge-case branch declared by the publisher (≥ N cases),
* each PII category (if applicable),
* a "regression anchor" — input known to have triggered a past
  customer issue.

Sprint 13 N21 adds property-based case discovery driven by N13
shadow capture.

**Rationale.**
* **Hand-curated is the lowest-risk first step** : property-based
  discovery requires shadow rollout infrastructure (N21 scope).
* **Future-compatible** : `GoldSet` has a `provenance: Provenance
  Source` field (`Manual` / `ShadowCapture` / `Mutation`) so
  Sprint 13 plugs in.

**Consequences.**
* `GoldSet.provenance: Provenance` Sprint 11 enum, only `Manual`
  variant.

---

### ADR-7. Gold runner = Firecracker microVM per case.

**Context.**  Where do gold cases execute ?

**Decision.**  Each case runs in a dedicated Firecracker microVM
from the N11 pool.  No shared VM state across cases.

**Rationale.**
* **Hermeticity** (G7, I11–I14).
* **Reuses N11 pool** : no new infra.
* **Parallelism** : the N11 pool already supports parallel cases ;
  N15 fairness scheduler allocates a tier-aware quota for gold.

**Consequences.**
* Gold runner depends on N11 + N15.
* Startup cost ~100 ms per case (N11 microVM warm pool).

**Rejected alternative.**
* **In-process unit-test-style execution** : breaks hermeticity
  (host clock, host FS).

---

### ADR-8. Cost cap = $100 per run by default ; Tier 3 may raise.

**Context.**  LLM gold runs can be expensive.

**Decision.**
* Default cap : $100 per gold run.
* Tier 2 publisher : can raise to $250 via gold-set field.
* Tier 3 publisher : can raise to $1 000 with operator override
  (1-of-3 EFE eng).
* Tier 0 / Tier 1 : capped at $100 always.

**Rationale.**
* **Bounded operational risk** : a runaway Mode C case won't
  bankrupt anyone.
* **Cache-driven** : on the 30-day sweep, cached calls cost
  essentially zero (N12).

**Consequences.**
* `GoldSet.cost_cap_usd: f64` with validation against the
  publisher's tier.
* `n19_gold_run_cost_cents` metric.

---

### ADR-9. Replay anchoring via N16.

**Context.**  How to test that Mode C agents produce equivalent
output across regions / time / providers.

**Decision.**  A `GoldCase.expected: GoldExpectedOutput` may be one
of :
* `Inline { value: serde_json::Value }` — author hand-wrote it.
* `ReplayAnchor { trace_id, span_id }` — reference an N16 trace ;
  expected output is what that trace produced ; the runner re-plays
  via N16's `ReplayEngine` and compares.

**Rationale.**
* **Reuses N16** : we already have the infrastructure.
* **Strong drift detection** : replay anchor + cosine threshold
  catches both provider drift AND prompt-template regressions.

**Consequences.**
* `GoldCase.expected` is an enum with two variants.
* `GoldRunner` depends on N16 `ReplayEngine`.

---

### ADR-10. Gold report is a first-class N6 audit row.

**Context.**  Where do gold reports live and how are they queried ?

**Decision.**  Every gold run emits an N6 audit chain row of kind
`GoldGateRun` carrying the entire `GoldReport` as its canonical
body.  In addition, a dedicated `gold.gold_run_ledger` table stores
the report for fast query (the audit row is the source of truth ;
the table is a materialised view).

**Rationale.**
* **N6 is the only audit substrate that survives DB corruption**
  (the audit chain HMAC + per-row content addressing).
* **Materialised view for query** : `GET /v1/gold-runs/{id}` doesn't
  walk the chain ; it reads the table.

**Consequences.**
* `GoldGateRun` audit-chain kind added.
* `gold.gold_run_ledger` table.

---

## 7. Component architecture

### 7.1 The 8 components — at a glance

```text
            Publisher portal                Operator portal
                       │                          │
                       ▼                          ▼
    ┌──────────────────────────┐    ┌────────────────────────┐
    │ GoldSetRegistry          │    │ Bypass workflow        │
    │  Draft → Approved        │    │  2-of-3 EFE eng sigs   │
    └──────────┬───────────────┘    └────────────┬───────────┘
               │                                 │
               ▼                                 │
    ┌──────────────────────────┐                 │
    │ GoldGate orchestrator    │◄────────────────┘
    │  ┌─────────┐ ┌────────┐  │
    │  │GoldRunner│→│Assertion│  │
    │  │ (N11)   │ │ Engine │  │
    │  └─────────┘ └────────┘  │
    └──────────┬───────────────┘
               ▼
    ┌──────────────────────────┐
    │ GoldRunLedger            │
    │  HMAC chain ; per agent  │
    │  (mirrors N18 pattern)   │
    └──────────┬───────────────┘
               ▼
        N6 audit chain ← GoldGateRun rows
        N17 LlmProvenance ← gold_case_id
        N18 PublishLedger ← AgentPromotedToTier1
```

### 7.2 `GoldCase` + `GoldSet` SDK types

**Lives** : `efe-nano-sdk/src/types/gold.rs`.

```rust
/// **N19** : a single golden test case.
pub struct GoldCase {
    pub gold_case_id: GoldCaseId,
    pub name: String,                          // human-readable
    pub input: serde_json::Value,              // conforms to agent.input_schema
    pub expected: GoldExpectedOutput,
    pub assertion: GoldAssertionStrategy,
    /// Per-case timeout (capped at 300 s).
    pub timeout_seconds: u32,
    /// Tags for selecting subsets at re-run time
    /// (e.g. "pii", "happy_path", "regression_anchor:CUS-1234").
    pub tags: Vec<String>,
}

pub struct GoldCaseId(pub Uuid);

pub enum GoldExpectedOutput {
    /// The expected output is inlined.
    Inline { value: serde_json::Value },
    /// Re-run an N16 trace ; the expected output is what that trace
    /// originally produced.
    ReplayAnchor { trace_id: Uuid, span_id: Uuid },
}

pub enum GoldAssertionStrategy {
    StrictEquality,
    NumericTolerance {
        /// JSON pointer → ε.  Fields not listed must match exactly.
        fields: std::collections::BTreeMap<String, f64>,
    },
    SemanticSimilarity {
        embedding_model: ModelIdentity,    // pin from N17 SDK
        cosine_threshold: f64,             // 0.0–1.0
        /// JSON pointer pointing at the field to embed.  If None,
        /// embed the whole serialised output.
        field_pointer: Option<String>,
    },
    /// Sprint 11 fallback for fixtures only ; rejected as the sole
    /// strategy on a real GoldSet.
    SchemaValidationOnly,
}

/// **N19** : the full curated bundle.
pub struct GoldSet {
    pub gold_set_id: GoldSetId,
    pub agent_id: NanoAgentId,
    /// Pinned to a specific agent VERSION at approval ; new agent
    /// versions trigger gold-set version bumps.
    pub agent_version: NanoAgentVersion,
    /// Hash of the agent's input schema captured at approval (I10).
    pub agent_input_schema_hash: Sha256Hash,
    pub agent_output_schema_hash: Sha256Hash,
    pub cases: Vec<GoldCase>,
    pub cost_cap_usd: f64,
    pub provenance: Provenance,
    pub status: GoldSetStatus,
    /// Operator approval signatures (≥ 2 required to move to
    /// `Approved`).
    pub approvers: Vec<OperatorSignature>,    // reuse N18 type
    pub created_at: SystemTime,
    pub approved_at: Option<SystemTime>,
    pub gold_set_hash: Sha256Hash,            // content hash
}

pub struct GoldSetId(pub Uuid);

pub enum GoldSetStatus { Draft, Approved, Archived }

pub enum Provenance {
    Manual,
    ShadowCapture { run_id: Uuid },          // Sprint 13
    Mutation { parent_set_id: GoldSetId },   // Sprint 13
}
```

### 7.3 `GoldSetRegistry` trait

```rust
#[async_trait]
pub trait GoldSetRegistry: Send + Sync + 'static {
    async fn register(
        &self, draft: GoldSet,
    ) -> Result<GoldSetId, GoldSetError>;

    async fn approve(
        &self, gold_set_id: GoldSetId, approvers: Vec<OperatorSignature>,
    ) -> Result<(), GoldSetError>;

    async fn archive(
        &self, gold_set_id: GoldSetId, rationale: String,
    ) -> Result<(), GoldSetError>;

    async fn lookup(
        &self, gold_set_id: GoldSetId,
    ) -> Result<GoldSet, GoldSetError>;

    /// Latest `Approved` gold set bound to a `(agent_id, version)`.
    async fn latest_approved_for(
        &self, agent_id: &NanoAgentId, version: &NanoAgentVersion,
    ) -> Result<Option<GoldSet>, GoldSetError>;

    async fn list_for_agent(
        &self, agent_id: &NanoAgentId,
    ) -> Result<Vec<GoldSet>, GoldSetError>;
}
```

### 7.4 `GoldRunner` trait

```rust
#[async_trait]
pub trait GoldRunner: Send + Sync + 'static {
    /// **N19** : Run a single case.  Returns the captured output +
    /// a per-case mini-report.
    async fn run_case(
        &self,
        agent: &NanoAgent,
        case: &GoldCase,
        frozen_clock_utc: SystemTime,
    ) -> Result<GoldCaseRun, GoldRunnerError>;
}

pub struct GoldCaseRun {
    pub case_id: GoldCaseId,
    pub trace_id: Uuid,                  // ties to N17 + N6
    pub captured_output: serde_json::Value,
    pub duration_ms: u32,
    pub cost_usd: f64,
    pub provider_calls: Vec<ProvenanceId>, // N17 provenance ids
    pub errored: Option<String>,
}
```

### 7.5 `AssertionEngine`

```rust
#[async_trait]
pub trait AssertionEngine: Send + Sync + 'static {
    async fn evaluate(
        &self,
        case: &GoldCase,
        captured: &serde_json::Value,
        expected: &serde_json::Value,
    ) -> Result<GoldCaseVerdict, AssertionError>;
}

pub enum GoldCaseVerdict {
    Passed { similarity: Option<f64> },
    Failed {
        reason: String,
        diff: Vec<GoldFieldDiff>,
        similarity: Option<f64>,
    },
    Errored { kind: GoldErrorKind, message: String },
    Skipped { reason: String },
}

pub struct GoldFieldDiff {
    pub json_pointer: String,
    pub expected: serde_json::Value,
    pub actual: serde_json::Value,
    pub epsilon_used: Option<f64>,
}
```

### 7.6 `GoldReport`

```rust
pub struct GoldReport {
    pub gold_run_id: GoldRunId,
    pub gold_set_id: GoldSetId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub gold_set_hash: Sha256Hash,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
    pub cases: Vec<GoldCaseEntry>,
    pub overall: GoldOverallVerdict,
    pub total_cost_usd: f64,
    pub cost_cap_usd: f64,
    pub bypass: Option<BypassCertificate>,
}

pub struct GoldRunId(pub Uuid);

pub struct GoldCaseEntry {
    pub case: GoldCase,
    pub run: GoldCaseRun,
    pub verdict: GoldCaseVerdict,
}

pub enum GoldOverallVerdict {
    Passed { cases_passed: u32, cases_total: u32 },
    Failed { cases_passed: u32, cases_failed: u32, cases_total: u32 },
    AbortedCostCap { cost_actual_usd: f64, cost_cap_usd: f64 },
    AbortedHermeticityBroken { case_id: GoldCaseId },
    Errored { error_count: u32, cases_total: u32 },
}
```

### 7.7 `GoldGate` orchestrator + `BypassCertificate`

```rust
pub struct GoldGate {
    pub registry: Arc<dyn GoldSetRegistry>,
    pub runner: Arc<dyn GoldRunner>,
    pub assertions: Arc<dyn AssertionEngine>,
    pub ledger: Arc<dyn GoldRunLedger>,
    pub metrics: Arc<N19Metrics>,
}

impl GoldGate {
    /// Main entry point.  Called by `RolloutPolicy::promote_to(
    /// Tier1)`.  Returns Ok with a passing report or a non-passing
    /// report ; only `Passed` overall verdicts permit promote.
    pub async fn run(
        &self,
        agent: &NanoAgent,
        gold_set: &GoldSet,
        bypass: Option<BypassCertificate>,
    ) -> Result<GoldReport, GoldGateError>;
}

pub struct BypassCertificate {
    pub approvers: Vec<OperatorSignature>,    // ≥ 2 required
    pub ticket_id: String,                    // INC-* or SEC-*
    pub rationale: String,                    // ≥ 64 chars
    pub valid_until: SystemTime,              // operator-set ≤ 14 d
}
```

### 7.8 N19 metrics

Twelve Prometheus surfaces in `n19_*` namespace :

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n19_gold_runs_total` | counter | `outcome`, `agent_tier` |
| 2 | `n19_gold_cases_executed_total` | counter | `strategy`, `outcome` |
| 3 | `n19_gold_run_cost_cents_total` | counter | `agent_tier` |
| 4 | `n19_gold_run_duration_seconds` | summary | `agent_tier` |
| 5 | `n19_gold_case_duration_seconds` | summary | `strategy` |
| 6 | `n19_gold_set_approvals_total` | counter | `outcome` |
| 7 | `n19_bypass_usage_total` | counter | `tier`, `ticket_class` |
| 8 | `n19_drift_sweep_demotions_total` | counter | `reason` |
| 9 | `n19_hermeticity_violations_total` | counter | `kind` |
| 10 | `n19_replay_anchor_lookups_total` | counter | `outcome` |
| 11 | `n19_assertion_engine_evaluations_total` | counter | `strategy`, `verdict` |
| 12 | `n19_gold_run_age_days` | gauge | `agent_id`, `tier` |

Cardinality bound : `agent_tier` (4) × `outcome` (5) × `strategy`
(4) = 80 per cross-labelled surface ; per-agent gauge bounded by the
~250 production agents at Sprint 11 capacity.

---

## 8. Module hierarchy & data model

### 8.1 Crate layout

```text
crates/
├── efe-nano-sdk/                  (existing)
│   └── src/types/
│       ├── gold.rs                 (NEW — § 7.2)
│       └── ...
├── efe-gold-test-gate/             (NEW crate)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── registry/
│   │   │   ├── mod.rs               (GoldSetRegistry trait)
│   │   │   └── memory.rs            (InMemoryGoldSetRegistry)
│   │   ├── runner/
│   │   │   ├── mod.rs               (GoldRunner trait)
│   │   │   ├── firecracker.rs       (FirecrackerGoldRunner — uses N11)
│   │   │   └── memory.rs            (InMemoryGoldRunner — tests)
│   │   ├── assertion/
│   │   │   ├── mod.rs               (AssertionEngine + Default impl)
│   │   │   ├── strict.rs            (StrictEquality)
│   │   │   ├── tolerance.rs         (NumericTolerance)
│   │   │   └── similarity.rs        (SemanticSimilarity)
│   │   ├── embedding/
│   │   │   ├── mod.rs               (EmbeddingProvider trait)
│   │   │   └── stub.rs              (StubEmbeddingProvider)
│   │   ├── ledger/
│   │   │   ├── mod.rs               (GoldRunLedger trait)
│   │   │   └── memory.rs
│   │   ├── gate.rs                  (GoldGate orchestrator)
│   │   ├── api.rs                   (HTTP query handlers — § 9.1)
│   │   ├── metrics.rs               (N19Metrics)
│   │   └── bin/
│   │       ├── efe_gold.rs          (CLI : preview-run + submit + approve)
│   │       └── efe_gold_verify.rs   (chain verifier)
│   └── Cargo.toml
```

### 8.2 SDK types layout (`efe-nano-sdk/src/types/gold.rs`)

Defines : `GoldCase`, `GoldCaseId`, `GoldExpectedOutput`, `GoldAssertionStrategy`,
`GoldSet`, `GoldSetId`, `GoldSetStatus`, `GoldRunId`, `GoldReport`, `GoldCaseEntry`,
`GoldCaseRun`, `GoldCaseVerdict`, `GoldFieldDiff`, `GoldOverallVerdict`,
`BypassCertificate`, `Provenance`, `GoldSetError`.

Re-exports at `efe_nano_sdk::*`.

### 8.3 Postgres schema (Sprint 12)

```sql
CREATE SCHEMA gold;

CREATE TABLE gold.gold_set (
    gold_set_id              UUID PRIMARY KEY,
    agent_id                 TEXT NOT NULL,
    agent_version            TEXT NOT NULL,
    agent_input_schema_hash  BYTEA NOT NULL,
    agent_output_schema_hash BYTEA NOT NULL,
    gold_set_hash            BYTEA NOT NULL UNIQUE,
    status                   TEXT NOT NULL,
    provenance_json          JSONB NOT NULL,
    cost_cap_usd             NUMERIC(10,2) NOT NULL,
    created_at               TIMESTAMPTZ NOT NULL,
    approved_at              TIMESTAMPTZ
);
CREATE INDEX ON gold.gold_set (agent_id, agent_version);

CREATE TABLE gold.gold_case (
    gold_case_id     UUID PRIMARY KEY,
    gold_set_id      UUID NOT NULL REFERENCES gold.gold_set(gold_set_id),
    name             TEXT NOT NULL,
    case_json        JSONB NOT NULL,
    timeout_seconds  INTEGER NOT NULL,
    tags             TEXT[] NOT NULL DEFAULT '{}'
);

CREATE TABLE gold.gold_set_approver (
    gold_set_id   UUID NOT NULL REFERENCES gold.gold_set(gold_set_id),
    operator_id   UUID NOT NULL,
    signed_at     TIMESTAMPTZ NOT NULL,
    ticket_id     TEXT NOT NULL,
    signature     BYTEA NOT NULL,
    PRIMARY KEY (gold_set_id, operator_id)
);

CREATE TABLE gold.gold_run_ledger (
    sequence_number BIGSERIAL PRIMARY KEY,
    gold_run_id     UUID NOT NULL UNIQUE,
    agent_id        TEXT NOT NULL,
    agent_version   TEXT NOT NULL,
    gold_set_id     UUID NOT NULL REFERENCES gold.gold_set(gold_set_id),
    canonical_body  BYTEA NOT NULL,
    hmac            BYTEA NOT NULL,
    prev_trace_id   UUID,
    body_json       JSONB NOT NULL,
    overall_verdict TEXT NOT NULL,
    cost_actual_usd NUMERIC(10,4) NOT NULL,
    bypass_used     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX ON gold.gold_run_ledger (agent_id, agent_version, created_at DESC);

CREATE TABLE gold.bypass_certificate (
    bypass_id        UUID PRIMARY KEY,
    gold_run_id      UUID NOT NULL REFERENCES gold.gold_run_ledger(gold_run_id),
    ticket_id        TEXT NOT NULL,
    rationale        TEXT NOT NULL,
    valid_until      TIMESTAMPTZ NOT NULL,
    approvers_json   JSONB NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints

```text
# Author workflow
POST /v1/gold-sets
    Body: GoldSet draft (status forced to Draft).
    Auth: bearer with publisher's key.
    → 201 Created { gold_set_id }
    → 400 (validation failed — fewer than 5 cases, missing PII case, ...)

POST /v1/gold-sets/{id}/approve
    Body: { approvers: [OperatorSignature, ...] }   # ≥ 2 required
    Auth: bearer with operator JWT (gold:approve scope).
    → 200 OK GoldSet (now Approved)
    → 403 (sig invalid / < 2 approvers)

GET /v1/gold-sets/{id}
    Auth: optional bearer
    → 200 OK GoldSet
    → 404

GET /v1/agents/{agent_id}/gold-sets
    → 200 OK [GoldSet]

# Run workflow
POST /v1/gold-runs
    Body: { agent_id, agent_version, gold_set_id, bypass: Option<BypassCertificate> }
    Auth: bearer with operator (gold:run scope) OR publisher of the agent
    → 202 Accepted { gold_run_id }
    → 409 (run already in progress)

GET /v1/gold-runs/{id}
    → 200 OK GoldReport
    → 404

GET /v1/agents/{agent_id}/{version}/gold-runs
    → 200 OK [GoldRunSummary]

# Drift sweep operator endpoints
POST /v1/gold-runs/drift-sweep
    Auth: operator
    → 202 Accepted { batch_id }

GET /v1/gold-runs/drift-sweep/{batch_id}
    → 200 OK DriftSweepReport
```

### 9.2 Internal Rust traits

Enumerated in § 7.3–7.6 + `GoldRunLedger` (mirrors N18 `PublishLedger`).

---

## 10. End-to-end workflows

### 10.1 Workflow A — Normal Tier 1 promotion

```
1. Publisher publishes v1.2.3 (signed via N18).
2. Agent lands in StabilityTier::TierExperimental.
3. Publisher drafts a GoldSet with ≥ 5 cases :
   - 1 happy-path StrictEquality (Mode A field)
   - 2 NumericTolerance cases (Mode B numeric outputs, ε=0.01)
   - 2 SemanticSimilarity cases (Mode C summarisation, cosine ≥ 0.92)
4. POST /v1/gold-sets → Draft.
5. Two EFE engineers POST /v1/gold-sets/{id}/approve with their
   OperatorSignatures → Approved.
6. Publisher (or ops) POST /v1/gold-runs { agent_id, agent_version,
   gold_set_id }.
7. GoldGate :
   a. Lookup the gold set ; check status == Approved + schema-hash
      match.
   b. For each case : runner.run_case in a Firecracker microVM ;
      collect output + LLM provenance ids.
   c. AssertionEngine.evaluate per case.
   d. Aggregate report ; emit ledger row.
8. If overall == Passed → promote_to(Tier1) succeeds ; N18 publish
   ledger emits AgentPromotedToTier1 with gold_run_id.
9. If overall != Passed → promote rejected ; report stored for triage.
```

### 10.2 Workflow B — Emergency bypass

```
1. Tier 1 agent has Sev-1 bug (P13).
2. Publisher publishes hot-fix v1.2.4 (signed).
3. Two EFE engineers sign a BypassCertificate :
   { approvers, ticket_id: "INC-9117",
     rationale: "Customer ACME blocked by NPE on empty input ;
                 90 % CAR-class customer ; full gold-run would take
                 32 min ; hot-fix is 1-line null guard.",
     valid_until: now + 7 days }
4. POST /v1/gold-runs with bypass set.
5. GoldGate :
   a. Verify both signatures via N18 SignatureVerifier.
   b. Skip case execution ; emit a ledger row of kind
      GoldRunBypassed.
   c. Catalog flags the agent BYPASSED until next clean run.
6. N13 canary picks up the new Tier 1 version with the bypass flag ;
   percentage starts at 5 % (not 50 %) for the next 24 h regardless
   of catalog policy (defense in depth).
7. Within 7 days, a clean gold run MUST succeed or the agent demotes
   automatically.
```

### 10.3 Workflow C — 30-day drift sweep

```
1. Scheduled job fires every 24 h, picks the agents whose last gold
   run is > 27 days old.
2. For each : POST /v1/gold-runs with the latest approved gold set.
3. On pass : update n19_gold_run_age_days = 0.
4. On fail : emit GoldDriftDetected ; demote agent to
   TierExperimental ; alert publisher + ops.
5. Bypassed agents whose 7-day bypass window elapses without a clean
   run are caught by the same job and demoted.
```

### 10.4 Workflow D — Replay-anchored gold case

```
1. Publisher includes a GoldCase with
   expected = ReplayAnchor { trace_id, span_id }.
2. GoldGate, at run time :
   a. Look up the N16 trace ; check it's within retention window.
   b. Re-execute the trace via ReplayEngine ; capture output.
   c. Run the agent under test on the same input.
   d. AssertionEngine compares the two captured outputs (not the
      static expected value).
3. Useful for : "this should produce the same answer that v1.0.0
   produced on this real-customer input" — strong drift signal.
```

### 10.5 Workflow E — Cost cap exceeded mid-run

```
1. Mode C agent has runaway tool-call loop.
2. After case 7 of 12, accumulated cost = $98 (cap=$100).
3. Case 8 starts ; the runner observes cost > cap mid-case ; aborts
   the case.
4. GoldGate emits overall = AbortedCostCap { actual: 100.42, cap: 100.0 } ;
   ledger row marks remaining cases Skipped.
5. Promote rejected (AbortedCostCap is NOT a pass).
6. Publisher trims cases or operator raises cap (one-of-three EFE
   eng signature).
```

---

## 11. Five-day implementation plan

### Day 41 — SDK types + GoldSetRegistry
* `efe-nano-sdk/src/types/gold.rs` (~900 LOC).
* `GoldSet::validate()` enforces ≥ 5 cases, ≥ 1 case per mode,
  PII coverage, cost-cap bounded by tier.
* `efe-gold-test-gate` new crate ; workspace member.
* `GoldSetRegistry` trait + `InMemoryGoldSetRegistry` impl.
* 60+ unit tests.

### Day 42 — Runner + Firecracker integration
* `GoldRunner` trait + `InMemoryGoldRunner` impl (tests).
* `FirecrackerGoldRunner` impl wiring N11 pool ; hermetic
  isolation (no net, frozen clock, restricted FS).
* eBPF hermeticity probe stub (Sprint 12 wires real probe).
* 30+ unit tests.

### Day 43 — AssertionEngine + 3 strategies + embedding stub
* `assertion::strict::StrictEquality` — recursive JSON equality
  with ordering normalisation.
* `assertion::tolerance::NumericTolerance` — JSON-pointer walk +
  ε comparison per field ; non-numeric fields require strict
  equality.
* `assertion::similarity::SemanticSimilarity` — cosine via
  `EmbeddingProvider`.
* `StubEmbeddingProvider` returning deterministic hashed vectors.
* 50+ unit tests covering happy + sad paths per strategy.

### Day 44 — GoldGate orchestrator + ledger + N18 wiring
* `GoldGate::run` end-to-end with cost-cap, parallelism (N15),
  bypass handling.
* `GoldRunLedger` HMAC chain (mirrors N18 ledger).
* N18 `PublishLedgerEntry::AgentPromotedToTier1` variant ; wire
  the publish-ledger reference.
* N17 `LlmProvenanceRecord.gold_case_id` field.

### Day 45 — Integration tests + metrics + drift sweep + CLI
* 15+ acceptance tests (`phase_4_n19_*`).
* `N19Metrics` Prometheus surface (12 surfaces).
* Drift-sweep scheduled job stub.
* `efe-gold` CLI (preview, submit, approve, run, list).
* `efe-gold-verify` chain verifier.
* Cross-cutting smoke tests with N16 + N17 + N18.
* Clippy strict + workspace build.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (15 tests, one per G)

```
phase_4_n19_gold_set_immutable_after_approve
phase_4_n19_gold_set_requires_two_approvers
phase_4_n19_gold_run_ledger_chain_verifies
phase_4_n19_three_strategies_decide_pass_fail
phase_4_n19_output_schema_violation_fails_case
phase_4_n19_promote_tier_1_requires_passing_gold_run
phase_4_n19_gold_run_isolated_in_microvm
phase_4_n19_cost_cap_aborts_run
phase_4_n19_cached_calls_drive_marginal_cost_to_zero
phase_4_n19_drift_sweep_demotes_on_fail
phase_4_n19_bypass_requires_two_of_three_approvals
phase_4_n19_llm_provenance_carries_gold_case_id
phase_4_n19_n18_ledger_entry_references_gold_run
phase_4_n19_gold_run_query_api_returns_report
phase_4_n19_metrics_emit_all_12_surfaces
```

### 12.2 Property tests

* `proptest_gold_set_validate_round_trip` — any valid GoldSet
  round-trips through serde.
* `proptest_strict_equality_symmetric` — `eq(a, b) == eq(b, a)`.
* `proptest_numeric_tolerance_within_epsilon_passes` —
  `|a - b| ≤ ε ⇒ pass`.
* `proptest_semantic_similarity_self_is_1` — `cosine(x, x) == 1.0`.

### 12.3 Chaos tests

* `chaos_runner_panic_marks_case_errored` (F1).
* `chaos_provider_down_retries_then_errors` (F3).
* `chaos_cost_cap_exceeded_aborts_cleanly` (F4).
* `chaos_concurrent_runs_serialised_per_agent` (F15).
* `chaos_hermeticity_probe_fires_on_network_leak` (F2).

### 12.4 Security tests

* `security_bypass_signature_forgery_rejected` — forged
  OperatorSignature fails verification (F7).
* `security_gold_set_hash_mismatch_rejected` — tampered
  gold set fails the integrity check (F6).
* `security_constant_time_strict_equality` — strict-equality
  comparison is constant-time over the input length.

### 12.5 Cross-cutting smoke tests

* `cross_n16_n19_replay_anchor_round_trip` — a gold case
  expecting a ReplayAnchor evaluates via N16.
* `cross_n17_n19_gold_case_id_on_provenance` — LLM call from
  gold-run carries the gold_case_id.
* `cross_n18_n19_promote_emits_publish_ledger_entry` — Tier 1
  promote emits AgentPromotedToTier1 with gold_run_id.

---

## 13. Cross-cutting alignment (N12 → N18)

| Subsystem | Concern | Resolution |
|---|---|---|
| **N11 microVM pool** | Hermetic isolation per gold case | Runner allocates N11 microVM ; net/clock/FS restricted |
| **N12 cache** | Cache LLM calls from gold runs to drive marginal cost down | Cache key includes `gold_case_id` |
| **N13 canary** | Promotion to Tier 1 gates the canary | `promote_to(Tier1)` calls GoldGate |
| **N14 envelope** | Envelope carries gold_run_id when emitted from a gold-run dispatch | Envelope's `gold_run_id: Option<Uuid>` field |
| **N15 fairness** | Gold runs must not starve customer traffic | Dedicated `OperationClass::GoldRun` with a tier-scoped quota |
| **N16 replay** | Replay-anchored gold cases | `GoldExpectedOutput::ReplayAnchor` uses `ReplayEngine` |
| **N17 provenance** | Gold-driven LLM calls are attributable | `LlmProvenanceRecord.gold_case_id` |
| **N18 publisher** | Promotion requires a gold run | `PublishLedgerEntry::AgentPromotedToTier1` carries gold_run_id ; bypass uses N18 OperatorSignature |

---

## 14. Sprint 11 ship vs Sprint 12/13 split

### Sprint 11 (this design)
* All 6 capabilities from § 0.
* 15 acceptance tests.
* In-memory registry + ledger.
* Stub embedding provider.
* Firecracker runner with hermetic isolation.
* Operator bypass workflow.
* Drift-sweep job scaffolding (manual trigger in Sprint 11 ; cron
  in Sprint 12).

### Sprint 12 N20 — Postgres + real embeddings + drift cron
* `PostgresGoldSetRegistry` + `PostgresGoldRunLedger` with RLS.
* Real `EmbeddingProvider` impls : OpenAI text-embedding-3-small,
  Cohere embed-multilingual-v3, sentence-transformers/all-MiniLM-L6
  -v2.
* Cron-driven 30-day drift sweep.
* Cost-cap raise-with-override path for Tier 3.
* `efe-gold` CLI ships in the GA bundle.

### Sprint 13 N21 — Shadow capture + mutation testing
* `Provenance::ShadowCapture` — N13 shadow rollout proposes gold
  cases for operator review.
* `Provenance::Mutation` — automated property-based case discovery
  from input schema.
* Cross-version diff visualisation in the publisher portal.
* Sprint 14 N22 : LLM-as-judge experimental strategy.

---

## 15. Annexes

### A1 — Assertion strategy taxonomy

| Strategy | Mode fit | ε | Embedding | When |
|---|---|---|---|---|
| `StrictEquality` | Mode A | n/a | n/a | Deterministic byte output |
| `NumericTolerance` | Mode B | per-field | n/a | Float / numeric drift acceptable within ε |
| `SemanticSimilarity` | Mode C | cosine 0–1 | pinned | LLM nondeterministic output |
| `SchemaValidationOnly` | (fixtures) | n/a | n/a | Sprint 11 stub only ; rejected on prod sets |

### A2 — GoldSet wire format (canonical body)

```
SHA-256 over CBOR-encoded :
{
  "agent_id": <string>,
  "agent_version": <string>,
  "agent_input_schema_hash": <bytes-32>,
  "agent_output_schema_hash": <bytes-32>,
  "cost_cap_usd": <f64>,
  "cases": [
    {
      "gold_case_id": <bytes-16>,
      "name": <string>,
      "input": <json>,
      "expected": <gold_expected_output>,
      "assertion": <assertion_strategy>,
      "timeout_seconds": <u32>,
      "tags": [<string>]
    }
  ],
  "provenance": <provenance>,
  "created_at": <int64>
}
```

The `gold_set_hash` is the SHA-256 of the canonical CBOR bytes.

### A3 — Tolerance taxonomy (per assertion strategy)

| Strategy | Tolerance type | Default | Operator override |
|---|---|---|---|
| `StrictEquality` | None | n/a | n/a |
| `NumericTolerance` | per-JSON-pointer ε | 0.01 | author-set |
| `SemanticSimilarity` | cosine threshold | 0.90 | author-set ; floor 0.50 |

### A4 — Operator runbook

#### A4.1 Approve a gold set

```bash
efe-gold approve <gold_set_id> \
  --operator-id $(whoami) \
  --ticket "GOLD-1234" \
  --rationale "Reviewed 12 cases ; PII coverage OK ; cost cap OK."
```

#### A4.2 Trigger a gold run

```bash
efe-gold run <agent_id> --version 1.2.3 --gold-set <id>
```

#### A4.3 Issue an emergency bypass

```bash
efe-gold bypass <agent_id> --version 1.2.4 \
  --approvers <op1>,<op2> \
  --ticket INC-9117 \
  --rationale "Customer ACME ; null guard hot-fix ; runway = 7d"
```

#### A4.4 Verify the gold-run ledger

```bash
efe-gold-verify ledger <agent_id>
efe-gold-verify ledger-from-file dump.json
```

### A5 — Comparison with related systems

| System | Their approach | N19's improvement |
|---|---|---|
| OpenAI Evals | Self-evaluated; no audit | + HMAC chain, operator approval |
| Anthropic Eval Suite | Internal use; no public framework | + multi-strategy, replay-anchored |
| Snorkel data slice testing | Probabilistic data slices | + strict equality where it applies |
| LangChain `Eval` | LLM-as-judge by default | + deterministic strategies, judge optional Sprint 14 |
| Kubernetes `Conformance Tests` | Pass/fail only | + cost cap, hermetic, audit-chained |

---

**End of N19 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 11 (Phase 4, day 41 – 45).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 + N17 + N18 →
N19 (this doc) → N20 Postgres + real embeddings → N21 shadow capture
→ N22 LLM-as-judge.

**Last-mile checklist** (Sprint 11 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on new crate.
- [ ] Pre-N19 Tier 1 agents grandfathered for 90 days.
- [ ] Gold run P99 latency benchmarked ≤ 5 min for 50 cases.
- [ ] Operator runbook A4 reviewed by ops + security team.
- [ ] PagerDuty integration for F2 (hermeticity violation) — sev1.
- [ ] PagerDuty integration for F14 (latency SLA breach) — sev2.
- [ ] Compliance team sign-off on bypass workflow.
- [ ] CLI tools `efe-gold` + `efe-gold-verify` shipped.
- [ ] Cost-cap dashboard for ops in Grafana.
- [ ] First two reference gold sets approved : `summarize_strict
      v3.0.0` (Tier 1 EFE-shipped agent) + `extract_table v3.0.0`
      (Tier 1 partner agent).
