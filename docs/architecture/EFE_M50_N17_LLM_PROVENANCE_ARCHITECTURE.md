# EFE M50 — N17 LLM Provenance Architecture

> **Sprint** : 9 (Phase 3, day 31 – 35 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md` (Sprint 9, shipped)
>
> **Anchors** : Sprint 5 `LlmAugmentedRuntime` (Mode C executor) is the
> integration point.  Sprint 6 HMAC audit chain is the storage substrate.
> N17 is **strictly additive** — every existing trace remains valid ;
> N17 introduces a new trace kind that anchors to the chain alongside
> them.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N17 closes (P1–P15)](#2-fifteen-incident-patterns-n17-closes-p1p15)
3. [Commitments — what ships in Sprint 9 (G1–G15)](#3-commitments--what-ships-in-sprint-9-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [N12 / N13 / N14 / N15 / N16 alignment](#13-n12--n13--n14--n15--n16-alignment)
14. [Sprint 9 ship vs Sprint 10/11 split](#14-sprint-9-ship-vs-sprint-1011-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The problem.**  Today's Mode C execution path (Sprint 5
`LlmAugmentedRuntime`) makes LLM provider calls and stores per-op cost
in the audit chain, BUT does **not** record the comprehensive
provenance needed by Phase 3 regulators :

- **No model fingerprint** — only `provider` + `model_id` strings ; the
  underlying weights or hosted-model version are not captured.
- **No prompt content-address** — system prompt + user prompt are
  hashed only as part of N12 cache keys ; the audit trace itself has
  no hash, so an auditor cannot answer "show me every call that used
  prompt X".
- **No provider request id** — when a provider returns a wrong answer
  and we need to file a support ticket, we have no canonical
  identifier to share.
- **No token-count reconciliation** — billing drifts between provider
  invoices and our recorded `cost_cents` are not detectable.
- **No tool-call lineage** — function calling is on the Sprint 10
  roadmap ; without explicit tool-call provenance now, adding it later
  breaks chain integrity.

Five regulators / customers have asked for one or more of these in
the Sprint 8 retro alone (incidents § 1.1).  EU AI Act Article 12
(record-keeping for high-risk AI systems) requires "automatically
generated logs of events" — Sprint 5's audit chain is *necessary but
not sufficient* without per-call LLM provenance.

**The fix.**  N17 introduces a **first-class `LlmProvenanceRecord`**
written for every Mode C call (and every N12 cache hit that served
in lieu of one).  Eight guarantees ship :

1. **Model identity is canonicalized** — `(provider, model_id,
   version, content_hash)` tuple recorded on every call, with hash
   verified pre-call against a `ModelRegistry`.
2. **Prompt is content-addressed** — BLAKE3 hashes of system prompt,
   user prompt, and assembled final prompt are stored ; the raw
   prompts go to the (already-existing) body store gated by the
   tenant's `AuditPolicy.include_input`.
3. **Provider response metadata captured** — `provider_request_id`,
   `provider_response_time_ms`, `provider_finish_reason`,
   `provider_region` — fields are `Option` so providers that omit
   them still produce valid records.
4. **Token accounting verified** — both client-side estimate AND
   provider response counts stored ; divergence raises a diff metric
   used by the Sprint 10 cost reconciliation job.
5. **Tool calls recorded** — a `Vec<ToolCallRecord>` captures every
   function call (name, args hash, response hash, ordinal) ; Sprint 10
   wires the runtime, Sprint 9 ships the SDK + chain types so the
   format is frozen.
6. **Safety + completeness flags** — refusal, content-filter trip,
   streaming-interrupted, partial-output cases are all explicit
   enum variants — no opaque error strings.
7. **PII handling captured** — `PiiStrategy` applied + the redaction
   token set are part of provenance, so post-hoc audit can verify
   "no PHI left tenant X".
8. **Cross-cutting integration** — every N12 cache hit emits a
   `LlmCacheHit` provenance trace ; every N16 replay emits a
   `LlmReplayCall` trace pointing at the original `LlmCall` trace.
   No black holes.

**Six capabilities ship in Sprint 9 N17** :

1. **`LlmProvenanceRecord`** SDK type (full wire format) + supporting
   sub-types (`ModelIdentity`, `TokenUsage`, `ToolCallRecord`,
   `Completeness`, `SafetyFlag`, `CostAccounting`).
2. **`ProvenanceId`** + new `LlmCall` / `LlmCacheHit` /
   `LlmReplayCall` / `ModelHashMismatch` trace kinds (extends
   Sprint 6 audit chain).
3. **`LlmProvenanceCollector`** trait + `DefaultLlmProvenance
   Collector` impl — wraps any `LlmClient` and emits a provenance
   record for every call.
4. **`ModelRegistry`** trait + `InMemoryModelRegistry` impl —
   stores expected model hashes + price tables ; verifies hashes
   pre-call.
5. **`LlmProvenanceStore`** trait + `InMemoryLlmProvenanceStore`
   impl — queryable per-tenant store with the four canonical access
   patterns (by trace, by swarm, by model, cost summary).
6. **HTTP-shaped query API** in the dispatcher's audit subsystem
   for the four access patterns + a `model_drift` event log.

**Out of scope (deferred to Sprint 10 N18 / Sprint 11 N19)** :

- Postgres-backed `LlmProvenanceStore` (Sprint 10 — schema migration).
- Live cost reconciliation daily job against provider invoices
  (Sprint 10 — needs M31 billing integration).
- Drift-detection dashboard with statistical alarm rules
  (Sprint 11 — needs SLO compute pipeline).
- Provenance-driven semantic comparison for N16 replay (Sprint 11 —
  needs the embedding index from M28).

**Backward compatibility.**  N17 changes are strictly additive :
- The Sprint 6 HMAC canonical_row_body format is **NOT** changed.
  Provenance lives in a NEW field `provenance_payload: Option<Bytes>`
  excluded from the canonical body (same pattern as N14's
  `swarm_envelope_trace_id`).  Pre-N17 chains still verify.
- Existing Sprint 5 `LlmCallResponse` already carries `tokens_in /
  tokens_out / cost_cents` ; N17 extends it via an additive
  `provenance_extension: Option<LlmProvenanceExtension>` field that
  defaults to `None`.

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N17

**Incident I-2026-05-02 — EU regulator audit query.**
The Irish DPC issued a Section 7(2) request asking for "every
automated decision made by the platform affecting an EU data subject
in March 2026, including the model identity, the inputs, and the
outputs".  We answered in 11 days because :
- Filtering the audit chain to Mode C ops required scanning every
  trace (no `is_llm_call` index).
- Reconstructing the model identity required joining against the
  catalog's `execution_mode.LlmAugmented.model` field, which
  changed Stable→Canary→Stable three times during the quarter.
- Token counts were derived from `cost_cents`, which lost precision
  ; we couldn't reproduce the exact request body.

**Incident I-2026-05-09 — Cost drift.**
M31 monthly bill from OpenAI showed 41.8 M tokens consumed.  Our
recorded `cost_cents` aggregated to 39.7 M tokens (5.0 % short).
Finance team needed to know where the 2.1 M tokens went.  Without
per-call token reconciliation, we couldn't tell whether (a) our
client estimator was off, (b) provider double-counted, or (c) a
silent infrastructure leak existed.

**Incident I-2026-05-15 — Customer prompt-injection claim.**
A customer alleged that a Mode C summarisation agent leaked another
tenant's data into their swarm output.  Investigation took 6 hours
because :
- The audit chain's `full_input` body captured the OPERATION input
  (the user's document), NOT the assembled LLM prompt.
- The system prompt was sourced from `efe-prompts` registry but the
  version wasn't recorded in the chain.
- We had no way to confirm "which exact bytes hit the LLM".

**Incident I-2026-05-19 — Provider model deprecation.**
OpenAI deprecated `gpt-4-0613` ; subsequent calls silently routed
to `gpt-4-0314`.  Our customers' Mode C agents showed measurable
drift in benchmark suite.  Without recording the actual model
version returned (not just the model_id we requested), we couldn't
prove "the provider swapped models on us".

**Incident I-2026-05-22 — HIPAA compliance gap.**
A healthcare-vertical customer demanded a written attestation that
"no PHI is sent to LLM providers".  We use Sprint 5 PII redaction
(`PiiStrategy::Strict`) — but the audit chain doesn't record :
- WHICH PII strategy was active for each call.
- WHICH redaction tokens replaced what.
The customer's HIPAA officer rejected the attestation because we
couldn't produce a forensic trail.

### 1.2 Why Sprint 9, immediately after N16

N16 (multi-region replay) presupposes that replay output can be
compared against original output.  For Mode C, that comparison is
meaningless unless we know :
- Which model was used originally.
- Which sampling seed.
- Whether the original was a cache hit or fresh LLM call.

N17 captures all of this.  Without N17, N16's replay-of-Mode-C is
forensically empty.  Shipping N17 in the same sprint cycle closes
both gaps in tandem.

### 1.3 Who consumes N17 once it ships

| Consumer | Use case |
|---|---|
| **Compliance team** | EU AI Act Article 12 record-keeping |
| **Customer DPO** | GDPR Article 22 right-to-explanation |
| **Finance / M31 billing** | Daily cost reconciliation vs provider invoices |
| **Customer support** | "What model answered my support ticket?" |
| **Platform engineering** | Provider migration audit (Anthropic → OpenAI) |
| **SRE on-call** | Provider request_id for support ticket filing |
| **Security team** | Prompt-injection forensics |
| **Research / EvalOps** | Model drift detection over time |
| **M24 GDPR saga** | Tombstone cascade includes provenance rows |
| **N16 replay engine** | Mode C tolerance evaluation needs model+seed |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns.  Family clustering :

* **Regulatory & compliance** (P1, P2, P7) — 3 patterns
* **Forensic & audit reconstruction** (P3, P4, P6, P15) — 4 patterns
* **Cost & accounting** (P5, P11) — 2 patterns
* **Operational integrity** (P8, P9, P10) — 3 patterns
* **Cross-cutting (N12 / N16 / N13)** (P12, P13, P14) — 3 patterns

---

## 2. Fifteen incident patterns N17 closes (P1–P15)

> **Methodology** : every P-row sources from (a) a real incident with
> ticket reference, (b) a regulator-level requirement, or (c) a Sprint
> 8 retro-flagged near-miss.  No speculative rows.

### Family A — Regulatory & compliance

**P1. Regulator demands an LLM-decision audit for a date window
in a jurisdiction.**
The EU AI Act Article 12 requires "automatically generated logs of
events occurring during the operation" of high-risk AI systems.  EU
DPC Section 7(2) requests already triggered (Incident I-2026-05-02 §
1.1).  Today's chain has no LLM-specific filter ; scanning every
trace for `execution_mode.is_llm_augmented()` is O(chain length).
N17 ships `LlmProvenanceStore::list_by_model + window` — indexed.

**P2. GDPR Article 22 right-to-explanation request.**
A customer asks "what was the reasoning behind the automated
decision X taken about me?".  We must produce the model, prompt,
inputs, and output.  Today we have inputs (from `full_input` body)
and the OperationResult ; we lack the assembled LLM prompt and the
model fingerprint.  N17's `LlmProvenanceRecord` carries all of it.

**P7. HIPAA / healthcare attestation : no PHI sent to LLM.**
The covered entity demands evidence that PHI redaction was active
on every Mode C call to an external provider.  Today, the
`ComplianceTags.pii_strategy` on the agent records the *configured*
strategy ; nothing records the *actually-applied* strategy at the
call site.  N17 fixes this : `LlmProvenanceRecord.pii_strategy_
applied` + `redaction_token_set` are mandatory fields.

### Family B — Forensic & audit reconstruction

**P3. Prompt-injection forensic backtrack.**
A leak occurred ; we need to determine which user prompt + system
prompt combination triggered it.  Today, the system prompt is
sourced from `efe-prompts` registry but the (template_id, version)
isn't in the chain.  N17 stamps `prompt_template_id + version` on
every record.

**P4. Provider model deprecation drift.**
Provider silently migrates `gpt-4-0613` → `gpt-4-0314` (Incident
I-2026-05-19).  Our customers' benchmark suite shows drift.  We
can't prove the migration because we recorded "the model we
requested", not "the model we got".  N17 records BOTH :
`requested_model_id` and the provider's `served_model_id` (from
response metadata when available).

**P6. Tool-call lineage trace.**
A function-calling LLM invokes `look_up_patient_chart(...)` ; the
patient was wrong.  We need to trace : what arguments did the model
pass ?  what did the tool return ?  did the model retry ?  Today,
Mode C is single-shot ; Sprint 10 will add tool calling.  N17 ships
the SDK trace format **now** so adding the runtime layer in Sprint
10 doesn't break chain integrity.

**P15. Same-prompt-different-output anomaly.**
Internal evaluator notices : same prompt run twice through the
same agent yielded different outputs.  Why ?  Sampling temperature ?
Model swap ?  Cache hit on the first call ?  N17 captures
`sampling.seed`, `sampling.temperature`, `cache_hit` — the
evaluator gets a structured answer.

### Family C — Cost & accounting

**P5. Token-count drift vs provider invoice.**
Monthly OpenAI bill shows 41.8 M tokens ; our `cost_cents`
aggregated reports 39.7 M (Incident I-2026-05-09).  We can't tell
where the 5 % drift came from.  N17 records BOTH the client
estimator's count AND the provider response's count ; daily diff
job (Sprint 10) computes drift per model per day.

**P11. Catalog-version migration cost surprise.**
A canary moved `summarize_strict` from `gpt-3.5-turbo` →
`gpt-4-turbo`.  Cost per call increased 10×.  Operators want a
weekly cost-by-model breakdown.  N17's `cost_summary` API answers
in O(1) — pre-aggregated by model_identity.

### Family D — Operational integrity

**P8. Provider returns safety refusal.**
A Mode C call hits the provider's content filter.  Provider
returns `{"refused": true, "category": "violence"}`.  Today we
record `Err(LlmCallError::Provider(...))` — opaque.  N17 ships
`SafetyFlag::Refused { category, provider_msg }` so analytics
can plot refusal rates by category.

**P9. Streaming response interrupted.**
We use the OpenAI streaming API ; mid-stream the connection drops.
Partial output is in hand.  Today, we either return the partial as
"success" or fail with a vague error.  N17 ships
`Completeness::Partial { received_tokens, expected_tokens }` and
stores the partial output flagged.

**P10. Provider outage — need request_id for support ticket.**
Provider returns 500.  We need their internal request_id to file
support.  N17 captures `provider_request_id` on every call,
including failed ones — recorded BEFORE we read the response body.

### Family E — Cross-cutting

**P12. Cache hit (N12) needs provenance lineage.**
Sprint 8 N12 idempotency cache serves a recorded Mode C output
without calling the LLM.  Today, the audit chain emits a `CacheHit`
sentinel.  But forensic audit asks "what was the original LLM
call that warmed this cache entry ?" — without a typed link, the
chain has to be scanned.  N17 ships `LlmCacheHit { original_trace_
id }` so the lineage is one hop.

**P13. Replay (N16) needs original-vs-replay LLM provenance.**
N16 replays Mode C in a target region.  The comparison needs to
know "did the replay's model match the original's model ?".  N17's
`LlmReplayCall` trace points at the original `LlmCall` trace and
records the replay's own model identity + token counts ; the
comparator can ASSERT model equality (or fail with explicit drift).

**P14. Canary rollout (N13) — model differs by version.**
N13 routes 5 % of traffic to `summarize_strict@v2` (using
`gpt-4-turbo`) and 95 % to `@v1` (using `gpt-3.5-turbo`).
Operators want a cost & drift comparison.  N17's `cost_summary`
filtered by `version` answers in seconds.

---

## 3. Commitments — what ships in Sprint 9 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### Identity & integrity

**G1. Model identity canonicalized.**  Every `LlmCall` provenance
record carries `ModelIdentity { provider, model_id,
requested_version, served_version, content_hash, supports_tool_use }`.
Acceptance : `phase_3_n17_model_identity_recorded`.

**G2. Prompt content-addressed.**  `prompt_inputs_hash`,
`system_prompt_hash`, `user_prompt_hash`, `final_prompt_hash`
(BLAKE3-256) ship on every record.  Acceptance :
`phase_3_n17_prompt_hashes_stable_across_serdes`.

**G3. Pre-call model hash verification.**  Before dispatching a
Mode C call, the runtime queries `ModelRegistry::verify_hash`.
Mismatch → fail-closed with `ModelHashMismatch` provenance trace +
alert.  Acceptance : `phase_3_n17_hash_mismatch_blocks_call`.

**G15. HMAC chain integrity preserved.**  The provenance record
lives in a NEW trace-level field `provenance_payload` excluded from
the canonical body — Sprint 6 chain verification UNCHANGED.
Acceptance : `phase_3_n17_pre_n17_chain_still_verifies`.

### Token & cost accounting

**G5. Both client-estimate and provider-reported token counts
recorded.**  `TokenUsage { client_estimated, provider_reported }`.
Sprint 10 daily reconciliation job consumes both.  Acceptance :
`phase_3_n17_token_drift_visible`.

**G11. Deterministic cost computation.**  `CostAccounting` carries
`prices_table_version` from the `ModelRegistry` ; cost recomputable
from `tokens × prices` at any time.  Acceptance :
`phase_3_n17_cost_recomputable_from_tokens_prices`.

### Forensic completeness

**G4. Provider response metadata captured.**  `provider_request_id`,
`provider_response_time_ms`, `provider_finish_reason`,
`provider_region` recorded ; all `Option` for providers that
omit fields.  Acceptance : `phase_3_n17_provider_metadata_recorded`.

**G6. Tool-call records frozen format.**  `Vec<ToolCallRecord>`
shipped in SDK ; runtime wiring deferred to Sprint 10 ; chain
format is frozen NOW.  Acceptance :
`phase_3_n17_tool_call_record_serdes_round_trip`.

**G7. Sampling parameters captured.**  `SamplingParams {
temperature, top_p, max_tokens, seed, stop_sequences }`.  Mandatory.
Acceptance : `phase_3_n17_sampling_params_recorded`.

**G8. Safety + completeness flags typed.**  `Completeness ∈
{Complete, Partial, Refused, Errored}` ; `SafetyFlag` enum with 8
variants.  Acceptance : `phase_3_n17_safety_flag_taxonomy_round_trip`.

### Privacy & PII

**G12. PII redaction provenance.**  `pii_strategy_applied` +
`redaction_token_set` mandatory.  Acceptance :
`phase_3_n17_pii_strategy_recorded_at_call_site`.

### Cross-cutting

**G13. Cache hits emit `LlmCacheHit` traces.**  N12 cache layer
extended to emit a typed cache-hit trace with `original_trace_id`
pointer.  Acceptance : `phase_3_n17_cache_hit_records_provenance`.

**G14. Replay calls emit `LlmReplayCall` traces.**  N16 multi-
region replay engine emits this trace for every Mode C op it
re-dispatches.  Acceptance : `phase_3_n17_replay_call_recorded`.

**G9. Multi-tenancy isolation on query API.**  Every query
endpoint enforces tenant scope via the JWT claims.  Cross-tenant
read returns 403.  Acceptance : `phase_3_n17_provenance_cross_
tenant_blocked`.

### Read API

**G10. Four canonical query patterns ship.**  `get_by_trace`,
`list_by_swarm`, `list_by_model`, `cost_summary`.  Each tenant-
scoped.  Acceptance : `phase_3_n17_four_query_patterns_work`.

---

## 4. Invariants — non-negotiables (I1–I20)

### Identity & integrity

**I1. Every Mode C op emits exactly one `LlmCall` provenance
trace** — or fails before LLM dispatch with no trace.  No partial
traces left in the chain after a panic mid-call.

**I2. The provenance trace is HMAC-chained.**  Sprint 6 invariant
preserved — N17 doesn't change the canonical body shape, only
attaches a new `provenance_payload` field excluded from the body.

**I3. The four prompt hashes appear in `provenance_payload`** :
`prompt_inputs_hash`, `system_prompt_hash`, `user_prompt_hash`,
`final_prompt_hash`.  Each is BLAKE3-256.

**I4. Model identity is canonicalized** :
`(provider, model_id, requested_version, served_version,
content_hash)`.  Equality semantics are field-by-field exact.

**I15. Model hash verified pre-call.**  Pre-flight check against
`ModelRegistry` ; on mismatch emit `ModelHashMismatch` provenance
trace + reject.

### Token & cost

**I5. Token counts from provider win** when both client-estimate
and provider-reported are available.  Both stored ; provider-
reported field is the canonical for cost reconciliation.

**I16. Cost cents derived deterministically** from
`tokens × prices_table[prices_table_version][model_identity]`.
Provider-quoted cost (when available) stored separately for
audit, not used in cost_cents field.

### Completeness & safety

**I6. Tool calls recorded with full lineage** — name, args hash,
result hash, ordinal.  Recursive tool calls (tool returns a tool
call request) appear as multiple ordinals on the same record.

**I7. Streaming-interrupted responses flagged.**  `Completeness ::
Partial { received_tokens, expected_tokens }`.  The partial output
is stored in body store ; the trace itself records the truncation.

**I14. Refusal / safety flag recorded** with the provider's
refusal kind.  The rejected content is NOT stored in body store
(privacy minimisation) ; only `final_prompt_hash` is.

### Cross-cutting

**I8. Cache hits emit `LlmCacheHit` traces, NOT `LlmCall`.**
The `LlmCacheHit` trace references the original `LlmCall` trace's
`provenance_id` for one-hop lineage.

**I9. Replay traces typed.**  N16 emits `LlmReplayCall` with
`replay_of: ProvenanceId` ; original's provenance_id resolves
back to the original `LlmCall` via the chain.

**I10. Provenance traces are READ-ONLY post-write.**  Same
invariant as Sprint 6 audit chain.

**I11. Multi-tenancy isolation.**  Same RLS pattern as audit chain.
Cross-tenant access → 403 + security alert.

**I12. Provider request_id captured when available.**  None is
acceptable — provider didn't return one.  Empty string is NOT
acceptable.

**I13. Sampling seed captured.**  Mandatory for Mode C
determinism contract.  `seed = None` means "non-deterministic by
design" ; explicit value means "should reproduce".

**I17. Prompt template version embedded.**  `(template_id,
template_version)` ; missing → fail with `UnknownPromptTemplate`
before LLM call.

**I18. Redaction token set carried.**  The N17 record carries the
exact tokens the redactor inserted.  Post-hoc audit can verify "no
PII passed through".

**I19. PII strategy applied recorded.**  The strategy ACTUALLY
applied at the call site (not just the configured one).

**I20. Provenance verifiable cross-region.**  When N16 ships a
chain cross-region, the provenance records ride along.  Target
region can verify HMAC + content hashes WITHOUT cross-region key
access (because chains carry the derived HMAC key per N16 G2).

---

## 5. Failure modes — what we PROTECT against (F1–F15)

> **Read convention** : `F-N : cause / detection / response / recovery`.

### Provider behaviour

**F1. Provider doesn't return request_id.**
* *Cause* : provider lacks the header or returns empty.
* *Detection* : header parse returns None.
* *Response* : `provider_request_id = None` — record the call
  normally, log a warn.
* *Recovery* : none needed.

**F2. Token counts not returned in provider response.**
* *Cause* : provider returns body without `usage` section.
* *Detection* : deserialization yields None.
* *Response* : use client-estimator counts, set `is_estimated =
  true`.
* *Recovery* : reconciliation job (Sprint 10) detects systemic
  drift, alerts ops.

**F7. Provider safety refusal.**
* *Cause* : provider content filter blocks the response.
* *Detection* : response carries `finish_reason = content_filter`
  or `refused = true`.
* *Response* : record `Completeness::Refused`, `SafetyFlag::
  ContentFilter { category }`.  Record `output_hash = None`.
* *Recovery* : op-level failure surfaced as
  `OperationErrorKind::PromptInjectionSuspected` (existing N8
  taxonomy).

**F12. Provider returns malformed response.**
* *Cause* : provider 500 / network truncation / JSON parse fail.
* *Detection* : `serde_json::from_slice` returns Err, or status code
  ≥ 400.
* *Response* : record `Completeness::Errored`, `output_hash =
  None`, `provider_request_id` captured if response headers parsed.
* *Recovery* : op error surfaces `LlmCallError::Provider`.

### Identity & verification

**F3. Model hash mismatch on pre-call.**
* *Cause* : provider silently rotated the model behind the API.
* *Detection* : `ModelRegistry::verify_hash` returns `Mismatch`.
* *Response* : emit `ModelHashMismatch` provenance trace.  Fail
  the LLM call with `LlmCallError::ModelHashMismatch`.  Surface as
  `OperationErrorKind::ModelIntegrityViolation` (NEW per § 6 ADR-7).
* *Recovery* : ops investigates, updates registry expected hash,
  resumes.

**F8. Prompt template version unknown.**
* *Cause* : agent references a template id not in registry.
* *Detection* : `PromptRegistry::resolve(id, version)` returns
  None.
* *Response* : fail BEFORE LLM call ; emit a `Preflight` failure
  trace per Sprint 7 N4 ; no provenance written.
* *Recovery* : operator publishes the missing template ; re-dispatch.

### Storage & integrity

**F4. Provenance write fails (chain store down).**
* *Cause* : Postgres connection error during audit insert.
* *Detection* : `ChainStore::insert` returns Err.
* *Response* : fail the OP — do NOT proceed without provenance
  (architecture G15).  Surface as
  `OperationErrorKind::InternalError`.
* *Recovery* : ops restores chain store ; idempotency cache (N12)
  prevents double-billing on retry.

**F15. Replay re-attempts after provenance integrity broken.**
* *Cause* : N16 replay detects chain HMAC mismatch.
* *Detection* : `verify_chain` returns Err.
* *Response* : abort replay ; emit `ReplayError::ChainBroken` ;
  no provenance overwrite.
* *Recovery* : forensic investigation by security team.

### Tool calls

**F5. Tool call returned error.**
* *Cause* : the tool function returned `Err(...)`.
* *Detection* : tool dispatch result is Err.
* *Response* : record the tool call with `result_kind = Errored`,
  store error message hash.  LLM may continue or stop based on
  agent design.
* *Recovery* : agent-specific.

### Streaming & partial

**F6. Streaming interrupted.**
* *Cause* : provider connection drops mid-stream.
* *Detection* : `Stream::poll_next` returns Err before terminator.
* *Response* : record `Completeness::Partial { received_tokens }`.
  Final prompt hash present ; output hash computed over partial
  content.
* *Recovery* : op-level decision — Sprint 9 fails the op ; Sprint
  10 will add retry policy.

### Cost & accounting

**F11. Cost computation overflow.**
* *Cause* : pathological token count × extreme price.
* *Detection* : `f64::INFINITY` after multiplication.
* *Response* : cap at `f64::MAX / 2` ; log a warn ; record
  `cost_overflow = true`.
* *Recovery* : ops investigates — likely a malicious or
  misconfigured agent.

### Multi-tenancy

**F14. Cross-tenant provenance read attempt.**
* *Cause* : misconfigured operator or malicious actor.
* *Detection* : query tenant ≠ stored provenance tenant.
* *Response* : 403 ; security audit trace.
* *Recovery* : security ops investigates.

### Cross-cutting (N12 / N16)

**F13. Cache hit returns Mode C response, but original chain row was
tombstoned.**
* *Cause* : M24 GDPR saga tombstoned the original `LlmCall` trace.
* *Detection* : `LlmCacheHit` would emit `original_trace_id` that
  no longer resolves.
* *Response* : reject the cache hit ; force re-dispatch (incurring
  fresh LLM cost) ; emit `CacheHitOrphaned` trace.
* *Recovery* : cache GC invalidates orphaned entries.

**F10. Replay model identity differs from original.**
* *Cause* : N16 target region serves a newer model version.
* *Detection* : `LlmReplayCall.model_identity != original.model_identity`.
* *Response* : record both ; emit `MatchOutcome::Mismatch` for the
  op ; the replay continues to its conclusion.  Sprint 11 hardens
  this with an "abort on drift" flag.
* *Recovery* : ops decides whether to accept or roll back model
  migration.

**F9. Multi-LLM swarm — one op's LLM call succeeds, another fails.**
* *Cause* : tenant has 3 Mode C ops in a swarm ; op 2 fails.
* *Detection* : per-op error.
* *Response* : each LLM call emits ITS OWN `LlmCall` provenance
  trace ; partial swarm-level data is in the envelope (N14).
* *Recovery* : standard per-op retry per `RetryPolicy`.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. Provenance lives in a NEW trace-level field, NOT in the canonical body.

**Context.**  Sprint 6 HMAC chain canonicalizes the trace body for
hashing.  Changing the canonical shape breaks every pre-N17 chain's
verification.

**Decision.**  Add a `provenance_payload: Option<Bytes>` field on
`NanoAgentTrace` — **excluded** from `canonical_row_body` (Sprint 6
treats this field as out-of-band metadata).  Same pattern as N14's
`swarm_envelope_trace_id`.

**Rationale.**
* **G15 invariant** — pre-N17 chains MUST continue verifying.
* **Storage flexibility** — provenance can be retrieved as a separate
  column / S3 blob without re-hashing chain.
* **Forward-compat** — future trace-level metadata can use the same
  pattern.

**Consequences.**
* Sprint 6 `canonical_row_body` builder ignores the new field.
* New `provenance_payload_hash` field IS included in canonical body
  (so we can verify provenance integrity without trusting the side
  storage).
* Pre-N17 traces have `provenance_payload = None` AND
  `provenance_payload_hash = None`.

**Rejected alternatives.**  Adding fields to the canonical body —
would force a new schema version + chain migration ; too costly for
backward compat.

---

### ADR-2. BLAKE3-256 for prompt hashing, not SHA-256.

**Context.**  Hash function for content-addressing prompts.

**Decision.**  BLAKE3 (32-byte digest).

**Rationale.**
* **Speed** : BLAKE3 is ~6× faster than SHA-256 on modern hardware.
  Mode C calls' hot path adds 3 hashes per call ; ~1 µs vs ~6 µs
  matters at 1 k ops/s.
* **Length** : 256-bit matches SHA-256 collision resistance.
* **Streaming** : BLAKE3 has native streaming support ; useful for
  long prompts.
* **Ecosystem** : Rust `blake3` crate is well-maintained.

**Consequences.**
* `blake3` dep added to `efe-nano-sdk` (already an indirect dep via
  some Rust crates ; check workspace).
* New `Blake3Hash` newtype in SDK with hex encoding.

**Rejected alternatives.**  SHA-256 (already used for Sprint 1 input
hashing) — keep both ; SHA-256 stays for backward compat in legacy
sites ; new sites use BLAKE3.

---

### ADR-3. Tool-call records are nested, flat-indexed by ordinal.

**Context.**  Recursive tool calls — a tool returns a result, the
LLM emits another tool call.  How to model ?

**Decision.**  Flat `Vec<ToolCallRecord>` indexed by `ordinal: u32`.
The model's "thinking trace" between tool calls is implicit —
captured by per-call `prompt_hash` (the assembled prompt for the
NEXT step embeds the previous tool's result).

**Rationale.**
* **Simple wire format** — operators can filter ToolCallRecords by
  ordinal range without recursive parsing.
* **Lossless** — recurrence relation is recoverable from the
  ordering ; agents can re-derive the dialogue.
* **Compatible with OpenAI / Anthropic schemas** — both APIs return
  flat lists.

**Consequences.**
* `tool_calls: Vec<ToolCallRecord>` field on
  `LlmProvenanceRecord`.
* Sprint 10 runtime wiring ; Sprint 9 ships the format only.

**Rejected alternatives.**  Recursive tree structure — too complex
to serialise + query.

---

### ADR-4. Redacted prompt hashes are recorded ; raw prompts gated by `AuditPolicy.include_input`.

**Context.**  Privacy minimisation — should the raw prompt EVER hit
the chain ?

**Decision.**  The prompt HASHES are always in the chain (small
fixed-size payload).  The RAW prompts go to the body store ONLY if
the agent's `AuditPolicy.include_input == true`.

**Rationale.**
* **GDPR Article 25** — privacy by design ; minimise data collected.
* **Compliance flexibility** — tenants who want full forensic
  recoverability opt in ; those that don't get the hash trail
  (still enables tampering detection).
* **Pre-N17 audit policy** is already a per-agent setting ; we
  reuse it.

**Consequences.**
* `LlmProvenanceRecord.final_prompt_hash` always present.
* Body store row `provenance.raw_prompts.{trace_id}` ONLY when
  agent opts in.

---

### ADR-5. Streaming traces written at completion, with completeness flag.

**Context.**  Streaming LLM responses ; when to write the trace ?

**Decision.**  Write ONCE, at stream completion (or error).  Set
`Completeness::{Complete, Partial, Refused, Errored}`.

**Rationale.**
* **Simplicity** — single transaction with the chain store.
* **No partial traces** — invariant I1 holds.
* **Forensic clarity** — auditor sees a single row per LLM call.

**Consequences.**
* Streaming runtime must buffer the response in memory before
  emitting the trace.  Memory usage bounded by `max_tokens` config.

**Rejected alternatives.**  Append-only trace updates as the stream
arrives — breaks chain immutability (I10).

---

### ADR-6. Provider-specific metadata is an opaque `serde_json::Value` map.

**Context.**  Each provider returns slightly different metadata.
OpenAI : `system_fingerprint`.  Anthropic : `stop_sequence`.
Mistral : `model_revision`.

**Decision.**  `provider_metadata: HashMap<String,
serde_json::Value>` — provider-specific keys allowed ; well-known
keys promoted to typed fields (`provider_request_id`,
`provider_response_time_ms`, etc.).

**Rationale.**
* **Forward-compat** — adding fields requires only a deserializer
  update.
* **Provider parity** — every provider gets full capture without
  schema bloat.

**Consequences.**
* `provider_metadata` field on every record.
* Sprint 10 will define typed promotion for OpenAI's
  `system_fingerprint` (used for model identity).

---

### ADR-7. Model hash verification fails closed.

**Context.**  Provider rotates model behind the API ; our recorded
hash mismatches actual.

**Decision.**  Pre-call hash verification ; mismatch → reject the
LLM call.  New error kind
`OperationErrorKind::ModelIntegrityViolation`.

**Rationale.**
* **Forensic integrity** — we must not emit traces with a model
  identity we couldn't verify.
* **Regulator alignment** — EU AI Act requires "automatic logging
  of events" — model swaps qualify.
* **Operator override** — registry has a `dev_skip_hash_check`
  flag for dev clusters.

**Consequences.**
* New SDK enum variant `OperationErrorKind::ModelIntegrityViolation`.
* New trace kind `ModelHashMismatch` emitted on rejection.
* Ops process : on every model deprecation, registry must be
  updated FIRST, then customers can use the new model.

**Rejected alternatives.**  Soft warning + continue — would mask
silent migrations.

---

### ADR-8. Provenance retention matches audit chain (7 years).

**Context.**  How long to keep provenance ?

**Decision.**  Same TTL as audit chain (7 years) for production
tenants ; configurable per-tenant via M24 GDPR saga.

**Rationale.**
* **Regulatory minimum** — EU AI Act Article 12 mandates 7 years
  for high-risk systems.
* **Storage cost** — bounded ; raw prompts are gated by AuditPolicy
  so most tenants only keep hashes (~ 200 bytes/record).
* **GDPR right-to-erasure** — M24 saga tombstones the row, leaving
  a hash-only stub for chain integrity.

**Consequences.**
* `provenance.llm_provenance` table partitioned by month for
  efficient GC.
* M24 saga extended : tombstoning a swarm tombstones its
  provenance records.

---

### ADR-9. Query API enforces tenant isolation at the SQL level.

**Context.**  Provenance query endpoints — same as audit chain.

**Decision.**  PostgreSQL Row-Level Security policy + trait-level
assertions (defense in depth).  Same pattern as N16's
`replay.replay_jobs`.

**Rationale.**
* **Defense in depth** — single layer is fragile.
* **Audit transparency** — RLS policy is inspectable in SQL.

**Consequences.**
* Schema migration ships RLS policies on day 1.
* Trait-level: every `LlmProvenanceStore::get*` takes a `tenant:
  TenantId` parameter.

---

### ADR-10. Cost reconciliation lands in Sprint 10, not Sprint 9.

**Context.**  Daily job comparing provider invoices to provenance-
derived cost.

**Decision.**  Defer to Sprint 10.  Sprint 9 ships the DATA (token
counts + price table) ; Sprint 10 ships the JOB (M31 billing
integration).

**Rationale.**
* **Scope discipline** — Sprint 9 is already large (N16 + N17).
* **Dependency** — needs M31 billing daily-pull API which is
  Sprint 10 anyway.

**Consequences.**
* Sprint 9 ships `cost_summary` endpoint with TWO numbers per
  model : "client-estimated total" and "provider-reported total".
* Sprint 10 builds the reconciliation job on top.

---

## 7. Component architecture

### 7.1 The 8 components — at a glance

```text
                          Mode C op dispatched
                                 │
                                 ▼
                ┌─────────────────────────────────────┐
                │  PromptTemplateRegistry (M21)        │
                │  - resolves (template_id, version)   │
                │  - emits PromptResolved event        │
                └────────────────┬────────────────────┘
                                 ▼
                ┌─────────────────────────────────────┐
                │  ModelRegistry                       │
                │  - looks up expected hash + prices   │
                │  - emits ModelHashMismatch on drift  │
                └────────────────┬────────────────────┘
                                 ▼
                ┌─────────────────────────────────────┐
                │  ProvenanceCollectingLlmClient       │
                │  - wraps any LlmClient               │
                │  - records prompts hashes, request_id│
                │  - emits LlmProvenanceRecord         │
                └────────────────┬────────────────────┘
                                 ▼
                ┌─────────────────────────────────────┐
                │  LlmProvenanceCollector              │
                │  - assembles full record             │
                │  - writes to LlmProvenanceStore      │
                │  - writes LlmCall trace to chain     │
                └────────────────┬────────────────────┘
                                 ▼
              ┌─────────────────────────────────────┐
              │  LlmProvenanceStore (Postgres / mem) │
              │  - by trace, by swarm, by model      │
              │  - cost summary                      │
              └─────────────────────────────────────┘
                                 │
                                 ▼ HTTP query API
                ┌─────────────────────────────────────┐
                │  N17 query handlers                  │
                │  - GET /v1/provenance/{trace_id}     │
                │  - GET /v1/provenance/swarm/{id}     │
                │  - GET /v1/provenance/cost?…         │
                │  - GET /v1/provenance/drift?…        │
                └─────────────────────────────────────┘
```

### 7.2 `LlmProvenanceRecord` (SDK type)

**Lives** : `efe-nano-sdk/src/types/llm_provenance.rs`.

```rust
pub struct LlmProvenanceRecord {
    // ─── Identity ────────────────────────────────────────────────
    pub provenance_id: ProvenanceId,
    pub trace_id: TraceId,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub tenant_id: TenantId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,

    // ─── Model identity (G1, G3, I4, I15) ─────────────────────────
    pub model_identity: ModelIdentity,
    pub model_hash_verified: bool,

    // ─── Prompt provenance (G2, I3) ───────────────────────────────
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub prompt_inputs_hash: Blake3Hash,
    pub system_prompt_hash: Blake3Hash,
    pub user_prompt_hash: Blake3Hash,
    pub final_prompt_hash: Blake3Hash,

    // ─── Sampling (G7, I13) ────────────────────────────────────────
    pub sampling: SamplingParams,

    // ─── Token + cost (G5, G11, I5, I16) ───────────────────────────
    pub token_usage: TokenUsage,
    pub cost_accounting: CostAccounting,

    // ─── Provider response (G4, I12) ───────────────────────────────
    pub provider_request_id: Option<String>,
    pub provider_response_time_ms: u32,
    pub provider_finish_reason: FinishReason,
    pub provider_region: Option<RegionCode>,
    pub provider_metadata: HashMap<String, serde_json::Value>,

    // ─── Output + completeness (G8, I7, I14) ──────────────────────
    pub output_hash: Option<Blake3Hash>,
    pub completeness: Completeness,
    pub safety_flags: Vec<SafetyFlag>,

    // ─── Tool calls (G6, I6) ───────────────────────────────────────
    pub tool_calls: Vec<ToolCallRecord>,

    // ─── PII (G12, I18, I19) ───────────────────────────────────────
    pub pii_strategy_applied: PiiStrategy,
    pub redaction_token_set: Vec<String>,

    // ─── Cross-cutting (G13, G14, I8, I9) ─────────────────────────
    pub replay_of: Option<ProvenanceId>,
    pub cache_hit: Option<CacheProvenance>,

    // ─── Timing ────────────────────────────────────────────────────
    pub recorded_at: SystemTime,
}

pub struct ModelIdentity {
    pub provider: String,
    pub model_id: String,
    pub requested_version: String,
    pub served_version: Option<String>,    // None when provider doesn't return it
    pub content_hash: Option<Blake3Hash>,   // None for hosted models
    pub supports_tool_use: bool,
}

pub struct TokenUsage {
    pub client_estimated_input: u32,
    pub client_estimated_output: u32,
    pub provider_reported_input: Option<u32>,
    pub provider_reported_output: Option<u32>,
    pub is_estimated: bool,
}

pub struct CostAccounting {
    pub cost_cents: f64,
    pub prices_table_version: u32,
    pub provider_quoted_cost_cents: Option<f64>,
    pub overflow_capped: bool,
}

pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Unknown,
}

