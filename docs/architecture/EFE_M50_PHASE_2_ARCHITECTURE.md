# EFE M50 Phase 2 — Audit + Modes B/C Architecture

**Status**            : DRAFT v1
**Date drafted**      : 2026-05-18
**Owner**             : Rodrigue Pekar (founding engineer) + Senior Platform Engineer #1 + Senior Platform Engineer #2 (TBH)
**Phase**             : 2 of 5 (Audit + Modes B/C)
**Sprints**           : 4, 5, 6
**Dates target**      : Aug 11 - Sep 28, 2026 (6 weeks, 3 × 2-week sprints)
**Team size**         : 3 (founder + 2 senior hires)
**Predecessor**       : Phase 1 (Sprints 1-3, ended Aug 10, 2026)
**Companion docs**    :
- [`EFE_M50_AGENTIC_ARCHITECTURE.md`](EFE_M50_AGENTIC_ARCHITECTURE.md) (master architecture, 2,219 lignes)
- [`EFE_M50_PHASE_1_ARCHITECTURE.md`](EFE_M50_PHASE_1_ARCHITECTURE.md) (predecessor, 2,068 lignes)
- [`EFE_M50_VERIFICATION_REPORT_2026-05-18.md`](EFE_M50_VERIFICATION_REPORT_2026-05-18.md) (audit, 15 défauts → 9 fixed)
- [`EFE_M_NUMBER_REGISTRY.md`](EFE_M_NUMBER_REGISTRY.md) (canonical M-number registry)
- [`../roadmap/EFE_M50_ROADMAP.md`](../roadmap/EFE_M50_ROADMAP.md) (14-sprint plan)
- [`../decisions/D-M50-INFRA-001.md`](../decisions/D-M50-INFRA-001.md) (Postgres+Redis decision)
- [`../decisions/D-M50-CRATES-001.md`](../decisions/D-M50-CRATES-001.md) (2-crate split)

> **Contrat additif strict (rappel)** : Phase 2 ne modifie **aucun**
> fichier source des 33 crates existants. Toutes les modifications
> sont localisées dans `crates/efe-nano/` + `crates/efe-nano-sdk/`,
> avec ajouts (ZÉRO suppression) au workspace `Cargo.toml`. Les
> intégrations avec `efe-llm-router` (M21), `efe-curation` (M23),
> `efe-agent-runner` (M25), `efe-cache` (M27), `efe-pii` (M20),
> `efe-events` (M28), `efe-metering` (M31) se font **par consommation
> de leurs API publiques uniquement** — aucune modification.
>
> Vérification à chaque PR : les 174 tests PR #27 scope + 51 tests
> STABILITY v17 freeze + 277 tests M50 Phase 1 restent verts.  CI gate
> enforced.  En fin de Phase 2 : cible 450+ tests M50 cumulés, 174 +
> 51 = 225 baseline non modifiés.

---

## Table des matières

0. Executive summary
1. Phase 2 — scope précis (in / out / deferred)
2. Module hierarchy of `efe-nano` during Phase 2
3. Complete type system additions (Phase 2 surface)
4. Sprint 4 — Mode B Local Model Runtime (Day-by-day)
5. Sprint 5 — Mode C LLM-Augmented Runtime (Day-by-day)
6. Sprint 6 — Audit Trace + Replay (Day-by-day)
7. Postgres schema additions (v004-v007)
8. Error handling additions
9. Performance contracts Phase 2
10. Integration matrix avec crates EFE existants
11. CI / build integration Phase 2
12. Phase 2 → Phase 3 gate criteria
13. Test catalogue Phase 2 (~ 180 new tests)
14. Risk register specific to Phase 2
15. Annexe A — Mode B model loader trait + Firecracker pool
16. Annexe B — Mode C LLM client + prompt rendering pipeline
17. Annexe C — HMAC chain extension SQL + Rust types
18. Annexe D — Replay engine algorithm
19. Annexe E — 10 new starter agents specs
20. Annexe F — Sprint daily plans (30 working days)
21. Annexe G — Decision log

---

## 0. Executive summary

Phase 2 ships the **execution diversity** + **regulatory backbone** of
the EFE M50 platform. By end of Sprint 6 (Sep 28, 2026), the codebase
will contain :

```
═══════════════════════════════════════════════════════════════════
  Phase 2 Outputs (Sprint 4 → Sprint 6)
═══════════════════════════════════════════════════════════════════

  Mode B runtime             : Firecracker microVM pool, ≥ 100 calls/s/node
                               Pre-loaded local models (NER, classifier, embedder)
  Mode C runtime             : LLM-augmented via efe-llm-router (M21)
                               Strict JSON schema enforcement on output
                               Cost cap pre-check + post-charge
  Audit chain                : HMAC-chained traces via efe-curation (M23)
                               Per-call recording, per-tenant chain
  Replay capability          : Deterministic re-execution of historical swarms
                               100 % match on 1k synthetic swarms
  10 new starter agents      : 5 Mode B (NER, classify) + 5 Mode C (summarize, extract)
                               Total starter pack : 20 agents (was 10 in Phase 1)

  Cumulative Rust LOC added  : ~ 5,500 lines (cum. Phase 1+2 : ~ 10,500)
  Cumulative test LOC        : ~ 2,200 lines (cum. Phase 1+2 : ~ 3,700)
  Postgres migrations        : +4 (v004-v007) = total 7 since Phase 1
  Public API surface          : +6 types in efe-nano-sdk (Phase 2 surface)

═══════════════════════════════════════════════════════════════════

  Performance contract (Phase 2 → 3 gate) :
    Mode A throughput        : ≥ 1,000 calls/sec/node   (inherited Phase 1)
    Mode B throughput        : ≥ 100 calls/sec/node     (NEW)
    Mode C throughput        : ≥ 10 calls/sec/node      (NEW)
    Mode B latency p99       : ≤ 200 ms (model inference)
    Mode C latency p99       : ≤ 5000 ms (LLM provider)
    Mode B cold start p99    : ≤ 500 ms (pool miss path)
    Mode B pool hit rate     : ≥ 95 % at moderate load
    Audit chain write p99    : ≤ 30 ms (HMAC + Postgres insert)
    Replay determinism rate  : 100 % on 1k synthetic swarms
    Replay throughput        : ≥ 100 swarms/sec re-execution

  Compliance contract :
    Audit trace               : Full HMAC chain (parent_id → SHA256 → HMAC-SHA256)
                                Per-tenant chain, verifiable end-to-end
    Cost cap                  : Pre-check via efe-billing budget API (stub)
                                Post-charge reconciliation on Mode C completion
    PII redaction             : Pre-LLM via efe-pii (M20) for Mode C
    Per-tenant isolation     : Firecracker microVM kernel boundary (Mode B/C)

═══════════════════════════════════════════════════════════════════
```

Phase 2 transforms the platform from **Mode A only** (CPU-bound pure
Rust) to **mode-agnostic** (CPU + local inference + LLM-augmented).
The audit chain extension makes the platform **regulatory-grade**
(GDPR Art. 22, AI Act Art. 12, FINMA, FDA PCCP).

### What Phase 2 unlocks

After Phase 2 gate (Sep 28, 2026) :
- Drug-discovery workflows (1k+ PDFs, NER on each, structured extraction)
- Financial risk workflows (1k+ trades, local VaR Monte Carlo, LLM memo)
- Legal due-diligence (PDF extraction → classification → LLM summarization)
- Any pipeline that mixes deterministic + ML + LLM steps in one swarm
- Full audit trail for regulators (replay any swarm by `request_id`)

### What Phase 2 does NOT ship

Explicitly deferred to later phases :
- MCP server tool exposure → Phase 3 Sprint 7
- Customer-facing Rust SDK macros → Phase 3 Sprint 8
- Python + TypeScript SDKs → Phase 3 Sprint 10-11
- Stripe billing real integration → Phase 4 Sprint 11
- Cosign signing of audit bundles → Phase 4 Sprint 12
- 7-year retention provisioning → Phase 4 Sprint 12
- GA launch → Phase 4 Sprint 13

### Inherited from Phase 1 (must remain green)

```
Phase 1 deliverables (already shipped) :
  - efe-nano-sdk : 73 unit tests + 7 doctests
  - efe-nano lib : 188 unit tests
  - efe-nano integration : 9 tests
  - M50 cumulative : 277 tests
  - 174 PR #27 + 51 STABILITY v17 baseline preserved
  - In-memory catalog with semver + tenant isolation
  - Mode A PureRustRuntime + HandlerRegistry
  - Dispatcher (Kahn DAG + JoinSet parallel + Ref/Merge validation)
  - In-process idempotency cache (SHA-256 canonical key + TTL)
  - 10 starter pack agents (Mode A, TIER-1, EFE-shipped)
  - Audit trace stub (Console + InMemory writers)

Phase 2 ADDS to this — never removes or modifies.
```

---

## 1. Phase 2 — scope précis

### 1.1 In scope (must ship)

| Capability | Sprint | Owner | Deliverable |
|------------|--------|-------|-------------|
| Mode B runtime trait | 4 | Senior #1 | `efe-nano/src/runtime/local_model.rs` |
| Model loader (NER bio, classifier, embedder) | 4 | Senior #1 | `efe-nano/src/runtime/model_loader.rs` |
| Firecracker microVM pool (Mode B) | 4 | Senior #1 | `efe-nano/src/runtime/pool/mod.rs` + `firecracker.rs` |
| 5 Mode B starter agents | 4 | Founder | `starter_pack/ner_clinical.rs`, etc. |
| Mode C runtime trait | 5 | Senior #2 | `efe-nano/src/runtime/llm_augmented.rs` |
| LLM router integration (M21) | 5 | Senior #2 | `efe-nano/src/runtime/llm_client.rs` |
| Strict JSON schema enforcement on LLM output | 5 | Senior #2 | `efe-nano/src/runtime/output_validator.rs` |
| PII redaction (M20) pre-LLM | 5 | Senior #2 | `efe-nano/src/runtime/pii_redactor.rs` |
| Budget pre-check + post-charge stub | 5 | Senior #2 | `efe-nano/src/budget/` |
| 5 Mode C starter agents | 5 | Founder | `starter_pack/summarize_strict.rs`, etc. |
| HMAC chain audit writer | 6 | Senior #1 | `efe-nano/src/audit/hmac_chain.rs` |
| Postgres audit writer + migration v004 | 6 | Senior #1 | `efe-nano/src/audit/postgres.rs` + `migrations/v004_*.sql` |
| Per-tenant chain verification | 6 | Senior #1 | `efe-nano/src/audit/verify.rs` |
| Replay engine | 6 | Senior #2 | `efe-nano/src/replay/` |
| Determinism checker | 6 | Senior #2 | `efe-nano/src/replay/determinism.rs` |
| Replay CLI hook (admin only) | 6 | Senior #2 | `efe-nano/src/replay/cli.rs` |
| Multi-mode runtime dispatcher | 5 | Senior #1+#2 | `efe-nano/src/runtime/multi_mode.rs` |
| ~ 180 new tests | 4-6 | All | tests/ + lib unit tests |

### 1.2 Out of scope (Phase 3+)

| Capability | Reason out of Phase 2 | Target phase |
|------------|------------------------|--------------|
| MCP tool exposure | Customer-facing, needs SDK first | Phase 3 Sprint 7 |
| Rust SDK macros | Author DX, after distribution ready | Phase 3 Sprint 8 |
| Python SDK | Customer language coverage | Phase 3 Sprint 10 |
| TypeScript SDK | Customer language coverage | Phase 3 Sprint 11 |
| Stripe billing real integration | Monetization gate, post-Beta | Phase 4 Sprint 11 |
| Cosign audit signing | Audit-grade tier hardening | Phase 4 Sprint 12 |
| 7-year retention | Audit-grade tier hardening | Phase 4 Sprint 12 |
| Documentation site | Public-facing, post-Beta | Phase 3 Sprint 9 |
| MCP Marketplace submission | Customer-facing | Phase 3 Sprint 9 |
| Multi-region failover | Scaling problem, post-GA | Phase 5 |

### 1.3 Deferred but planned (within Phase 2)

These are anticipated but final decisions are within Phase 2 sprints :

- **Mode B model registry** : Sprint 4 ships a hard-coded set
  (`ner_clinical_v1`, `ner_legal_v1`, `intent_classify_v1`, `embedder_v1`,
  `lang_detect_v1`). Sprint 6+ adds dynamic registration via catalog.
- **Mode C provider preference** : Sprint 5 ships with default routing
  (efe-llm-router selects). Per-agent provider pinning lands Sprint 7+.
- **Audit chain back-fill** : Phase 1 InMemoryAuditWriter traces are NOT
  back-filled into the HMAC chain. The chain starts fresh at Sprint 6
  Day 1 with a `genesis` row per tenant.
- **Replay storage** : Sprint 6 uses Postgres for trace storage. ClickHouse
  migration for cold storage lands Phase 4.

---

## 2. Module hierarchy of `efe-nano` during Phase 2

State of `crates/efe-nano/src/` at end of each sprint :

### 2.1 End Sprint 3 (= start of Phase 2 = inherited from Phase 1)

```
crates/efe-nano/src/
├── lib.rs
├── error.rs
├── audit/
│   ├── mod.rs                      (AuditWriter trait + NanoAgentTrace)
│   └── stub.rs                     (ConsoleAuditWriter + InMemoryAuditWriter)
├── catalog/
│   ├── mod.rs
│   └── in_memory.rs                (InMemoryCatalog with semver)
├── dispatcher/
│   ├── mod.rs                      (Dispatcher orchestrator)
│   ├── types.rs                    (DagPlan, Layer, OpHandle, OperationOutcome)
│   ├── dag.rs                      (Kahn topo + cycle detection)
│   └── executor.rs                 (parallel JoinSet executor)
├── idempotency/
│   └── mod.rs                      (in-process TTL SHA-256 cache)
├── runtime/
│   ├── mod.rs                      (Runtime trait re-exports)
│   ├── context.rs                  (CallContext)
│   └── pure_rust.rs                (Mode A PureRustRuntime)
└── starter_pack/
    ├── mod.rs                      (register_all_starter_pack)
    ├── json_validate.rs            (Mode A)
    ├── xml_validate.rs             (Mode A)
    ├── regex_extract.rs            (Mode A)
    ├── date_normalize.rs           (Mode A)
    ├── currency_normalize.rs       (Mode A)
    ├── markdown_extract_headings.rs (Mode A)
    ├── csv_normalize.rs            (Mode A)
    ├── sql_parse_safe.rs           (Mode A)
    ├── html_extract_main_content.rs (Mode A)
    └── pdf_extract_text.rs         (Mode A)
```

### 2.2 End Sprint 4 (Mode B Local Model)

```
crates/efe-nano/src/
├── (Phase 1 state, unchanged)
├── runtime/
│   ├── mod.rs
│   ├── context.rs
│   ├── pure_rust.rs                  (Mode A, Phase 1)
│   ├── local_model.rs                (NEW — Mode B executor)
│   ├── model_loader.rs               (NEW — ModelLoader trait + registry)
│   ├── pool/
│   │   ├── mod.rs                    (NEW — VmPool trait)
│   │   ├── firecracker.rs            (NEW — Firecracker-backed VmPool)
│   │   └── stub.rs                   (NEW — InMemoryVmPool for tests)
│   └── multi_mode.rs                 (NEW — Mode A/B dispatcher)
└── starter_pack/
    ├── (Phase 1 agents, unchanged)
    ├── ner_clinical.rs               (NEW — Mode B, biomedical NER)
    ├── ner_legal.rs                  (NEW — Mode B, legal NER)
    ├── intent_classify_generic.rs    (NEW — Mode B, intent classifier)
    ├── lang_detect.rs                (NEW — Mode B, language detection)
    └── embed_text.rs                 (NEW — Mode B, text embedder)
```

### 2.3 End Sprint 5 (Mode C LLM-Augmented)

```
crates/efe-nano/src/
├── (Sprint 4 state)
├── budget/
│   ├── mod.rs                        (NEW — BudgetEnforcer trait)
│   └── stub.rs                       (NEW — InMemoryBudgetEnforcer)
├── runtime/
│   ├── (Sprint 4 state)
│   ├── llm_augmented.rs              (NEW — Mode C executor)
│   ├── llm_client.rs                 (NEW — adapter to efe-llm-router)
│   ├── output_validator.rs           (NEW — strict JSON schema on LLM output)
│   └── pii_redactor.rs               (NEW — adapter to efe-pii)
└── starter_pack/
    ├── (Sprint 4 state)
    ├── summarize_strict.rs           (NEW — Mode C, length-capped summary)
    ├── extract_structured.rs         (NEW — Mode C, JSON-schema extraction)
    ├── classify_topic_llm.rs         (NEW — Mode C, free-form topic classification)
    ├── sentiment_nuanced.rs          (NEW — Mode C, multi-label sentiment)
    └── qa_grounded.rs                (NEW — Mode C, citation-grounded Q&A)
```

### 2.4 End Sprint 6 (= end Phase 2 ; Audit + Replay)

```
crates/efe-nano/src/
├── (Sprint 5 state)
├── audit/
│   ├── mod.rs                        (unchanged from Phase 1)
│   ├── stub.rs                       (unchanged from Phase 1)
│   ├── hmac_chain.rs                 (NEW — HMAC chain writer)
│   ├── postgres.rs                   (NEW — PostgresAuditWriter)
│   └── verify.rs                     (NEW — chain integrity verifier)
├── replay/
│   ├── mod.rs                        (NEW — ReplayEngine)
│   ├── determinism.rs                (NEW — per-call determinism check)
│   ├── reconstructor.rs              (NEW — rebuild SwarmRequest from traces)
│   └── cli.rs                        (NEW — admin CLI hook)
└── migrations/
    ├── v004_create_nano_agent_traces.sql        (NEW Sprint 6)
    ├── v005_create_traces_chain_indexes.sql     (NEW Sprint 6)
    ├── v006_create_replay_attempts.sql          (NEW Sprint 6)
    └── v007_constraints_traces_chain_fk.sql     (NEW Sprint 6)
```

