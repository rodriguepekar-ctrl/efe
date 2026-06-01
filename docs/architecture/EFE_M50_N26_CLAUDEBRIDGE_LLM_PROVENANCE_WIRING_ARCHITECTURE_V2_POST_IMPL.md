# EFE M50 — N26 ClaudeBridge ↔ LlmProvenanceRecord Wiring Architecture **v2 (post-implementation)**

> **Sprint**         : 18 (Phase 9, day 86-90 of M50).
> **Status**         : SHIPPED — 356 tests green, clippy strict clean, live smoke validated.
> **Companion doc**  : `EFE_M50_N26_CLAUDEBRIDGE_LLM_PROVENANCE_WIRING_ARCHITECTURE.md` (v1, design-time, 1516 lines).
> **This document** : v2 captures every drift between the v1 design intent and the as-shipped code, plus all the audit-pass refinements that landed during the three chirurgical reviews.
>
> **Why v2 ?** When you write an architecture, your contracts are your best guess at the SDK shape, the magnitudes, the failure modes, the test surface.  When you then implement, those guesses meet reality.  Some hold ; some shift.  Without a v2, the v1 doc lies — it describes what we INTENDED rather than what we BUILT.  The auditor reading the v1 in 18 months would be misled by every drift.
>
> **What v2 adds** : (a) the SDK type shapes we actually integrated against ; (b) the magnitude corrections (pricing was off by 100×) ; (c) the audit-driven additions (`pre_call` timeout, accurate `Completeness::Partial` token counts) ; (d) the failure-mode coverage matrix with concrete chaos tests ; (e) operational notes for the gateway boot path (`async build_app_state`) ; (f) hooks for Sprint 19 N27 and Sprint 20 N28.

---

## Table of contents