pub enum Completeness {
    Complete,
    Partial { received_tokens: u32, expected_tokens: u32 },
    Refused { category: SafetyCategory },
    Errored { kind: String },
}

pub enum SafetyFlag {
    PiiDetected,
    PromptInjectionSuspected,
    OutputModerationTriggered,
    ContentFilter { category: SafetyCategory },
    ToxicityDetected,
    SelfHarmDetected,
    JailbreakAttemptDetected,
    PromptLengthExceeded,
}

pub enum SafetyCategory {
    Hate,
    Sexual,
    Violence,
    SelfHarm,
    SexualMinors,
    ViolenceGraphic,
    Harassment,
    Other(String),
}

pub struct SamplingParams {
    pub temperature: f32,
    pub top_p: Option<f32>,
    pub max_tokens: u32,
    pub seed: Option<u64>,
    pub stop_sequences: Vec<String>,
}

pub struct ToolCallRecord {
    pub ordinal: u32,
    pub tool_name: String,
    pub arguments_hash: Blake3Hash,
    pub result_hash: Option<Blake3Hash>,
    pub result_kind: ToolCallResultKind,
}

pub enum ToolCallResultKind {
    Ok,
    Errored,
    Cancelled,
    Skipped,
}

pub struct CacheProvenance {
    pub original_trace_id: TraceId,
    pub original_provenance_id: ProvenanceId,
    pub cache_key_version: u32,
}