Total at end of Phase 2 : **~ 55 .rs files** in `efe-nano/`, **7 SQL
migrations** (3 Phase 1 + 4 Phase 2), **~ 20 nano agents** in starter
pack (10 Phase 1 + 10 Phase 2).

### 2.5 SDK additions (`crates/efe-nano-sdk/src/`) during Phase 2

```
crates/efe-nano-sdk/src/
├── (Phase 1 state)
└── types/
    ├── (Phase 1 types, unchanged)
    └── runtime_b_c.rs                (NEW — Mode B/C public types)
```

The SDK gets new variants on `ExecutionMode` :
- `ExecutionMode::LocalModel { model_id: String, model_version: String }`
- `ExecutionMode::LlmAugmented { prompt_template_id, max_tokens, temperature, providers_allow_list }`

These were stubbed in Phase 1 (so the type already exists) ; Sprint 4
+ 5 ship the runtime implementations.

---

## 3. Complete type system additions (Phase 2 surface)

Phase 1 shipped the bulk of the SDK type system. Phase 2 adds the
runtime-side types + the per-mode metadata structs.

### 3.1 ModelLoader trait + Model handle (Sprint 4)

```rust
// efe-nano/src/runtime/model_loader.rs

use async_trait::async_trait;
use std::sync::Arc;

/// Loader for Mode B local models.
///
/// Phase 2 ships a single in-process implementation that holds
/// pre-loaded models in memory. Phase 4 swaps in a sharded loader
/// across the microVM pool.
#[async_trait]
pub trait ModelLoader: Send + Sync + 'static {
    /// Get or lazily load a model by id + version.
    async fn get_or_load(
        &self,
        model_id: &str,
        model_version: &str,
    ) -> Result<Arc<dyn LocalModel>, ModelLoaderError>;

    /// Pre-warm the loader with a fixed set of models.
    async fn warm(&self, models: &[(String, String)]) -> Result<(), ModelLoaderError>;

    /// Pool health snapshot.
    async fn health(&self) -> ModelLoaderHealth;
}

#[async_trait]
pub trait LocalModel: Send + Sync + 'static {
    /// Run inference. Input is the validated JSON payload from the
    /// nano agent ; output must match the agent's output_schema.
    async fn infer(&self, input: serde_json::Value) -> Result<serde_json::Value, ModelInferError>;

    /// Model identifier + version for audit + tracing.
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn family(&self) -> ModelFamily;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    NamedEntityRecognition,
    Classifier,
    LanguageDetector,
    Embedder,
    Generic,
}

#[derive(Debug)]
pub struct ModelLoaderHealth {
    pub loaded_models: u32,
    pub total_memory_mb: u32,
    pub avg_load_ms: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelLoaderError {
    #[error("model '{0}' not found in registry")]
    NotFound(String),
    #[error("model load failed : {0}")]
    LoadFailed(String),
    #[error("loader internal error : {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ModelInferError {
    #[error("inference failed : {0}")]
    Inference(String),
    #[error("input schema mismatch : {0}")]
    InputSchema(String),
    #[error("output schema mismatch : {0}")]
    OutputSchema(String),
    #[error("timed out after {0} ms")]
    Timeout(u32),
}
```

### 3.2 VmPool trait + Firecracker handle (Sprint 4)

```rust
// efe-nano/src/runtime/pool/mod.rs

use async_trait::async_trait;
use std::time::Duration;

/// Pool of warm microVMs available for Mode B/C execution.
///
/// Phase 2 ships a Firecracker-backed implementation via the existing
/// `efe-agent-runner` (M25) crate API. Phase 4 adds dynamic scaling.
#[async_trait]
pub trait VmPool: Send + Sync + 'static {
    /// Acquire a warm VM. Returns immediately if pool has capacity,
    /// otherwise blocks up to `wait_budget` (then provisions on demand).
    async fn acquire(
        &self,
        kind: VmKind,
        wait_budget: Duration,
    ) -> Result<VmLease, VmPoolError>;

    /// Pool health snapshot.
    async fn health(&self) -> VmPoolHealth;
}

/// Kind of VM requested. Pool can keep different warm-set sizes per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VmKind {
    /// Mode B : pre-loaded model microVM.
    ModelInference,
    /// Mode C : lightweight microVM with LLM HTTP client only.
    LlmCaller,
}

/// RAII lease for a VM. On Drop, VM returns to pool (if pool not full)
/// or is destroyed.
pub struct VmLease {
    pub vm_id: uuid::Uuid,
    pub kind: VmKind,
    pub acquired_at: std::time::Instant,
    /// Handle into the executor — the lease holds the channel by which
    /// callers send inference requests + read responses.
    pub(crate) executor: Box<dyn VmExecutor>,
}

impl VmLease {
    pub async fn execute(&mut self, request: VmRequest) -> Result<VmResponse, VmExecError> {
        self.executor.execute(request).await
    }
}

#[async_trait]
pub(crate) trait VmExecutor: Send + Sync {
    async fn execute(&mut self, req: VmRequest) -> Result<VmResponse, VmExecError>;
}

#[derive(Debug, Clone)]
pub struct VmRequest {
    pub agent_id: String,
    pub agent_version: String,
    pub input: serde_json::Value,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone)]
pub struct VmResponse {
    pub output: serde_json::Value,
    pub cpu_ms_used: u32,
    pub memory_mb_peak: u32,
}

#[derive(Debug, Clone)]
pub struct VmPoolHealth {
    pub kind_states: std::collections::HashMap<VmKind, PerKindHealth>,
}

#[derive(Debug, Clone)]
pub struct PerKindHealth {
    pub warm_slots: u32,
    pub in_use_slots: u32,
    pub max_slots: u32,
    pub hit_rate_60s: f32,        // [0.0, 1.0]
    pub p99_provision_ms: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum VmPoolError {
    #[error("pool exhausted : {wanted} requested, {available} available")]
    Exhausted { wanted: u32, available: u32 },
    #[error("pool acquire timed out after {0:?}")]
    AcquireTimeout(Duration),
    #[error("VM provisioning failed : {0}")]
    ProvisionFailed(String),
    #[error("internal : {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VmExecError {
    #[error("VM execution failed : {0}")]
    Execution(String),
    #[error("VM crashed during execution")]
    Crashed,
    #[error("VM execution timed out after {0} ms")]
    Timeout(u32),
}
```

### 3.3 LLM client adapter (Sprint 5)

```rust
// efe-nano/src/runtime/llm_client.rs

use async_trait::async_trait;
use std::sync::Arc;

/// Lightweight adapter trait over `efe-llm-router::LLMRouter`.
///
/// We do not directly expose the LLMRouter struct because :
/// 1. It pulls in many transitive dependencies (providers, audit, cache).
/// 2. The trait surface gives us testability via mock implementations.
///
/// Phase 2 ships a single production impl wrapping the real router +
/// a mock for unit tests.
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    /// Render the prompt + call the underlying LLM router.
    ///
    /// The router decides which provider to use based on tenant config
    /// + cost governor + circuit breaker.
    async fn call(
        &self,
        request: LlmCallRequest,
    ) -> Result<LlmCallResponse, LlmCallError>;

    /// Estimate cost in cents before committing to the call. Used by
    /// the budget pre-check.
    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64;
}

/// Subset of `efe-llm-router::RouterRequest` exposed at the M50 layer.
/// We keep `prompt_template_id` here so the M21 PromptRegistry remains
/// the source of truth for prompt content (no string concat in M50).
#[derive(Debug, Clone)]
pub struct LlmCallRequest {
    pub tenant_id: efe_nano_sdk::TenantId,
    pub prompt_template_id: String,
    pub template_inputs: serde_json::Value,
    pub max_tokens: u32,
    pub temperature: f32,
    pub providers_allow_list: Vec<String>,
    pub response_format: ResponseFormat,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone)]
pub enum ResponseFormat {
    /// Free-form text (rare for M50 — most agents need structured).
    FreeText,
    /// JSON object validated against this schema. The router will
    /// instruct compatible providers (OpenAI tool-use, Anthropic JSON
    /// mode) to emit JSON ; M50 then strictly validates.
    JsonSchema(serde_json::Value),
}

#[derive(Debug, Clone)]
pub struct LlmCallResponse {
    pub content: serde_json::Value,         // already JSON-parsed if JsonSchema
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_cents: f64,
    pub provider_name: String,
    pub model_name: String,
    pub cache_hit: bool,
    pub duration_ms: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmCallError {
    #[error("provider error : {0}")]
    Provider(String),
    #[error("response is not valid JSON : {0}")]
    InvalidJson(String),
    #[error("response failed schema validation : {0}")]
    SchemaMismatch(String),
    #[error("budget pre-check failed : insufficient {cents_remaining} cents")]
    BudgetInsufficient { cents_remaining: f64 },
    #[error("call timed out after {0} ms")]
    Timeout(u32),
    #[error("internal : {0}")]
    Internal(String),
}
```

### 3.4 BudgetEnforcer trait (Sprint 5)

```rust
// efe-nano/src/budget/mod.rs

use async_trait::async_trait;
use efe_nano_sdk::TenantId;

/// Per-tenant budget enforcement for Mode C (LLM) calls.
///
/// Phase 2 ships an in-memory stub. Sprint 11 (Phase 4) replaces it
/// with the real `efe-billing` (M31) integration.
#[async_trait]
pub trait BudgetEnforcer: Send + Sync + 'static {
    /// Reserve `cents` from the tenant's budget. Returns `Ok(reservation_id)`
    /// on success, `BudgetError::Insufficient` if the pre-check fails.
    async fn pre_check(
        &self,
        tenant: TenantId,
        cents: f64,
    ) -> Result<ReservationId, BudgetError>;

    /// Convert the reservation into a final charge after the call
    /// completes with `actual_cents` (may be less than reserved).
    async fn commit(
        &self,
        reservation: ReservationId,
        actual_cents: f64,
    ) -> Result<(), BudgetError>;

    /// Release the reservation without charging (e.g. on error).
    async fn release(&self, reservation: ReservationId) -> Result<(), BudgetError>;

    /// Current budget snapshot for a tenant.
    async fn snapshot(&self, tenant: TenantId) -> Result<BudgetSnapshot, BudgetError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub uuid::Uuid);

#[derive(Debug, Clone)]
pub struct BudgetSnapshot {
    pub tenant: TenantId,
    pub cap_cents: f64,
    pub used_cents: f64,
    pub reserved_cents: f64,
    pub remaining_cents: f64,
    pub window_resets_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget insufficient : {requested_cents} cents requested, {remaining_cents} remaining")]
    Insufficient { requested_cents: f64, remaining_cents: f64 },
    #[error("reservation '{0}' not found or already committed/released")]
    UnknownReservation(uuid::Uuid),
    #[error("budget service internal : {0}")]
    Internal(String),
}
```

### 3.5 HmacChainWriter (Sprint 6)

```rust
// efe-nano/src/audit/hmac_chain.rs

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::{AuditWriter, NanoAgentTrace};
use efe_nano_sdk::TenantId;

/// Postgres-backed audit writer that links each trace into a per-tenant
/// HMAC chain.
///
/// Schema design (cf § 7 below + Annexe C) :
/// ```sql
/// CREATE TABLE nano_agent_traces (
///     trace_id            UUID         PRIMARY KEY,
///     tenant_id           UUID         NOT NULL,
///     parent_trace_id     UUID         REFERENCES nano_agent_traces(trace_id),
///     chain_parent_id     UUID         REFERENCES nano_agent_traces(trace_id),
///     swarm_request_id    UUID         NOT NULL,
///     operation_id        TEXT         NOT NULL,
///     nano_agent_id       TEXT         NOT NULL,
///     nano_agent_version  TEXT         NOT NULL,
///     execution_mode      TEXT         NOT NULL,
///     input_hash          BYTEA        NOT NULL,      -- SHA-256 32 bytes
///     output_hash         BYTEA        NULL,
///     error_kind          TEXT         NULL,
///     started_at          TIMESTAMPTZ  NOT NULL,
///     completed_at        TIMESTAMPTZ  NULL,
///     latency_ms          INTEGER      NULL,
///     cost_cents          DOUBLE PRECISION NOT NULL,
///     payload_hash        BYTEA        NOT NULL,      -- SHA-256 of the row body
///     hmac                BYTEA        NOT NULL       -- HMAC-SHA-256(chain_parent_id || payload_hash, key)
/// );
/// CREATE INDEX traces_tenant_started ON nano_agent_traces (tenant_id, started_at DESC);
/// CREATE INDEX traces_swarm           ON nano_agent_traces (swarm_request_id);
/// CREATE INDEX traces_chain_parent    ON nano_agent_traces (chain_parent_id);
/// ```
pub struct HmacChainAuditWriter {
    pool: PgPool,
    hmac_key_source: HmacKeySource,
}

impl HmacChainAuditWriter {
    pub fn new(pool: PgPool, key_source: HmacKeySource) -> Self {
        Self { pool, hmac_key_source: key_source }
    }

    fn audit_key_for(&self, tenant: TenantId) -> Result<Vec<u8>, AuditChainError> {
        self.hmac_key_source.derive(tenant)
    }
}

/// How HMAC keys are derived. Phase 2 uses ENV-secret + tenant id ;
/// Phase 4 swaps in HashiCorp Vault.
pub enum HmacKeySource {
    /// Derive HMAC key from `EFE_NANO_AUDIT_HMAC_MASTER` env var via
    /// HKDF(salt = tenant_id, info = "efe-nano/audit/v1").
    EnvHkdf,
    /// Vault transit engine (Phase 4).
    #[allow(dead_code)]
    VaultTransit { path: String },
}

impl HmacKeySource {
    pub fn derive(&self, tenant: TenantId) -> Result<Vec<u8>, AuditChainError> {
        // Implementation in Sprint 6 Day 2.
        match self {
            Self::EnvHkdf => {
                let master = std::env::var("EFE_NANO_AUDIT_HMAC_MASTER")
                    .map_err(|_| AuditChainError::MissingEnvVar)?;
                // HKDF-SHA-256(secret = master, salt = tenant_uuid, info = "...")
                let key = hkdf_sha256(master.as_bytes(), tenant.as_uuid().as_bytes(), b"efe-nano/audit/v1");
                Ok(key)
            }
            Self::VaultTransit { .. } => {
                Err(AuditChainError::NotImplemented("vault transit deferred to Phase 4".into()))
            }
        }
    }
}

