# EFE M50 Sprint 5 — Mode C LLM-Augmented Runtime Architecture

**Status**            : DRAFT v1
**Date drafted**      : 2026-05-18
**Owner**             : Senior Platform Engineer #2 (Mode C lead) + Rodrigue Pekar + Senior Platform Engineer #1
**Sprint**            : 5 of 14 (Phase 2, week 3-4)
**Dates target**      : Aug 26 - Sep 5, 2026 (10 working days)
**Predecessor**       : Sprint 4 — Mode B Local Model Runtime (closed Aug 22, 2026)
**Successor**         : Sprint 6 — Audit + Replay (starts Sep 10, 2026)
**Companion docs**    :
- [`EFE_M50_PHASE_2_ARCHITECTURE.md`](EFE_M50_PHASE_2_ARCHITECTURE.md) (parent, 3 507 lignes)
- [`EFE_M50_AGENTIC_ARCHITECTURE.md`](EFE_M50_AGENTIC_ARCHITECTURE.md) (master, 2 219 lignes)
- [`EFE_M50_PHASE_1_ARCHITECTURE.md`](EFE_M50_PHASE_1_ARCHITECTURE.md) (Phase 1, 2 068 lignes)
- [`EFE_M_NUMBER_REGISTRY.md`](EFE_M_NUMBER_REGISTRY.md) (M-number registry)
- [`../decisions/D-M50-INFRA-001.md`](../decisions/D-M50-INFRA-001.md) (Postgres+Redis decision)
- [`../decisions/D-M50-CRATES-001.md`](../decisions/D-M50-CRATES-001.md) (2-crate split)

> **Contrat additif strict (rappel)** : Sprint 5 ne modifie **aucun**
> fichier source des 33 crates existants **à l'exception** de
> `efe-metering/src/meter.rs` qui reçoit 3 nouveaux variants
> additifs (`NanoAgentCalls`, `NanoAgentLlmTokens`,
> `NanoAgentVmSeconds`) — extension additive non-cassante (cf
> décision G.8 de Phase 2 arch).  Toutes les autres modifications
> sont localisées dans `crates/efe-nano/` + `crates/efe-nano-sdk/`,
> en additif pur.  Les intégrations avec `efe-llm-router` (M21),
> `efe-pii` (M20), `efe-prompts` (M21), `efe-cache` (M27) se font
> **par consommation de leurs API publiques uniquement** — aucune
> modification.
>
> Vérification à chaque PR : les 174 tests PR #27 scope + 51 tests
> STABILITY v17 freeze + 386 tests M50 cumulés (Phase 1 + Sprint 4)
> restent verts.  En fin de Sprint 5 : cible 458+ tests M50, 225
> baseline non modifiés, CI gate enforced.

---

## Table des matières

0. Executive summary
1. Sprint 5 — scope précis (in / out / deferred)
2. Module hierarchy delta during Sprint 5
3. Complete type system (Sprint 5 surface)
4. LlmClient — trait + LlmRouterClient + MockLlmClient
5. OutputValidator — strict JSON Schema + retry semantics
6. BudgetEnforcer — trait + InMemoryBudgetEnforcer stub
7. PiiRedactor — adapter wrapping efe-pii::Redactor
8. LlmAugmentedRuntime — Mode C executor (sequence diagram)
9. MultiModeRuntime — Mode C extension
10. VmPool — Mode C (`VmKind::LlmCaller`) implementation
11. 5 starter Mode C agents — manifests + prompt templates
12. efe-metering Meter variants (additive M31 — decision G.8)
13. Sprint 5 — day-by-day plan (10 working days)
14. Performance contracts Sprint 5
15. Integration matrix avec crates EFE (M20, M21, M27, M28, M31)
16. CI / build integration Sprint 5
17. Sprint 5 → Sprint 6 gate criteria
18. Test catalogue Sprint 5 (~ 72 tests)
19. Risk register specific to Sprint 5
20. Annexe A — Complete LlmRouterClient implementation
21. Annexe B — OutputValidator strict + retry logic
22. Annexe C — BudgetEnforcer reservation pattern + race-safety
23. Annexe D — PII redaction walker + strategies
24. Annexe E — 5 starter agent specs + prompt templates
25. Annexe F — Meter variant addition diff to M31
26. Annexe G — Decision log

---

## 0. Executive summary

Sprint 5 ships the **third execution mode** of the EFE M50 platform :
Mode C — LLM-augmented operations.  By end of Sprint 5 (Sep 5, 2026),
customers can mix Mode A (pure Rust) + Mode B (local ML model) + Mode C
(LLM-augmented via M21 router) in a single swarm.

```
═══════════════════════════════════════════════════════════════════
  Sprint 5 Outputs (Aug 26 → Sep 5, 2026)
═══════════════════════════════════════════════════════════════════

  Mode C runtime            : LlmAugmentedRuntime wired to efe-llm-router (M21)
                              Strict JSON schema enforcement on LLM output
                              Single-attempt + retry-once on parse failure

  PII redaction             : PiiRedactor wrapping efe-pii::Redactor (M20)
                              Default-on for Mode C, configurable per agent

  Budget cap                : BudgetEnforcer trait + in-memory stub
                              pre_check / commit / release / snapshot
                              Real efe-billing (M31) wired Sprint 11

  5 Mode C starter agents   : summarize_strict, extract_structured,
                              classify_topic_llm, sentiment_nuanced,
                              qa_grounded
                              Total starter pack : 20 agents (10 A + 5 B + 5 C)

  Metering integration      : 3 new Meter variants (additive to M31)
                                NanoAgentCalls
                                NanoAgentLlmTokens
                                NanoAgentVmSeconds

  Pool LlmCaller            : StubVmPool already supports VmKind::LlmCaller ;
                              Sprint 5 wires it to Mode C executor

  Cumulative Rust LOC added : ~ 2 200 lines (cum. Sprint 4 + 5 : ~ 5 900)
  Cumulative test LOC       : ~ 800 lines (cum. Sprint 4 + 5 : ~ 2 700)
  Public API surface delta  : +6 types in efe-nano-sdk (Sprint 5 additions)

═══════════════════════════════════════════════════════════════════

  Performance contract (Sprint 5 exit gate) :
    Mode C throughput        : ≥ 10 calls/sec/node (bottleneck = LLM provider)
    Mode C latency p99       : ≤ 5 000 ms
    Mode C pool hit rate     : ≥ 99 % at moderate load (lightweight VMs)
    Budget pre-check         : ≤ 1 ms (in-memory stub)
    Output validation        : ≤ 5 ms (cached JSONSchema compile)
    PII redaction            : ≤ 50 ms per 10 KB text (delegates to M20)

  Compliance contract :
    PII redaction default    : pre-LLM redact via M20 (no PII to provider)
    Budget pre-check         : reservation before call, charge after
    Audit trace              : execution_mode="llm_augmented",
                                llm_tokens_used populated
    Per-agent provider list  : `providers_allow_list` in manifest

═══════════════════════════════════════════════════════════════════
```

### What Sprint 5 unlocks

After Sprint 5 close (Sep 5, 2026) :
- **Drug discovery memos** : pdf_extract → ner_clinical → summarize_strict
- **Risk assessments** : trade_parse → calculate_var → extract_structured (LLM-driven)
- **Legal Q&A** : doc_chunk → embed_text → qa_grounded (citations included)
- **Compliance flagging** : classify_topic_llm with PII redaction across PHI/PII
- **Multi-aspect sentiment** : sentiment_nuanced (positive/negative/neutral × aspect)

### What Sprint 5 does NOT ship

Explicitly deferred :
- Real `efe-billing` (M31) integration → Sprint 11
- Cosign signing of LLM responses → Sprint 12
- LLM provider preference per-tenant override (uses M21 default routing for now)
- Token-level streaming output (M50 returns final response, not streamed) → Phase 5
- Multi-turn LLM conversations (single-shot only) → Phase 5

### Inherited from Sprint 4 (preserved)

```
Phase 1 + Sprint 4 deliverables (already shipped) :
  - efe-nano-sdk : 73 unit tests + 7 doctests
  - efe-nano lib : 272 unit tests
  - efe-nano integration : 9 phase_1 + 25 phase_2_mode_b tests
  - M50 cumulative : 386 tests
  - Phase 1 starter pack : 10 Mode A agents
  - Phase 2 Sprint 4 starter pack : 5 Mode B agents
  - InProcessModelLoader + 5 builtin models
  - StubVmPool (Mode B + Mode C kinds, Mode C executes returns NotYetWired)
  - LocalModelRuntime + MultiModeRuntime (Mode A + B wired)
  - 174 PR #27 + 51 STABILITY v17 baseline preserved

Sprint 5 ADDS to this — never removes or modifies inherited modules.
```

---

## 1. Sprint 5 — scope précis

### 1.1 In scope (must ship)

| Capability | Owner | Deliverable file |
|------------|-------|------------------|
| `LlmClient` trait + abstractions | Senior #2 | `efe-nano/src/runtime/llm_client.rs` |
| `LlmRouterClient` (production impl wrapping M21) | Senior #2 | `efe-nano/src/runtime/llm_client.rs` |
| `MockLlmClient` (for tests) | Senior #2 | `efe-nano/src/runtime/llm_client.rs` |
| `OutputValidator` (strict JSON schema enforcement) | Senior #2 | `efe-nano/src/runtime/output_validator.rs` |
| `BudgetEnforcer` trait | Senior #1 | `efe-nano/src/budget/mod.rs` |
| `InMemoryBudgetEnforcer` stub | Senior #1 | `efe-nano/src/budget/stub.rs` |
| `PiiRedactor` (adapter on `efe-pii::Redactor`) | Senior #1 | `efe-nano/src/runtime/pii_redactor.rs` |
| `LlmAugmentedRuntime` (Mode C executor) | Senior #1 + #2 | `efe-nano/src/runtime/llm_augmented.rs` |
| Mode C wiring in `MultiModeRuntime` | Senior #1 + #2 | `efe-nano/src/runtime/multi_mode.rs` (extend) |
| Mode C support in `StubVmPool::execute` | Senior #1 | `efe-nano/src/runtime/pool/stub.rs` (extend) |
| `summarize_strict` Mode C agent | Founder | `efe-nano/src/starter_pack/summarize_strict.rs` |
| `extract_structured` Mode C agent | Founder | `efe-nano/src/starter_pack/extract_structured.rs` |
| `classify_topic_llm` Mode C agent | Founder | `efe-nano/src/starter_pack/classify_topic_llm.rs` |
| `sentiment_nuanced` Mode C agent | Founder | `efe-nano/src/starter_pack/sentiment_nuanced.rs` |
| `qa_grounded` Mode C agent | Founder | `efe-nano/src/starter_pack/qa_grounded.rs` |
| `register_all_starter_pack_with_mode_b_and_c` | Founder | `efe-nano/src/starter_pack/mod.rs` (extend) |
| 3 new `Meter` variants additive to M31 | Senior #1 | `efe-metering/src/meter.rs` (additive PR) |
| `ResponseFormat` enum (FreeText / JsonSchema) | Senior #2 | `efe-nano/src/runtime/llm_client.rs` |
| `PiiStrategy` enum + manifest field extension | Senior #1 | `efe-nano-sdk/src/types/compliance.rs` (extend) |
| ~ 72 new tests | All | tests/ + unit tests inline |

### 1.2 Out of scope (Sprint 6+)

| Capability | Reason out of Sprint 5 | Target |
|------------|-------------------------|--------|
| Real `efe-billing` (M31) integration | Stub satisfies trait ; full M31 integration is Phase 4 work | Sprint 11 |
| HMAC audit chain for Mode C traces | Audit chain extension is Sprint 6 deliverable | Sprint 6 |
| Cosign signing of LLM responses | Audit-grade tier hardening | Sprint 12 |
| Provider preference per-tenant | M21 already supports tenant config ; M50 stays default | Sprint 7+ |
| Token streaming back to caller | M21 doesn't stream ; would need new protocol | Phase 5 |
| Multi-turn conversations | Single-shot LLM call only ; multi-turn is conversational AI | Phase 5 |
| Real Firecracker microVM for LlmCaller pool | Stub pool sufficient for Phase 2 ; Sprint 8 swap-in | Sprint 8 |

### 1.3 Deferred but planned (within Sprint 5)

These are anticipated but final decisions are within Sprint 5 sprint :

- **OutputValidator retry strategy** : Sprint 5 Day 6 lands "retry once with stronger prompt" ; Sprint 5 Day 1-5 ships single-attempt only. The retry path is feature-flagged for the first half of sprint to keep risk contained.
- **PII redaction default policy** : Phase 2 ships `PiiStrategy::PreLlmRedact` as default ; customers can opt out per-agent via manifest field after explicit DPIA sign-off.
- **Budget cap defaults** : InMemoryBudgetEnforcer ships with `100 cents per tenant per call` cap as a safe default ; real per-tenant caps come from M31 in Sprint 11.

---

## 2. Module hierarchy delta during Sprint 5

State of `crates/efe-nano/src/` at end of each day :

### 2.1 End Sprint 4 (= start of Sprint 5)

Inherited from previous sprint (cf Phase 2 doc § 2.2-2.3) :

```
crates/efe-nano/src/
├── lib.rs
├── error.rs                      (ModelInferError, ModelLoaderError, VmPoolError, ModeNotWired)
├── audit/                        (Phase 1 stub, HMAC chain Sprint 6)
├── catalog/                      (InMemoryCatalog)
├── dispatcher/                   (DAG planner + executor with Arc<dyn NanoAgentExecutor>)
├── idempotency/                  (in-process TTL cache)
├── runtime/
│   ├── context.rs
│   ├── local_model.rs            (Mode B executor — Sprint 4)
│   ├── model_loader.rs           (5 builtin models — Sprint 4)
│   ├── multi_mode.rs             (Mode A + B dispatcher — Sprint 4)
│   ├── pool/
│   │   ├── mod.rs                (VmPool trait + VmKind)
│   │   └── stub.rs               (StubVmPool — Sprint 4)
│   ├── pure_rust.rs              (Mode A executor — Phase 1)
│   └── mod.rs                    (NanoAgentExecutor trait — Sprint 4)
└── starter_pack/
    ├── mod.rs                    (register_all_starter_pack + _with_mode_b)
    ├── (10 Mode A agents Phase 1)
    └── (5 Mode B agents Sprint 4)
```