pub struct Blake3Hash([u8; 32]);

pub struct ProvenanceId(Uuid);
```

### 7.3 `ModelRegistry`

**Lives** : `efe-llm-provenance/src/registry.rs`.

```rust
#[async_trait]
pub trait ModelRegistry: Send + Sync + 'static {
    async fn lookup(&self, identity: &ModelIdentity) -> Result<RegisteredModel, RegistryError>;
    async fn verify_hash(&self, identity: &ModelIdentity, actual_hash: &Blake3Hash) -> Result<(), RegistryError>;
    async fn register(&self, model: RegisteredModel) -> Result<(), RegistryError>;
    async fn deprecate(&self, identity: &ModelIdentity, at: SystemTime) -> Result<(), RegistryError>;
    async fn list_active(&self) -> Result<Vec<RegisteredModel>, RegistryError>;
}

pub struct RegisteredModel {
    pub identity: ModelIdentity,
    pub expected_content_hash: Option<Blake3Hash>,
    pub price_per_million_input_tokens_cents: f64,
    pub price_per_million_output_tokens_cents: f64,
    pub prices_table_version: u32,
    pub registered_at: SystemTime,
    pub deprecated_at: Option<SystemTime>,
    pub dev_skip_hash_check: bool,
}
```

In-memory impl + future Postgres impl.  Ops use the SDK CLI to
register / deprecate models.

### 7.4 `LlmProvenanceCollector`

**Lives** : `efe-llm-provenance/src/collector.rs`.

```rust
pub struct DefaultLlmProvenanceCollector {
    store: Arc<dyn LlmProvenanceStore>,
    chain_writer: Arc<dyn AuditWriter>,
    model_registry: Arc<dyn ModelRegistry>,
    metrics: Option<Arc<N17Metrics>>,
}