#[async_trait]
impl AuditWriter for HmacChainAuditWriter {
    async fn record(&self, trace: NanoAgentTrace) {
        // Implementation in Sprint 6 Day 3-4.
        // Sketch :
        // 1. SHA256 of canonical row body → payload_hash.
        // 2. SELECT trace_id FROM nano_agent_traces
        //    WHERE tenant_id = $1 ORDER BY started_at DESC LIMIT 1
        //    → chain_parent_id (None for first per tenant).
        // 3. HMAC-SHA-256(chain_parent_id || payload_hash, audit_key)
        //    → hmac.
        // 4. INSERT INTO nano_agent_traces.
        // 5. On Postgres error : log via tracing!() and degrade
        //    gracefully (do NOT panic — audit writer must be
        //    resilient ; missing traces flagged via metric).
        let _ = (trace, &self.pool, &self.hmac_key_source);
        unimplemented!("Sprint 6 Day 3-4 — TBD")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuditChainError {
    #[error("HMAC master key environment variable is missing")]
    MissingEnvVar,
    #[error("Postgres error : {0}")]
    Postgres(String),
    #[error("chain broken at trace {0}")]
    ChainBroken(Uuid),
    #[error("HMAC mismatch at trace {0}")]
    HmacMismatch(Uuid),
    #[error("not implemented : {0}")]
    NotImplemented(String),
}

fn hkdf_sha256(_secret: &[u8], _salt: &[u8], _info: &[u8]) -> Vec<u8> {
    // Sprint 6 Day 2 : implement via the `hkdf` crate (already used by M23).
    Vec::new()
}
```

### 3.6 ReplayEngine (Sprint 6)

```rust
// efe-nano/src/replay/mod.rs

use std::sync::Arc;
use uuid::Uuid;

use crate::audit::AuditWriter;
use crate::dispatcher::Dispatcher;
use efe_nano_sdk::{Catalog, SwarmRequest, SwarmResponse, TenantId};

/// Engine that re-executes a historical swarm by `request_id` and
/// compares output hashes to the originally-recorded ones.
///
/// Phase 2 supports replay of swarms that consist **entirely of
/// FullDeterministic Mode A operations**. Mode B/C replay is best-effort
/// (no determinism guarantee for LLMs) and is reported separately.
pub struct ReplayEngine {
    pool: sqlx::PgPool,
    catalog: Arc<dyn Catalog>,
    dispatcher: Arc<Dispatcher>,
}

impl ReplayEngine {
    pub fn new(
        pool: sqlx::PgPool,
        catalog: Arc<dyn Catalog>,
        dispatcher: Arc<Dispatcher>,
    ) -> Self {
        Self { pool, catalog, dispatcher }
    }

    /// Replay a swarm by `request_id`. Returns a report comparing
    /// each op's original output_hash to the freshly-computed one.
    pub async fn replay(
        &self,
        tenant: TenantId,
        request_id: Uuid,
    ) -> Result<ReplayReport, ReplayError> {
        // 1. Load all traces for this request_id.
        // 2. Reconstruct the SwarmRequest from traces.
        // 3. Verify HMAC chain integrity.
        // 4. Re-dispatch the request.
        // 5. Compare per-op output_hash.
        // 6. Build a ReplayReport.
        let _ = (tenant, request_id, &self.pool, &self.catalog, &self.dispatcher);
        unimplemented!("Sprint 6 Day 7-10")
    }
}

#[derive(Debug, Clone)]
pub struct ReplayReport {
    pub request_id: Uuid,
    pub tenant: TenantId,
    pub chain_verified: bool,
    pub total_ops: u32,
    pub matched_ops: u32,
    pub mismatched_ops: u32,
    pub skipped_non_deterministic_ops: u32,
    pub per_op: Vec<PerOpReplay>,
    pub new_response: Option<SwarmResponse>,
}

#[derive(Debug, Clone)]
pub struct PerOpReplay {
    pub op_id: String,
    pub nano_agent_id: String,
    pub original_output_hash: [u8; 32],
    pub replay_output_hash: Option<[u8; 32]>,
    pub status: ReplayOpStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayOpStatus {
    Match,
    Mismatch,
    SkippedNonDeterministic,
    ReplayFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("no traces found for request {0}")]
    NotFound(Uuid),
    #[error("HMAC chain broken : {0}")]
    ChainBroken(String),
    #[error("Postgres error : {0}")]
    Postgres(String),
    #[error("dispatcher error : {0}")]
    Dispatch(String),
}

mod determinism;
mod reconstructor;
pub use determinism::DeterminismChecker;
pub use reconstructor::SwarmReconstructor;
```

---

## 4. Sprint 4 — Mode B Local Model Runtime (Day-by-day)

**Dates** : Aug 11-25, 2026 (10 working days)
**Team** : Rodrigue (founder) + Senior #1 + Senior #2 (TBH)
**Goal** : Ship `LocalModelRuntime` + `ModelLoader` + `VmPool` + 5 Mode
B starter agents. Demo : 100 NER calls in parallel on biomedical text,
hit pool at 95 %+.

### 4.1 Sprint 4 entry conditions

```
[ ] Phase 1 ended cleanly :
    [✓] 277 M50 tests green
    [✓] 174 PR #27 + 51 STABILITY baseline green
    [✓] Phase 1 gate report signed off
[ ] Senior #2 onboarded (Week 1 ramp-up done in pre-Sprint week)
[ ] efe-agent-runner (M25) v1.0 API documented and available
[ ] Postgres staging DB provisioned for Sprint 6 prep (read-only Sprint 4)
[ ] 5 biomedical NER + 5 legal NER test fixtures procured (anonymized)
[ ] Decision : which embedder to ship in Sprint 4 (locked at Phase 2
     kickoff meeting Aug 11 morning)
```

### 4.2 Day-by-day plan

#### Day 1 (Mon Aug 11)
**Morning** : Phase 2 kickoff meeting (3 h).
- Review Phase 1 outcomes + lessons learned.
- Finalize ModelLoader + VmPool trait surfaces (locked at lunch).
- Sign off on Mode B model registry (5 models : ner_clinical_v1,
  ner_legal_v1, intent_classify_v1, lang_detect_v1, embedder_v1).
- Hand-off : Senior #1 leads ModelLoader; Senior #2 leads VmPool;
  Founder leads the 5 starter agents.

**Afternoon** : All three start coding in parallel.

Senior #1 :
- Create `efe-nano/src/runtime/model_loader.rs`
- Implement `InMemoryModelLoader` struct with a HashMap-backed registry
- Stub `ModelLoader` trait methods (return errors)
- Write 4 unit tests for trait surface (compile-only smoke + error mapping)

Senior #2 :
- Create `efe-nano/src/runtime/pool/mod.rs`
- Define `VmPool` trait + `VmKind`, `VmLease` types
- Stub `InMemoryVmPool` (no real Firecracker yet)
- Write 4 unit tests for trait surface

Founder :
- Audit `efe-agent-runner` (M25) public API for Firecracker usage
- Sketch 5 Mode B agent manifest entries in `starter_pack/ner_clinical.rs`,
  `ner_legal.rs`, `intent_classify_generic.rs`, `lang_detect.rs`,
  `embed_text.rs` (no handlers yet)
- Commit : 5 new files, each with `manifest()` only (no `handler()`).

**End-of-day** : 3 PRs open, all reviewed by next morning. Build green.

#### Day 2 (Tue Aug 12)
Senior #1 :
- Implement model loading from disk (`.onnx` files via `tract` crate)
- `ner_clinical_v1` : load BiomedNER ONNX model (~ 200 MB)
- `embedder_v1` : load MiniLM-L6-v2 ONNX (~ 80 MB)
- Add 6 tests : load success, load failure, idempotent get_or_load, warm
  multiple models, health snapshot

Senior #2 :
- Define `VmExecutor` trait (private) + channel-based stub impl
- `InMemoryVmPool::acquire` returns a stub lease that runs the request
  in-process (no actual Firecracker yet — Sprint 4 Day 4-5 wires real
  microVM)
- Add 5 tests : acquire success, acquire timeout, lease drop returns to
  pool, kind isolation, health snapshot

Founder :
- Implement Mode B handler shape (see § 4.3) for `ner_clinical`
- Wire the handler to call `ModelLoader::get_or_load("ner_clinical_v1", "1.0.0")`
  then `model.infer(input).await`
- Add 3 tests on the agent's manifest + handler error paths (model
  not loaded → Internal error; bad input schema → SchemaValidation)

**End-of-day** : ModelLoader can load 2 models; VmPool stub works in
unit tests; first Mode B handler shape demonstrated.

#### Day 3 (Wed Aug 13)
Senior #1 :
- Add 3 more models : `ner_legal_v1`, `intent_classify_v1`,
  `lang_detect_v1`. All ONNX, ~ 50-150 MB each.
- Total registry : 5 models, ~ 600 MB at warm state.
- Performance baseline : per-model `infer()` runs in 30-150 ms on
  reference hardware (M2 Mac Mini fixture). Document in tests.

Senior #2 :
- Wire `efe-agent-runner` (M25) Firecracker API into `pool/firecracker.rs`
- Provision a single-VM test : spawn a microVM, send 1 JSON request via
  vsock, receive response, tear down.
- Pool config : 5 warm VMs of kind `ModelInference` (one per model).
- Add 2 integration tests : single VM end-to-end + 5-VM pool warm-up.

Founder :
- Implement 4 remaining Mode B handlers : `ner_legal`,
  `intent_classify_generic`, `lang_detect`, `embed_text`.
- Each handler : look up the right model via `ModelLoader`, call infer,
  validate output via JSON schema.
- Add 4 tests per handler (manifest + 3 handler paths).

**End-of-day** : 5 Mode B handlers integrated; Firecracker single-VM
demo proven; warm-up tests added.

#### Day 4 (Thu Aug 14)
Senior #1 :
- Add cancel + retry semantics to `ModelLoader::get_or_load` (race
  protection : if 10 concurrent loads of same model, only 1 actual load
  + 9 await the same future).
- Property test : 100 concurrent loads of same model → exactly 1 actual
  load_disk call.

Senior #2 :
- Implement pool scaling : if `acquire(kind, budget)` finds no warm VM,
  spawn a new one within budget OR return `Exhausted` after budget.
- Tests : pool grows from 5 → 20 VMs over 1 sec under load; respects
  `max_slots`; hit_rate metric updates correctly.

Founder :
- Connect Mode B handlers to the VmPool (via `MultiModeRuntime`).
- `MultiModeRuntime::execute_with_context` dispatches based on
  `agent.execution_mode` :
  - `PureRust { entry }` → existing `PureRustRuntime`
  - `LocalModel { model_id, .. }` → acquire VM from pool, call infer.
  - `LlmAugmented { .. }` → return `Internal("Mode C not yet wired")` (Sprint 5)

**End-of-day** : `MultiModeRuntime` dispatches Mode A + Mode B correctly.

#### Day 5 (Fri Aug 15)
All three : integration day.

- Wire Mode B agents into `starter_pack::register_all_starter_pack`.
- All 15 starter agents (10 Mode A from Phase 1 + 5 Mode B) load on
  fresh dispatcher instantiation.
- Integration test : run a 50-op DAG mixing Mode A + Mode B (e.g.
  10 × pdf_extract_text → 10 × ner_clinical) end-to-end.
- Benchmark : Mode B throughput hit ≥ 50 calls/sec on dev hardware
  (target 100 calls/sec on staging hardware — measure Sprint 6).

**End-of-day** : Mode B integration end-to-end demo committed; sprint
mid-point checkpoint with team.

#### Day 6-7 (Mon-Tue Aug 18-19)
Senior #1 :
- Production-grade health metrics (`tracing::info!` per acquire,
  Prometheus gauge for warm/in_use, p99 acquire histogram).
- Add a `warm()` method that pre-loads N models at startup ; called
  from runtime `new_with_warmup()`.

Senior #2 :
- Production-grade pool scaling : back-pressure when at max capacity,
  fairness across kinds (no kind starves another).
- Per-VM crash handling : if a VM crashes mid-execute, retry on a fresh
  VM (up to `RetryPolicy::max_attempts`).
- 4 chaos tests : crash VM mid-execute, kill 50 % of pool, pool full +
  burst, fairness between Mode B + Mode C kinds.

Founder :
- Refine Mode B agent manifests with realistic `cost_estimate_cents`
  (0.1 - 0.5 cents per call) and `latency_p99_ms_target` (≤ 200).
- 5 integration tests for end-to-end Mode B agent execution through the
  dispatcher (with mock model loader).

#### Day 8-9 (Wed-Thu Aug 20-21)
Senior #1 + Senior #2 : performance pass.
- Profile Mode B path (catalog lookup → schema validate → VM acquire →
  infer → output schema validate → audit record).
- Identify bottlenecks : schema compilation cached (Phase 1 already
  recompiles per call — fix this with a `Arc<JSONSchema>` cache on
  the dispatcher).
- Reduce per-call overhead from ~ 30 ms (initial measurement) to
  ≤ 5 ms (overhead only ; model inference itself is ~ 50-150 ms).
- Bench result : Mode B throughput on staging-class hardware ≥ 100
  calls/sec/node ✓.

Founder :
- Documentation update : `crates/efe-nano/README.md` Sprint 4 section.
- Update `EFE_M50_PHASE_1_ARCHITECTURE.md` § 5 (Phase 1 → 2 transition)
  to reference the new modules.

#### Day 10 (Fri Aug 22)
Sprint 4 close :
- All Mode B tests pass : 35 new tests (5 ModelLoader + 9 VmPool + 4
  Firecracker integration + 5 Mode B handlers + 5 chaos tests + 7
  perf tests).
- Cumulative M50 test count : 277 (Phase 1) + 35 (Sprint 4) = 312.
- Sprint 4 retro meeting (1 h Fri afternoon).
- Sprint 5 kickoff prep (Senior #2 takes the Mode C lead).

### 4.3 Mode B handler shape (canonical)

```rust
// Example : ner_clinical.rs

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("ner_clinical").unwrap(),
        version: NanoAgentVersion::new("1.0.0").unwrap(),
        description: "Biomedical NER : extract DRUG, DISEASE, GENE, PHASE entities.".to_string(),
        author: Author::Efe,
        category: Category::EntityRecognition,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "ner_clinical_v1".to_string(),
            model_version: "1.0.0".to_string(),
        },
        determinism: DeterminismLevel::ProbabilisticFlavor,    // model inference, mostly deterministic
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "maxLength": 50_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["entities", "count"],
            "properties": {
                "count": { "type": "integer", "minimum": 0 },
                "entities": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["text", "kind", "start", "end", "score"],
                        "properties": {
                            "text": { "type": "string" },
                            "kind": { "enum": ["DRUG", "DISEASE", "GENE", "PHASE"] },
                            "start": { "type": "integer", "minimum": 0 },
                            "end": { "type": "integer", "minimum": 0 },
                            "score": { "type": "number", "minimum": 0, "maximum": 1 }
                        }
                    }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 2_048,
            cpu_ms: 5_000,
            wall_clock_ms: 10_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.1,
        latency_p99_ms_target: 200,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr_relevant: true,         // medical text → personal data
            hipaa_relevant: true,
            ai_act_high_risk: false,
            dpia_required: true,         // medical NER triggers DPIA
            ..ComplianceTags::default()
        },
        mcp_tool_metadata: None,
        created_at: Utc::now(),
    }
}

// Handler does NOT exist for Mode B agents the same way as Mode A.
// Instead, `MultiModeRuntime` looks up the model via `ModelLoader` and
// invokes it directly. No closure registration in `HandlerRegistry`.
```

### 4.4 Mode B execution flow (sequence)

```
Caller → Dispatcher → MultiModeRuntime.execute_with_context(agent, input, ctx)
                       │
                       ├── Mode A path (Phase 1) — unchanged
                       │
                       └── Mode B path (Sprint 4) :
                            │
                            ├── 1. Validate input vs agent.input_schema (cached JSONSchema)
                            ├── 2. Check ctx.deadline (early return if past)
                            ├── 3. budget.pre_check(tenant, agent.cost_estimate_cents) → ReservationId
                            │      (stub returns Ok immediately in Sprint 4 ; full impl Sprint 5)
                            ├── 4. vm_pool.acquire(VmKind::ModelInference, wait_budget) → VmLease
                            ├── 5. lease.execute(VmRequest { agent_id, input, timeout_ms })
                            │      │
                            │      ├── Inside microVM :
                            │      │   model_loader.get_or_load(model_id, version) → Arc<dyn LocalModel>
                            │      │   model.infer(input) → output
                            │      │   metering : track CPU + memory
                            │      └── Return output via vsock
                            ├── 6. Validate output vs agent.output_schema
                            ├── 7. budget.commit(reservation, actual_cents)
                            ├── 8. Build NanoAgentResponse
                            └── 9. audit.record(NanoAgentTrace { execution_mode: "local_model", ... })
```

### 4.5 Sprint 4 acceptance criteria

```
[ ] ModelLoader trait + InMemoryModelLoader shipped
[ ] 5 ONNX models loadable : ner_clinical, ner_legal, intent, lang_detect, embedder
[ ] VmPool trait + InMemoryVmPool + FirecrackerVmPool shipped
[ ] FirecrackerVmPool integrates with efe-agent-runner (M25)
[ ] 5 Mode B starter agents registered in starter_pack
[ ] MultiModeRuntime dispatches Mode A + Mode B correctly
[ ] Mode B throughput ≥ 50 calls/sec on dev (target 100 on staging)
[ ] Pool hit rate ≥ 95 % at moderate load (50 concurrent ops)
[ ] 35 new tests pass
[ ] 174 PR #27 + 51 STABILITY + 277 Phase 1 toujours verts
[ ] cumulative test count : 312 M50 tests
```

---

## 5. Sprint 5 — Mode C LLM-Augmented Runtime (Day-by-day)

**Dates** : Aug 26 - Sep 9, 2026 (10 working days)
**Team** : full team (founder + 2 seniors)
**Goal** : Ship `LlmAugmentedRuntime` + `LlmClient` adapter +
`OutputValidator` + 5 Mode C starter agents. Demo : extract structured
fields from 100 PDFs using LLM with strict JSON schema validation.

### 5.1 Sprint 5 entry conditions

```
[ ] Sprint 4 closed cleanly :
    [✓] 312 M50 tests green
    [✓] Mode B throughput ≥ 50 calls/sec on dev
[ ] efe-llm-router (M21) v1.x stable + tenant config available for tests
[ ] efe-pii (M20) Redactor public API documented
[ ] efe-prompts (M21) PromptRegistry available for template rendering
[ ] efe-billing (M31) BudgetEnforcer trait shape signed off (or stub plan agreed)
[ ] Decision : initial set of 5 prompt templates (`m50.summarize.v1`,
    `m50.extract_structured.v1`, etc.) registered in PromptRegistry