### 2.2 End Sprint 5 (= end of week 4 = mid-Phase 2)

```
crates/efe-nano/src/
├── (everything from Sprint 4 unchanged)
├── budget/                                       ★ NEW Sprint 5
│   ├── mod.rs                                    (BudgetEnforcer trait + types)
│   └── stub.rs                                   (InMemoryBudgetEnforcer)
├── runtime/
│   ├── (Sprint 4 modules unchanged)
│   ├── llm_augmented.rs                          ★ NEW Sprint 5 (Mode C executor)
│   ├── llm_client.rs                             ★ NEW Sprint 5 (LlmClient trait + impls)
│   ├── output_validator.rs                       ★ NEW Sprint 5
│   ├── pii_redactor.rs                           ★ NEW Sprint 5
│   ├── multi_mode.rs                             (extended : adds Mode C)
│   └── pool/
│       └── stub.rs                               (extended : LlmCaller execute via LlmClient)
└── starter_pack/
    ├── mod.rs                                    (extended : register_all_..._and_c)
    ├── (15 Mode A + B agents from Phase 1 + Sprint 4)
    ├── summarize_strict.rs                       ★ NEW Sprint 5
    ├── extract_structured.rs                     ★ NEW Sprint 5
    ├── classify_topic_llm.rs                     ★ NEW Sprint 5
    ├── sentiment_nuanced.rs                      ★ NEW Sprint 5
    └── qa_grounded.rs                            ★ NEW Sprint 5
```

### 2.3 SDK additions (`crates/efe-nano-sdk/src/`)

```
crates/efe-nano-sdk/src/
├── (existing modules)
└── types/
    ├── compliance.rs                             (extended : add PiiStrategy enum)
    └── (other types unchanged)
```

The SDK gets :
- `PiiStrategy` enum on `ComplianceTags` (`None`, `PreLlmRedact`, `Strict`).
- Re-exports for `BudgetSnapshot`, `ReservationId` (Sprint 5 public types).

Cumulative file count at end of Sprint 5 : **~ 47 .rs files** in `efe-nano/` (35 from Sprint 4 + 12 new).
Cumulative starter pack count : **20 agents** (10 Mode A + 5 Mode B + 5 Mode C).

---

## 3. Complete type system (Sprint 5 surface)

Sprint 5 adds 6 new public types + extends one existing SDK type.

### 3.1 `LlmClient` trait + supporting types (Sprint 5)

```rust
// efe-nano/src/runtime/llm_client.rs

use async_trait::async_trait;
use efe_nano_sdk::TenantId;

/// Adapter trait over `efe_llm_router::LLMRouter::complete`.
///
/// We use a trait instead of using LLMRouter directly so that :
/// - We can mock the router in unit tests (MockLlmClient).
/// - We can swap the underlying impl in Phase 4 (per-tenant routing,
///   cost rebalancing, etc.) without touching every Mode C call site.
/// - The M50 contract evolves independently of M21's internal contract.
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    /// Render prompt + call LLM + return structured response.
    ///
    /// The implementation must :
    /// - Render `request.prompt_template_id` against `request.template_inputs`
    ///   via `efe-prompts::PromptRegistry`.
    /// - Invoke the underlying router with the rendered prompt.
    /// - Parse the response text into JSON if `response_format = JsonSchema`.
    /// - Map provider errors to `LlmCallError`.
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError>;

    /// Estimate cost in cents for budget pre-check.
    ///
    /// Estimation uses :
    /// - Input length / 4 ≈ token count.
    /// - max_tokens for output.
    /// - $0.01 per 1k input tokens + $0.03 per 1k output tokens (rough avg).
    /// This is over-estimated by design ; real cost from `response.cost_cents`
    /// is reconciled in `budget.commit`.
    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64;
}

/// One LLM call request at the M50 layer.
#[derive(Debug, Clone)]
pub struct LlmCallRequest {
    pub tenant_id: TenantId,
    pub prompt_template_id: String,
    pub template_inputs: serde_json::Value,
    pub max_tokens: u32,
    pub temperature: f32,
    pub providers_allow_list: Vec<String>,
    pub response_format: ResponseFormat,
    pub timeout_ms: u32,
}

/// Response format hint for the LLM call.
///
/// - `FreeText` : LLM produces plain text ; the caller (e.g. `qa_grounded`
///    agent) extracts JSON via regex if needed.
/// - `JsonSchema` : LLM is instructed to emit JSON conforming to the
///    schema ; the response is `serde_json::from_str`ed.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    FreeText,
    JsonSchema(serde_json::Value),
}

/// LLM call response at the M50 layer.
#[derive(Debug, Clone)]
pub struct LlmCallResponse {
    /// Already JSON-parsed if `response_format = JsonSchema` ; otherwise
    /// a `Value::String(...)`.
    pub content: serde_json::Value,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_cents: f64,
    pub provider_name: String,
    pub model_name: String,
    pub cache_hit: bool,
    pub duration_ms: u32,
}

/// Closed taxonomy of Mode C call errors.
#[derive(Debug, thiserror::Error)]
pub enum LlmCallError {
    #[error("provider error : {0}")]
    Provider(String),

    #[error("response is not valid JSON : {0}")]
    InvalidJson(String),

    #[error("response failed schema validation : {0}")]
    SchemaMismatch(String),

    #[error("budget insufficient : {remaining_cents} cents remaining")]
    BudgetInsufficient { remaining_cents: f64 },

    #[error("call timed out after {0} ms")]
    Timeout(u32),

    #[error("prompt template '{0}' not found in registry")]
    PromptNotFound(String),

    #[error("internal : {0}")]
    Internal(String),
}
```

### 3.2 `OutputValidator` (Sprint 5)

```rust
// efe-nano/src/runtime/output_validator.rs

use dashmap::DashMap;
use jsonschema::{Draft, JSONSchema};
use std::sync::Arc;

/// Strict JSON schema validator with **compile cache**.
///
/// Compiling a JSONSchema (draft 2020-12) costs ~ 100-500 μs.  At 10
/// cps/node sustained, that's significant overhead — so we cache compiled
/// schemas by a stable hash of the schema body.
pub struct OutputValidator {
    cache: DashMap<u64, Arc<JSONSchema>>,
    /// Retry policy when first parse fails.
    retry_policy: RetryPolicy,
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Max attempts (1 = no retry, ≥ 2 = retry).  Sprint 5 default 2.
    pub max_attempts: u8,
    /// If true, the retry uses a stronger prompt instruction
    /// "Reply ONLY with valid JSON matching the schema..."
    pub use_stronger_prompt_on_retry: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            use_stronger_prompt_on_retry: true,
        }
    }
}

impl OutputValidator {
    pub fn new() -> Self {
        Self::with_policy(RetryPolicy::default())
    }

    pub fn with_policy(retry_policy: RetryPolicy) -> Self {
        Self {
            cache: DashMap::new(),
            retry_policy,
        }
    }

    /// Validate a value against a schema. Compiled schema is cached on
    /// first miss.
    pub fn validate(
        &self,
        value: &serde_json::Value,
        schema: &serde_json::Value,
    ) -> Result<(), ValidationError> {
        let key = stable_hash(schema);
        let compiled = match self.cache.get(&key) {
            Some(c) => Arc::clone(&c),
            None => {
                let compiled = JSONSchema::options()
                    .with_draft(Draft::Draft202012)
                    .compile(schema)
                    .map(Arc::new)
                    .map_err(|e| ValidationError::SchemaCompile(e.to_string()))?;
                self.cache.insert(key, Arc::clone(&compiled));
                compiled
            }
        };
        if let Err(errors) = compiled.validate(value) {
            let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
            return Err(ValidationError::SchemaMismatch(msgs.join("; ")));
        }
        Ok(())
    }

    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }

    /// Pre-warm the cache by compiling N schemas up front.
    pub fn warm_with(&self, schemas: &[serde_json::Value]) -> Result<(), ValidationError> {
        for schema in schemas {
            // Compile + cache without validating any value.
            let key = stable_hash(schema);
            if !self.cache.contains_key(&key) {
                let compiled = JSONSchema::options()
                    .with_draft(Draft::Draft202012)
                    .compile(schema)
                    .map(Arc::new)
                    .map_err(|e| ValidationError::SchemaCompile(e.to_string()))?;
                self.cache.insert(key, compiled);
            }
        }
        Ok(())
    }
}

impl Default for OutputValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("schema compile failed : {0}")]
    SchemaCompile(String),
    #[error("output does not match schema : {0}")]
    SchemaMismatch(String),
}

fn stable_hash(value: &serde_json::Value) -> u64 {
    use std::hash::Hasher;
    let canonical = crate::idempotency::canonicalize_json(value);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(canonical.as_bytes());
    hasher.finish()
}
```

### 3.3 `BudgetEnforcer` trait + stub (Sprint 5)

```rust
// efe-nano/src/budget/mod.rs

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use efe_nano_sdk::TenantId;
use uuid::Uuid;

/// Per-tenant budget enforcement for Mode C (LLM) calls.
///
/// Sprint 5 ships [`stub::InMemoryBudgetEnforcer`].  Sprint 11 (Phase 4)
/// replaces it with the real `efe-billing` (M31) integration.
///
/// # Reservation lifecycle
///
/// 1. `pre_check(tenant, cents)` — reserves `cents` from the tenant's
///    cap, returns a `ReservationId`.
/// 2. The Mode C executor invokes the LLM.
/// 3. `commit(reservation, actual_cents)` — converts the reservation
///    into a final charge.  `actual_cents` is the real provider cost
///    (from `LlmCallResponse::cost_cents`).
/// 4. On error, `release(reservation)` — refunds the reservation
///    without charging.
///
/// All reservations expire after 60 s — orphaned reservations (caller
/// crash) auto-release.
#[async_trait]
pub trait BudgetEnforcer: Send + Sync + 'static {
    /// Reserve `cents` from the tenant's budget.
    async fn pre_check(
        &self,
        tenant: TenantId,
        cents: f64,
    ) -> Result<ReservationId, BudgetError>;

    /// Convert the reservation into a final charge.
    /// `actual_cents` may be ≤ reserved — the difference is refunded.
    /// If `actual_cents` > reserved, an over-budget warning is logged
    /// but the charge succeeds (provider already paid).
    async fn commit(
        &self,
        reservation: ReservationId,
        actual_cents: f64,
    ) -> Result<(), BudgetError>;

    /// Refund the reservation without charging.
    async fn release(&self, reservation: ReservationId) -> Result<(), BudgetError>;

    /// Current budget snapshot for a tenant.
    async fn snapshot(&self, tenant: TenantId) -> Result<BudgetSnapshot, BudgetError>;
}

/// Opaque reservation ID returned by `pre_check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub Uuid);

impl ReservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of a tenant's budget state.
#[derive(Debug, Clone)]
pub struct BudgetSnapshot {
    pub tenant: TenantId,
    /// Total budget for the current window (cents).
    pub cap_cents: f64,
    /// Already-committed spend in the window.
    pub used_cents: f64,
    /// Reserved (pre_check'd but not committed) cents.
    pub reserved_cents: f64,
    /// Available headroom = cap - used - reserved.
    pub remaining_cents: f64,
    /// When the budget window resets (typically start of next month).
    pub window_resets_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget insufficient : requested {requested_cents} cents, {remaining_cents} remaining")]
    Insufficient {
        requested_cents: f64,
        remaining_cents: f64,
    },
    #[error("reservation {0} not found or already committed/released")]
    UnknownReservation(Uuid),
    #[error("budget service internal error : {0}")]
    Internal(String),
}

pub mod stub;
pub use stub::InMemoryBudgetEnforcer;
```

### 3.4 `PiiRedactor` trait + adapter (Sprint 5)

```rust
// efe-nano/src/runtime/pii_redactor.rs

use async_trait::async_trait;
use efe_pii::redactor::Redactor as PiiRedactorTrait;
use std::sync::Arc;

/// Walk a JSON value tree and redact PII in every string field.
///
/// Phase 2 Sprint 5 strategy : every string value is run through
/// `efe-pii::Redactor::redact()` which returns the input with PII spans
/// replaced by tokens (e.g. `[EMAIL_1]`, `[PHONE_2]`).
///
/// Phase 4 will add :
/// - Per-field redaction policy (skip non-PII fields like "language" enum).
/// - Cross-field correlation (same email → same token across fields).
/// - Structured PII (e.g. names within nested objects).
pub struct PiiRedactor {
    inner: Arc<dyn PiiRedactorTrait + Send + Sync>,
}

impl PiiRedactor {
    pub fn new(inner: Arc<dyn PiiRedactorTrait + Send + Sync>) -> Self {
        Self { inner }
    }

    /// Redact PII from every string field in `value`.
    pub async fn redact_json(
        &self,
        value: serde_json::Value,
    ) -> Result<serde_json::Value, PiiError> {
        self.walk(value).await
    }