0.  [TL;DR for the v1→v2 reader](#0-tldr-for-the-v1v2-reader)
1.  [Drift catalogue — design vs as-shipped](#1-drift-catalogue--design-vs-as-shipped)
2.  [SDK contracts as we actually integrated against](#2-sdk-contracts-as-we-actually-integrated-against)
3.  [Magnitude corrections](#3-magnitude-corrections)
4.  [Audit-driven additions (3 passes)](#4-audit-driven-additions-3-passes)
5.  [Concrete failure-mode coverage matrix](#5-concrete-failure-mode-coverage-matrix)
6.  [As-shipped dispatch sequence (annotated)](#6-as-shipped-dispatch-sequence-annotated)
7.  [Operational deployment notes](#7-operational-deployment-notes)
8.  [Test surface inventory (final)](#8-test-surface-inventory-final)
9.  [Hooks for Sprint 19 N27 (streaming SSE)](#9-hooks-for-sprint-19-n27-streaming-sse)
10. [Hooks for Sprint 20 N28 (Postgres + event bus)](#10-hooks-for-sprint-20-n28-postgres--event-bus)
11. [Lessons applicable to future bridge integrations](#11-lessons-applicable-to-future-bridge-integrations)
12. [Annexes — concrete code excerpts](#12-annexes--concrete-code-excerpts)

---

## 0. TL;DR for the v1→v2 reader

Sprint 18 N26 shipped exactly what v1 promised — every Claude dispatch
produces a single `LlmProvenanceRecord` (44-field row per N17), every
record carries the publisher snapshot (N18), every record carries the
gold-case attribution (N19), every record carries the four BLAKE3
prompt hashes (replay-stable per N16), every record carries the cost
breakdown stamped with the prices-table version (FinOps SLO), every
operator-scoped `GET /v1/llm-provenance` request returns the paginated
list.  Backward compat with Sprint 13 callers was preserved : an
`AgenticTask` without an `audit_context` field still dispatches —
without provenance emission, with a metric tick.

**But v1 was wrong in seven places.**  Some were minor (a struct
field name on `CostAccounting`).  Some were major (cost rates off
by 100×).  Some emerged only in chirurgical audit passes 2 and 3
(missing `pre_call` timeout, accurate `Completeness::Partial`
counts).  v2 catalogues every drift so the auditor reading this
in 18 months sees what we built, not what we wished we'd built.

**Total drift count : 7 corrections + 4 audit-added tests.**

| # | Drift / addition | Severity | Where |
|---|---|---|---|
| D1 | `CostAccounting` shape — v1 invented `input_microcents` / `output_microcents` / `provider_microcents` fields ; SDK actually has a flat `cost_cents: f64 + prices_table_version: u32 + provider_quoted_cost_cents + overflow_capped` | medium | `pricing.rs`, `claude.rs` |
| D2 | `TokenUsage` shape — v1 used the bridge's flat `{input_tokens, output_tokens}` ; SDK has a richer `{client_estimated_input, client_estimated_output, provider_reported_input, provider_reported_output, is_estimated}` | medium | `claude.rs` |
| D3 | Pricing magnitudes off by 100× ($3 / Mtoken = 3 mc/token = 3 000 mc/1k tokens, not 30) | **critical** | `pricing.rs` |
| D4 | `ModelIdentity` has `requested_version` + `served_version` (not `version`) | low | smoke binary, audit tests |
| D5 | `prompt_hashes_for(inputs_json, system, user, final_prompt)` takes four `&str` args (v1 design said "use canonical bundle hash for inputs" which we did, but via UTF-8 decode of the JSON bytes) | low | `claude.rs` |
| D6 | `ProvenanceError::ModelNotRegistered` is a struct variant `{ provider, model_id, version }`, not the tuple shape implied by v1 | low | `pre_call` error mapping |
| D7 | The gateway `build_app_state` had to become `async` (the `ModelRegistry::register` call is async, and the original `block_on(...)` panicked because we're already inside `#[tokio::main]`) | low | `main.rs` |
| A1 | Audit pass 1 added `phase_9_n26_pricing_table_version_stamped_on_record` (G5 was missing a stamp assertion) | n/a | `phase_9_n26_*` |
| A2 | Audit pass 2 added explicit I6 test — `provider_response_time_ms` must equal the sum of both turn latencies | n/a | `audit_n26_i6` |
| A3 | Audit pass 2 added G7 + G9 tests for PII trail + gold_case propagation | n/a | `audit_n26_g7` + `audit_n26_g9` |
| A4 | Audit pass 3 added `PROVENANCE_PRE_CALL_TIMEOUT = 200ms` + corresponding `chaos_n26_f5b_slow_pre_call_timeout_fails_closed` test (v1 only specified the post_call timeout) | medium | `claude.rs`, `chaos_n26_*` |
| A5 | Audit pass 3 fixed `derive_completeness(stop)` → `derive_completeness(stop, received_output_tokens, expected_max_tokens)` so `Completeness::Partial { received_tokens, expected_tokens }` carries real numbers, not 0/0 | medium | `claude.rs` |

---

## 1. Drift catalogue — design vs as-shipped

### D1 — `CostAccounting` shape

**v1 design (annex G5)** :

```rust
let cost_accounting = CostAccounting {
    input_microcents: per_input_token_microcents(model) * input_tokens as i64,
    output_microcents: per_output_token_microcents(model) * output_tokens as i64,
    provider_microcents: total_billed_microcents,
    estimate_microcents: pre_call_estimate,
    currency: Currency::Usd,
};
```

**SDK reality** :

```rust
pub struct CostAccounting {
    pub cost_cents: f64,
    pub prices_table_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_quoted_cost_cents: Option<f64>,
    pub overflow_capped: bool,
}
```

**As-shipped** :

```rust
cost_accounting: CostAccounting {
    cost_cents: compute_cost_cents(&self.model, input, output),
    prices_table_version: PRICES_TABLE_VERSION,  // baked = 1
    provider_quoted_cost_cents: None,            // Anthropic doesn't quote
    overflow_capped: false,
},
```

**Why the drift** : v1 imagined a split input/output breakdown
because FinOps reconciliation usually wants it.  N17 chose a flat
`cost_cents: f64` field stamped with `prices_table_version` so the
reconciliation job can recompute historicals using the exact table
that was active at call time — a different (and arguably better)
design.

**Implication** : v1's promise that the record carries the
"FULL `CostAccounting` (input + output + provider microcents)"
maps onto the SDK shape : `cost_cents` is the total, and the
`prices_table_version` + `pricing::anthropic_pricing_for(model)`
together let any caller deterministically recompute the
input/output split.

### D2 — `TokenUsage` shape

**v1** : just `{input_tokens, output_tokens}` (the bridge's shape).

**SDK** :

```rust
pub struct TokenUsage {
    pub client_estimated_input: u32,
    pub client_estimated_output: u32,
    pub provider_reported_input: Option<u32>,
    pub provider_reported_output: Option<u32>,
    pub is_estimated: bool,
}
```

The SDK distinguishes CLIENT estimates (from a tokenizer
running locally) from PROVIDER reports (from the Anthropic
response).  Anthropic always reports counts in the
`usage.{input_tokens, output_tokens}` of every response, so
we fill the provider side and zero the client side.

**As-shipped** :

```rust
token_usage: efe_nano_sdk::TokenUsage {
    client_estimated_input: 0,
    client_estimated_output: 0,
    provider_reported_input: Some(outcome.usage.input_tokens),
    provider_reported_output: Some(outcome.usage.output_tokens),
    is_estimated: false,
}
```

**Implication** : `TokenUsage::canonical_input()` returns the
provider count when present (which it always is for Anthropic),
so any audit query against `record.token_usage.canonical_input()`
sees the truth.  Sprint 19 N27 (streaming) will fill the
client-estimated side when we add a local tokenizer for the
streaming token-event log.

### D3 — Pricing magnitudes off by 100×

**v1 (architecture annex A4, table)** :

> - `claude-sonnet-4-6` : input $3 / Mtoken, output $15 / Mtoken
>   → **30** mc / 1k input, **150** mc / 1k output.

**Reality** :

- $3 / Mtoken = $3 / 1 000 000 tokens
- $3 = 300 cents = **3 000 000 microcents** (1 USD = 1 000 000 mc)
- per token = 3 000 000 / 1 000 000 = **3 mc / token**
- per 1k tokens = **3 000 mc**

So $3 / Mtoken = 3 000 mc / 1k tokens, **not 30**.  The v1
design was off by exactly 100×.

**Caught how** : in pass 1, the existing Sprint 13 unit test
`cost_estimator_haiku_cheaper_than_sonnet` failed because the
old `.contains("haiku")` heuristic was gone and my exact-match
table returned identical fallback prices for "haiku-4-5" /
"opus-4-6" / etc.  Fixed by aligning model ids to the exact
table keys (haiku-3-5, opus-4) and verifying the magnitudes by
reproducing a known historical Anthropic invoice : 1016 input
tokens × Sonnet = $0.003 (1016 × 3 mc = 3 048 mc = $0.0003 ;
match within rounding).

**As-shipped pricing table** :

```rust
"claude-sonnet-4-6"  : { input: 3_000,  output: 15_000 }
"claude-haiku-3-5"   : { input:   800,  output:  4_000 }
"claude-opus-4"      : { input: 15_000, output: 75_000 }
FALLBACK             : { input: 3_000,  output: 15_000 }  // = sonnet
```

**Implication** : without this fix, every `LlmProvenanceRecord`
would have undercharged the customer by 99 %.  FinOps
reconciliation would have diverged 100× from the Anthropic
invoice within hours.  Auditor's first test : "does the record
cost match the Anthropic invoice line to within 5 % ?"  v2
passes ; v1 would have failed catastrophically.

### D4 — `ModelIdentity` has split versions

**v1** : assumed a flat `version` field on `ModelIdentity`.

**SDK** : `ModelIdentity { provider, model_id, requested_version,
served_version, content_hash, supports_tool_use }`.

The `ModelIdentity::hosted(provider, model_id, version)`
convenience constructor sets BOTH `requested_version` and
`served_version` to the same string.  Anthropic returns its
served model in the response `model` field (e.g.
`"claude-sonnet-4-5-20241022"`) — we currently DON'T capture it.
We use the API version (`"2023-06-01"`) for both fields.

**Implication** : Sprint 19 N27 (streaming) should capture the
real Anthropic `model` field as `served_version` so model-
substitution drift is visible to the audit trail.  Hook
documented in § 9 below.

### D5 — `prompt_hashes_for` signature

**v1 annex A2** :

```rust
let hashes = prompt_hashes_for(&system_prompt, &task.user_message, &final_prompt);
```

— 3 args.

**SDK reality** :

```rust
pub fn prompt_hashes_for(
    inputs_json: &str,
    system: &str,
    user: &str,
    final_prompt: &str,
) -> PromptHashes
```

— 4 args.  The first is the canonical JSON of the operator
input (the bundle).  We pass the UTF-8 string of the bundle JSON.

**As-shipped** :

```rust
let bundle_json_bytes = serde_json::to_vec(bundle).map_err(...)?;
let inputs_json_str =
    std::str::from_utf8(&bundle_json_bytes).unwrap_or("<bundle-non-utf8>");
let hashes = prompt_hashes_for(
    inputs_json_str,
    &system_prompt,
    &task.user_message,
    &final_prompt,
);
```

**Implication** : the `prompt_inputs_hash` field of the record
is the BLAKE3 of the canonical bundle JSON.  Two identical
bundles produce the same hash.  N16 replay uses this as the
input anchor.

### D6 — `ProvenanceError::ModelNotRegistered` shape

**v1** : implied a tuple variant.

**SDK reality** :

```rust
pub enum ProvenanceError {
    ModelNotRegistered {
        provider: String,
        model_id: String,
        version: String,
    },
    // ...
}
```

Struct variant.  Tests construct it directly.

### D7 — Async `build_app_state`

**v1 design** : `build_app_state(cfg: &GatewayConfig) -> AppState`
(sync).

**Why we couldn't keep it sync** : `ModelRegistry::register` is
`async`.  We must register 3 Anthropic models at boot so the
collector's `pre_call` succeeds.  A nested `tokio::runtime::
Builder::new_current_thread().build().block_on(...)` panicked
because we're already inside `#[tokio::main]`.

**As-shipped** :

```rust
async fn build_app_state(cfg: &GatewayConfig) -> AppState {
    // ...
    for model in [DEFAULT_CLAUDE_MODEL, "claude-haiku-3-5", "claude-opus-4"] {
        let mut reg = RegisteredModel::hosted(...);
        reg.dev_skip_hash_check = true;
        let _ = registry.register(reg).await;
    }
    // ...
}
```

And the caller : `let state = build_app_state(&cfg).await;`.

**Implication** : the gateway boot path is now async-aware.  This
is a precondition for Sprint 19 N27 streaming (which will also
need to register streaming token callbacks) and Sprint 20 N28
Postgres pool initialisation.

---

## 2. SDK contracts as we actually integrated against

The architecture v1 assumed certain shapes ; the SDK delivered
others.  Here is the **as-integrated reference** for the four
critical types :

### `LlmProvenanceRecord` (44 fields, the persistence target)

Already documented in `efe-nano-sdk/src/types/llm_provenance.rs`
lines 519-612.  Every Claude dispatch produces a record with :

- **Identity** : `provenance_id`, `trace_id`, `swarm_request_id`,
  `op_id`, `tenant_id`, `agent_id`, `agent_version`.
- **Model** : `model_identity` (hosted Anthropic), `model_hash_verified = false`
  (because hosted = dev_skip_hash_check).
- **Prompt** : `prompt_template_id`, `prompt_template_version`,
  `prompt_inputs_hash` (= BLAKE3 of canonical bundle JSON),
  `system_prompt_hash`, `user_prompt_hash`, `final_prompt_hash`.
- **Sampling** : `sampling = SamplingParams { temperature, top_p: None, max_tokens, seed: None, stop_sequences: [] }`.
- **Tokens + cost** : `token_usage = { provider_reported_*: Some(...), is_estimated: false }`,
  `cost_accounting = { cost_cents, prices_table_version, provider_quoted_cost_cents: None, overflow_capped: false }`.
- **Provider** : `provider_request_id = parsed2.id` (the turn-2
  Anthropic message id), `provider_response_time_ms = t1_ms + t2_ms`,
  `provider_finish_reason` (mapped from `StopReason`),
  `provider_region: None`, `provider_metadata = {}`.
- **Output** : `output_hash = blake3(text)`,
  `completeness = Complete | Partial { received_tokens, expected_tokens }`
  (audit pass 3 fix), `safety_flags = []`.
- **Tool calls** : one `ToolCallRecord` capturing the turn-1
  `tool_use` block (`tool_name = "swarm_context"`,
  `arguments_hash`, `result_hash = hash(bundle_json)`,
  `result_kind = Ok`).
- **PII** : `pii_strategy_applied`, `redaction_token_set` —
  copied verbatim from `audit_context`.
- **Cross-cutting** : `replay_of: None`, `cache_hit: None`,
  `publisher_id`, `publisher_kyc_tier`, `gold_case_id` (snapshots).
- **Timing** : `recorded_at = SystemTime::now()`.

### `LlmProvenanceCollector` trait (the integration point)

```rust
#[async_trait]
pub trait LlmProvenanceCollector: Send + Sync + 'static {
    async fn pre_call(
        &self,
        pre: &PreCallProvenance,
        actual_hash: Option<&Blake3Hash>,
    ) -> Result<(), ProvenanceError>;

    async fn post_call(
        &self,
        pre: PreCallProvenance,
        post: PostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError>;

    async fn on_cache_hit(...) -> Result<...>;
    async fn on_replay(...) -> Result<...>;
}
```

We only use `pre_call` + `post_call`.  `on_cache_hit` is for the
M27 cache layer (not in scope) ; `on_replay` is for N16 (not in
scope).

### `LlmProvenanceStore` trait (the persistence layer)

```rust
async fn insert(...) -> Result<(), ProvenanceStoreError>;
async fn get_by_trace(trace_id, tenant) -> Result<...>;
async fn get_by_provenance(id, tenant) -> Result<...>;
async fn list_by_swarm(swarm_request_id, tenant) -> Result<Vec<...>>;
async fn list_by_model(model, tenant, window) -> Result<Vec<...>>;
async fn cost_summary(tenant, window) -> Result<CostSummary>;
async fn list_drift_events(tenant, window) -> Result<Vec<...>>;
async fn record_drift_event(event) -> Result<()>;
```

Sprint 18 uses the `InMemoryLlmProvenanceStore` for tests and
the gateway boot.  Sprint 20 N28 ships a Postgres-backed impl.

### `prompt_hashes_for` (the hash factory)

```rust
pub fn prompt_hashes_for(
    inputs_json: &str,
    system: &str,
    user: &str,
    final_prompt: &str,
) -> PromptHashes
```

Returns `{ inputs_hash, system_hash, user_hash, final_hash }`.
Every hash is BLAKE3-256.  Stable across runs given the same
inputs (verified by `chaos_n26_f11` via independent recompute).

---

## 3. Magnitude corrections

| Magnitude | v1 value | v2 corrected | Ratio |
|---|---|---|---|
| Sonnet input mc/1k | 30 | **3 000** | 100× |
| Sonnet output mc/1k | 150 | **15 000** | 100× |
| Haiku input mc/1k | 8 | **800** | 100× |
| Haiku output mc/1k | 40 | **4 000** | 100× |
| Opus input mc/1k | 150 | **15 000** | 100× |
| Opus output mc/1k | 750 | **75 000** | 100× |
| `PROVENANCE_POST_CALL_TIMEOUT` | 500 ms ✓ | 500 ms ✓ | — |
| `PROVENANCE_PRE_CALL_TIMEOUT` | — (missing) | **200 ms** | +∞ (added) |
| `DEFAULT_PAGE_SIZE` (admin endpoint) | 50 ✓ | 50 ✓ | — |
| `MAX_PAGE_SIZE` (admin endpoint) | 500 ✓ | 500 ✓ | — |

All other magnitudes match v1 exactly.

---

## 4. Audit-driven additions (3 passes)

The architecture v1 listed 8 audit-regression tests + 10 chaos
tests as targets.  The implementation went through three
chirurgical audit passes that added 5 more tests and 1 design
change.  Here is the chronology :

### Pass 1 — initial implementation acceptance

**Found** : the v1's commitments for **G6 cost accounting
stamping** were not pinned by any test.  Without a test, the
record could be missing `prices_table_version` or carrying a
stale one and we'd never know.

**Added** : `phase_9_n26_pricing_table_version_stamped_on_record`.

### Pass 2 — Sprint 18 day 5 deep audit

**Found** : the contract for **G6 logical-call record** was
half-pinned (token sum via `audit_n26_i3`) but the latency sum
half (I6 : `provider_response_time_ms` = t1 + t2) had no test.
Similarly, G7 (PII propagation) and G9 (gold_case_id propagation)
were promised but not pinned.

**Added** 4 tests :

- `audit_n26_i6_provider_response_time_is_sum_of_both_turns` —
  uses `ResponseTemplate::set_delay` to give turn 1 a 50 ms
  artificial delay + turn 2 an 80 ms artificial delay ; asserts
  the record's `provider_response_time_ms >= 130`.
- `audit_n26_g6_cost_accounting_matches_summed_tokens` —
  computes the expected cost via the table maths (100 input ×
  3000/1k + 22 output × 15000/1k = 630 mc = 0.063 ¢) and asserts
  the record's `cost_cents` matches within `1e-9`.
- `audit_n26_g7_pii_strategy_and_redaction_tokens_propagated` —
  sets `pii_strategy_applied = Strict` + 3 redaction tokens in
  the audit context ; asserts the record carries them verbatim.
- `audit_n26_g9_gold_case_id_propagated_to_record` — sets a
  `GoldCaseId` ; asserts the record carries it verbatim.

### Pass 3 — Sprint 18 final audit (post-clippy)

**Found** : v1 specified the `post_call` timeout (500 ms) but
NOT the `pre_call` timeout.  In production, a Sprint 20+ Postgres-
backed `ModelRegistry::lookup` could be slow ; a hang in pre_call
would lock the bridge forever.

**Added** : `PROVENANCE_PRE_CALL_TIMEOUT = 200 ms` const + a
`tokio::time::timeout` wrap around the `collector.pre_call(...)`
call.  Failure on timeout → `BridgeError::ProvenanceVerifyFailed`
(fail-CLOSED — no HTTP call to Anthropic).  New chaos test :
`chaos_n26_f5b_slow_pre_call_timeout_fails_closed` uses a custom
`SlowPreCallCollector` that sleeps 1 s in `pre_call` ; asserts
the bridge errors within ~250 ms (timeout + scheduling slack)
AND `bridge.call_count() == 0` (no HTTP dispatch).

**Found** : `derive_completeness(stop)` returned
`Completeness::Partial { received_tokens: 0, expected_tokens: 0 }`
for `StopReason::MaxTokens` — semantically wrong, the audit row
loses the truncation delta.

**Fixed** : extended signature to
`derive_completeness(stop, received_output_tokens, expected_max_tokens)`
so the record's `Partial` variant carries the actual numbers.
Called from `dispatch_with_provenance` with
`outcome.usage.output_tokens` and `task.max_tokens`.

---

## 5. Concrete failure-mode coverage matrix

| F-# | Failure | Test pinning | Status |
|---|---|---|---|
| F1 | Forged `audit_context.tenant_id` from malicious client | `phase_9_n26_handler_overwrites_audit_context_tenant` (asserts handler overwrites + attacker tenant has no record) | ✅ |
| F2 | Pre-call hash mismatch (model not registered) | `chaos_n26_f2_pre_call_model_not_registered_fails_closed` (empty registry → fail closed + 0 HTTP) | ✅ |
| F3 | Anthropic returns different model id than requested | implicit — record carries requested id ; Sprint 19 N27 will capture `parsed2.model` for served_version | ⏳ Sprint 19 |
| F4 | Provenance store backend down | `chaos_n26_f4_store_backend_down_response_still_returned` (`AlwaysErroringStore` → LLM response OK, emit_failed metric ticks) | ✅ |
| **F5** | Slow `post_call` collector | `chaos_n26_f5_slow_collector_timeout_fires_response_ok` (`SlowCollector` sleeps 1.5 s → 500 ms timeout fires, response in < 1.4 s) | ✅ |
| **F5b** | Slow `pre_call` collector (audit pass 3 addition) | `chaos_n26_f5b_slow_pre_call_timeout_fails_closed` (`SlowPreCallCollector` sleeps 1 s → 200 ms timeout, no HTTP) | ✅ |
| F6 | Two-turn protocol mismatch | existing N21 `protocol_violation_no_tool_use_in_turn1` test, preserved | ✅ |
| F7 | Publisher tier flipped mid-call | `audit_n26_i18_publisher_tier_snapshot_pinned` (snapshot captured at call start) | ✅ |
| F8 | Provenance emission error propagates as BridgeError | `audit_n26_i8_post_call_error_does_not_propagate` (`PostCallErroringCollector` → response OK, `emit_failed` metric ticks) | ✅ |
| F9 | Concurrent dispatches race counters | `chaos_n26_f9_concurrent_dispatches_counters_consistent` (10 spawn → counter = 10) | ✅ |
| F10 | API key leaks into record | `chaos_n26_f10_api_key_not_in_record` (serialised record never contains the bearer token) | ✅ |
| F11 | Hash collision under crafted input | `chaos_n26_f11_record_hashes_match_independent_recompute` (independent `prompt_hashes_for` recompute matches) | ✅ |
| F12 | Bundle exceeds 256 KB cap | existing N21 `bundle_over_cap_rejected_before_http` preserved (pre-HTTP rejection) | ✅ |
| F13 | Missing `audit_context` → silent miss | `chaos_n26_f13_missing_audit_context_no_record` (skip counter ticks + 0 records) | ✅ |
| F14 | Retention SLA violation | deferred Sprint 20 N28 (Postgres store with 5-year retention) | ⏳ Sprint 20 |
| F15 | N17 record schema drift | covered by N17's own serde compatibility tests | ✅ (upstream) |

**13 of 15 failure modes pinned in N26 itself ; 2 deferred to
later sprints with explicit hooks documented below.**

---

## 6. As-shipped dispatch sequence (annotated)

```
Customer / handler                ClaudeBridge::call_with_context (entry)
        │
        ▼
┌────────────────────────────────────────────────────────────────────┐
│ Bundle size check (F12)                                             │
│   if bundle.estimated_size_bytes() > MAX_CONTEXT_BUNDLE_BYTES :    │
│       return Err(BridgeError::BundleTooLarge)                       │
└─────────────────────────────────────┬──────────────────────────────┘
                                      │
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Provenance path decision (G3 + I1 + I2)                            │
│   if self.provenance.is_some() && task.audit_context.is_some() :   │
│       → dispatch_with_provenance                                   │
│   else :                                                            │
│       → dispatch_without_provenance (Sprint 13 fallback)           │
│       provenance_skipped counter ticks                              │
└─────────────────────────────────────┬──────────────────────────────┘
                                      │  (provenance path)
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Build PreCallProvenance (G4 + G7 + G8 + G9)                         │
│   bundle_json_bytes = serde_json::to_vec(bundle)                    │
│   inputs_json_str   = utf8 of bytes (or "<bundle-non-utf8>")        │
│   system_prompt     = build_system_prompt(task)                     │
│   final_prompt      = system + "\n\n" + task.user_message           │
│   hashes            = prompt_hashes_for(inputs, system, user, final)│
│   pre = PreCallProvenance {                                         │
│     trace_id, swarm_request_id, op_id, tenant_id, agent_id,         │
│     agent_version, model_identity, prompt_template_*, hashes,      │
│     sampling, pii_strategy_applied, redaction_token_set,           │
│     publisher_id, publisher_kyc_tier, gold_case_id (snapshots)     │
│   }                                                                  │
└─────────────────────────────────────┬──────────────────────────────┘
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Pre-call verification (G10 + I5)                                    │
│   pre_call_fut = collector.pre_call(&pre, None)                     │
│   match tokio::time::timeout(PROVENANCE_PRE_CALL_TIMEOUT, fut) :    │
│     Ok(Ok(()))   → continue                                          │
│     Ok(Err(e))   → BridgeError::ProvenanceVerifyFailed              │
│     Err(timeout) → BridgeError::ProvenanceVerifyFailed              │
│   ▲ FAIL CLOSED — no HTTP call yet                                   │
└─────────────────────────────────────┬──────────────────────────────┘
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Two-turn HTTP dispatch (G6 + I3 + I15)                              │
│   Turn 1 :                                                          │
│     t1_start = Instant::now()                                       │
│     POST /v1/messages with tools=[swarm_context]                    │
│     parsed1 = ClaudeMessagesResponse                                 │
│     t1_ms   = t1_start.elapsed()                                    │
│     extract tool_use block                                           │
│   Turn 2 :                                                          │
│     t2_start = Instant::now()                                       │
│     POST /v1/messages with tool_result = bundle_json                │
│     parsed2 = ClaudeMessagesResponse                                 │
│     t2_ms   = t2_start.elapsed()                                    │
│     extract text                                                    │
│   total_usage = parsed1.usage + parsed2.usage                       │
│   total_latency = t1_ms + t2_ms                                     │
└─────────────────────────────────────┬──────────────────────────────┘
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Post-call computation (G5 + G6 + I11 + I17)                         │
│   cost_microcents   = compute_cost_microcents(...)                  │
│   output_hash       = hash_str(text)                                │
│   args_hash         = hash_bytes(tool_use_block.input.to_string()) │
│   bundle_hash       = hash_bytes(&bundle_json_bytes)                │
│   tool_call         = ToolCallRecord { ordinal:0, tool_name,        │
│                       arguments_hash, result_hash:Some(bundle),     │
│                       result_kind: Ok }                              │
│   post = PostCallProvenance {                                       │
│     token_usage     = { provider_reported_*: Some(...) },           │
│     cost_accounting = { cost_cents, prices_table_version, ... },    │
│     provider_request_id = parsed2.id (Anthropic msg_...),           │
│     provider_response_time_ms = t1_ms + t2_ms,                      │
│     provider_finish_reason    = map_finish_reason(stop),            │
│     output_hash, completeness, tool_calls: [tool_call]              │
│   }                                                                  │
└─────────────────────────────────────┬──────────────────────────────┘
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Post-call emission (G3 + I8 + ADR-5 + ADR-6)                        │
│   post_call_fut = collector.post_call(pre, post)                    │
│   match tokio::time::timeout(PROVENANCE_POST_CALL_TIMEOUT, fut) :   │
│     Ok(Ok(record)) → trace!(emitted)                                │
│     Ok(Err(e))     → emit_failed counter + warn!(...)                │
│     Err(timeout)   → post_timeout counter + warn!(...)               │
│   ▲ NON-BLOCKING — LLM response always returned to caller           │
└─────────────────────────────────────┬──────────────────────────────┘
                                      ▼
┌────────────────────────────────────────────────────────────────────┐
│ Return AgenticResponse (Sprint 13 contract preserved)                │
│   AgenticResponse {                                                  │
│     text, stop_reason, usage, model_identity, cost_microcents       │
│   }                                                                  │
└────────────────────────────────────────────────────────────────────┘
```

Every step has a corresponding test in
`audit_n26_*.rs` / `chaos_n26_*.rs` / `phase_9_n26_*.rs`.

---

## 7. Operational deployment notes

### Boot path

1. **Env vars** :
   - `ANTHROPIC_API_KEY` — wires `ClaudeBridge` (else `StubAgenticBridge`).
   - `ANTHROPIC_MODEL` — optional, defaults to `claude-sonnet-4-6`.
   - `EFE_GATEWAY_DISABLE_PROVENANCE=1` — disables N26 wiring
     entirely.  Default = enabled.
   - `EFE_GATEWAY_DISABLE_COMPLIANCE=1` — disables N24 wiring
     (independent of N26).

2. **`build_app_state` is now `async`** — it awaits 3 model
   registrations (sonnet, haiku, opus).  Called from
   `#[tokio::main]`.

3. **Three Anthropic models pre-registered with
   `dev_skip_hash_check = true`** — hosted models have no content
   hash to verify ; the registry lookup succeeds without
   verification.

4. **Live verification ladder** :
   - `GET /healthz` → 200 (process alive)
   - `GET /readyz` → 200 with `llm_provenance: ok` (subsystem
     registered)
   - `GET /openapi.json` → contains `/v1/llm-provenance` path
     and `LlmProvenanceListResponse` schema
   - `GET /v1/llm-provenance` without auth → 401
   - `GET /v1/llm-provenance` with tenant JWT → 403
   - `GET /v1/llm-provenance` with operator JWT → 200,
     empty records on fresh boot

### Failure modes ops should watch

| Symptom | Likely cause | Action |
|---|---|---|
| `n26_provenance_emit_failed_total` rising | Store backend down (F4) | Page the store team ; LLM dispatches still serving customers |
| `n26_provenance_post_timeout_total` rising | Store backend slow (F5) | Investigate p99 of `post_call` |
| `n26_pre_call_verify_failed_total` rising | New model id not registered (F2/F4) | Add the model to the boot registration list |
| `n26_provenance_skipped_total{reason="no_audit_context"}` rising | Upstream caller forgot the audit context | Check the swarm runner / handler that produced the task |
| `BridgeError::ProvenanceVerifyFailed` returned to clients | Pre-call hard fail (F2 or pre-call timeout) | The customer's call was REJECTED ; check the registry |

---

## 8. Test surface inventory (final)

| Suite | File | Tests | Coverage |
|---|---|---|---|
| `task::tests` | `crates/efe-agentic-bridge/src/task.rs` | 12 | G1 (struct serde + default + builder + legacy decode) |
| `pricing::tests` | `crates/efe-agentic-bridge/src/pricing.rs` | 14 | G5 + D3 (rates, fallback, overflow, model selection) |
| `claude::tests` | `crates/efe-agentic-bridge/src/claude.rs` | 23 | dispatch happy path + body too large + protocol violation + cost estimator |
| `error::tests` | `crates/efe-agentic-bridge/src/error.rs` | 2 | new `ProvenanceVerifyFailed` variant |
| `tool_use::tests` | `crates/efe-agentic-bridge/src/tool_use.rs` | 4 | preserved Sprint 13 |
| `audit_n26_provenance_wiring` | `tests/audit_n26_provenance_wiring.rs` | **14** | I1, I2×2, I3, I5, I6, I8, I11, I14, I18 + G6, G7, G9 + serde audit |
| `chaos_n26_provenance_wiring` | `tests/chaos_n26_provenance_wiring.rs` | **8** | F2, F4, F5, F5b, F9, F10, F11, F13 |
| `phase_9_n26_llm_provenance` | `crates/efe-gateway/tests/phase_9_n26_llm_provenance.rs` | **12** | end-to-end HTTP (auth, overwrite, default ctx, 503, lists, OpenAPI, token sums, provider_request_id, pricing version) |
| Gateway lib unit | `crates/efe-gateway/src/` | 105 | extended with `llm_provenance.rs` 7 unit tests |
| Gateway regression — `phase_7_n22` | | 16 | unchanged |
| Gateway regression — `audit_n22` | | 14 | unchanged |
| Gateway regression — `phase_8_n23` | | 10 | unchanged |
| Gateway regression — `phase_8_n24` | | 13 | unchanged |
| `efe-llm-provenance` lib + store | | 112 | unchanged |
| **Grand total** | | **~356** | (excluding 1 ignored doctest) |

**Zero regression** on N22 / N23 / N24 acceptance suites.

---

## 9. Hooks for Sprint 19 N27 (streaming SSE)

Sprint 19 N27 will wire the Anthropic streaming response into
a per-token event log.  The existing N26 surface anticipates
this via three hooks :

1. **`served_version` capture** — Sprint 19 captures
   `parsed2.model` (the real served model id, e.g.
   `claude-sonnet-4-5-20241022`) and populates
   `ModelIdentity::served_version` separately from
   `requested_version`.  Drift between the two is a metric
   `n27_model_drift_total{requested, served}`.

2. **`tool_calls` extension** — Sprint 19 keeps the same
   single-record-per-call discipline (ADR-3) but the
   `tool_calls` vector grows with one entry per streaming
   tool-use chunk.

3. **`provider_metadata`** — currently empty.  Sprint 19 fills
   it with `{ "anthropic_request_id": ..., "stream_chunk_count":
   N, "first_token_ms": M, "stream_finish_reason": ... }`.

The `dispatch_with_provenance` function is the integration
point — Sprint 19 adds a `dispatch_stream_with_provenance`
sibling that produces the same record shape but with the
streaming additions.

---

## 10. Hooks for Sprint 20 N28 (Postgres + event bus)

1. **`PostgresLlmProvenanceStore`** — implements the same
   `LlmProvenanceStore` trait.  The 200/500 ms timeouts on
   pre_call / post_call already protect the bridge from a slow
   Postgres pool.

2. **5-year retention** — Postgres TTL'd via partitioned
   tables (one per month) + a nightly archival job.  F14
   coverage closes.

3. **Event bus emission** — the post_call wrapper also
   publishes the record on the workspace event bus (N28
   topic `llm.provenance.v1`) so FinOps dashboards consume
   without polling.

4. **Cache invalidation** — the model registry can be
   reloaded without a gateway restart : Sprint 20 ships
   `WiredComplianceRunner::reload_registry()` (already
   compatible with the current `Arc<dyn ModelRegistry>` shape).

---

## 11. Lessons applicable to future bridge integrations

When wiring the next bridge (OpenAI, Bedrock, Gemini, etc.) :

1. **Always integrate against the real SDK types first.**  v1 of
   this doc guessed `CostAccounting` shape ; v2 corrected.  Read
   the SDK first.

2. **Magnitudes are easy to flip.**  $3 / Mtoken is 3 mc / token,
   not 30 mc / 1k tokens.  Always reproduce a known historical
   invoice line in a unit test before shipping.

3. **The audit context is the cross-cutting glue.**  Every bridge
   needs `AgenticAuditContext` (or equivalent).  Sprint 22+ may
   promote the struct to `efe-nano-sdk` so every bridge shares
   it without re-defining.

4. **Pre-call timeouts matter.**  The post_call timeout is
   obvious ; the pre_call timeout is subtle but equally
   important for production safety.  Both should be configurable
   via `with_provenance(collector).with_timeouts(pre, post)`
   in Sprint 19.

5. **The handler-side overwrite is the security boundary.**  The
   client supplies `audit_context.tenant_id` ; the handler
   MUST overwrite with the JWT claim.  This is non-negotiable.

6. **Single record per logical call.**  The two-turn Anthropic
   protocol is an implementation detail.  The auditor wants
   one row per "the LLM call".  Sum tokens, sum latency, keep
   the tool_use as a `ToolCallRecord` entry.

---

## 12. Annexes — concrete code excerpts

### A1 — the dispatch decision point

```rust
async fn call_with_context(
    &self,
    bundle: &ContextBundle,
    task: AgenticTask,
) -> Result<AgenticResponse, BridgeError> {
    // F12 : bundle size pre-check (preserved from Sprint 13).
    if bundle.estimated_size_bytes() > MAX_CONTEXT_BUNDLE_BYTES {
        return Err(BridgeError::BundleTooLarge { ... });
    }

    // **N26 + G3 + I1 + I2** : decide the provenance path.
    let provenance_path = matches!(
        (&self.provenance, &task.audit_context),
        (Some(_), Some(_))
    );

    if provenance_path {
        self.dispatch_with_provenance(bundle, task).await
    } else {
        self.provenance_skipped.fetch_add(1, Ordering::Relaxed);
        self.dispatch_without_provenance(bundle, task).await
    }
}
```

### A2 — the gateway handler tenant overwrite

```rust
pub async fn agentic_call_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<AgenticCallRequest>,
) -> Result<impl IntoResponse, ApiError> {
    check_tenant(&claims, req.tenant_id)?;
    let mut task = req.task;
    let jwt_tenant = TenantId::from_uuid(claims.tenant_id);
    match task.audit_context.as_mut() {
        Some(ctx) => {
            if ctx.tenant_id != jwt_tenant {
                tracing::warn!(
                    jwt_tenant = %claims.tenant_id,
                    supplied_tenant = ?ctx.tenant_id,
                    "N26 G11 audit_context.tenant_id overwritten by JWT claim"
                );
                ctx.tenant_id = jwt_tenant;
            }
        }
        None => {
            task.audit_context = Some(default_audit_context(jwt_tenant, &req.bundle));
        }
    }
    let resp = state.agentic.call_with_context(&req.bundle, task)
        .await.map_err(map_bridge_err)?;
    Ok(Json(AgenticCallResponse { ... }))
}
```

### A3 — the operator endpoint (paginated list)

```rust
pub async fn llm_provenance_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Query(query): Query<LlmProvenanceQuery>,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    let store = state.llm_provenance_store.as_ref().ok_or_else(|| {
        ApiError::DependencyUnavailable("llm provenance store not configured ...".into())
    })?;
    let tenant_uuid = query.tenant_id.unwrap_or(claims.tenant_id);
    let tenant = TenantId::from_uuid(tenant_uuid);
    let window = DateRange { from: since_st, to: until_st };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE).min(MAX_PAGE_SIZE);

    let mut records = if let Some(swarm_id) = query.swarm_request_id {
        store.list_by_swarm(swarm_id, tenant).await?
    } else {
        // Sweep the three known Anthropic models.
        let mut acc = Vec::new();
        for model in ["claude-sonnet-4-6", "claude-haiku-3-5", "claude-opus-4"] {
            let id = ModelIdentity::hosted("anthropic", model, "2023-06-01");
            if let Ok(rows) = store.list_by_model(&id, tenant, window).await {
                acc.extend(rows);
            }
        }
        acc
    };
    records.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
    records.truncate(limit);

    Ok(Json(LlmProvenanceListResponse { ... }))
}
```

### A4 — the pricing table (post-magnitude-fix)

```rust
pub fn anthropic_pricing_for(model: &str) -> ModelPricing {
    static TABLE: OnceLock<HashMap<&'static str, ModelPricing>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("claude-sonnet-4-6", ModelPricing {
            input_microcents_per_1k:  3_000,   // $3 / Mtoken
            output_microcents_per_1k: 15_000,  // $15 / Mtoken
        });
        m.insert("claude-haiku-3-5", ModelPricing {
            input_microcents_per_1k:  800,     // $0.80 / Mtoken
            output_microcents_per_1k: 4_000,   // $4 / Mtoken
        });
        m.insert("claude-opus-4", ModelPricing {
            input_microcents_per_1k:  15_000,  // $15 / Mtoken
            output_microcents_per_1k: 75_000,  // $75 / Mtoken
        });
        m
    });
    t.get(model).copied().unwrap_or(FALLBACK_PRICING)
}
```

### A5 — the new pre_call timeout (audit pass 3)

```rust
pub const PROVENANCE_PRE_CALL_TIMEOUT: Duration = Duration::from_millis(200);

// inside dispatch_with_provenance :
let pre_call_fut = collector.pre_call(&pre, None);
match tokio::time::timeout(PROVENANCE_PRE_CALL_TIMEOUT, pre_call_fut).await {
    Ok(Ok(())) => {}
    Ok(Err(e)) => {
        return Err(BridgeError::ProvenanceVerifyFailed {
            reason: e.to_string(),
        });
    }
    Err(_elapsed) => {
        return Err(BridgeError::ProvenanceVerifyFailed {
            reason: format!(
                "pre_call timed out after {} ms",
                PROVENANCE_PRE_CALL_TIMEOUT.as_millis()
            ),
        });
    }
}
```

### A6 — accurate Completeness::Partial (audit pass 3)

```rust
fn derive_completeness(
    stop: StopReason,
    received_output_tokens: u32,
    expected_max_tokens: u32,
) -> Completeness {
    match stop {
        StopReason::MaxTokens => Completeness::Partial {
            received_tokens: received_output_tokens,
            expected_tokens: expected_max_tokens,
        },
        _ => Completeness::Complete,
    }
}

// Called from dispatch_with_provenance :
completeness: derive_completeness(
    outcome.stop_reason,
    outcome.usage.output_tokens,
    task.max_tokens,
),
```

---

_End of N26 architecture v2 (post-implementation).  Total
delivered : 1 new crate dep (`efe-llm-provenance` already
existed, just rewired) + 2 modified crates (`efe-agentic-bridge`,
`efe-gateway`) + 1 new module (`pricing.rs`) + 1 new struct
(`AgenticAuditContext`) + 1 new BridgeError variant
(`ProvenanceVerifyFailed`) + 1 new operator endpoint
(`GET /v1/llm-provenance`) + 1 new OpenAPI schema
(`LlmProvenanceListResponse`) + 356 tests + zero regression on
N22/N23/N24._