```

### 5.2 Day-by-day plan

#### Day 1 (Mon Aug 26)
**Morning** : Sprint 5 kickoff meeting (2 h).
- Confirm `LlmClient` trait surface (locked at lunch).
- Confirm PII redaction strategy (default-on for Mode C, opt-out per
  agent via `agent.compliance.pii_redaction_strategy` enum).
- Hand-off : Senior #2 leads `LlmClient` + `OutputValidator` ;
  Senior #1 leads `BudgetEnforcer` stub + `PiiRedactor` ;
  Founder leads the 5 Mode C agents.

**Afternoon** :

Senior #2 :
- Create `efe-nano/src/runtime/llm_client.rs`
- Define `LlmClient` trait (cf § 3.3)
- Implement `LlmRouterClient` (production impl wrapping efe-llm-router)
  and `MockLlmClient` (for tests)
- 4 unit tests : trait shape, mock returns canned response, error mapping
  from `RouterError` → `LlmCallError`

Senior #1 :
- Create `efe-nano/src/budget/mod.rs`
- Define `BudgetEnforcer` trait + `InMemoryBudgetEnforcer`
- 6 unit tests : pre_check success, pre_check insufficient, commit,
  release, double-commit error, snapshot accuracy

Founder :
- Stub 5 Mode C agent manifests : `summarize_strict`,
  `extract_structured`, `classify_topic_llm`, `sentiment_nuanced`,
  `qa_grounded` (manifest only, no handlers)
- Each manifest references a prompt template id (e.g.
  `m50.summarize.v1`)

**End-of-day** : 3 PRs open, foundations in place.

#### Day 2 (Tue Aug 27)
Senior #2 :
- Implement `LlmRouterClient::call(LlmCallRequest)` :
  - Render prompt via `efe-prompts::PromptRegistry` (no string concat).
  - Build `efe-llm-router::RouterRequest` from `LlmCallRequest`.
  - Call `LLMRouter::route(...)` (existing M21 API).
  - Parse response : if `ResponseFormat::JsonSchema`, attempt
    `serde_json::from_str` on the response text.
  - Map `RouterResponse` → `LlmCallResponse`.
- 5 unit tests with `MockLlmClient` + 2 integration tests with real
  M21 router pointing at a sandbox provider.

Senior #1 :
- Create `efe-nano/src/runtime/pii_redactor.rs`
- Adapter wrapping `efe-pii::Redactor`
- Method : `async fn redact(&self, input: &serde_json::Value) → serde_json::Value`
- Walk the JSON tree, redact string fields via M20 (PII tokens become
  `[REDACTED:EMAIL]`, `[REDACTED:PHONE]`, etc.).
- 5 unit tests : email redaction, phone redaction, idempotency
  (redact twice = once), nested objects, arrays.

Founder :
- Implement `summarize_strict` Mode C handler shape :
  - Input : `{ "text": "...", "max_sentences": 3 }`
  - Output : `{ "summary": "...", "sentence_count": 3 }`
- Wire to `LlmClient::call()` via the not-yet-existing Mode C executor
  (use mocks for now).
- 4 tests : valid input → mock returns valid output, mock returns bad
  JSON → SchemaMismatch error, max_sentences out of range → input
  schema error, empty text → input schema error.

#### Day 3 (Wed Aug 28)
Senior #2 :
- Create `efe-nano/src/runtime/output_validator.rs`
- Strict JSON schema enforcement on LLM output :
  - First pass : `serde_json::from_str` (raw → Value).
  - Second pass : compile schema once, validate against draft 2020-12.
  - Third pass : "structural reflexes" (e.g. enum values exact match,
    required fields present, no extra keys if `additionalProperties:
    false`).
- 6 tests : valid JSON + matches schema, valid JSON but wrong shape,
  invalid JSON, additional properties rejected when forbidden,
  enum mismatch, deeply nested schema.

Senior #1 :
- Create `efe-nano/src/runtime/llm_augmented.rs`
- `LlmAugmentedRuntime::execute_with_context(agent, input, ctx)` :
  1. Validate input vs agent.input_schema.
  2. PII redact via `PiiRedactor::redact(input)` if
     `agent.compliance.pii_redaction_strategy != PiiStrategy::None`.
  3. `budget.pre_check(tenant, estimated_cost)` → reservation.
  4. Build `LlmCallRequest` from agent metadata + redacted input.
  5. `llm_client.call(request)` → response.
  6. Strict `OutputValidator::validate(response.content, agent.output_schema)`.
  7. `budget.commit(reservation, response.cost_cents)`.
  8. Build `NanoAgentResponse` + record audit trace.
- 5 unit tests (with mocks).

Founder :
- Implement 4 remaining Mode C handlers : `extract_structured`,
  `classify_topic_llm`, `sentiment_nuanced`, `qa_grounded`.
- Each handler is just the manifest — `LlmAugmentedRuntime` does all
  the heavy lifting based on `agent.execution_mode.LlmAugmented`.

#### Day 4 (Thu Aug 29)
All :
- Wire `LlmAugmentedRuntime` into `MultiModeRuntime`.
- Integration test : 10-op DAG mixing Mode A + Mode B + Mode C
  (e.g. pdf_extract → ner_clinical → summarize_strict).
- Test : LLM returns malformed JSON → op marked Failed with
  `SchemaValidationFailed`, downstream ops Skipped.
- Test : Budget insufficient → op marked Failed with
  `BudgetExceeded`, dispatcher returns early.

#### Day 5 (Fri Aug 26)
Sprint 5 mid-point checkpoint.
- All 3 components (LlmClient, BudgetEnforcer, PiiRedactor) shipped.
- 5 Mode C agents wired end-to-end (with mocks for CI ; sandbox
  provider for staging).
- 30 new tests added cumulative for Sprint 5 mid-point.
- Demo to the team : Mode C extract_structured on 5 sample PDFs.

#### Day 6 (Mon Sep 1)
Senior #2 :
- Add response-format tuning : route OpenAI provider with `tools=[]`
  forcing JSON, Anthropic provider with `system_prompt_suffix` for
  JSON output, fallback to retry with stronger prompt if first attempt
  returns invalid JSON.
- 3 retry tests : 1-attempt success, 2-attempt success (first malformed),
  exhausted retries → Failed.

Senior #1 :
- Add cost cap reconciliation :
  - Pre-check uses `agent.cost_estimate_cents` (manifest-declared).
  - Post-charge uses `response.cost_cents` (provider-reported).
  - If post-charge > pre-check, log a warning + still commit.
- 4 tests : pre = post, pre > post (under-budget OK), pre < post
  (over-budget warning), commit then double-commit (error).

Founder :
- Edge case tests : empty LLM response, provider timeout, provider error,
  per-tenant provider preference (tenant A uses Anthropic, tenant B
  uses OpenAI).

#### Day 7-8 (Tue-Wed Sep 2-3)
All :
- Performance pass on Mode C path.
- Profile : prompt rendering ≤ 1 ms, LLM call ≥ 95 % of latency
  (acceptable — provider-bound), output validation ≤ 5 ms.
- Throughput on dev hardware : 5 calls/sec/node (target 10 on staging).
- Pool hit rate for `VmKind::LlmCaller` : ≥ 99 % (lightweight VMs).

#### Day 9 (Thu Sep 4)
Hardening :
- Cancellation : when `ctx.deadline` passes mid-LLM-call, send cancel
  via `tokio::time::timeout` on the call future.
- Re-run all M50 tests : Phase 1 + Sprint 4 + Sprint 5.

#### Day 10 (Fri Sep 5)
Sprint 5 close :
- 60 new tests added (cumulative Sprint 5 : ~ 60).
- Cumulative M50 test count : 312 + 60 = 372.
- Sprint 5 retro.
- Sprint 6 kickoff prep (Senior #1 takes the audit chain lead).

### 5.3 Mode C handler shape (canonical)

```rust
// Example : extract_structured.rs

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("extract_structured").unwrap(),
        version: NanoAgentVersion::new("1.0.0").unwrap(),
        description: "LLM-based structured extraction with strict JSON schema.".to_string(),
        author: Author::Efe,
        category: Category::StructuredExtract,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            prompt_template_id: "m50.extract_structured.v1".to_string(),
            max_tokens: 2_000,
            temperature: 0.1,        // low temperature for structured output
            providers_allow_list: vec![
                "anthropic:claude-sonnet-4-5".to_string(),
                "openai:gpt-5".to_string(),
            ],
        },
        determinism: DeterminismLevel::Stochastic,
        idempotency: IdempotencyPolicy::Manual { ttl_sec: 3_600 },
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text", "fields"],
            "properties": {
                "text": { "type": "string", "maxLength": 50_000 },
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["name", "type"],
                        "properties": {
                            "name": { "type": "string", "pattern": "^[a-z_][a-z0-9_]*$" },
                            "type": { "enum": ["string", "number", "boolean", "date"] },
                            "description": { "type": "string" }
                        }
                    },
                    "minItems": 1,
                    "maxItems": 20
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["extracted"],
            "properties": {
                "extracted": { "type": "object" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(2_000),
            network_egress_kb: 50,
        },
        cost_estimate_cents: 2.0,        // ~ $0.02 per call typical
        latency_p99_ms_target: 5_000,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr_relevant: true,
            ai_act_high_risk: false,
            pii_redaction_strategy: PiiStrategy::PreLlmRedact,
            ..ComplianceTags::default()
        },
        mcp_tool_metadata: None,
        created_at: Utc::now(),
    }
}
```

### 5.4 Mode C execution flow (sequence)

```
Caller → Dispatcher → MultiModeRuntime.execute_with_context(agent, input, ctx)
                       │
                       └── Mode C path (Sprint 5) :
                            │
                            ├── 1. Validate input vs agent.input_schema (cached)
                            ├── 2. Check ctx.deadline
                            ├── 3. PiiRedactor.redact(input)
                            │       → redacted_input (e.g. emails replaced by tokens)
                            ├── 4. budget.pre_check(tenant, agent.cost_estimate_cents)
                            │       → ReservationId
                            │       OR → Err(BudgetInsufficient) early-return
                            ├── 5. Build LlmCallRequest from agent.execution_mode + redacted_input
                            ├── 6. llm_client.call(request) (via efe-llm-router M21)
                            │       │
                            │       ├── M21 selects provider (anthropic / openai / vertex...)
                            │       ├── M21 checks cost governor + circuit breaker
                            │       ├── M21 hits cache if applicable
                            │       ├── M21 calls provider
                            │       └── M21 returns RouterResponse
                            │
                            ├── 7. OutputValidator.validate(response.content, agent.output_schema)
                            │       → strict JSON schema match (or retry once with stronger prompt)
                            ├── 8. budget.commit(reservation, response.cost_cents)
                            ├── 9. Build NanoAgentResponse (with provider+model in metadata)
                            └── 10. audit.record(NanoAgentTrace {
                                       execution_mode: "llm_augmented",
                                       llm_tokens_used: Some(response.tokens_in + tokens_out),
                                       cost_cents: response.cost_cents,
                                       ...
                                   })
```

### 5.5 Sprint 5 acceptance criteria

```
[ ] LlmClient trait + LlmRouterClient + MockLlmClient shipped
[ ] OutputValidator with strict JSON schema enforcement
[ ] BudgetEnforcer trait + InMemoryBudgetEnforcer stub
[ ] PiiRedactor adapter wrapping efe-pii (M20)
[ ] LlmAugmentedRuntime + MultiModeRuntime dispatch Mode A/B/C
[ ] 5 Mode C starter agents : summarize_strict, extract_structured,
    classify_topic_llm, sentiment_nuanced, qa_grounded
[ ] Mode C throughput ≥ 5 calls/sec on dev (target 10 on staging)
[ ] Pool hit rate for LlmCaller VMs ≥ 99 %
[ ] 60 new tests pass cumulative Sprint 5
[ ] 174 PR #27 + 51 STABILITY + 277 Phase 1 toujours verts
[ ] cumulative test count : 372 M50 tests
```

---

## 6. Sprint 6 — Audit Trace + Replay (Day-by-day)

**Dates** : Sep 10-24, 2026 (10 working days)
**Team** : full team
**Goal** : Ship HMAC-chained audit trace + replay engine. Demo : replay
1,000 historical swarms with 100 % determinism match on Mode A ops.

### 6.1 Sprint 6 entry conditions

```
[ ] Sprint 5 closed cleanly :
    [✓] 372 M50 tests green
    [✓] Mode C wired end-to-end
[ ] Postgres staging DB ready, EFE-NANO-AUDIT-HMAC-MASTER env var set
[ ] efe-curation (M23) HMAC chain primitives reviewed (sample code +
    test fixtures available)
[ ] DPO + counsel : audit retention policy locked per tier (Phase 2
    uses : 7d free, 30d dev, 1y pro, 7y enterprise stub — full retention
    enforcement Phase 4)
[ ] Decision : audit chain genesis row format locked (Sprint 6 Day 1)
```

### 6.2 Day-by-day plan

#### Day 1 (Mon Sep 10)
**Morning** : Sprint 6 kickoff meeting (2 h).
- Review audit chain design vs M23 implementation.
- Confirm SQL schema (cf § 7.1).
- Hand-off : Senior #1 leads `hmac_chain.rs` + `postgres.rs` + `verify.rs` ;
  Senior #2 leads `replay/` module ; Founder leads tests + 5 starter
  agent tweaks (add `compliance.replayable: bool` where applicable).

**Afternoon** :

Senior #1 :
- Create migration `v004_create_nano_agent_traces.sql` (cf § 7.1).
- Implement `PostgresAuditWriter` skeleton in `efe-nano/src/audit/postgres.rs`.
- 3 tests : connection lifecycle, INSERT + SELECT round-trip,
  per-tenant isolation.

Senior #2 :
- Create `efe-nano/src/replay/mod.rs` with `ReplayEngine` skeleton.
- Stub `replay(tenant, request_id)` (returns hardcoded report).
- 2 unit tests : engine constructable, replay returns NotFound for
  unknown request_id.

Founder :
- Add `replayable: bool` flag to `ComplianceTags`. Default `true` for
  Mode A (FullDeterministic), `true` for Mode B (ProbabilisticFlavor —
  same model + same seed produces same output), `false` for Mode C
  (LLM stochastic).
- Update 20 starter agent manifests with explicit `replayable` value.

#### Day 2 (Tue Sep 11)
Senior #1 :
- Implement HKDF-based key derivation (cf § 3.5 `HmacKeySource::EnvHkdf`).
- 4 tests : key deterministic, different tenants → different keys,
  missing env var → error, key length is 32 bytes.

Senior #2 :
- Implement `SwarmReconstructor::reconstruct(traces) → SwarmRequest`.
  Given a Vec of traces (sorted by `started_at`), rebuild the
  original `SwarmRequest` from the recorded inputs and op metadata.
- 5 tests : 1-op reconstruction, 10-op DAG with deps, missing trace,
  out-of-order traces, edge case (input_hash collision impossible
  with SHA-256).

Founder :
- Add a "deterministic seed" hash to `NanoAgentTrace`. For Mode B, the
  inference random seed (if any) is captured ; for Mode A it's
  unused. This lets Sprint 6 Day 7-8 verify Mode B replay where
  applicable.

#### Day 3 (Wed Sep 12)
Senior #1 :
- Implement `HmacChainAuditWriter::record(trace)` (cf § 3.5).
- Wire it to write inside a transaction :
  ```sql
  BEGIN;
    SELECT trace_id FROM nano_agent_traces
      WHERE tenant_id = $1 ORDER BY started_at DESC LIMIT 1 FOR UPDATE;
    INSERT INTO nano_agent_traces (...) VALUES (...);
  COMMIT;
  ```
- 4 tests : single insert, sequential inserts form chain, concurrent
  inserts serialize correctly (no duplicate chain_parent_id), insert
  failure does not corrupt chain.

Senior #2 :
- Implement `DeterminismChecker::compare(original_hash, replay_hash)
  → ReplayOpStatus`.
- For FullDeterministic agents : `Match` if hashes equal, `Mismatch`
  otherwise.
- For ProbabilisticFlavor : `Match` if hashes equal OR if a
  per-agent-defined approximation matches (Phase 2 : exact match only ;
  approximation defer Sprint 12).
- For Stochastic : always `SkippedNonDeterministic`.
- 4 tests covering all combinations.

#### Day 4 (Thu Sep 13)
Senior #1 :
- Implement `verify_chain(tenant_id, until)` :
  - Walk traces in order, recompute HMAC per row, compare.
  - Return `Ok(count)` if all verified, `Err(ChainBroken { trace_id })`
    or `Err(HmacMismatch { trace_id })` on first failure.
- 6 tests : empty chain, 10-row chain valid, broken parent_id link,
  HMAC mismatch detected, single-row tenant, chain spans 1k rows.

Senior #2 :
- Wire `ReplayEngine::replay(tenant, request_id)` :
  1. Load all traces for `request_id` (ORDER BY started_at ASC).
  2. Call `verify_chain` partial walk for these traces (sub-chain
     verification).
  3. Call `SwarmReconstructor::reconstruct(traces)` → SwarmRequest.
  4. Call `dispatcher.dispatch(reconstructed_request)`.
  5. For each op, compare `original_output_hash` vs new output hash.
  6. Build `ReplayReport`.
- 4 integration tests : single op replay, 10-op DAG, mixed
  modes (skip non-deterministic), failed dispatch.

Founder :
- Add 5 manifest tweaks : agents that should always be replayable.

#### Day 5 (Fri Sep 14)
Sprint 6 mid-point.
- Audit chain : write + verify end-to-end demo.
- Replay : single + 10-op DAG end-to-end demo.
- 30 new tests added cumulative Sprint 6.

#### Day 6 (Mon Sep 17)
Senior #1 :
- Performance pass on audit writer.
- Batching : group N traces in a single INSERT (default N=10, configurable).
- Background flush task : if buffer not full, flush every 100 ms.
- Target : ≥ 5k writes/sec on staging.
- 4 perf tests + 3 chaos tests (Postgres down → degrade gracefully ;
  buffer full → block briefly ; large traces — 100 KB input_hash
  payloads).

Senior #2 :
- Replay performance pass.
- Parallel replay : run 10 replays concurrently via `tokio::JoinSet`.
- Target : 100 swarms/sec replay throughput on dev.
- 3 perf tests.

#### Day 7-8 (Tue-Wed Sep 18-19)
All :
- 1,000-swarm determinism replay test.
- Generate 1,000 synthetic swarms (mix of Mode A only) via property-
  based test harness (proptest).
- Execute them. Record traces.
- Replay each. Assert 100 % match.
- Add to integration tests in `tests/phase_2_replay_corpus.rs`.

#### Day 9 (Thu Sep 20)
Senior #1 :
- Add `replay/cli.rs` admin command : `efe-nano-admin audit replay
  --request-id <uuid> --output report.json`.
- Tests : CLI argument parsing, error cases, JSON output format.

Senior #2 + Founder :
- Documentation update : audit chain section in `EFE_M50_AGENTIC_ARCHITECTURE.md`
  (just additive section).

#### Day 10 (Fri Sep 21)
Sprint 6 close :
- 90 new tests added cumulative Sprint 6.
- Cumulative M50 test count : 372 + 90 = 462.
- 1,000-swarm replay corpus : 100 % match on FullDeterministic, 100 % match
  on ProbabilisticFlavor (with cached seed), 100 % SkippedNonDeterministic
  on LLM-augmented.
- Sprint 6 retro + Phase 2 close.

### 6.3 Sprint 6 acceptance criteria

```
[ ] HmacChainAuditWriter shipped (HKDF key + Postgres write + chain link)
[ ] PostgresAuditWriter wired to dispatcher via Arc<dyn AuditWriter>
[ ] verify_chain(tenant, until) correctly detects breakage + HMAC mismatch
[ ] SwarmReconstructor rebuilds SwarmRequest from traces
[ ] DeterminismChecker classifies Match / Mismatch / Skipped
[ ] ReplayEngine::replay returns valid ReplayReport
[ ] CLI : efe-nano-admin audit replay --request-id works
[ ] Audit chain write throughput ≥ 5k writes/sec on staging
[ ] Replay throughput ≥ 100 swarms/sec on dev
[ ] 1k-swarm determinism test : 100 % match on deterministic ops
[ ] 90 new tests pass cumulative Sprint 6
[ ] 174 PR #27 + 51 STABILITY + 277 Phase 1 + 372 Sprint 4-5 toujours verts
[ ] cumulative test count : 462 M50 tests
[ ] Phase 2 exit gate : signed off by founder + 2 seniors
```

---

## 7. Postgres schema additions (v004-v007)

Phase 2 introduces 4 new Postgres migrations. All migrations are
forward-only and idempotent (via `IF NOT EXISTS`).

### 7.1 Migration v004 — `nano_agent_traces`

```sql
-- v004_create_nano_agent_traces.sql
-- Migration : create the audit trace table for M50 nano agent calls.