    fn walk(
        &self,
        v: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, PiiError>> + Send + '_>> {
        Box::pin(async move {
            match v {
                serde_json::Value::String(s) => {
                    let redacted = self.inner.redact(&s).await.map_err(|e| {
                        PiiError::RedactionFailed(format!("{e}"))
                    })?;
                    Ok(serde_json::Value::String(redacted))
                }
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        out.push(self.walk(v).await?);
                    }
                    Ok(serde_json::Value::Array(out))
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, v) in map {
                        out.insert(k, self.walk(v).await?);
                    }
                    Ok(serde_json::Value::Object(out))
                }
                primitive => Ok(primitive),
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PiiError {
    #[error("PII redaction failed : {0}")]
    RedactionFailed(String),
}
```

### 3.5 `PiiStrategy` extension to `ComplianceTags` (SDK)

```rust
// efe-nano-sdk/src/types/compliance.rs — Sprint 5 ADDITIVE diff

@@ existing ComplianceTags fields @@

+/// PII handling strategy for Mode C agents.
+#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
+#[serde(rename_all = "snake_case")]
+pub enum PiiStrategy {
+    /// No redaction (opt-out — requires explicit tenant DPIA sign-off).
+    None,
+
+    /// Pre-LLM redaction (Sprint 5 default for Mode C).
+    /// Input PII is replaced by tokens BEFORE the LLM call.
+    #[default]
+    PreLlmRedact,
+
+    /// Strict mode : pre-LLM redact + audit-trace redacted form +
+    /// reject any LLM response that contains the redaction tokens
+    /// (preventing accidental leakage).
+    Strict,
+}

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
  pub struct ComplianceTags {
      // ... existing fields ...
+
+     /// PII redaction strategy. Default `None` for Mode A/B, `PreLlmRedact`
+     /// is set explicitly by Mode C starter agent manifests.
+     #[serde(default)]
+     pub pii_redaction_strategy: PiiStrategy,
  }
```

This is additive : default value is `PiiStrategy::None`, so existing manifests are unchanged.

### 3.6 New `NanoAgentError` variants (Sprint 5)

```rust
// efe-nano/src/error.rs — Sprint 5 ADDITIVE diff

@@ existing NanoAgentError variants @@

+    /// Mode C LLM call error (Sprint 5).
+    #[error("LLM call error : {0}")]
+    LlmCall(#[from] crate::runtime::llm_client::LlmCallError),
+
+    /// Output validator error (Sprint 5).
+    #[error("output validation error : {0}")]
+    OutputValidation(#[from] crate::runtime::output_validator::ValidationError),
+
+    /// Budget enforcer error (Sprint 5).
+    #[error("budget error : {0}")]
+    Budget(#[from] crate::budget::BudgetError),
+
+    /// PII redaction error (Sprint 5).
+    #[error("PII error : {0}")]
+    Pii(#[from] crate::runtime::pii_redactor::PiiError),
```

With the corresponding `OperationErrorKind` mapping :

```rust
NanoAgentError::LlmCall(LlmCallError::BudgetInsufficient { .. })
    => OperationErrorKind::BudgetExceeded,
NanoAgentError::LlmCall(LlmCallError::Timeout(_))
    => OperationErrorKind::Timeout,
NanoAgentError::LlmCall(LlmCallError::SchemaMismatch(_))
    => OperationErrorKind::SchemaValidationFailed,
NanoAgentError::LlmCall(_) => OperationErrorKind::RuntimeError,

NanoAgentError::OutputValidation(_) => OperationErrorKind::SchemaValidationFailed,

NanoAgentError::Budget(BudgetError::Insufficient { .. })
    => OperationErrorKind::BudgetExceeded,
NanoAgentError::Budget(_) => OperationErrorKind::InternalError,

NanoAgentError::Pii(_) => OperationErrorKind::RuntimeError,
```

---

## 4. LlmClient — trait + LlmRouterClient + MockLlmClient

### 4.1 Design rationale

The `LlmClient` trait is a thin facade over `efe-llm-router::LLMRouter`.
Why a trait :

1. **Testability** : the dispatcher + Mode C executor need a way to test
   their logic without hitting a real LLM provider.  `MockLlmClient`
   returns canned responses or errors per call.
2. **Refactor isolation** : M21 may refactor its API in Phase 3 (add
   `prompt: String` field, deprecate `candidates`).  The trait shields
   M50 from M21 churn.
3. **Per-tenant routing flexibility** : Sprint 7+ may add per-tenant
   provider preferences directly at the M50 layer (without modifying
   M21).  A trait abstraction makes this trivial.

### 4.2 `LlmRouterClient` production implementation

The production impl wraps M21's `LLMRouter::complete` :

```rust
// efe-nano/src/runtime/llm_client.rs

use std::sync::Arc;

pub struct LlmRouterClient {
    router: Arc<efe_llm_router::router::LLMRouter>,
}

impl LlmRouterClient {
    pub fn new(router: Arc<efe_llm_router::router::LLMRouter>) -> Self {
        Self { router }
    }
}

#[async_trait]
impl LlmClient for LlmRouterClient {
    async fn call(
        &self,
        request: LlmCallRequest,
    ) -> Result<LlmCallResponse, LlmCallError> {
        let start = std::time::Instant::now();

        // Build M21's RouterRequest.
        // Note : M21 uses `sentence_redacted` field as the user prompt
        // body (legacy name from M21's taxonomy-classification origin).
        // For Mode C nano agents, we pack the rendered prompt body here.
        // The actual rendering happens via M21's PromptRegistry lookup of
        // `prompt_template_id` ; we pass template inputs through
        // `candidates` as JSON (per M21 convention) but the templates
        // know to ignore `candidates` for Mode C usage.
        let router_req = efe_llm_router::router::RouterRequest {
            tenant_id: request.tenant_id.to_string(),
            catalog: "m50_mode_c".to_string(),         // distinguishes M50 traffic in M21 logs
            bundle_version: "1.0".to_string(),
            sentence_redacted: serde_json::to_string(&request.template_inputs)
                .map_err(|e| LlmCallError::Internal(format!("inputs serialize : {e}")))?,
            candidates: vec![],
            prompt_template_id: request.prompt_template_id.clone(),
            max_tokens: request.max_tokens,
            timeout_ms: request.timeout_ms,
        };

        let router_resp = self.router.complete(router_req).await.map_err(|e| {
            // Map M21 error taxonomy to ours.
            map_router_error(e)
        })?;

        // Parse response if JsonSchema mode.
        let content = match &request.response_format {
            ResponseFormat::FreeText => serde_json::Value::String(router_resp.text.clone()),
            ResponseFormat::JsonSchema(_) => {
                parse_llm_json(&router_resp.text)
                    .map_err(|e| LlmCallError::InvalidJson(format!(
                        "{e}; raw text = {:?}", router_resp.text
                    )))?
            }
        };

        let duration_ms = start.elapsed().as_millis().min(u32::MAX as u128) as u32;
        Ok(LlmCallResponse {
            content,
            tokens_in: router_resp.tokens_in,
            tokens_out: router_resp.tokens_out,
            cost_cents: router_resp.cost_usd * 100.0,
            provider_name: router_resp.provider_name,
            model_name: router_resp.model_name,
            cache_hit: router_resp.cache_hit,
            duration_ms,
        })
    }

    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64 {
        // Rough heuristic. Calibrated against typical Mode C agent in
        // Sprint 5 Day 7 perf pass.
        let input_chars = serde_json::to_string(&request.template_inputs)
            .map(|s| s.len())
            .unwrap_or(500);
        let est_input_tokens = (input_chars / 4) as f64;
        let est_output_tokens = request.max_tokens as f64;
        // $0.01 per 1k input tokens + $0.03 per 1k output tokens → cents.
        let input_cents = est_input_tokens * 0.001;
        let output_cents = est_output_tokens * 0.003;
        input_cents + output_cents
    }
}

fn map_router_error(e: efe_llm_router::error::RouterError) -> LlmCallError {
    // Sprint 5 Day 1 lock : the exact M21 RouterError variants.
    // For now, treat all as Provider errors.
    LlmCallError::Provider(e.to_string())
}

/// Parse LLM text as JSON.  Handles markdown code fences and minor
/// formatting nits (LLMs love to wrap JSON in ```json ... ```).
fn parse_llm_json(raw: &str) -> Result<serde_json::Value, serde_json::Error> {
    let cleaned = strip_markdown_fences(raw.trim());
    serde_json::from_str(cleaned)
}

fn strip_markdown_fences(s: &str) -> &str {
    // Remove leading ```json or ``` and trailing ```.
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}
```

### 4.3 `MockLlmClient` for tests

```rust
// efe-nano/src/runtime/llm_client.rs (cont.)

use tokio::sync::RwLock;

/// Mock client for unit tests.  Returns canned responses keyed by
/// `prompt_template_id`.  Falls back to a generic stub if no canned
/// response is configured.
pub struct MockLlmClient {
    canned: RwLock<std::collections::HashMap<String, MockResponse>>,
    default_response: MockResponse,
    call_count: std::sync::atomic::AtomicU32,
}

#[derive(Debug, Clone)]
pub enum MockResponse {
    Ok(LlmCallResponse),
    Err(String),
}

impl MockLlmClient {
    pub fn new() -> Self {
        Self {
            canned: RwLock::new(std::collections::HashMap::new()),
            default_response: MockResponse::Ok(LlmCallResponse {
                content: serde_json::json!({"summary": "default mock"}),
                tokens_in: 10,
                tokens_out: 5,
                cost_cents: 0.1,
                provider_name: "mock".into(),
                model_name: "mock-v1".into(),
                cache_hit: false,
                duration_ms: 1,
            }),
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }

    pub async fn set_canned(&self, prompt_template_id: &str, response: MockResponse) {
        let mut guard = self.canned.write().await;
        guard.insert(prompt_template_id.to_string(), response);
    }

    pub fn set_default(&mut self, response: MockResponse) {
        self.default_response = response;
    }

    pub fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for MockLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn call(
        &self,
        request: LlmCallRequest,
    ) -> Result<LlmCallResponse, LlmCallError> {
        self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let canned = self.canned.read().await;
        let response = canned.get(&request.prompt_template_id)
            .cloned()
            .unwrap_or_else(|| self.default_response.clone());
        match response {
            MockResponse::Ok(r) => Ok(r),
            MockResponse::Err(msg) => Err(LlmCallError::Provider(msg)),
        }
    }

    fn estimate_cost_cents(&self, _: &LlmCallRequest) -> f64 {
        0.5    // fixed estimate for testing budget logic
    }
}
```

---

## 5. OutputValidator — strict JSON Schema + retry semantics

### 5.1 Validation flow

```
LLM response text
  ↓
parse_llm_json(text)   ← strip markdown fences, try serde_json::from_str
  ↓ Ok(Value)                          ↓ Err
  Validate against agent.output_schema   Retry once with stronger prompt
  ↓ Ok                                   ↓ Still Err
  Return Value                           Surface as OutputValidation::SchemaMismatch
```

### 5.2 Retry-once semantics (Sprint 5 Day 6)

Default `RetryPolicy { max_attempts: 2, use_stronger_prompt_on_retry: true }`.

The Mode C executor catches the **first** `LlmCallError::InvalidJson` or
`OutputValidation::SchemaMismatch`, then :

1. Re-calls the LLM with an augmented prompt template that prepends :

   ```
   IMPORTANT : Your previous response could not be parsed.  Reply ONLY
   with valid JSON conforming to the schema below.  Do not include any
   prose, markdown fences, or extra fields.
   <schema body>
   ```

2. If second attempt also fails, surface `SchemaValidationFailed` to the
   dispatcher (op → Failed, downstream → Skipped).

This single retry handles ~ 90 % of intermittent LLM output formatting
issues (per industry experience).  Sprint 7+ may add exponential
back-off + 3-attempt retry for Pro/Enterprise tiers.

### 5.3 Cache hit rate target

`OutputValidator` caches compiled `JSONSchema` by stable canonical hash.
Target hit rate : **99.99 %** in production (5 schemas × 5 starter agents
= 25 unique schemas — easily fits in cache).

---

## 6. BudgetEnforcer — trait + InMemoryBudgetEnforcer stub

### 6.1 `InMemoryBudgetEnforcer` Sprint 5 stub

```rust
// efe-nano/src/budget/stub.rs

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use efe_nano_sdk::TenantId;
use std::sync::Mutex;
use uuid::Uuid;

use super::{BudgetEnforcer, BudgetError, BudgetSnapshot, ReservationId};

const DEFAULT_CAP_CENTS: f64 = 100_000.0;            // $1 000 per tenant per window
const DEFAULT_WINDOW_DAYS: i64 = 30;
const RESERVATION_TTL_SEC: i64 = 60;

/// In-process budget enforcer.
///
/// Tracks per-tenant `(cap, used, reserved)` triple.  All state is
/// lost on process restart — Phase 4's real M31 integration replaces
/// this with persistent Postgres-backed counters.
pub struct InMemoryBudgetEnforcer {
    /// `tenant_id → TenantState`.  DashMap for lock-free reads.
    state: DashMap<TenantId, TenantState>,
    /// Currently-active reservations.
    reservations: DashMap<ReservationId, Reservation>,
    /// Default cap per new tenant.
    default_cap_cents: f64,
    /// Window duration in days.
    window_days: i64,
}

struct TenantState {
    cap_cents: f64,
    used_cents: Mutex<f64>,
    reserved_cents: Mutex<f64>,
    window_resets_at: Mutex<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct Reservation {
    tenant: TenantId,
    cents: f64,
    created_at: DateTime<Utc>,
}

impl InMemoryBudgetEnforcer {
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_CAP_CENTS)
    }

    pub fn with_cap(default_cap_cents: f64) -> Self {
        Self {
            state: DashMap::new(),
            reservations: DashMap::new(),
            default_cap_cents,
            window_days: DEFAULT_WINDOW_DAYS,
        }
    }

    fn ensure_state(&self, tenant: TenantId) {
        if !self.state.contains_key(&tenant) {
            self.state.insert(
                tenant,
                TenantState {
                    cap_cents: self.default_cap_cents,
                    used_cents: Mutex::new(0.0),
                    reserved_cents: Mutex::new(0.0),
                    window_resets_at: Mutex::new(
                        Utc::now() + Duration::days(self.window_days),
                    ),
                },
            );
        }
    }

    fn prune_expired_reservations(&self) {
        let cutoff = Utc::now() - Duration::seconds(RESERVATION_TTL_SEC);
        self.reservations.retain(|_, r| r.created_at > cutoff);
    }
}

impl Default for InMemoryBudgetEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BudgetEnforcer for InMemoryBudgetEnforcer {
    async fn pre_check(
        &self,
        tenant: TenantId,
        cents: f64,
    ) -> Result<ReservationId, BudgetError> {
        self.ensure_state(tenant);
        self.prune_expired_reservations();

        let state = self.state.get(&tenant).expect("ensure_state inserted");
        let mut used = state.used_cents.lock().unwrap();
        let mut reserved = state.reserved_cents.lock().unwrap();

        let remaining = state.cap_cents - *used - *reserved;
        if remaining < cents {
            return Err(BudgetError::Insufficient {
                requested_cents: cents,
                remaining_cents: remaining,
            });
        }
        *reserved += cents;
        let res_id = ReservationId::new();
        self.reservations.insert(
            res_id,
            Reservation {
                tenant,
                cents,
                created_at: Utc::now(),
            },
        );
        Ok(res_id)
    }

    async fn commit(
        &self,
        reservation: ReservationId,
        actual_cents: f64,
    ) -> Result<(), BudgetError> {
        let r = self
            .reservations
            .remove(&reservation)
            .ok_or_else(|| BudgetError::UnknownReservation(reservation.0))?
            .1;

        let state = self
            .state
            .get(&r.tenant)
            .ok_or_else(|| BudgetError::Internal("tenant state missing".into()))?;
        let mut used = state.used_cents.lock().unwrap();
        let mut reserved = state.reserved_cents.lock().unwrap();

        // Release the reservation.
        *reserved = (*reserved - r.cents).max(0.0);
        // Charge actual cost.
        *used += actual_cents;
        if actual_cents > r.cents {
            tracing::warn!(
                target: "efe_nano::budget",
                tenant = %r.tenant,
                reserved = r.cents,
                actual = actual_cents,
                "actual cost exceeded reservation"
            );
        }
        Ok(())
    }

    async fn release(&self, reservation: ReservationId) -> Result<(), BudgetError> {
        let r = self
            .reservations
            .remove(&reservation)
            .ok_or_else(|| BudgetError::UnknownReservation(reservation.0))?
            .1;

        let state = self
            .state
            .get(&r.tenant)
            .ok_or_else(|| BudgetError::Internal("tenant state missing".into()))?;
        let mut reserved = state.reserved_cents.lock().unwrap();
        *reserved = (*reserved - r.cents).max(0.0);
        Ok(())
    }

    async fn snapshot(&self, tenant: TenantId) -> Result<BudgetSnapshot, BudgetError> {
        self.ensure_state(tenant);
        let state = self.state.get(&tenant).expect("ensure_state inserted");
        let used = *state.used_cents.lock().unwrap();
        let reserved = *state.reserved_cents.lock().unwrap();
        let resets = *state.window_resets_at.lock().unwrap();
        Ok(BudgetSnapshot {
            tenant,
            cap_cents: state.cap_cents,
            used_cents: used,
            reserved_cents: reserved,
            remaining_cents: state.cap_cents - used - reserved,
            window_resets_at: resets,
        })
    }
}
```

### 6.2 Race-safety analysis

Two concurrent `pre_check(tenant_a, 50)` calls when remaining = 60 :

```
Thread T1                       Thread T2
─────────────────────           ─────────────────────
get state lock                  blocks
read remaining = 60
60 ≥ 50 → OK
reserved += 50
unlock
                                acquire state lock
                                read remaining = 10
                                10 < 50 → Err(Insufficient)
                                unlock
```

The `state.reserved_cents` Mutex inside DashMap provides per-tenant
serialization — two concurrent `pre_check` calls on the same tenant cannot
both pass the check.  Different tenants are independent (DashMap shards).

---

## 7. PiiRedactor — adapter wrapping efe-pii::Redactor

### 7.1 Integration with M20 (`efe-pii`)

`efe-pii` exposes the `Redactor` trait + `PresidioClient` implementation.
`PresidioClient` proxies to a Microsoft Presidio HTTP service that handles
NER + regex-based PII detection across :
- EMAIL_ADDRESS
- PHONE_NUMBER
- US_SSN, IBAN, CREDIT_CARD
- DATE_TIME
- PERSON (NER-based)
- LOCATION (NER-based)
- IP_ADDRESS
- MEDICAL_LICENSE
- US_DRIVER_LICENSE
- ... 30+ entity types

The `Redactor::redact(&str) -> String` returns the input with PII spans
replaced by tokens (e.g. `[EMAIL_1]`, `[PHONE_2]`).  Token preservation
across same-input runs is **not** guaranteed in Phase 2 Sprint 5 — that's
Sprint 12 work (cross-call PII correlation).

### 7.2 PiiRedactor walk algorithm

The redactor walks the JSON tree recursively :

- `Value::String(s)` → `Redactor::redact(s)` async call
- `Value::Array(arr)` → recurse on each item
- `Value::Object(map)` → recurse on each (key, value) ; **keys are NOT
  redacted** (they're field names, not PII)
- Primitives (`Null`, `Bool`, `Number`) → pass-through

### 7.3 Strategies (`PiiStrategy` enum)

| Strategy | When used | Behavior |
|----------|-----------|----------|
| `None` | Mode A (no PII concern) or opt-out with DPIA | No redaction |
| `PreLlmRedact` | **Default for Mode C** | Redact input BEFORE LLM call |
| `Strict` | High-risk Mode C (medical, financial) | Pre-LLM redact + reject LLM responses containing redaction tokens (anti-leakage check) |

### 7.4 Performance

PII redaction adds latency :
- Local regex-only redaction (without NER) : 1-5 ms per KB
- With Presidio NER : 50-200 ms per 10 KB

To meet the Sprint 5 throughput target (10 cps/node Mode C), we batch
short calls and cache common patterns at the Redactor level (M20 handles
this internally).

---

## 8. LlmAugmentedRuntime — Mode C executor (sequence diagram)

### 8.1 Top-level pipeline

```
Caller → MultiModeRuntime.execute_with_context(agent, input, ctx)
                       │
                       └── ExecutionMode::LlmAugmented path
                            │
                            ▼
                       LlmAugmentedRuntime.execute_with_context(agent, input, ctx)
                            │
                       ┌────┴───────────────────────────────────────────┐
                       │                                                │
                       │  1. Validate input vs agent.input_schema       │
                       │     (cached compile via OutputValidator)       │
                       │                                                │
                       │  2. Check ctx.deadline → early Timeout         │
                       │                                                │
                       │  3. PII redact :                               │
                       │     if compliance.pii_redaction_strategy       │
                       │        != PiiStrategy::None :                  │
                       │       redacted_input = pii_redactor.redact_json(input) │
                       │     else :                                     │
                       │       redacted_input = input.clone()           │
                       │                                                │
                       │  4. Estimate cost :                            │
                       │     est = max(                                 │
                       │       agent.cost_estimate_cents,               │
                       │       llm_client.estimate_cost_cents(req)      │
                       │     )                                          │
                       │                                                │
                       │  5. Budget pre-check :                         │
                       │     reservation = budget.pre_check(            │
                       │       ctx.tenant_id, est)                      │
                       │     → if Err(Insufficient) : early-return      │
                       │                                                │
                       │  6. Acquire LlmCaller VM slot :                │
                       │     lease = pool.acquire(VmKind::LlmCaller, …) │
                       │     → if Err(AcquireTimeout) : release         │
                       │       reservation + early-return               │
                       │                                                │
                       │  7. Build LlmCallRequest from                  │
                       │     agent.execution_mode.LlmAugmented +        │
                       │     redacted_input.                            │
                       │                                                │
                       │  8. lease.execute(vm_req) → vm_resp            │
                       │     (StubVmPool calls llm_client.call(req))    │
                       │                                                │
                       │  9. If response_format = JsonSchema :          │
                       │     output_validator.validate(                 │
                       │       vm_resp.output, agent.output_schema)     │
                       │     → on first failure, retry once with        │
                       │       stronger prompt (Day 6 feature)          │
                       │                                                │
                       │  10. If PiiStrategy::Strict :                  │
                       │      check vm_resp.output doesn't contain      │
                       │      any redaction tokens (anti-leakage)       │
                       │                                                │
                       │  11. Budget commit :                           │
                       │      budget.commit(reservation, vm_resp.cost)  │
                       │                                                │
                       │  12. Build NanoAgentResponse :                 │
                       │      - llm_tokens_used = tokens_in + tokens_out│
                       │      - cost_cents = actual provider cost       │
                       │      - latency_ms                              │
                       │                                                │
                       │  13. Record audit trace :                      │
                       │      execution_mode = "llm_augmented"          │
                       │      llm_tokens_used = Some(N)                 │
                       │                                                │
                       │  14. Emit metering events :                    │
                       │      Meter::NanoAgentCalls += 1                │
                       │      Meter::NanoAgentLlmTokens += N            │
                       │      Meter::NanoAgentVmSeconds += elapsed.as_secs() │
                       │                                                │
                       └────────────────────────────────────────────────┘
```

### 8.2 Error path — budget pre-check fails

```
Step 5 : budget.pre_check → Err(Insufficient { requested, remaining })
  ↓
Map to NanoAgentError::Budget(BudgetError::Insufficient {...})
  ↓
Map to OperationError { kind: BudgetExceeded, message: "..." }
  ↓
Dispatcher records op as Failed
  ↓
Downstream ops are Skipped via dependency propagation
```

### 8.3 Error path — output schema mismatch (with retry)

```
Step 9 : output_validator.validate → Err(SchemaMismatch)
  ↓
If retry_policy.max_attempts > 1 :
  Re-call LLM with stronger prompt (Day 6)
  ↓
  If second attempt validates → continue normally
  ↓
  If second attempt also fails → SchemaValidationFailed
```

### 8.4 Error path — VM crash / timeout

Same pattern as Mode B (Sprint 4 § 4.4) :

- `lease.execute → Err(VmPoolError::AcquireTimeout(N))` → `Timeout(N)`
  surface
- `lease.execute → Err(VmPoolError::VmCrashed)` → retry once on a fresh
  lease, then `ResourceExceeded` if persistent

---

## 9. MultiModeRuntime — Mode C extension

Sprint 4 shipped `MultiModeRuntime` with Mode A + Mode B only.  Sprint 5
extends it :

```rust
// efe-nano/src/runtime/multi_mode.rs — Sprint 5 ADDITIVE diff

pub struct MultiModeRuntime {
    mode_a: Arc<PureRustRuntime>,
    mode_b: Option<Arc<LocalModelRuntime>>,
+   mode_c: Option<Arc<LlmAugmentedRuntime>>,
}

impl MultiModeRuntime {
    pub fn mode_a_only(mode_a: Arc<PureRustRuntime>) -> Self {
        Self { mode_a, mode_b: None, mode_c: None }
    }

    pub fn mode_a_and_b(
        mode_a: Arc<PureRustRuntime>,
        mode_b: Arc<LocalModelRuntime>,
    ) -> Self {
        Self { mode_a, mode_b: Some(mode_b), mode_c: None }
    }

+   pub fn mode_a_b_c(
+       mode_a: Arc<PureRustRuntime>,
+       mode_b: Arc<LocalModelRuntime>,
+       mode_c: Arc<LlmAugmentedRuntime>,
+   ) -> Self {
+       Self {
+           mode_a,
+           mode_b: Some(mode_b),
+           mode_c: Some(mode_c),
+       }
+   }

    pub fn builder(mode_a: Arc<PureRustRuntime>) -> MultiModeBuilder { ... }
+
+   pub fn has_mode_c(&self) -> bool {
+       self.mode_c.is_some()
+   }
}

#[async_trait]
impl NanoAgentExecutor for MultiModeRuntime {
    async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        match &agent.execution_mode {
            ExecutionMode::PureRust { .. } => {
                self.mode_a.execute_with_context(agent, input, ctx).await
            }
            ExecutionMode::LocalModel { .. } => {
                let runtime = self.mode_b.as_ref().ok_or_else(|| {
                    NanoAgentError::ModeNotWired("local_model".to_string())
                })?;
                runtime.execute_with_context(agent, input, ctx).await
            }
-           ExecutionMode::LlmAugmented { .. } => {
-               Err(NanoAgentError::ModeNotWired("llm_augmented".to_string()))
-           }
+           ExecutionMode::LlmAugmented { .. } => {
+               let runtime = self.mode_c.as_ref().ok_or_else(|| {
+                   NanoAgentError::ModeNotWired("llm_augmented".to_string())
+               })?;
+               runtime.execute_with_context(agent, input, ctx).await
+           }
        }
    }
}

#[derive(Debug, Clone)]
pub struct MultiModeHealth {
    pub mode_a_alive: bool,
    pub mode_b_alive: bool,
-   pub mode_c_alive: bool,                  // always false in Sprint 4
+   pub mode_c_alive: bool,                  // set by mode_c.is_some()
}
```

Builder pattern extension :

```rust
pub struct MultiModeBuilder {
    mode_a: Arc<PureRustRuntime>,
    mode_b: Option<Arc<LocalModelRuntime>>,
+   mode_c: Option<Arc<LlmAugmentedRuntime>>,
}

impl MultiModeBuilder {
    pub fn with_mode_b(mut self, b: Arc<LocalModelRuntime>) -> Self {
        self.mode_b = Some(b); self
    }
+   pub fn with_mode_c(mut self, c: Arc<LlmAugmentedRuntime>) -> Self {
+       self.mode_c = Some(c); self
+   }
    pub fn build(self) -> MultiModeRuntime {
        MultiModeRuntime {
            mode_a: self.mode_a,
            mode_b: self.mode_b,
+           mode_c: self.mode_c,
        }
    }
}
```

---

## 10. VmPool — Mode C (`VmKind::LlmCaller`) implementation

Sprint 4 shipped `StubVmPool` with `VmKind::LlmCaller` permits +
acquire/release semantics, but `LlmCaller` executions returned
`VmPoolError::ProvisionFailed("Sprint 5 not yet wired")`.

Sprint 5 wires the actual execution path :

```rust
// efe-nano/src/runtime/pool/stub.rs — Sprint 5 ADDITIVE diff

pub struct StubVmPool {
    config: StubVmPoolConfig,
    permits: HashMap<VmKind, Arc<Semaphore>>,
    loader: Arc<dyn ModelLoader>,            // Mode B
+   llm_client: Option<Arc<dyn LlmClient>>,  // Mode C (Sprint 5)
    metrics: Arc<Mutex<Metrics>>,
}

impl StubVmPool {
    pub fn new(
        config: StubVmPoolConfig,
        loader: Arc<dyn ModelLoader>,
    ) -> Self {
        // ... existing Sprint 4 code ...
+       Self { ..., llm_client: None, ... }
    }

+   pub fn new_with_llm(
+       config: StubVmPoolConfig,
+       loader: Arc<dyn ModelLoader>,
+       llm_client: Arc<dyn LlmClient>,
+   ) -> Self {
+       let mut pool = Self::new(config, loader);
+       pool.llm_client = Some(llm_client);
+       pool
+   }
}

// Inside StubExecutor::execute :

VmKind::LlmCaller => {
-   Err(VmPoolError::ProvisionFailed(
-       "Mode C (LlmCaller) not yet wired in StubVmPool ; Sprint 5".into(),
-   ))
+   let client = self.llm_client.as_ref().ok_or_else(|| {
+       VmPoolError::ProvisionFailed(
+           "Mode C requested but no LlmClient wired".into(),
+       )
+   })?;
+   let llm_req = req.llm_call_request.ok_or_else(|| {
+       VmPoolError::Internal("LlmCaller request missing llm_call_request".into())
+   })?;
+   let fut = client.call(llm_req);
+   match tokio::time::timeout(timeout, fut).await {
+       Ok(Ok(resp)) => Ok(VmResponse {
+           output: resp.content,
+           cpu_ms_used: resp.duration_ms,
+           memory_mb_peak: 0,
+           llm_tokens_used: Some(resp.tokens_in + resp.tokens_out),
+           llm_cost_cents: Some(resp.cost_cents),
+       }),
+       Ok(Err(e)) => Err(VmPoolError::Internal(format!("LLM call : {e}"))),
+       Err(_) => Err(VmPoolError::AcquireTimeout(req.timeout_ms)),
+   }
}
```

`VmRequest` is extended :

```rust
pub struct VmRequest {
    // ... existing fields ...
+   /// Mode C only : the rendered LLM call request.
+   pub llm_call_request: Option<crate::runtime::llm_client::LlmCallRequest>,
}
```

`VmResponse` is extended :

```rust
pub struct VmResponse {
    pub output: serde_json::Value,
    pub cpu_ms_used: u32,
    pub memory_mb_peak: u32,
+   pub llm_tokens_used: Option<u32>,        // Mode C only
+   pub llm_cost_cents: Option<f64>,         // Mode C only
}
```

These additions are backward-compatible (`Option<...>` defaults to `None`
for Mode B).

---

## 11. 5 starter Mode C agents — manifests + prompt templates

### 11.1 `summarize_strict` agent

```rust
// efe-nano/src/starter_pack/summarize_strict.rs

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("summarize_strict").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "LLM summarization with cap on sentence count.".to_string(),
        author: Author::Efe,
        category: Category::Summarization,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            prompt_template_id: "m50.summarize_strict.v1".to_string(),
            max_tokens: 800,
            temperature: 0.3,
        },
        determinism: DeterminismLevel::Probabilistic,
        idempotency: IdempotencyPolicy::Manual { ttl_sec: 3_600 },
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text", "max_sentences"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 50_000 },
                "max_sentences": { "type": "integer", "minimum": 1, "maximum": 10 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["summary", "sentence_count"],
            "properties": {
                "summary": { "type": "string", "minLength": 1 },
                "sentence_count": { "type": "integer", "minimum": 1, "maximum": 10 }
            },
            "additionalProperties": false
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(800),
            network_egress_kb: 50,
        },
        cost_estimate_cents: 2.0,
        latency_p99_ms_target: 3_000,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr: true,
            pii_redaction_strategy: PiiStrategy::PreLlmRedact,
            ..ComplianceTags::default()
        },
        mcp_tool_metadata: None,
        created_at: Utc::now(),
    }
}
```

Prompt template (`m50.summarize_strict.v1` registered in M21
PromptRegistry — done in Sprint 5 Day 1 via a M21 publish PR) :

```
SYSTEM:
You are a precise summarization assistant.  Read the user's text and
produce a summary with EXACTLY {{max_sentences}} sentences.  Reply with
JSON conforming to the schema:
{
  "summary": "...",
  "sentence_count": <int>
}
Do not include any prose outside the JSON.  Do not use markdown fences.

USER:
{{template_inputs_json}}
```

### 11.2 `extract_structured` agent

```rust
NanoAgent {
    id: NanoAgentId::new("extract_structured").expect("id valid"),
    description: "LLM-based structured extraction with caller-defined fields.",
    category: Category::StructuredExtract,
    execution_mode: ExecutionMode::LlmAugmented {
        provider: "anthropic".to_string(),       // preferred for JSON mode
        model: "claude-sonnet-4-5".to_string(),
        prompt_template_id: "m50.extract_structured.v1".to_string(),
        max_tokens: 2_000,
        temperature: 0.1,
    },
    determinism: DeterminismLevel::Probabilistic,
    input_schema: json!({
        "type": "object",
        "required": ["text", "fields"],
        "properties": {
            "text": { "type": "string", "maxLength": 50_000 },
            "fields": {
                "type": "array",
                "minItems": 1,
                "maxItems": 20,
                "items": {
                    "type": "object",
                    "required": ["name", "type"],
                    "properties": {
                        "name": { "type": "string", "pattern": "^[a-z_][a-z0-9_]*$" },
                        "type": { "enum": ["string", "number", "boolean", "date"] },
                        "description": { "type": "string" }
                    }
                }
            }
        }
    }),
    output_schema: json!({
        "type": "object",
        "required": ["extracted"],
        "properties": {
            "extracted": { "type": "object" }
        }
    }),
    cost_estimate_cents: 2.5,
    latency_p99_ms_target: 5_000,
    ..
}
```

### 11.3 `classify_topic_llm` agent

```rust
NanoAgent {
    id: NanoAgentId::new("classify_topic_llm").expect("id valid"),
    description: "Free-form topic classification via LLM (no closed taxonomy).",
    category: Category::Classification,
    execution_mode: ExecutionMode::LlmAugmented {
        provider: "openai".to_string(),
        model: "gpt-5-mini".to_string(),     // cheaper for classification
        prompt_template_id: "m50.classify_topic.v1".to_string(),
        max_tokens: 200,
        temperature: 0.0,                     // deterministic-ish
    },
    determinism: DeterminismLevel::Probabilistic,
    input_schema: json!({
        "type": "object",
        "required": ["text"],
        "properties": {
            "text": { "type": "string", "minLength": 1, "maxLength": 10_000 }
        }
    }),
    output_schema: json!({
        "type": "object",
        "required": ["topic", "confidence"],
        "properties": {
            "topic": { "type": "string", "minLength": 1, "maxLength": 64 },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
        }
    }),
    cost_estimate_cents: 0.5,
    latency_p99_ms_target: 2_000,
    ..
}
```

### 11.4 `sentiment_nuanced` agent

```rust
NanoAgent {
    id: NanoAgentId::new("sentiment_nuanced").expect("id valid"),
    description: "Multi-aspect sentiment analysis (positive/negative/neutral per aspect).",
    category: Category::Classification,
    execution_mode: ExecutionMode::LlmAugmented {
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        prompt_template_id: "m50.sentiment_nuanced.v1".to_string(),
        max_tokens: 500,
        temperature: 0.2,
    },
    output_schema: json!({
        "type": "object",
        "required": ["overall_sentiment", "intensity", "aspects"],
        "properties": {
            "overall_sentiment": { "enum": ["positive", "neutral", "negative"] },
            "intensity": { "type": "number", "minimum": 0, "maximum": 1 },
            "aspects": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["aspect", "sentiment"],
                    "properties": {
                        "aspect": { "type": "string", "maxLength": 64 },
                        "sentiment": { "enum": ["positive", "neutral", "negative"] }
                    }
                }
            }
        }
    }),
    cost_estimate_cents: 1.0,
    latency_p99_ms_target: 2_500,
    ..
}
```

### 11.5 `qa_grounded` agent

```rust
NanoAgent {
    id: NanoAgentId::new("qa_grounded").expect("id valid"),
    description: "Answer a question given a context, citing source spans.",
    category: Category::StructuredExtract,
    execution_mode: ExecutionMode::LlmAugmented {
        provider: "anthropic".to_string(),
        model: "claude-opus-4-5".to_string(),    // best for grounded Q&A
        prompt_template_id: "m50.qa_grounded.v1".to_string(),
        max_tokens: 1_500,
        temperature: 0.0,
    },
    input_schema: json!({
        "type": "object",
        "required": ["question", "context"],
        "properties": {
            "question": { "type": "string", "minLength": 1, "maxLength": 1_000 },
            "context": { "type": "string", "minLength": 1, "maxLength": 50_000 }
        }
    }),
    output_schema: json!({
        "type": "object",
        "required": ["answer", "citations"],
        "properties": {
            "answer": { "type": "string", "minLength": 1 },
            "citations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["span_start", "span_end", "source"],
                    "properties": {
                        "span_start": { "type": "integer", "minimum": 0 },
                        "span_end": { "type": "integer", "minimum": 0 },
                        "source": { "type": "string" }
                    }
                }
            }
        }
    }),
    cost_estimate_cents: 3.0,
    latency_p99_ms_target: 4_000,
    ..
}
```

---

## 12. efe-metering Meter variants (additive M31 — decision G.8)

Sprint 5 introduces 3 new `Meter` variants via an additive PR to
`efe-metering` :

```rust
// crates/efe-metering/src/meter.rs — Sprint 5 ADDITIVE diff (decision G.8)

