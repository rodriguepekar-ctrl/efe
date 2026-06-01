# EFE M50 Phase 1 — Foundation Architecture

**Status**            : DRAFT v1
**Date drafted**      : 2026-05-18
**Owner**             : Rodrigue Pekar (founding engineer) + Senior Platform Engineer #1 (TBH)
**Phase**             : 1 of 5 (Foundation)
**Sprints**           : 1, 2, 3
**Dates target**      : Jun 16 - Aug 10, 2026 (6 weeks, 3 × 2-week sprints)
**Team size**         : 2 (founder + 1 senior hire)
**Companion docs**    :
- [`EFE_M50_AGENTIC_ARCHITECTURE.md`](EFE_M50_AGENTIC_ARCHITECTURE.md) (master architecture, 2,219 lignes)
- [`EFE_M50_VERIFICATION_REPORT_2026-05-18.md`](EFE_M50_VERIFICATION_REPORT_2026-05-18.md) (audit, 15 défauts → 9 fixed)
- [`EFE_M50_SPRINT_0_VALIDATION_REPORT.md`](EFE_M50_SPRINT_0_VALIDATION_REPORT.md) (Phase 0 closure)
- [`EFE_M_NUMBER_REGISTRY.md`](EFE_M_NUMBER_REGISTRY.md) (canonical M-number registry)
- [`../roadmap/EFE_M50_ROADMAP.md`](../roadmap/EFE_M50_ROADMAP.md) (14-sprint plan)
- [`../decisions/D-M50-INFRA-001.md`](../decisions/D-M50-INFRA-001.md) (Postgres+Redis decision)
- [`../decisions/D-M50-CRATES-001.md`](../decisions/D-M50-CRATES-001.md) (2-crate split)

> **Contrat additif strict (rappel)** : Phase 1 ne modifie **aucun**
> fichier source des 33 crates existants. Toutes les modifications
> sont localisées dans `crates/efe-nano/` + `crates/efe-nano-sdk/`,
> avec ajouts (ZÉRO suppression) au workspace `Cargo.toml`.
>
> Vérification à chaque PR : les 174 tests PR #27 scope + 51 tests
> STABILITY v17 freeze restent verts.  CI gate enforced.

---

## Table des matières

0. Executive summary
1. Phase 1 — scope précis (in / out / deferred)
2. Module hierarchy of `efe-nano` during Phase 1
3. Complete type system (Phase 1 surface)
4. Sprint 1 — Catalog + NAR Mode A (Day-by-day)
5. Sprint 2 — Postgres catalog + 5 starter agents (Day-by-day)
6. Sprint 3 — Dispatcher v0 + 5 more agents (Day-by-day)
7. Postgres schema + migrations
8. Error handling strategy
9. Performance contracts
10. Integration matrix with existing EFE crates
11. CI / build integration
12. Phase 1 → Phase 2 gate criteria
13. Test catalogue (~ 80 new tests)
14. Risk register specific to Phase 1
15. Annexe A — Complete Rust trait definitions
16. Annexe B — Postgres migration SQL (v001-v003)
17. Annexe C — Per-nano-agent detailed specs (10 starters)
18. Annexe D — Sprint daily plans (30 working days)
19. Annexe E — Decision log

---

## 0. Executive summary

Phase 1 ships the **foundation** of the EFE M50 Nano Agent Swarm
platform. By end of Sprint 3 (Aug 10, 2026), the codebase will contain :

```
═══════════════════════════════════════════════════════════════════
  Phase 1 Outputs (Sprint 1 → Sprint 3)
═══════════════════════════════════════════════════════════════════

  Catalog                    : Postgres-backed, versioned, per-tenant
  Mode A runtime             : Pure Rust thread pool, ≥ 1k calls/sec/node
  Dispatcher v0              : DAG planner + parallel layer execution
  Starter pack (10 agents)   : All Mode A, all TIER-1 from Day 1
  Test surface (~ 80 new)    : Catalog 25 + Mode A 15 + Dispatcher 25 + Agents 15

  Cumulative Rust LOC added  : ~ 5,000 lines (efe-nano + efe-nano-sdk)
  Cumulative test LOC        : ~ 1,500 lines
  Postgres migrations        : 3 (v001, v002, v003)
  Public API surface          : ~ 12 types in efe-nano-sdk

═══════════════════════════════════════════════════════════════════

  Performance contract (Phase 1 → 2 gate) :
    Mode A throughput        : ≥ 1,000 calls/sec/node
    Mode A latency p99       : ≤ 50 ms (per call)
    Dispatcher overhead p99  : ≤ 50 ms (DAG planning + dispatch)
    Catalog lookup p99       : ≤ 20 ms

  Compliance contract :
    Schema validation        : Strict (input + output) before/after execute
    Audit trace               : Stub (full chain Sprint 6)
    Cost cap                  : Stub (full enforcement Sprint 5)
    Per-tenant isolation     : Logical (Postgres row-level), microVM Sprint 4

═══════════════════════════════════════════════════════════════════
```

Phase 1 is foundational : it does **not** ship customer-facing features
(MCP integration is Sprint 7, billing is Sprint 11).  It ships the
**internal substrate** on which all later sprints build.

### What Phase 1 unlocks

After Phase 1 gate (Aug 10, 2026) :
- Internal benchmarks can run swarms of 10-100 Mode A operations
- New nano agents can be added by writing a single Rust file
- Test framework is in place to gate any future regression
- Performance baseline established for Phase 2 to extend

### What Phase 1 does NOT ship

Explicitly deferred to later phases :
- Mode B (local model) runtime → Sprint 4
- Mode C (LLM-augmented) runtime → Sprint 5
- Full audit chain integration with M23 HMAC → Sprint 6
- MCP server tool exposure → Sprint 7
- Customer-facing SDK macros → Sprint 8
- Billing + Stripe → Sprints 7+11
- Replay capability → Sprint 6
- Multi-region deployment → Phase 4

---

## 1. Phase 1 — scope précis

### 1.1 In scope (must ship)

| Capability | Sprint | Owner | Deliverable |
|------------|--------|-------|-------------|
| Catalog types (`NanoAgent`, `NanoAgentRef`, etc.) | 1 | Senior #1 | `efe-nano-sdk/src/types/mod.rs` |
| In-memory catalog store | 1 | Senior #1 | `efe-nano/src/catalog/in_memory.rs` |
| Mode A runtime (pure Rust thread pool) | 1 | Founder | `efe-nano/src/runtime/pure_rust.rs` |
| Schema validation (input + output) | 1 | Founder | Uses `jsonschema` crate |
| Resource limits enforcement | 1 | Founder | `efe-nano/src/runtime/limits.rs` |
| Postgres catalog backend | 2 | Senior #1 | `efe-nano/src/catalog/postgres.rs` |
| Catalog migration v001 | 2 | Senior #1 | `efe-nano/migrations/v001_*.sql` |
| Per-tenant scoping | 2 | Senior #1 | row-level filtering |
| 5 starter agents (Sprint 2) | 2 | Founder | `efe-nano/src/starter_pack/json_validate.rs`, etc. |
| Dispatcher v0 (DAG + parallel) | 3 | Senior #1 | `efe-nano/src/dispatcher/` |
| Idempotency key handling | 3 | Senior #1 | uses `efe-cache` (M27) |
| 5 more starter agents (Sprint 3) | 3 | Founder | `efe-nano/src/starter_pack/*.rs` |
| ~ 80 new tests | 1-3 | Both | `crates/efe-nano/tests/*.rs` |

### 1.2 Out of scope (Phase 2+)

| Capability | Reason out of Phase 1 | Target phase |
|------------|------------------------|--------------|
| Mode B runtime (local model) | Needs model loader + Firecracker integration | Phase 2 Sprint 4 |
| Mode C runtime (LLM-augmented) | Needs M21 LLM router integration | Phase 2 Sprint 5 |
| Audit chain integration | Needs M23 HMAC chain extension | Phase 2 Sprint 6 |
| Replay capability | Requires audit trace storage | Phase 2 Sprint 6 |
| MCP server exposure | Customer-facing, Phase 3 | Phase 3 Sprint 7 |
| Customer SDK macros | Customer authoring, Phase 3 | Phase 3 Sprint 8 |
| Stripe billing | Customer monetization, Phase 3 | Phase 3 Sprint 7/11 |

### 1.3 Deferred but planned

These are anticipated but final decisions are Phase 2+ :

- **Audit chain stub** in Phase 1 : log each call to a temporary table
  `nano_agent_traces_phase1` ; Sprint 6 migrates to full HMAC chain.
- **Cost cap stub** in Phase 1 : decrement a per-tenant counter ; Sprint 5
  integrates with `efe-billing` (M31) budget enforcement.
- **Idempotency cache** in Phase 1 : uses `efe-cache` (M27) Redis-backed,
  with 24h TTL. Production tuning in Phase 3.

---

## 2. Module hierarchy of `efe-nano` during Phase 1

State of `crates/efe-nano/src/` at end of each sprint :

### 2.1 End Sprint 1

```
crates/efe-nano/src/
├── lib.rs                          (re-exports + crate constants)
├── error.rs                        (NanoAgentError enum)
├── catalog/
│   ├── mod.rs                      (Catalog trait + ops)
│   ├── types_re_export.rs         (re-export from efe-nano-sdk::types)
│   └── in_memory.rs                (InMemoryCatalog impl)
├── runtime/
│   ├── mod.rs                      (Runtime trait)
│   ├── pure_rust.rs                (Mode A executor)
│   └── limits.rs                   (ResourceLimits enforcement)
├── audit/
│   ├── mod.rs                      (Audit trait stub)
│   └── stub.rs                     (Phase 1 stub impl, console + temp table)
└── (other modules : empty stubs)
```