impl DefaultLlmProvenanceCollector {
    pub async fn pre_call(&self, partial: PreCallProvenance) -> Result<(), ProvenanceError>;
    pub async fn post_call(&self, full: LlmProvenanceRecord) -> Result<(), ProvenanceError>;
    pub async fn on_cache_hit(&self, hit: CacheHitProvenance) -> Result<(), ProvenanceError>;
    pub async fn on_replay(&self, replay: ReplayProvenance) -> Result<(), ProvenanceError>;
}
```

### 7.5 `ProvenanceCollectingLlmClient` wrapper

Wraps any `LlmClient` (Sprint 5 trait).

```rust
pub struct ProvenanceCollectingLlmClient {
    inner: Arc<dyn LlmClient>,
    collector: Arc<DefaultLlmProvenanceCollector>,
    prompt_registry: Arc<dyn PromptRegistry>,
}

#[async_trait]
impl LlmClient for ProvenanceCollectingLlmClient {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError> {
        // 1. Resolve prompt template + version.
        // 2. Verify model hash via registry.
        // 3. Compute prompt hashes.
        // 4. Call inner.call(...).
        // 5. Parse provider response, compute output hash.
        // 6. Assemble LlmProvenanceRecord.
        // 7. Emit via collector.post_call.
        // 8. Return LlmCallResponse (with provenance_extension attached).
    }
}
```

### 7.6 `LlmProvenanceStore`

**Lives** : `efe-llm-provenance/src/store/`.

```rust
#[async_trait]
pub trait LlmProvenanceStore: Send + Sync + 'static {
    async fn insert(&self, record: LlmProvenanceRecord) -> Result<(), StoreError>;
    async fn get_by_trace(&self, trace_id: TraceId, tenant: TenantId)
        -> Result<LlmProvenanceRecord, StoreError>;
    async fn list_by_swarm(&self, swarm_id: Uuid, tenant: TenantId)
        -> Result<Vec<LlmProvenanceRecord>, StoreError>;
    async fn list_by_model(&self, model: &ModelIdentity, tenant: TenantId,
                            window: DateRange) -> Result<Vec<LlmProvenanceRecord>, StoreError>;
    async fn cost_summary(&self, tenant: TenantId, window: DateRange)
        -> Result<CostSummary, StoreError>;
    async fn list_drift_events(&self, tenant: TenantId, window: DateRange)
        -> Result<Vec<ModelHashMismatchEvent>, StoreError>;
}