@@ existing Meter variants @@

  pub enum Meter {
      ClassifyRequests,
      AgentRunSteps,
      AgentRunSeconds,
      MultimodalVisionRequests,
      MultimodalAudioSeconds,
      WebhookDeliveries,
      BatchClassifiedRows,
      AuditExportGb,
+     // M50 Phase 2 Sprint 5 additions :
+     /// Total nano agent calls across Mode A + B + C.
+     NanoAgentCalls,
+     /// Tokens consumed by Mode C nano agents (token-cost billing).
+     NanoAgentLlmTokens,
+     /// Mode B/C VM wall-clock seconds (compute billing).
+     NanoAgentVmSeconds,
  }
```

Plus the corresponding `name()` arm + `all()` extension + `from_name()`
arm.  Pricing table in `efe-billing` gets 3 new rows :

```rust
// efe-billing pricing_plan additive rows (Sprint 5)

PricingPlan {
    meter: Meter::NanoAgentCalls,
    free_tier_quota: 1_000,
    cost_per_unit_cents: 0.01,
    overage_per_unit_cents: 0.05,
    plan: PricingTier::Developer,
}
// ... 3 plans × 3 meters = 9 new rows ...
```

This is the **only** modification to a pre-existing crate in Sprint 5
(decision G.8 in Phase 2 architecture).  Tests in
`efe-billing::pricing::tests::all_meters_priced` get 3 new rows.

The PR is :
1. Drafted in Sprint 5 Day 4 by Senior #1 (paired with M31 owner).
2. Reviewed in Sprint 5 Day 5.
3. Merged in Sprint 5 Day 6 (early-merge to unblock Mode C metering
   integration on Day 7-8).

---

## 13. Sprint 5 — day-by-day plan (10 working days)

### Sprint 5 entry conditions

```
[ ] Sprint 4 closed cleanly (Aug 22, 2026) :
    [✓] 386 M50 tests green
    [✓] Mode B throughput ≥ 50 cps on dev
