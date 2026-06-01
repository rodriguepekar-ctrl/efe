# M07 — Inference Adapter
## Architecture complète, ultra-détaillée
### (polyglot : vLLM local + Claude + GPT + Gemini + Mistral, sous une seule API)

---

**Document** : spécification d'architecture
**Module Python** : `efe_inference` (paquet) + sous-paquets `backends/`, `templating/`, `tools/`, `safety/`, `cache/`, `resilience/`, `accounting/`, `streaming/`, `api/`
**Sprint ciblé** : Sprint 7 (semaines 13-14) du plan MVP
**Dépend de** : M06 (Model Registry & Serving), M10 (Fairness & Compliance), M11 (API Gateway) — pour auth + quotas
**Consommé par** : M11 (proxy final vers clients), M08 (circuit-breaker signals), M10 (safety audit trail)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante
**SLO durs** :
  - TTFT p95 ≤ 350 ms (backend local) / 900 ms (backend hosted)
  - Inter-token latency p95 ≤ 60 ms
  - Cache hit latency p95 ≤ 25 ms
  - Adapter overhead p99 ≤ 15 ms (hors backend)
  - Disponibilité 99.95 %/mois avec fallback chain
  - Zéro requête servie sans PII scrub + moderation check

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP — ce qui est IN et OUT](#2-scope-mvp--ce-qui-est-in-et-out)
3. [Théorie : l'abstraction polyglotte](#3-théorie--labstraction-polyglotte)
4. [Vue d'ensemble — 7 plans](#4-vue-densemble--7-plans)
5. [Cycle de vie d'une requête](#5-cycle-de-vie-dune-requête)
6. [Modèle de données](#6-modèle-de-données)
7. [Backend adapters](#7-backend-adapters)
8. [Templating de prompt](#8-templating-de-prompt)
9. [Tool / function calling](#9-tool--function-calling)
10. [Streaming](#10-streaming)
11. [Safety pipeline](#11-safety-pipeline)
12. [Semantic cache](#12-semantic-cache)
13. [Résilience](#13-résilience)
14. [Token accounting &amp; cost](#14-token-accounting--cost)
15. [NATS topics](#15-nats-topics)
16. [Interfaces publiques](#16-interfaces-publiques)
17. [Stratégie de tests](#17-stratégie-de-tests)
18. [Observabilité](#18-observabilité)
19. [Plan de sprint M07](#19-plan-de-sprint-m07)
20. [Critères d'acceptation](#20-critères-dacceptation)
21. [Risques et mitigations](#21-risques-et-mitigations)
22. [Annexes](#22-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M07 est la **couche d'adaptation polyglotte** entre le routeur M06 (qui
sait **où** router) et les **moteurs d'inférence concrets** (vLLM local,
Claude, GPT, Gemini, Mistral, PEFT custom). Elle répond à cinq
questions :

1. **Comment parler à N moteurs avec une seule API client ?**
   → `InferenceAdapter.generate(ChatRequest) → ChatResponse` uniforme.
2. **Comment formater un prompt différemment par famille de modèle ?**
   → `PromptTemplater` avec templates versionnés par `model_family`.
3. **Comment traduire les outils/functions entre OpenAI et Anthropic ?**
   → `ToolAdapter` bi-directionnel, avec validation JSON Schema.
4. **Comment garantir qu'aucune requête ne fuite de PII ni ne viole de
   policy ?** → pipeline `Safety` en entrée + sortie, audit-trail signé.
5. **Comment tenir le SLO même quand un backend tombe ?**
   → circuit-breaker + retry + fallback chain + semantic cache.

En une phrase : **M07 est le traducteur universel de l'EFE** — tout
changement de modèle (promotion M05, bascule M06, fallback M07) est
invisible aux clients, et toutes les garanties (PII, cost, latence,
safety) sont appliquées uniformément quel que soit le backend.

### 1.2 Contraintes non-négociables

| Contrainte | Valeur | Preuve |
|---|---|---|
| PII scrub obligatoire | tout input/output passe Presidio + allowlist | compteur `efe_inference_safety_scrub_total` à 100 % coverage |
| Content moderation | tout prompt utilisateur + toute réponse | audit trail par `request_id` |
| TTFT p95 local | ≤ 350 ms | histogram Prometheus + alerte |
| TTFT p95 hosted | ≤ 900 ms | idem |
| Adapter overhead p99 | ≤ 15 ms (hors backend) | histogramme dédié |
| Token accounting | exact à ± 0 token via tokenizer officiel | reconciliation quotidienne vs facturation |
| Cache pas de cross-tenant | partition clé par `tenant_id` | test dédié + audit |
| Fallback déterministe | ordre stable : primary → secondary → safe_local | config gelée, testé |
| Audit rollback-safe | chaque call → ligne signée dans chaîne M05 si flagged | HMAC via `AuditChannel` |

### 1.3 Position dans le pipeline EFE

```
                      ┌─────────────────────────────┐
                      │  M11 API Gateway (AuthN/Z)  │
                      └──────────────┬──────────────┘
                                     │ /v1/chat (or /v1/completions)
                                     ▼
              ┌──────────────────────────────────────────────┐
              │      M07 InferenceAdapter (stateless)        │
              │  ┌────────────────────────────────────────┐  │
              │  │ 1. Normalize → ChatRequest              │  │
              │  │ 2. Safety.preflight(input)              │  │
              │  │ 3. Cache.lookup(semantic)               │  │
              │  │ 4. BackendSelector.pick (via M06)       │  │
              │  │ 5. PromptTemplater.render               │  │
              │  │ 6. Backend.generate (streamed)          │  │
              │  │ 7. ToolAdapter.normalize_response       │  │
              │  │ 8. Safety.postflight(output)            │  │
              │  │ 9. Cache.put                            │  │
              │  │ 10. TokenAccount.bill                   │  │
              │  │ 11. Telemetry.emit                      │  │
              │  └────────────────────────────────────────┘  │
              └──────────────┬───────────────────────────────┘
                             │
                             ▼  (per-backend dispatch)
       ┌────────────┬─────────────┬─────────────┬─────────────┐
       │ vLLM local │  Claude API │   GPT API   │  Mistral    │
       │  (via M06) │ (anthropic) │  (openai)   │  (hosted)   │
       └────────────┴─────────────┴─────────────┴─────────────┘

  Amont : M11 forwarde après auth/quota
  Aval  : M06 pour routing "local" ; SDKs externes pour hosted
          M10 reçoit les flags safety
          M08 reçoit les signaux circuit-breaker
```

### 1.4 Relations inter-modules

**M06 → M07** : M07 **ne route pas**. Pour les backends locaux
(`backend_kind == "vllm_local"`), M07 consulte le `RouteDecision` de M06
via un appel gRPC côté control-plane, puis pousse la requête vers
l'URL du pod. Les artefacts, versions et canary progressifs sont pris
en charge par M06.

**M07 → M08** : quand un backend accumule trop d'erreurs, le
`CircuitBreaker` passe en état `OPEN` et émet
`efe.inference.backend.circuit_opened`. M08 peut réagir (rollback
déclenché, kill-switch).

**M07 → M10** : chaque flag `safety_block` est publié sur
`efe.inference.safety.flagged` avec payload signé HMAC ; M10 le
consomme pour enrichir Model Card + compliance reports.

**M07 → M11** : M11 forwarde les requêtes client après auth + quota.
M07 expose `/v1/chat`, `/v1/completions`, `/v1/embeddings`
OpenAI-compatibles + `/v1/messages` Anthropic-compatible.

### 1.5 Ce qui N'EST PAS dans M07

| Hors scope | Raison | Où ça vit |
|---|---|---|
| Runtime vLLM (load/serve) | Déjà dans M06.serving | M06 |
| Routing `(tenant, task) → endpoint` | M06.TenantRouter | M06 |
| Cosign verify | M06.signing | M06 |
| Canary progression | M06.canary | M06 |
| Rollback exec | M06.rollback | M06 |
| Auth OIDC / API key | M11.api_gateway | M11 |
| Rate-limit global | M11.rate_limit | M11 |
| Fairness metrics | M10 | M10 |
| Live metrics collection | M12 (observability infra) | M12 |

**M07 consomme** ces services mais n'en est pas responsable.

---

## 2. Scope MVP — ce qui est IN et OUT

### 2.1 IN au MVP (sprint 7, 2 semaines)

**Backends supportés** :

- **vLLM local** (via M06) — adapter `VllmBackend`.
- **Anthropic Claude** (Sonnet 3.5, Opus 3) — adapter `ClaudeBackend`.
- **OpenAI** (GPT-4o, GPT-4o-mini, GPT-3.5) — adapter `OpenAiBackend`.
- **Mistral hosted** (Mistral Large) — adapter `MistralBackend`.
- **PEFT local runtime** (adapter LoRA direct sans vLLM) pour tests et
  tenants < 10 req/s — adapter `PeftBackend`.

**Capabilities** :

- Chat completions (messages API).
- Text completions (legacy OpenAI).
- Embeddings (OpenAI `text-embedding-3-*` + local `sentence-transformers`).
- Streaming SSE + gRPC bidi.
- Tool / function calling (OpenAI + Anthropic dialects normalisés).
- Multimodal **text + image in** (Claude vision, GPT-4o vision, local
  VLM avec adapter dédié).
- Semantic cache (Redis + RediSearch).
- Verbatim cache (hash exact pour prompt déterministes).
- PII scrub (Presidio) + content moderation (OpenAI Moderations +
  local keyword bloc-list).
- Prompt-injection detection heuristique + LLM judge optionnel.
- Circuit breaker 3-états par (backend, tenant).
- Retry + jitter exponentiel.
- Fallback chain (primary → secondary → safe_local).
- Token counting exact (tiktoken, anthropic-tokenizer, sentencepiece).
- Cost tracking ($/1K prompt tokens, $/1K completion tokens).
- Prompt templates pour 5 familles (ChatML, Llama-3-Instruct,
  Claude, OpenAI, Gemini).
- Audit trail signé pour tout flag safety.
- Metrics Prometheus + traces OpenTelemetry + logs structurés.

### 2.2 OUT au MVP

| Fonctionnalité | Raison | Jalon |
|---|---|---|
| Speculative decoding coordination | complexité vs gain moyen | v1.1 |
| Cross-request batching "continuous" | déjà géré par vLLM | reste owned by M06 |
| Audio in/out (Whisper, TTS) | hors périmètre core | v1.2 |
| Vidéo | trop spécialisé | v2.0 |
| Fine-grained multi-agent orchestration | LangGraph-style | v1.1 optionnel |
| Multi-vendor embedding ranking | rerankers | v1.1 |
| Retrieval-augmented generation (RAG) built-in | vit dans M11 ou app layer | — |
| Model distillation on-the-fly | M04 l'handle offline | — |
| Custom fine-tuning from chat | M03/M04 | — |

### 2.3 Roadmap post-MVP

- **v1.1** : RAG helpers, speculative decoding, rerankers, audio, LLM
  prompt-injection judge avancé.
- **v1.2** : vidéo, cross-region backend routing, self-hosted
  embedding models.
- **v2.0** : agentic orchestration native (state machines de tool
  calls multi-tours, planning).

---

## 3. Théorie : l'abstraction polyglotte

### 3.1 Le problème à résoudre

Un écosystème LLM moderne a **au moins 6 API différentes** pour "parler
à un modèle" :

- OpenAI `/v1/chat/completions` (messages `{role, content, tool_calls}`).
- Anthropic `/v1/messages` (messages `{role, content: [text/image/tool_use]}`).
- Google Gemini `generateContent` (structure `contents: [parts]`).
- Mistral `/v1/chat/completions` (OpenAI-like, mais subtilités).
- Cohere / AI21 / custom.
- vLLM local OpenAI-compat mais sans tool_use natif.

Chaque API a :
- Son **format de prompt** (chat template propre ou tokens spéciaux).
- Son **format tool call** différent.
- Son **tokenizer différent** (gpt-4 vs Claude vs Llama-3).
- Ses **codes d'erreur différents**.
- Ses **formats de streaming différents**.
- Ses **tarifs différents**.

**Sans couche d'abstraction**, chaque équipe client écrit son propre
wrapper, applique sa propre PII scrub, son propre retry — avec des
garanties inégales. **Avec M07**, toutes ces préoccupations sont
centralisées, testées, auditées.

### 3.2 Invariants fondamentaux

**I1 — Idempotence de normalisation**
> Pour toute paire `(ChatRequest A, ChatRequest B)` telle que
> `canonical(A) == canonical(B)`, les normalisations produisent le même
> prompt formaté pour un même `(model_family, template_version)`.

**I2 — Round-trip tool call**
> Pour toute `ToolCall` en format OpenAI traversant l'adapter
> Anthropic puis retour, le JSON final est équivalent modulo
> `id` (regénéré côté Anthropic).

**I3 — PII non-fuitée en telemetry**
> Tout log, metric label, span attribute, audit entry ne contient
> **jamais** le texte brut du prompt ou de la réponse après la phase
> Safety. Seuls les hashes SHA-256 + les flags de catégorie sont
> persistés.

**I4 — Determinisme de fallback**
> Pour un `(tenant, task)` donné, la fallback chain est lue une fois
> par démarrage de pod et ne change pas pendant la vie de l'adapter.
> Deux requêtes identiques ayant les mêmes backends tous verts
> aboutissent au **même backend primaire**.

**I5 — Cache strict-tenant**
> Pour tout lookup semantic cache, la clé inclut `tenant_id`. Aucun
> hit ne traverse jamais une frontière tenant, même si le texte
> embeddé est identique.

**I6 — Token count exact**
> Pour toute réponse, `prompt_tokens + completion_tokens` correspond
> au comptage du **tokenizer officiel du modèle** ; nous ne donnons
> jamais d'estimation. Écart maximal accepté : **0 token**.

**I7 — Circuit breaker directionnel**
> Un backend en `OPEN` refuse la requête **avant** tout appel réseau.
> Il ne bloque jamais une tentative de récupération si `HALF_OPEN`
> parce que le retry passe par une fenêtre de sondage dédiée.

**I8 — Safety pre-block stops downstream**
> Si `Safety.preflight` retourne `block`, **aucun appel backend** n'est
> émis, **aucune facturation** n'est enregistrée, la réponse cliente
> est une erreur safety standard avec code `content_policy_violation`.

### 3.3 Budget temporel d'une requête non-streamée

```
Client t=0
  │
  │ ──(network + M11)── t=2 ms
  ▼
M07 reçoit ChatRequest
  ├── Normalize                     0.5 ms
  ├── Safety.preflight (PII + mod)  3.0 ms   (local regex + Presidio)
  ├── Cache.lookup (semantic)       5.0 ms   (Redis + embed 0 token)
  ├── Backend.select                0.2 ms   (in-memory)
  ├── Template.render               0.3 ms
  ├── Backend.generate             200 ms    (local 7B, full completion)
  ├── Tool normalize                0.2 ms
  ├── Safety.postflight             3.0 ms
  ├── Cache.put                     1.0 ms   (async fire-and-forget)
  ├── TokenAccount.bill             0.1 ms
  ├── Telemetry.emit                0.5 ms
  └── Serialize out                 0.3 ms
                                   ──────
 Total adapter overhead (hors backend) : ≈ 14 ms p99
 + backend 200 ms = 214 ms end-to-end
```

Pour le streaming, le TTFT (time-to-first-token) suit une trajectoire
différente :

```
t=0  : Client → M11 → M07
t=5  : Normalize + Safety.preflight + Cache miss + Backend.select + Template
t=6  : Backend.stream.start (first byte out toward M07)
t=40 : First token from backend (local 7B A100)
t=45 : SSE chunk out toward client (safety postflight deferred to end)

TTFT client-side ≈ 45 ms  (local, A100)
```

Hosted backends ajoutent ~500-800 ms de round-trip réseau.

### 3.4 Topologie de panne

```
Client
  │
  ▼
M11 (indépendant de M07)
  │
  ▼
M07 pod A ─── fail? ───► M07 pod B (replica K8s, stateless)
  │
  ▼
Backend fallback chain :
  1. vLLM-local (primary)
  2. Claude hosted (secondary)
  3. safe-local-7B (last resort)

Chaque étape du chain a son propre :
- CircuitBreaker  (3-state : CLOSED / OPEN / HALF_OPEN)
- RetryPolicy     (max_attempts, backoff_base, jitter, timeout)
- TimeoutBudget   (connect, first_byte, inter_token)
```

Si `vLLM-local` OPEN et `Claude` OPEN, on tombe sur `safe-local-7B`.
Si *aussi* OPEN, on renvoie 503 avec header `Retry-After`.

---

## 4. Vue d'ensemble — 7 plans

### 4.1 Request Plane (normalisation input)

- `ChatRequest` dataclass canonique (messages, tools, sampling,
  metadata).
- `RequestNormalizer` : accepte OpenAI format, Anthropic format,
  native EFE format ; produit `ChatRequest`.
- `RequestValidator` : schéma Pydantic, refus sur champs inconnus.

### 4.2 Backend Plane (adapters)

- `BackendAdapter` protocol.
- Implémentations : `VllmBackend`, `ClaudeBackend`, `OpenAiBackend`,
  `MistralBackend`, `PeftBackend`.
- `BackendRegistry` : mapping `(model_name, capability) → Adapter`.
- `BackendSelector` : applique la `FallbackChain`, pilote retries.

### 4.3 Response Plane (normalisation output)

- `ChatResponse` canonique (choices, usage, model, finish_reason,
  tool_calls, content).
- `ResponseNormalizer` inverse du Request Plane.
- `StreamMultiplexer` : reçoit chunks par backend, émet SSE / gRPC
  uniformément.

### 4.4 Safety Plane (filtres)

- `PiiScrubber` (Presidio + regex custom).
- `ContentModerator` (OpenAI Moderations + local blocklist).
- `PromptInjectionDetector` (heuristique + allowlist).
- `OutputPolicyFilter` (catégories PG-13 / professional / custom).
- `SafetyAuditor` (émet vers M10 via NATS + audit chain M05).

### 4.5 Cache Plane (semantic + verbatim)

- `VerbatimCache` (SHA-256 du prompt + params → réponse).
- `SemanticCache` (embedding + cosine threshold + RediSearch).
- Invalidation : sur promotion M05 (nouveau modèle → drop cache par
  `(tenant, task)`).

### 4.6 Resilience Plane (retry + circuit + fallback)

- `CircuitBreaker` par `(backend_id, tenant)`.
- `RetryPolicy` (exponentiel, jitter, timeout par tentative).
- `FallbackChain` (ordre stable).
- `TimeoutBudget` (deadline propagée depuis le client).

### 4.7 Observability Plane

- Metrics : TTFT, ITL, tokens/sec, cache hit, safety flags, circuit
  state, cost $/1K.
- Traces : spans par stage.
- Logs : échantillonnés, avec redaction automatique (pas de PII).
- Dashboards Grafana pré-provisionnés.

---

## 5. Cycle de vie d'une requête

### 5.1 Flow détaillé (requête non-streamée)

```
1. Reception
   HTTP POST /v1/chat/completions
   headers: X-EFE-Tenant-Id, X-EFE-Task-Id, X-EFE-Request-Id
   body: OpenAI-format ChatRequest
      ↓
2. RequestNormalizer.normalize(raw) → ChatRequest canonique
      ↓
3. RequestValidator.validate(ChatRequest) → OK | ValidationError
      ↓
4. SafetyContext = Safety.preflight(ChatRequest)
      ├── PiiScrubber.scrub(messages)       → scrubbed_messages, pii_flags
      ├── ContentModerator.check(messages)  → categories
      ├── InjectionDetector.check(messages) → risk_score
      └── Policy.decide(flags)              → ALLOW | BLOCK | REVIEW
     if BLOCK → raise SafetyBlocked, emit audit event, return 403
      ↓
5. CacheKey = CacheKeyBuilder.build(ChatRequest, tenant, task)
   verbatim_hit = VerbatimCache.get(CacheKey)
   if verbatim_hit → return (no backend call)
   semantic_hit = SemanticCache.get(ChatRequest, tenant, task, threshold)
   if semantic_hit → return (no backend call)
      ↓
6. RouteDecision = ModelRegistryClient.route(tenant, task, variant)
      ↓  (M06 call, cached 1s)
7. BackendSelector.select(RouteDecision, FallbackChain) → chain
      ↓
8. For each backend in chain:
      - CircuitBreaker.check(backend, tenant) → if OPEN, skip
      - RetryLoop:
          prompt = PromptTemplater.render(backend.family, ChatRequest)
          result = Backend.generate(prompt, sampling, tools)
          if result.ok → break
          else retry with backoff
      - If success, break outer loop
   if exhausted → raise AllBackendsDown
      ↓
9. Response = ResponseNormalizer.normalize(backend_result)
   ToolAdapter.normalize(Response.tool_calls)
      ↓
10. Safety.postflight(Response) → categories, scrubbed_output
    if blocked → 403 + audit
      ↓
11. VerbatimCache.put(CacheKey, Response) (async)
    SemanticCache.put(ChatRequest, Response, embedding) (async)
      ↓
12. TokenAccount.bill(tenant, backend, prompt_tokens, completion_tokens)
      ↓
13. Telemetry.emit(span, metrics, log)
      ↓
14. return Response → M11 → client
```

### 5.2 Flow streamé (chunks)

Identique jusqu'à l'étape 8, puis :

```
8'. For chunk in Backend.stream(...):
      - Accumulator += chunk.delta
      - yield SSE chunk to client
10'. After stream end:
      - Safety.postflight(Accumulator.final_text) → may append
        [SafetyBlocked] sentinel if violated post-hoc
11'. Cache.put(final) (async)
12'. TokenAccount.bill(final_counts)
```

Safety post-flight est en **différé** sur streaming pour ne pas
bloquer les premiers tokens. Si violation détectée, le stream se
termine avec un chunk `[SAFETY_BLOCK]` et le client voit
`finish_reason: content_filter`.

### 5.3 Sentinelle d'arrêt anticipé

Pendant un stream, l'adapter surveille :
- Longueur cumulative (hard cap `max_tokens`).
- Règles d'arrêt custom (regex match).
- Cancellation client (connection drop ⇒ backend abort via RPC).
- Deadline dépassée (`timeout_budget_s`).

---

## 6. Modèle de données

### 6.1 Tables Postgres (DDL condensé)

```sql
-- ============================================================
-- BackendRegistration — métadonnées par backend déployé
-- ============================================================
CREATE TABLE inference_backends (
    backend_id        UUID PRIMARY KEY,
    name              TEXT NOT NULL,              -- "claude-sonnet-3.5"
    kind              TEXT NOT NULL,              -- vllm_local|anthropic|openai|mistral|peft
    endpoint_url      TEXT,                        -- for hosted: base URL
    api_key_secret_id TEXT,                        -- KMS secret reference
    model_family      TEXT NOT NULL,              -- claude|gpt|llama3|mistral
    tokenizer_name    TEXT NOT NULL,              -- cl100k_base|claude|sentencepiece-...
    capabilities      JSONB NOT NULL,             -- {chat: true, tools: true, vision: true, ...}
    cost_per_1k_prompt_usd       DECIMAL(10, 6),
    cost_per_1k_completion_usd   DECIMAL(10, 6),
    max_context_tokens           INTEGER NOT NULL,
    rate_limit_rpm               INTEGER,
    priority                     SMALLINT NOT NULL DEFAULT 100,
    state             TEXT NOT NULL CHECK (state IN ('active','draining','disabled')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (name)
);

-- Mapping tenant → chain (ordered). Per (tenant, task) if present,
-- else falls back to (tenant, NULL), else (NULL, NULL).
CREATE TABLE inference_fallback_chains (
    tenant_id         TEXT NOT NULL,
    task_id           TEXT,                         -- NULLable
    position          SMALLINT NOT NULL,
    backend_id        UUID NOT NULL REFERENCES inference_backends(backend_id),
    timeout_ms        INTEGER NOT NULL DEFAULT 30000,
    weight_pct        SMALLINT NOT NULL DEFAULT 100,
    PRIMARY KEY (tenant_id, task_id, position)
);

-- ============================================================
-- PromptTemplate — versioned Jinja2 templates per model_family
-- ============================================================
CREATE TABLE prompt_templates (
    template_id       UUID PRIMARY KEY,
    name              TEXT NOT NULL,              -- chat|completion|tool_call
    model_family      TEXT NOT NULL,              -- claude|gpt|llama3|mistral|chatml
    version           TEXT NOT NULL,              -- semver
    content           TEXT NOT NULL,              -- Jinja2 template
    content_hash      BYTEA NOT NULL,             -- SHA-256
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deprecated_at     TIMESTAMPTZ,
    UNIQUE (name, model_family, version)
);

-- ============================================================
-- ToolSchema — JSON Schema of each tool exposed
-- ============================================================
CREATE TABLE tool_schemas (
    tool_id           UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL,
    name              TEXT NOT NULL,
    version           TEXT NOT NULL,
    description       TEXT NOT NULL,
    parameters_schema JSONB NOT NULL,              -- JSON Schema draft 2020-12
    parameters_hash   BYTEA NOT NULL,              -- SHA-256
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deprecated_at     TIMESTAMPTZ,
    UNIQUE (tenant_id, name, version)
);

-- ============================================================
-- SafetyPolicy — per-tenant policy bundle
-- ============================================================
CREATE TABLE safety_policies (
    policy_id         UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL,
    name              TEXT NOT NULL,
    pii_categories    JSONB NOT NULL DEFAULT '["email","phone","iban","ssn","credit_card"]',
    moderation_thresholds JSONB NOT NULL DEFAULT '{}',
    block_on_injection_risk SMALLINT NOT NULL DEFAULT 80,  -- 0-100
    allow_list_regex  JSONB NOT NULL DEFAULT '[]',
    deny_list_regex   JSONB NOT NULL DEFAULT '[]',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    active_from       TIMESTAMPTZ,
    active_until      TIMESTAMPTZ,
    UNIQUE (tenant_id, name)
);

-- ============================================================
-- InferenceLog — sampled request log (NO PII, only hashes)
-- ============================================================
CREATE TABLE inference_logs (
    request_id        UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    backend_id        UUID REFERENCES inference_backends(backend_id),
    prompt_hash       BYTEA,                        -- SHA-256 (scrubbed)
    response_hash     BYTEA,                        -- SHA-256 (scrubbed)
    prompt_tokens     INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    ttft_ms           INTEGER,
    total_latency_ms  INTEGER,
    finish_reason     TEXT,
    cache_kind        TEXT,                         -- verbatim|semantic|miss
    safety_flags      JSONB NOT NULL DEFAULT '{}',
    cost_usd          DECIMAL(12, 8),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX ix_inf_logs_tenant_time
    ON inference_logs (tenant_id, created_at DESC);

-- ============================================================
-- SafetyEvent — every block / flag (rich audit trail)
-- ============================================================
CREATE TABLE safety_events (
    event_id          UUID PRIMARY KEY,
    request_id        UUID NOT NULL,
    tenant_id         TEXT NOT NULL,
    stage             TEXT NOT NULL CHECK (stage IN (
                          'preflight_pii','preflight_moderation',
                          'preflight_injection','postflight_pii',
                          'postflight_moderation','postflight_policy')),
    severity          TEXT NOT NULL CHECK (severity IN (
                          'info','warn','block')),
    categories        JSONB NOT NULL,
    payload_hash      BYTEA NOT NULL,                -- SHA-256
    signature_b64     TEXT NOT NULL,                 -- HMAC of audit
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX ix_safety_tenant_time
    ON safety_events (tenant_id, created_at DESC);

-- ============================================================
-- CircuitState — persisted last-known state (rehydration on boot)
-- ============================================================
CREATE TABLE circuit_states (
    tenant_id         TEXT NOT NULL,
    backend_id        UUID NOT NULL REFERENCES inference_backends(backend_id),
    state             TEXT NOT NULL CHECK (state IN ('closed','open','half_open')),
    opened_at         TIMESTAMPTZ,
    failure_count     INTEGER NOT NULL DEFAULT 0,
    success_count     INTEGER NOT NULL DEFAULT 0,
    last_transition_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, backend_id)
);
```

### 6.2 Schéma Redis

```
# Verbatim cache (exact hash match)
inf:cache:v:{tenant}:{task}:{hash}      STRING (JSON ChatResponse), TTL 3600s

# Semantic cache — RediSearch index
inf:cache:s:{tenant}:{task}             HASH (per entry with embedding vector)
    embedding_vec     FLOAT32 x 1536
    prompt_hash       STRING
    response_json     STRING
    created_at_ms     INT
    ttl_expires_ms    INT
FT.CREATE inf:cache:s:{tenant}:{task} ... VECTOR FIELD embedding_vec

# Circuit breaker hot state (synced to DB on transition)
inf:circuit:{tenant}:{backend_id}       HASH
    state            CLOSED|OPEN|HALF_OPEN
    failure_count    INT
    success_count    INT
    opened_at_ms     INT
    last_transition_ms INT

# Token-bucket per-backend rate limit
inf:rate:{backend_id}:{tenant}          HASH (same as M06 quota)

# Dead-letter queue (requests that failed all backends)
inf:dlq                                 STREAM
```

### 6.3 Canonical ChatRequest

```python
@dataclass(frozen=True, slots=True)
class ChatMessage:
    role: Literal["system", "user", "assistant", "tool"]
    content: tuple[ContentBlock, ...]        # text, image, tool_use, tool_result
    name: str | None = None                   # for tool role
    tool_call_id: str | None = None

@dataclass(frozen=True, slots=True)
class ContentBlock:
    kind: Literal["text", "image", "tool_use", "tool_result"]
    text: str | None = None
    image_url: str | None = None             # data: or https:
    image_mime: str | None = None
    tool_call_id: str | None = None
    tool_name: str | None = None
    tool_args_json: str | None = None
    tool_result_content: str | None = None

@dataclass(frozen=True, slots=True)
class SamplingParams:
    temperature: float = 1.0
    top_p: float = 1.0
    top_k: int | None = None
    max_tokens: int = 1024
    stop: tuple[str, ...] = ()
    presence_penalty: float = 0.0
    frequency_penalty: float = 0.0
    seed: int | None = None

@dataclass(frozen=True, slots=True)
class ToolDef:
    name: str
    description: str
    parameters_json_schema: dict

@dataclass(frozen=True, slots=True)
class ChatRequest:
    tenant_id: str
    task_id: str
    model_hint: str | None                      # "claude-sonnet-3.5" or None = auto
    messages: tuple[ChatMessage, ...]
    sampling: SamplingParams
    tools: tuple[ToolDef, ...] = ()
    tool_choice: Literal["auto", "required", "none"] = "auto"
    stream: bool = False
    user_id: str | None = None
    request_id: str                              # UUIDv7
    timeout_budget_s: float = 30.0
    metadata: dict[str, str] = None
```

### 6.4 Canonical ChatResponse

```python
@dataclass(frozen=True, slots=True)
class Usage:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int                           # = prompt + completion
    prompt_cache_hits: int = 0                  # Anthropic-style prompt cache

@dataclass(frozen=True, slots=True)
class ChatChoice:
    index: int
    message: ChatMessage
    finish_reason: Literal[
        "stop", "length", "content_filter",
        "tool_calls", "timeout", "backend_error",
    ]

@dataclass(frozen=True, slots=True)
class ChatResponse:
    request_id: str
    model: str                                   # concrete backend model
    choices: tuple[ChatChoice, ...]
    usage: Usage
    backend_kind: str                            # vllm_local|anthropic|...
    backend_latency_ms: float                    # pure backend
    adapter_overhead_ms: float                   # everything but backend
    safety_flags: dict[str, list[str]]            # category → subcategories
    cached: Literal["no", "verbatim", "semantic"] = "no"
```

---

## 7. Backend adapters

### 7.1 Protocol `BackendAdapter`

```python
class BackendAdapter(Protocol):
    """Uniform interface over all LLM backends."""

    kind: BackendKind
    model_family: ModelFamily

    async def generate(
        self, req: BackendRequest,
    ) -> BackendResponse:
        ...

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[BackendChunk]:
        ...

    async def embed(
        self, texts: list[str], *, model: str,
    ) -> list[list[float]]:
        ...

    async def health(self) -> HealthStatus:
        ...

    async def close(self) -> None:
        ...
```

`BackendRequest` est déjà rendu sous le format attendu par le backend
(e.g. prompt Claude XML ou OpenAI messages array) — le formatting est
fait par `PromptTemplater` en amont.

### 7.2 `VllmBackend` (délègue à M06)

```python
@dataclass
class VllmBackend:
    kind = BackendKind.VLLM_LOCAL
    model_family: ModelFamily

    m06_router_url: str            # internal gRPC to M06 TenantRouter
    http_client: httpx.AsyncClient  # HTTP/2, keep-alive

    async def generate(self, req):
        # 1. Ask M06 for the route (cached 1s at M07 too).
        route = await self._route(req.tenant_id, req.task_id)
        # 2. Forward the request to route.endpoint_url.
        payload = self._render_openai(req)
        r = await self.http_client.post(
            f"{route.endpoint_url}/v1/chat/completions",
            json=payload,
            timeout=req.timeout_s,
        )
        return self._parse_openai(r.json(), req)
```

La clé est que vLLM expose déjà une API OpenAI-compat — `VllmBackend`
se limite donc à un wrap httpx avec le routing M06.

### 7.3 `ClaudeBackend` (Anthropic)

```python
@dataclass
class ClaudeBackend:
    kind = BackendKind.ANTHROPIC
    model_family = ModelFamily.CLAUDE

    api_key: SecretStr
    base_url: str = "https://api.anthropic.com"
    http_client: httpx.AsyncClient

    async def generate(self, req):
        # Translate ChatMessage → Anthropic messages array.
        messages, system = self._to_anthropic(req.messages)
        tools = self._to_anthropic_tools(req.tools)
        payload = {
            "model": req.model,
            "messages": messages,
            "system": system,
            "tools": tools,
            "max_tokens": req.sampling.max_tokens,
            "temperature": req.sampling.temperature,
            # ...
        }
        r = await self.http_client.post(
            f"{self.base_url}/v1/messages",
            headers={
                "x-api-key": self.api_key.get_secret_value(),
                "anthropic-version": "2023-06-01",
            },
            json=payload, timeout=req.timeout_s,
        )
        return self._from_anthropic(r.json(), req)
```

Spécificités Claude :
- Messages alternent `user/assistant` strictement.
- `system` est un champ top-level (pas dans messages).
- `tool_use` et `tool_result` sont des content blocks, pas des rôles.
- Vision via `{type: "image", source: {...}}` blocks.

### 7.4 `OpenAiBackend`

```python
@dataclass
class OpenAiBackend:
    kind = BackendKind.OPENAI
    model_family = ModelFamily.GPT

    api_key: SecretStr
    base_url: str = "https://api.openai.com/v1"
    org_id: str | None = None
```

L'API est la référence de notre format canonique pour chat : les
adapters "vers" OpenAI sont des identités quasi-triviales. Le
tokenizer est `tiktoken.encoding_for_model(name)`.

### 7.5 `MistralBackend`

OpenAI-compatible avec des nuances (pas de `tool_choice: "none"`,
certains champs ignorés). Adapter isole ces quirks.

### 7.6 `PeftBackend` (local, sans vLLM)

Pour les tenants très petits ou les tests : charge l'adapter LoRA
directement via `peft.PeftModel` + `transformers`. Débit faible (~5-10
req/s sur A100), mais zéro dépendance runtime vLLM. Utilisé en
fallback "safe" dans la chain.

### 7.7 `GeminiBackend` (optionnel MVP, sinon v1.1)

### 7.8 `BackendRegistry` et `BackendFactory`

```python
class BackendRegistry:
    def __init__(self) -> None:
        self._adapters: dict[str, BackendAdapter] = {}

    def register(self, name: str, adapter: BackendAdapter) -> None: ...
    def get(self, name: str) -> BackendAdapter: ...
    def list_by_capability(self, cap: Capability) -> list[str]: ...
    async def health_all(self) -> dict[str, HealthStatus]: ...
```

La factory lit la table `inference_backends` au démarrage, instancie
l'adapter approprié, et enregistre dans `BackendRegistry`.

### 7.9 Health probing

Chaque adapter implémente `health()`. Un daemon sidecar probe tous les
backends toutes les 30 s ; un backend `health=unhealthy` passe
automatiquement en `draining` côté `BackendRegistry` et reçoit 0
trafic jusqu'à recovery (3 probes verts consécutifs).

---

## 8. Templating de prompt

### 8.1 Rôle du templater

Chaque modèle attend un format "tokens spéciaux" différent :

- **Llama-3-Instruct** : `<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\n{content}<|eot_id|>...`
- **ChatML** (anciennement Mistral instruct) : `<|im_start|>user\n{content}<|im_end|>\n`
- **Claude Sonnet 3.5** : `\n\nHuman: {content}\n\nAssistant:` (legacy) ou JSON messages via API.
- **GPT-4** : API messages (pas de tokens exposés).
- **Gemini** : structure `contents: [{role, parts: [...]}]`.

Notre `PromptTemplater` :
1. Prend un `ChatRequest` canonique.
2. Regarde le `model_family`.
3. Applique le template Jinja2 approprié (chargé depuis `prompt_templates`).
4. Retourne la chaîne formatée (ou l'objet structuré pour APIs JSON).

### 8.2 Exemple : template Llama-3

```jinja2
{%- if system -%}
<|begin_of_text|><|start_header_id|>system<|end_header_id|>

{{ system }}<|eot_id|>
{%- else -%}
<|begin_of_text|>
{%- endif -%}
{%- for msg in messages -%}
<|start_header_id|>{{ msg.role }}<|end_header_id|>

{{ msg.content_text }}<|eot_id|>
{%- endfor -%}
<|start_header_id|>assistant<|end_header_id|>

```

### 8.3 Versioning

Un template est adressé par `(name, model_family, version)`. La
promotion d'une nouvelle version passe par :

1. Propose PR avec le nouveau template.
2. CI run `pytest tests_inference/test_templates.py` qui compare sur
   10 cas de référence (fixtures) l'output hash.
3. Si changement intentionnel, documenter dans CHANGELOG + bumper
   semver.
4. `deprecated_at` sur l'ancienne version 90 jours après.

### 8.4 Test fixtures

Pour chaque `(model_family, template_version)`, on stocke un ensemble
de 10 cas `ChatRequest → expected_prompt_bytes` dans
`tests_inference/fixtures/templates/`. Toute évolution de template
doit mettre à jour ces fixtures (sinon test fail) — c'est un
changement conscient.

---

## 9. Tool / function calling

### 9.1 Schéma canonique

Un outil a :
- `name` (identifiant stable, regex `^[a-z_][a-z0-9_]*$`).
- `description` (ce que fait l'outil, pour que le modèle décide).
- `parameters_json_schema` (JSON Schema draft 2020-12).

Côté modèle, la sortie est :
- `tool_calls: [{name, arguments: <json>}]`.

### 9.2 Translation OpenAI ↔ Anthropic

**OpenAI format** :

```json
{
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "get_weather",
        "description": "...",
        "parameters": {...}
      }
    }
  ]
}
```

**Anthropic format** :

```json
{
  "tools": [
    {
      "name": "get_weather",
      "description": "...",
      "input_schema": {...}
    }
  ]
}
```

Le mapping est bijectif modulo la clé `parameters`↔`input_schema` et
le wrap `function:{...}`. `ToolAdapter` l'automatise.

### 9.3 Tool call response

**OpenAI** : `tool_calls: [{id, type: "function", function: {name, arguments: "<JSON string>"}}]`.

**Anthropic** : content block `{type: "tool_use", id, name, input: {...}}`.

`ToolAdapter.normalize_response` les ramène au format canonique :

```python
@dataclass(frozen=True, slots=True)
class ToolCall:
    id: str
    name: str
    arguments: dict[str, object]    # parsed JSON
```

### 9.4 Validation et repair

Les LLM produisent parfois du JSON invalide ou des clés manquantes.
`ToolValidator` :

1. Tente parse JSON → échec → `JsonRepair` (heuristiques : fix quotes,
   trailing commas, unescaped backslashes).
2. Valide contre `parameters_json_schema` via `jsonschema`.
3. Si validation échoue → option : re-appeler le modèle avec le
   message d'erreur (jusqu'à 2 tentatives), ou renvoyer une réponse
   d'erreur standard `tool_arguments_invalid`.

### 9.5 Tool call via constrained generation (backends locaux)

Pour `PeftBackend` et `VllmBackend`, on peut activer le **grammar
constrained decoding** (vLLM guided decoding) :

```python
sampling = SamplingParams(
    guided_json=tool.parameters_json_schema,
)
```

Ceci force le modèle à émettre du JSON valide du premier coup, évitant
le round-trip repair.

---

## 10. Streaming

### 10.1 SSE (Server-Sent Events)

```
GET /v1/chat/completions (stream=true)

data: {"choices":[{"delta":{"role":"assistant","content":""}}]}

data: {"choices":[{"delta":{"content":"Hello"}}]}

data: {"choices":[{"delta":{"content":" world"}}]}

data: {"choices":[{"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

### 10.2 gRPC server streaming

```proto
service InferenceService {
  rpc StreamChat(ChatRequestPb) returns (stream ChatChunkPb);
}
```

### 10.3 Chunk normalization

Chaque backend émet des chunks différents :

- **OpenAI** : SSE `{choices: [{delta: {content: "..."}}]}`.
- **Anthropic** : SSE events typés `content_block_delta`, `message_stop`.
- **Gemini** : chunks de `GenerateContentResponse`.
- **vLLM** : SSE OpenAI-compat.

`StreamMultiplexer` les normalise en une séquence de `ChatChunk` :

```python
@dataclass(frozen=True, slots=True)
class ChatChunk:
    delta_text: str | None
    delta_tool_calls: list[ToolCallDelta] | None
    finish_reason: str | None
    usage: Usage | None         # typically only on last chunk
```

### 10.4 Backpressure

Chaque adapter utilise un `anyio` TaskGroup avec queue bornée (64
chunks). Si le client consomme lentement, la tâche de lecture
backend **ne bloque pas** plus de 250 ms sur put ; au-delà, on drop
les chunks les plus anciens et on émet un log de warning. Le client
reçoit l'ensemble complet à la fin (accumulator non-tronqué).

### 10.5 Cancellation

Si le client ferme la connexion :
- FastAPI raise `asyncio.CancelledError` dans le handler.
- Adapter propage `cancel()` à la tâche backend.
- Backend HTTP client envoie un RST ou un abort frame (HTTP/2).
- Le backend distant peut continuer à charger (on est facturés quand
  même côté OpenAI/Anthropic), mais notre token accounting reflète
  les tokens effectivement reçus seulement.

---

## 11. Safety pipeline

### 11.1 Architecture

```
ChatRequest
    │
    ├─► PiiScrubber (preflight)
    │       │
    │       ▼
    │   scrubbed_messages + pii_flags
    │
    ├─► ContentModerator (preflight)
    │       │
    │       ▼
    │   categories: {hate: 0.02, violence: 0.01, sexual: 0.78, ...}
    │
    ├─► PromptInjectionDetector (preflight)
    │       │
    │       ▼
    │   injection_risk_score: 0.0-1.0
    │
    └─► PolicyDecider
            │
            ▼
        Decision: ALLOW | BLOCK | REVIEW
        → if BLOCK: raise SafetyBlocked, audit event, 403
        → if REVIEW: defer to LLM judge (optional v1.1)
        → if ALLOW: pass through
    ... backend call ...
ChatResponse
    │
    ├─► PiiScrubber (postflight)
    ├─► ContentModerator (postflight)
    └─► OutputPolicyFilter
            │
            ▼
        Decision: ALLOW | BLOCK | REDACT
```

### 11.2 PiiScrubber

- **Moteur** : [Presidio](https://github.com/microsoft/presidio) avec
  `AnalyzerEngine` + `AnonymizerEngine`.
- **Catégories default** : EMAIL, PHONE, IBAN, SSN, CREDIT_CARD,
  IP_ADDRESS, URL, PERSON, LOCATION.
- **Config par tenant** : `safety_policies.pii_categories` override.
- **Comportement** : par défaut `REDACT` (`[EMAIL_REDACTED]`), mais on
  peut mode `HASH` (hash déterministe + salt par tenant) ou `BLOCK`
  (refuse la requête).
- **PII en sortie** : on applique aussi sur la réponse. Si un modèle a
  régurgité un email issu d'une fuite d'entraînement, on le scrub.

### 11.3 ContentModerator

Pipeline :
1. **Local keyword blocklist** (bumper rapide, 2 ms, 0 token coût).
2. **OpenAI Moderations API** (backend-agnostic, 200 ms, gratuit).
3. **(Optionnel) Azure AI Content Safety** (pour tenants premium).

Décision :
- Score max sur toute catégorie > `moderation_thresholds[category]` → BLOCK.
- Score > 0.5 sur catégories "hate, violence, sexual" → REVIEW (v1.1).
- Autrement → ALLOW avec flags stockés en audit.

### 11.4 PromptInjectionDetector

**Heuristiques MVP** :
- Match regex sur patterns connus (`"ignore previous instructions"`,
  `"disregard the system prompt"`, `"you are now"`, ...).
- Détection de caractères zero-width (U+200B, U+FEFF).
- Détection de langues mixtes soudaines dans un même message.

**Score** : 0-100 ; `block_on_injection_risk` default 80.

v1.1 ajoutera un LLM judge (Claude Haiku ou GPT-4o-mini) qui reçoit le
message et retourne `{injection: bool, confidence: float}`.

### 11.5 OutputPolicyFilter

Profils par tenant :
- `pg13` : bloque contenu sexuel/violent explicite.
- `professional` : bloque langage informel + émojis dans certaines
  sections.
- `custom` : regex liste fournie par le tenant.

### 11.6 SafetyAuditor

Chaque décision **non-ALLOW** émet :
1. Un `SafetyEvent` en Postgres (hashé, signé HMAC).
2. Un message NATS `efe.inference.safety.flagged` pour M10.
3. Une metric `efe_inference_safety_flagged_total{category, severity}`.

**Invariant I3** (PII en telemetry) : le `payload_hash` est le hash du
texte **déjà scrubbed** (pas le brut). Même un opérateur avec accès
DB ne peut pas reconstruire le message original.

---

## 12. Semantic cache

### 12.1 Pourquoi

Certains prompts récurrents (FAQ, classifications standard,
extractions structurées) arrivent 1000× par jour avec des variations
mineures. Un cache **verbatim** n'aide pas (hash différent), mais un
cache **sémantique** (embedding + cosine) peut renvoyer la même
réponse pour une similarité ≥ 0.95.

Gain typique : **30-50 %** des requêtes "FAQ-like" servies en < 25 ms
au lieu de ~200 ms.

### 12.2 Fonctionnement

1. Calcule `embedding = EmbedModel.encode(canonical_prompt)`
   (e.g. `text-embedding-3-small`, 1536 dim).
2. `RediSearch KNN` : `FT.SEARCH inf:cache:s:{tenant}:{task} "*=>[KNN 1 @embedding_vec $q AS score]"`.
3. Si `score < cosine_threshold` (e.g. 0.05 distance, i.e.
   0.95 similarity), on renvoie la réponse cached.
4. Sinon, backend call + `HSET` du nouvel embedding + réponse.

### 12.3 Partition par `(tenant, task)`

Chaque `(tenant, task)` a son propre index RediSearch. **Aucune
contamination cross-tenant possible.** C'est I5.

### 12.4 Invalidation

Quand M06 publie `efe.registry.routing.updated`, M07 subscribe et
**drop l'index** correspondant :

```
async def _on_routing_update(event):
    tenant = event.payload["tenant_id"]
    task = event.payload["task_id"]
    await redis.execute("FT.DROPINDEX", f"inf:cache:s:{tenant}:{task}",
                         "DD")  # delete docs too
```

Sinon la cache servirait des réponses du vieux modèle après bascule.

### 12.5 TTL + eviction

- TTL default : 3600 s (1 h).
- Eviction : LRU via `redis.io/.../FT.CREATE ... OFFSET`.
- Capacité max : 100k entries par index.

### 12.6 Configurabilité par tenant

```yaml
semantic_cache:
  enabled: true
  embed_model: text-embedding-3-small
  cosine_threshold: 0.95
  ttl_s: 3600
  max_entries_per_task: 100_000
```

Certains tenants (médecine, finance) peuvent vouloir cache désactivé —
c'est un flag.

---

## 13. Résilience

### 13.1 RetryPolicy

```python
@dataclass(frozen=True, slots=True)
class RetryPolicy:
    max_attempts: int = 3
    backoff_base_s: float = 0.5
    backoff_max_s: float = 5.0
    jitter_factor: float = 0.2        # +/- 20%
    retry_on: frozenset[str] = frozenset({
        "connection_error", "timeout", "server_error_5xx",
        "rate_limit_429",
    })
    give_up_on: frozenset[str] = frozenset({
        "auth_error", "bad_request", "content_policy_violation",
    })
```

Backoff : `sleep = min(backoff_max, backoff_base * 2**attempt) * (1 ± jitter)`.

### 13.2 CircuitBreaker 3-states

```
                 ┌──────────┐
                 │  CLOSED  │  (normal operation)
                 └─────┬────┘
                       │ failure_count >= threshold (5 fails / 60s)
                       ▼
                 ┌──────────┐
                 │   OPEN   │  (refuse all calls for cool_down_s)
                 └─────┬────┘
                       │ cool_down_s elapsed
                       ▼
                 ┌───────────┐
                 │ HALF_OPEN │  (allow 1 probe call)
                 └─────┬─────┘
                       │
            ┌──────────┴──────────┐
            │                     │
        fail                      OK
            │                     │
            ▼                     ▼
         OPEN                   CLOSED
      (cool_down reset)      (counts reset)
```

**Config** :
- `failure_threshold` : 5 fails in a 60s window → OPEN.
- `success_threshold` : 1 probe OK → CLOSED (conservative : 3 in high-risk).
- `cool_down_s` : 30s.

**Per-tenant** : chaque `(tenant, backend)` a son circuit indépendant.
Un bug sur le tenant A ne doit pas ouvrir le circuit pour le tenant B.

### 13.3 TimeoutBudget

```python
@dataclass(frozen=True, slots=True)
class TimeoutBudget:
    connect_s: float = 5.0
    first_byte_s: float = 15.0
    inter_token_s: float = 10.0       # streaming
    total_s: float = 60.0
```

Chaque backend hérite d'une copie. Le `total_s` est passé aux HTTP
clients comme deadline absolue.

### 13.4 FallbackChain

Config par `(tenant, task)` dans `inference_fallback_chains` :

```
position=0: vllm-local-7b        (primary, timeout 10s)
position=1: claude-sonnet-3.5    (secondary, timeout 30s)
position=2: peft-safe-7b         (last-resort, timeout 30s)
```

`BackendSelector.select()` traverse la chain :

```python
for entry in chain:
    if circuit.is_open(entry.backend_id, tenant): continue
    try:
        return await retry_loop(entry.backend_id, req)
    except Unrecoverable:
        raise  # don't try next; e.g. auth error
    except Recoverable:
        continue
raise AllBackendsDown
```

### 13.5 Dead-letter queue

Si tous les backends échouent, la requête est poussée sur Redis Stream
`inf:dlq` avec le payload scrubbed. Opérations peut inspecter et
rejouer manuellement. Stream purgé à 7 jours.

---

## 14. Token accounting & cost

### 14.1 Tokenizers officiels

| Famille | Tokenizer | Package |
|---|---|---|
| GPT-4 / GPT-4o | `cl100k_base` / `o200k_base` | tiktoken |
| Claude | Anthropic-specific BPE | anthropic SDK |
| Llama-3 | SentencePiece | sentencepiece |
| Mistral | SentencePiece | sentencepiece |
| Gemini | SentencePiece-like | google-genai |

`TokenCounter.count(text, model_family)` retourne un `int`
**exactement** égal à ce que facture le backend. C'est I6.

### 14.2 Cost table

```sql
-- inference_backends table fields:
cost_per_1k_prompt_usd      DECIMAL(10, 6)
cost_per_1k_completion_usd  DECIMAL(10, 6)
```

Valeurs types MVP :
- Claude Sonnet 3.5 : $3/1M prompt, $15/1M completion.
- GPT-4o : $2.5/1M prompt, $10/1M completion.
- vLLM local (7B A100) : $0 (coût GPU amorti ailleurs).

Coût = `(prompt_tokens / 1000) * cost_per_1k_prompt_usd
       + (completion_tokens / 1000) * cost_per_1k_completion_usd`.

### 14.3 Aggregation et billing

Chaque requête complétée → une ligne `inference_logs` avec `cost_usd`.
Un ETL quotidien :
1. Aggrège par `(tenant_id, day)`.
2. Pousse vers M12 billing pipeline.
3. Reconcilie avec factures backend hosted (Claude, OpenAI) à ± 1 %.

### 14.4 Budget enforcement

Un tenant peut définir un `daily_budget_usd` (dans M11). M07 maintient
un compteur Redis `inf:spend:{tenant}:{day}` ; avant d'émettre une
requête, si le compteur excède le budget → 402 `budget_exceeded`.

### 14.5 Refund sur échec

Si la requête échoue **après** billing (rare), on refund via un
NATS event `efe.inference.cost.refunded` consommé par le billing.

---

## 15. NATS topics

### 15.1 Topics produits par M07

| Topic | Payload | Consommateur | Volume |
|---|---|---|---|
| `efe.inference.completed` | `{request_id, tenant, backend, tokens, cost_usd, latency_ms}` | M12 telemetry | haut |
| `efe.inference.safety.flagged` | `{event_id, request_id, stage, categories}` | M10 | moyen |
| `efe.inference.backend.circuit_opened` | `{tenant, backend_id, opened_at}` | M08 | rare |
| `efe.inference.backend.circuit_closed` | idem | M08 | rare |
| `efe.inference.cache.hit` | `{tenant, task, kind: verbatim|semantic}` | M12 | haut |
| `efe.inference.cost.billed` | `{tenant, request_id, cost_usd}` | billing | haut |
| `efe.inference.cost.refunded` | `{tenant, request_id, cost_usd, reason}` | billing | rare |

### 15.2 Topics consommés par M07

| Topic | Producteur | Action |
|---|---|---|
| `efe.registry.routing.updated` | M06 | drop semantic cache pour `(tenant, task)` |
| `efe.registry.rollback.completed` | M06 | drop circuit breaker hot state + cache |
| `efe.inference.policy.updated` | M10 | reload `safety_policies` |

### 15.3 Format HMAC

Identique à M06 (voir `M06_ARCHITECTURE.md §15.2`) : envelope JSON
avec `schema_version`, `event_type`, `event_id`, `occurred_at`,
`tenant_id`, `payload`, `hmac`. La clé est `signing.nats_hmac_key_hex`.

---

## 16. Interfaces publiques

### 16.1 Python (in-process)

```python
from efe_inference import InferenceAdapter, ChatRequest, ChatMessage

adapter = InferenceAdapter.from_settings(settings)

resp = await adapter.generate(ChatRequest(
    tenant_id="acme",
    task_id="summarize",
    model_hint=None,   # auto-pick from chain
    messages=(
        ChatMessage(role="system", content=(TextBlock("You are concise."),)),
        ChatMessage(role="user",   content=(TextBlock("Hello!"),)),
    ),
    sampling=SamplingParams(temperature=0.2, max_tokens=128),
    stream=False,
))

print(resp.choices[0].message.content)
print(resp.usage)      # Usage(prompt_tokens=12, completion_tokens=8, total=20)
print(resp.backend_kind, resp.backend_latency_ms)
```

### 16.2 gRPC

```proto
service InferenceService {
  rpc Chat(ChatRequestPb) returns (ChatResponsePb);
  rpc StreamChat(ChatRequestPb) returns (stream ChatChunkPb);
  rpc Embed(EmbedRequestPb) returns (EmbedResponsePb);
  rpc Health(HealthRequestPb) returns (HealthResponsePb);
}
```

### 16.3 HTTP REST — triple compat

M07 expose **trois façades** parallèles sur le même cœur :

- `/v1/chat/completions` — OpenAI-compat (défaut, majorité des SDKs).
- `/v1/messages` — Anthropic-compat (pour migrations depuis Claude
  directement).
- `/v1/inference/chat` — Native EFE (canonique, exposé pour SDKs
  internes + UI).

Chaque endpoint normalise vers `ChatRequest`, traverse le pipeline
identique, puis dé-normalise en sortie.

### 16.4 Headers standard

| Header | Direction | Usage |
|---|---|---|
| `X-EFE-Tenant-Id` | in | forcé par M11, sinon 401 |
| `X-EFE-Task-Id` | in | requis sauf `/admin` |
| `X-EFE-Request-Id` | in/out | idempotence ; généré si absent |
| `X-EFE-Model-Hint` | in | override du chain primary |
| `X-EFE-Trace-Id` | out | OTel trace |
| `X-EFE-Backend-Kind` | out | debug ; quel backend a servi |
| `X-EFE-Cache` | out | `miss|verbatim|semantic` |
| `X-EFE-Cost-USD` | out | coût estimé |

### 16.5 SDK

```python
# efe-tool (Python CLI)
efe-tool inference chat --tenant acme --task summarize --prompt "Hello"

# efe-sdk (Python)
from efe_sdk import EfeClient
client = EfeClient(base_url="https://api.efe.io", api_key=...)
response = await client.chat(tenant="acme", task="summarize", messages=[...])
```

Le SDK est générée depuis OpenAPI (Python) + protobuf (Python/TS).

---

## 17. Stratégie de tests

### 17.1 Unit (≥ 300 cas)

```
tests_inference/
  test_request_normalizer.py         (OpenAI/Anthropic/native round-trips)
  test_response_normalizer.py
  test_chat_message_validation.py
  test_prompt_templater_llama3.py
  test_prompt_templater_chatml.py
  test_prompt_templater_claude.py
  test_prompt_templater_gpt.py
  test_prompt_templater_gemini.py
  test_tool_adapter_openai_to_anthropic.py  (round-trip, no data loss)
  test_tool_adapter_anthropic_to_openai.py
  test_tool_validator_repair_json.py
  test_pii_scrubber_email_phone_iban.py
  test_content_moderator_blocklist.py
  test_prompt_injection_detector.py
  test_verbatim_cache_hit_miss.py
  test_semantic_cache_partitioning.py        (strict tenant boundary)
  test_circuit_breaker_fsm.py                (3 states × transitions)
  test_retry_policy_backoff.py
  test_fallback_chain_deterministic.py
  test_timeout_budget_propagation.py
  test_token_counter_cl100k.py
  test_token_counter_claude.py
  test_token_counter_sentencepiece.py
  test_cost_calculator_exact.py
  test_budget_enforcer_blocks_at_cap.py
  test_stream_multiplexer_claude_to_sse.py
  test_stream_multiplexer_openai_to_sse.py
  test_safety_auditor_hmac_signed.py
  test_model_hint_override.py
  test_endpoint_resolution_cache_1s.py
```

### 17.2 Integration (≥ 60 cas)

```
tests_inference_integration/
  test_e2e_chat_via_vllm_local.py            (kind cluster + vLLM in docker)
  test_e2e_chat_via_claude_mock.py            (respx mock of Anthropic API)
  test_e2e_chat_via_openai_mock.py
  test_e2e_streaming_sse_end_to_end.py
  test_e2e_tool_call_openai_roundtrip.py
  test_e2e_tool_call_anthropic_roundtrip.py
  test_e2e_vision_multimodal_image_in.py
  test_e2e_fallback_primary_down.py           (primary circuit open, secondary serves)
  test_e2e_fallback_all_down_returns_503.py
  test_e2e_safety_pii_scrubbed_in_and_out.py
  test_e2e_safety_block_returns_403.py
  test_e2e_cache_invalidated_on_promote.py    (subscribe M06 NATS event)
  test_e2e_budget_blocks_at_daily_cap.py
  test_e2e_cost_logged_and_billed.py
```

### 17.3 Contract tests par backend

Chaque backend a un jeu de fixtures `(ChatRequest, expected_backend_payload)`
qui vérifient que notre adapter produit **exactement** le format
attendu. Maintenu via `pytest-snapshot`.

### 17.4 Chaos tests

```
tests_inference_chaos/
  test_chaos_backend_timeout_triggers_circuit.py
  test_chaos_partial_stream_cancelled.py
  test_chaos_redis_down_cache_bypass.py
  test_chaos_clock_skew_token_accounting.py
  test_chaos_network_partition_fallback.py
  test_chaos_huge_message_memory_bound.py
  test_chaos_concurrent_safety_block.py
  test_chaos_llm_hallucinated_tool_repair.py
```

### 17.5 Couverture cible

- `safety/` : **98 %** (critique).
- `cache/` : **95 %**.
- `resilience/` : **95 %**.
- `backends/` : **85 %** (mocks externes compliquent).
- `templating/` : **90 %**.
- Global : **≥ 90 %**.

---

## 18. Observabilité

### 18.1 Metrics Prometheus

```
# ---- Throughput ----
efe_inference_requests_total{tenant, task, backend, outcome="ok|err|safety"}
efe_inference_chat_latency_seconds{tenant, task, backend}  (histogram)
efe_inference_ttft_seconds{tenant, task, backend}           (histogram)
efe_inference_inter_token_latency_seconds{tenant, backend}  (histogram)
efe_inference_adapter_overhead_seconds{tenant, task}        (histogram; SLO <15ms p99)

# ---- Tokens & cost ----
efe_inference_prompt_tokens_total{tenant, task, backend}
efe_inference_completion_tokens_total{tenant, task, backend}
efe_inference_cost_usd_total{tenant, task, backend}
efe_inference_budget_rejections_total{tenant}

# ---- Cache ----
efe_inference_cache_hits_total{tenant, task, kind="verbatim|semantic"}
efe_inference_cache_misses_total{tenant, task}
efe_inference_cache_put_latency_seconds{kind}               (histogram)
efe_inference_cache_evictions_total{tenant, task}

# ---- Safety ----
efe_inference_safety_scrub_total{tenant, category}
efe_inference_safety_moderation_flagged_total{tenant, category}
efe_inference_safety_injection_flagged_total{tenant}
efe_inference_safety_block_total{tenant, stage}

# ---- Resilience ----
efe_inference_retry_total{tenant, backend, reason}
efe_inference_circuit_state{tenant, backend}       (gauge 0=closed 1=half 2=open)
efe_inference_fallback_used_total{tenant, from_backend, to_backend}
efe_inference_backend_health{backend}              (gauge 0|1)

# ---- Errors ----
efe_inference_backend_errors_total{tenant, backend, code}
efe_inference_tool_repair_total{tenant, backend, success}
efe_inference_timeout_total{tenant, backend, phase}
efe_inference_dlq_entries_total{tenant}
```

### 18.2 Traces OpenTelemetry

Span racine : `inference.chat`. Enfants :

- `inference.normalize.request`
- `inference.safety.preflight`
- `inference.cache.lookup.{verbatim,semantic}`
- `inference.backend.select`
- `inference.template.render`
- `inference.backend.call.{backend_kind}`
  - `http.request` (auto-instrumenté via httpx)
- `inference.response.normalize`
- `inference.tool.adapt`
- `inference.safety.postflight`
- `inference.cache.put`
- `inference.billing`

Sampling : 5 % global, 100 % sur erreurs + safety blocks + latency
outliers (> SLO).

**I3** (PII in telemetry) : seuls les hashes + flags sont attachés en
attributs, jamais le texte brut.

### 18.3 Logs structurés

Format JSON, champs :

```
{
  "ts": "...",
  "level": "info",
  "event": "inference_completed",
  "request_id": "...",
  "tenant_id": "...",
  "task_id": "...",
  "backend": "claude-sonnet-3.5",
  "prompt_hash": "sha256:...",
  "response_hash": "sha256:...",
  "prompt_tokens": 132,
  "completion_tokens": 48,
  "cost_usd": "0.00123",
  "cache": "miss",
  "safety_flags": {},
  "total_latency_ms": 812,
  "ttft_ms": 540,
  "adapter_overhead_ms": 12,
  "trace_id": "...",
  "span_id": "..."
}
```

Redaction : un middleware log-processor rejette toute entrée
contenant des clés `prompt_text`, `response_text`, `messages` — défense
en profondeur contre les bugs.

### 18.4 Alertes Alertmanager

```yaml
groups:
- name: efe.inference.slo
  rules:
  - alert: TtftP95OverBudget
    expr: histogram_quantile(0.95, rate(efe_inference_ttft_seconds_bucket{backend=~"vllm.*"}[5m])) > 0.35
    for: 3m
    labels: {severity: warning, module: M07}
    annotations: { summary: "Local TTFT p95 > 350ms" }

  - alert: AdapterOverheadP99OverBudget
    expr: histogram_quantile(0.99, rate(efe_inference_adapter_overhead_seconds_bucket[5m])) > 0.015
    for: 5m
    labels: {severity: warning, module: M07}

  - alert: SafetyBlockRateSpike
    expr: rate(efe_inference_safety_block_total[5m]) > 0.1 * rate(efe_inference_requests_total[5m])
    for: 10m
    labels: {severity: warning, module: M07}

  - alert: CircuitOpenForPrimary
    expr: efe_inference_circuit_state{backend=~"vllm-local-.*"} == 2
    for: 1m
    labels: {severity: critical, module: M07}

  - alert: BudgetRejectionsSpike
    expr: rate(efe_inference_budget_rejections_total[5m]) > 1
    for: 5m
    labels: {severity: info, module: M07}
```

### 18.5 Dashboards Grafana

**Panels** :
1. RPS total + par backend (stacked area).
2. TTFT p50/p95/p99 par backend.
3. Cost $/h par tenant.
4. Cache hit rate par `(tenant, task)`.
5. Safety block rate par stage.
6. Circuit states (grid).
7. Fallback flow Sankey (primary → secondary → last-resort).
8. Token throughput (input vs output) par backend.
9. Latency breakdown (normalize, safety, cache, backend, post).
10. Top 10 most expensive tenants (last 24h).
11. DLQ entries (should stay at 0).
12. Backend health (timeline).

---

## 19. Plan de sprint M07

### 19.1 Lots (5 lots × 2 devs × 10 jours)

| Lot | Durée | Contenu | DRI |
|---|---|---|---|
| **L1 — Request/Response core** | 2 j | `ChatRequest`, `ChatMessage`, `ChatResponse`, normalizers, validators, Pydantic schemas | Dev A |
| **L2 — Backends** | 3 j | `BackendAdapter` protocol, `VllmBackend`, `ClaudeBackend`, `OpenAiBackend`, `MistralBackend`, `PeftBackend`, `BackendRegistry` | Dev B |
| **L3 — Templating + Tools** | 2 j | Jinja2 templates (5 familles), `PromptTemplater`, `ToolAdapter`, `ToolValidator` + repair | Dev A |
| **L4 — Safety + Cache** | 2 j | `PiiScrubber`, `ContentModerator`, `PromptInjectionDetector`, `VerbatimCache`, `SemanticCache` + RediSearch | Dev B |
| **L5 — Resilience + API** | 1 j | `CircuitBreaker`, `RetryPolicy`, `FallbackChain`, SSE + gRPC streaming, FastAPI app, CLI | Dev A+B |

### 19.2 Dependencies critiques

- M06 `RouteDecision` + NATS `routing.updated` doivent être stables
  avant L2.
- M11 doit passer `X-EFE-Tenant-Id` + auth vérifiée avant L5 expose
  publiquement.
- M10 peut démarrer en parallèle si M07 L4 expose un contract NATS
  `safety.flagged` dès le jour 1.

### 19.3 Definition of Done

- [ ] Tous les tests unitaires verts.
- [ ] E2E avec kind + mock Anthropic/OpenAI verts.
- [ ] Contract tests snapshot à jour.
- [ ] Couverture ≥ 90 % globale.
- [ ] mypy --strict vert sur `src/efe_inference/`.
- [ ] ruff check zéro erreur.
- [ ] SLO TTFT local ≤ 350 ms mesuré sur staging.
- [ ] SLO adapter overhead ≤ 15 ms p99 mesuré sur staging.
- [ ] Dashboard Grafana provisionné.
- [ ] 5 alertes Alertmanager en place et testées.
- [ ] Runbooks : `runbook-safety-spike.md`, `runbook-circuit-open.md`,
  `runbook-budget-rejection.md`, `runbook-dlq-investigation.md`.

---

## 20. Critères d'acceptation

### 20.1 Fonctionnel

1. Une requête OpenAI-format reçue sur `/v1/chat/completions` est
   servie uniformément que le backend soit vLLM local, Claude, GPT,
   Mistral ou PEFT local — sans modification client.
2. Un outil OpenAI-format traverse Claude et revient en OpenAI-format
   correct (round-trip I2).
3. Une requête avec PII (email + IBAN) est scrubée avant d'atteindre
   le backend ; les flags sont dans `safety_events`.
4. Un prompt "ignore previous instructions" est détecté comme
   injection risk > 80 et bloqué (403 + audit).
5. Une requête en double exacte renvoie une hit de cache verbatim en
   < 25 ms.
6. Une requête similaire à 95 % (cosine) renvoie une hit de cache
   semantic.
7. Quand le primary backend tombe (circuit OPEN), la requête est
   servie par le secondary sans erreur client.
8. Quand tous les backends sont OPEN, la requête renvoie 503
   `all_backends_down` avec header `Retry-After`.
9. Le token accounting correspond exactement au tokenizer officiel (0
   écart).
10. Le cost logged correspond à ± 1 % de la facture backend réelle
    (réconciliation quotidienne).

### 20.2 Non-fonctionnel

1. TTFT p95 vLLM local < 350 ms.
2. TTFT p95 Claude hosted < 900 ms.
3. Adapter overhead p99 < 15 ms (hors backend).
4. Cache hit p95 < 25 ms.
5. Disponibilité bout-à-bout ≥ 99.95 %/mois avec fallback.

### 20.3 Sécurité & conformité

1. **Aucune** PII en logs / metrics / traces / audit (test dédié qui
   inspecte 10 000 entrées).
2. Toute réponse est modérée avant retour client (test couverture
   100 %).
3. Audit trail safety est signé HMAC et vérifiable bout-à-bout.
4. API keys des backends hosted sont en KMS/Vault, rotation trimestrielle.
5. Partition tenant stricte dans cache (test cross-tenant leak).

### 20.4 Qualité d'ingénierie

1. ≥ 300 tests unitaires + 60 intégration + 8 chaos.
2. Couverture ≥ 90 % globale, ≥ 98 % sur `safety/`.
3. mypy --strict + ruff check verts.
4. OpenAPI 3.1 complet pour les 3 façades (OpenAI, Anthropic, native).
5. Contract tests par backend avec snapshots versionnés.

---

## 21. Risques et mitigations

| Risque | Impact | Probabilité | Mitigation |
|---|---|---|---|
| API backend hosted change format sans préavis | rupture prod | Moyen | Contract tests + snapshot alert ; versioning de headers API |
| Tokenizer OpenAI change (o200k → o300k) | count incorrect | Moyen | Pin version tiktoken ; alert sur écart facture ± 2 % |
| Semantic cache empoisonné (bad response cached) | mauvais service répété | Faible | TTL 1h + invalidation sur promote ; content moderation pré-cache |
| PII scrubber faux négatif | fuite compliance | Faible | Presidio + regex custom + audit + tests red-team |
| Moderation API down | blocage ou bypass | Moyen | Fallback blocklist locale ; circuit breaker sur modération elle-même |
| Cost runaway (bug infinite retry) | facture explose | Moyen | Budget enforcer hard cap + alert à 80 % quota |
| Circuit breaker trop agressif | fausse panne | Moyen | Seuils conservateurs (5 fails/60s) ; tests chaos |
| Prompt injection passe le détecteur | exfiltration | Haute | Défense en profondeur : detector + output filter + system prompt hardening |
| Embedding model drift (cache | hit rate baisse) | Faible | Re-embed sur changement ; alert hit rate < 20 % |
| Session sticky tool_call sur fallback | incohérence outils | Faible | Tool state stored côté client (stateless M07) ; session_id propagé |
| Concurrent semantic cache writes race | doublons Redis | Faible | RedisLock par `(tenant, task, embedding_hash_prefix)` |
| Audit chain M05 unavailable | safety events perdus | Faible | Buffer local + replay ; métrique `audit_backlog` |

Runbooks (liste minimale) :

- `runbook-backend-down.md`
- `runbook-safety-block-spike.md`
- `runbook-cost-spike.md`
- `runbook-cache-poisoning.md`
- `runbook-tokenizer-drift.md`
- `runbook-dlq-replay.md`

---

## 22. Annexes

### A. Helm values (extrait)

```yaml
inference:
  replicaCount: 6
  image:
    repository: efe-registry.io/core/inference
    tag: "1.0.0"
  resources:
    requests: { cpu: 500m, memory: 1Gi }
    limits:   { cpu: 4,    memory: 8Gi }

  backends:
    - name: vllm-local-7b
      kind: vllm_local
      priority: 100
    - name: claude-sonnet-3.5
      kind: anthropic
      apiKeySecret: efe-claude-key
      endpoint: https://api.anthropic.com
      priority: 200
    - name: gpt-4o
      kind: openai
      apiKeySecret: efe-openai-key
      priority: 210
    - name: mistral-large
      kind: mistral
      apiKeySecret: efe-mistral-key
      endpoint: https://api.mistral.ai
      priority: 220

  safety:
    pii:
      engine: presidio
      categories: [email, phone, iban, ssn, credit_card, ip]
    moderation:
      providers: [openai_moderations, local_blocklist]
    injection:
      threshold: 80

  cache:
    verbatim:
      ttl_s: 3600
    semantic:
      enabled: true
      embed_model: text-embedding-3-small
      cosine_threshold: 0.95
      ttl_s: 3600
      max_entries_per_task: 100000

  resilience:
    retry:
      max_attempts: 3
      backoff_base_s: 0.5
    circuit:
      failure_threshold: 5
      window_s: 60
      cool_down_s: 30
    timeout:
      connect_s: 5
      first_byte_s: 15
      inter_token_s: 10
      total_s: 60
```

### B. Exemple normalisation OpenAI → canonique

```python
# Input (OpenAI)
raw = {
    "model": "gpt-4o",
    "messages": [
        {"role": "system", "content": "Be concise."},
        {"role": "user",   "content": "What is 2+2?"},
    ],
    "temperature": 0.2,
    "max_tokens": 64,
}

# After RequestNormalizer.normalize:
req = ChatRequest(
    tenant_id="acme",
    task_id="qa",
    model_hint="gpt-4o",
    messages=(
        ChatMessage(role="system",
                     content=(TextBlock("Be concise."),)),
        ChatMessage(role="user",
                     content=(TextBlock("What is 2+2?"),)),
    ),
    sampling=SamplingParams(temperature=0.2, max_tokens=64),
    tools=(),
    stream=False,
    request_id="018e-xxx-yyyy",
    timeout_budget_s=30.0,
)
```

### C. Exemple tool call Anthropic → canonique

```python
# Backend (Anthropic) returned:
claude_json = {
    "role": "assistant",
    "content": [
        {"type": "text", "text": "Let me check the weather."},
        {"type": "tool_use", "id": "tool_01", "name": "get_weather",
         "input": {"city": "Paris"}},
    ],
    "stop_reason": "tool_use",
}

# After ResponseNormalizer.normalize + ToolAdapter.normalize:
resp = ChatResponse(
    request_id="018e-xxx",
    model="claude-sonnet-3.5",
    choices=(
        ChatChoice(
            index=0,
            message=ChatMessage(
                role="assistant",
                content=(
                    TextBlock("Let me check the weather."),
                    ToolUseBlock(
                        tool_call_id="tool_01",
                        tool_name="get_weather",
                        tool_args={"city": "Paris"},
                    ),
                ),
            ),
            finish_reason="tool_calls",
        ),
    ),
    usage=Usage(prompt_tokens=..., completion_tokens=..., total_tokens=...),
    backend_kind="anthropic",
    backend_latency_ms=...,
    adapter_overhead_ms=...,
    safety_flags={},
    cached="no",
)
```

### D. Circuit breaker FSM (Lua pour Redis)

```lua
-- circuit_breaker.lua
-- KEYS[1] = "inf:circuit:{tenant}:{backend}"
-- ARGV[1] = now_ms
-- ARGV[2] = "success" | "failure"
-- ARGV[3] = failure_threshold
-- ARGV[4] = success_threshold
-- ARGV[5] = cool_down_ms

local state = redis.call('HGET', KEYS[1], 'state') or 'closed'
local fails = tonumber(redis.call('HGET', KEYS[1], 'failures') or '0')
local succs = tonumber(redis.call('HGET', KEYS[1], 'successes') or '0')
local opened_at = tonumber(redis.call('HGET', KEYS[1], 'opened_at') or '0')

if ARGV[2] == 'success' then
    if state == 'half_open' then
        succs = succs + 1
        if succs >= tonumber(ARGV[4]) then
            state = 'closed'; fails = 0; succs = 0
        end
    elseif state == 'closed' then
        fails = 0
    end
elseif ARGV[2] == 'failure' then
    if state == 'closed' then
        fails = fails + 1
        if fails >= tonumber(ARGV[3]) then
            state = 'open'
            opened_at = tonumber(ARGV[1])
        end
    elseif state == 'half_open' then
        state = 'open'
        opened_at = tonumber(ARGV[1])
        succs = 0
    end
end

if state == 'open' then
    if tonumber(ARGV[1]) - opened_at >= tonumber(ARGV[5]) then
        state = 'half_open'
    end
end

redis.call('HMSET', KEYS[1],
    'state', state, 'failures', fails, 'successes', succs,
    'opened_at', opened_at,
    'last_transition_ms', ARGV[1])
redis.call('EXPIRE', KEYS[1], 3600)
return state
```

### E. Glossaire M07

| Terme | Définition |
|---|---|
| **Backend** | Moteur d'inférence concret (vLLM local, Claude, GPT...). |
| **Adapter** | Wrapper Python qui traduit notre format canonique vers celui d'un backend spécifique. |
| **ChatRequest** | Forme canonique d'une requête d'inférence, backend-agnostic. |
| **ChatResponse** | Forme canonique d'une réponse d'inférence. |
| **PromptTemplater** | Rend un `ChatRequest` en chaîne formatée selon `model_family`. |
| **ToolAdapter** | Traduit tool_call entre OpenAI et Anthropic formats. |
| **CircuitBreaker** | FSM 3-états qui coupe le trafic vers un backend défaillant. |
| **FallbackChain** | Liste ordonnée de backends à essayer en cascade. |
| **SemanticCache** | Cache basé sur embeddings + cosine similarity (RediSearch). |
| **VerbatimCache** | Cache basé sur hash exact du prompt canonique. |
| **PiiScrubber** | Supprime ou remplace les PII avant logging / backend call. |
| **ContentModerator** | Pipeline de modération (OpenAI Moderations + blocklist locale). |
| **TokenCounter** | Comptage exact via tokenizer officiel du modèle. |
| **TimeoutBudget** | Ensemble hiérarchique de deadlines (connect, first-byte, inter-token, total). |
| **BudgetEnforcer** | Bloque les requêtes si le tenant a dépassé son budget $/jour. |
| **StreamMultiplexer** | Normalise les chunks de tous les backends en un format SSE/gRPC uniforme. |
| **SafetyAuditor** | Émet et signe les événements safety vers M10. |
| **ModelFamily** | Classe de modèles partageant un même tokenizer + chat template (claude, gpt, llama3, mistral, gemini). |

---

**Fin du document M07.**
*Version 1.0 — spec gelée pour implémentation — dernière revue 2026-04-20.*
*Toute évolution de contrat (backend signatures, ChatRequest, safety policy)
passe par une ADR référençant ce document.*