pub struct CostSummary {
    pub by_model: HashMap<ModelIdentity, ModelCostRow>,
    pub total_cents: f64,
    pub total_client_estimated_tokens: u64,
    pub total_provider_reported_tokens: Option<u64>,
    pub drift_ratio: Option<f64>,    // (provider - client) / client
}
```

### 7.7 Audit chain extension

New trace kinds (in addition to Sprint 6 + N12 + N13 + N14
sentinels) :

| Trace kind | Emits | Purpose |
|---|---|---|
| `LlmCall` | Every Mode C call | Main provenance trace |
| `LlmCacheHit` | N12 cache hit serving Mode C | Cache lineage |
| `LlmReplayCall` | N16 replay of Mode C | Replay provenance |
| `ModelHashMismatch` | F3 drift detection | Alert trace |
| `ToolCallExecuted` | Per tool call (Sprint 10) | Nested provenance |

All 5 are **filtered** by N16 replay (like Sprint 6 sentinels —
they're forensic markers, not re-dispatchable ops).

### 7.8 N17 metrics

Twelve Prometheus surfaces in `n17_*` namespace :

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n17_llm_calls_total` | counter | `model_id`, `outcome` |
| 2 | `n17_llm_tokens_input_total` | counter | `model_id` |
| 3 | `n17_llm_tokens_output_total` | counter | `model_id` |
| 4 | `n17_llm_cost_cents_total` | counter | `model_id`, `tenant_tier` |
| 5 | `n17_llm_response_time_seconds` | histogram | `model_id` |
| 6 | `n17_model_hash_mismatch_total` | counter | `model_id` |
| 7 | `n17_provenance_writes_total` | counter | `outcome` |
| 8 | `n17_cache_hits_total` | counter | `tenant_tier` |
| 9 | `n17_replay_calls_total` | counter | `target_region` |
| 10 | `n17_safety_flags_total` | counter | `flag` |
| 11 | `n17_token_count_drift_ratio` | gauge | `model_id` |
| 12 | `n17_provenance_query_latency_seconds` | histogram | `endpoint` |