[ ] efe-llm-router (M21) v1.x stable + tenant config available
[ ] efe-pii (M20) Redactor public API reviewed (Sprint 5 Day 0 pre-week)
[ ] efe-prompts (M21) PromptRegistry available for template lookup
[ ] efe-billing (M31) BudgetEnforcer trait surface signed off OR stub plan agreed (decision : stub for Sprint 5)
[ ] 5 prompt templates drafted in human-readable form (Sprint 5 Day 1 lock)
[ ] M31 owner agrees to additive Meter PR plan (Sprint 5 Day 4 merge)
```

### Day 1 (Mon Aug 26)
**Morning : Sprint 5 kickoff meeting (2 h)**.
- Confirm `LlmClient` trait surface (locked at lunch).
- Confirm PII redaction default strategy (`PreLlmRedact` for Mode C).
- Hand-off : Senior #2 leads `LlmClient` + `OutputValidator` ; Senior #1
  leads `BudgetEnforcer` + `PiiRedactor` ; Founder leads 5 starter agents.

**Afternoon** :

Senior #2 :
- Create `efe-nano/src/runtime/llm_client.rs`.
- Define `LlmClient` trait + `LlmCallRequest` + `LlmCallResponse` +
  `ResponseFormat` + `LlmCallError` types.
- Implement `MockLlmClient` skeleton (returns canned responses or default
  stub).
- 4 unit tests : trait shape compiles, mock canned response returned, mock
  fallback to default, call_count tracked.

Senior #1 :
- Create `efe-nano/src/budget/mod.rs` + `efe-nano/src/budget/stub.rs`.
- Define `BudgetEnforcer` trait + `ReservationId` + `BudgetSnapshot` +
  `BudgetError`.
- Implement `InMemoryBudgetEnforcer` with default 100k cents cap.
- 6 unit tests : `pre_check` success, `pre_check` insufficient,
  `commit` succeeds, `commit` over-budget logs warning, `release`,
  `snapshot`.

Founder :
- Create 5 stub manifest files for Mode C agents : `summarize_strict.rs`,
  `extract_structured.rs`, `classify_topic_llm.rs`,
  `sentiment_nuanced.rs`, `qa_grounded.rs`.
- Each ships only the `manifest()` function for now.
- 5 tests per agent : manifest is_classification/_extraction/_summarization,
  manifest_is_mode_c, manifest_pii_strategy, manifest_cost > 0,
  manifest_p99_target_set.

**End-of-day** : 3 PRs open, build green.

### Day 2 (Tue Aug 27)
Senior #2 :
- Implement `LlmRouterClient::call(request)` :
  - Build `RouterRequest` from `LlmCallRequest`.
  - Serialize `template_inputs` into `sentence_redacted`.
  - Call `router.complete(req)`.
  - Parse response into `Value` if `JsonSchema` (use `parse_llm_json` +
    `strip_markdown_fences`).
  - Map `RouterError` → `LlmCallError`.
- 5 unit tests with `MockLlmClient` + 2 integration tests with a real
  M21 router pointed at a sandbox provider.

Senior #1 :
- Create `efe-nano/src/runtime/pii_redactor.rs`.
- Implement `PiiRedactor::redact_json(value)` walking the JSON tree.
- Wrap `efe_pii::redactor::Redactor` (use `PresidioClient` for production
  ; mock impl for tests).
- 6 unit tests : string field redacted, array of strings, nested object,
  primitives pass through, empty object, very long string.

Founder :
- Pre-register 5 prompt templates in M21 PromptRegistry via a small PR :
  - `m50.summarize_strict.v1`
  - `m50.extract_structured.v1`
  - `m50.classify_topic.v1`
  - `m50.sentiment_nuanced.v1`
  - `m50.qa_grounded.v1`
- 5 template tests : each template renders without error given typical
  inputs.

### Day 3 (Wed Aug 28)
Senior #2 :
- Create `efe-nano/src/runtime/output_validator.rs`.
- Implement `OutputValidator::validate(value, schema)` with compile cache
  (DashMap).
- Add `RetryPolicy` struct + `warm_with` pre-compile method.
- 8 unit tests : valid JSON + matches schema, missing field rejected,
  enum mismatch, additionalProperties false rejected, deeply nested,
  cache hit on second validate, warm_with pre-compiles, error message
  includes schema path.

Senior #1 :
- Create `efe-nano/src/runtime/llm_augmented.rs`.
- Implement `LlmAugmentedRuntime::execute_with_context` (steps 1-12 of §
  8.1 ; retry logic in Day 6).
- 5 unit tests : end-to-end with mock LLM + mock budget + mock PII redactor,
  budget insufficient → BudgetExceeded, PII redact called when strategy
  != None, schema mismatch → SchemaValidationFailed, mode mismatch
  → Internal.

Founder :
- Wire 5 manifest files to the dispatcher via
  `register_all_starter_pack_with_mode_b_and_c` in `starter_pack/mod.rs`.
- 5 integration tests : each Mode C agent's manifest is registrable +
  retrievable from catalog.

### Day 4 (Thu Aug 29)
**Morning : All** — integration day.
- Wire `LlmAugmentedRuntime` into `MultiModeRuntime` (extend Sprint 4's
  `mode_a_and_b` → `mode_a_b_c`).
- Extend `StubVmPool` to handle `VmKind::LlmCaller` execution via the
  injected `LlmClient`.
- 4 integration tests : 1-op Mode C swarm end-to-end with MockLlmClient,
  3-op DAG mixing Mode A + B + C, Mode C only swarm, Mode A + C only swarm
  (no Mode B wired).

**Afternoon** :

Senior #1 paired with M31 owner :
- Draft additive PR to `efe-metering` adding 3 new `Meter` variants
  (decision G.8 of Phase 2).
- Add 3 rows to `efe-billing` pricing table.
- 2 tests in `efe-billing::pricing::tests::all_meters_priced`.

Senior #2 + Founder :
- Add metering integration to `LlmAugmentedRuntime::execute_with_context`
  step 14 :
  - `Meter::NanoAgentCalls += 1`
  - `Meter::NanoAgentLlmTokens += tokens_in + tokens_out`
  - `Meter::NanoAgentVmSeconds += elapsed.as_secs_f64()`
- 3 unit tests : metering called on Mode C success, NOT called on failure,
  correct counters incremented.

### Day 5 (Fri Aug 30)
**Mid-point checkpoint** :
- 5 Mode C agents wired end-to-end with MockLlmClient.
- ~ 30 new tests added cumulative Sprint 5.
- Demo to the team : `extract_structured` extracts 3 fields from a sample
  PDF text via Mode C in a swarm.

Senior #1 :
- Production-grade `BudgetEnforcer` :
  - Per-window reset (30 days default).
  - Expired-reservation pruning every minute.
  - Tracing logs on over-budget commits.
- 3 chaos tests : 1000 concurrent pre_checks on same tenant + commits,
  100 concurrent commits + releases, reservation expiration after 60 s.

Senior #2 :
- Production-grade `OutputValidator` :
  - Pre-warm cache with all 5 starter agent output schemas at
    dispatcher construction time.
  - p99 validate latency benchmark : target ≤ 5 ms (single-thread).
- 2 perf tests + 1 cache-hit-rate test.

Founder :
- Fill out 5 starter Mode C handlers (no per-agent handler logic — the
  `LlmAugmentedRuntime` does all the lifting — just polish manifest
  metadata, latency targets, cost estimates).

### Day 6 (Mon Sep 1)
Senior #2 :
- Implement **retry-once with stronger prompt** :
  - If first LLM call returns invalid JSON OR schema-mismatched JSON,
    rebuild the prompt with the schema body injected + retry once.
  - Track retry count in `LlmCallResponse` metadata.
- 4 tests : 1-attempt success, 2-attempt success (first malformed JSON),
  2-attempt success (first schema-mismatched), exhausted retries
  → SchemaValidationFailed.

Senior #1 :
- Cost cap reconciliation :
  - Pre-check uses `llm_client.estimate_cost_cents(request)` (over-estimate).
  - Post-charge uses `response.cost_cents` (actual provider cost).
  - If actual > reserved : log warning + still commit (provider paid).
- 5 tests : pre = post, actual < reserved → refund, actual > reserved
  → warn + commit, commit then double-commit (UnknownReservation error),
  release then commit (UnknownReservation error).

Founder :
- Edge case tests for each starter agent :
  - `summarize_strict` : LLM returns 5 sentences when `max_sentences = 3`
    → SchemaValidationFailed.
  - `extract_structured` : 0 fields requested → InputSchemaValidation.
  - `classify_topic_llm` : empty text → InputSchemaValidation.
  - `sentiment_nuanced` : LLM returns invalid enum → retry → success.
  - `qa_grounded` : citation spans out-of-bounds → SchemaValidationFailed.

### Day 7 (Tue Sep 2)
**Performance pass on Mode C path** :

Senior #2 + Senior #1 :
- Profile Mode C call (estimated breakdown) :
  - Input schema validate : ~ 1 ms
  - PII redact (10 KB text) : ~ 50-200 ms
  - Budget pre-check : ~ 0.1 ms (in-memory)
  - VM acquire : ~ 0.5 ms (warm pool)
  - LLM call : 800-3000 ms (provider-bound, dominant)
  - Output schema validate : ~ 1 ms (cached)
  - Budget commit : ~ 0.1 ms
  - Audit record : ~ 1 ms (in-memory)
  - Metering emit : ~ 0.1 ms
- Target : Mode C overhead (everything except LLM call) ≤ 250 ms p99.
- Stress test : 50 concurrent Mode C calls, measure p50/p99/p99.9 latency
  + throughput.
- Mode C throughput on dev (with mock LLM) : ≥ 100 cps/node (proves
  overhead is fast enough).
- Mode C throughput on staging (with real LLM) : ≥ 10 cps/node.

### Day 8 (Wed Sep 3)
**Hardening** :

Senior #2 :
- Cancellation : `tokio::time::timeout(ctx.deadline - now, fut)`
  wraps the LLM call.  Verify cancel propagates to the LLM HTTP
  client (M21 internals).
- 3 cancel tests : exact-budget cancel, 2x-budget cancel, very-short
  budget cancels before LLM call even starts.

Senior #1 :
- `PiiStrategy::Strict` mode : anti-leakage check on output.
  - If output contains `[EMAIL_n]`, `[PHONE_n]`, etc. tokens → reject
    with `PiiError::LeakDetected`.
- 4 tests : output clean → success, output contains EMAIL token → reject,
  multi-token leak → reject, strict mode disabled → no check.

Founder :
- Re-run all 5 starter agent tests against an LLM provider (sandbox
  account on Anthropic + OpenAI sandboxes).  Validate p99 latency
  targets : 3000/5000/2000/2500/4000 ms.

### Day 9 (Thu Sep 4)
**Coverage closure** :

Senior #1 + #2 + Founder :
- Run all Sprint 5 tests : 70+ new tests.
- Run all Phase 1 + Sprint 4 tests : 386 — must stay green.
- Run STABILITY v17 + workspace baseline : 51 + 2 178 preserved.
- Fix any flaky tests (target : 0 flakes in 10 runs).

Senior #1 :
- Documentation update : `crates/efe-nano/README.md` Sprint 5 section
  (Mode C overview + 5 starter agents + budget + PII).
- Update `EFE_M50_PHASE_2_ARCHITECTURE.md` § 5 (Mode C is now done).

### Day 10 (Fri Sep 5)
**Sprint 5 close** :
- 72+ new tests cumulative Sprint 5.
- Cumulative M50 test count : 386 (Sprint 4) + 72 (Sprint 5) = **458+**.
- Sprint 5 retro meeting (1 h Fri afternoon).
- Sprint 6 kickoff prep : Senior #1 takes the audit chain lead.

---

## 14. Performance contracts Sprint 5

### 14.1 Latency targets per Mode C call

| Phase | p50 | p99 |
|-------|-----|-----|
| Input schema validate (cached) | < 0.5 ms | < 2 ms |
| PII redact (typical 1 KB text) | < 10 ms | < 50 ms |
| PII redact (large 10 KB text) | < 50 ms | < 200 ms |
| Budget pre-check | < 0.1 ms | < 0.5 ms |
| VM acquire (LlmCaller, warm pool) | < 1 ms | < 5 ms |
| LLM call (provider-bound) | 800-1500 ms | 3000-5000 ms |
| Output schema validate (cached) | < 0.5 ms | < 2 ms |
| Budget commit | < 0.1 ms | < 0.5 ms |
| Audit record (stub) | < 0.5 ms | < 2 ms |
| Metering emit | < 0.1 ms | < 0.5 ms |
| **Total Mode C overhead** | < 60 ms | < 250 ms |
| **Total Mode C end-to-end** | < 1500 ms | < 5000 ms |

### 14.2 Throughput targets

| Metric | Sprint 5 dev (mock LLM) | Sprint 5 staging (real LLM) | Phase 4 target |
|--------|--------------------------|------------------------------|----------------|
| Mode C calls/sec/node | ≥ 100 | ≥ 10 | ≥ 50 |
| Pool hit rate (LlmCaller) | ≥ 99 % | ≥ 99 % | ≥ 99.5 % |
| Output validation cache hit | ≥ 99.99 % | ≥ 99.99 % | ≥ 99.99 % |
| Budget pre-check p99 latency | ≤ 0.5 ms | ≤ 0.5 ms | ≤ 0.1 ms (M31) |

### 14.3 Cost model per Mode C call

| Provider | Avg cost per call | Margin at $0.02/call (Pro tier) |
|----------|--------------------|----------------------------------|
| Anthropic Claude Sonnet 4.5 | $0.015 | 25 % |
| OpenAI GPT-5 | $0.020 | 0 % (break-even) |
| OpenAI GPT-5-mini | $0.005 | 75 % |
| Anthropic Claude Haiku | $0.003 | 85 % |

The dispatcher should default-route to cheaper providers when agent
manifest allows (configured per-tenant in M21 in Phase 4).

---

## 15. Integration matrix avec crates EFE existants

| Crate | Sprint 5 usage | Modifications required |
|-------|-----------------|-------------------------|
| `efe-nano-sdk` | Adds `PiiStrategy` enum on `ComplianceTags` ; additive | **AUCUNE** (additive enum + default `None`) |
| `efe-llm-router` (M21) | `LLMRouter::complete()` for Mode C | **AUCUNE** — wrapped in `LlmRouterClient` |
| `efe-prompts` (M21) | `PromptRegistry::lookup` for templates | **ADDS** 5 prompt templates (data, not code) |
| `efe-pii` (M20) | `Redactor::redact()` pre-LLM | **AUCUNE** — wrapped in `PiiRedactor` |
| `efe-cache` (M27) | Passthrough via M21's internal LLM cache | **AUCUNE** |
| `efe-events` (M28) | Emit `nano-agent-completed` event | **AUCUNE** |
| `efe-metering` (M31) | New 3 `Meter` variants | **ADDS** variants (decision G.8) |
| `efe-billing` (M31) | New 3 pricing rules (additive) | **ADDS** rows |
| `efe-curation` (M23) | NOT used in Sprint 5 (Sprint 6 only) | — |
| `efe-agent-runner` (M25) | NOT used in Sprint 5 (real Firecracker Sprint 8+) | — |
| `efe-meta-swarm` (M14, M25) | NOT used | — |

**Verdict additivité** : ✓ no existing source file modified except
`efe-metering` (additive Meter variants, documented in decision G.8).
The integration is **almost** byte-identical — the M31 modification is
the single documented exception.

---

## 16. CI / build integration Sprint 5

### 16.1 Workspace `Cargo.toml` deltas

```toml
# Sprint 5 ADDS to crates/efe-nano/Cargo.toml :