### 2.2 End Sprint 2

```
crates/efe-nano/src/
├── lib.rs
├── error.rs
├── catalog/
│   ├── mod.rs
│   ├── types_re_export.rs
│   ├── in_memory.rs                (kept for tests)
│   └── postgres.rs                 (PostgresCatalog impl)
├── runtime/
│   ├── mod.rs
│   ├── pure_rust.rs
│   └── limits.rs
├── audit/
│   └── (unchanged)
├── starter_pack/
│   ├── mod.rs
│   ├── json_validate.rs            (Sprint 2)
│   ├── xml_validate.rs             (Sprint 2)
│   ├── regex_extract.rs            (Sprint 2)
│   ├── date_normalize.rs           (Sprint 2)
│   └── currency_normalize.rs       (Sprint 2)
└── migrations/
    ├── v001_create_nano_agents.sql      (NEW)
    └── v002_create_nano_agent_versions.sql (NEW)
```

### 2.3 End Sprint 3 (= end Phase 1)

```
crates/efe-nano/src/
├── lib.rs
├── error.rs
├── catalog/
│   └── ... (Sprint 2 state)
├── runtime/
│   └── ... (Sprint 1 state)
├── audit/
│   └── ... (stub)
├── dispatcher/                     (NEW Sprint 3)
│   ├── mod.rs                      (Dispatcher trait + entry)
│   ├── types.rs                    (SwarmRequest, SwarmResponse, Operation)
│   ├── dag.rs                      (DAG planner + cycle detection)
│   ├── executor.rs                 (layer-by-layer parallel execution)
│   └── idempotency.rs              (uses efe-cache M27)
├── starter_pack/
│   ├── ... (Sprint 2 state)
│   ├── pdf_extract_text.rs         (Sprint 3)
│   ├── html_extract_main.rs        (Sprint 3)
│   ├── markdown_extract.rs         (Sprint 3)
│   ├── csv_normalize.rs            (Sprint 3)
│   └── sql_parse_safe.rs           (Sprint 3)
└── migrations/
    ├── v001_*.sql
    ├── v002_*.sql
    └── v003_create_idempotency_index.sql (Sprint 3 if needed)
```

Total at end of Phase 1 : **~ 30 .rs files** in `efe-nano/`, **3 SQL migrations**, **~ 10 nano agents** in starter pack.

---

## 3. Complete type system (Phase 1 surface)

All types defined in `efe-nano-sdk` (public) and consumed by `efe-nano` (runtime).

### 3.1 Catalog types

```rust
// efe-nano-sdk/src/types/mod.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a nano agent (snake_case, ≤ 64 chars).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NanoAgentId(pub String);

impl NanoAgentId {
    pub fn new(s: impl Into<String>) -> Result<Self, NanoAgentIdError> {
        let s = s.into();
        if !is_valid_snake_case(&s) {
            return Err(NanoAgentIdError::InvalidFormat);
        }
        if s.len() > 64 {
            return Err(NanoAgentIdError::TooLong);
        }
        Ok(Self(s))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NanoAgentIdError {
    #[error("ID must be snake_case (a-z, 0-9, _) starting with a letter")]
    InvalidFormat,
    #[error("ID must be ≤ 64 chars")]
    TooLong,
}

/// Semantic version of a nano agent (semver).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NanoAgentVersion(pub String);

impl NanoAgentVersion {
    pub fn new(s: impl Into<String>) -> Result<Self, NanoAgentIdError> {
        let s = s.into();
        // Validates semver (e.g. "1.0.0", "0.1.2-beta.1")
        semver::Version::parse(&s).map_err(|_| NanoAgentIdError::InvalidFormat)?;
        Ok(Self(s))
    }
}

/// Reference to a nano agent (id + version constraint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoAgentRef {
    pub id: NanoAgentId,
    pub version: VersionReq,  // e.g. "1.x", "^0.1.0", "=1.2.3"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionReq(pub String);  // semver::VersionReq under the hood

/// Full catalog entry for a nano agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoAgent {
    pub id: NanoAgentId,
    pub version: NanoAgentVersion,
    pub description: String,        // ≤ 256 chars
    pub author: Author,
    pub category: Category,
    pub stability_tier: StabilityTier,
    
    pub execution_mode: ExecutionMode,
    pub determinism: DeterminismLevel,
    pub idempotency: IdempotencyPolicy,
    
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
    pub error_schema: Option<JsonSchema>,
    
    pub limits: ResourceLimits,
    pub cost_estimate_cents: f64,
    pub latency_p99_ms_target: u32,
    
    pub audit: AuditPolicy,
    pub compliance: ComplianceTags,
    
    pub mcp_tool_metadata: Option<McpToolMetadata>,  // Used Sprint 7
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by_tenant: Option<TenantId>,  // None for EFE-shipped
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Author {
    Efe,
    Tenant(TenantId),
    Community(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Category {
    Extraction,         // PDF text, HTML, etc.
    Validation,         // JSON, XML, SQL parsing
    Classification,     // language, sentiment, intent
    Transformation,     // date norm, currency norm
    LookUp,             // DB lookup, URL fetch
    EntityRecognition,  // NER
    Summarization,      // (Mode C, Phase 2)
    StructuredExtract,  // (Mode C, Phase 2)
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StabilityTier {
    TierExperimental,   // No guarantees
    Tier1,              // 12-month schema freeze
    Tier2,              // 36-month schema freeze
    TierAudit,          // 7+ year freeze + cosign signed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum ExecutionMode {
    /// Phase 1 : pure Rust thread pool.
    PureRust {
        /// Entry function identifier (resolved at runtime).
        entry: String,
    },
    /// Phase 2 (Sprint 4) — placeholder type.
    LocalModel {
        model_id: String,
        model_version: String,
    },
    /// Phase 2 (Sprint 5) — placeholder type.
    LlmAugmented {
        provider: String,
        model: String,
        prompt_template_id: String,
        max_tokens: u32,
        temperature: f32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeterminismLevel {
    /// Same input → same output, every time.
    FullDeterministic,
    /// May vary (LLM, sampling-based).
    Probabilistic,
    /// External I/O, time-dependent.
    SideEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IdempotencyPolicy {
    /// Auto-hash input, cache for `ttl_sec`.
    Auto { ttl_sec: u32 },
    /// Customer provides idempotency key.
    Manual { ttl_sec: u32 },
    /// Every invocation executes anew.
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_mb: u32,             // max RSS during execution
    pub cpu_ms: u32,                // max CPU time
    pub wall_clock_ms: u32,         // max wall-clock latency
    pub llm_tokens: Option<u32>,    // Mode C only
    pub network_egress_kb: u32,     // 0 for Mode A
}

impl ResourceLimits {
    /// Default for Mode A nano agents.
    pub fn default_mode_a() -> Self {
        Self {
            memory_mb: 256,
            cpu_ms: 1_000,
            wall_clock_ms: 5_000,
            llm_tokens: None,
            network_egress_kb: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditPolicy {
    /// Include input in trace ?
    pub include_input: bool,
    /// Include output in trace ?
    pub include_output: bool,
    /// Redact PII before logging ?
    pub redact_pii: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceTags {
    /// Subject to GDPR data processing rules ?
    pub gdpr: bool,
    /// Subject to HIPAA (PHI) handling ?
    pub hipaa: bool,
    /// Subject to GxP (pharma) audit ?
    pub gxp: bool,
    /// Subject to FINMA (Swiss financial) audit ?
    pub finma: bool,
    /// Subject to EU AI Act Article 9 high-risk ?
    pub ai_act_high_risk: bool,
    /// DPIA required before execution ?
    pub dpia_required: bool,
}

/// Tenant identifier (UUID v4 from efe-server tenant table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(pub Uuid);

/// JSON Schema (draft 2020-12) for validation.
pub type JsonSchema = serde_json::Value;

/// MCP tool metadata (Sprint 7+ usage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolMetadata {
    pub mcp_name: String,           // e.g. "efe_nano_pdf_extract_text"
    pub mcp_description: String,
    pub visible_by_default: bool,
}
```

### 3.2 Swarm types (Sprint 3)

```rust
// efe-nano-sdk/src/types/swarm.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRequest {
    pub request_id: Uuid,
    pub tenant_id: TenantId,
    pub caller_agent_id: Option<String>,    // e.g. "claude-sonnet-4-5"
    pub operations: Vec<Operation>,
    pub max_parallelism: u32,                // safety cap (default 100)
    pub timeout_ms: u32,                     // overall batch (default 60000)
    pub idempotency_key: Option<String>,
    pub streaming: bool,                     // future Phase 3
    pub audit_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub op_id: String,                       // customer-chosen, regex ^[a-zA-Z0-9_-]{1,64}$
    pub agent: NanoAgentRef,
    pub input: OperationInput,
    pub depends_on: Vec<String>,             // op_ids
    pub timeout_ms: Option<u32>,             // override per-op
    pub retry: RetryPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum OperationInput {
    /// Literal JSON value.
    Literal { value: serde_json::Value },
    /// Reference to another op's output.
    Ref { op_id: String, json_path: String },
    /// Merge multiple references.
    Merge { sources: Vec<OperationInput> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u8,    // 1-5
    pub backoff_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResponse {
    pub request_id: Uuid,
    pub overall_status: SwarmStatus,
    pub results: std::collections::HashMap<String, OperationResult>,
    pub total_latency_ms: u32,
    pub total_cost_cents: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SwarmStatus {
    Success,
    PartialFailure,    // some ops failed
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub op_id: String,
    pub status: OpStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<OperationError>,
    pub latency_ms: u32,
    pub cost_cents: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,    // dependency failed
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationError {
    pub kind: ErrorKind,
    pub message: String,
    pub source_op_id: Option<String>,    // if dependency failed
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorKind {
    SchemaValidationFailed,
    ResourceExceeded,
    RuntimeError,
    DependencyFailed,
    BudgetExceeded,
    Timeout,
    AgentNotFound,
    InvalidDag,
    InternalError,
}
```