---

## 8. Module hierarchy & data model

### 8.1 Crate layout

```text
crates/
├── efe-nano-sdk/                  (existing)
│   └── src/types/
│       ├── llm_provenance.rs      (NEW)
│       └── ...
├── efe-nano/                      (existing)
│   └── src/runtime/
│       ├── provenance_client.rs   (NEW — wraps LlmClient)
│       └── ...
├── efe-llm-provenance/            (NEW crate)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── collector.rs
│   │   ├── registry.rs
│   │   ├── store/
│   │   │   ├── trait.rs
│   │   │   ├── memory.rs
│   │   │   └── postgres.rs   (Sprint 10)
│   │   ├── metrics.rs
│   │   └── query.rs          (HTTP handlers)
│   └── Cargo.toml
```

### 8.2 SDK types layout (`efe_nano_sdk::types::llm_provenance`)

```rust
pub mod llm_provenance {
    // All types from § 7.2.
}

// Re-exports at types::*
pub use llm_provenance::{
    Blake3Hash, CacheProvenance, Completeness, CostAccounting, FinishReason, LlmProvenanceRecord,
    ModelIdentity, ProvenanceId, RegisteredModel, SafetyCategory, SafetyFlag, SamplingParams,
    ToolCallRecord, ToolCallResultKind, TokenUsage,
};
```