[dependencies]
# Sprint 5 (Mode C LLM-Augmented Runtime) :
efe-llm-router   = { path = "../efe-llm-router" }
efe-pii          = { path = "../efe-pii" }
efe-prompts      = { path = "../efe-prompts" }
efe-metering     = { path = "../efe-metering" }
```

No external (crates.io) dependencies are added in Sprint 5 — everything
goes through existing EFE modules.

### 16.2 CI matrix expansion

```yaml
# .github/workflows/m50-sprint-5.yml
name: M50 Sprint 5 CI

on:
  pull_request:
    paths:
      - 'crates/efe-nano/**'
      - 'crates/efe-nano-sdk/**'

jobs:
  build:
    runs-on: ubuntu-latest-large
    steps:
      - uses: actions/checkout@v4
      - run: cargo test -p efe-nano-sdk
      - run: cargo test -p efe-nano --lib
      - run: cargo test -p efe-nano --test phase_1_integration
      - run: cargo test -p efe-nano --test phase_2_mode_b
      - run: cargo test -p efe-nano --test phase_2_mode_c
        env:
          EFE_TEST_LLM_DISABLED: "1"        # CI uses MockLlmClient
          EFE_TEST_PII_DISABLED: "1"        # CI uses MockPiiRedactor (no Presidio)
      - run: cargo clippy -p efe-nano-sdk -p efe-nano --all-targets -- -D warnings
      - run: cargo test --workspace --exclude efe-bench-harness   # baseline preservation