CREATE TABLE IF NOT EXISTS nano_agent_traces (
    trace_id            UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,

    -- Parent trace (for swarm/op nesting). Different from chain_parent.
    parent_trace_id     UUID         REFERENCES nano_agent_traces(trace_id),
    -- HMAC chain parent (latest trace for this tenant when this row inserted).
    chain_parent_id     UUID         REFERENCES nano_agent_traces(trace_id),

    swarm_request_id    UUID         NOT NULL,
    operation_id        TEXT         NOT NULL,
    caller_agent_id     TEXT         NULL,

    nano_agent_id       TEXT         NOT NULL,
    nano_agent_version  TEXT         NOT NULL,
    execution_mode      TEXT         NOT NULL CHECK (execution_mode IN ('pure_rust', 'local_model', 'llm_augmented')),

    input_hash          BYTEA        NOT NULL,                  -- SHA-256 32 bytes
    output_hash         BYTEA        NULL,                       -- NULL on error
    error_kind          TEXT         NULL,

    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NULL,
    latency_ms          INTEGER      NULL,
    cost_cents          DOUBLE PRECISION NOT NULL DEFAULT 0,
    llm_tokens_used     INTEGER      NULL,

    -- HMAC chain fields
    payload_hash        BYTEA        NOT NULL,                  -- SHA-256 of canonical row body
    hmac                BYTEA        NOT NULL,                  -- HMAC-SHA-256(chain_parent_id || payload_hash, key)

    -- Metadata
    audit_tags          TEXT[]       NOT NULL DEFAULT '{}'::TEXT[],
    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Row-level security : per-tenant isolation.
ALTER TABLE nano_agent_traces ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON nano_agent_traces
    USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));
```

### 7.2 Migration v005 — chain indexes

```sql
-- v005_create_traces_chain_indexes.sql

-- Primary query : list traces for a tenant in time order (chain walk).
CREATE INDEX IF NOT EXISTS traces_tenant_started_idx
    ON nano_agent_traces (tenant_id, started_at DESC);

-- Replay : fetch all traces for a swarm_request_id.
CREATE INDEX IF NOT EXISTS traces_swarm_idx
    ON nano_agent_traces (swarm_request_id);

-- Chain integrity : navigate via chain_parent_id.
CREATE INDEX IF NOT EXISTS traces_chain_parent_idx
    ON nano_agent_traces (chain_parent_id) WHERE chain_parent_id IS NOT NULL;

-- Searchable by audit_tags (GIN for array containment).
CREATE INDEX IF NOT EXISTS traces_audit_tags_gin
    ON nano_agent_traces USING GIN (audit_tags);

-- Per-agent analytics.
CREATE INDEX IF NOT EXISTS traces_nano_agent_started_idx
    ON nano_agent_traces (nano_agent_id, started_at DESC);
```

### 7.3 Migration v006 — `replay_attempts`

```sql
-- v006_create_replay_attempts.sql
-- Records each replay invocation for audit + analytics.