### 8.3 Postgres schema (Sprint 10)

```sql
CREATE SCHEMA provenance;

CREATE TABLE provenance.llm_provenance (
    provenance_id            UUID PRIMARY KEY,
    trace_id                 UUID NOT NULL UNIQUE,
    swarm_request_id         UUID NOT NULL,
    op_id                    TEXT NOT NULL,
    tenant_id                UUID NOT NULL,
    agent_id                 TEXT NOT NULL,
    agent_version            TEXT NOT NULL,
    -- Model identity
    provider                 TEXT NOT NULL,
    model_id                 TEXT NOT NULL,
    requested_version        TEXT NOT NULL,
    served_version           TEXT,
    content_hash             BYTEA,
    model_hash_verified      BOOL NOT NULL,
    -- Prompt
    prompt_template_id       TEXT NOT NULL,
    prompt_template_version  TEXT NOT NULL,
    prompt_inputs_hash       BYTEA NOT NULL,
    system_prompt_hash       BYTEA NOT NULL,
    user_prompt_hash         BYTEA NOT NULL,
    final_prompt_hash        BYTEA NOT NULL,
    -- Sampling
    sampling_json            JSONB NOT NULL,
    -- Tokens & cost
    token_usage_json         JSONB NOT NULL,
    cost_cents               NUMERIC(18, 6) NOT NULL,
    -- Provider metadata
    provider_request_id      TEXT,
    provider_response_time_ms INTEGER NOT NULL,
    provider_finish_reason   TEXT NOT NULL,
    provider_region          TEXT,
    provider_metadata_json   JSONB,
    -- Output + safety
    output_hash              BYTEA,
    completeness_json        JSONB NOT NULL,
    safety_flags_json        JSONB,
    -- Tool calls
    tool_calls_json          JSONB,
    -- PII
    pii_strategy_applied     TEXT NOT NULL,
    redaction_token_set      TEXT[],
    -- Cross-cutting
    replay_of                UUID,
    cache_hit_json           JSONB,
    -- Timing
    recorded_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX ON provenance.llm_provenance (tenant_id, recorded_at DESC);
CREATE INDEX ON provenance.llm_provenance (tenant_id, swarm_request_id);
CREATE INDEX ON provenance.llm_provenance (tenant_id, provider, model_id, recorded_at);
ALTER TABLE provenance.llm_provenance ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON provenance.llm_provenance
    USING (tenant_id = current_setting('app.current_tenant_id')::uuid);
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints

```text
GET /v1/provenance/{trace_id}
    Auth: Bearer <JWT>
    → 200 OK LlmProvenanceRecord (JSON)
    → 401 / 403 / 404

GET /v1/provenance/swarm/{swarm_request_id}
    → 200 OK [LlmProvenanceRecord]

GET /v1/provenance/model
    Query: ?provider=openai&model_id=gpt-4&from=...&to=...
    → 200 OK { records: [...], total_count, total_cost }

GET /v1/provenance/cost
    Query: ?tenant_id=...&from=...&to=...
    → 200 OK CostSummary

GET /v1/provenance/drift
    Query: ?tenant_id=...&from=...&to=...
    → 200 OK [ModelHashMismatchEvent]
```

### 9.2 Internal Rust traits

Already enumerated in § 7.3–7.6.

---

## 10. End-to-end workflows

### 10.1 Workflow A — Normal Mode C call

```
1. Dispatcher invokes Mode C op.
2. ProvenanceCollectingLlmClient.call(request):
   a. Resolve prompt template via PromptRegistry.
   b. Look up model in ModelRegistry.
   c. Verify model hash → ok or ModelHashMismatch trace.
   d. Apply PII redaction ; record strategy + token set.
   e. Compute final_prompt_hash, system_prompt_hash, etc.
   f. Call inner.call() → LlmCallResponse.
   g. Parse response → output_hash, token_usage.
   h. Assemble LlmProvenanceRecord.
   i. Collector.post_call(record):
      - LlmProvenanceStore.insert(record).
      - AuditWriter.record(LlmCall trace).
3. Return LlmCallResponse.
```

### 10.2 Workflow B — Cache hit (N12 integration)

```
1. Dispatcher invokes Mode C op.
2. IdempotencyCache hit on the cache key.
3. Cache returns the recorded OperationResult.
4. Dispatcher records:
   a. CacheHit sentinel trace (existing N12 behaviour).
   b. LlmCacheHit provenance trace (NEW) pointing at
      original_provenance_id (looked up via the cache entry).
5. ProvenanceStore.insert(cache_hit_record).
```

### 10.3 Workflow C — Replay (N16 integration)

```
1. N16 worker re-dispatches a Mode C op.
2. ProvenanceCollectingLlmClient.call(replay_request):
   a. Sets replay_of = Some(original_provenance_id) in context.
3. Provenance record emitted with replay_of populated.
4. N17 query for the swarm shows BOTH original + replay records.
```

### 10.4 Workflow D — Cost reconciliation (Sprint 10 preview)

```
1. M31 billing fetches OpenAI invoice for previous day.
2. Job queries LlmProvenanceStore.cost_summary(tenant, day).
3. Compute drift = (provider_invoice - provenance.total_cents) / provider_invoice.
4. If |drift| > 5%, alert + open ticket.
```

### 10.5 Workflow E — Model hash mismatch detection

```
1. Op dispatched ; ProvenanceCollectingLlmClient queries
   ModelRegistry.verify_hash(identity, current_hash).
2. Registry returns Mismatch (provider rotated model silently).
3. Collector emits ModelHashMismatch trace.
4. Op fails with OperationErrorKind::ModelIntegrityViolation.
5. Alert raised via PagerDuty (configured externally).
6. Ops updates registry expected_content_hash to the new value
   after validation ; resumes traffic.