### 3.3 Runtime context (server-side, in `efe-nano`)

```rust
// efe-nano/src/runtime/context.rs (NOT exposed via SDK)

use efe_nano_sdk::types::*;

pub struct CallContext {
    pub tenant_id: TenantId,
    pub trace_id: TraceId,
    pub parent_trace_id: Option<TraceId>,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub deadline: tokio::time::Instant,
    pub audit: Arc<dyn AuditWriter>,
    pub budget: Arc<dyn BudgetCheck>,
    pub idempotency_cache: Arc<dyn IdempotencyCache>,
}

#[derive(Debug, Clone, Copy)]
pub struct TraceId(pub Uuid);
```

### 3.4 Catalog API

```rust
// efe-nano/src/catalog/mod.rs

#[async_trait::async_trait]
pub trait Catalog: Send + Sync + 'static {
    /// Get a nano agent by ref (resolves version constraint).
    async fn get(&self, r: &NanoAgentRef, tenant: TenantId) -> Result<Arc<NanoAgent>>;
    
    /// List nano agents (per-tenant filtered).
    async fn list(&self, tenant: TenantId, filter: ListFilter) -> Result<Vec<Arc<NanoAgent>>>;
    
    /// Search by description / category.
    async fn search(&self, tenant: TenantId, query: &str) -> Result<Vec<Arc<NanoAgent>>>;
    
    /// Publish a new version (customer-owned tenant only).
    async fn publish(&self, tenant: TenantId, agent: NanoAgent) -> Result<NanoAgentId>;
    
    /// Promote stability tier (admin-only).
    async fn promote(&self, id: &NanoAgentId, version: &NanoAgentVersion, to: StabilityTier) -> Result<()>;
    
    /// Soft delete (unpublish) a version (customer's own).
    async fn unpublish(&self, tenant: TenantId, id: &NanoAgentId, version: &NanoAgentVersion) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct ListFilter {
    pub category: Option<Category>,
    pub stability_tier_min: Option<StabilityTier>,
    pub include_efe_shipped: bool,
    pub include_tenant_owned: bool,
    pub include_community: bool,
    pub limit: u32,
    pub offset: u32,
}
```

### 3.5 Runtime API

```rust
// efe-nano/src/runtime/mod.rs

#[async_trait::async_trait]
pub trait Runtime: Send + Sync + 'static {
    /// Execute a single nano agent invocation.
    async fn execute(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse>;
    
    /// Health check.
    async fn health(&self) -> RuntimeHealth;
}

pub struct NanoAgentResponse {
    pub output: serde_json::Value,
    pub latency_ms: u32,
    pub cost_cents: f64,
    pub trace_id: TraceId,
    pub cache_hit: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    pub mode_a_alive: bool,
    pub mode_b_alive: bool,    // false in Phase 1
    pub mode_c_alive: bool,    // false in Phase 1
    pub pool_metrics: PoolMetrics,
}
```

### 3.6 Dispatcher API (Sprint 3)

```rust
// efe-nano/src/dispatcher/mod.rs

#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync + 'static {
    /// Execute a swarm of nano agent operations with DAG dependencies.
    async fn execute(&self, request: SwarmRequest) -> Result<SwarmResponse>;
    
    /// Validate a SwarmRequest without executing (dry-run).
    async fn validate(&self, request: &SwarmRequest) -> Result<DagPlan>;
}

#[derive(Debug, Clone)]
pub struct DagPlan {
    pub layers: Vec<Layer>,    // topological order
    pub total_ops: usize,
    pub max_layer_width: usize,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub depth: u32,
    pub op_ids: Vec<String>,
}
```

---

## 4. Sprint 1 — Catalog + NAR Mode A (Day-by-day)

**Dates target** : Jun 16-30, 2026 (10 working days)
**Team** : Founder + Senior Platform Engineer #1 (newly onboarded)
**Goal** : Catalog (in-memory) + Mode A runtime + 25+ tests

### 4.1 Sprint 1 Day-by-day

#### Day 1 (Jun 16, lundi) — Onboarding + kickoff

**Founder** :
- Onboarding Senior #1 (1h walk-through of M50 architecture)
- Sprint 1 kickoff meeting (1h)
- Pair-code on `efe-nano-sdk/src/types/mod.rs` skeleton

**Senior #1** :
- Read all M50 architecture + verification + roadmap (3h)
- Setup local dev environment + run all 174 PR #27 tests (1h)
- First PR : add Cargo.toml stub deps (jsonschema, semver, chrono)

**Deliverable** : Onboarding complete, dev env green, types skeleton.

#### Day 2 (Jun 17, mardi) — Types implementation

**Both** :
- Implement `NanoAgentId`, `NanoAgentVersion`, `NanoAgentRef`, `VersionReq`
- Validation logic (snake_case check, semver check)
- 10 unit tests for type construction + validation

**Files** : `efe-nano-sdk/src/types/identifier.rs`, `efe-nano-sdk/src/types/version.rs`.

**Deliverable** : PR #1 — `feat(efe-nano-sdk): identifier and version types`

#### Day 3 (Jun 18, mercredi) — Catalog types

**Both** :
- Implement `NanoAgent`, `Category`, `StabilityTier`, `ExecutionMode`,
  `DeterminismLevel`, `IdempotencyPolicy`
- Implement `ResourceLimits`, `AuditPolicy`, `ComplianceTags`
- `ResourceLimits::default_mode_a()` constructor
- 15 unit tests for serde round-trip + invariants

**Files** : `efe-nano-sdk/src/types/catalog.rs`, `efe-nano-sdk/src/types/limits.rs`.

**Deliverable** : PR #2 — `feat(efe-nano-sdk): catalog types`

#### Day 4 (Jun 19, jeudi) — In-memory catalog

**Senior #1** :
- Implement `InMemoryCatalog` in `efe-nano/src/catalog/in_memory.rs`
- Uses `dashmap::DashMap<(NanoAgentId, NanoAgentVersion), Arc<NanoAgent>>`
- Implement `Catalog::get`, `list`, `search`, `publish`, `unpublish`
- Version resolution (latest matching semver req)

**Founder** :
- Design schema validation strategy (`jsonschema` crate vs custom)
- Decide on JSON Schema draft (2020-12)
- Implement schema validation helper

**Deliverable** : PR #3 — `feat(efe-nano): in-memory catalog`

#### Day 5 (Jun 20, vendredi) — Catalog tests

**Both** :
- Write 25 integration tests for catalog operations :
  - Get by exact version
  - Get by version constraint (^1.0.0)
  - List per-tenant scoping
  - Publish + retrieve
  - Search by category
  - Unpublish (soft delete)
  - Edge cases : not found, invalid ID, etc.
- All tests pass

**Deliverable** : PR #4 — `test(efe-nano): catalog integration tests`

#### Day 6 (Jun 23, lundi) — Mode A runtime

**Founder** :
- Implement `runtime::pure_rust::PureRustRuntime`
- `tokio::task::spawn_blocking` for Rust function execution
- Schema validation before + after execute
- Resource limits enforcement (memory not checked Phase 1 — Sprint 4)
- Error handling (catch panics, convert to OperationError)

**Senior #1** :
- Design `CallContext` + `AuditWriter` + `BudgetCheck` + `IdempotencyCache` traits
- Implement stub impls : `LogOnlyAuditWriter`, `AlwaysAllowBudgetCheck`,
  `NoOpIdempotencyCache` (for Sprint 1 testing)

**Deliverable** : PR #5 — `feat(efe-nano): mode A pure-rust runtime`

#### Day 7 (Jun 24, mardi) — First nano agent : json_validate

**Both** :
- Implement `starter_pack::json_validate::JsonValidate`
- Registers itself in the catalog at module init
- Input schema : `{ "schema": JsonSchema, "value": any }`
- Output schema : `{ "valid": boolean, "errors": [string] }`
- Uses `jsonschema` crate
- 5 unit tests + 1 integration test (via PureRustRuntime)

**Deliverable** : PR #6 — `feat(efe-nano): starter pack json_validate`

#### Day 8 (Jun 25, mercredi) — Schema validation tests + perf bench

**Founder** :
- Add bench : `cargo bench --bench mode_a_throughput`
- Target : 1,000 calls/sec/node on basic JSON validate
- Measure : p50, p95, p99 latency

**Senior #1** :
- 10 unit tests for schema validation edge cases
- Error handling tests : panics, OOM (simulated), timeout

**Deliverable** : PR #7 — `perf(efe-nano): mode A bench + schema tests`

#### Day 9 (Jun 26, jeudi) — Integration with audit stub

**Both** :
- Implement `audit::stub::ConsoleAuditWriter` (logs to stderr)
- Implement `audit::stub::TempTableAuditWriter` (writes to `nano_agent_traces_phase1` table — Postgres setup deferred to Sprint 2)
- Wire audit into PureRustRuntime
- 5 tests for audit invocation