CREATE TABLE IF NOT EXISTS replay_attempts (
    replay_id           UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,

    -- Swarm being replayed.
    original_request_id UUID         NOT NULL,
    triggered_by        TEXT         NOT NULL,                   -- 'admin', 'customer', 'regulator'
    triggered_by_actor  TEXT         NULL,

    -- Replay outcome.
    chain_verified      BOOLEAN      NOT NULL,
    total_ops           INTEGER      NOT NULL,
    matched_ops         INTEGER      NOT NULL,
    mismatched_ops      INTEGER      NOT NULL,
    skipped_ops         INTEGER      NOT NULL,

    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NOT NULL,
    duration_ms         INTEGER      NOT NULL,

    -- Full report stored as JSONB for debugging.
    report_jsonb        JSONB        NOT NULL,

    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS replay_attempts_tenant_started_idx
    ON replay_attempts (tenant_id, started_at DESC);
CREATE INDEX IF NOT EXISTS replay_attempts_original_idx
    ON replay_attempts (original_request_id);
```

### 7.4 Migration v007 — chain integrity constraints

```sql
-- v007_constraints_traces_chain_fk.sql
-- Hardening : ensure chain parent FK enforces same-tenant invariant.

-- Trigger : reject INSERT where chain_parent_id is from a different
-- tenant. (Prevents cross-tenant chain poisoning.)
CREATE OR REPLACE FUNCTION enforce_chain_parent_tenant()
RETURNS TRIGGER AS $$
DECLARE
    parent_tenant UUID;
BEGIN
    IF NEW.chain_parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT tenant_id INTO parent_tenant
      FROM nano_agent_traces
     WHERE trace_id = NEW.chain_parent_id;
    IF parent_tenant IS NULL THEN
        RAISE EXCEPTION 'chain_parent_id % does not exist', NEW.chain_parent_id;
    END IF;
    IF parent_tenant != NEW.tenant_id THEN
        RAISE EXCEPTION 'chain_parent_id from different tenant : % vs %', parent_tenant, NEW.tenant_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER enforce_chain_parent_tenant_trigger
    BEFORE INSERT ON nano_agent_traces
    FOR EACH ROW
    EXECUTE FUNCTION enforce_chain_parent_tenant();
```

---

## 8. Error handling additions

Phase 2 extends `NanoAgentError` (Phase 1) with new variants for
Mode B/C + audit + replay failure modes :

```rust
// efe-nano/src/error.rs — Phase 2 ADDITIONS only

#[derive(Debug, thiserror::Error)]
pub enum NanoAgentError {
    // ... Phase 1 variants (unchanged) ...

    /// Mode B local model error.
    #[error("local model error : {0}")]
    LocalModel(#[from] crate::runtime::ModelInferError),

    /// Mode C LLM call error.
    #[error("LLM call error : {0}")]
    LlmCall(#[from] crate::runtime::LlmCallError),

    /// Mode B/C VM pool error.
    #[error("VM pool error : {0}")]
    VmPool(#[from] crate::runtime::VmPoolError),

    /// Audit chain error (write or verify).
    #[error("audit chain error : {0}")]
    AuditChain(#[from] crate::audit::AuditChainError),

    /// Replay engine error.
    #[error("replay error : {0}")]
    Replay(#[from] crate::replay::ReplayError),
}
```

The error → `OperationError` mapping (for surface to SwarmResponse) is
extended :

```rust
impl From<&NanoAgentError> for OperationError {
    fn from(err: &NanoAgentError) -> Self {
        let kind = match err {
            // Phase 1 variants...
            NanoAgentError::LocalModel(_) => OperationErrorKind::RuntimeError,
            NanoAgentError::LlmCall(LlmCallError::BudgetInsufficient { .. }) => {
                OperationErrorKind::BudgetExceeded
            }
            NanoAgentError::LlmCall(_) => OperationErrorKind::RuntimeError,
            NanoAgentError::VmPool(VmPoolError::AcquireTimeout(_)) => OperationErrorKind::Timeout,
            NanoAgentError::VmPool(_) => OperationErrorKind::ResourceExceeded,
            NanoAgentError::AuditChain(_) => OperationErrorKind::InternalError,
            NanoAgentError::Replay(_) => OperationErrorKind::InternalError,
        };
        OperationError::new(kind, err.to_string())
    }
}
```

---

## 9. Performance contracts Phase 2

### 9.1 Latency targets (per call)

| Operation | p50 | p99 | Phase 1 baseline |
|-----------|-----|-----|-------------------|
| Mode A nano agent | < 10 ms | < 50 ms | ✓ inherited |
| Mode B nano agent | < 80 ms | < 200 ms | NEW |
| Mode B cold start (pool miss) | < 200 ms | < 500 ms | NEW |
| Mode C nano agent (LLM) | < 1,500 ms | < 5,000 ms | NEW |
| Mode C cold start | < 50 ms | < 200 ms | NEW |
| Audit chain write | < 10 ms | < 30 ms | NEW |
| Audit chain verify (1k rows) | < 100 ms | < 500 ms | NEW |
| Replay (10-op swarm) | < 200 ms | < 800 ms | NEW |
| Replay throughput | 100 swarms/s | 50 swarms/s | NEW |

### 9.2 Throughput targets (per node)

| Metric | Target Sprint 6 | Target Phase 4 | Note |
|--------|------------------|------------------|------|
| Mode A calls/sec | 1,000 | 5,000 | inherited Phase 1 |
| Mode B calls/sec | 100 | 500 | bottlenecked by model inference |
| Mode C calls/sec | 10 | 50 | bottlenecked by LLM provider |
| Audit chain writes/sec | 5,000 | 20,000 | Postgres + batching |
| Replay swarms/sec | 100 | 1,000 | parallel JoinSet |

### 9.3 Pool hit rate targets

| Pool kind | Target hit rate | Capacity |
|-----------|------------------|----------|
| Mode B (ModelInference) | ≥ 95 % at 100 cps | 20 warm VMs, scale to 200 |
| Mode C (LlmCaller) | ≥ 99 % at 10 cps | 50 warm VMs, scale to 500 |

### 9.4 Cost model (per call)

| Mode | Compute cost | LLM cost | Audit cost | Total ~ |
|------|---------------|----------|-------------|----------|
| Mode A | $0.0001 | $0 | $0.00005 | ~ $0.0002 |
| Mode B | $0.001 | $0 | $0.00005 | ~ $0.001 |
| Mode C | $0.002 | passthrough ($0.01-0.05) | $0.0001 | ~ $0.02 |

Pricing margin (per call) at Developer tier ($0.001/call) :
- Mode A : ~ 5x margin
- Mode B : ~ 1x margin (push to Pro tier)
- Mode C : negative on Developer tier (use Pro+)

---

## 10. Integration matrix avec crates EFE existants

Phase 2 consumes more EFE crates than Phase 1 (which only used
`efe-nano-sdk` internally). The matrix below lists each crate, what
Phase 2 uses from it, and confirms **no modifications**.

| Crate | Phase 2 usage | Modifications required |
|-------|----------------|-------------------------|
| `efe-nano-sdk` | Public types (already shipped) | **AUCUNE** |
| `efe-llm-router` (M21) | `LLMRouter::route()` for Mode C | **AUCUNE** — wrapped in `LlmRouterClient` |
| `efe-prompts` (M21) | `PromptRegistry::render()` for templates | **AUCUNE** — read-only consumer |
| `efe-curation` (M23) | HMAC chain pattern (referenced, not called) | **AUCUNE** — pattern duplicated for nano traces, NO FK to curation tables |
| `efe-agent-runner` (M25) | Firecracker microVM provisioning | **AUCUNE** — wrapped in `FirecrackerVmPool` |
| `efe-cache` (M27) | LlmCache passthrough via M21 | **AUCUNE** — used transitively via M21 |
| `efe-pii` (M20) | `Redactor::redact()` pre-LLM | **AUCUNE** — wrapped in `PiiRedactor` |
| `efe-events` (M28) | Emit `nano-agent-completed` event | **AUCUNE** — new event variant, schema extensible |
| `efe-metering` (M31) | Track `nano_agent_calls` meter | **AUCUNE** — new meter variant (see § 10.1) |
| `efe-billing` (M31) | `BudgetEnforcer` trait (stub in Phase 2) | **AUCUNE** — real integration Sprint 11 |
| `efe-meta-swarm` (M14, M25) | NOT used in Phase 2 | — |
| `bench/harness` | CB-50_nano_local_model_throughput, CB-50_nano_llm_augmented_throughput | **ADDS** scenarios |
| `STABILITY_CONTRACT.md` | § 30 extension for Phase 2 surface | **ADDS** sub-sections, § 1-29 unchanged |

**Verdict additivité** : ✓ all existing crates remain **byte-identical**.

### 10.1 `efe-metering` Meter variant (Phase 2 ADDS)

Phase 2 needs a new meter variant for nano agent call billing. This
requires touching `efe-metering` BUT only via additive enum extension
(no existing variant modified). Note : this is the ONE exception to
"zero modification" — adding a new enum variant is unavoidable for
billing integration.

```rust
// crates/efe-metering/src/meter.rs — ADDITIVE diff

@@ existing ClassifyRequests, AgentRunSteps, ... @@

  pub enum Meter {
      ClassifyRequests,
      AgentRunSteps,
      AgentRunSeconds,
      MultimodalVisionRequests,
      MultimodalAudioSeconds,
      WebhookDeliveries,
      BatchClassifiedRows,
      AuditExportGb,
+     // M50 Phase 2 :
+     NanoAgentCalls,         // Mode A + B + C calls (all modes)
+     NanoAgentLlmTokens,     // tokens consumed by Mode C (for token-cost billing)
+     NanoAgentVmSeconds,     // microVM wall-clock for Mode B/C (compute billing)
  }
```

Adding variants does NOT break consumers : existing match-arms keep
working (Rust enum exhaustiveness applies only to `match` without
fallback). Tests in `efe-billing::pricing::tests::all_meters_priced`
get 3 new rows.

**Decision** : Senior #1 ships this in Sprint 5 Day 5 (~ 30 min work)
via a separate small PR, paired with `efe-billing`'s pricing table
update. Reviewed by founder + Senior #2.

---

## 11. CI / build integration Phase 2

### 11.1 Workspace `Cargo.toml` deltas

```toml
# Phase 2 ADDS the following dependencies to crates/efe-nano/Cargo.toml :

[dependencies]
# Existing Phase 1 deps unchanged ...

# Mode B :
tract-onnx       = "0.21"            # ONNX inference engine (pure Rust)
hkdf             = "0.12"            # HKDF for HMAC key derivation
hmac             = "0.12"
hex              = "0.4"

# Sprint 6 :
sqlx             = { workspace = true, features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "json"] }
```

### 11.2 CI matrix expansion

Phase 1 CI ran : `cargo build`, `cargo test`, `cargo clippy -- -D
warnings` per-crate. Phase 2 adds :

```yaml
# .github/workflows/m50-phase-2.yml
name: M50 Phase 2 CI

on:
  pull_request:
    paths:
      - 'crates/efe-nano/**'
      - 'crates/efe-nano-sdk/**'

jobs:
  build:
    runs-on: ubuntu-latest-large
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_PASSWORD: ci
        ports: [5432:5432]
    steps:
      - uses: actions/checkout@v4
      - run: cargo test -p efe-nano-sdk
      - run: cargo test -p efe-nano --lib
      - run: cargo test -p efe-nano --test phase_1_integration
      - run: cargo test -p efe-nano --test phase_2_mode_b
        env:
          EFE_TEST_FIRECRACKER_DISABLED: "1"   # CI uses InMemoryVmPool
      - run: cargo test -p efe-nano --test phase_2_mode_c
        env:
          EFE_TEST_LLM_DISABLED: "1"            # CI uses MockLlmClient
      - run: cargo test -p efe-nano --test phase_2_audit_chain
        env:
          DATABASE_URL: postgres://postgres:ci@localhost/test
          EFE_NANO_AUDIT_HMAC_MASTER: "ci-test-master-32-bytes-fixed-xyz"
      - run: cargo test -p efe-nano --test phase_2_replay
      - run: cargo clippy -p efe-nano-sdk -p efe-nano --all-targets -- -D warnings
      - run: cargo test --workspace --exclude efe-bench-harness   # baseline preservation
```

### 11.3 Test naming convention

Phase 2 tests are organized into 4 integration test files :

```
crates/efe-nano/tests/
├── phase_1_integration.rs       (existing, 9 tests, unchanged)
├── phase_2_mode_b.rs            (NEW Sprint 4, ~ 15 tests)
├── phase_2_mode_c.rs            (NEW Sprint 5, ~ 20 tests)
├── phase_2_audit_chain.rs       (NEW Sprint 6, ~ 25 tests, gated by DATABASE_URL)
└── phase_2_replay.rs            (NEW Sprint 6, ~ 15 tests, gated by DATABASE_URL)
```

Plus unit tests inline in each module.

### 11.4 Performance gate in CI

Sprint 6 Day 6 adds a CI performance gate that runs a 60-second
throughput benchmark and fails if :
- Mode A < 1,000 calls/sec
- Mode B < 50 calls/sec (lower bar for CI ; staging hits 100)
- Audit writes < 1,000/sec (lower bar for CI ; staging hits 5k)

The gate is **soft** initially (warn-only) and becomes **hard**
(fail-the-build) at Sprint 6 Day 10.

---

## 12. Phase 2 → Phase 3 gate criteria

These are the **must-be-true conditions** before opening Phase 3
(Sprint 7). Each item is a CI-checkable gate.

```
[ ] Mode A + B + C runtimes all operational                  (functional)
[ ] 20 starter pack agents (10 Phase 1 + 5 Mode B + 5 Mode C) (functional)
[ ] Mode A throughput ≥ 1,000 calls/sec/node                 (perf)
[ ] Mode B throughput ≥ 100 calls/sec/node on staging        (perf)
[ ] Mode C throughput ≥ 10 calls/sec/node on staging         (perf)
[ ] Audit chain write throughput ≥ 5k/sec on staging          (perf)
[ ] Replay : 1k swarms 100% match on FullDeterministic        (correctness)
[ ] Replay : 1k swarms 100% match on ProbabilisticFlavor      (correctness)
[ ] Replay : 100% skipped on Stochastic (Mode C)              (correctness)
[ ] HMAC chain verify on 10k traces : no false positives      (correctness)
[ ] 174 PR #27 + 51 STABILITY + 277 Phase 1 toujours verts    (additivity)
[ ] cumulative M50 tests ≥ 460                                (coverage)
[ ] cargo clippy -- -D warnings : zero lints                  (hygiene)
[ ] all Postgres migrations v001-v007 apply cleanly forward   (schema)
[ ] DPO sign-off on audit retention policy per tier           (compliance)
[ ] Founder + 2 seniors retro : no critical open issues       (process)
```

Gate is **opened** by the founder after all items checked. Phase 3
Sprint 7 cannot start until gate is open.

---

## 13. Test catalogue Phase 2 (~ 185 new tests)

### 13.1 Sprint 4 tests (~ 35)

| File | Test count | Purpose |
|------|-----------|---------|
| `runtime/model_loader.rs::tests` | 8 | ModelLoader trait surface, load success/failure, idempotent get_or_load, warm, health |
| `runtime/pool/mod.rs::tests` | 9 | VmPool trait, acquire, lease drop, kind isolation, scaling, health |
| `runtime/pool/firecracker.rs::tests` | 5 | Real Firecracker integration (gated by `EFE_TEST_FIRECRACKER_DISABLED`) |
| `runtime/multi_mode.rs::tests` | 4 | Mode A + B dispatch, Mode C returns NotYetImpl |
| `starter_pack/ner_clinical.rs::tests` | 3 | Manifest + handler with mock model |
| `starter_pack/ner_legal.rs::tests` | 2 | Manifest + handler |
| `starter_pack/intent_classify_generic.rs::tests` | 2 | Manifest + handler |
| `starter_pack/lang_detect.rs::tests` | 2 | Manifest + handler |
| `starter_pack/embed_text.rs::tests` | 2 | Manifest + handler |
| `tests/phase_2_mode_b.rs` | 15 | End-to-end Mode B via dispatcher, 50-op DAG, pool hit rate |

**Subtotal Sprint 4** : ~ 52 tests (target was 35 ; expanded during sprint).

### 13.2 Sprint 5 tests (~ 60)

| File | Test count | Purpose |
|------|-----------|---------|
| `runtime/llm_client.rs::tests` | 8 | LlmClient trait, MockLlmClient, error mapping |
| `runtime/output_validator.rs::tests` | 10 | Strict JSON schema enforcement |
| `budget/mod.rs::tests` | 9 | BudgetEnforcer trait, pre_check/commit/release |
| `runtime/pii_redactor.rs::tests` | 7 | PII redaction strategies |
| `runtime/llm_augmented.rs::tests` | 6 | Mode C end-to-end via mocks |
| `starter_pack/summarize_strict.rs::tests` | 3 | Manifest + handler |
| `starter_pack/extract_structured.rs::tests` | 3 | Manifest + handler |
| `starter_pack/classify_topic_llm.rs::tests` | 2 | Manifest + handler |
| `starter_pack/sentiment_nuanced.rs::tests` | 2 | Manifest + handler |
| `starter_pack/qa_grounded.rs::tests` | 2 | Manifest + handler |
| `tests/phase_2_mode_c.rs` | 20 | End-to-end Mode C, mixed-mode DAG, budget enforcement |

**Subtotal Sprint 5** : ~ 72 tests.

### 13.3 Sprint 6 tests (~ 90)

| File | Test count | Purpose |
|------|-----------|---------|
| `audit/hmac_chain.rs::tests` | 10 | HKDF, HMAC compute, chain integrity, key length |
| `audit/postgres.rs::tests` | 12 | Insert, sequential chain, concurrent inserts, RLS |
| `audit/verify.rs::tests` | 6 | Chain walk, broken parent, HMAC mismatch, edge cases |
| `replay/determinism.rs::tests` | 6 | Match/Mismatch/Skipped classification |
| `replay/reconstructor.rs::tests` | 8 | SwarmRequest rebuild from traces |
| `replay/mod.rs::tests` | 10 | ReplayEngine end-to-end with stubs |
| `replay/cli.rs::tests` | 4 | CLI argument parsing |
| `tests/phase_2_audit_chain.rs` | 15 | E2E chain write + verify with real Postgres |
| `tests/phase_2_replay.rs` | 20 | E2E replay with 1k synthetic corpus |

**Subtotal Sprint 6** : ~ 91 tests.

### 13.4 Cumulative Phase 2 test catalogue

```
Phase 1 inherited        : 277 tests (188 lib + 9 integration + 73 SDK + 7 doctest)
Sprint 4 (Mode B)        : + 52 tests
Sprint 5 (Mode C)        : + 72 tests
Sprint 6 (Audit + Replay): + 91 tests
─────────────────────────────────────
Cumulative end Phase 2   : 492 tests
```

Plus the baseline preserved : 174 PR #27 + 51 STABILITY v17 + 280
efe-bench-harness OK = 505 baseline tests untouched.

**Grand total at end of Phase 2** : 492 M50 + 505 baseline = ~ 997
test results, all green.

---

## 14. Risk register specific to Phase 2

### 14.1 Critical risks (P0)

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Firecracker integration friction** : M25 API changes mid-sprint | Medium | High (blocks Sprint 4) | Sprint 4 Day 1 lock M25 v1.x ; if M25 changes, use existing pinned version from Cargo.lock |
| **Model inference too slow** : Phase 2 model choices miss latency target | Medium | High (blocks Sprint 4 gate) | Sprint 0 already validated 5 ONNX models on dev hardware ; if staging fails, drop to 3 fastest models for Sprint 4 deliverable |
| **LLM provider degradation** : during Sprint 5, provider API breaks | Low | High (blocks Sprint 5) | M21 circuit breaker fails over ; if persistent, Sprint 5 uses MockLlmClient for tests + freezes one provider as Sprint 5 reference |
| **HMAC chain race condition** : concurrent inserts produce duplicate chain_parent_id | Medium | Critical (data integrity) | Use `SELECT ... FOR UPDATE` inside transaction (Sprint 6 Day 3) ; chaos test 100 concurrent inserts |
| **Replay non-determinism leakage** : Mode B model gives different output for same input | Low | High (replay test fails) | Pin model RNG seed in `LocalModel::infer` ; document `DeterminismLevel` semantics |

### 14.2 Major risks (P1)

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **PII redaction false negatives** : email pattern misses a corner case | High | Medium (compliance risk) | Use efe-pii (M20) audited library ; do NOT roll own regex |
| **Budget pre-check too slow** : adds 50 ms to every Mode C call | Medium | Medium (perf miss) | InMemoryBudgetEnforcer ≤ 1 ms ; real M31 integration deferred Sprint 11 |
| **Postgres connection exhaustion** under audit writer load | Medium | Medium (audit gaps) | Connection pool ≥ 50, batched writes, degrade gracefully on pool exhausted |
| **Senior #2 onboarding gap** : new hire ramp-up slow | Medium | High (Sprint 4 delay) | Pre-Sprint week reserved for ramp-up ; pair-programming Days 1-3 |
| **Cargo build time bloat** from new deps (sqlx, tract-onnx) | High | Low (DX hit) | Use sccache in CI ; gate heavy deps behind feature flags |

### 14.3 Minor risks (P2)

| Risk | Mitigation |
|------|------------|
| Mode B model file size pushes Docker image > 2 GB | Multi-stage Docker build, separate model image |
| Test flakiness on concurrent VM acquire | Tokio test-util pause-time + deterministic scheduling |
| Audit chain table grows fast | Pruning + partitioning plan in Sprint 11 |
| Replay CLI UX poor | Defer polish to Sprint 13 ; functional ship in Sprint 6 |

### 14.4 Rollback plan per sprint

```
Sprint 4 rollback (if Mode B unshippable) :
  - Keep Phase 1 deliverables intact
  - Skip Mode B for Phase 2 GA, focus on Mode C + Audit
  - Mode B re-attempted in Phase 4 hardening

Sprint 5 rollback (if Mode C unshippable) :
  - Keep Sprint 4 Mode B
  - Mark 5 Mode C agents as "experimental" (TIER-EXPERIMENTAL)
  - Defer GA-grade Mode C to Sprint 8+

Sprint 6 rollback (if HMAC chain unshippable) :
  - Keep Phase 1 audit stub (ConsoleAuditWriter + InMemoryAuditWriter)
  - Postgres traces still written (but unchained)
  - Chain verification + Replay deferred to Sprint 12
```

---

## 15. Annexe A — Mode B model loader trait + Firecracker pool

### 15.1 `InMemoryModelLoader` implementation sketch

```rust
// efe-nano/src/runtime/model_loader.rs

use async_trait::async_trait;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tract_onnx::prelude::*;

pub struct InMemoryModelLoader {
    /// Map (model_id, version) → lazy-loaded model handle.
    models: DashMap<(String, String), Arc<OnceCell<Arc<dyn LocalModel>>>>,
    /// Where the .onnx files live on disk.
    model_dir: PathBuf,
    /// Loaded model count (for health reporting).
    loaded: Arc<std::sync::atomic::AtomicU32>,
}

impl InMemoryModelLoader {
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            models: DashMap::new(),
            model_dir,
            loaded: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }
}

#[async_trait]
impl ModelLoader for InMemoryModelLoader {
    async fn get_or_load(
        &self,
        model_id: &str,
        model_version: &str,
    ) -> Result<Arc<dyn LocalModel>, ModelLoaderError> {
        let key = (model_id.to_string(), model_version.to_string());
        let cell = self.models.entry(key.clone()).or_insert_with(|| Arc::new(OnceCell::new())).clone();

        // OnceCell guarantees at-most-one initialisation across concurrent calls.
        let model = cell.get_or_try_init(|| async {
            let path = self.model_dir.join(format!("{model_id}-{model_version}.onnx"));
            let onnx_model = tract_onnx::onnx()
                .model_for_path(&path)
                .map_err(|e| ModelLoaderError::LoadFailed(format!("{path:?} : {e}")))?
                .into_optimized()
                .map_err(|e| ModelLoaderError::LoadFailed(format!("optimize : {e}")))?
                .into_runnable()
                .map_err(|e| ModelLoaderError::LoadFailed(format!("runnable : {e}")))?;

            let handle: Arc<dyn LocalModel> = Arc::new(TractModel {
                id: model_id.to_string(),
                version: model_version.to_string(),
                family: family_for(model_id),
                inner: onnx_model,
            });
            self.loaded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok::<_, ModelLoaderError>(handle)
        }).await?;
        Ok(Arc::clone(model))
    }

    async fn warm(&self, models: &[(String, String)]) -> Result<(), ModelLoaderError> {
        for (id, v) in models {
            self.get_or_load(id, v).await?;
        }
        Ok(())
    }

    async fn health(&self) -> ModelLoaderHealth {
        ModelLoaderHealth {
            loaded_models: self.loaded.load(std::sync::atomic::Ordering::Relaxed),
            total_memory_mb: 0,                    // TODO Phase 4 : measure
            avg_load_ms: 0,                         // TODO Phase 4 : measure
        }
    }
}

struct TractModel {
    id: String,
    version: String,
    family: ModelFamily,
    inner: tract_onnx::prelude::SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>,
}

#[async_trait]
impl LocalModel for TractModel {
    async fn infer(&self, input: serde_json::Value) -> Result<serde_json::Value, ModelInferError> {
        // 1. Convert serde JSON to tract Tensor (family-specific).
        // 2. Run inference.
        // 3. Convert output Tensor back to serde JSON.
        // Family-specific conversion in Sprint 4 Day 2-3.
        let _ = input;
        unimplemented!("Sprint 4 Day 2-3 — TBD")
    }

    fn id(&self) -> &str { &self.id }
    fn version(&self) -> &str { &self.version }
    fn family(&self) -> ModelFamily { self.family }
}

fn family_for(model_id: &str) -> ModelFamily {
    match model_id {
        "ner_clinical_v1" | "ner_legal_v1" => ModelFamily::NamedEntityRecognition,
        "intent_classify_v1" => ModelFamily::Classifier,
        "lang_detect_v1" => ModelFamily::LanguageDetector,
        "embedder_v1" => ModelFamily::Embedder,
        _ => ModelFamily::Generic,
    }
}
```

### 15.2 `FirecrackerVmPool` skeleton

```rust
// efe-nano/src/runtime/pool/firecracker.rs

use async_trait::async_trait;
use efe_agent_runner::FirecrackerHandle;       // existing M25 API
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, Mutex};

use super::{VmPool, VmKind, VmLease, VmPoolError, VmPoolHealth, PerKindHealth};

pub struct FirecrackerVmPool {
    inner: Arc<FirecrackerPoolInner>,
}

struct FirecrackerPoolInner {
    config: FirecrackerPoolConfig,
    warm: Mutex<std::collections::HashMap<VmKind, Vec<FirecrackerHandle>>>,
    permits_per_kind: std::collections::HashMap<VmKind, Arc<Semaphore>>,
    /// Sliding-window hit-rate counter.
    metrics: Mutex<PoolMetrics>,
}

#[derive(Debug, Clone)]
pub struct FirecrackerPoolConfig {
    pub mode_b_warm: u32,           // default 20
    pub mode_b_max: u32,            // default 200
    pub mode_c_warm: u32,           // default 50
    pub mode_c_max: u32,            // default 500
    pub provision_timeout: Duration,
    pub agent_runner_endpoint: String,
}

impl FirecrackerVmPool {
    pub async fn new(config: FirecrackerPoolConfig) -> Result<Self, VmPoolError> {
        let mut permits = std::collections::HashMap::new();
        permits.insert(VmKind::ModelInference, Arc::new(Semaphore::new(config.mode_b_max as usize)));
        permits.insert(VmKind::LlmCaller, Arc::new(Semaphore::new(config.mode_c_max as usize)));
        let pool = Self {
            inner: Arc::new(FirecrackerPoolInner {
                config: config.clone(),
                warm: Mutex::new(std::collections::HashMap::new()),
                permits_per_kind: permits,
                metrics: Mutex::new(PoolMetrics::default()),
            }),
        };
        // Pre-warm.
        pool.warmup().await?;
        Ok(pool)
    }

    async fn warmup(&self) -> Result<(), VmPoolError> {
        // Sprint 4 Day 6 implementation.
        Ok(())
    }
}

#[async_trait]
impl VmPool for FirecrackerVmPool {
    async fn acquire(
        &self,
        kind: VmKind,
        wait_budget: Duration,
    ) -> Result<VmLease, VmPoolError> {
        let start = Instant::now();
        let permit = self.inner.permits_per_kind.get(&kind)
            .ok_or_else(|| VmPoolError::Internal("unknown VmKind".into()))?;

        let owned_permit = tokio::time::timeout(wait_budget, permit.clone().acquire_owned())
            .await
            .map_err(|_| VmPoolError::AcquireTimeout(wait_budget))?
            .map_err(|_| VmPoolError::Internal("semaphore closed".into()))?;

        // Try to take a warm VM ; if none, provision one within budget.
        let handle = {
            let mut warm = self.inner.warm.lock().await;
            match warm.entry(kind).or_insert_with(Vec::new).pop() {
                Some(h) => {
                    self.inner.metrics.lock().await.record_hit(kind);
                    h
                }
                None => {
                    let elapsed = start.elapsed();
                    let remaining = wait_budget.checked_sub(elapsed).unwrap_or(Duration::ZERO);
                    self.inner.metrics.lock().await.record_miss(kind);
                    drop(warm);  // release lock before slow provisioning
                    self.provision_new_vm(kind, remaining).await?
                }
            }
        };

        Ok(VmLease {
            vm_id: handle.id(),
            kind,
            acquired_at: start,
            executor: Box::new(FirecrackerExecutor {
                handle,
                pool: Arc::clone(&self.inner),
                kind,
                _permit: owned_permit,
            }),
        })
    }

    async fn health(&self) -> VmPoolHealth {
        let metrics = self.inner.metrics.lock().await;
        let warm = self.inner.warm.lock().await;
        let mut kind_states = std::collections::HashMap::new();
        for &kind in &[VmKind::ModelInference, VmKind::LlmCaller] {
            kind_states.insert(kind, PerKindHealth {
                warm_slots: warm.get(&kind).map(|v| v.len() as u32).unwrap_or(0),
                in_use_slots: 0,            // TODO compute from permits
                max_slots: match kind {
                    VmKind::ModelInference => self.inner.config.mode_b_max,
                    VmKind::LlmCaller => self.inner.config.mode_c_max,
                },
                hit_rate_60s: metrics.hit_rate(kind),
                p99_provision_ms: metrics.p99_provision_ms(kind),
            });
        }
        VmPoolHealth { kind_states }
    }
}

impl FirecrackerVmPool {
    async fn provision_new_vm(
        &self,
        _kind: VmKind,
        _budget: Duration,
    ) -> Result<FirecrackerHandle, VmPoolError> {
        // Sprint 4 Day 3-4 : call efe-agent-runner to provision a new microVM
        // configured with the right model image (Mode B) or LLM HTTP client
        // image (Mode C).
        unimplemented!("Sprint 4 Day 3-4 — TBD")
    }
}

struct FirecrackerExecutor {
    handle: FirecrackerHandle,
    pool: Arc<FirecrackerPoolInner>,
    kind: VmKind,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[async_trait]
impl super::VmExecutor for FirecrackerExecutor {
    async fn execute(&mut self, _req: super::VmRequest) -> Result<super::VmResponse, super::VmExecError> {
        // Send via vsock, read response.
        // Sprint 4 Day 3-4.
        unimplemented!()
    }
}

#[derive(Default)]
struct PoolMetrics {
    hits: std::collections::HashMap<VmKind, u64>,
    misses: std::collections::HashMap<VmKind, u64>,
    provision_latencies: std::collections::HashMap<VmKind, Vec<u32>>,
}

impl PoolMetrics {
    fn record_hit(&mut self, kind: VmKind) { *self.hits.entry(kind).or_default() += 1; }
    fn record_miss(&mut self, kind: VmKind) { *self.misses.entry(kind).or_default() += 1; }
    fn hit_rate(&self, kind: VmKind) -> f32 {
        let h = *self.hits.get(&kind).unwrap_or(&0) as f32;
        let m = *self.misses.get(&kind).unwrap_or(&0) as f32;
        if h + m == 0.0 { 1.0 } else { h / (h + m) }
    }
    fn p99_provision_ms(&self, kind: VmKind) -> u32 {
        let mut v = self.provision_latencies.get(&kind).cloned().unwrap_or_default();
        if v.is_empty() { return 0; }
        v.sort_unstable();
        let idx = (v.len() as f32 * 0.99) as usize;
        v[idx.min(v.len() - 1)]
    }
}
```

---

## 16. Annexe B — Mode C LLM client + prompt rendering pipeline

### 16.1 `LlmRouterClient` implementation sketch

```rust
// efe-nano/src/runtime/llm_client.rs

use async_trait::async_trait;
use efe_llm_router::router::{LLMRouter, RouterRequest};
use efe_prompts::PromptRegistry;
use std::sync::Arc;

pub struct LlmRouterClient {
    router: Arc<LLMRouter>,
    prompts: Arc<PromptRegistry>,
}

impl LlmRouterClient {
    pub fn new(router: Arc<LLMRouter>, prompts: Arc<PromptRegistry>) -> Self {
        Self { router, prompts }
    }
}

#[async_trait]
impl LlmClient for LlmRouterClient {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError> {
        let start = std::time::Instant::now();

        // 1. Render prompt via efe-prompts.
        let rendered = self.prompts.render(&request.prompt_template_id, &request.template_inputs)
            .map_err(|e| LlmCallError::Internal(format!("prompt render : {e}")))?;

        // 2. Build router request.
        //    M21 RouterRequest needs : tenant_id, catalog, bundle_version,
        //    sentence_redacted, candidates, prompt_template_id, max_tokens,
        //    timeout_ms.
        //    Phase 2 uses a fixed `catalog="m50"` and bundle="1".  We pack
        //    the rendered prompt into `sentence_redacted` (despite the
        //    name — M21 will refactor in Phase 3 to a generic field).
        let req = RouterRequest {
            tenant_id: request.tenant_id.to_string(),
            catalog: "m50".to_string(),
            bundle_version: "1".to_string(),
            sentence_redacted: rendered,
            candidates: vec![],  // M50 doesn't use classification candidates
            prompt_template_id: request.prompt_template_id.clone(),
            max_tokens: request.max_tokens,
            timeout_ms: request.timeout_ms,
        };

        // 3. Route.
        let router_resp = self.router.route(req).await
            .map_err(|e| LlmCallError::Provider(e.to_string()))?;

        // 4. Parse response if JsonSchema mode.
        let content = match &request.response_format {
            ResponseFormat::FreeText => serde_json::Value::String(router_resp.text.clone()),
            ResponseFormat::JsonSchema(_) => {
                serde_json::from_str::<serde_json::Value>(&router_resp.text)
                    .map_err(|e| LlmCallError::InvalidJson(format!("{e} : raw={}", router_resp.text)))?
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
        // Rough heuristic Sprint 5 ; refined Sprint 7.
        // Assume $0.01 per 1k tokens input + $0.03 per 1k tokens output.
        let estimated_input_tokens = (request.template_inputs.to_string().len() / 4) as u32;
        let input_cost = estimated_input_tokens as f64 * 0.0001;
        let output_cost = request.max_tokens as f64 * 0.0003;
        (input_cost + output_cost) * 100.0  // → cents
    }
}
```

### 16.2 `OutputValidator` implementation

```rust
// efe-nano/src/runtime/output_validator.rs

use jsonschema::{Draft, JSONSchema};
use std::sync::Arc;

pub struct OutputValidator {
    /// Compiled schema cache keyed by schema-hash. JSONSchema compile
    /// is ~ 200 μs which adds up at 5k QPS.
    schema_cache: dashmap::DashMap<u64, Arc<JSONSchema>>,
}

impl OutputValidator {
    pub fn new() -> Self {
        Self { schema_cache: dashmap::DashMap::new() }
    }

    pub fn validate(
        &self,
        output: &serde_json::Value,
        schema: &serde_json::Value,
    ) -> Result<(), ValidationError> {
        let key = stable_hash(schema);
        let compiled = self.schema_cache.entry(key).or_try_insert_with(|| {
            JSONSchema::options()
                .with_draft(Draft::Draft202012)
                .compile(schema)
                .map(Arc::new)
                .map_err(|e| ValidationError::SchemaCompile(e.to_string()))
        })?.clone();

        if let Err(errors) = compiled.validate(output) {
            let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
            return Err(ValidationError::SchemaMismatch(msgs.join("; ")));
        }
        Ok(())
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

### 16.3 `PiiRedactor` implementation

```rust
// efe-nano/src/runtime/pii_redactor.rs

use efe_pii::Redactor;
use std::sync::Arc;

pub struct PiiRedactor {
    inner: Arc<dyn Redactor + Send + Sync>,
}

impl PiiRedactor {
    pub fn new(inner: Arc<dyn Redactor + Send + Sync>) -> Self {
        Self { inner }
    }

    /// Walk the JSON value tree, redacting any string fields.
    pub async fn redact(
        &self,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, PiiError> {
        Ok(self.redact_value(input.clone()).await?)
    }

    fn redact_value(
        &self,
        v: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, PiiError>> + Send + '_>> {
        Box::pin(async move {
            match v {
                serde_json::Value::String(s) => {
                    let redacted = self.inner.redact(&s).await
                        .map_err(|e| PiiError::Redaction(e.to_string()))?;
                    Ok(serde_json::Value::String(redacted))
                }
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        out.push(self.redact_value(v).await?);
                    }
                    Ok(serde_json::Value::Array(out))
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, v) in map {
                        out.insert(k, self.redact_value(v).await?);
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
    Redaction(String),
}
```

---

## 17. Annexe C — HMAC chain extension SQL + Rust types

### 17.1 Canonical row body for `payload_hash`

The `payload_hash` is computed over a **canonical** representation of
the row's body fields (everything except `payload_hash`, `hmac`,
`inserted_at`). Canonicalization uses the same `canonicalize_json`
function as the idempotency cache (Phase 1) for consistency.

```rust
// efe-nano/src/audit/hmac_chain.rs — canonical row body

fn canonical_row_body(trace: &NanoAgentTrace) -> Vec<u8> {
    use serde_json::json;
    let body = json!({
        "trace_id": trace.trace_id.to_string(),
        "tenant_id": trace.tenant_id.to_string(),
        "parent_trace_id": trace.parent_trace_id.map(|t| t.to_string()),
        "swarm_request_id": trace.swarm_request_id.to_string(),
        "operation_id": trace.operation_id,
        "caller_agent_id": trace.caller_agent_id,
        "nano_agent_id": trace.nano_agent_id.as_str(),
        "nano_agent_version": trace.nano_agent_version.as_str(),
        "execution_mode": trace.execution_mode,
        "input_hash": hex::encode(trace.input_hash),
        "output_hash": trace.output_hash.map(hex::encode),
        "error_kind": trace.error_kind,
        "started_at": trace.started_at.to_rfc3339(),
        "completed_at": trace.completed_at.map(|d| d.to_rfc3339()),
        "latency_ms": trace.latency_ms,
        "cost_cents": trace.cost_cents,
        "llm_tokens_used": trace.llm_tokens_used,
    });
    crate::idempotency::canonicalize_json(&body).into_bytes()
}

fn compute_payload_hash(trace: &NanoAgentTrace) -> [u8; 32] {
    use sha2::Digest;
    let body = canonical_row_body(trace);
    let digest = sha2::Sha256::digest(&body);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn compute_hmac(chain_parent_id: Option<uuid::Uuid>, payload_hash: &[u8; 32], key: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    match chain_parent_id {
        Some(p) => mac.update(p.as_bytes()),
        None => mac.update(b"genesis"),
    }
    mac.update(payload_hash);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}
```

### 17.2 Chain verification algorithm

```rust
// efe-nano/src/audit/verify.rs

use sqlx::PgPool;
use uuid::Uuid;

pub async fn verify_chain(
    pool: &PgPool,
    tenant_id: efe_nano_sdk::TenantId,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<u64, AuditChainError> {
    let rows: Vec<TraceRow> = sqlx::query_as(
        r#"
        SELECT trace_id, chain_parent_id, payload_hash, hmac,
               tenant_id, parent_trace_id, swarm_request_id, operation_id,
               caller_agent_id, nano_agent_id, nano_agent_version, execution_mode,
               input_hash, output_hash, error_kind, started_at, completed_at,
               latency_ms, cost_cents, llm_tokens_used
        FROM nano_agent_traces
        WHERE tenant_id = $1
          AND ($2::TIMESTAMPTZ IS NULL OR started_at < $2)
        ORDER BY started_at ASC
        "#,
    )
    .bind(tenant_id.as_uuid())
    .bind(until)
    .fetch_all(pool)
    .await
    .map_err(|e| AuditChainError::Postgres(e.to_string()))?;

    let key = HmacKeySource::EnvHkdf.derive(tenant_id)?;
    let mut prev_id: Option<Uuid> = None;
    for row in &rows {
        if row.chain_parent_id != prev_id {
            return Err(AuditChainError::ChainBroken(row.trace_id));
        }
        let trace: NanoAgentTrace = row.into();
        let expected_payload_hash = compute_payload_hash(&trace);
        if expected_payload_hash != row.payload_hash.as_slice() {
            return Err(AuditChainError::HmacMismatch(row.trace_id));
        }
        let expected_hmac = compute_hmac(row.chain_parent_id, &expected_payload_hash, &key);
        if expected_hmac.as_slice() != row.hmac.as_slice() {
            return Err(AuditChainError::HmacMismatch(row.trace_id));
        }
        prev_id = Some(row.trace_id);
    }
    Ok(rows.len() as u64)
}

// TraceRow is the sqlx-deserializable shape (mirrors the SQL columns).
#[derive(sqlx::FromRow)]
struct TraceRow {
    trace_id: Uuid,
    chain_parent_id: Option<Uuid>,
    payload_hash: Vec<u8>,
    hmac: Vec<u8>,
    // ... other columns ...
}
```

---

## 18. Annexe D — Replay engine algorithm

### 18.1 High-level flow

```
ReplayEngine::replay(tenant, request_id) →
  ┌─────────────────────────────────────────────────────────────┐
  │ 1. Load traces                                              │
  │    SELECT * FROM nano_agent_traces                          │
  │     WHERE tenant_id = tenant AND swarm_request_id = req_id  │
  │     ORDER BY started_at ASC                                 │
  └─────────────────────────────────────────────────────────────┘
                  │
                  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ 2. Verify sub-chain                                          │
  │    For each trace, verify it links to the previous one      │
  │    via chain_parent_id + HMAC. Bail on first failure.       │
  └─────────────────────────────────────────────────────────────┘
                  │
                  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ 3. Reconstruct SwarmRequest                                  │
  │    Group traces by op_id, build Operation { op_id,          │
  │    agent: NanoAgentRef::new(id), input: Literal(...),       │
  │    depends_on: [...], ... }                                 │
  │    Note : depends_on is reconstructed from parent_trace_id  │
  │    via a BFS over the trace forest.                         │
  └─────────────────────────────────────────────────────────────┘
                  │
                  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ 4. Re-dispatch                                               │
  │    dispatcher.dispatch(reconstructed_request) →              │
  │      Result<SwarmResponse, DispatcherError>                  │
  └─────────────────────────────────────────────────────────────┘
                  │
                  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ 5. Compare per-op output_hash                                │
  │    For each op :                                             │
  │      original_hash = trace.output_hash                       │
  │      replay_hash   = sha256(canonical(replay_output))        │
  │      status = determinism_checker.compare(original_hash,    │
  │                replay_hash, trace.execution_mode,           │
  │                agent.determinism)                            │
  └─────────────────────────────────────────────────────────────┘
                  │
                  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ 6. Build + persist ReplayReport                              │
  │    INSERT INTO replay_attempts (...)                         │
  │    Return ReplayReport to caller                             │
  └─────────────────────────────────────────────────────────────┘
```

### 18.2 Determinism check rules

| Agent's `DeterminismLevel` | `Match` rule | `Mismatch` action |
|----------------------------|---------------|----------------------|
| `FullDeterministic` (Mode A) | exact hash equality | Report Mismatch ; flag for investigation |
| `ProbabilisticFlavor` (Mode B with fixed seed) | exact hash equality | Report Mismatch ; check seed |
| `Stochastic` (Mode C LLM) | always `SkippedNonDeterministic` | no comparison |

### 18.3 `SwarmReconstructor` algorithm

```rust
// efe-nano/src/replay/reconstructor.rs

use efe_nano_sdk::{
    NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, SwarmRequest, TenantId,
};
use std::collections::HashMap;
use uuid::Uuid;

pub struct SwarmReconstructor;

impl SwarmReconstructor {
    pub fn reconstruct(
        traces: &[ReconstructorInput],
    ) -> Result<SwarmRequest, ReplayError> {
        if traces.is_empty() {
            return Err(ReplayError::NotFound(Uuid::nil()));
        }
        let request_id = traces[0].swarm_request_id;
        let tenant_id = traces[0].tenant_id;

        // Build a map trace_id → op_id for parent resolution.
        let trace_to_op: HashMap<Uuid, String> = traces.iter()
            .map(|t| (t.trace_id, t.operation_id.clone()))
            .collect();

        let mut operations: Vec<Operation> = Vec::with_capacity(traces.len());
        for t in traces {
            let depends_on: Vec<String> = t.parent_trace_id.iter()
                .filter_map(|p| trace_to_op.get(p).cloned())
                .collect();

            operations.push(Operation {
                op_id: t.operation_id.clone(),
                agent: NanoAgentRef::new(NanoAgentId::new(&t.nano_agent_id).unwrap()),
                input: OperationInput::Literal { value: t.input_value.clone() },
                depends_on,
                timeout_ms: t.latency_ms,
                retry: RetryPolicy::default(),
            });
        }

        Ok(SwarmRequest {
            request_id,
            tenant_id,
            caller_agent_id: traces[0].caller_agent_id.clone(),
            operations,
            max_parallelism: 100,
            timeout_ms: 60_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec!["replay".to_string()],
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReconstructorInput {
    pub trace_id: Uuid,
    pub swarm_request_id: Uuid,
    pub tenant_id: TenantId,
    pub parent_trace_id: Option<Uuid>,
    pub operation_id: String,
    pub nano_agent_id: String,
    pub input_value: serde_json::Value,    // de-hashed from log table (Phase 2 stores full input for replay)
    pub latency_ms: Option<u32>,
    pub caller_agent_id: Option<String>,
}
```

**Important note** : Phase 2's `nano_agent_traces` table stores only
`input_hash` (32 bytes SHA-256). To enable replay, Sprint 6 Day 5 adds
a sibling table `nano_agent_inputs` keyed by `input_hash` that stores
the full deduplicated JSON input. This makes replay possible without
bloating the main traces table.

```sql
-- Sprint 6 Day 5 ADDS this auxiliary table (after-Sprint-6 migration v008
-- if needed) :
CREATE TABLE IF NOT EXISTS nano_agent_inputs (
    input_hash      BYTEA       PRIMARY KEY,
    tenant_id       UUID        NOT NULL,
    full_payload    JSONB       NOT NULL,
    first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    refcount        INTEGER     NOT NULL DEFAULT 1
);
```

---

## 19. Annexe E — 10 new starter agents specs

### 19.1 Mode B agents (Sprint 4 — 5 agents)

#### E.1.1 `ner_clinical`

| Field | Value |
|-------|-------|
| Mode | B (LocalModel) |
| Model | `ner_clinical_v1` v1.0.0 (BiomedBERT-derivative, ONNX, ~ 200 MB) |
| Category | EntityRecognition |
| Compliance | gdpr_relevant=true, hipaa_relevant=true, dpia_required=true |
| Cost | $0.001 / call |
| Latency p99 | 200 ms |
| Use case | Extract DRUG, DISEASE, GENE, PHASE from medical text |
| Test coverage | 3 tests : manifest, valid input, invalid input |

#### E.1.2 `ner_legal`

| Field | Value |
|-------|-------|
| Mode | B (LocalModel) |
| Model | `ner_legal_v1` v1.0.0 (LegalBERT-derivative, ONNX, ~ 150 MB) |
| Category | EntityRecognition |
| Compliance | gdpr_relevant=true, dpia_required=true |
| Cost | $0.001 / call |
| Latency p99 | 150 ms |
| Use case | Extract CASE, CITATION, PARTY, CLAUSE from legal text |

#### E.1.3 `intent_classify_generic`

| Field | Value |
|-------|-------|
| Mode | B (LocalModel) |
| Model | `intent_classify_v1` (MiniLM fine-tuned, ONNX, ~ 80 MB) |
| Category | Classification |
| Compliance | gdpr_relevant=false |
| Cost | $0.0005 / call |
| Latency p99 | 80 ms |
| Use case | Classify free-text into 50 generic intents (buy, complain, request_info, ...) |

#### E.1.4 `lang_detect`

| Field | Value |
|-------|-------|
| Mode | B (LocalModel) |
| Model | `lang_detect_v1` (fastText port, ONNX, ~ 5 MB) |
| Category | Classification |
| Compliance | gdpr_relevant=false |
| Cost | $0.0001 / call |
| Latency p99 | 20 ms |
| Use case | Detect language from text (140+ languages, ISO 639-1 codes) |

#### E.1.5 `embed_text`

| Field | Value |
|-------|-------|
| Mode | B (LocalModel) |
| Model | `embedder_v1` (MiniLM-L6-v2, ONNX, ~ 80 MB) |
| Category | Other (Embedding) |
| Compliance | gdpr_relevant=true (text may contain PII) |
| Cost | $0.0005 / call |
| Latency p99 | 100 ms |
| Use case | Produce 384-dim embedding vector for downstream similarity / retrieval |

### 19.2 Mode C agents (Sprint 5 — 5 agents)

#### E.2.1 `summarize_strict`

| Field | Value |
|-------|-------|
| Mode | C (LlmAugmented) |
| Prompt template | `m50.summarize.v1` |
| Providers | anthropic:claude-sonnet-4-5, openai:gpt-5 |
| Category | Summarization |
| Compliance | gdpr_relevant=true, pii_redaction=true |
| Cost | $0.02 / call |
| Latency p99 | 3,000 ms |
| Output schema | `{ summary: string, sentence_count: int }` strict |
| Use case | Cap-length summary with exact sentence count |

#### E.2.2 `extract_structured`

| Field | Value |
|-------|-------|
| Mode | C (LlmAugmented) |
| Prompt template | `m50.extract_structured.v1` |
| Providers | anthropic:claude-sonnet-4-5 (preferred for JSON), openai:gpt-5 |
| Category | StructuredExtract |
| Compliance | gdpr_relevant=true, pii_redaction=true |
| Cost | $0.025 / call |
| Latency p99 | 5,000 ms |
| Output schema | `{ extracted: object }` with user-supplied field list |
| Use case | Extract named fields from free-text using a customer-specified schema |

#### E.2.3 `classify_topic_llm`

| Field | Value |
|-------|-------|
| Mode | C (LlmAugmented) |
| Prompt template | `m50.classify_topic.v1` |
| Providers | openai:gpt-5-mini, anthropic:claude-haiku |
| Category | Classification |
| Compliance | gdpr_relevant=true, pii_redaction=true |
| Cost | $0.005 / call |
| Latency p99 | 2,000 ms |
| Output schema | `{ topic: string, confidence: number }` |
| Use case | Free-form topic classification (no closed taxonomy) |

#### E.2.4 `sentiment_nuanced`

| Field | Value |
|-------|-------|
| Mode | C (LlmAugmented) |
| Prompt template | `m50.sentiment_nuanced.v1` |
| Providers | anthropic:claude-sonnet, openai:gpt-5 |
| Category | Classification |
| Compliance | gdpr_relevant=true |
| Cost | $0.01 / call |
| Latency p99 | 2,500 ms |
| Output schema | `{ sentiment: enum, intensity: number, aspects: [{aspect, sentiment}] }` |
| Use case | Multi-aspect sentiment (e.g. "service:positive, price:negative") |

#### E.2.5 `qa_grounded`

| Field | Value |
|-------|-------|
| Mode | C (LlmAugmented) |
| Prompt template | `m50.qa_grounded.v1` |
| Providers | anthropic:claude-opus, openai:gpt-5 |
| Category | StructuredExtract |
| Compliance | gdpr_relevant=true, ai_act_high_risk=false (informational) |
| Cost | $0.03 / call |
| Latency p99 | 4,000 ms |
| Output schema | `{ answer: string, citations: [{span_start, span_end, source}] }` |
| Use case | Answer a question given a context, citing source spans |

---

## 20. Annexe F — Sprint daily plans (30 working days)

Recap of all 30 working days across Phase 2 :

```
═══════════════════════════════════════════════════════════════════
SPRINT 4 — Mode B (Aug 11 - Aug 22)
═══════════════════════════════════════════════════════════════════
Day 1  Mon Aug 11  Kickoff. ModelLoader + VmPool + 5 agent stubs.
Day 2  Tue Aug 12  ONNX model loading (ner_clinical, embedder).
Day 3  Wed Aug 13  3 more models + Firecracker single-VM proof.
Day 4  Thu Aug 14  Model concurrent-load race + pool scaling.
Day 5  Fri Aug 15  Mid-point: MultiModeRuntime end-to-end demo.
Day 6  Mon Aug 18  Health metrics + warm() pre-load.
Day 7  Tue Aug 19  Production pool scaling + chaos tests.
Day 8  Wed Aug 20  Schema cache + perf to 100 cps target.
Day 9  Thu Aug 21  Docs update + integration test corpus.
Day 10 Fri Aug 22  Sprint 4 close + retro.

═══════════════════════════════════════════════════════════════════
SPRINT 5 — Mode C (Aug 26 - Sep 5)
═══════════════════════════════════════════════════════════════════
Day 1  Mon Aug 26  Kickoff. LlmClient + BudgetEnforcer + 5 agent stubs.
Day 2  Tue Aug 27  LlmRouterClient implementation + PiiRedactor.
Day 3  Wed Aug 28  OutputValidator + LlmAugmentedRuntime skeleton.
Day 4  Thu Aug 29  Wire MultiModeRuntime A+B+C + mixed-mode DAG test.
Day 5  Fri Aug 30  Mid-point: 5 Mode C agents end-to-end demo.
Day 6  Mon Sep 1   JSON response retry + provider preference.
Day 7  Tue Sep 2   Cost cap reconciliation + Meter::NanoAgent* PR.
Day 8  Wed Sep 3   Performance pass on Mode C path.
Day 9  Thu Sep 4   Cancellation + edge cases.
Day 10 Fri Sep 5   Sprint 5 close + retro.

═══════════════════════════════════════════════════════════════════
SPRINT 6 — Audit + Replay (Sep 10 - Sep 21)
═══════════════════════════════════════════════════════════════════
Day 1  Mon Sep 10  Kickoff. v004 migration + HmacChainAuditWriter skeleton.
Day 2  Tue Sep 11  HKDF key derivation + SwarmReconstructor.
Day 3  Wed Sep 12  HmacChainAuditWriter::record + DeterminismChecker.
Day 4  Thu Sep 13  verify_chain + ReplayEngine wired end-to-end.
Day 5  Fri Sep 14  Mid-point: chain write + replay demo. Add nano_agent_inputs sibling table.
Day 6  Mon Sep 17  Audit writer batching + perf target 5k/sec.
Day 7  Tue Sep 18  Replay parallel + 1k-corpus generation.
Day 8  Wed Sep 19  1k-swarm determinism test : 100 % match achieved.
Day 9  Thu Sep 20  CLI : efe-nano-admin audit replay command.
Day 10 Fri Sep 21  Sprint 6 close + Phase 2 retro.

═══════════════════════════════════════════════════════════════════
PHASE 2 EXIT GATE — Sep 28, 2026
═══════════════════════════════════════════════════════════════════
```

---

## 21. Annexe G — Decision log

### G.1 Mode B model registry — hard-coded vs dynamic

**Decision** : hard-coded set of 5 models in Phase 2. Dynamic registration
via catalog deferred to Phase 3 Sprint 8.

**Rationale** :
- Ship date pressure : Sprint 4 has 10 days, ONNX integration eats most
  of it.
- 5 starter Mode B agents only need 5 models — hard-coding is simpler.
- Customer-published Mode B agents need a security audit anyway
  (model file signing) — pushing to Sprint 8+ aligns with cosign work.

**Trade-off** : customers cannot bring their own Mode B models in Phase 2.
We mitigate by clearly stating "Mode B = EFE-shipped only in Phase 2"
in the architecture doc + marketing.

### G.2 Mode C JSON response parsing — strict vs lenient

**Decision** : strict parsing (single attempt + retry once).

**Rationale** :
- Customers depend on output_schema contract. Lenient parsing (e.g.
  "strip markdown fences first") risks silent data corruption.
- Strict + retry once gives 99 %+ success rate on modern LLMs that
  support JSON mode.
- Surface failure as `SchemaValidationFailed` so the dispatcher can
  skip downstream ops (matching Phase 1 semantics).

**Trade-off** : 1 % of calls hit a hard fail where lenient would have
succeeded. Customers can request "lenient mode" as a per-agent flag in
Phase 4.

### G.3 Audit chain key derivation — env-HKDF vs Vault

**Decision** : env-HKDF in Phase 2, Vault Transit in Phase 4.

**Rationale** :
- Phase 2 ships internally only ; no customer key access yet.
- Env-HKDF is simpler to operate, no Vault dependency.
- Phase 4 GA requires HSM-grade key custody → Vault Transit.

**Trade-off** : Phase 2 audit chain is "fork-safe but not Vault-grade".
This is acceptable for non-customer-facing usage.

### G.4 Replay scope — all modes vs deterministic only

**Decision** : determinism-aware replay. Mode A + Mode B (with seed)
get 100 % match expectation ; Mode C marked SkippedNonDeterministic.

**Rationale** :
- LLM replay is inherently impossible without storing the exact
  provider response (which defeats the purpose of replay).
- We can still verify *the chain* for Mode C ops — just not their
  output reproducibility.
- This matches industry practice (e.g. SOC2 Type II).

**Trade-off** : customers expecting "Mode C replay" need education
that this is a fundamental LLM limitation, not an EFE shortcoming.

### G.5 Mode B determinism — fixed seed vs free random

**Decision** : fixed RNG seed per Mode B model invocation, captured
in trace.

**Rationale** :
- ONNX models with dropout layers (e.g. some NER) are non-deterministic
  by default.
- Fixed seed (seeded from `input_hash`) makes them deterministic given
  same input.
- This unlocks Mode B replay → critical for regulatory replay use cases.

**Trade-off** : tiny precision loss vs free-running RNG (negligible for
classification + NER). Trace size grows by 32 bytes (seed bytes).

### G.6 BudgetEnforcer — stub vs real M31 integration

**Decision** : stub in Phase 2, real M31 in Sprint 11 (Phase 4).

**Rationale** :
- M31 billing service in early Phase 2 is itself in flux.
- Stub satisfies the trait contract + gives us realistic test cases.
- Coupling M50 Phase 2 to M31's GA blocks Phase 2 GA on someone else's
  timeline.

**Trade-off** : Phase 2 ships without real budget enforcement. We
clearly mark this in the public docs + ship a default-permissive
budget (e.g. 100k cents per tenant per month).

### G.7 PII redaction — pre-LLM vs post-LLM

**Decision** : pre-LLM redaction (default) for Mode C.

**Rationale** :
- Sending PII to a third-party LLM is the highest compliance risk.
- Pre-LLM redaction eliminates this risk entirely.
- Post-LLM "scrubbing" the response cannot un-leak data that was already
  sent.

**Trade-off** : some semantically-relevant signal is lost in redaction
(e.g. names disambiguating entities). Customers can opt out per-agent
via `compliance.pii_redaction_strategy = None` after explicit DPIA.

### G.8 New `Meter` variants — additive to M31

**Decision** : ship Meter::NanoAgentCalls + NanoAgentLlmTokens +
NanoAgentVmSeconds as additive variants in Sprint 5 Day 5.

**Rationale** :
- Billing M31 requires explicit Meter variants — no extensibility
  point exists.
- Adding enum variants is the standard Rust pattern (non-breaking
  for consumers using catch-all `_` match arms).
- Decision approved by M31 owner + Senior #1.

**Trade-off** : this is the ONE exception to "zero-modification of
existing crates" in Phase 2. Documented prominently in § 10.1.

---

## 22. References & cross-links

**Predecessor docs** :
- `EFE_M50_PHASE_1_ARCHITECTURE.md` — Phase 1 (Sprints 1-3) deliverables
- `EFE_M50_AGENTIC_ARCHITECTURE.md` — master architecture, all phases
- `EFE_M_NUMBER_REGISTRY.md` — canonical M-number lookup

**Successor docs** (not yet written) :
- `EFE_M50_PHASE_3_ARCHITECTURE.md` — MCP + SDK + Distribution (Sprints 7-10)
- `EFE_M50_PHASE_4_ARCHITECTURE.md` — Hardening + GA (Sprints 11-13)

**Decision records** (will be added during Sprint 4 kickoff) :
- `D-M50-PHASE2-001.md` — Mode B model registry hard-coding (G.1)
- `D-M50-PHASE2-002.md` — Mode C JSON parsing strictness (G.2)
- `D-M50-PHASE2-003.md` — Audit chain key derivation (G.3)
- `D-M50-PHASE2-004.md` — Replay determinism semantics (G.4)
- `D-M50-PHASE2-005.md` — BudgetEnforcer stub strategy (G.6)
- `D-M50-PHASE2-006.md` — New Meter variants (G.8)

**External references** :
- ONNX Runtime spec : <https://onnx.ai/onnx/intro/concepts.html>
- Firecracker MicroVM : <https://firecracker-microvm.github.io/>
- HKDF RFC 5869 : <https://www.rfc-editor.org/rfc/rfc5869>
- HMAC RFC 2104 : <https://www.rfc-editor.org/rfc/rfc2104>
- EU AI Act Annex III : <https://artificialintelligenceact.eu/>
- GDPR Art. 22 (automated decisions) : <https://gdpr-info.eu/art-22-gdpr/>

---

## 23. Phase 2 risk matrix summary

```
                              Probability →
                              Low      Medium       High
              ────────────────────────────────────────────────
              Critical        HMAC race  Firecracker  ─
                              (mitigated) (M25 lock)
              ────────────────────────────────────────────────
   Impact ↓   Major           Mode C    Senior #2     Cargo
                              degraded  onboarding    build
                              (mock)    (pair days)   bloat
              ────────────────────────────────────────────────
              Minor           CLI UX    Pool tuning   Test
                              (defer)   (chaos tests) flakiness
              ────────────────────────────────────────────────
```

Red items (Critical+High) require **daily founder check-in** during
the relevant sprint. Yellow items (Major) reviewed at sprint mid-point.
Green items addressed in retro.

---

## 24. Glossary (Phase 2 additions)

| Term | Definition |
|------|------------|
| **Mode A** | Pure Rust thread-pool runtime ; Phase 1 baseline. |
| **Mode B** | Local model microVM runtime ; Sprint 4 deliverable. |
| **Mode C** | LLM-augmented runtime via M21 ; Sprint 5 deliverable. |
| **VmPool** | Pool of warm Firecracker microVMs categorized by `VmKind`. |
| **VmKind** | Either `ModelInference` (Mode B) or `LlmCaller` (Mode C). |
| **HMAC chain** | Per-tenant audit trail where each row's HMAC binds to the previous row's id + this row's content. |
| **Chain parent** | The trace whose `trace_id` equals this row's `chain_parent_id` ; the immediately-previous trace for the same tenant. |
| **Replay** | Re-executing a historical swarm and comparing per-op output hashes to detect drift. |
| **Determinism level** | Per-agent flag (FullDeterministic / ProbabilisticFlavor / Stochastic) driving replay expectations. |
| **PII redactor** | Adapter wrapping `efe-pii` (M20) that walks the JSON input tree and replaces PII strings with redaction tokens. |
| **BudgetEnforcer** | Per-tenant pre-check + post-charge interface for Mode C cost cap. |
| **HKDF** | RFC 5869 key derivation function used to derive per-tenant HMAC keys from a master secret. |
| **Output validator** | Strict JSON schema enforcer for Mode C LLM responses. |
| **Reservation** | A budget pre-check artifact returned by `BudgetEnforcer::pre_check` ; later committed or released. |

---

**End of EFE M50 Phase 2 Architecture document.**

Phase 2 ships a **mode-agnostic, audit-ready** EFE nano agent platform
in 6 weeks with a team of 3 engineers. By Sep 28, 2026, customers can
mix Mode A + Mode B + Mode C operations in one swarm, every call is
HMAC-chained for regulatory replay, and the platform is ready for the
SDK + MCP + Distribution work of Phase 3.

Total Rust LOC at end of Phase 2 : ~ 10,500 lines.
Total tests at end of Phase 2 : 492 M50 + 505 baseline = ~ 997 green.
Performance contracts met : Mode A 1k cps + Mode B 100 cps + Mode C
10 cps + Audit 5k writes/sec + Replay 100 swarms/sec.

The path is now clear from Phase 2 exit (Sep 28) → Phase 3 entry
(Sep 29) → MCP bridge live → SDK shipped → Public Beta launch → Phase 4
GA on Dec 31, 2026.