```

### 16.3 Test naming convention

```
crates/efe-nano/tests/
├── phase_1_integration.rs        (existing, 9 tests, unchanged)
├── phase_2_mode_b.rs              (existing, 25 tests Sprint 4, unchanged)
├── phase_2_mode_c.rs              (NEW Sprint 5, ~ 20 tests)
└── (Sprint 6 adds phase_2_audit_chain.rs + phase_2_replay.rs)
```

Plus unit tests inline in each module.

### 16.4 Performance gate in CI

Sprint 5 Day 9 adds a CI performance gate :
- Mode A throughput < 1 000 cps → soft warn
- Mode B throughput < 50 cps → soft warn
- Mode C overhead p99 < 250 ms → soft warn (with mock LLM)
- Cumulative test runtime < 90 s → hard fail (regression guard)

Soft warnings become hard at Sprint 6 Day 10.

---

## 17. Sprint 5 → Sprint 6 gate criteria

```
[ ] Mode C runtime operational                              (functional)
[ ] All 5 starter Mode C agents registered + tested         (functional)
[ ] PII redaction default-on for Mode C                     (compliance)
[ ] Budget pre-check + commit + release working              (functional)
[ ] OutputValidator with retry-once shipped                 (correctness)
[ ] PiiStrategy::Strict anti-leakage check working           (compliance)
[ ] Mode C throughput ≥ 100 cps/node on dev (mock LLM)       (perf)
[ ] Mode C throughput ≥ 10 cps/node on staging (real LLM)    (perf)
[ ] Mode C overhead p99 ≤ 250 ms                            (perf)
[ ] LlmCaller pool hit rate ≥ 99 %                          (perf)
[ ] Output validation cache hit rate ≥ 99.99 %              (perf)
[ ] efe-metering Meter variants additive PR merged           (process)
[ ] efe-billing pricing rows added for 3 new meters          (process)
[ ] 174 PR #27 + 51 STABILITY + 386 (Phase1+Sprint4) green   (additivity)
[ ] cumulative M50 tests ≥ 458                              (coverage)
[ ] cargo clippy -- -D warnings : zero lint                  (hygiene)
[ ] cargo doc -p efe-nano --no-deps : zero warning           (hygiene)
[ ] No flaky tests across 10 CI runs                          (quality)
```

Gate is **opened** by the founder + Senior #1 + Senior #2 after all items
checked.  Sprint 6 Day 1 cannot start until gate is open.

---

## 18. Test catalogue Sprint 5 (~ 72 tests)

### 18.1 Unit tests (~ 52)

| File | Test count | Purpose |
|------|-----------|---------|
| `runtime/llm_client.rs::tests` | 9 | LlmClient trait, Mock fixture, error mapping, JSON parsing with markdown fences, cost estimate |
| `runtime/output_validator.rs::tests` | 10 | Strict validation, cache hit, additionalProperties, nested schema, warm_with, retry policy default, error path |
| `budget/mod.rs::tests` | 4 | Trait shape, ReservationId uniqueness, BudgetError display |
| `budget/stub.rs::tests` | 8 | pre_check / commit / release / snapshot, insufficient, multi-tenant isolation, expiration, double-commit |
| `runtime/pii_redactor.rs::tests` | 6 | String redacted, nested object, array, primitives pass through, empty object, very-long string |
| `runtime/llm_augmented.rs::tests` | 7 | E2E with mocks, budget insufficient → BudgetExceeded, PII redact called, schema mismatch → SchemaValidationFailed, mode mismatch → Internal, retry, strict-mode leak |
| `starter_pack/summarize_strict.rs::tests` | 1 | Manifest |
| `starter_pack/extract_structured.rs::tests` | 1 | Manifest |
| `starter_pack/classify_topic_llm.rs::tests` | 1 | Manifest |
| `starter_pack/sentiment_nuanced.rs::tests` | 1 | Manifest |
| `starter_pack/qa_grounded.rs::tests` | 1 | Manifest |
| `runtime/multi_mode.rs::tests` (extended) | 3 | mode_a_b_c constructor, with_mode_c builder, has_mode_c |

**Subtotal unit tests** : ~ 52

### 18.2 Integration tests (~ 20)

| File | Test count | Purpose |
|------|-----------|---------|
| `tests/phase_2_mode_c.rs` | 20 | E2E Mode C via dispatcher with MockLlmClient, mixed mode A+B+C DAG, budget exhausted DAG, PII redact pipeline, strict mode anti-leakage, retry success, retry exhausted, 20-op stress, concurrent tenants, cost aggregation, manifests registered, multi-mode health, builder parity Mode C, idempotent twin Mode C |

**Subtotal integration tests** : ~ 20

### 18.3 Cumulative count

```
Phase 1 inherited           : 277 tests
Sprint 4 (Mode B) inherited : + 109 tests = 386
Sprint 5 (Mode C)           : + 72 tests = 458 cumulative
```

Plus baseline preserved : 174 PR #27 + 51 STABILITY v17 = 225 baseline
tests untouched.

**Grand total at end of Sprint 5** : 458 + 225 = **683 tests across the
relevant scope**.

---

## 19. Risk register specific to Sprint 5

### 19.1 Critical risks (P0)

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **M21 router API breaks during Sprint 5** | Low | Critical | Pin M21 to Cargo.lock SHA at Sprint 5 Day 1 ; upstream changes blocked for sprint duration |
| **LLM provider returns garbled JSON** | High | Medium | OutputValidator retry-once with stronger prompt handles ~ 90 % ; remaining 10 % surface as SchemaValidationFailed (transparent failure mode) |
| **PII redaction false-negative on PHI** | Medium | Critical (HIPAA) | Use Presidio NER + regex (production-grade) ; never roll-our-own ; PiiStrategy::Strict reject on token leak |
| **Budget pre-check race condition** | Medium | High | Per-tenant Mutex inside DashMap ; chaos test 1k concurrent pre_checks ; tested Day 5 |
| **Cost estimate too low vs actual** | High | Medium | Over-estimate by 2x in stub ; commit() handles over-budget gracefully with warn log |

### 19.2 Major risks (P1)

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **M21 PromptRegistry doesn't have 5 Mode C templates** | High | High | Founder pre-registers 5 templates Day 2 ; if M21 doesn't have a publish API, escalate to M21 owner |
| **efe-pii (M20) Presidio service unavailable in CI** | High | Medium | MockPiiRedactor for CI ; integration test against M20 staging only |
| **Mode C throughput < 10 cps on staging** | Medium | High | Pool hit rate optimization + connection pooling at M21 layer ; if persistent, defer Pro tier to Sprint 7 |
| **5 prompt templates produce inconsistent outputs** | Medium | Medium | Sprint 5 Day 5 demos run all 5 templates against 3 sample inputs each ; flagged drift triggers Day 6 retry tuning |

### 19.3 Minor risks (P2)

| Risk | Mitigation |
|------|------------|
| Test fixtures contain accidental real-PII | Pre-commit hook scans for SSN/credit card patterns ; CI fails if detected |
| Mode C audit traces leak input bodies | AuditPolicy::redact_pii_only flag enforced per agent compliance |
| Mode C VM lease pool exhausted under burst | Default pool size 50 → 500 ; chaos test confirms scaling |
| Sprint 5 ships before retry-once tested in staging | Day 8 sandbox test mandatory |

### 19.4 Rollback plan

```
Mid-sprint Mode C unshippable :
  - Skip Mode C for Phase 2 GA, defer to Sprint 7
  - Sprint 5 reverts to "Phase 2 = Mode A + Mode B only"
  - Mode C re-attempted Sprint 7-8 after MCP bridge ships

Mid-sprint PII redactor unstable :
  - Disable Mode C agents that flag GDPR/HIPAA compliance
  - 3 of 5 starter agents remain (classify_topic_llm,
    sentiment_nuanced + qa_grounded) since they don't auto-flag PII

Mid-sprint OutputValidator retry unstable :
  - Disable retry, ship single-attempt only
  - Failed schema validations surface as SchemaValidationFailed
  - Customer impact : ~ 5-10 % more transparent failures (acceptable for Beta)
```

---

## 20. Annexe A — Complete LlmRouterClient implementation

(Code body already sketched in § 4.2 above ; full version in
`crates/efe-nano/src/runtime/llm_client.rs` post-Sprint 5).

Key design contracts :
- `LlmClient::call` is the single entry point.
- `parse_llm_json` strips markdown fences (`​```json`, `​```​`).
- `map_router_error` is the single conversion point from M21 errors.
- `MockLlmClient` is documented as test-only.
- Cost is in **cents** at the M50 layer ; M21 returns USD ; conversion happens here.

---

## 21. Annexe B — OutputValidator strict + retry logic

Pseudo-code for the retry mechanism (inside `LlmAugmentedRuntime::execute_with_context`) :

```rust
let mut attempt = 1;
let max_attempts = output_validator.retry_policy().max_attempts;