**Deliverable** : PR #8 — `feat(efe-nano): audit stub`

#### Day 10 (Jun 27, vendredi) — Sprint 1 retro + Sprint 2 planning

**Both** :
- Run full test suite : 174 PR #27 + 51 STABILITY + ~ 25 new = 250+ tests
- Confirm 0 regressions
- Sprint 1 retrospective (1h)
- Sprint 2 detailed planning (1h)
- Update Phase 1 architecture doc with Sprint 1 actuals

**Deliverable** : Sprint 1 closure report.

### 4.2 Sprint 1 success criteria

```
[ ] InMemoryCatalog operations work (get, list, search, publish, unpublish)
[ ] PureRustRuntime executes json_validate end-to-end
[ ] Schema validation strict (input + output)
[ ] Audit stub writes to log + temp table
[ ] Throughput Mode A : ≥ 500 calls/sec/node (interim, full 1k at Sprint 3)
[ ] Latency Mode A p99 : ≤ 100 ms (interim, ≤ 50 ms at Sprint 3)
[ ] 25+ new tests pass
[ ] 174 PR #27 + 51 STABILITY tests still green
[ ] All PRs merged, CI green
```

---

## 5. Sprint 2 — Postgres catalog + 5 starter agents (Day-by-day)

**Dates target** : Jul 1-15, 2026
**Team** : Founder + Senior #1
**Goal** : Postgres-backed catalog + 5 more starter agents + 30+ new tests

### 5.1 Sprint 2 Day-by-day

#### Day 1 (Jul 1, mardi) — Postgres migration v001

**Senior #1** :
- Write `efe-nano/migrations/v001_create_nano_agents.sql`
- Create tables : `nano_agents`, `nano_agent_versions`
- Indexes : (tenant_id, category), (id, version)
- Constraints : valid stability_tier, valid execution_mode

**Founder** :
- Setup local Postgres for testing
- Write `tests/common/postgres_helper.rs` for test setup

**Deliverable** : PR #9 — `feat(efe-nano): postgres migration v001`

#### Day 2 (Jul 2, mercredi) — Postgres catalog implementation

**Senior #1** :
- Implement `PostgresCatalog` in `efe-nano/src/catalog/postgres.rs`
- Uses `sqlx::PgPool`
- `get`, `list`, `search`, `publish`, `unpublish` operations
- Per-tenant row-level security (RLS) policy

**Deliverable** : PR #10 — `feat(efe-nano): postgres catalog implementation`

#### Day 3 (Jul 3, jeudi) — Catalog migration test + RLS verification

