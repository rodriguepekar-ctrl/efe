# EFE M50 — Agentic Architecture (Nano Agent Swarm)

**Status**         : DRAFT v1
**Date drafted**   : 2026-05-18
**Author**         : Rodrigue Pekar (Founding Engineer)
**Companion docs** :
- [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) (target § 30)
- [`docs/EFE_CORE_BENCHMARK_CATALOGUE.md`](../EFE_CORE_BENCHMARK_CATALOGUE.md) (target CB-50_*)
- [`docs/marketing/EFE-demo-EXP6-agent-2026-05-18.md`](../marketing/EFE-demo-EXP6-agent-2026-05-18.md) (the wrapper PoC)

**Module ID**      : **M50 — Nano Agent Swarm (NAS)**
**Codename**       : "EFE Agentic"
**Stability tier** : TIER-EXPERIMENTAL → TIER-1 (after PR-30 closure)

**Contrat additif strict** :
> Ce module M50 est **purement additif**.  Il **n'enlève** et **ne modifie**
> aucun fichier des 33 crates existants, des 174 tests PR #27 scope, ou du
> STABILITY contract v1.7 (qui passera à v1.8 avec l'ajout de § 30).  Il ajoute :
> - **2 nouvelles crates** : `efe-nano` (runtime + dispatcher + catalog) et
>   `efe-nano-sdk` (SDK public Rust + Python + TS via FFI)
> - 1 nouvelle section au STABILITY contract (§ 30) — bump v1.7 → v1.8
> - 1 nouvelle CB scenario family (CB-50_nano_*)
> - 1 nouveau M-number (M50) au catalogue
> - 1 extension au MCP server existant (M16) — ajout de tools nano agents,
>   tools existants (`efe_classify`, `efe_register_catalog`, et le reste du
>   surface actuel) inchangés
> - Note infra : assume Postgres (déjà utilisé par M23), ClickHouse (nouvelle
>   dépendance infra à provisionner), Stripe (feature flag présent dans
>   `efe-billing`).  Validation infra opérationnelle à faire en Sprint 0.
>
> Tous les composants existants restent **byte-identical**.  Le freeze
> test `stability_contract_v17` continue à passer 51/51 après ajout de M50.

---

## Table des matières

0. Executive summary
1. Le problème de marché qu'on résout
2. La thèse architecturale
3. Vue d'ensemble — les 4 layers d'EFE après M50
4. Composants détaillés de M50
   - 4.1 Nano Agent Catalog (NAC)
   - 4.2 Nano Agent Runtime (NAR)
   - 4.3 Swarm Dispatcher (SD)
   - 4.4 MCP Bridge Extension (MCB)
   - 4.5 Audit Trace per Call (ATC)
   - 4.6 Usage Metering & Billing (UMB)
   - 4.7 Developer SDK & CLI
5. Matrice d'intégration avec les composants existants
6. Data models (types Rust + JSON schemas)
7. Data flow examples
8. Performance characteristics
9. Security & isolation model
10. Compliance & audit
11. Pricing & business model
12. Open source strategy
13. Implementation roadmap (~ 6.5 mois, 14 sprints × 2 sem)
14. Breaking changes audit (preuve d'additivité)
15. Les 20 nano agents starter pack
16. Customer authoring path
17. Testing strategy
18. Failure modes & recovery
19. Glossary
20. Decisions log

---

## 0. Executive summary

EFE M50 ajoute une **4ème couche** au-dessus de l'infrastructure
EFE existante : un **essaim de nano agents** que les agents IA des
clients (Claude, GPT-4, Gemini, agents custom) **appellent via MCP
pour accélérer leurs sous-tâches déterministes**.

**Thèse en une phrase** :

> *Les agents IA en production sont lents et chers parce qu'ils
> appellent un LLM pour chaque sous-tâche.  EFE leur fournit un
> essaim de nano agents spécialisés et déterministes, exécutables
> en parallèle, avec audit-trail par appel.  Résultat mesurable :
> 10-40× speedup, 50-70 % cost reduction, et audit-grade by
> construction.*

**Ce module change strictement** :
- Le **positionnement** : EFE devient "B2A" (Business-to-Agent) en plus
  de B2B classique
- Le **business model** : ajout d'un usage-based tier ($0.001-0.01 / call)
  en plus des enterprise contracts
- Le **TAM** : passe de "audit-grade infra pour régulés Suisses
  (~ 100 M USD)" à "infrastructure pour agents IA mondialement
  (~ 10 Md USD)"  — TAM expressé en USD pour alignement avec
  benchmarks industrie SaaS (Snowflake, Datadog, etc.)

**Ce module change strictement RIEN à** :
- L'architecture L1/L2/L3 existante
- Les 174 tests PR #27 release-grade
- Le STABILITY contract v1.7 (juste ajout § 30)
- La signature de tout binaire existant
- Les contrats clients enterprise tier signés

---

## 1. Le problème de marché qu'on résout

### 1.1 Le pattern actuel des agents IA en production (2026)

Les agents IA "modernes" (Claude Sonnet, GPT-4o, Gemini Ultra avec
tool use) suivent ce pattern :

```
[1] User : "Summarize these 100 PDFs and extract phase 3 trial results"

[2] Agent (LLM call #1) : "I'll process them one by one"

[3] Agent (LLM call #2-101) : for each PDF :
      - extract text
      - parse structure
      - find sections about phase 3
      - extract results
      - synthesize

[4] Agent (LLM call #102) : "Here is the summary"

Total : 102 LLM calls.  Each ~ 3-5 sec.  Total ~ 5-9 min.
Cost : 0.10-0.50 € per call × 102 = 10-50 €.
Audit : opaque — agent's internal reasoning is not logged per step.
Hallucination : LLM invents trial details on 5-10 % of PDFs.
```

### 1.2 Les 4 maladies de cette approche

| Maladie | Impact | Pour qui c'est critique |
|---------|--------|--------------------------|
| **Latence séquentielle** | 5-9 min par workflow complexe | Tous les agents en production temps réel |
| **Coût LLM cumulatif** | 10-50 € par exécution | Toute boîte avec > 1k workflows/jour |
| **Hallucination sur tâches déterministes** | 5-15 % d'erreurs sur parsing JSON, dates, regex | Industries régulées (FDA, FINMA, GxP) |
| **Audit-trail opaque** | Pas de logging par sous-tâche | Compliance obligatoire (DORA, AI Act) |

### 1.3 Le pattern correct (que M50 implémente)

```
[1] User : "Summarize these 100 PDFs and extract phase 3 trial results"

[2] Agent (LLM call #1) : décide d'utiliser EFE swarm

[3] Agent → EFE MCP call : SwarmRequest with DAG :
      - 100 × pdf_extract_text in parallel
      - 100 × ner_clinical_phase (depends on extract)
      - 100 × filter_phase3 (depends on NER)
      - 100 × aggregate_phase3_results

[4] EFE essaim :
      - Plans DAG → executes layer by layer in parallel
      - 100 extract → 5 sec wall-clock
      - 100 NER → 3 sec wall-clock
      - 100 filter → 1 sec wall-clock
      - 100 aggregate → 1 sec wall-clock
      Total layer time : 10 sec

[5] Agent (LLM call #2) : reasoning final on aggregated results
      → 5 sec, 1 LLM call

Total : 15 sec instead of 5-9 min.  41× speedup.
Cost : 0.50 € EFE nano agents + 0.50 € Claude = 1 €.  10-50× cheaper.
Audit : per-nano-call trace via HMAC chain.  Replayable indefinitely.
Hallucination : 0 % on deterministic sub-tasks (nano agents are
                Rust functions or schema-validated LLM calls).
```

### 1.4 La taille du problème

Le marché des agents IA en production en 2026 :

| Source | Estimation 2026 | Estimation 2028 |
|--------|-----------------|-----------------|
| Total LLM API spend (Anthropic + OpenAI + open) | ~ 80 Md $ | ~ 250 Md $ |
| % wasted on sub-task LLM calls (deterministic) | ~ 30-40 % | ~ 25-35 % |
| TAM for nano agent acceleration | **~ 25-30 Md $** | **~ 60-80 Md $** |

→ EFE Agentic capture même 1 % de ce TAM = 250-300 M $ ARR potentiel
en 2028.

---

## 2. La thèse architecturale

### 2.1 Pourquoi un nano agent ≠ un agent IA

**Un agent IA** (Claude, GPT-4, agent custom) :
- Multi-tour
- Stateful (mémoire conversation)
- Probabiliste (LLM-driven)
- Multi-capability (peut tout faire, mal sur tâches déterministes)
- Slow (sec per call)
- Cher ($0.01-0.50 per call)
- Hallucine sur les sous-tâches déterministes

**Un nano agent** (EFE) :
- **Single-tour** (fire-and-forget pour la plupart)
- **Stateless** (chaque appel indépendant)
- **Déterministe** (Rust function ou LLM call avec schema validation)
- **Single-capability** (1 nano agent = 1 tâche bien définie)
- **Fast** (10-500 ms per call)
- **Cheap** ($0.0001-0.01 per call)
- **Predictable** (couvert par tests + schema)

### 2.2 La taxonomie des nano agents

3 types principaux selon le coût compute :

| Type | Compute | Exemple | Latence | Coût |
|------|---------|---------|---------|------|
| **Pure deterministic** | Rust CPU function | JSON parse, regex extract, schema validate | 1-10 ms | $0.0001 |
| **Local model** | Petit modèle ML (NER, classification) | NER bio-NER, intent classify | 50-200 ms | $0.001 |
| **LLM-augmented** | LLM call avec schema enforcement | "Extract clinical phase from text" with strict JSON | 500-2000 ms | $0.01 |

### 2.3 Pourquoi un essaim et pas une bibliothèque de fonctions

Une simple bibliothèque de fonctions (LangChain, Haystack) ne suffit
pas parce qu'elle ne fournit pas :

1. **Parallel execution at scale** (1000+ calls en parallèle isolés)
2. **Audit-grade trace per call** (HMAC chain replayable)
3. **Per-tenant isolation** (Firecracker microVM)
4. **Cost cap per call** (LLM budget enforcement)
5. **Compliance gating** (DPIA, AI Act per call)
6. **Failure isolation** (1 nano agent fail ≠ batch fail)
7. **Streaming results** (partial results à mesure que les nano agents terminent)

L'essaim **EST** la solution architecturale qui répond à ces 7 exigences
simultanément.  C'est ce qu'EFE a déjà construit (L2 meta-swarm) ;
M50 ajoute la couche application-level "nano agent" au-dessus.

---

## 3. Vue d'ensemble — les 4 layers d'EFE après M50

```
┌────────────────────────────────────────────────────────────────────┐
│  LAYER 4 — EFE AGENTIC (M50, NEW)                                  │
│                                                                     │
│  ┌──────────────────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │ Nano Agent       │  │ Swarm        │  │ MCP Bridge Extension │ │
│  │ Catalog (NAC)    │  │ Dispatcher   │  │ — exposes each nano  │ │
│  │ — typed schemas  │  │ — DAG runner │  │  agent as MCP tool  │ │
│  └──────────────────┘  └──────────────┘  └──────────────────────┘ │
│                                                                     │
│  ┌──────────────────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │ Nano Agent       │  │ Usage        │  │ Audit Trace per Call │ │
│  │ Runtime (NAR)    │  │ Metering     │  │ — extends HMAC chain │ │
│  │ — CPU + Firecr.  │  │ & Billing    │  │                       │ │
│  └──────────────────┘  └──────────────┘  └──────────────────────┘ │
│                                                                     │
│              Developer SDK + CLI : efe-nano init/publish/run        │
└──────────────────────────────────────────┬─────────────────────────┘
                                            │  uses
                                            ▼
┌────────────────────────────────────────────────────────────────────┐
│  LAYER 3 — Audit & compliance (M22-M28, existing, untouched)       │
│  HMAC chain | DPIA gating | AI Act | Cosign | Bench platform       │
└────────────────────────────────────────────────────────────────────┘
                                            │  uses
                                            ▼
┌────────────────────────────────────────────────────────────────────┐
│  LAYER 2 — Meta-swarm federation (M14, M20, M25, existing)         │
│  Controller | Participants | Firecracker μVM | gRPC mesh           │
└────────────────────────────────────────────────────────────────────┘
                                            │  uses
                                            ▼
┌────────────────────────────────────────────────────────────────────┐
│  LAYER 1 — Classification + LLM API (M14-M21, existing)            │
│  MCP server (M16) | LLM router (M21) | Prompts | Cache             │
└────────────────────────────────────────────────────────────────────┘
```

**Clé** : chaque flèche ↓ est "uses", JAMAIS "replaces".  Layer 4
**consomme** Layer 3, 2, 1, **ne les modifie pas**.

---

## 4. Composants détaillés de M50

### 4.1 Nano Agent Catalog (NAC) — M50.1

#### 4.1.1 Rôle

Le catalogue est le **registre typé et versionné** de tous les nano
agents disponibles sur l'instance EFE.  Il contient :

- **EFE-shipped nano agents** : ~ 20 nano agents starter pack (cf § 15)
- **Customer-published nano agents** : un nano agent published par
  un customer est privé à son tenant (per-tenant catalog) ou
  public (opt-in)
- **EFE-promoted community nano agents** : nano agents tiers
  vérifiés par EFE pour qualité + audit

#### 4.1.2 Schéma d'un nano agent

```rust
// crates/efe-nano/src/catalog/types.rs

pub struct NanoAgent {
    pub id: NanoAgentId,                  // e.g. "pdf_extract_text"
    pub version: semver::Version,         // e.g. 1.2.3
    pub category: Category,               // Extraction, Validation, etc.
    pub author: Author,                   // EFE, customer tenant_id, community
    pub stability_tier: StabilityTier,    // TIER-EXPERIMENTAL, TIER-1, TIER-2

    pub input_schema: JsonSchema,         // strict validation
    pub output_schema: JsonSchema,        // strict validation
    pub error_schema: JsonSchema,         // typed errors

    pub execution: ExecutionMode,         // see below
    pub determinism: DeterminismLevel,    // FullDeterministic, Probabilistic, Side-effect
    pub idempotency: IdempotencyKey,      // Auto, Manual, Disabled

    pub limits: ResourceLimits,           // memory_mb, cpu_ms, llm_tokens
    pub cost_per_call_cents: f64,         // pricing per invocation
    pub latency_p99_ms: u32,              // SLA target

    pub mcp_tool: McpToolMetadata,        // for MCP server exposure

    pub audit: AuditPolicy,               // INCLUDE_IO, REDACT_PII, etc.
    pub compliance: ComplianceTags,       // GDPR, HIPAA, GxP, FINMA
}

pub enum ExecutionMode {
    PureRust { entry: fn(In) -> Result<Out> },           // M50.2.A
    LocalModel { model_id: ModelId, version: String },   // M50.2.B
    LlmAugmented {                                        // M50.2.C
        provider: LlmProvider,
        model: String,
        prompt_template_id: PromptId,
        max_tokens: u32,
        temperature: f32,
        output_validator: OutputValidator,
    },
}

pub enum DeterminismLevel {
    FullDeterministic,        // Pure Rust ; same input → same output
    Probabilistic,             // LLM call ; may vary slightly
    SideEffect { kind: Kind }, // External I/O ; non-deterministic by design
}

pub enum IdempotencyKey {
    Auto,                      // hash(input) → cached for 24h
    Manual { ttl_sec: u32 },   // customer provides idempotency key
    Disabled,                  // every call executes anew
}

pub struct ResourceLimits {
    pub memory_mb: u32,         // max memory per invocation
    pub cpu_ms: u32,            // max CPU time
    pub wall_clock_ms: u32,     // max wall-clock latency
    pub llm_tokens: Option<u32>, // for LLM-augmented only
    pub network_egress_kb: u32,  // 0 for pure deterministic
}
```

#### 4.1.3 Versioning et stability

Chaque nano agent suit **semver** :

```
1.2.3
│ │ │
│ │ └─ Patch : bugfixes, identical input/output schemas
│ └─── Minor : new optional fields in input/output
└───── Major : breaking changes to schemas
```

Un client qui pin `pdf_extract_text@1.x` reçoit toutes les versions
1.y.z mais jamais 2.0.0 (qui pourrait casser son code).

**Stability tiers** :

| Tier | Garanties | Use case |
|------|-----------|----------|
| **TIER-EXPERIMENTAL** | Aucune | Beta, peut disparaître |
| **TIER-1** | Schema gel 12 mois | Production-safe |
| **TIER-2** | Schema gel 36 mois | Enterprise-safe |
| **TIER-AUDIT** | Schema gel 7+ ans + cosign | Régulé (FINMA, FDA, GxP) |

Les 20 nano agents starter pack sont **TIER-1** dès Day 1.

#### 4.1.4 API du catalogue

```rust
pub trait NanoAgentCatalog {
    async fn list(&self, filter: ListFilter) -> Result<Vec<NanoAgent>>;
    async fn get(&self, id: &NanoAgentId, version: VersionReq) -> Result<NanoAgent>;
    async fn search(&self, query: &str) -> Result<Vec<NanoAgent>>;
    async fn publish(&self, tenant: TenantId, na: NanoAgent) -> Result<NanoAgentId>;
    async fn unpublish(&self, tenant: TenantId, id: &NanoAgentId) -> Result<()>;
    async fn promote(&self, id: &NanoAgentId, to_tier: StabilityTier) -> Result<()>;
}
```

Implémentation : `crates/efe-nano/src/catalog/store.rs`.  Backend
Postgres avec full-text index sur description.

### 4.2 Nano Agent Runtime (NAR) — M50.2

#### 4.2.1 Trois modes d'exécution

##### Mode A — Pure Rust (sub-10 ms latency)

```rust
// crates/efe-nano/src/runtime/pure_rust.rs

pub async fn execute_pure_rust(
    agent: &NanoAgent,
    input: serde_json::Value,
    ctx: &CallContext,
) -> Result<NanoAgentResponse> {
    let validated = agent.input_schema.validate(&input)?;
    let entry = agent.execution.as_pure_rust()?;
    
    let timer = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || (entry)(validated)).await??;
    let elapsed = timer.elapsed();
    
    agent.output_schema.validate(&result)?;
    
    ctx.audit.record(NanoAgentCall {
        agent_id: agent.id.clone(),
        version: agent.version.clone(),
        input_hash: hash(&input),
        output_hash: hash(&result),
        latency_ms: elapsed.as_millis() as u32,
        mode: ExecutionMode::PureRust,
    });
    
    Ok(NanoAgentResponse { output: result, latency_ms: elapsed.as_millis() as u32 })
}
```

**Caractéristiques** :
- Pas d'isolation Firecracker (pure Rust dans un thread)
- 1-10 ms typique
- Cost : ~ $0.0001 (CPU time uniquement)
- Use case : JSON parse, schema validation, regex, format

##### Mode B — Local Model (sub-200 ms latency)

```rust
// crates/efe-nano/src/runtime/local_model.rs

pub async fn execute_local_model(
    agent: &NanoAgent,
    input: serde_json::Value,
    ctx: &CallContext,
) -> Result<NanoAgentResponse> {
    let validated = agent.input_schema.validate(&input)?;
    let model_id = agent.execution.as_local_model()?;
    
    let model = ctx.model_loader.get_or_load(model_id).await?;
    
    let timer = std::time::Instant::now();
    let result = model.infer(validated).await?;
    let elapsed = timer.elapsed();
    
    agent.output_schema.validate(&result)?;
    
    // Audit + metering
    // ...
}
```

**Caractéristiques** :
- Pre-loaded models (NER bio, classification multilingue)
- 50-200 ms typique
- Cost : $0.001
- Use case : NER, classification, sentiment, embeddings

##### Mode C — LLM-augmented (sub-2s latency)

```rust
// crates/efe-nano/src/runtime/llm_augmented.rs

pub async fn execute_llm_augmented(
    agent: &NanoAgent,
    input: serde_json::Value,
    ctx: &CallContext,
) -> Result<NanoAgentResponse> {
    let validated = agent.input_schema.validate(&input)?;
    let llm_spec = agent.execution.as_llm_augmented()?;
    
    // Cost cap check (M50.6)
    ctx.budget.precharge(ctx.tenant, llm_spec.cost_estimate())?;
    
    // Route via existing LLM router (M21)
    let prompt = ctx.prompts.render(llm_spec.prompt_template_id, &validated)?;
    let llm_response = ctx.llm_router.call(
        llm_spec.provider,
        llm_spec.model,
        prompt,
        LlmOptions {
            max_tokens: llm_spec.max_tokens,
            temperature: llm_spec.temperature,
            response_format: ResponseFormat::JsonSchema(agent.output_schema.clone()),
        },
    ).await?;
    
    // Strict schema validation on LLM output
    let validated_output = agent.output_schema.validate_strict(&llm_response.content)?;
    
    // Final cost reconciliation
    ctx.budget.charge(ctx.tenant, llm_response.actual_cost);
    
    // Audit + metering
    // ...
}
```

**Caractéristiques** :
- Uses existing LLM router (M21) — no duplication
- Strict JSON schema enforcement on output
- Cost cap pre-check + post-charge
- 500-2000 ms typique
- Cost : $0.01-0.05
- Use case : entity extraction, summarization with schema, structured reasoning

#### 4.2.2 Cold start optimization

Le cold start dépend du mode :

**Mode A (Pure Rust)** — pas de microVM
- Exécution dans un thread pool tokio
- Cold start = négligeable (< 1 ms, juste task scheduling)
- Pas de pool dédié, le tokio runtime gère

**Mode B (Local Model)** — Firecracker microVM avec modèle pré-chargé
- Pool de microVMs pré-warmées avec models loaded
- Cold start hit (pool warm) : 5-20 ms
- Cold start miss (provisioning) : 100-500 ms

**Mode C (LLM-augmented)** — Firecracker microVM minimal + LLM call externe
- Pool de microVMs lightweight
- Cold start hit : 5-10 ms (mais LLM call externe est le bottleneck)
- Cold start miss : 50-200 ms

Pool config pour Mode B et C :

```
Mode B pool config :
  - 20 microVMs avec common models loaded (NER, classifier, embedder)
  - Dynamic scaling 20→200 microVMs sur 30 sec en charge
  - Target hit rate > 99 %

Mode C pool config :
  - 50 microVMs lightweight (juste LLM HTTP client)
  - Dynamic scaling 50→500 microVMs sur 30 sec
  - Target hit rate > 99.5 %
  
Mode A : pas de pool, tokio thread pool natif
```

Implementation : `crates/efe-nano/src/runtime/pool.rs`.

#### 4.2.3 Resource limits enforcement

Chaque nano agent declared its limits in catalog.  Runtime enforces :

```rust
pub fn enforce_limits(agent: &NanoAgent, ctx: &CallContext) -> Result<()> {
    if ctx.tenant_quota.remaining_memory_mb < agent.limits.memory_mb {
        bail!("tenant memory quota exceeded");
    }
    if ctx.tenant_quota.remaining_cpu_ms < agent.limits.cpu_ms {
        bail!("tenant cpu quota exceeded");
    }
    // ... etc
    Ok(())
}
```

### 4.3 Swarm Dispatcher (SD) — M50.3

#### 4.3.1 Le SwarmRequest

```rust
// crates/efe-nano/src/dispatcher/types.rs

pub struct SwarmRequest {
    pub request_id: Uuid,
    pub tenant: TenantId,
    pub caller_agent_id: Option<String>,  // e.g. "claude-sonnet-4-5", "openai-gpt-4o"
    
    pub operations: Vec<Operation>,
    pub max_parallelism: u32,             // safety cap
    pub timeout_ms: u32,                   // overall batch timeout
    pub idempotency_key: Option<String>,
    pub streaming: bool,                   // if true, partial results streamed back
    pub audit_tags: Vec<String>,           // for searchable traces
}

pub struct Operation {
    pub op_id: String,                     // customer-chosen identifier
    pub agent: NanoAgentRef,               // catalog ref
    pub input: OperationInput,
    pub depends_on: Vec<String>,           // op_ids this depends on
    pub timeout_ms: Option<u32>,           // per-op override
    pub retry: RetryPolicy,
}

pub enum OperationInput {
    Literal(serde_json::Value),
    Reference { op_id: String, json_path: String },  // refer to another op's output
    Merge(Vec<OperationInput>),                       // merge multiple references
}
```

#### 4.3.2 DAG planning algorithm

```
Input : SwarmRequest with N operations
Output : Execution plan (ordered batches)

1. Build dependency graph G :
   - Nodes = operations
   - Edges = depends_on relationships
   
2. Cycle detection :
   - If G has a cycle → reject with InvalidDag error
   
3. Topological sort + layer assignment :
   - Layer 0 = all ops with no dependencies
   - Layer K+1 = all ops whose deps are in layers ≤ K
   
4. Within each layer, batch by execution mode :
   - Pure Rust ops → dispatched to NAR pool (M50.2.A)
   - Local model ops → dispatched to model pool (M50.2.B)
   - LLM-augmented → dispatched to LLM router queue (M50.2.C, M21)
   
5. Enforce max_parallelism :
   - If layer has > max_parallelism ops, sub-batch with backpressure
   
6. Execute layers sequentially, ops within layer in parallel.
```

#### 4.3.3 Result aggregation et streaming

```rust
pub enum SwarmResponse {
    Complete {
        results: HashMap<String, OperationResult>,  // op_id → result
        total_latency_ms: u32,
        total_cost_cents: f64,
        audit_trace_id: AuditTraceId,
    },
    Streaming {
        rx: mpsc::Receiver<StreamedOperationResult>,
        completion_handle: JoinHandle<Result<()>>,
    },
}

pub struct StreamedOperationResult {
    pub op_id: String,
    pub status: OpStatus,                  // Pending, Running, Success, Failed
    pub output: Option<serde_json::Value>,
    pub error: Option<OperationError>,
    pub latency_ms: u32,
    pub cost_cents: f64,
}
```

#### 4.3.4 Failure modes

| Failure type | Behavior |
|--------------|----------|
| 1 op times out | Marked Failed, downstream ops cancelled |
| 1 op returns error | Marked Failed, downstream skipped, batch continues |
| Tenant quota exceeded mid-batch | Batch terminated, partial results returned |
| LLM provider down | Auto-retry on alternate provider (via M21 router) |
| Firecracker microVM crash | Op marked Failed, microVM recycled |
| Network partition | Caller gets partial results + reconnection hint |

#### 4.3.5 Idempotency

Si `idempotency_key` est fourni :
- EFE store la response complète sous cette clé pour 24h
- Replay de la même requête (même `idempotency_key` + même hash de body) = même response
- Permet aux clients de retry safely après timeout réseau

### 4.4 MCP Bridge Extension (MCB) — M50.4

#### 4.4.1 Extension du MCP server existant (M16)

Le MCP server M16 actuel expose un set de tools couvrant la
surface classification + audit + bundle (≥ 6 selon README, plus
selon évolution récente — vérification d'inventaire en Sprint 0).
M50 **ajoute** des MCP tools spécifiques nano agents, **sans
modifier ni retirer aucun tool existant** :

```python
# mcp-server/mcp_server/tools/nano_agents.py (NEW FILE — additif)

from mcp_server.dispatcher import register_tool

# Generic dispatcher tool
@register_tool(
    name="efe_nano_swarm",
    description="Execute a swarm of nano agents in parallel with DAG dependencies.",
    input_schema=SWARM_REQUEST_SCHEMA,
    output_schema=SWARM_RESPONSE_SCHEMA,
)
async def efe_nano_swarm(payload: dict, ctx: McpContext) -> dict:
    # Forward to /v1/swarm endpoint of efe-server
    return await ctx.efe_client.swarm(payload)

# Discoverable per-nano-agent tools (auto-generated from catalog)
for agent in await catalog.list_public():
    @register_tool(
        name=f"efe_nano_{agent.id}",
        description=agent.description,
        input_schema=agent.input_schema,
        output_schema=agent.output_schema,
    )
    async def _per_agent_tool(payload: dict, ctx: McpContext, agent=agent):
        return await ctx.efe_client.call_nano_agent(agent.id, payload)
```

Donc un agent IA qui se connecte via MCP voit :

```
Available tools :
  ─ efe_classify (M16 original)
  ─ efe_register_catalog (M16 original)
  ─ efe_evolve_catalog (M16 original)
  ─ efe_list_catalogs (M16 original)
  ─ efe_get_audit_log (M16 original)
  ─ efe_get_admin_quotas (M16 original)
  ─ efe_nano_swarm (M50 new)
  ─ efe_nano_pdf_extract_text (M50 new)
  ─ efe_nano_json_validate (M50 new)
  ─ efe_nano_ner_clinical (M50 new)
  ... (20+ nano agent tools)
```

Le client Claude Desktop, Cursor, Zed, Continue, Cline les voit
automatiquement.

#### 4.4.2 Authentication

Inchangé vs M16 : OIDC token, per-tenant scope.

#### 4.4.3 Per-tenant tool visibility

Un nano agent published par le tenant A est visible uniquement
pour le tenant A (sauf si flag `public: true`).

### 4.5 Audit Trace per Call (ATC) — M50.5

#### 4.5.1 Extension du HMAC chain existant

Le HMAC chain M23 existant fournit déjà :
- `audit_id` séquentiel
- `prev_hmac` (link au précédent)
- `payload_hmac` (HMAC du payload)
- `signed_at`
- `tenant_id`

M50 ajoute une **nouvelle table** `nano_agent_traces` avec FK vers
le chain principal :

```sql
CREATE TABLE nano_agent_traces (
    trace_id            UUID PRIMARY KEY,
    audit_id            BIGINT NOT NULL REFERENCES audit_chain(audit_id),
    parent_trace_id     UUID NULL REFERENCES nano_agent_traces(trace_id),
    swarm_request_id    UUID NOT NULL,
    operation_id        TEXT NOT NULL,
    
    tenant_id           UUID NOT NULL,
    caller_agent_id     TEXT NULL,        -- e.g. "claude-sonnet-4-5"
    nano_agent_id       TEXT NOT NULL,
    nano_agent_version  TEXT NOT NULL,
    execution_mode      TEXT NOT NULL,    -- PureRust, LocalModel, LlmAugmented
    
    input_hash          BYTEA NOT NULL,   -- SHA-256 of input
    output_hash         BYTEA NULL,        -- SHA-256 of output (NULL if failed)
    error_kind          TEXT NULL,
    
    started_at          TIMESTAMPTZ NOT NULL,
    completed_at        TIMESTAMPTZ NULL,
    latency_ms          INTEGER NULL,
    
    cost_cents          NUMERIC(10, 4) NOT NULL,
    llm_tokens_used     INTEGER NULL,
    
    payload_storage_uri TEXT NULL,         -- if FullPayload retention
    
    UNIQUE (swarm_request_id, operation_id)
);

CREATE INDEX idx_nano_traces_tenant_time ON nano_agent_traces(tenant_id, started_at DESC);
CREATE INDEX idx_nano_traces_swarm ON nano_agent_traces(swarm_request_id);
CREATE INDEX idx_nano_traces_agent ON nano_agent_traces(nano_agent_id);
```

#### 4.5.2 Retention policies

| Tier | Hash retention | Full payload retention |
|------|----------------|------------------------|
| Free | 7 days | 24 hours |
| Developer | 30 days | 7 days |
| Pro | 1 year | 90 days |
| Enterprise | 7 years | 1 year |
| **Audit-grade** | **7+ years (cosign signed)** | **7+ years (cosign signed)** |

#### 4.5.3 Replay capability

```rust
pub async fn replay_swarm(
    request_id: Uuid,
    ctx: &ReplayContext,
) -> Result<SwarmResponse> {
    // 1. Fetch all nano_agent_traces for this request_id
    let traces = ctx.db.list_traces(request_id).await?;
    
    // 2. Verify HMAC chain integrity
    audit::verify_chain(&traces, &ctx.signing_keys)?;
    
    // 3. Verify nano agent versions still exist in catalog
    for trace in &traces {
        ctx.catalog.get(&trace.nano_agent_id, trace.nano_agent_version.into())?;
    }
    
    // 4. Re-execute each operation with original input
    //    (input fetched from payload_storage_uri if retained, else
    //     verified-only via input_hash)
    for trace in &traces {
        let original_input = ctx.fetch_payload(trace.payload_storage_uri).await?;
        let new_output = ctx.runtime.execute(&trace.nano_agent_id, original_input).await?;
        let new_output_hash = hash(&new_output);
        
        // Determinism check
        if trace.output_hash != Some(new_output_hash) {
            // For deterministic agents, this is a regression
            // For probabilistic agents (LLM), expected variance — log but allow
            if trace.determinism == DeterminismLevel::FullDeterministic {
                bail!("Replay drift detected for {}", trace.operation_id);
            }
        }
    }
    
    Ok(...)
}
```

C'est la **garantie audit** : un régulateur peut re-jouer toute
exécution swarm 7 ans plus tard et vérifier que le résultat tient.

### 4.6 Usage Metering & Billing (UMB) — M50.6

#### 4.6.1 Counter design

Chaque appel nano agent incrémente :
- Compteur per-tenant per-day
- Compteur per-tenant per-nano-agent
- Compteur per-tenant per-execution-mode
- Coût cumulé en cents

```rust
pub struct MeterEvent {
    pub tenant_id: TenantId,
    pub nano_agent_id: NanoAgentId,
    pub execution_mode: ExecutionMode,
    pub timestamp: DateTime<Utc>,
    pub cost_cents: f64,
    pub llm_tokens: Option<u32>,
}
```

Storage : ClickHouse pour aggregation high-volume.  Hourly rollups
vers Postgres pour reporting.

#### 4.6.2 Real-time cost cap enforcement

Avant chaque LLM-augmented call :

```rust
pub async fn precharge(
    tenant: TenantId,
    estimated_cost_cents: f64,
    ctx: &MeterContext,
) -> Result<PrechargeReceipt> {
    let remaining_budget = ctx.budget.remaining(tenant).await?;
    
    if remaining_budget < estimated_cost_cents {
        bail!(BudgetExceeded {
            tenant,
            requested: estimated_cost_cents,
            remaining: remaining_budget,
        });
    }
    
    // Reserve the budget (atomic decrement)
    ctx.budget.reserve(tenant, estimated_cost_cents).await
}
```

Post-call :

```rust
pub async fn settle(
    receipt: PrechargeReceipt,
    actual_cost_cents: f64,
    ctx: &MeterContext,
) -> Result<()> {
    let delta = actual_cost_cents - receipt.estimated_cost_cents;
    if delta > 0.0 {
        ctx.budget.charge_extra(receipt.tenant, delta).await?;
    } else if delta < 0.0 {
        ctx.budget.refund(receipt.tenant, -delta).await?;
    }
    
    ctx.meter.record(MeterEvent {
        tenant_id: receipt.tenant,
        cost_cents: actual_cost_cents,
        // ...
    });
    
    Ok(())
}
```

#### 4.6.3 Pricing tiers

| Tier | Per-call price | Volume cap | Audit retention | Support |
|------|----------------|------------|------------------|---------|
| Free | $0 (1k/mo) | 1,000 | 7d hash | Community |
| Developer | $0.001 | 1M/mo | 30d hash, 7d full | Email |
| Pro | $0.0005 | 100M/mo | 1y hash, 90d full | Slack + 24h SLA |
| Enterprise | $100k/y base + $0.0003 | Unlimited | 7y hash, 1y full | Dedicated |
| Audit-grade | $500k/y base + $0.001 | Unlimited | 7y signed cosign | 24/7 + SLA |

#### 4.6.4 Stripe integration

```rust
// Monthly billing job (M50.6.D)
pub async fn bill_tenants(ctx: &BillingContext) -> Result<()> {
    for tenant in ctx.tenants.list_active().await? {
        let usage = ctx.meter.aggregate_month(tenant.id).await?;
        let invoice = calculate_invoice(&tenant.plan, &usage);
        
        ctx.stripe.create_invoice(&tenant.stripe_customer_id, &invoice).await?;
    }
    Ok(())
}
```

### 4.7 Developer SDK & CLI — M50.7

#### 4.7.1 Rust SDK

```rust
// crates/efe-nano-sdk/src/lib.rs

#[efe_nano::nano_agent(
    id = "my_pdf_summary",
    version = "0.1.0",
    category = "extraction",
    determinism = "Probabilistic",
)]
pub async fn my_pdf_summary(input: PdfSummaryInput) -> Result<PdfSummaryOutput> {
    // Customer logic here
    // Can call other nano agents via ctx.nano.call(...)
}

pub struct PdfSummaryInput {
    pub file_url: String,
    pub max_summary_length: u32,
}

pub struct PdfSummaryOutput {
    pub summary: String,
    pub key_topics: Vec<String>,
}
```

Le macro `#[efe_nano::nano_agent]` génère :
- Le manifest JSON pour le catalogue
- Les schemas JSON depuis les types Rust
- Le wrapper d'audit + metering
- Le scaffolding MCP

#### 4.7.2 Python SDK

```python
# pip install efe-nano

from efe_nano import nano_agent

@nano_agent(
    id="my_pdf_summary",
    version="0.1.0",
    category="extraction",
    determinism="probabilistic",
)
async def my_pdf_summary(input: PdfSummaryInput) -> PdfSummaryOutput:
    # Customer logic here
    pass
```

#### 4.7.3 TypeScript SDK

```typescript
// npm install @efe/nano

import { nanoAgent } from '@efe/nano';

export const myPdfSummary = nanoAgent({
    id: "my_pdf_summary",
    version: "0.1.0",
    category: "extraction",
    determinism: "probabilistic",
    handler: async (input: PdfSummaryInput): Promise<PdfSummaryOutput> => {
        // Customer logic here
    },
});
```

#### 4.7.4 CLI

```bash
# Scaffold a new nano agent project
efe-nano init my-agent-project

# Test locally
efe-nano test --input '{"file_url":"..."}'

# Publish to private catalog
efe-nano publish --tenant my-tenant --stability tier-1

# Promote to higher stability tier
efe-nano promote my_pdf_summary@0.1.0 --to tier-2

# List nano agents (catalog browse)
efe-nano list --category extraction
```

---

## 5. Matrice d'intégration avec composants existants

| Composant existant | Comment M50 l'utilise | Modifications requises |
|---------------------|------------------------|-------------------------|
| `efe-server` (M14) | Nouveau endpoint `/v1/swarm` | **AJOUT** d'un router module, **rien retiré** |
| `efe-llm-router` (M21) | Mode C nano agents routent via lui | **AUCUNE** — interface inchangée |
| `efe-prompts` (M21) | Templates pour LLM-augmented nano agents | **AUCUNE** — utilise API existante |
| `efe-cache` (M27) | Cache des outputs idempotents | **AUCUNE** — utilise put/get existants |
| `efe-events` (M28) | Emission events nano-agent-completed | **AUCUNE** — event type est nouveau, schema extensible |
| `efe-meta-swarm` (M14, M25) | Mode C dispatch via participants L2 | **AUCUNE** — utilise gRPC mesh existant |
| `efe-agent-runner` (M25) | Firecracker pour mode B/C isolation | **AUCUNE** — utilise lib existante |
| `mcp-server` (M16) | Extension avec nano agent tools | **AJOUT** d'un sous-module `tools/nano_agents.py`, **6 tools existants INCHANGÉS** |
| `efe-pii` (M20) | Redaction pre-LLM dans Mode C | **AUCUNE** — utilise API existante |
| `efe-curation` (M23) | Audit chain HMAC pour traces | **AUCUNE** — extend table, pas le chain core |
| `efe-active-learning` (M24) | Feedback loop sur usage nano agents | **AUCUNE** — consomme events |
| `efe-billing` | Stripe integration pour usage-based | **AJOUT** d'un sub-module `nano_billing.rs` |
| `efe-metering` | Tracking nano agent calls | **AJOUT** d'un meter type, **schemas existants INCHANGÉS** |
| `bench/harness` | Nouvelle CB scenario family `CB-50_nano_*` | **AJOUT** scenarios, **35 existants INCHANGÉS** |
| `STABILITY_CONTRACT.md` | Nouvelle section § 30 — Nano Agent Swarm | **AJOUT** d'une section, **§ 1-29 INCHANGÉES** |
| `efe-bench-harness` tests | Nouveau test file `nano_agent_contract.rs` | **AJOUT** test, **51 freeze + 174 PR #27 tests INCHANGÉS** |

**Verdict additivité** : ✓ tous les composants existants restent
**byte-identical**.  M50 est ajouté **autour** d'eux, pas **dedans**.

---

## 6. Data models (Rust types + JSON schemas)

### 6.1 Top-level type hierarchy

```rust
// crates/efe-nano/src/types.rs

// Catalog
pub struct NanoAgent { /* cf § 4.1.2 */ }
pub struct NanoAgentId(pub String);
pub struct NanoAgentRef {
    pub id: NanoAgentId,
    pub version: VersionReq,
}

// Runtime
pub struct CallContext { /* tenant, budget, audit, llm_router, cache */ }
pub struct NanoAgentResponse {
    pub output: serde_json::Value,
    pub latency_ms: u32,
    pub cost_cents: f64,
    pub trace_id: TraceId,
    pub cache_hit: bool,
}

// Swarm
pub struct SwarmRequest { /* cf § 4.3.1 */ }
pub struct SwarmResponse { /* cf § 4.3.3 */ }
pub struct ExecutionPlan { /* layered DAG */ }

// Audit
pub struct NanoAgentTrace { /* mirrors SQL schema § 4.5.1 */ }

// Metering
pub struct MeterEvent { /* cf § 4.6.1 */ }
pub struct UsageAggregate {
    pub tenant: TenantId,
    pub period: DateRange,
    pub total_calls: u64,
    pub total_cost_cents: f64,
    pub by_agent: HashMap<NanoAgentId, AgentStats>,
}
```

### 6.2 JSON Schema for `SwarmRequest`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://efe.example.com/schemas/m50/swarm-request-v1.json",
  "title": "EFE Nano Agent Swarm Request",
  "type": "object",
  "required": ["request_id", "operations"],
  "properties": {
    "request_id": { "type": "string", "format": "uuid" },
    "caller_agent_id": { "type": "string", "maxLength": 256 },
    "operations": {
      "type": "array",
      "minItems": 1,
      "maxItems": 10000,
      "items": { "$ref": "#/$defs/Operation" }
    },
    "max_parallelism": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
    "timeout_ms": { "type": "integer", "minimum": 1000, "maximum": 600000, "default": 60000 },
    "idempotency_key": { "type": "string", "maxLength": 256 },
    "streaming": { "type": "boolean", "default": false },
    "audit_tags": { "type": "array", "items": { "type": "string" }, "maxItems": 16 }
  },
  "$defs": {
    "Operation": {
      "type": "object",
      "required": ["op_id", "agent", "input"],
      "properties": {
        "op_id": { "type": "string", "pattern": "^[a-zA-Z0-9_-]{1,64}$" },
        "agent": {
          "type": "object",
          "required": ["id"],
          "properties": {
            "id": { "type": "string" },
            "version": { "type": "string" }
          }
        },
        "input": { "$ref": "#/$defs/OperationInput" },
        "depends_on": { "type": "array", "items": { "type": "string" } },
        "timeout_ms": { "type": "integer", "minimum": 100, "maximum": 60000 },
        "retry": { "$ref": "#/$defs/RetryPolicy" }
      }
    },
    "OperationInput": {
      "oneOf": [
        { "type": "object", "properties": { "literal": {} } },
        { "type": "object", "required": ["ref"], "properties": { "ref": { "type": "object" } } },
        { "type": "object", "required": ["merge"], "properties": { "merge": { "type": "array" } } }
      ]
    },
    "RetryPolicy": {
      "type": "object",
      "properties": {
        "max_attempts": { "type": "integer", "minimum": 1, "maximum": 5 },
        "backoff_ms": { "type": "integer", "minimum": 0 }
      }
    }
  }
}
```

---

## 7. Data flow examples

### 7.1 Example A — Roche drug discovery agent

```
Agent (Claude Sonnet) :
   Task : "Find all phase 3 trial results in these 100 PDFs and
           summarize cardiovascular outcomes."
   ↓
   [LLM call #1] : "I will use efe_nano_swarm tool."
   ↓
   MCP call efe_nano_swarm with operations :
      - 100 × pdf_extract_text (parallelizable)
      - 100 × ner_clinical_phase (depends on extract)
      - 100 × filter_phase3 (depends on NER)
      - 100 × extract_cv_outcomes (depends on filter)
   ↓
EFE Swarm Dispatcher :
   - Parse + validate SwarmRequest
   - Build DAG : 4 layers, 100 ops each, total 400 ops
   - Layer 0 : 100 pdf_extract_text in parallel
                → Mode A (pure Rust, PDF lib)
                → 5 sec wall-clock, $0.10 total
   - Layer 1 : 100 ner_clinical_phase in parallel
                → Mode B (local biomedical NER model)
                → 3 sec wall-clock, $0.50 total
   - Layer 2 : 100 filter_phase3 in parallel
                → Mode A (regex + classifier)
                → 1 sec wall-clock, $0.05 total
   - Layer 3 : 100 extract_cv_outcomes in parallel
                → Mode C (LLM with JSON schema)
                → 8 sec wall-clock, $5.00 total
   - Audit chain : 400 entries logged
   - Cost : $5.65 total
   ↓
   Returns SwarmResponse to agent (15 sec total)
   ↓
Agent (Claude Sonnet) :
   [LLM call #2] : reasoning on aggregated results
                   → 8 sec, $0.50
   ↓
   Final summary to user.

Total : 23 sec wall-clock + 6.15 € cost
vs Without EFE : 23 min wall-clock + 50 € cost
Speedup : 60x.  Cost saving : 88 %.
```

### 7.2 Example B — UBS trade risk assessment

```
Agent (custom UBS Quant Agent) :
   Task : "For these 1000 trades, calculate risk metrics and flag
           any that violate Basel III + internal limits."
   ↓
   MCP call efe_nano_swarm with operations :
      - 1000 × parse_trade_json (parallelizable, Mode A)
      - 1000 × calculate_var_95 (Mode B, local Monte Carlo model)
      - 1000 × check_basel_compliance (Mode A, deterministic rules)
      - 1000 × flag_internal_limits (Mode A, lookup)
   ↓
EFE Swarm Dispatcher :
   - 4000 total ops, 4 layers
   - Layer 0 : 1000 parse_trade_json
                → 2 sec, $0.05
   - Layer 1 : 1000 calculate_var_95
                → 10 sec (heavier compute), $1.00
   - Layer 2 : 1000 check_basel_compliance
                → 2 sec, $0.05
   - Layer 3 : 1000 flag_internal_limits
                → 1 sec, $0.05
   - Audit chain : 4000 entries (regulatory mandatory for FINMA)
   - Total : 15 sec, $1.15
   ↓
Agent receives structured response :
   { "compliant": 950, "violations": 50, "flagged": 12 }
   ↓
   Decides next action (escalation to human risk officer for the 12 flagged).
```

### 7.3 Example C — Kovant DD memo generation

```
Agent (Kovant DD Agent) :
   Task : "Generate DD memo for company X based on the data room."
   ↓
   MCP call efe_nano_swarm :
      - 1 × fetch_data_room_inventory
      - K × pdf_extract_text (K = number of docs)
      - K × classify_doc_type
      - 1 × extract_cap_table (depends on classified)
      - 1 × extract_revenue (depends on classified)
      - 1 × extract_team_info (depends on classified)
      - 1 × screen_conflicts (independent)
      - 1 × generate_memo_draft (depends on all above, Mode C)
   ↓
EFE Swarm Dispatcher :
   - DAG : 4 layers
   - Layer 0 : fetch_data_room + K × pdf_extract
   - Layer 1 : K × classify + screen_conflicts (parallel branches)
   - Layer 2 : extract_cap_table + extract_revenue + extract_team_info
   - Layer 3 : generate_memo_draft (Mode C, full LLM)
   ↓
   Total : ~ 30 sec, $2.50
   
Result : Structured DD memo with red flag annotations.
Compared to manual DD : ~ 4 hours of associate time.
Speedup : 480x.  Cost : $2.50 vs $400 (associate cost).
```

---

## 8. Performance characteristics

### 8.1 Latency targets

| Operation | p50 | p99 |
|-----------|-----|-----|
| Catalog lookup | < 5 ms | < 20 ms |
| Mode A nano agent (pure Rust) | < 10 ms | < 50 ms |
| Mode B nano agent (local model) | < 80 ms | < 200 ms |
| Mode C nano agent (LLM) | < 1500 ms | < 5000 ms |
| SwarmRequest dispatch overhead | < 50 ms | < 200 ms |
| MCP tool call overhead | < 100 ms | < 500 ms |

### 8.2 Throughput

| Metric | Target Day 1 | Target 12 months | Note |
|--------|---------------|-------------------|------|
| Mode A calls/sec/node | 1,000 | 5,000 | Rust async + tokio |
| Mode B calls/sec/node | 100 | 500 | bottlenecked by model inference |
| Mode C calls/sec/node | 10 | 50 | bottlenecked by LLM provider |
| Audit chain writes/sec | 5,000 | 20,000 | Postgres + batching |
| Concurrent swarms in flight | 1,000 | 100,000 | realistic at scale |

**Note** : les targets 12 mois supposent l'embauche de 2-3 senior
platform engineers + provisioning d'infrastructure scale (ClickHouse
cluster, Postgres read replicas, Redis pool).  Ces targets sont
aspirationnels et seront révisés selon performance réelle Sprint 10.

### 8.3 Cold start

| Pool state | Latency added |
|------------|----------------|
| Pool hit (warm μVM available) | < 5 ms |
| Pool miss (μVM provisioned on demand) | 50-200 ms |
| Pool full + queue | depends on queue depth |

Target : 99.5 % pool-hit rate at peak load.

### 8.4 Cost model

```
Per-call cost = compute_cost + llm_cost + storage_cost + audit_cost

  compute_cost  = $0.00001 per CPU-ms (Mode A) to $0.0001 per CPU-ms (Mode C)
  llm_cost      = $0 (Mode A/B) to passthrough of provider cost (Mode C)
  storage_cost  = ~ $0.0001 per call (trace storage in ClickHouse)
  audit_cost    = ~ $0.00005 per call (HMAC + chain link)

Pricing margin (per call) :
  Free tier        : $0 (loss-leader, 1k/mo cap)
  Developer tier   : $0.001 (margin ~ 5-10x for Mode A)
  Pro tier         : $0.0005 (margin ~ 2-5x at scale)
  Enterprise       : custom + base fee
  Audit-grade      : custom + audit overhead amortized
```

---

## 9. Security & isolation model

### 9.1 Tenant isolation

```
Per-tenant boundaries enforced :

  Network        :  Each tenant has its own Cilium NetworkPolicy
                    No tenant-to-tenant traffic
  Storage        :  Per-tenant Postgres schema (or row-level security)
  Audit chain    :  Per-tenant chain, tenant_id in every entry
  Catalog        :  Private nano agents per-tenant, public ones shared read-only
  Budget         :  Per-tenant budget, atomic
  Logs           :  Tagged tenant_id, redacted in shared dashboards
```

### 9.2 Per-call isolation

Mode B/C nano agents tournent dans Firecracker microVM :
- Kernel boundary
- No shared filesystem with other calls
- Network egress whitelisted per nano agent
- Memory limit enforced by kernel cgroups

Mode A nano agents tournent dans un thread pool :
- No microVM (pour latence)
- Sandbox via Rust type system + capability tokens
- Async cancellation on timeout
- Memory tracked via Rust allocator

### 9.3 Code injection prevention

Pour les customer-published nano agents :
- Input strictly validated against JSON schema
- No `eval()` or dynamic code execution
- Mode A : pure Rust, compiled, no runtime injection possible
- Mode B/C : sandboxed model serving, prompts templated (no string concat)

### 9.4 Supply chain

- Customer-published Mode A nano agents : reviewed before TIER-1 promotion
- Customer-published Mode B/C : require signed bundle (Sigstore cosign)
- Catalog entries signed at publish time
- SBOM generated for each nano agent

---

## 10. Compliance & audit

### 10.1 DPIA per call

Si un nano agent traite des données personnelles :
- `compliance.dpia_required = true` dans le catalogue
- Avant call : check tenant a un DPIA signed pour ce nano agent
- Si non : refuse l'appel avec `DpiaMissing` error
- Audit chain log la check

### 10.2 AI Act gating

Nano agents classés "high-risk" (per EU AI Act Annex III) :
- Require human oversight metadata in catalog
- Step approval workflow enforced before output returned
- Audit log includes the human reviewer ID

### 10.3 Regulatory mapping

| Regulation | EFE M50 enforcement mechanism |
|------------|--------------------------------|
| **GDPR Art. 22** | Audit trace per automated decision |
| **GDPR Art. 5(1)(f)** (integrity) | HMAC chain + cosign signing |
| **EU AI Act Art. 9** (risk mgmt) | Stability tier + step approval |
| **EU AI Act Art. 12** (logging) | Audit chain replay |
| **DORA Art. 5** (ICT risk) | Multi-tenant isolation + audit |
| **FDA PCCP** | Versioned nano agents + change control |
| **GxP** | Reproducibility via replay capability |
| **FINMA** | Audit retention 7+ years signed cosign |
| **HIPAA** | PHI handling via M20 + Audit-grade tier |
| **Swiss AMLA** | 7-year KYC trace retention |

### 10.4 Audit retention

```
Free tier             :  7 days (LIFO eviction)
Developer            :  30 days hash + 7 days full payload
Pro                  :  1 year hash + 90 days full payload
Enterprise           :  7 years hash + 1 year full payload
Audit-grade tier     :  7+ years hash + 7+ years full payload, cosign signed
```

### 10.5 Replay & verification

```bash
# Customer or regulator can verify any historical swarm execution :
efe-cli audit replay --request-id <uuid> --output report.json

# Output :
#   - All nano agent calls in the swarm
#   - HMAC chain verification ✓
#   - Determinism check per call ✓
#   - Cosign signature on chain ✓
#   - Replay output matches stored output ✓
```

---

## 11. Pricing & business model

### 11.1 Tiered pricing

| Tier | Monthly | Calls included | Overage | Audit | Support |
|------|---------|----------------|---------|-------|---------|
| Free | $0 | 1,000 | none | 7d hash | Community |
| Developer | $99 | 1M | $0.001/call | 30d | Email |
| Pro | $499 | 100M | $0.0005/call | 1y | Slack 24h SLA |
| Enterprise | $8,300+ ($100k/y) | Unlimited | $0.0003 | 7y | Dedicated TAM |
| Audit-grade | $41,600+ ($500k/y) | Unlimited | $0.001 | 7y+cosign | 24/7 SLA |

### 11.2 Unit economics

Pour le tier Developer :
```
Revenue per call : $0.001
Marginal cost    : $0.00005 (compute + storage + audit)
Margin per call  : 95 %
```

Pour le tier Enterprise :
```
Revenue per call : $0.0003 + amortized $100k base
Marginal cost    : $0.00005
At 1B calls/year : effective $0.0004/call → ~ 87 % margin
```

### 11.3 Customer journey

```
Discovery       : Developer finds EFE via MCP Marketplace or HN post
Trial           : Signs up, gets 1k free calls/month
Adoption        : Hits free tier limit, upgrades to Developer ($99/mo)
Growth          : Volume scales, upgrade to Pro ($499/mo)
Enterprise pull : Their employer (Roche, UBS) sees they use EFE,
                   buys Enterprise license for compliance
Audit-grade     : Regulated function (clinical, trading) requires
                   audit-grade tier
```

C'est le **PLG** (Product-Led Growth) classique, qui aboutit à
des enterprise contracts.

### 11.4 18-month revenue target

| Month | Free users | Developer | Pro | Enterprise | ARR |
|-------|------------|-----------|-----|------------|-----|
| M6 | 500 | 50 | 5 | 0 | $90k |
| M12 | 2,000 | 200 | 20 | 1 | $400k |
| M18 | 5,000 | 500 | 50 | 3 | $1.2M |
| M24 | 10,000 | 1,000 | 100 | 8 + 2 audit | $4M |

---

## 12. Open source strategy

### 12.1 What's MIT-licensed

- **`efe-nano` core API** (catalog interface, dispatcher trait)
- **`efe-nano-sdk`** (Rust, Python, TypeScript)
- **CLI `efe-nano`** (init, publish, test)
- **20 starter nano agents** (Mode A only — pure Rust)
- **MCP server extension** (M16 base + M50 nano agent tools)

### 12.2 What's commercial

- **Catalog hosting service** (the `efe-nano` SaaS)
- **Runtime infrastructure** (Firecracker pools, model serving)
- **Audit chain commercial features** (cosign signing, 7y retention)
- **Mode B model serving** (proprietary fine-tuned models)
- **Mode C LLM routing** with cost cap (uses M21 commercial config)
- **Enterprise tier features** (custom SLA, dedicated VPC)

### 12.3 MCP Marketplace publishing

```
Publish 20 starter nano agents in Anthropic's MCP marketplace :
  → Free tier users on Claude Desktop discover EFE
  → Bottom-up adoption funnel
  → Conversion to paid tier when they hit free limit
```

### 12.4 Community contribution model

- GitHub PR to add new starter nano agents
- Reviewed by EFE team
- Promoted to TIER-1 once tests pass
- Contributors credited in catalog

---

## 13. Implementation roadmap (~ 6.5 months, 14 sprints × 2 weeks)

### M+0 (Jun 2026) — Pre-Kovant landed

**Pre-condition** : Kovant invest closed, capital available.

```
Sprint 0  (Jun 1-15)   : Setup M50 crate scaffold, freeze docs structure
Sprint 1  (Jun 15-30)  : NAC + NAR Mode A implemented
                          5 starter nano agents (Mode A only)
                          Local tests pass
```

### M+1 (Jul 2026)

```
Sprint 2  (Jul 1-15)   : NAR Mode B (model loader, pool)
                          5 more starter nano agents (Mode B)
Sprint 3  (Jul 15-31)  : Swarm Dispatcher (DAG planner, executor)
                          Integration tests with L2 meta-swarm
```

### M+2 (Aug 2026)

```
Sprint 4  (Aug 1-15)   : NAR Mode C (LLM router integration)
                          5 more starter nano agents (Mode C)
Sprint 5  (Aug 15-31)  : Audit Trace per Call
                          Replay capability
                          Postgres schema migration
```

### M+3 (Sep 2026)

```
Sprint 6  (Sep 1-15)   : MCP Bridge Extension
                          Auto-generated MCP tools per nano agent
                          Test with Claude Desktop, Cursor
Sprint 7  (Sep 15-30)  : Usage Metering & Billing
                          Stripe integration
                          Free tier rate limiter
```

### M+4 (Oct 2026) — SDK étalé sur 4 sprints (Rust prioritaire, Python+TS plus tard)

```
Sprint 8  (Oct 1-15)   : Rust SDK + macros + CLI core (init/test/publish)
                          5 last starter nano agents
                          (Python + TS SDKs déplacés à Sprint 10-11)
Sprint 9  (Oct 15-31)  : Documentation site (efe-nano.com)
                          MCP Marketplace submission (Rust-native first)
                          CLI complete (promote, list, search)
```

### M+5 (Nov 2026) — Python + TS SDKs + perf hardening

```
Sprint 10 (Nov 1-15)   : Python SDK (pyo3 bindings)
                          Performance hardening, pool warm-up optimization
Sprint 11 (Nov 15-30)  : TypeScript SDK (napi bindings)
                          Load testing (1k → 5k calls/sec, realistic targets)
                          Public Beta launch + first developer adoption
```

### M+6 (Dec 2026)

```
Sprint 12 (Dec 1-15)   : Audit-grade tier hardening
                          Cosign signing in production
                          7-year retention infrastructure
Sprint 13 (Dec 15-31)  : GA launch
                          Kovant case study published
                          Conversion funnel optimization
```

### Resource requirement

- Sprint 0-5 : Solo founder (Rodrigue) + 1 senior platform engineer hire post-Kovant
- Sprint 6+ : + 1 product engineer hire
- Sprint 9+ : + 1 developer advocate / docs writer

Total team by M+6 : 4 people (founder + 3 hires).

---

## 14. Breaking changes audit (preuve d'additivité)

### 14.1 Audit par crate

Each of 33 existing crates audited :

| Crate | M50 modifies it? | Evidence |
|-------|------------------|----------|
| `efe-core` | NO | M50 doesn't touch core types |
| `efe-kernels` | NO | M50 doesn't touch CUDA/CPU compute |
| `efe-ffi` | NO | M50 doesn't touch FFI |
| `efe-server` | ADDS endpoint | Adds `/v1/swarm` router, existing endpoints unchanged |
| `efe-telemetry` | NO | M50 emits events via existing metric registry |
| `efe-prompts` | NO | M50 reads prompts via existing API |
| `efe-llm-router` | NO | M50 calls via existing trait |
| `efe-admin` | NO | M50 admin via separate CLI |
| `efe-saga` | NO | M50 doesn't trigger sagas |
| `efe-curation` | NO | HMAC chain extended via FK, not modified |
| `efe-active-learning` | NO | M50 emits events consumed by AL |
| `efe-agent-runner` | NO | M50 uses Firecracker via existing lib |
| `efe-cache` | NO | M50 uses cache via existing API |
| `efe-events` | NO | M50 emits new event type, schema extensible |
| `efe-pii` | NO | M50 calls redaction via existing API |
| `efe-region` | NO | M50 routes via existing region map |
| `efe-retrieval` | NO | M50 doesn't change retrieval |
| `efe-vector` | NO | M50 doesn't change vector store |
| `efe-multimodal` | NO | M50 doesn't change multimodal |
| `efe-kg` | NO | M50 doesn't change KG |
| `efe-graphrag` | NO | M50 doesn't change GraphRAG |
| `efe-meta-swarm` | NO | M50 uses gRPC mesh via existing client |
| `efe-meta-swarm-proto` | NO | M50 has own proto, doesn't modify swarm proto |
| `efe-meta-swarm-server` | NO | M50 doesn't modify controller/participant binaries |
| `efe-metering` | ADDS meter type | New `MeterEvent` variant, existing variants unchanged |
| `efe-billing` | ADDS billing rule | New billing rule for nano-agent usage |
| `efe-backup` | NO | M50 included in standard backup |
| `efe-cluster-provisioner` | NO | M50 deployed via existing infra |
| `efe-bench` | NO | M50 doesn't change bench core |
| `bench/harness` | ADDS scenarios | CB-50_nano_* scenarios added, 35 existing unchanged |
| `slos/rollup` | NO | M50 emits SLI metrics via existing API |
| `smoke/runner` | NO | M50 has own smoke tests |
| `tools/loadgen` | NO | M50 has own load testing tools |

### 14.2 Tests after M50 added

```
PR #27 scope : 174 tests       → still 174 tests, all PASS (untouched)
STABILITY v1.7 freeze : 51 tests → still 51 tests, all PASS
M50 new tests :
  - efe-nano unit tests          → ~ 100 tests
  - M50 contract freeze          → ~ 30 tests (new section § 30)
  - CB-50_nano_* scenarios       → ~ 10 scenarios
  - Integration tests             → ~ 50 tests
  - Replay correctness           → ~ 20 tests
  
Total after M50 : ~ **385 tests** (174 + 51 + ~ 160 new)

Aucun test existant ne casse.
```

### 14.3 STABILITY contract delta

```diff
docs/STABILITY_CONTRACT.md (target post-M50)

@@ EXISTING § 1-29 UNCHANGED @@

+ ## § 30  M50 — Nano Agent Swarm
+
+ ### 30.1  Module identifier
+ Module ID : M50
+ Stability tier : TIER-EXPERIMENTAL (Day 1) → TIER-1 (after 90 days production)
+
+ ### 30.2  Catalog schema (frozen at TIER-1)
+ [JSON schema fully specified]
+
+ ### 30.3  Swarm request schema (frozen at TIER-1)
+ [JSON schema fully specified]
+
+ ### 30.4  Audit trace schema (frozen at TIER-AUDIT)
+ [SQL schema fully specified, immutable post-TIER-AUDIT]
+
+ ### 30.5  Resource limits (frozen at TIER-1)
+ - Mode A : memory ≤ 256 MB, CPU ≤ 1000 ms, no network
+ - Mode B : memory ≤ 2 GB, CPU ≤ 5000 ms, no external network
+ - Mode C : memory ≤ 512 MB, CPU ≤ 10000 ms, LLM via M21 only
+
+ ### 30.6  Audit trail addendum
+ [PENDING — populated at first GA release]
```

---

## 15. Les 20 nano agents starter pack

**Exception au cycle 90-day TIER-1 promotion** : les 20 nano agents
starter pack sont **audités en interne par EFE** (revue de code + tests
de contrat + benchmarks de performance + scan compliance par l'équipe
EFE) avant promotion directe à TIER-1 dès Day 1.  Cette exception
s'applique uniquement aux nano agents **shipped par EFE** ; les
customer-published nano agents suivent strictement la règle 90-day
en production (§ 16.2).

### 15.1 Extraction (5 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `pdf_extract_text` | A | Extract plain text from PDF | $0.0001 | 50 ms |
| `html_extract_main_content` | A | Extract main article from HTML | $0.0001 | 30 ms |
| `markdown_extract_headings` | A | Build TOC from markdown | $0.0001 | 10 ms |
| `email_extract_body` | A | Parse email RFC 5322, extract clean body | $0.0001 | 20 ms |
| `docx_extract_text` | A | Extract text from .docx | $0.0001 | 40 ms |

### 15.2 Validation & parsing (5 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `json_validate` | A | Validate JSON against schema | $0.00005 | 5 ms |
| `xml_validate` | A | Validate XML against XSD | $0.00005 | 10 ms |
| `sql_parse_safe` | A | Parse SQL, reject if dangerous | $0.0001 | 20 ms |
| `regex_extract` | A | Run regex, return all matches | $0.00005 | 5 ms |
| `csv_normalize` | A | Parse + normalize CSV | $0.0001 | 30 ms |

### 15.3 Classification (3 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `language_detect` | B | Detect language (99 + langs) | $0.001 | 100 ms |
| `sentiment_classify` | B | Pos/neg/neutral sentiment | $0.001 | 150 ms |
| `intent_classify_generic` | B | Generic intent classification | $0.001 | 150 ms |

### 15.4 Entity recognition (3 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `ner_generic` | B | PERSON/ORG/LOC/DATE/MONEY | $0.001 | 200 ms |
| `ner_clinical` | B | DRUG/DISEASE/GENE/PHASE | $0.001 | 200 ms |
| `ner_legal` | B | CASE/CITATION/PARTY/CLAUSE | $0.001 | 200 ms |

### 15.5 Transformation (2 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `date_normalize` | A | Parse date string, output ISO 8601 | $0.00005 | 5 ms |
| `currency_normalize` | A | "$1,234.56 USD" → {value, currency, normalized} | $0.00005 | 5 ms |

### 15.6 LLM-augmented (2 agents)

| ID | Mode | Description | Cost | Latency p99 |
|----|------|-------------|------|--------------|
| `summarize_strict` | C | LLM summary with strict length cap | $0.01 | 2000 ms |
| `extract_structured` | C | LLM extraction with JSON schema enforcement | $0.01 | 2500 ms |

---

## 16. Customer authoring path

### 16.1 Step-by-step

```bash
# 1. Install SDK
$ pip install efe-nano  # or cargo add efe-nano-sdk

# 2. Scaffold project
$ efe-nano init my-portfolio-monitor
$ cd my-portfolio-monitor

# 3. Edit nano_agent.py (generated)
$ cat nano_agent.py
from efe_nano import nano_agent

@nano_agent(
    id="portfolio_redgreen_score",
    version="0.1.0",
    category="classification",
    determinism="probabilistic",
    description="Score a portfolio company red/amber/green based on signals",
)
async def portfolio_redgreen_score(input: PortfolioInput) -> RedGreenScore:
    # Customer's domain logic
    ...

# 4. Test locally
$ efe-nano test --input '{"company_id": "X", "signals": {...}}'
# Runs nano agent in local microVM, validates schemas

# 5. Publish to private catalog
$ efe-nano publish --tenant kovant-prod --stability tier-experimental
# Publishes to Kovant's private catalog

# 6. Use from Kovant's agent
# In Claude Desktop, the new tool efe_nano_portfolio_redgreen_score appears
# Kovant's portfolio agent can now call it via MCP
```

### 16.2 Promotion path

```
TIER-EXPERIMENTAL  →  TIER-1
  Conditions :
    - 90 days in production with ≥ 1000 successful calls
    - p99 latency within declared SLA
    - 0 schema-validation failures
    - Customer files a Promotion Request
    - EFE team reviews + signs off

TIER-1  →  TIER-2
  Conditions :
    - 6 months in TIER-1
    - ≥ 100k successful calls
    - Customer commits to schema freeze for 36 months

TIER-2  →  TIER-AUDIT
  Conditions :
    - 12 months in TIER-2
    - Regulatory use case (FINMA, FDA, GxP)
    - Audit by independent third party
    - Customer commits to 7+ years schema freeze
    - Cosign signing enabled
```

---

## 17. Testing strategy

### 17.1 New test files (additive to existing 174)

```
bench/harness/tests/
├── m50_catalog_contract.rs           # ~ 30 tests on NAC schema
├── m50_runtime_modes.rs               # ~ 20 tests on NAR
├── m50_dispatcher_dag.rs              # ~ 25 tests on SD
├── m50_mcp_bridge.rs                  # ~ 15 tests on MCB
├── m50_audit_trace_replay.rs          # ~ 20 tests on ATC + replay
├── m50_metering_billing.rs            # ~ 15 tests on UMB
├── m50_starter_pack_contract.rs       # ~ 20 tests on 20 starter agents
└── m50_integration_e2e.rs              # ~ 15 end-to-end scenarios

Total : ~ 160 new tests
Plus existing : 174 PR #27 + 51 STABILITY = 225
Grand total post-M50 : 385 tests
```

### 17.2 CB-50_nano_* benchmark scenarios

```
bench/scenarios/CB-50_nano_*.yaml :
  CB-50_nano_pure_rust_throughput
  CB-50_nano_local_model_throughput
  CB-50_nano_llm_augmented_throughput
  CB-50_nano_swarm_dag_parallelism
  CB-50_nano_audit_chain_writes
  CB-50_nano_replay_correctness
  CB-50_nano_cold_start_latency
  CB-50_nano_pool_hit_rate
  CB-50_nano_concurrent_swarms
  CB-50_nano_compliance_dpia_gating
```

### 17.3 STABILITY freeze tests for § 30

```rust
// bench/harness/tests/stability_contract_v18.rs (NEW)

#[test]
fn v1_8_section_30_nano_swarm_module_present() { ... }

#[test]
fn v1_8_section_30_1_module_id_is_m50() { ... }

#[test]
fn v1_8_section_30_2_catalog_schema_frozen() { ... }

#[test]
fn v1_8_section_30_3_swarm_request_schema_frozen() { ... }

#[test]
fn v1_8_section_30_4_audit_trace_schema_frozen() { ... }

#[test]
fn v1_8_section_30_5_resource_limits_immutable() { ... }
```

Note : we move from `stability_contract_v17` to `stability_contract_v18` —
this is the **canonical pattern** for adding a major section.  The v17
tests stay untouched, v18 adds the new contract.

### 17.4 Performance test gates

For TIER-1 promotion :
```
$ cargo bench --test m50_perf
  pure_rust_throughput    : 1000+ calls/sec/node ✓
  local_model_throughput  : 100+  calls/sec/node ✓
  llm_throughput          : 10+   calls/sec/node ✓
  pool_hit_rate           : 99.5+ %                ✓
  swarm_overhead          : < 50 ms p50            ✓
```

If any fails → block TIER-1 promotion.

---

## 18. Failure modes & recovery

### 18.1 Failure modes catalog

| Failure | Detection | Recovery |
|---------|-----------|----------|
| Nano agent crash (Mode A) | Process exits | Caught by runtime, returned as `RuntimeFailure` |
| Nano agent OOM | Cgroup signal | Caught, returned as `ResourceExceeded` |
| Mode B model load failure | Model loader exception | Cached as failure, retry after 60 sec |
| Mode C LLM provider down | M21 router fails | Auto-retry on alternate provider |
| Mode C cost cap exceeded | Pre-check at dispatch | Returned as `BudgetExceeded` before invocation |
| Swarm timeout overall | Wall-clock timer | Partial results returned with status |
| Postgres write failure (audit) | DB error | Retry 3x, fail open if still down — TIER-AUDIT triggers alert |
| Stripe billing failure | Invoice creation fails | Retry + dead-letter queue + ops alert |
| MCP client disconnects mid-swarm | TCP RST | Swarm continues, results buffered for reconnection |
| Catalog corruption | Schema validation on load | Refuse to start, page on-call |

### 18.2 Disaster recovery

Standard EFE DR procedures (M28 Backup) apply :
- Postgres : continuous WAL backup, 7-year retention
- ClickHouse : daily snapshots, 1-year retention
- Audit chain : cosign signed daily snapshots to S3
- Catalog : versioned, all changes audit-logged

RTO : 4 hours (recover service)
RPO : 5 minutes (potential data loss in audit chain)

For TIER-AUDIT customers : RPO 0 minutes (sync replication).

---

## 19. Glossary

| Term | Definition |
|------|------------|
| **Nano agent** | A single-task, deterministic-or-near-deterministic, schema-validated function exposed via EFE's catalog. |
| **Swarm** | A batch of nano agents executed in parallel with optional DAG dependencies. |
| **SwarmRequest** | JSON document describing the operations + dependencies of a swarm. |
| **DAG** | Directed Acyclic Graph of operations within a swarm. |
| **Mode A / B / C** | Pure Rust / Local Model / LLM-Augmented nano agent execution modes. |
| **NAC** | Nano Agent Catalog. |
| **NAR** | Nano Agent Runtime. |
| **SD** | Swarm Dispatcher. |
| **MCB** | MCP Bridge (extension of M16). |
| **ATC** | Audit Trace per Call. |
| **UMB** | Usage Metering & Billing. |
| **Stability tier** | TIER-EXPERIMENTAL → TIER-1 → TIER-2 → TIER-AUDIT. |
| **Cold start** | Time from cold microVM provisioning to ready-to-execute. |
| **Pool hit** | A pre-warmed microVM was available for the nano agent invocation. |
| **Replay** | Re-execute a historical swarm to verify reproducibility. |
| **Audit-grade tier** | Highest customer tier with 7+ years cosign-signed retention. |

---

## 20. Decisions log

### D-M50-001 — Why "M50" not "M36" ?

We jumped to M50 to signal this is a major capability extension,
not just the next iterative module.  M36-M49 are reserved for
in-between iterations on existing layers.

### D-M50-002 — Why not modify the existing meta-swarm directly ?

The meta-swarm L2 is operating well as an infrastructure primitive.
Adding nano agents directly to it would entangle two concerns
(infra-level isolation vs application-level task batching).  M50
keeps these layers separate, which preserves architectural
modularity.

### D-M50-003 — Why three execution modes instead of unified ?

Different nano agents have radically different cost/latency
profiles.  Forcing all into a Firecracker microVM would impose
50-200 ms cold start on a 5 ms regex extraction — 10-40x overhead.
The three modes let us optimize per-agent.

### D-M50-004 — Why JSON Schema validation instead of typed gRPC ?

JSON Schema is more accessible to non-Rust developers (Python,
TypeScript SDKs).  Internal communication still uses Rust types
+ serde, but the catalog-facing schema is JSON.

### D-M50-005 — Why usage-based pricing instead of seat-based ?

Nano agents are consumed by AI agents, not by humans.  Per-seat
pricing makes no sense.  Per-call aligns cost with value.

### D-M50-006 — Why not adopt existing standards (e.g. OpenAPI for catalog) ?

JSON Schema (which we use) is part of OpenAPI 3.1.  We adopt the
schema layer of OpenAPI, but the runtime semantics (DAG, parallelism,
audit) are EFE-specific.

### D-M50-007 — Why expose every nano agent as a separate MCP tool ?

Anthropic's MCP design encourages tool granularity.  Exposing each
nano agent as a separate tool means Claude / Cursor / etc. discover
them automatically in their tool palette and select intelligently.

### D-M50-008 — Why TIER-AUDIT requires cosign signing ?

For regulated industries (FINMA, FDA), provenance is critical.
Cosign + Sigstore provides cryptographic guarantee that the audit
trace was not tampered with after the fact.  This is already used
in EFE's quarterly publish pipeline (M22).

### D-M50-009 — Should starter pack be open source ?

Yes, MIT.  This drives adoption.  The runtime infrastructure is
the moat, not the 20 starter nano agents.

### D-M50-010 — How does M50 interact with `efe-active-learning` (M24) ?

M50 emits usage events.  M24 can use these for closed-loop ML on
nano agent quality.  But M50 does not depend on M24.

---

## Annex A — Module relationship diagram

```
                                    ┌─────────────┐
                                    │  Customer's │
                                    │  AI agent   │
                                    │ (Claude,    │
                                    │  GPT-4,     │
                                    │  custom)   │
                                    └──────┬──────┘
                                           │
                                           │ MCP protocol
                                           ▼
              ┌────────────────────────────────────────────┐
              │  M16 MCP Server (existing) + M50.4 Bridge  │
              └─────────────────────┬──────────────────────┘
                                    │
                                    ▼
              ┌────────────────────────────────────────────┐
              │  M50.3 Swarm Dispatcher                    │
              │  - DAG planning                            │
              │  - Per-tenant budget check                 │
              │  - Audit chain entry creation             │
              └─────────────────────┬──────────────────────┘
                                    │
                ┌───────────────────┼───────────────────┐
                ▼                   ▼                   ▼
       ┌────────────────┐  ┌────────────────┐  ┌────────────────┐
       │ M50.2.A        │  │ M50.2.B        │  │ M50.2.C        │
       │ Pure Rust      │  │ Local Model    │  │ LLM-augmented  │
       │ runtime        │  │ runtime        │  │ runtime        │
       │ (thread pool)  │  │ (Firecracker)  │  │ (Firecracker   │
       │                │  │                │  │  + M21 router) │
       └────────────────┘  └────────────────┘  └────────────────┘
                                    │
                                    ▼
              ┌────────────────────────────────────────────┐
              │  M50.5 Audit Trace + M23 HMAC chain        │
              │  Persisted in Postgres + Cosign signed     │
              └────────────────────────────────────────────┘
                                    │
                                    ▼
              ┌────────────────────────────────────────────┐
              │  M50.6 Usage Metering + Stripe billing     │
              └────────────────────────────────────────────┘
```

---

## Annex B — File layout for M50

```
crates/
└── efe-nano/                          (NEW CRATE — M50 entire scope)
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── catalog/
    │   │   ├── mod.rs                  (M50.1)
    │   │   ├── types.rs
    │   │   ├── store.rs               (Postgres backend)
    │   │   └── api.rs
    │   ├── runtime/
    │   │   ├── mod.rs                  (M50.2)
    │   │   ├── pure_rust.rs           (Mode A)
    │   │   ├── local_model.rs         (Mode B)
    │   │   ├── llm_augmented.rs       (Mode C)
    │   │   └── pool.rs                 (microVM pool)
    │   ├── dispatcher/
    │   │   ├── mod.rs                  (M50.3)
    │   │   ├── dag.rs
    │   │   ├── executor.rs
    │   │   └── stream.rs
    │   ├── mcp_bridge.rs               (M50.4 — extends mcp-server)
    │   ├── audit.rs                    (M50.5 — extends M23 chain)
    │   ├── billing/
    │   │   ├── mod.rs                  (M50.6)
    │   │   ├── meter.rs
    │   │   ├── budget.rs
    │   │   └── stripe.rs
    │   └── starter_pack/                (M50 starter agents)
    │       ├── pdf_extract_text.rs
    │       ├── json_validate.rs
    │       └── ... (20 total)
    ├── tests/
    │   ├── catalog_test.rs
    │   ├── runtime_test.rs
    │   ├── dispatcher_test.rs
    │   ├── audit_test.rs
    │   └── starter_pack_test.rs
    └── benches/
        ├── pure_rust_throughput.rs
        ├── local_model_throughput.rs
        └── swarm_dag.rs

crates/
└── efe-nano-sdk/                       (NEW CRATE — public SDK)
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── macros.rs                    (proc-macro for #[nano_agent])
        └── runtime.rs                   (client-side)

mcp-server/                              (EXISTING — extended)
└── mcp_server/
    └── tools/
        └── nano_agents.py               (NEW — exposes M50 tools)

bench/
├── scenarios/                           (EXISTING — extended)
│   └── CB-50_nano_*.yaml                (NEW — 10 scenarios)
└── harness/tests/                        (EXISTING — extended)
    ├── m50_catalog_contract.rs           (NEW)
    ├── m50_runtime_modes.rs              (NEW)
    ├── m50_dispatcher_dag.rs             (NEW)
    ├── m50_mcp_bridge.rs                 (NEW)
    ├── m50_audit_trace_replay.rs         (NEW)
    ├── m50_metering_billing.rs           (NEW)
    ├── m50_starter_pack_contract.rs      (NEW)
    └── m50_integration_e2e.rs            (NEW)

docs/
├── architecture/
│   └── EFE_M50_AGENTIC_ARCHITECTURE.md   (THIS FILE)
├── STABILITY_CONTRACT.md                  (EXISTING — extended with § 30)
└── EFE_CORE_BENCHMARK_CATALOGUE.md        (EXISTING — extended with CB-50_*)
```

---

## 21. Open questions for next iteration

1. **MCP Marketplace listing strategy** — do we publish all 20 starter
   nano agents at once, or stage them ?
2. **Mode B model serving** — own infrastructure (TGI, vLLM) or
   outsource (Modal, RunPod) ?
3. **Pricing tier transitions** — auto-upgrade or require explicit
   customer action ?
4. **GraphQL or REST or gRPC** for the swarm API ?  Currently
   designed REST-first.  gRPC for high-volume customers ?
5. **Multi-region deployment** — replicate catalog or partition ?
6. **Customer-published Mode C nano agents** — allow custom prompts
   or require schema enforcement only ?
7. **Audit-grade tier customer onboarding** — manual review by EFE
   compliance team or self-serve with attestation ?
8. **Conflict between nano agent versions across customers** — strict
   semver or allow customer override ?

These will be answered in subsequent design reviews.

---

_End of M50 architecture document._

_This architecture preserves 100 % of EFE's existing capabilities and
adds the nano agent swarm capability as a strictly additive Layer 4.
The 33 existing crates are unmodified.  The 174 PR #27 scope tests
continue to pass.  The 51 STABILITY § 1-29 freeze tests continue to
pass.  M50 introduces ~ 160 new tests, ~ 5,000 new lines of Rust,
1 new crate, 1 SDK crate, and extends the MCP server by ~ 200 lines._

_Companion : `docs/marketing/EFE-demo-EXP6-agent-2026-05-18.md` is
the wrapper PoC that demonstrates the "B2A" paradigm.  M50 is the
production-grade realization of that PoC._