```

---

## 11. Five-day implementation plan

### Day 1 — SDK types + chain extensions
* `efe-nano-sdk/src/types/llm_provenance.rs` (~600 LOC).
* `Blake3Hash` newtype + hex encoding.
* Bumped `REPLAY_SCHEMA_VERSION` for N17 changes (constant rename).
* New trace kinds added to existing trace-kind enum.
* New `OperationErrorKind::ModelIntegrityViolation`.
* 40+ unit tests : serde round-trip, hash equality, enum
  exhaustiveness.

### Day 2 — `efe-llm-provenance` crate skeleton + ModelRegistry + Store
* New crate `efe-llm-provenance` registered in workspace.
* `ModelRegistry` trait + `InMemoryModelRegistry` impl.
* `LlmProvenanceStore` trait + `InMemoryLlmProvenanceStore` impl.
* 50+ unit tests.

### Day 3 — Collector + ProvenanceCollectingLlmClient
* `DefaultLlmProvenanceCollector`.
* `ProvenanceCollectingLlmClient` wrapping any `LlmClient`.
* Integration with Sprint 5 `LlmAugmentedRuntime`.
* Tests : mock LlmClient + collector → verify provenance record.

### Day 4 — Cache + Replay integration
* N12 idempotency cache extended to emit `LlmCacheHit` provenance.
* N16 replay engine emits `LlmReplayCall` provenance.
* Cross-cutting tests.

### Day 5 — Query API + N17 metrics + final verification
* HTTP query handlers for the 5 endpoints.
* 12 N17 Prometheus surfaces wired.
* Integration tests (`phase_3_n17_*`).
* Clippy strict + workspace build.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (15 tests, one per G)

```
phase_3_n17_model_identity_recorded
phase_3_n17_prompt_hashes_stable_across_serdes
phase_3_n17_hash_mismatch_blocks_call
phase_3_n17_provider_metadata_recorded
phase_3_n17_token_drift_visible
phase_3_n17_tool_call_record_serdes_round_trip
phase_3_n17_sampling_params_recorded
phase_3_n17_safety_flag_taxonomy_round_trip
phase_3_n17_provenance_cross_tenant_blocked
phase_3_n17_four_query_patterns_work
phase_3_n17_cost_recomputable_from_tokens_prices
phase_3_n17_pii_strategy_recorded_at_call_site
phase_3_n17_cache_hit_records_provenance
phase_3_n17_replay_call_recorded
phase_3_n17_pre_n17_chain_still_verifies
```

### 12.2 Property-based tests

* `proptest_provenance_record_serdes` — every field shape
  serialize↔deserialize.
* `proptest_blake3_hash_hex_round_trip`.
* `proptest_token_drift_computation_consistent`.

### 12.3 Compliance scenario tests

* `compliance_eu_ai_act_audit_query_returns_records`.
* `compliance_gdpr_article_22_explanation_includes_prompt`.
* `compliance_hipaa_pii_strategy_recorded`.

### 12.4 Chaos tests

* `chaos_provenance_store_down_fails_op` — F4 enforcement.
* `chaos_model_hash_changes_mid_run_alerts` — F3 detection.

---

## 13. N12 / N13 / N14 / N15 / N16 alignment

| Subsystem | Concern | Resolution |
|---|---|---|
| **N12 cache** | Cache hits skip the LLM → no LlmCall record | Emit `LlmCacheHit` provenance with `original_trace_id` link |
| **N13 shadow** | Shadow calls are real LLM calls too | Shadow's LlmCall records its own provenance ; `audit_tags` includes `shadow` |
| **N14 envelope** | Envelope summarizes swarm-level data | LlmProvenance per-op is queried separately ; envelope carries `llm_call_count` |
| **N15 fairness** | Mode C calls gated by N15 LLM bucket | Provenance records `gated_at: SystemTime` to correlate with N15 metrics |
| **N16 replay** | Replay re-emits provenance | `LlmReplayCall` records `replay_of` pointer ; N16 comparator uses model_identity equality |

---

## 14. Sprint 9 ship vs Sprint 10/11 split

### Sprint 9 (this design)
- All 6 capabilities from § 0.
- 15 acceptance tests.
- In-memory store + registry.
- ProvenanceCollectingLlmClient wired with Sprint 5
  LlmAugmentedRuntime.

### Sprint 10 N18 — Postgres backing + cost reconciliation
- `PostgresLlmProvenanceStore` with RLS.
- Daily cost reconciliation job vs M31 billing.
- Schema migration + GC by retention TTL.

### Sprint 11 N19 — Drift-detection dashboard
- Grafana dashboard for `n17_token_count_drift_ratio`.
- Statistical alarm rules.

### Sprint 12+ — Embedding-based semantic comparison
- Vector similarity for N16 Mode C tolerance.
- Needs M28 embedding index.

---

## 15. Annexes

### A1 — Regulatory mapping

| Regulation | Article | N17 commitment |
|---|---|---|
| EU AI Act | Art. 12 (record-keeping) | G1 + G2 + G4 + G5 |
| GDPR | Art. 22 (right to explanation) | G2 + G7 |
| GDPR | Art. 25 (privacy by design) | ADR-4 |
| HIPAA Security Rule | § 164.312(b) (audit controls) | G12 + I19 |
| FTC AI Order (2023) | Transparency | G1 + G4 |
| EU DSA | Art. 27 (transparency report) | G10 |

### A2 — Prompt-hash spec

```
BLAKE3-256 over UTF-8-encoded prompt content.
Hex-encoded for wire format (64 chars).
Stable across serde (no canonical-JSON normalization).
```

### A3 — Sample provenance JSON

```jsonc
{
  "provenance_id": "550e8400-e29b-41d4-a716-446655440000",
  "trace_id": "...",
  "swarm_request_id": "...",
  "op_id": "summarize_input",
  "tenant_id": "...",
  "agent_id": "summarize_strict",
  "agent_version": "1.2.3",
  "model_identity": {
    "provider": "openai",
    "model_id": "gpt-4-turbo",
    "requested_version": "2024-04-09",
    "served_version": "2024-04-09",
    "content_hash": null,
    "supports_tool_use": true
  },
  "model_hash_verified": false,
  "prompt_template_id": "summarize-v3",
  "prompt_template_version": "1.0.0",
  "prompt_inputs_hash": "a3f5...",
  "system_prompt_hash": "b7e1...",
  "user_prompt_hash": "c4d2...",
  "final_prompt_hash": "e9f0...",
  "sampling": {
    "temperature": 0.0,
    "top_p": null,
    "max_tokens": 500,
    "seed": null,
    "stop_sequences": []
  },
  "token_usage": {
    "client_estimated_input": 1240,
    "client_estimated_output": 312,
    "provider_reported_input": 1238,
    "provider_reported_output": 314,
    "is_estimated": false
  },
  "cost_accounting": {
    "cost_cents": 4.124,
    "prices_table_version": 17,
    "provider_quoted_cost_cents": null,
    "overflow_capped": false
  },
  "provider_request_id": "req_abc123",
  "provider_response_time_ms": 487,
  "provider_finish_reason": "stop",
  "provider_region": "us-east-1",
  "provider_metadata": {
    "system_fingerprint": "fp_a1b2c3d4"
  },
  "output_hash": "f1a2...",
  "completeness": { "kind": "complete" },
  "safety_flags": [],
  "tool_calls": [],
  "pii_strategy_applied": "Strict",
  "redaction_token_set": ["[REDACTED-EMAIL]", "[REDACTED-PHONE]"],
  "replay_of": null,
  "cache_hit": null,
  "recorded_at": "2026-05-20T15:42:17Z"
}
```

### A4 — Operator runbook

#### A4.1 Registering a new model

```bash
efe-models register \
  --provider openai \
  --model-id gpt-4-turbo \
  --version 2024-04-09 \
  --price-input-tokens-cents-per-million 1000 \
  --price-output-tokens-cents-per-million 3000
```

#### A4.2 Investigating a drift event

```bash
efe-provenance drift --tenant <id> --from 2026-05-01 --to 2026-05-31
```

#### A4.3 GDPR Article 22 query

```bash
efe-provenance get --trace-id <uuid>
```

#### A4.4 Cost summary for finance

```bash
efe-provenance cost --tenant <id> --from 2026-05-01 --to 2026-05-31 --json > cost.json
```

### A5 — Comparison with related systems

| System | Their approach | N17's improvement |
|---|---|---|
| LangSmith | Per-call trace with prompt + output | + HMAC chain integrity, + cross-region replay link |
| OpenAI Audit Logs | Provider-side per-request log | + tenant-side comparable record, + drift detection |
| Anthropic Workbench | Prompt versioning | + same, + integrated with Sprint 6 audit chain |
| AWS Bedrock CloudTrail | Coarse-grained AWS API call log | + content-addressed prompts, + per-op detail |

---

**End of N17 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 9 (Phase 3, day 31 – 35).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 → N17 (this doc) → N18 Postgres backing → N19 drift dashboard.

**Last-mile checklist** (Sprint 9 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on new crate.
- [ ] Pre-N17 chains still verify (regression test).
- [ ] Cost summary endpoint returns drift_ratio.
- [ ] Compliance team sign-off on EU AI Act mapping (Annex A1).
- [ ] CLI tools `efe-models` + `efe-provenance` shipped.
- [ ] PagerDuty integration : F3 + F4 paging routes verified.