**Both** :
- Integration tests for PostgresCatalog
- Verify per-tenant isolation (tenant A cannot see tenant B's agents)
- Migration up/down tests
- 15 integration tests

**Deliverable** : PR #11 — `test(efe-nano): postgres catalog integration`

#### Day 4 (Jul 4, vendredi) — Starter agent : xml_validate

**Founder** :
- Implement `starter_pack::xml_validate::XmlValidate`
- Uses `xmlrs` or `quick-xml` crate
- Input : `{ "xml": string, "xsd_schema": string }`
- Output : `{ "valid": boolean, "errors": [string] }`
- 5 unit tests + 1 integration test

**Deliverable** : PR #12 — `feat(efe-nano): starter pack xml_validate`

#### Day 5 (Jul 7, lundi) — Starter agent : regex_extract

**Founder** :
- Implement `starter_pack::regex_extract::RegexExtract`
- Uses `regex` crate
- Input : `{ "text": string, "pattern": string }`
- Output : `{ "matches": [match] }` where match = `{ "groups": [...], "start": int, "end": int }`
- DOS protection : timeout via `regex::RegexBuilder::dfa_size_limit()`
- 5 unit tests

**Deliverable** : PR #13 — `feat(efe-nano): starter pack regex_extract`

#### Day 6 (Jul 8, mardi) — Starter agent : date_normalize

**Founder** :
- Implement `starter_pack::date_normalize::DateNormalize`
- Uses `chrono` + `chrono-humanize` (or `dateparser` crate)
- Input : `{ "input": string, "input_format": Option<string> }`
- Output : `{ "iso8601": string, "timestamp_ms": int, "tz_offset": string }`
- 5 unit tests

**Deliverable** : PR #14 — `feat(efe-nano): starter pack date_normalize`

#### Day 7 (Jul 9, mercredi) — Starter agent : currency_normalize

**Founder** :
- Implement `starter_pack::currency_normalize::CurrencyNormalize`
- Pure Rust parser (no external API for Sprint 2)
- Input : `{ "input": string }` (e.g. "1,234.56 USD" or "€1.234,56")
- Output : `{ "value": float, "currency_iso4217": string, "normalized": string }`
- Handles : US format, European format, Indian format (lakh/crore)
- 5 unit tests

**Deliverable** : PR #15 — `feat(efe-nano): starter pack currency_normalize`

#### Day 8 (Jul 10, jeudi) — Idempotency cache wiring

**Senior #1** :
- Wire `efe-cache` (M27) Redis-backed `IdempotencyCache`
- Hash(input) → cache for `ttl_sec` per nano agent policy
- 5 unit tests

**Deliverable** : PR #16 — `feat(efe-nano): idempotency cache wiring`

#### Day 9 (Jul 11, vendredi) — Catalog seeding for all 5 starter agents

**Both** :
- Migration script `v002_seed_starter_pack.sql` seeds initial catalog
- All 5 agents registered with TIER-1 + EFE author + correct schemas
- 5 integration tests verifying catalog → execution end-to-end

**Deliverable** : PR #17 — `feat(efe-nano): catalog seeding for starter pack`

#### Day 10 (Jul 14, lundi) — Performance check + Sprint 2 closure

**Both** :
- Run bench : `cargo bench --bench mode_a_throughput`
- Confirm ≥ 800 calls/sec/node (target 1k by Sprint 3)
- Sprint 2 retrospective
- Update Phase 1 architecture doc with Sprint 2 actuals

**Deliverable** : Sprint 2 closure report.

### 5.2 Sprint 2 success criteria

```
[ ] Postgres catalog operations work end-to-end
[ ] Per-tenant isolation verified (RLS)
[ ] 5 starter agents implemented : json_validate, xml_validate,
    regex_extract, date_normalize, currency_normalize
[ ] All 5 agents in catalog at TIER-1
[ ] 30+ new tests pass
[ ] Throughput Mode A : ≥ 800 calls/sec/node (interim)
[ ] 174 PR #27 + 51 STABILITY tests still green
[ ] Idempotency cache wired with efe-cache (M27)
```

---

## 6. Sprint 3 — Dispatcher v0 + 5 more agents (Day-by-day)

**Dates target** : Jul 15-Aug 1, 2026 (note : overlap with Sprint 2 last day)
**Team** : Founder + Senior #1
**Goal** : Dispatcher v0 + 5 more starter agents + 25+ new tests + Phase 1 gate

### 6.1 Sprint 3 Day-by-day

#### Day 1 (Jul 15, mardi) — Swarm types + dispatcher skeleton

**Senior #1** :
- Implement Swarm types in `efe-nano-sdk` :
  - `SwarmRequest`, `Operation`, `OperationInput`, `RetryPolicy`
  - `SwarmResponse`, `OperationResult`, `OpStatus`, `OperationError`, `ErrorKind`
  - `SwarmStatus`
- 10 serde round-trip tests

**Founder** :
- Skeleton `dispatcher::Dispatcher` trait + `DispatcherImpl` struct

**Deliverable** : PR #18 — `feat(efe-nano-sdk): swarm types`

#### Day 2 (Jul 16, mercredi) — DAG planner

**Senior #1** :
- Implement `dispatcher::dag::plan(request: &SwarmRequest) -> Result<DagPlan>`
- Topological sort (Kahn's algorithm)
- Cycle detection → returns `InvalidDag` error
- Layer assignment (max_layer_width tracked)
- 8 unit tests : valid DAGs, cyclic DAGs, disconnected DAGs, single-op DAGs

**Deliverable** : PR #19 — `feat(efe-nano): DAG planner with cycle detection`

#### Day 3 (Jul 17, jeudi) — Layer executor

**Senior #1** :
- Implement `dispatcher::executor::execute_layer(layer: &Layer, ctx) -> Result<Vec<OperationResult>>`
- Uses `tokio::task::JoinSet` for parallel execution
- Respects `max_parallelism` cap (bounded JoinSet)
- Handles per-op timeout
- 8 unit tests

**Deliverable** : PR #20 — `feat(efe-nano): parallel layer executor`

#### Day 4 (Jul 18, vendredi) — End-to-end dispatcher integration

**Both** :
- Implement `Dispatcher::execute` orchestrator :
  1. Validate `SwarmRequest` against schema
  2. Plan DAG
  3. Execute layer-by-layer
  4. Aggregate results
  5. Return `SwarmResponse`
- 10 integration tests : 100-op DAG, simple chain, branching, mixed errors

**Deliverable** : PR #21 — `feat(efe-nano): end-to-end dispatcher`

#### Day 5 (Jul 22, lundi) — Starter agent : pdf_extract_text

**Founder** :
- Implement `starter_pack::pdf_extract_text::PdfExtractText`
- Uses `pdf-extract` crate
- Input : `{ "pdf_bytes_base64": string, "max_pages": Option<u32> }`
- Output : `{ "text": string, "page_count": u32 }`
- 5 unit tests (use small PDF fixtures)

**Deliverable** : PR #22 — `feat(efe-nano): starter pack pdf_extract_text`

#### Day 6 (Jul 23, mardi) — Starter agent : html_extract_main_content

**Founder** :
- Implement `starter_pack::html_extract_main::HtmlExtractMain`
- Uses `scraper` or `readability` crate
- Input : `{ "html": string, "url": Option<string> }`
- Output : `{ "title": string, "main_text": string, "byline": Option<string> }`
- 5 unit tests

**Deliverable** : PR #23 — `feat(efe-nano): starter pack html_extract_main_content`

#### Day 7 (Jul 24, mercredi) — Starter agent : markdown_extract_headings

**Founder** :
- Implement `starter_pack::markdown_extract::MarkdownExtractHeadings`
- Uses `pulldown-cmark` crate
- Input : `{ "markdown": string }`
- Output : `{ "headings": [{ "level": u8, "text": string, "anchor": string }] }`
- 5 unit tests

**Deliverable** : PR #24 — `feat(efe-nano): starter pack markdown_extract_headings`

#### Day 8 (Jul 25, jeudi) — Starter agent : csv_normalize

**Founder** :
- Implement `starter_pack::csv_normalize::CsvNormalize`
- Uses `csv` crate
- Input : `{ "csv": string, "delimiter": Option<string>, "has_header": bool }`
- Output : `{ "headers": [string], "rows": [[string]], "row_count": u32 }`
- 5 unit tests

**Deliverable** : PR #25 — `feat(efe-nano): starter pack csv_normalize`

#### Day 9 (Jul 28, lundi) — Starter agent : sql_parse_safe

**Founder** :
- Implement `starter_pack::sql_parse_safe::SqlParseSafe`
- Uses `sqlparser` crate (RustSec audited)
- Input : `{ "sql": string, "dialect": "postgres"|"mysql"|"sqlite" }`
- Output : `{ "ast_json": JSON, "tables": [string], "is_safe_select_only": bool }`
- Refuses DROP, DELETE, UPDATE, INSERT (returns safe=false but parses)
- 5 unit tests

**Deliverable** : PR #26 — `feat(efe-nano): starter pack sql_parse_safe`

#### Day 10 (Jul 29, mardi) — Phase 1 gate verification

**Both** :
- Run full bench : 1k calls/sec/node target on Mode A
- Run all tests : ~ 250 (174 + 51 + ~ 80 new) + Phase 1 tests
- Phase 1 → Phase 2 gate review
- Sprint 3 retrospective
- Update architecture doc with Phase 1 actuals

**Deliverable** : Phase 1 gate report.

### 6.2 Sprint 3 success criteria

```
[ ] DagPlanner with cycle detection works
[ ] Layer executor parallelizes correctly
[ ] End-to-end dispatcher : 100-op SwarmRequest executes
[ ] 5 more starter agents : pdf, html, markdown, csv, sql (10 total)
[ ] All 10 starter agents at TIER-1
[ ] 25+ new tests pass (total 80+ for Phase 1)
[ ] Throughput Mode A : ≥ 1,000 calls/sec/node (GATE)
[ ] Latency Mode A p99 : ≤ 50 ms (GATE)
[ ] 174 PR #27 + 51 STABILITY tests still green
```

---

## 7. Postgres schema + migrations

### 7.1 v001_create_nano_agents.sql (Sprint 2 Day 1)

```sql
-- Migration v001 : create base nano_agents catalog
-- Author : EFE M50 Phase 1, Sprint 2 Day 1
-- Date   : 2026-07-01

BEGIN;

CREATE TABLE nano_agents (
    -- Primary key : composite (id, version) makes versions distinct
    id              TEXT NOT NULL,
    version         TEXT NOT NULL,
    PRIMARY KEY (id, version),
    
    -- Metadata
    description     TEXT NOT NULL CHECK (char_length(description) <= 256),
    author_type     TEXT NOT NULL CHECK (author_type IN ('efe', 'tenant', 'community')),
    author_tenant_id UUID NULL,    -- non-null only if author_type = 'tenant'
    author_community_name TEXT NULL,
    
    category        TEXT NOT NULL CHECK (category IN (
        'extraction', 'validation', 'classification', 'transformation',
        'lookup', 'entity_recognition', 'summarization', 'structured_extract',
        'other'
    )),
    stability_tier  TEXT NOT NULL CHECK (stability_tier IN (
        'tier_experimental', 'tier_1', 'tier_2', 'tier_audit'
    )),
    
    -- Execution
    execution_mode  TEXT NOT NULL CHECK (execution_mode IN (
        'pure_rust', 'local_model', 'llm_augmented'
    )),
    determinism     TEXT NOT NULL CHECK (determinism IN (
        'full_deterministic', 'probabilistic', 'side_effect'
    )),
    
    -- Schemas (JSON Schema draft 2020-12)
    input_schema    JSONB NOT NULL,
    output_schema   JSONB NOT NULL,
    error_schema    JSONB NULL,
    
    -- Idempotency
    idempotency_policy JSONB NOT NULL,    -- { "type": "auto", "ttl_sec": 86400 }
    
    -- Resource limits
    memory_mb       INTEGER NOT NULL CHECK (memory_mb > 0),
    cpu_ms          INTEGER NOT NULL CHECK (cpu_ms > 0),
    wall_clock_ms   INTEGER NOT NULL CHECK (wall_clock_ms > 0),
    llm_tokens      INTEGER NULL,
    network_egress_kb INTEGER NOT NULL DEFAULT 0,
    
    -- Cost + perf SLA
    cost_estimate_cents NUMERIC(10, 4) NOT NULL,
    latency_p99_ms_target INTEGER NOT NULL CHECK (latency_p99_ms_target > 0),
    
    -- Audit
    audit_include_input  BOOLEAN NOT NULL DEFAULT TRUE,
    audit_include_output BOOLEAN NOT NULL DEFAULT TRUE,
    audit_redact_pii     BOOLEAN NOT NULL DEFAULT FALSE,
    
    -- Compliance
    compliance_gdpr     BOOLEAN NOT NULL DEFAULT FALSE,
    compliance_hipaa    BOOLEAN NOT NULL DEFAULT FALSE,
    compliance_gxp      BOOLEAN NOT NULL DEFAULT FALSE,
    compliance_finma    BOOLEAN NOT NULL DEFAULT FALSE,
    compliance_ai_act_high_risk BOOLEAN NOT NULL DEFAULT FALSE,
    compliance_dpia_required    BOOLEAN NOT NULL DEFAULT FALSE,
    
    -- MCP metadata (Sprint 7+ usage)
    mcp_tool_metadata JSONB NULL,
    
    -- Audit metadata
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    soft_deleted_at TIMESTAMPTZ NULL,
    
    -- Constraints on author <-> tenant
    CONSTRAINT author_consistency CHECK (
        (author_type = 'efe' AND author_tenant_id IS NULL AND author_community_name IS NULL) OR
        (author_type = 'tenant' AND author_tenant_id IS NOT NULL AND author_community_name IS NULL) OR
        (author_type = 'community' AND author_tenant_id IS NULL AND author_community_name IS NOT NULL)
    )
);

-- Indexes
CREATE INDEX idx_nano_agents_tenant_category ON nano_agents (author_tenant_id, category)
    WHERE author_tenant_id IS NOT NULL;
CREATE INDEX idx_nano_agents_efe_category ON nano_agents (category)
    WHERE author_type = 'efe';
CREATE INDEX idx_nano_agents_stability_tier ON nano_agents (stability_tier);
CREATE INDEX idx_nano_agents_id ON nano_agents (id);
CREATE INDEX idx_nano_agents_soft_deleted ON nano_agents (soft_deleted_at)
    WHERE soft_deleted_at IS NOT NULL;

-- Full-text search on description
CREATE INDEX idx_nano_agents_description_fts
    ON nano_agents USING gin (to_tsvector('english', description));

-- Comments for psql \d+
COMMENT ON TABLE nano_agents IS 'M50 catalog : registry of all nano agents (EFE-shipped, tenant-published, community).';
COMMENT ON COLUMN nano_agents.id IS 'Unique snake_case identifier (≤ 64 chars).';
COMMENT ON COLUMN nano_agents.version IS 'Semver version string.';
COMMENT ON COLUMN nano_agents.stability_tier IS 'Promotion tier: tier_experimental → tier_1 → tier_2 → tier_audit.';
COMMENT ON COLUMN nano_agents.execution_mode IS 'pure_rust (Phase 1), local_model (Phase 2 Sprint 4), llm_augmented (Phase 2 Sprint 5).';

COMMIT;
```

### 7.2 v002_seed_starter_pack.sql (Sprint 2 Day 9)

```sql
-- Migration v002 : seed initial starter pack
-- Author : EFE M50 Phase 1, Sprint 2 Day 9
-- Date   : 2026-07-11

BEGIN;

INSERT INTO nano_agents (
    id, version, description, author_type, author_tenant_id, author_community_name,
    category, stability_tier, execution_mode, determinism,
    input_schema, output_schema, error_schema,
    idempotency_policy,
    memory_mb, cpu_ms, wall_clock_ms, llm_tokens, network_egress_kb,
    cost_estimate_cents, latency_p99_ms_target,
    audit_include_input, audit_include_output, audit_redact_pii,
    compliance_gdpr, compliance_hipaa, compliance_gxp, compliance_finma,
    compliance_ai_act_high_risk, compliance_dpia_required,
    mcp_tool_metadata, created_at
) VALUES
(
    'json_validate', '1.0.0',
    'Validate a JSON value against a JSON Schema (draft 2020-12).',
    'efe', NULL, NULL,
    'validation', 'tier_1', 'pure_rust', 'full_deterministic',
    '{"type": "object", "required": ["schema", "value"], "properties": {"schema": {"type": "object"}, "value": {}}}'::jsonb,
    '{"type": "object", "required": ["valid", "errors"], "properties": {"valid": {"type": "boolean"}, "errors": {"type": "array", "items": {"type": "string"}}}}'::jsonb,
    NULL,
    '{"type": "auto", "ttl_sec": 86400}'::jsonb,
    256, 1000, 5000, NULL, 0,
    0.01, 50,
    TRUE, TRUE, FALSE,
    FALSE, FALSE, FALSE, FALSE, FALSE, FALSE,
    NULL, NOW()
),
-- xml_validate, regex_extract, date_normalize, currency_normalize similar...
;

COMMIT;
```

### 7.3 v003_idempotency_index.sql (Sprint 3 if needed)

```sql
-- Migration v003 : optional index for idempotency cache hits
-- Only if benchmark shows lookup bottleneck

BEGIN;

CREATE INDEX IF NOT EXISTS idx_nano_agents_lookup_hot
    ON nano_agents (id, version) INCLUDE (
        execution_mode, input_schema, output_schema, memory_mb, cpu_ms,
        wall_clock_ms, audit_include_input, audit_include_output
    )
    WHERE soft_deleted_at IS NULL;

COMMIT;
```

---

## 8. Error handling strategy

### 8.1 Error type hierarchy

```rust
// efe-nano/src/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NanoAgentError {
    #[error("schema validation failed : {0}")]
    SchemaValidation(#[from] SchemaValidationError),
    
    #[error("resource exceeded : {0}")]
    ResourceExceeded(ResourceKind),
    
    #[error("runtime error : {message}")]
    Runtime { message: String },
    
    #[error("catalog operation failed : {0}")]
    Catalog(#[from] CatalogError),
    
    #[error("dispatcher error : {0}")]
    Dispatcher(#[from] DispatcherError),
    
    #[error("budget exceeded for tenant {tenant_id}")]
    BudgetExceeded { tenant_id: TenantId, remaining_cents: f64 },
    
    #[error("agent not found : {id} v{version}")]
    AgentNotFound { id: String, version: String },
    
    #[error("database error : {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("internal error : {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("invalid agent ID : {0}")]
    InvalidId(#[from] NanoAgentIdError),
    
    #[error("version constraint not satisfied for {id} : {req}")]
    VersionNotFound { id: String, req: String },
    
    #[error("tenant {0} not authorized to publish to id {1}")]
    Unauthorized(TenantId, NanoAgentId),
    
    #[error("agent {0} already exists at version {1}")]
    AlreadyExists(NanoAgentId, NanoAgentVersion),
}

#[derive(Debug, Error)]
pub enum DispatcherError {
    #[error("DAG validation failed : {reason}")]
    InvalidDag { reason: String },
    
    #[error("cycle detected involving ops : {ops:?}")]
    CycleDetected { ops: Vec<String> },
    
    #[error("max parallelism exceeded : {0}")]
    TooParallel(u32),
    
    #[error("swarm timed out after {0} ms")]
    Timeout(u32),
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceKind {
    Memory,
    Cpu,
    WallClock,
    NetworkEgress,
}
```

### 8.2 Error propagation pattern

```
[ User HTTP call ] 
       │
       ▼
[ efe-server /v1/swarm handler ]
       │
       ├── 200 OK + SwarmResponse (success)
       └── HTTP error code based on NanoAgentError:
           - 400 : SchemaValidation, InvalidDag, CycleDetected
           - 401 : Unauthorized (catalog)
           - 402 : BudgetExceeded
           - 404 : AgentNotFound
           - 408 : Timeout
           - 422 : ResourceExceeded
           - 500 : Internal, Database, Runtime
```

### 8.3 Per-operation error handling

In a `SwarmResponse`, individual ops can fail without aborting the whole batch :

```rust
SwarmResponse {
    overall_status: SwarmStatus::PartialFailure,
    results: hashmap!{
        "op-1".to_string() => OperationResult { status: OpStatus::Success, ... },
        "op-2".to_string() => OperationResult { 
            status: OpStatus::Failed,
            error: Some(OperationError { kind: ErrorKind::SchemaValidationFailed, ... }),
            ...
        },
        "op-3".to_string() => OperationResult { 
            status: OpStatus::Skipped,
            error: Some(OperationError { kind: ErrorKind::DependencyFailed, source_op_id: Some("op-2".to_string()), ... }),
            ...
        },
    },
    ...
}
```

The dispatcher continues executing **independent ops** even if some
fail.  Ops depending on a failed op are marked `Skipped`.

---

## 9. Performance contracts

### 9.1 Phase 1 → Phase 2 gate performance targets

| Metric | Target | Measurement method |
|--------|--------|--------------------|
| Mode A throughput | ≥ 1,000 calls/sec/node | `cargo bench --bench mode_a_throughput` |
| Mode A latency p50 | ≤ 5 ms | per-call histogram |
| Mode A latency p99 | ≤ 50 ms | per-call histogram |
| Catalog get p99 | ≤ 20 ms | postgres query with index |
| DAG planner p99 | ≤ 50 ms for 100-op DAG | unit benchmark |
| Layer executor overhead | ≤ 10 ms per layer | per-layer timer |

### 9.2 Benchmark suite

```
benches/mode_a_throughput.rs        Mode A throughput / latency
benches/catalog_lookup.rs            Catalog read perf
benches/dispatcher_dag.rs            DAG plan + execute for 10/100/1000-op
benches/schema_validation.rs         input/output schema cost
```

Run via :
```bash
cargo bench -p efe-nano --bench mode_a_throughput
cargo bench -p efe-nano --bench catalog_lookup
cargo bench -p efe-nano --bench dispatcher_dag
cargo bench -p efe-nano --bench schema_validation
```

### 9.3 Profiling strategy

If targets missed at end of Sprint 3 :

1. `perf record` + `flamegraph` to identify hot paths
2. Common bottlenecks expected :
   - JSON Schema validation (consider `jsonschema-rs` vs `jsonschema`)
   - Tokio task scheduling overhead (consider `tokio-uring` for IO)
   - DashMap contention in catalog (consider sharding)
3. Buffer 1 sprint allocated for perf hardening (Sprint 10) if needed

---

## 10. Integration matrix with existing EFE crates

| Existing crate | M50 uses it for | Phase 1 sprint | Surface used |
|----------------|------------------|----------------|---------------|
| `efe-cache` (M27) | Idempotency cache | Sprint 2-3 | `EmbeddingCache::put/get` pattern reused |
| `efe-server` (M10) | HTTP/gRPC routing | Sprint 3 | New `/v1/swarm` POST endpoint |
| `efe-curation` (M23) | (deferred Sprint 6) | — | Will extend HMAC chain |
| `efe-agent-runner` (M25) | (deferred Sprint 4) | — | Will reuse `FirecrackerDriver` |
| `efe-llm-router` (M21) | (deferred Sprint 5) | — | Will use `LLMRouter::call` |
| `efe-pii` (M29) | (deferred Sprint 5) | — | Will use redaction API |
| `efe-events` (M28) | Event emission | Sprint 3 | New `NanoAgentCallCompleted` event type |
| `efe-metering` (M31) | (deferred Sprint 7) | — | Per-call metering |
| `efe-billing` (M31) | (deferred Sprint 11) | — | Stripe integration |
| `efe-active-learning` (M24) | (consumer of M50) | — | Will subscribe to M50 events |

### 10.1 New efe-server endpoint (Sprint 3)

```rust
// efe-server/src/handlers/nano_swarm.rs (NEW FILE — additive)

use axum::{Json, http::StatusCode};
use efe_nano::dispatcher::Dispatcher;
use efe_nano_sdk::types::{SwarmRequest, SwarmResponse};

pub async fn nano_swarm_handler(
    auth: AuthContext,
    Json(request): Json<SwarmRequest>,
) -> Result<Json<SwarmResponse>, ApiError> {
    let dispatcher = SHARED_DISPATCHER.get().unwrap();
    let response = dispatcher.execute(request).await?;
    Ok(Json(response))
}

pub fn register_routes(router: Router) -> Router {
    router.route("/v1/swarm", post(nano_swarm_handler))
}
```

**Critical** : `efe-server`'s existing handlers/routes are **unchanged**.
The new `/v1/swarm` route is **appended** via a new module.

---

## 11. CI / build integration

### 11.1 Workflow changes

```
.github/workflows/ci.yml additions (ADDITIVE) :

- name: efe-nano-sdk tests
  run: cargo test -p efe-nano-sdk --all-features

- name: efe-nano tests
  run: cargo test -p efe-nano --all-features
  
- name: efe-nano bench (only on tag)
  if: startsWith(github.ref, 'refs/tags/')
  run: cargo bench -p efe-nano

- name: Phase 1 additivity check
  run: |
    # Confirm 174 PR #27 tests + 51 STABILITY tests still pass
    cargo test -p efe-bench-harness \
      --test g11_round_trip \
      --test g11_tolerance_used_populated \
      --test stability_contract_v17 \
      --test backfill_edge_cases \
      --test pr27_workflow_yaml_invariants \
      --test s3_audit_chain_integrity \
      --test s9_s12_monitoring_chain_integrity
```

### 11.2 sqlx migration in CI

```yaml
- name: Run sqlx migrations
  run: |
    # Postgres service (already in CI)
    cd crates/efe-nano
    sqlx migrate run --database-url $TEST_DATABASE_URL
```

---

## 12. Phase 1 → Phase 2 gate criteria

```
[ ] All 10 starter nano agents implemented and TIER-1
    ├ Sprint 2 : json_validate, xml_validate, regex_extract,
    │           date_normalize, currency_normalize
    └ Sprint 3 : pdf_extract_text, html_extract_main,
                 markdown_extract, csv_normalize, sql_parse_safe

[ ] Catalog operational
    ├ In-memory (Sprint 1)
    ├ Postgres-backed (Sprint 2)
    └ Per-tenant RLS verified

[ ] Mode A runtime operational
    ├ tokio thread pool executor
    ├ schema validation pre/post
    ├ resource limits enforced
    └ audit stub writing

[ ] Dispatcher v0 operational
    ├ DAG planner with cycle detection
    ├ Parallel layer executor
    └ End-to-end SwarmRequest → SwarmResponse

[ ] Performance targets
    ├ Mode A throughput ≥ 1k calls/sec/node
    ├ Mode A latency p99 ≤ 50 ms
    └ Catalog get p99 ≤ 20 ms

[ ] Test coverage
    ├ ~ 80 new tests added across efe-nano + efe-nano-sdk
    ├ 174 PR #27 + 51 STABILITY tests still green
    └ Bench suite operational

[ ] Senior #2 hire signed (gate to Phase 2 hire timing)

[ ] Architecture doc updated with Phase 1 actuals
```

---

## 13. Test catalogue (~ 80 new tests)

### 13.1 Per-module test count target

```
efe-nano-sdk tests :
  types/identifier_test.rs         8 tests
  types/version_test.rs            5 tests
  types/catalog_test.rs           10 tests
  types/swarm_test.rs              7 tests
                                  ──
                                  30 tests

efe-nano tests :
  catalog_in_memory_test.rs       12 tests
  catalog_postgres_test.rs        15 tests
  runtime_pure_rust_test.rs       10 tests
  runtime_limits_test.rs           5 tests
  dispatcher_dag_test.rs            8 tests
  dispatcher_executor_test.rs       8 tests
  dispatcher_e2e_test.rs           10 tests
  starter_pack_test.rs           20 tests (~ 2 per agent × 10 agents)
                                  ──
                                  88 tests

GRAND TOTAL Phase 1 new tests : ~ 118 tests
```

(118 > 80 estimate ; refined upward during planning.)

### 13.2 Test categories

- **Unit tests** : pure logic, no I/O
- **Integration tests** : with Postgres test container
- **Property tests** : `proptest` for DAG cycle detection robustness
- **Benchmarks** : `criterion` for performance measurement

### 13.3 Coverage target

- Phase 1 code coverage : **≥ 80 % line coverage**
- Critical paths (dispatcher, runtime) : **≥ 90 %**
- Schema validation : **100 %** (all branches covered)

---

## 14. Risk register specific to Phase 1

### 14.1 High-impact risks

| Risk | Probability | Mitigation |
|------|-------------|------------|
| Senior #1 onboarding slower than expected | Medium | Pair-coding Day 1-2, gradual ownership transfer |
| Postgres RLS too restrictive for catalog | Low | Test RLS thoroughly Sprint 2 Day 3 |
| `jsonschema` crate too slow | Medium | Profile early, consider `jsonschema-rs` as fallback |
| DAG cycle detection edge cases | Medium | Property tests with `proptest` |
| 1k calls/sec/node not reached | Medium | Profile Sprint 3 Day 10, optimize Sprint 10 buffer |

### 14.2 Medium-impact risks

| Risk | Probability | Mitigation |
|------|-------------|------------|
| `pdf-extract` crate licensing issue | Low | Audit licenses Sprint 3 Day 5 |
| Idempotency cache collisions | Low | Hash with SHA-256 (collision-resistant) |
| Postgres connection pool tuning | Medium | Use `sqlx` defaults; tune in Phase 4 |
| Senior #2 hire late (Phase 2 prep) | Medium | Start search Sprint 2 |

### 14.3 Low-impact risks

| Risk | Probability | Mitigation |
|------|-------------|------------|
| CI Postgres flakiness | Low | Use `testcontainers-rs` for isolated CI runs |
| `cargo bench` overhead in CI | Low | Only run on tag releases |

---

## 15. Annexe A — Complete Rust trait definitions

```rust
// All public traits used in Phase 1.
// See efe-nano-sdk/src/traits.rs for the canonical definitions.

#[async_trait::async_trait]
pub trait Catalog: Send + Sync + 'static {
    async fn get(&self, r: &NanoAgentRef, tenant: TenantId) -> Result<Arc<NanoAgent>>;
    async fn list(&self, tenant: TenantId, filter: ListFilter) -> Result<Vec<Arc<NanoAgent>>>;
    async fn search(&self, tenant: TenantId, query: &str) -> Result<Vec<Arc<NanoAgent>>>;
    async fn publish(&self, tenant: TenantId, agent: NanoAgent) -> Result<NanoAgentId>;
    async fn unpublish(&self, tenant: TenantId, id: &NanoAgentId, v: &NanoAgentVersion) -> Result<()>;
    async fn promote(&self, id: &NanoAgentId, v: &NanoAgentVersion, to: StabilityTier) -> Result<()>;
}

#[async_trait::async_trait]
pub trait Runtime: Send + Sync + 'static {
    async fn execute(&self, agent: &NanoAgent, input: serde_json::Value, ctx: &CallContext) -> Result<NanoAgentResponse>;
    async fn health(&self) -> RuntimeHealth;
}

#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync + 'static {
    async fn execute(&self, request: SwarmRequest) -> Result<SwarmResponse>;
    async fn validate(&self, request: &SwarmRequest) -> Result<DagPlan>;
}

#[async_trait::async_trait]
pub trait AuditWriter: Send + Sync + 'static {
    async fn record(&self, trace: NanoAgentTrace) -> Result<()>;
}

#[async_trait::async_trait]
pub trait BudgetCheck: Send + Sync + 'static {
    async fn precharge(&self, tenant: TenantId, cents: f64) -> Result<PrechargeReceipt>;
    async fn settle(&self, receipt: PrechargeReceipt, actual_cents: f64) -> Result<()>;
}

#[async_trait::async_trait]
pub trait IdempotencyCache: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>>;
    async fn put(&self, key: String, value: serde_json::Value, ttl_sec: u32) -> Result<()>;
}

/// Trait every nano agent must implement (proc-macro generated in Sprint 8).
#[async_trait::async_trait]
pub trait NanoAgentImpl: Send + Sync + 'static {
    async fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value, OperationError>;
    fn manifest(&self) -> &NanoAgent;
}
```

---

## 16. Annexe B — Postgres migration SQL (already shown § 7)

See section 7 above for full SQL.  Files :
- `crates/efe-nano/migrations/v001_create_nano_agents.sql`
- `crates/efe-nano/migrations/v002_seed_starter_pack.sql`
- `crates/efe-nano/migrations/v003_idempotency_index.sql` (if needed)

---

## 17. Annexe C — Per-nano-agent detailed specs

### 17.1 json_validate

```yaml
id: json_validate
version: "1.0.0"
description: Validate a JSON value against a JSON Schema (draft 2020-12).
category: validation
stability_tier: tier_1
execution_mode: pure_rust
determinism: full_deterministic

input_schema:
  type: object
  required: [schema, value]
  properties:
    schema: { type: object }       # JSON Schema (any draft)
    value: {}                       # any JSON value

output_schema:
  type: object
  required: [valid, errors]
  properties:
    valid: { type: boolean }
    errors:
      type: array
      items: { type: string }

limits:
  memory_mb: 256
  cpu_ms: 1000
  wall_clock_ms: 5000

cost_estimate_cents: 0.01
latency_p99_ms_target: 50

implementation_notes:
  - Use `jsonschema` crate (RustSec audited)
  - Compile JSON Schema once per call (cache compiled schemas?)
  - Return all errors, not just first
```

### 17.2 xml_validate

```yaml
id: xml_validate
version: "1.0.0"
description: Validate XML against an XSD schema.
category: validation
stability_tier: tier_1
execution_mode: pure_rust
determinism: full_deterministic

input_schema:
  type: object
  required: [xml]
  properties:
    xml: { type: string }
    xsd_schema: { type: string, nullable: true }    # optional XSD

output_schema:
  type: object
  required: [valid, well_formed, errors]
  properties:
    valid: { type: boolean }       # both well-formed AND schema-valid
    well_formed: { type: boolean }
    errors: { type: array, items: { type: string } }

limits: same as json_validate
cost_estimate_cents: 0.01
latency_p99_ms_target: 80
```

### 17.3 regex_extract

```yaml
id: regex_extract
version: "1.0.0"
description: Run a regex against text and return matches with groups.
category: extraction
stability_tier: tier_1
execution_mode: pure_rust
determinism: full_deterministic

input_schema:
  type: object
  required: [text, pattern]
  properties:
    text: { type: string }
    pattern: { type: string }
    case_insensitive: { type: boolean, default: false }
    multiline: { type: boolean, default: false }

output_schema:
  type: object
  required: [matches]
  properties:
    matches:
      type: array
      items:
        type: object
        properties:
          groups: { type: array, items: { type: string } }
          start: { type: integer }
          end: { type: integer }

limits:
  memory_mb: 128
  cpu_ms: 500     # tight to prevent regex DoS
  wall_clock_ms: 2000

cost_estimate_cents: 0.005
latency_p99_ms_target: 30

implementation_notes:
  - Use `regex` crate with dfa_size_limit = 1 MB
  - Compile regex once per call
  - DoS protection : reject patterns with backtracking
```

### 17.4 date_normalize

```yaml
id: date_normalize
version: "1.0.0"
description: Parse a date string and emit ISO 8601 + timestamp.
category: transformation
stability_tier: tier_1
execution_mode: pure_rust
determinism: full_deterministic

input_schema:
  type: object
  required: [input]
  properties:
    input: { type: string }
    input_format: { type: string, nullable: true }    # optional strptime hint
    target_tz: { type: string, default: "UTC" }

output_schema:
  type: object
  required: [iso8601, timestamp_ms]
  properties:
    iso8601: { type: string }       # e.g. "2026-05-18T10:30:00Z"
    timestamp_ms: { type: integer }
    tz_offset: { type: string }     # e.g. "+02:00"

cost_estimate_cents: 0.001
latency_p99_ms_target: 10

implementation_notes:
  - Use `chrono` + `dateparser`
  - Fallback to heuristic detection
```

### 17.5 currency_normalize

```yaml
id: currency_normalize
version: "1.0.0"
description: Parse a currency string (US/EU/Indian format) and emit numeric value + ISO 4217 currency.
category: transformation
stability_tier: tier_1
execution_mode: pure_rust
determinism: full_deterministic

input_schema:
  type: object
  required: [input]
  properties:
    input: { type: string }       # e.g. "$1,234.56" or "€1.234,56" or "₹1,23,456"

output_schema:
  type: object
  required: [value, currency_iso4217, normalized]
  properties:
    value: { type: number }
    currency_iso4217: { type: string }    # e.g. "USD", "EUR", "INR"
    normalized: { type: string }          # canonical form

cost_estimate_cents: 0.001
latency_p99_ms_target: 5

implementation_notes:
  - Pure Rust parser (no external API)
  - Handles : US, EU, Indian formats
  - Symbol detection : $, €, £, ₹, ¥, ₽, etc.
  - Returns "UNKNOWN" for currency if no symbol detected
```

### 17.6-17.10 Sprint 3 nano agents (similar detail)

For brevity, the remaining 5 nano agents follow the same template :
- `pdf_extract_text` (uses `pdf-extract` crate, latency p99 200 ms)
- `html_extract_main_content` (uses `readability` crate, latency p99 100 ms)
- `markdown_extract_headings` (uses `pulldown-cmark`, latency p99 10 ms)
- `csv_normalize` (uses `csv` crate, latency p99 50 ms)
- `sql_parse_safe` (uses `sqlparser`, latency p99 30 ms)

Each ships with input/output schemas, resource limits, cost estimates,
and 5-10 unit + integration tests.

---

## 18. Annexe D — Sprint daily plans (30 working days)

```
Phase 1 — 30 working days total

Sprint 1 (Jun 16-27, 10 days)
─────────────────────────────
Day 1   Mon Jun 16   Onboarding + types skeleton
Day 2   Tue Jun 17   NanoAgentId, Version, Ref types + 10 tests
Day 3   Wed Jun 18   Catalog types (NanoAgent, etc.) + 15 tests
Day 4   Thu Jun 19   InMemoryCatalog impl
Day 5   Fri Jun 20   Catalog integration tests (25 total)
Day 6   Mon Jun 23   PureRustRuntime impl
Day 7   Tue Jun 24   First agent (json_validate) + 5 tests
Day 8   Wed Jun 25   Mode A bench + 10 schema validation tests
Day 9   Thu Jun 26   Audit stub + integration
Day 10  Fri Jun 27   Sprint 1 retro + closure

Sprint 2 (Jul 1-14, 10 days)
────────────────────────────
Day 1   Tue Jul 1    Postgres migration v001
Day 2   Wed Jul 2    PostgresCatalog impl
Day 3   Thu Jul 3    Postgres tests + RLS verification (15 tests)
Day 4   Fri Jul 4    xml_validate (5 tests)
Day 5   Mon Jul 7    regex_extract (5 tests)
Day 6   Tue Jul 8    date_normalize (5 tests)
Day 7   Wed Jul 9    currency_normalize (5 tests)
Day 8   Thu Jul 10   Idempotency cache wiring
Day 9   Fri Jul 11   Catalog seeding migration v002
Day 10  Mon Jul 14   Sprint 2 retro + closure

Sprint 3 (Jul 15-Aug 1, 10 days)
────────────────────────────────
Day 1   Tue Jul 15   Swarm types + dispatcher skeleton (10 tests)
Day 2   Wed Jul 16   DAG planner + cycle detection (8 tests)
Day 3   Thu Jul 17   Layer executor (8 tests)
Day 4   Fri Jul 18   End-to-end dispatcher (10 tests)
Day 5   Mon Jul 22   pdf_extract_text (5 tests)
Day 6   Tue Jul 23   html_extract_main_content (5 tests)
Day 7   Wed Jul 24   markdown_extract_headings (5 tests)
Day 8   Thu Jul 25   csv_normalize (5 tests)
Day 9   Mon Jul 28   sql_parse_safe (5 tests)
Day 10  Tue Jul 29   Phase 1 gate verification + retro
```

---

## 19. Annexe E — Decision log

| ID | Decision | Phase 1 sprint | Rationale |
|----|----------|----------------|-----------|
| D-M50-P1-001 | Use `jsonschema` crate for schema validation | Sprint 1 | RustSec audited, draft 2020-12 support |
| D-M50-P1-002 | InMemoryCatalog before PostgresCatalog | Sprint 1 | Faster iteration, swap in Sprint 2 |
| D-M50-P1-003 | Postgres row-level security for per-tenant | Sprint 2 | Native Postgres feature, well-tested |
| D-M50-P1-004 | `regex` crate (not `regress`) | Sprint 2 | Standard, performant, DoS-resistant |
| D-M50-P1-005 | `pdf-extract` crate for PDF | Sprint 3 | Pure Rust, no native dep |
| D-M50-P1-006 | Tokio JoinSet for parallel layer exec | Sprint 3 | Idiomatic, bounded |
| D-M50-P1-007 | Kahn's algorithm for topological sort | Sprint 3 | O(V+E), simple to implement |
| D-M50-P1-008 | Idempotency cache via `efe-cache` (M27) | Sprint 2 | Reuse existing M27 Redis infra |
| D-M50-P1-009 | New `/v1/swarm` endpoint in `efe-server` | Sprint 3 | Additive, no existing endpoint modified |

---

## 20. Sign-off & next steps

### 20.1 Phase 1 deliverables checklist

```
At end of Sprint 3 (Aug 1, 2026) :

[ ] crates/efe-nano-sdk/                ~ 1,500 lines Rust
    └── 12 public types, 30 tests
[ ] crates/efe-nano/                     ~ 3,500 lines Rust
    └── 30+ files, 88 tests
[ ] Postgres migrations                  3 SQL files (v001, v002, v003?)
[ ] 10 starter nano agents (all TIER-1)
[ ] /v1/swarm endpoint in efe-server (additive)
[ ] CI workflow updated (additive)
[ ] Phase 1 → Phase 2 gate report
[ ] Architecture doc v2 (this file with actuals)
```

### 20.2 What follows (Phase 2 preview)

After Phase 1 gate :
- Sprint 4 : NAR Mode B (Firecracker via M25 reuse) + 3 local model agents
- Sprint 5 : NAR Mode C (LLM via M21 reuse) + 2 LLM-augmented + 5 mixed agents
- Sprint 6 : Audit chain integration (M23 HMAC) + replay capability

Phase 2 doubles the team to 3 (Senior #2 onboards Sprint 4 Day 1).

### 20.3 Decision board for Phase 1 → 2 transition

The Phase 1 → 2 gate review (Aug 1) decides :
- **GO**  : all criteria met → Sprint 4 starts Aug 11
- **NO-GO** : 1-2 criteria missed → 1 sprint of catch-up, retest
- **PIVOT** : fundamental issue → strategic review with board

---

## 21. References

- M50 Architecture : `EFE_M50_AGENTIC_ARCHITECTURE.md`
- M50 Verification Report : `EFE_M50_VERIFICATION_REPORT_2026-05-18.md`
- M50 Sprint 0 Validation : `EFE_M50_SPRINT_0_VALIDATION_REPORT.md`
- M-Number Registry : `EFE_M_NUMBER_REGISTRY.md`
- M50 Roadmap : `EFE_M50_ROADMAP.md`
- Decisions :
  - `D-M50-INFRA-001.md` (Postgres+Redis)
  - `D-M50-CRATES-001.md` (2-crate split)
- Firecracker Runbook : `EFE_M50_FIRECRACKER_RUNBOOK.md`
- MCP Tools Inventory : `MCP_TOOLS_INVENTORY_2026-06.md`

---

_End of Phase 1 architecture document._
_Phase 1 spans Sprints 1-3 (Jun 16 - Aug 1, 2026)._
_Companion : verification report to be drafted during Sprint 3 Day 10._

_Total : ~ 1,600 lignes (this document)._
_Total Rust + SQL added during Phase 1 : ~ 5,000 + 100 lines._
_Total tests added during Phase 1 : ~ 118 tests._
_Total existing tests modified : 0 (strict additivity)._