loop {
    let llm_response = llm_client.call(llm_req.clone()).await?;
    let content = llm_response.content;

    match output_validator.validate(&content, &agent.output_schema) {
        Ok(()) => return Ok(content),
        Err(ValidationError::SchemaMismatch(msg)) if attempt < max_attempts => {
            tracing::warn!(
                attempt = attempt,
                schema_mismatch = %msg,
                "LLM output failed schema, retrying with stronger prompt"
            );
            // Augment prompt with schema body for retry.
            llm_req.template_inputs = inject_schema_reminder(
                llm_req.template_inputs.clone(),
                &agent.output_schema,
            );
            attempt += 1;
            continue;
        }
        Err(e) => return Err(NanoAgentError::OutputValidation(e)),
    }
}
```

The `inject_schema_reminder` function adds the schema body to the template
inputs under a `_retry_reminder` key, which the prompt template renders
inline when present.

---

## 22. Annexe C — BudgetEnforcer reservation pattern + race-safety

(Already detailed in § 6.1-6.2).

Key invariants :
- Per-tenant `Mutex<f64>` protects `reserved` + `used` atomically.
- Reservation TTL 60 s — orphans auto-release.
- DashMap shards prevent inter-tenant contention.
- Multiple concurrent commits/releases on the same reservation : first
  wins, others get `UnknownReservation`.

---

## 23. Annexe D — PII redaction walker + strategies

(Already detailed in § 7).

`PiiRedactor::walk` is a tail-recursive Box::pin'd async function (Rust
doesn't yet support direct async recursion in trait methods).  Edge cases :

- **Empty string** : passed through (`Redactor::redact("")` returns `""`).
- **JSON keys** : never redacted (they're structural identifiers).
- **Nested arrays-of-objects-of-arrays** : full depth traversal, no
  depth limit (relying on `serde_json::Value` recursion depth = ~ 128).

---

## 24. Annexe E — 5 starter agent specs + prompt templates

### E.1 `summarize_strict`

(Spec already in § 11.1.)

Prompt template `m50.summarize_strict.v1` :

```
SYSTEM:
You are a precise summarization assistant.  Read the user's text and
produce a summary with EXACTLY {{max_sentences}} sentences.  Reply with
JSON conforming to this schema:
{
  "summary": "<your summary>",
  "sentence_count": <integer between 1 and 10>
}
Do not include any prose outside the JSON.  Do not use markdown code fences.
{{#if _retry_reminder}}
IMPORTANT — RETRY ATTEMPT : Your previous response was malformed.
Reply ONLY with valid JSON conforming to the schema above.
{{/if}}

USER:
Text to summarize:
{{text}}

Required sentence count: {{max_sentences}}
```

### E.2 `extract_structured`

(Spec already in § 11.2.)

Prompt template `m50.extract_structured.v1` :

```
SYSTEM:
You are a precise structured extraction assistant.  Read the user's text
and extract the requested fields.  Reply with JSON conforming to this
schema:
{
  "extracted": {
    "<field_name_1>": <field_value_typed>,
    ...
  }
}

If a field cannot be determined from the text, set its value to null.
Do not include any prose outside the JSON.

USER:
Text:
{{text}}

Fields to extract:
{{#each fields}}
- {{name}} ({{type}}){{#if description}} — {{description}}{{/if}}
{{/each}}
```

### E.3 `classify_topic_llm`

(Spec already in § 11.3.)

Prompt template `m50.classify_topic.v1` :

```
SYSTEM:
You are a topic classifier.  Read the user's text and assign a topic
(short free-form noun phrase, ≤ 64 chars) + confidence (0.0-1.0).
Reply ONLY with JSON :
{
  "topic": "<noun phrase>",
  "confidence": <0.0-1.0>
}

USER:
{{text}}
```

### E.4 `sentiment_nuanced`

(Spec already in § 11.4.)

Prompt template `m50.sentiment_nuanced.v1` :

```
SYSTEM:
You are a nuanced sentiment analyzer.  Read the user's text and produce :
- overall_sentiment : "positive" / "neutral" / "negative"
- intensity : 0.0 (no signal) - 1.0 (strong signal)
- aspects : list of (aspect, sentiment) pairs (max 10)

Reply ONLY with JSON matching this schema. Do not add prose.

USER:
{{text}}
```

### E.5 `qa_grounded`

(Spec already in § 11.5.)

Prompt template `m50.qa_grounded.v1` :

```
SYSTEM:
You are a grounded Q&A assistant.  Answer the user's question using ONLY
the provided context.  Every claim in your answer MUST cite a span in
the context (byte offset start/end).  Reply with JSON matching this
schema:
{
  "answer": "<answer text>",
  "citations": [
    { "span_start": <int>, "span_end": <int>, "source": "context" }
  ]
}

If the context doesn't contain the answer, set "answer" to "Cannot be
determined from the context." and "citations" to [].

USER:
Question:
{{question}}

Context:
{{context}}
```

---

## 25. Annexe F — Meter variant addition diff to M31

The exact PR that lands in Sprint 5 Day 4-6 :

```diff
--- a/crates/efe-metering/src/meter.rs
+++ b/crates/efe-metering/src/meter.rs
@@ -20,6 +20,12 @@ pub enum Meter {
     WebhookDeliveries,
     BatchClassifiedRows,
     AuditExportGb,
+    // M50 Sprint 5 — Mode C / Nano Agent metering :
+    /// Total nano agent calls across Mode A + B + C.
+    NanoAgentCalls,
+    /// Tokens consumed by Mode C nano agents (token-cost billing).
+    NanoAgentLlmTokens,
+    /// Mode B/C VM wall-clock seconds (compute billing).
+    NanoAgentVmSeconds,
 }

 impl Meter {
@@ -52,6 +58,9 @@ impl Meter {
             Self::WebhookDeliveries => "webhook_deliveries",
             Self::BatchClassifiedRows => "batch_classified_rows",
             Self::AuditExportGb => "audit_export_gb",
+            Self::NanoAgentCalls => "nano_agent_calls",
+            Self::NanoAgentLlmTokens => "nano_agent_llm_tokens",
+            Self::NanoAgentVmSeconds => "nano_agent_vm_seconds",
         }
     }

@@ -67,6 +76,9 @@ impl Meter {
             Meter::WebhookDeliveries,
             Meter::BatchClassifiedRows,
             Meter::AuditExportGb,
+            Meter::NanoAgentCalls,
+            Meter::NanoAgentLlmTokens,
+            Meter::NanoAgentVmSeconds,
         ]
     }

@@ -83,6 +95,9 @@ impl Meter {
             "webhook_deliveries" => Some(Self::WebhookDeliveries),
             "batch_classified_rows" => Some(Self::BatchClassifiedRows),
             "audit_export_gb" => Some(Self::AuditExportGb),
+            "nano_agent_calls" => Some(Self::NanoAgentCalls),
+            "nano_agent_llm_tokens" => Some(Self::NanoAgentLlmTokens),
+            "nano_agent_vm_seconds" => Some(Self::NanoAgentVmSeconds),
             _ => None,
         }
     }
```

Plus pricing rows in `efe-billing` (separate file).

The PR includes :
- 1 file change in `efe-metering`
- 1 file change in `efe-billing` (pricing table)
- 1 test addition in `efe-billing::pricing::tests::all_meters_priced`
- 0 file changes in `efe-server`, `efe-saga`, or any other crate (the
  additive enum variants don't require any consumer to update — they
  use the variants additively).

---

## 26. Annexe G — Decision log

### G.1 LlmClient as a trait abstraction over efe-llm-router

**Decision** : Ship `LlmClient` trait + `LlmRouterClient` (prod) +
`MockLlmClient` (test) rather than calling `LLMRouter::complete` directly.

**Rationale** :
- Testability : MockLlmClient is invaluable for unit tests + CI
  (avoiding LLM API costs + flakiness).
- Refactor isolation : M21 may add per-tenant routing knobs in Phase 3 ;
  trait abstraction keeps M50 stable.
- Future per-call cost control : trait can dispatch to multiple providers
  based on M50-side policy in Phase 4.

**Trade-off** : One more indirection layer. Negligible runtime cost
(trait method dispatch ≤ 1 ns).

### G.2 OutputValidator retry-once with stronger prompt

**Decision** : Default retry policy is `max_attempts = 2,
use_stronger_prompt_on_retry = true`.

**Rationale** :
- LLMs produce malformed JSON ~ 1-5 % of the time even with explicit
  instructions.
- Single retry with schema reinforcement handles ~ 90 % of those.
- 2 attempts is the right balance : higher (3-5) risks budget blowout
  for chronically failing inputs.

**Trade-off** : Doubled LLM cost for ~ 1-5 % of calls. Acceptable.

### G.3 BudgetEnforcer stub in Sprint 5

**Decision** : Ship `InMemoryBudgetEnforcer` in Sprint 5 ; real `efe-billing`
(M31) integration deferred to Sprint 11 (Phase 4).

**Rationale** :
- M31 in early Sprint 5 is itself in flux.
- Stub satisfies the trait contract + gives realistic tests.
- Coupling Mode C ship to M31 GA blocks Phase 2 GA on someone else's
  timeline.

**Trade-off** : Sprint 5 ships without real budget enforcement. Default
permissive ($1 000 / tenant / month cap). Documented as Beta-grade.

### G.4 PII redaction default-on for Mode C

**Decision** : `PiiStrategy::PreLlmRedact` is the default for Mode C
agents ; opt-out requires explicit `PiiStrategy::None` in manifest.

**Rationale** :
- LLM provider exposure is the highest compliance risk (data leaves the
  EFE perimeter).
- Pre-LLM redaction eliminates this risk entirely.
- Default-on prevents accidental exposure when authoring new Mode C
  agents.

**Trade-off** : Semantic signal loss for some agents (e.g. NER would lose
the entity itself). Accepted because Mode B is the appropriate tool for
NER, not Mode C.

### G.5 Meter variants as additive M31 enum extension

**Decision** : Add 3 new variants (`NanoAgentCalls`, `NanoAgentLlmTokens`,
`NanoAgentVmSeconds`) to `efe-metering::Meter` via additive PR.

**Rationale** :
- `Meter` is a closed enum (Prometheus cardinality budget).
- M50 billing requires explicit variants — no extensibility point exists.
- Adding enum variants is the standard Rust pattern (non-breaking for
  consumers using catch-all `_` match arms ; breaking only for exhaustive
  matches, which Rust catches at compile time).
- Decision pre-approved by M31 owner + Senior #1 in Phase 2 architecture
  (decision G.8).

**Trade-off** : This is the **only** modification to a pre-existing crate
in Sprint 5. Documented prominently + tested via
`efe-billing::pricing::tests::all_meters_priced`.

### G.6 Strict response_format JSON Schema vs lenient parse

**Decision** : Strict parse + retry once + fail.

**Rationale** :
- Customers depend on `output_schema` contract.
- Lenient parsing risks silent data corruption.
- Failure mode is transparent (`SchemaValidationFailed` is a public
  error code customers can handle).

**Trade-off** : 1-2 % of calls fail where lenient might have succeeded.
Acceptable for Beta ; Pro+ may add "lenient mode" flag in Sprint 13+.

### G.7 5 starter agents on 5 different prompt templates

**Decision** : Ship 5 distinct Mode C starter agents (summarize_strict,
extract_structured, classify_topic_llm, sentiment_nuanced, qa_grounded)
each with its own prompt template.

**Rationale** :
- These 5 patterns cover ~ 80 % of customer LLM use cases (per industry
  signal).
- Each pattern has a different output schema, exercising the validator.
- 5 patterns give us a realistic measure of throughput + latency
  variance.

**Trade-off** : 5 prompt templates to maintain. Centralized in M21
PromptRegistry ; version-pinned.

### G.8 Strict mode anti-leakage check

**Decision** : `PiiStrategy::Strict` rejects LLM responses containing
redaction tokens (`[EMAIL_n]`, etc.).

**Rationale** :
- Defense-in-depth : even with pre-LLM redaction, an LLM could
  accidentally echo or hallucinate a token in the response.
- Strict mode is opt-in (high-risk agents : clinical, financial).
- Rejection surfaces as `Pii(LeakDetected)` → `RuntimeError` → caller
  can retry without strict mode if appropriate.

**Trade-off** : Strict mode has ~ 0.5 % false-positive rate (legitimate
tokens that happen to match the redaction pattern). Acceptable for
audit-grade customers.

### G.9 Mode C uses StubVmPool LlmCaller kind (not real Firecracker)

**Decision** : Phase 2 ships `StubVmPool` for both Mode B and Mode C.
Real Firecracker integration via `efe-agent-runner` lands Sprint 8.

**Rationale** : Same as Sprint 4 — Firecracker integration is itself a
multi-week project ; bundling it with Mode C is risky. The trait surface
is stable.

**Trade-off** : Mode C runs in-process without microVM isolation in
Phase 2. Documented as Beta-grade ; real isolation lands Sprint 8.

---

## 27. References & cross-links

**Predecessor docs** :
- `EFE_M50_PHASE_2_ARCHITECTURE.md` — parent architecture
- `EFE_M50_PHASE_1_ARCHITECTURE.md` — Phase 1 (Sprints 1-3)
- `EFE_M50_AGENTIC_ARCHITECTURE.md` — master, all phases

**External references** :
- Anthropic Claude API : <https://docs.anthropic.com/en/api/messages>
- OpenAI Chat API : <https://platform.openai.com/docs/api-reference/chat>
- Microsoft Presidio : <https://microsoft.github.io/presidio/>
- JSON Schema Draft 2020-12 : <https://json-schema.org/draft/2020-12/>
- EU AI Act Annex III (high-risk AI) : <https://artificialintelligenceact.eu/>
- GDPR Art. 5(1)(c) (data minimisation) : <https://gdpr-info.eu/art-5-gdpr/>

**Decisions log** (to be added during Sprint 5 kickoff) :
- `D-M50-SPRINT5-001.md` — LlmClient trait abstraction (G.1)
- `D-M50-SPRINT5-002.md` — OutputValidator retry-once (G.2)
- `D-M50-SPRINT5-003.md` — BudgetEnforcer stub strategy (G.3)
- `D-M50-SPRINT5-004.md` — PII redaction default-on (G.4)
- `D-M50-SPRINT5-005.md` — Meter additive PR (G.5)
- `D-M50-SPRINT5-006.md` — Strict JSON schema (G.6)
- `D-M50-SPRINT5-007.md` — 5 starter agents on 5 templates (G.7)
- `D-M50-SPRINT5-008.md` — Strict mode anti-leakage (G.8)
- `D-M50-SPRINT5-009.md` — StubVmPool for LlmCaller (G.9)

---

## 28. Sprint 5 risk matrix visual

```
                              Probability →
                              Low      Medium       High
              ────────────────────────────────────────────────
              Critical        M21 API  Budget race  PII NER
                              break    (mitigated   false-neg
                                       per-tenant   (Presidio
                                       Mutex)       industry)
              ────────────────────────────────────────────────
   Impact ↓   Major           Mode C   M21 templates Garbled
                              < 10 cps missing      JSON
                              (M21     (founder    (retry-once
                              tune)    pre-publishes)handles)
              ────────────────────────────────────────────────
              Minor           CI flake Test fixture Strict mode
                              (CI gate accidental   false-pos
                              retry)   PII (pre-    (~ 0.5 %)
                                       commit scan)
              ────────────────────────────────────────────────
```

Red items (Critical+High) require **daily founder check-in** during
Sprint 5. Yellow items (Major) reviewed at sprint mid-point. Green items
addressed in retro.

---

## 29. Glossary (Sprint 5 additions)

| Term | Definition |
|------|------------|
| **Mode C** | LLM-augmented runtime — Sprint 5 deliverable. |
| **LlmClient** | M50-side trait abstracting `efe-llm-router::LLMRouter::complete`. |
| **LlmRouterClient** | Production impl of `LlmClient` wrapping M21. |
| **MockLlmClient** | Test-only impl returning canned responses. |
| **ResponseFormat** | Either `FreeText` or `JsonSchema(value)` — controls how the response text is parsed. |
| **OutputValidator** | Strict JSON-schema enforcer with compile cache + retry policy. |
| **RetryPolicy** | `(max_attempts, use_stronger_prompt_on_retry)` config for OutputValidator. |
| **BudgetEnforcer** | Per-tenant pre_check / commit / release / snapshot for Mode C cost cap. |
| **ReservationId** | Opaque UUID returned by `pre_check`, consumed by `commit` or `release`. |
| **BudgetSnapshot** | `{ cap, used, reserved, remaining, window_resets_at }` per tenant. |
| **PiiRedactor** | Adapter over `efe_pii::Redactor` that walks JSON trees recursively. |
| **PiiStrategy** | Either `None` / `PreLlmRedact` / `Strict` — per-agent redaction policy. |
| **`m50.<agent>.v1`** | Prompt template ID convention for the 5 Sprint 5 starter agents. |
| **LlmCaller** | Existing `VmKind` from Sprint 4 ; wired to actual LLM call in Sprint 5. |
| **Strict mode anti-leakage** | `PiiStrategy::Strict` rejects LLM responses containing redaction tokens. |
| **NanoAgent{Calls,LlmTokens,VmSeconds}** | 3 new `Meter` variants additive to M31 (decision G.5). |

---

**End of EFE M50 Sprint 5 Architecture document.**

Sprint 5 ships **Mode C** in 10 working days with a team of 3 engineers.
By Sep 5, 2026, customers can mix Mode A + Mode B + Mode C operations in
one swarm, with PII redaction default-on, budget pre-checks per tenant,
strict JSON schema enforcement on LLM outputs, and 5 Mode C starter
agents covering 80 % of typical LLM use cases (summarization,
extraction, classification, sentiment, grounded Q&A).

Total Rust LOC at end of Sprint 5 : ~ 5 900 lines Sprint 4+5 (cumulative
~ 16 400 with Phase 1).  Total tests at end of Sprint 5 : ≥ 458 M50 +
225 baseline = ~ 683 green. Performance contracts met : Mode C
throughput 10 cps/node + overhead p99 ≤ 250 ms + pool hit rate ≥ 99 % +
output validation cache hit ≥ 99.99 %.

The path is now clear from Sprint 5 close (Sep 5, 2026) → Sprint 6
entry (Sep 10, 2026) → HMAC audit chain + Replay engine.  By Sep 28
(Phase 2 GA), customers have a **mode-agnostic, audit-ready, regulatory-
grade** EFE nano agent platform.
