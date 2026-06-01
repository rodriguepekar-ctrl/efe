# Sprint M20 — BYO-LLM Agent SDK + LLM-helper mcp-server tools : architecture

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M19 GA (Production hardening livré : observability + cosign + BYOK + HTTP+SSE multi-replica).
>   M19 a fourni les fondations — observabilité, audit chain HMAC,
>   Redis multi-replica — sans lesquelles M20 (premier pas LLM) serait
>   irresponsable.
> **Default state** : **OFF par défaut** (cf. § 15 + roadmap § F.7).
>   Aucun tenant existant ne voit de changement après upgrade vers
>   EFE 1.4-pre tant qu'aucun flag M20 n'est activé.
>
> **Pré-requis** : avoir lu :
> - [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md)
>   (M19, blueprint complet ; 522 pin tests GREEN).
> - [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md)
>   (roadmap stratégique ; § 6 = high-level M20 scope ; § F = feature
>   flag specification normative).
> - `mcp-server/` (M16-M19 livré, **522 pin tests GREEN**, 34 outils,
>   profiles `core`/`advanced`/`agent-native`/`agent-native-deploy`
>   /`experimental`).
> - **MCP sampling spec** (M16-M17 carry-over) :
>   <https://modelcontextprotocol.io/specification#sampling>
> - **Anthropic Python SDK** : <https://github.com/anthropics/anthropic-sdk-python>
> - **OpenAI Python SDK** : <https://github.com/openai/openai-python>
> - **AWS boto3 Bedrock Runtime** :
>   <https://boto3.amazonaws.com/v1/documentation/api/latest/reference/services/bedrock-runtime.html>
> - **Google Vertex AI SDK** :
>   <https://cloud.google.com/python/docs/reference/aiplatform/latest>
> - **vLLM OpenAI-compatible API** :
>   <https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html>
> - **llama.cpp HTTP server** :
>   <https://github.com/ggml-org/llama.cpp/tree/master/tools/server>
>
> **Mission M20** : faire le **premier pas LLM** d'EFE en restant
> **read-only et sans backend changes**. Concrètement :
>
> 1. **Librairie Python `efe-agent`** (NEW package
>    `clients/python/efe_agent/`) : abstraction provider multi-vendor,
>    boucle agent réutilisable, BYO-LLM (le client choisit son
>    provider, paye ses propres tokens).
> 2. **5 nouveaux mcp-server tools** opérateur-facing
>    (LLM-helper category, profile `agent-native`) :
>    `efe_explain_classification`, `efe_compare_bundles_quality`,
>    `efe_suggest_test_sentences`, `efe_analyze_confusion_matrix`,
>    `efe_draft_runbook`. Tous **read-only** (no `is_write=True`).
>    Tous utilisent **MCP sampling delegation** par défaut (le LLM
>    call se fait côté client, EFE ne voit jamais les tokens).
> 3. **Aucun changement backend Rust**. Aucun bump EFE 1.x contract
>    (1.3 reste GA). Aucun nouveau service à déployer.
> 4. **Opt-in everywhere**, default OFF (cf. roadmap § F).
>
> **Pourquoi M20 d'abord** : M20 est le sprint le plus **conservateur**
> de la roadmap M20-M25. Il prouve que le pattern "EFE + LLM" fonctionne
> sans engager Mode-2 (M21, backend-heavy) ni cost governance (M21).
> Il prépare l'écosystème (provider SDK choices, prompt patterns,
> validation discipline) pour les sprints lourds qui suivent.
>
> **Volumétrie estimée** :
> - Python `efe-agent` lib : **~2 200 LOC** + ~800 LOC tests.
> - mcp-server M20 tools : **~600 LOC** + ~400 LOC tests.
> - Documentation : ~1 500 LOC (operator guide + examples).
> - **Aucune ligne Rust**.
> - **Pin tests cumulés post-M20** : **~557** (vs 522 post-M19, +35).
>
> **Backward compat 100 %** :
>   - Toute la surface M16-M19 (34 tools, 11 resources, 10 prompts)
>     reste inchangée fonctionnellement.
>   - 5 nouveaux tools registrés **conditionnellement** sur
>     `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=true`. Sans flag, pas
>     enregistrés, pas listés, RPC invocation retourne `tool_disabled`.
>   - Image Docker mcp-server : **n'inclut pas** `efe-agent` Python
>     lib par défaut (`EFE_FEATURE_M20_AGENT_SDK_BUNDLED=false`). Les
>     opérateurs doivent `pip install efe-agent` séparément.
>
> **Compliance posture EFE 1.3 GA toujours préservée** : M20 ne mute
> aucune attestation. SOC 2 / ISO 27001 / GDPR Art. 32 / NIS2 / EU
> CRA inchangés. **EU AI Act 2024/1689 : N/A à M20** car aucun LLM
> dans le critical path EFE (sampling delegation = LLM côté client,
> tenant choisit son provider et assume sa propre conformité).
> DPIA addendum minimal pour M20 (cf. roadmap § E.1).

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Vision & rationale](#2-vision--rationale)
3. [Scope & non-goals](#3-scope--non-goals)
4. [Catalogue M20 — vue d'ensemble](#4-catalogue-m20--vue-densemble)
5. [Architecture overview](#5-architecture-overview)
6. [La librairie Python `efe-agent`](#6-la-librairie-python-efe-agent)
7. [Provider implementations (8)](#7-provider-implementations-8)
8. [Tool 1 — `efe_explain_classification`](#8-tool-1--efe_explain_classification)
9. [Tool 2 — `efe_compare_bundles_quality`](#9-tool-2--efe_compare_bundles_quality)
10. [Tool 3 — `efe_suggest_test_sentences`](#10-tool-3--efe_suggest_test_sentences)
11. [Tool 4 — `efe_analyze_confusion_matrix`](#11-tool-4--efe_analyze_confusion_matrix)
12. [Tool 5 — `efe_draft_runbook`](#12-tool-5--efe_draft_runbook)
13. [Sampling delegation deep-dive](#13-sampling-delegation-deep-dive)
14. [Direct provider mode (`efe-agent` lib path)](#14-direct-provider-mode)
15. [Activation flags M20](#15-activation-flags-m20)
16. [Threat model M20 (delta vs M19)](#16-threat-model-m20)
17. [Performance & SLOs M20](#17-performance--slos-m20)
18. [Compatibility matrix](#18-compatibility-matrix)
19. [Migration M19 → M20 (no-action default)](#19-migration-m19--m20)
20. [Test plan (~35 nouveaux pin tests)](#20-test-plan)
21. [Deployment scenarios](#21-deployment-scenarios)
22. [Open questions](#22-open-questions)
23. [FAQ](#23-faq)
24. [Glossary M20](#24-glossary-m20)
25. [References](#25-references)

**Annexes**

- [A — Full JSON Schemas (5 tools)](#a--full-json-schemas)
- [B — Provider integration details (8 providers)](#b--provider-integration-details)
- [C — Agent loop sequence diagrams](#c--agent-loop-sequence-diagrams)
- [D — Pin tests inventory M20](#d--pin-tests-inventory-m20)
- [E — Operator workflow examples (5)](#e--operator-workflow-examples)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Premier pas LLM d'EFE — librairie Python BYO-LLM `efe-agent` + 5 mcp-server tools opérateur-facing, **sans backend changes**. |
| **Durée** | 1 sprint (D55-D58), ~30 PM-W ≈ 2 mois calendaires équipe pic 2 ETP. |
| **Ressources** | 1 senior Python (lead) + 0.5 senior Python (provider intégrations) + 0.25 SRE (CI matrix) + 0.25 doc + 0.1 sécurité (review prompt-injection). |
| **Budget** | ~$80 k incluant audit pass post-livraison. |
| **Volumétrie code** | ~2 200 LOC `efe-agent` + ~600 LOC mcp-server tools + ~400 LOC tests + ~800 LOC tests `efe-agent` = **~4 000 LOC total**. **Aucune ligne Rust**. |
| **Pin tests** | **+35 GREEN** (vs 522 M19 → ~557 cumulés mcp-server). |
| **Backward compat** | 100 %. **Default OFF** (4-niveaux : helm / server / tenant / per-tool args). Sans flag activé, mcp-server M20 = mcp-server M19 GA bit-for-bit. |
| **Risk profile** | **Low-medium** — pas de backend changes, pas de service nouveau, pas de coût LLM EFE-side (sampling delegation). Risques principaux : (1) prompt-injection contre les sampling outputs, (2) qualité variable selon provider client, (3) sundry pip dependency creep. Mitigation : output validation strict (Pydantic), per-tool prompt templates versionnés, provider SDKs en `extras_require`. |
| **Compliance impact** | **Zéro changement** SOC 2 / ISO 27001 / GDPR Art. 32 / NIS2 / EU CRA. **EU AI Act 2024/1689 : N/A** (aucun LLM dans critical path EFE). DPIA addendum minimal documentant le pattern sampling delegation. |
| **EFE contract bump** | **Aucun**. EFE 1.3 backend reste GA. mcp-server M20 features 100 % opt-in. |
| **Strategic outcome** | M20 valide les patterns (provider abstraction, prompt validation, sampling delegation) qui seront re-utilisés massivement en M21+. C'est le sprint "low-risk apprentissage". |

---

## 2. Vision & rationale

### 2.1 Le saut "client externe" → "agent-native helper"

M16-M19 ont livré un mcp-server **passif** : il expose des tools, le
client (LLM externe) raisonne, EFE répond. **Aucun helper côté EFE
qui aide l'opérateur à faire des choses LLM-augmented** — le client
doit tout faire.

M20 introduit le concept de **« helper tool »** : un tool MCP qui :
- N'est **pas** un appel direct au backend EFE (pas un classify, pas
  un bundle action).
- Sert **l'opérateur** (curation, debug, eval) plutôt que le runtime
  (classify multi-tenant).
- Utilise un **prompt templaté** que l'EFE controle.
- Emet une **sampling request** vers le client MCP (qui exécute le
  LLM) et **valide** la réponse.

L'opérateur se voit présenter dans son client MCP (Claude Desktop /
Cursor / agent custom) une catégorie de **5 nouveaux tools** qui
augmentent ses workflows quotidiens. Le LLM tourne **côté client**
(c'est un raccourci pour ne pas faire l'opérateur réécrire le prompt
à chaque fois) ; EFE n'a aucun coût LLM.

### 2.2 Pourquoi Python lib + mcp-server tools (pas backend)

**Décision cardinale M20** : aucun backend Rust touché. Trois raisons :

1. **Risk minimization** : un changement backend = bump contract +
   migration tenants + risque regression Mode-1 (p99 200 ms). M20
   n'a pas besoin de toucher au backend.
2. **Vélocité** : Python lib + mcp-server tools = 2 mois. Backend
   changes pour Mode-2 = 6 mois (M21). Découpage rationnel.
3. **Apprentissage** : M20 valide les patterns (provider abstraction,
   prompt versionning, validation Pydantic) avec un blast radius
   minimal avant le big-bang Mode-2.

**Conséquence** : `efe-agent` Python lib **N'EST PAS** déployée dans
le mcp-server par défaut. Image Docker mcp-server reste inchangée
en taille. Opérateur fait `pip install efe-agent` séparément sur
sa machine ou son pod CI.

### 2.3 Pourquoi sampling delegation préférée

**Sampling delegation** est le pattern MCP standard où l'opérateur
client-side LLM exécute le call. Pourquoi c'est le default M20 :

| Aspect | Sampling delegation | Direct provider (lib `efe-agent`) |
|---|---|---|
| Coût LLM | Payé par client | Payé par client (BYO) |
| Visibilité EFE | Aucune (privacy +) | Aucune (privacy +) |
| Audit chain | Tool call audité, sampling result audité | Pareil |
| Choix de modèle | Décidé par client | Configurable côté lib |
| Compliance | Tenant assume | Tenant assume |
| Fonctionnalité hors-MCP | N/A | Oui (CLI, batch, CI) |

Le **sampling delegation** rend M20 zéro-coût pour EFE. La **direct
provider** ouvre les use-cases CI / batch sans MCP client. **Les deux
modes co-existent** dans M20 (cf. § 14).

### 2.4 4 patterns d'usage observés (sources : 5 design partners post-M19)

**Pattern A — « Pourquoi EFE a classifié ce ticket comme X ? »** :
opérateur reçoit une classify, veut comprendre la logique avant de
contredire ou valider. → `efe_explain_classification`.

**Pattern B — « Mon nouveau bundle est-il meilleur que l'ancien ? »** :
avant promotion, opérateur veut un diff qualité (pas juste accuracy
metric agrégée mais cas qui divergent). → `efe_compare_bundles_quality`.

**Pattern C — « Je manque de tests sur cette classe » :** opérateur
voit que sa classe `payment_dispute` n'a que 8 sentences gold ;
il veut booster à 64 (limite D12). → `efe_suggest_test_sentences`.

**Pattern D — « Quelles classes confondons-nous ? »** : opérateur a
sa confusion matrix, veut une analyse sémantique des paires
ambiguës (split / merge / reword). → `efe_analyze_confusion_matrix`.

**Pattern E — « Cette alerte SLO M19 vient de péter, je n'ai pas de
runbook »** : SRE on-call reçoit alert `M19-OBS-3` (log volume), pas
de runbook pré-écrit. → `efe_draft_runbook`.

Les 5 patterns donnent les 5 tools livrés en M20. **Pas plus, pas
moins** — chaque tool doit prouver son ROI sur 1 design partner
pilote (cf. § 22.4).

### 2.5 Pourquoi 5 tools (pas plus, pas moins)

**Raison du nombre** :
- < 5 : les 5 patterns observés ci-dessus sont **distincts** ;
  combiner 2 dans 1 tool donnerait un schema trop large et ambigu.
- > 5 : la fenêtre M20 est 30 PM-W. Chaque tool = ~3 PM-W (handler +
  schema + prompt template + 7 pin tests + doc). 5 × 3 = 15 PM-W ;
  les 15 restants vont à la lib `efe-agent` + provider implementations.

**Pas de tool « efe_propose_taxonomy_split »** dans M20 : ce pattern
existe (D5 design partner), mais il **modifie** la taxonomie via
écriture cluster — c'est M23 (curation, write-pattern). M20 reste
strictement read-only.

### 2.6 Anti-pattern : pourquoi M20 ne fait PAS de "agent runtime"

Tentation : faire tourner l'agent loop **dans** mcp-server. Rejeté car :
- Augmente blast radius (mcp-server crash si LLM call hang).
- Coût LLM EFE-side (vs BYO client-side).
- Multi-tenant isolation devient complexe.
- Déjà couvert par M25 (agent runtime expérimental).

**M20 = stateless tools + Python lib**. Pas d'agent loop in-process.

---

## 3. Scope & non-goals

### 3.1 In scope (livraison M20)

**Python `efe-agent` library** :
- Package `clients/python/efe_agent/`.
- `LLMProvider` Protocol (Python `typing.Protocol`) avec 4 methods :
  `complete`, `stream`, `health`, `cost_model`.
- 8 provider implémentations en `extras_require` (chaque provider
  optionnel, `pip install efe-agent[anthropic]`, etc.).
- `Agent` class (boucle sampling → tool → sampling, max iterations
  cap, audit propagation).
- `FallbackChain` (multi-provider failover avec circuit breaker
  per-provider).
- `PromptTemplate` parser (header YAML + body Jinja2 minimal).
- `examples/` directory (3 examples runnable : batch_classify,
  operator_curation, incident_response).
- CLI minimal `efe-agent run --prompt ... --provider ...`.

**5 nouveaux mcp-server tools** dans `mcp-server/mcp_server/tools/m20_*.py` :
- `efe_explain_classification` — explain a classify result.
- `efe_compare_bundles_quality` — diff qualité 2 bundles.
- `efe_suggest_test_sentences` — generate edge sentences.
- `efe_analyze_confusion_matrix` — analyse top confusions.
- `efe_draft_runbook` — draft runbook from incident event.

**Profile registration** :
- 5 tools rejoignent le profile `agent-native`.
- Pas de nouveau profile.
- Conditionnel : flag `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=true`.

**Activation flags** (cf. roadmap § F.7) :
- 2 server-level env-vars.
- 1 tenant-level config flag.
- Default OFF.

**Audit + observability** :
- Chaque tool call audité via M14-bis HMAC chain (carry-over).
- OTel span `mcp.tool.dispatch` avec `mcp.tool.name` + `mcp.tool.is_helper=true`.
- Métrique Prometheus `efe_mcp_tool_calls_total{tool, profile}`.

**Documentation** :
- Operator guide `docs/m20_operator_guide.md`.
- Migration guide `docs/m20_migration_guide.md` (court : "no action
  required by default").
- DPIA addendum `docs/compliance/m20_dpia_addendum.md`.

### 3.2 Out of scope (explicitement)

| Hors scope | Pourquoi | Quand |
|---|---|---|
| Mode-2 backend (anchors LLM-routed) | Backend-heavy, coût governance | M21 |
| Mode-3 backend | Multi-turn requires Mode-2 | M22 |
| LLMRouter Rust service | M20 = client-side LLM, no server router | M21 |
| Cost governance per-tenant | M20 sampling delegation = client paye | M21 |
| Curation tools écriture (split, merge) | Modifie taxonomie cluster = write | M23 |
| Active learning loop | Pipeline trainings, gold labels | M24 |
| Agent runtime in-cluster | Sandboxing, K8s CRDs | M25 |
| Embedded LLM in mcp-server image | Image bloat, GPU dependency | Jamais (par design) |
| Custom finetuning | Hors mission EFE (zero-shot) | Jamais |
| Streaming MCP responses | Optionnel, pas de design partner demandé | Future (M21+) |
| Multi-modal LLM input (images) | EFE classify text-only | Jamais (Mode-1) |
| Tool-use round-tripping in efe-agent lib | M20 lib = single-shot LLM call par helper ; tool-use boucle = M25 agent runtime | M25 |
| Provider price discovery / negotiation | Hors scope | Jamais |
| Caching LLM outputs côté EFE | Sampling delegation = pas de visibilité ; direct mode = client-side cache | M21 (LLMRouter) |
| Prompt registry versionnée Postgres | Embedded in lib + mcp-server tools = code | M21 |

### 3.3 Constraints

**Hard constraints** (non-négociables) :

- **C1** : Aucun changement backend Rust (Cargo workspaces inchangées).
- **C2** : Aucun bump EFE contract (1.3 reste GA après M20).
- **C3** : Image Docker mcp-server inchangée en taille par défaut
  (no LLM provider SDK pré-installé).
- **C4** : Default OFF — un cluster M20 sans aucun flag activé est
  bit-for-bit M19 GA.
- **C5** : Tous les tools sont read-only (`is_write=False`).
  `confirm:true` pattern non-applicable.
- **C6** : Pas d'écriture vers le cluster (NEVER mutates state, M16
  invariant carry-over).
- **C7** : Sampling delegation respecte la spec MCP (pas de
  customization vendor-specific).
- **C8** : Output validation Pydantic + JSON Schema sur **tous** les
  helpers (anti-hallucination).

**Soft constraints** (priorités) :

- **S1** : Latency overhead M20 tools = sampling round-trip (~200 ms-2 s
  selon client provider). EFE-side overhead < 50 ms p99.
- **S2** : Code coverage `efe-agent` lib ≥ 90 %.
- **S3** : Code coverage M20 mcp-server tools ≥ 95 %.
- **S4** : Pin tests ajoutent au minimum 35 nouveaux tests.
- **S5** : Documentation operator guide ≤ 30 min de lecture.

---

## 4. Catalogue M20 — vue d'ensemble

### 4.1 Outils (39 cumulés au final, dont 5 NEW si M20 enabled)

| Tool | Profile | Write? | Status M20 | Notes |
|---|---|:---:|:---:|---|
| (34 outils M16-M19) | (varies) | varies | ✅ unchanged | M16-M19 carry-over |
| `efe_explain_classification` | agent-native | No | 🆕 conditional | Sampling delegation |
| `efe_compare_bundles_quality` | agent-native | No | 🆕 conditional | Sampling delegation |
| `efe_suggest_test_sentences` | agent-native | No | 🆕 conditional | Sampling delegation |
| `efe_analyze_confusion_matrix` | agent-native | No | 🆕 conditional | Sampling delegation |
| `efe_draft_runbook` | agent-native | No | 🆕 conditional | Sampling delegation |

**Conditional** = enregistré au démarrage de mcp-server **uniquement
si** `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=true` **AND** tenant flag
ON. Sinon les 5 tools n'apparaissent pas dans `list_tools` output.

### 4.2 Resources (11 — pas de changement)

M20 n'ajoute aucune resource. Les 5 helpers utilisent les resources
existantes pour récupérer le contexte qu'ils passent au LLM via
sampling :

- `efe://bundles/{slug}/{version}/manifest` — pour
  `efe_compare_bundles_quality`.
- `efe://catalogs/{name}/stats` — pour
  `efe_analyze_confusion_matrix`.
- `efe://server/health` (M19 carry-over) — pour
  `efe_draft_runbook` context.
- (autres resources M16-M19) — pour préfixer les prompts.

### 4.3 Prompts (10 — pas de changement)

M20 n'ajoute aucun prompt MCP (les **5 tools utilisent leurs propres
prompt templates internes**, pas exposés via MCP `prompts/get`).
Les prompts MCP existants restent à 10 (M19 carry-over).

### 4.4 Profile tiers (5 — pas de changement)

M20 ne crée pas de nouveau profile. Les 5 tools rejoignent
`agent-native` (read-only operator helpers).

### 4.5 Templates internes (NEW — non-MCP)

5 templates internes dans `mcp-server/mcp_server/tools/m20_prompts/` :

- `explain_classification_v1.md`
- `compare_bundles_quality_v1.md`
- `suggest_test_sentences_v1.md`
- `analyze_confusion_matrix_v1.md`
- `draft_runbook_v1.md`

Chaque template est un fichier Markdown avec header YAML
(version, eval status, provider compat). Versionné semver-mineur
(v1 → v2) sur changement breaking de contenu.

---

## 5. Architecture overview

### 5.1 Component diagram

```
                ┌─────────────────────────────────────────────┐
                │ MCP CLIENT (Claude Desktop / Cursor / ...) │
                │                                             │
                │  ┌────────────────────────────────────┐    │
                │  │ External LLM (Claude / GPT-5 / ...) │    │
                │  └─────────┬──────────────────────────┘    │
                │            │ ▲                              │
                │   sampling │ │ sampling response            │
                │   request  │ │                              │
                │            ▼ │                              │
                │  ┌────────────────────────────────────┐    │
                │  │ MCP transport (stdio / HTTP+SSE)   │    │
                │  └─────────┬──────────────────────────┘    │
                └────────────┼─────────────────────────────────┘
                             │
                             │ JSON-RPC 2.0
                             ▼
                ┌────────────────────────────────────────────────┐
                │ mcp-server (M20 enabled)                       │
                │                                                 │
                │  ┌─────────────────────────────────────┐       │
                │  │ Dispatcher (M16 carry-over)         │       │
                │  │  ─ tool registry (37 tools max)     │       │
                │  │  ─ profile gating                    │       │
                │  │  ─ rate limit + audit (M19 carry)   │       │
                │  └──────────────┬──────────────────────┘       │
                │                 │                              │
                │   tool dispatch │                              │
                │                 ▼                              │
                │  ┌─────────────────────────────────────┐       │
                │  │ M20 helper tool handler             │       │
                │  │  1. Validate input (Pydantic)       │       │
                │  │  2. Fetch context (resources)       │       │
                │  │  3. Render prompt template          │       │
                │  │  4. Issue sampling request via MCP  │       │
                │  │  5. Validate sampling output        │       │
                │  │  6. Format response                  │       │
                │  └──────────────┬──────────────────────┘       │
                │                 │                              │
                │     sampling    │                              │
                │     request     │                              │
                │                 ▼                              │
                │  ┌─────────────────────────────────────┐       │
                │  │ Sampling adapter (M16-M17 carry)    │       │
                │  └─────────────────────────────────────┘       │
                │                                                 │
                │  (Audit chain HMAC-keyed per-tenant)            │
                └────────────────────────────────────────────────┘
                             │
                             │ (read-only)
                             ▼
                ┌────────────────────────────────────────────────┐
                │ EFE backend (Rust, M19 GA, unchanged)          │
                │  ─ Mode-1 classify                              │
                │  ─ Bundle metadata                              │
                │  ─ Catalog stats                                │
                └────────────────────────────────────────────────┘

         OR (alternative path)
                             │
                             ▼
                ┌────────────────────────────────────────────────┐
                │ Operator workstation                            │
                │                                                 │
                │  ┌─────────────────────────────────────┐       │
                │  │ Python script using `efe-agent` lib │       │
                │  │  ─ from efe_agent import Agent      │       │
                │  │  ─ Agent(provider=AnthropicProvider) │       │
                │  │  ─ agent.run("...")                  │       │
                │  └──────────────┬──────────────────────┘       │
                │                 │                              │
                │      direct     │                              │
                │      provider   ▼                              │
                │  ┌─────────────────────────────────────┐       │
                │  │ Anthropic / OpenAI / vLLM / ...     │       │
                │  └──────────────┬──────────────────────┘       │
                │                 │                              │
                │      MCP RPC    ▼                              │
                │  ┌─────────────────────────────────────┐       │
                │  │ mcp-server (read-only profile)      │       │
                │  └─────────────────────────────────────┘       │
                └────────────────────────────────────────────────┘
```

### 5.2 Sampling delegation pattern (M16-M17 carry-over)

**Sequence** :

```
mcp-client       mcp-server                External LLM
     │              │                            │
     │ tool/call    │                            │
     │ ──────────►  │                            │
     │              │ sampling/createMessage     │
     │              │ ─────────────────────────► │
     │              │                            │
     │              │      (LLM execution)       │
     │              │                            │
     │              │ ◄───────────────────────── │
     │              │ sampling response          │
     │              │                            │
     │              │ (validate + format)        │
     │              │                            │
     │ tool/call    │                            │
     │ ◄──────────  │                            │
     │ result       │                            │
```

**Key points** :
- mcp-server **n'exécute jamais** le LLM directement (sauf en mode
  `direct provider`, § 14).
- Sampling response est validée Pydantic.
- Si validation échoue, retry policy (max 2 retries) avec un prompt
  régénéré contenant les erreurs.
- Audit emit après validation OK.

### 5.3 Direct provider mode (`efe-agent` lib)

Use case : opérateur veut faire un batch eval sans démarrer Claude
Desktop.

```python
from efe_agent import Agent
from efe_agent.llm import AnthropicProvider

provider = AnthropicProvider(model="claude-sonnet-4-5")
agent = Agent(
    mcp_endpoint="http://efe-mcp.example.com:8800",
    oidc_token=os.environ["EFE_OIDC_TOKEN"],
    llm_provider=provider,
)

# Use a M20 helper directly:
result = await agent.helper(
    "explain_classification",
    catalog="support-routing",
    sentence="My laptop won't boot",
    classification_result={
        "label": "hardware_issue",
        "confidence": 0.87,
        "anchors_used": ["hardware", "boot", "device"],
    },
)
print(result.explanation)
```

Le `Agent.helper()` fait :
1. Render le prompt template (même que côté mcp-server tool).
2. Call provider directement (pas de sampling delegation).
3. Validate output (même validation que côté mcp-server).
4. Optionnellement, écrire un audit event via mcp-server resource
   read (pour traçabilité).

### 5.4 Lifecycle & threading

**mcp-server M20 tools** : exécution synchrone async/await (pattern
M16-M19), pas de threading dédié. Sampling round-trip dure 200 ms-2 s
selon client.

**`efe-agent` Python lib** : ASGI-compatible (uvicorn / hypercorn),
async by default. Provider SDKs sont async-native (Anthropic /
OpenAI ont des `AsyncClient`).

**Concurrency** : `Agent` instances sont thread-safe via
`asyncio.Lock` interne sur le LLM call. Plusieurs Agents en parallèle
OK.

---

## 6. La librairie Python `efe-agent`

### 6.1 Package layout

```
clients/python/
├── pyproject.toml                    # NEW — efe-agent package
├── efe_agent/
│   ├── __init__.py
│   ├── __version__.py                # __version__ = "0.20.0"
│   ├── agent.py                      # Agent class
│   ├── exceptions.py                 # Exception hierarchy
│   ├── types.py                      # Pydantic models
│   ├── prompts/                      # Prompt templates
│   │   ├── __init__.py
│   │   ├── loader.py
│   │   ├── explain_classification_v1.md
│   │   ├── compare_bundles_quality_v1.md
│   │   ├── suggest_test_sentences_v1.md
│   │   ├── analyze_confusion_matrix_v1.md
│   │   └── draft_runbook_v1.md
│   ├── llm/                          # Provider abstractions
│   │   ├── __init__.py
│   │   ├── base.py                   # LLMProvider Protocol
│   │   ├── fallback.py               # FallbackChain
│   │   ├── circuit_breaker.py        # Circuit breaker
│   │   ├── anthropic.py              # AnthropicProvider
│   │   ├── openai.py                 # OpenAIProvider
│   │   ├── bedrock.py                # BedrockProvider
│   │   ├── azure_openai.py           # AzureOpenAIProvider
│   │   ├── vertex.py                 # VertexProvider
│   │   ├── vllm.py                   # VLLMProvider (OpenAI-compat)
│   │   ├── llama_cpp.py              # LlamaCppProvider
│   │   └── mlx.py                    # MLXProvider
│   ├── helpers/                      # Tool wrappers (parallel to mcp-server)
│   │   ├── __init__.py
│   │   ├── explain_classification.py
│   │   ├── compare_bundles_quality.py
│   │   ├── suggest_test_sentences.py
│   │   ├── analyze_confusion_matrix.py
│   │   └── draft_runbook.py
│   ├── mcp_client.py                 # Lightweight MCP client
│   ├── audit.py                      # Audit event emission
│   ├── cli.py                        # `efe-agent` CLI
│   └── examples/
│       ├── __init__.py
│       ├── batch_classify.py
│       ├── operator_curation.py
│       └── incident_response.py
└── tests/
    ├── conftest.py
    ├── test_agent.py
    ├── test_llm_providers.py
    ├── test_helpers.py
    └── test_cli.py
```

### 6.2 `pyproject.toml`

```toml
[project]
name = "efe-agent"
version = "0.20.0"
description = "BYO-LLM agent SDK for Engaging Fluide Éphémère (EFE)"
authors = [{name = "EFE Team"}]
readme = "README.md"
requires-python = ">=3.11"
license = {text = "Apache-2.0"}
dependencies = [
    "httpx>=0.27.0",
    "pydantic>=2.6.0",
    "PyYAML>=6.0",
    "structlog>=24.1.0",         # M19 carry-over
    "opentelemetry-api>=1.28.0",  # M19 carry-over
    "tenacity>=8.2.0",            # retry logic
]
classifiers = [
    "Programming Language :: Python :: 3.11",
    "Programming Language :: Python :: 3.12",
    "Programming Language :: Python :: 3.13",
    "License :: OSI Approved :: Apache Software License",
]

[project.optional-dependencies]
anthropic = ["anthropic>=0.45.0"]
openai = ["openai>=1.50.0"]
bedrock = ["boto3>=1.35.0"]
azure-openai = ["openai>=1.50.0", "azure-identity>=1.18.0"]
vertex = ["google-cloud-aiplatform>=1.70.0"]
vllm = []   # vLLM uses OpenAI-compat, no extra deps
llama-cpp = []   # llama.cpp uses OpenAI-compat
mlx = ["mlx-lm>=0.20.0"]
all = [
    "anthropic>=0.45.0",
    "openai>=1.50.0",
    "boto3>=1.35.0",
    "azure-identity>=1.18.0",
    "google-cloud-aiplatform>=1.70.0",
    "mlx-lm>=0.20.0",
]
test = [
    "pytest>=8.0",
    "pytest-asyncio>=0.23",
    "pytest-cov>=4.0",
    "respx>=0.20",
]

[project.scripts]
efe-agent = "efe_agent.cli:main"
```

### 6.3 `LLMProvider` Protocol

```python
# clients/python/efe_agent/llm/base.py

from typing import Protocol, AsyncIterator, runtime_checkable
from .types import LLMResponse, ProviderHealth, CostModel


@runtime_checkable
class LLMProvider(Protocol):
    """Provider abstraction for BYO-LLM.
    
    Implementations live in efe_agent/llm/{provider}.py.
    """
    
    @property
    def name(self) -> str:
        """Stable provider identifier (e.g. 'anthropic', 'vllm-local')."""
        ...
    
    @property
    def supports_tools(self) -> bool:
        """Whether the provider supports tool/function calling."""
        ...
    
    @property
    def supports_streaming(self) -> bool:
        """Whether the provider supports streaming responses."""
        ...
    
    @property
    def max_context_tokens(self) -> int:
        """Maximum input tokens supported (for the configured model)."""
        ...
    
    @property
    def cost_model(self) -> CostModel:
        """Pricing info for cost estimation."""
        ...
    
    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse:
        """Single-shot completion."""
        ...
    
    async def stream(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        timeout_s: float = 30.0,
    ) -> AsyncIterator[LLMResponse]:
        """Streaming completion. Default impl yields a single response."""
        ...
    
    async def health(self) -> ProviderHealth:
        """Provider liveness check."""
        ...
```

### 6.4 Pydantic types

```python
# clients/python/efe_agent/types.py

from pydantic import BaseModel, Field
from typing import Literal, Any


class CostModel(BaseModel):
    """Pricing parameters for cost estimation."""
    input_per_1m_tokens_usd: float = Field(ge=0)
    output_per_1m_tokens_usd: float = Field(ge=0)
    currency: Literal["USD"] = "USD"


class LLMResponse(BaseModel):
    """Provider-agnostic LLM response."""
    text: str
    tokens_in: int = Field(ge=0)
    tokens_out: int = Field(ge=0)
    cost_usd: float = Field(ge=0.0)
    finish_reason: Literal["stop", "length", "tool_use", "safety", "error"]
    tool_calls: list[dict] | None = None
    cache_hit: bool = False
    provider_name: str
    model_name: str
    raw_response: Any | None = Field(default=None, exclude=True)


class ProviderHealth(BaseModel):
    available: bool
    latency_p50_ms: float | None = None
    latency_p99_ms: float | None = None
    error_rate_30d: float | None = None
    last_check_ts: float


class HelperRequest(BaseModel):
    """Base request model for all M20 helpers."""
    catalog: str | None = None
    sentence: str | None = None
    classification_result: dict | None = None


class ExplainClassificationOutput(BaseModel):
    """Output of efe_explain_classification helper.
    
    Strict schema — anti-hallucination.
    """
    explanation: str = Field(min_length=20, max_length=1000)
    key_factors: list[str] = Field(min_length=1, max_length=5)
    confidence_assessment: Literal["high", "medium", "low"]
    suggested_alternatives: list[str] = Field(default_factory=list, max_length=3)


class CompareBundlesOutput(BaseModel):
    """Output of efe_compare_bundles_quality helper."""
    n_sentences_compared: int = Field(ge=0)
    n_divergent: int = Field(ge=0)
    accuracy_delta_pp: float
    divergence_examples: list[dict] = Field(max_length=20)
    summary: str = Field(min_length=20, max_length=500)
    recommendation: Literal["promote", "investigate", "do_not_promote"]


class SuggestTestSentencesOutput(BaseModel):
    """Output of efe_suggest_test_sentences helper."""
    sentences: list[str] = Field(min_length=1, max_length=64)
    difficulty_self_assessment: dict[str, Literal["easy", "medium", "hard", "edge"]]
    rationale: str = Field(min_length=20, max_length=500)


class ConfusionAnalysisOutput(BaseModel):
    """Output of efe_analyze_confusion_matrix helper."""
    top_confusion_pairs: list[dict] = Field(max_length=10)
    proposed_actions: list[dict] = Field(max_length=10)
    summary: str = Field(min_length=20, max_length=1000)


class DraftRunbookOutput(BaseModel):
    """Output of efe_draft_runbook helper."""
    runbook_markdown: str = Field(min_length=200)
    estimated_resolution_time_min: int = Field(ge=1, le=240)
    primary_diagnostics: list[str] = Field(min_length=1, max_length=10)
    risk_level: Literal["low", "medium", "high"]
```

### 6.5 `Agent` class

```python
# clients/python/efe_agent/agent.py

from typing import Any
import structlog
from .llm.base import LLMProvider
from .mcp_client import MCPClient
from .helpers import HELPERS
from .types import LLMResponse


log = structlog.get_logger()


class Agent:
    """High-level facade for BYO-LLM workflows against an EFE mcp-server.
    
    Two modes:
    1. helper(name, **args)  — invoke a M20 helper directly via direct
                                 provider mode (no MCP sampling).
    2. mcp_tool(name, args)  — invoke any mcp-server tool via MCP RPC ;
                                 sampling delegates to this Agent's provider.
    """
    
    def __init__(
        self,
        *,
        mcp_endpoint: str,
        oidc_token: str,
        llm_provider: LLMProvider,
        timeout_s: float = 60.0,
        max_retries: int = 2,
    ) -> None:
        self.mcp = MCPClient(endpoint=mcp_endpoint, oidc_token=oidc_token)
        self.llm = llm_provider
        self.timeout_s = timeout_s
        self.max_retries = max_retries
    
    async def helper(
        self,
        name: str,
        **kwargs: Any,
    ) -> Any:
        """Invoke a M20 helper directly using direct provider mode.
        
        Identical prompt + validation logic as the mcp-server tool, but
        runs locally — useful for batch CI / scripts.
        """
        if name not in HELPERS:
            raise ValueError(
                f"Unknown helper {name!r}. Known: {list(HELPERS.keys())}"
            )
        helper_cls = HELPERS[name]
        helper = helper_cls(agent=self)
        return await helper.run(**kwargs)
    
    async def mcp_tool(
        self,
        name: str,
        arguments: dict,
    ) -> dict:
        """Invoke any mcp-server tool via MCP RPC.
        
        Sampling requests from mcp-server are routed back to this
        Agent's provider.
        """
        return await self.mcp.call_tool(
            name=name,
            arguments=arguments,
            sampling_callback=self._handle_sampling,
        )
    
    async def _handle_sampling(self, request: dict) -> LLMResponse:
        """Bridge MCP sampling request → direct provider call."""
        return await self.llm.complete(
            system_prompt=request.get("systemPrompt"),
            messages=request["messages"],
            max_tokens=request.get("maxTokens", 1024),
            temperature=request.get("temperature", 0.0),
            timeout_s=self.timeout_s,
        )
    
    async def aclose(self) -> None:
        await self.mcp.aclose()
```

### 6.6 `FallbackChain`

```python
# clients/python/efe_agent/llm/fallback.py

import asyncio
import structlog
from typing import Any
from .base import LLMProvider
from .circuit_breaker import CircuitBreaker
from ..exceptions import AllProvidersFailedError, ProviderError
from ..types import LLMResponse, ProviderHealth, CostModel


log = structlog.get_logger()


class FallbackChain:
    """Multi-provider failover with per-provider circuit breaker.
    
    Implements the LLMProvider Protocol so it can be swapped in.
    """
    
    def __init__(
        self,
        *,
        primary: LLMProvider,
        fallbacks: list[LLMProvider],
        per_provider_timeout_s: float = 30.0,
        circuit_breaker_threshold: int = 5,
        circuit_breaker_open_duration_s: float = 60.0,
    ) -> None:
        self._chain: list[LLMProvider] = [primary, *fallbacks]
        self._timeout = per_provider_timeout_s
        self._breakers: dict[str, CircuitBreaker] = {
            p.name: CircuitBreaker(
                threshold=circuit_breaker_threshold,
                open_duration_s=circuit_breaker_open_duration_s,
            )
            for p in self._chain
        }
    
    @property
    def name(self) -> str:
        return f"fallback({','.join(p.name for p in self._chain)})"
    
    @property
    def supports_tools(self) -> bool:
        return all(p.supports_tools for p in self._chain)
    
    @property
    def supports_streaming(self) -> bool:
        return all(p.supports_streaming for p in self._chain)
    
    @property
    def max_context_tokens(self) -> int:
        return min(p.max_context_tokens for p in self._chain)
    
    @property
    def cost_model(self) -> CostModel:
        return self._chain[0].cost_model
    
    async def complete(self, **kwargs: Any) -> LLMResponse:
        errors: list[Exception] = []
        for provider in self._chain:
            breaker = self._breakers[provider.name]
            if not breaker.is_closed():
                log.debug("circuit_open", provider=provider.name)
                continue
            try:
                resp = await asyncio.wait_for(
                    provider.complete(**kwargs),
                    timeout=self._timeout,
                )
                breaker.record_success()
                return resp
            except (ProviderError, asyncio.TimeoutError) as e:
                breaker.record_failure()
                errors.append(e)
                log.warning(
                    "provider_failed",
                    provider=provider.name,
                    error=str(e),
                )
        raise AllProvidersFailedError(
            f"All {len(self._chain)} providers failed",
            errors=errors,
        )
    
    async def stream(self, **kwargs: Any):
        # Streaming not supported through fallback chain in M20.
        # Direct stream from first available provider only.
        for provider in self._chain:
            breaker = self._breakers[provider.name]
            if breaker.is_closed():
                async for chunk in provider.stream(**kwargs):
                    yield chunk
                return
        raise AllProvidersFailedError("All providers' circuits open")
    
    async def health(self) -> ProviderHealth:
        results = await asyncio.gather(
            *[p.health() for p in self._chain],
            return_exceptions=True,
        )
        any_available = any(
            isinstance(r, ProviderHealth) and r.available
            for r in results
        )
        return ProviderHealth(
            available=any_available,
            last_check_ts=__import__("time").time(),
        )
```

### 6.7 Circuit breaker

```python
# clients/python/efe_agent/llm/circuit_breaker.py

import time
from enum import Enum


class CircuitState(Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half_open"


class CircuitBreaker:
    def __init__(self, *, threshold: int, open_duration_s: float) -> None:
        self._threshold = threshold
        self._open_duration = open_duration_s
        self._state = CircuitState.CLOSED
        self._consecutive_failures = 0
        self._opened_at: float | None = None
    
    def record_success(self) -> None:
        self._consecutive_failures = 0
        self._state = CircuitState.CLOSED
    
    def record_failure(self) -> None:
        self._consecutive_failures += 1
        if self._consecutive_failures >= self._threshold:
            self._state = CircuitState.OPEN
            self._opened_at = time.monotonic()
    
    def is_closed(self) -> bool:
        if self._state == CircuitState.CLOSED:
            return True
        if self._state == CircuitState.OPEN:
            if time.monotonic() - (self._opened_at or 0) >= self._open_duration:
                self._state = CircuitState.HALF_OPEN
                return True   # let one request through
            return False
        # HALF_OPEN: let through ; record_success/failure transitions
        return True
```

### 6.8 Prompt template loader

```python
# clients/python/efe_agent/prompts/loader.py

import yaml
from pathlib import Path
from pydantic import BaseModel
from string import Template


class PromptTemplate(BaseModel):
    name: str
    version: int
    license: str
    provider_compat: list[str]
    min_context_tokens: int
    max_response_tokens: int
    eval_pass: bool
    eval_accuracy: float | None = None
    body: str


_TEMPLATE_CACHE: dict[str, PromptTemplate] = {}


def load(template_id: str) -> PromptTemplate:
    """Load a prompt template by id (e.g. 'explain_classification_v1').
    
    Cached on first load.
    """
    if template_id in _TEMPLATE_CACHE:
        return _TEMPLATE_CACHE[template_id]
    
    path = Path(__file__).parent / f"{template_id}.md"
    if not path.exists():
        raise FileNotFoundError(f"Prompt template not found: {template_id}")
    
    raw = path.read_text(encoding="utf-8")
    if not raw.startswith("---"):
        raise ValueError(f"Template {template_id} missing YAML header")
    
    _, header_yaml, body = raw.split("---", 2)
    header = yaml.safe_load(header_yaml)
    template = PromptTemplate(body=body.strip(), **header)
    _TEMPLATE_CACHE[template_id] = template
    return template


def render(template_id: str, **kwargs) -> str:
    """Load + substitute (Python string.Template — no Jinja2 in M20)."""
    tmpl = load(template_id)
    return Template(tmpl.body).safe_substitute(**kwargs)
```

---

## 7. Provider implementations (8)

### 7.1 Anthropic

```python
# clients/python/efe_agent/llm/anthropic.py

import os
from typing import AsyncIterator
import anthropic
from .base import LLMProvider
from ..types import LLMResponse, ProviderHealth, CostModel


_ANTHROPIC_MODEL_PRICING = {
    "claude-opus-4-7": (15.0, 75.0),
    "claude-sonnet-4-5": (3.0, 15.0),
    "claude-haiku-4": (0.30, 1.50),
}

_ANTHROPIC_MAX_CONTEXT = {
    "claude-opus-4-7": 1_000_000,
    "claude-sonnet-4-5": 200_000,
    "claude-haiku-4": 200_000,
}


class AnthropicProvider:
    """LLMProvider implementation for Anthropic API."""
    
    def __init__(
        self,
        *,
        model: str = "claude-sonnet-4-5",
        api_key: str | None = None,
    ) -> None:
        self._model = model
        self._client = anthropic.AsyncAnthropic(
            api_key=api_key or os.environ["ANTHROPIC_API_KEY"],
        )
    
    @property
    def name(self) -> str:
        return f"anthropic:{self._model}"
    
    @property
    def supports_tools(self) -> bool:
        return True
    
    @property
    def supports_streaming(self) -> bool:
        return True
    
    @property
    def max_context_tokens(self) -> int:
        return _ANTHROPIC_MAX_CONTEXT.get(self._model, 200_000)
    
    @property
    def cost_model(self) -> CostModel:
        in_, out_ = _ANTHROPIC_MODEL_PRICING.get(self._model, (0.0, 0.0))
        return CostModel(
            input_per_1m_tokens_usd=in_,
            output_per_1m_tokens_usd=out_,
        )
    
    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse:
        kwargs = {
            "model": self._model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "timeout": timeout_s,
        }
        if system_prompt:
            kwargs["system"] = system_prompt
        if tools:
            kwargs["tools"] = tools
        
        resp = await self._client.messages.create(**kwargs)
        text = ""
        tool_calls: list[dict] = []
        for block in resp.content:
            if block.type == "text":
                text += block.text
            elif block.type == "tool_use":
                tool_calls.append({
                    "id": block.id,
                    "name": block.name,
                    "input": block.input,
                })
        
        cost = self._compute_cost(resp.usage.input_tokens, resp.usage.output_tokens)
        
        return LLMResponse(
            text=text,
            tokens_in=resp.usage.input_tokens,
            tokens_out=resp.usage.output_tokens,
            cost_usd=cost,
            finish_reason=self._map_stop_reason(resp.stop_reason),
            tool_calls=tool_calls or None,
            provider_name=self.name,
            model_name=self._model,
            raw_response=resp,
        )
    
    async def stream(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        timeout_s: float = 30.0,
    ) -> AsyncIterator[LLMResponse]:
        # Simplified : single-response yield for M20.
        resp = await self.complete(
            system_prompt=system_prompt,
            messages=messages,
            tools=tools,
            max_tokens=max_tokens,
            temperature=temperature,
            timeout_s=timeout_s,
        )
        yield resp
    
    async def health(self) -> ProviderHealth:
        try:
            await self._client.messages.create(
                model=self._model,
                messages=[{"role": "user", "content": "ping"}],
                max_tokens=10,
            )
            return ProviderHealth(
                available=True,
                last_check_ts=__import__("time").time(),
            )
        except Exception:
            return ProviderHealth(
                available=False,
                last_check_ts=__import__("time").time(),
            )
    
    def _compute_cost(self, in_: int, out_: int) -> float:
        cm = self.cost_model
        return (
            in_ * cm.input_per_1m_tokens_usd / 1_000_000
            + out_ * cm.output_per_1m_tokens_usd / 1_000_000
        )
    
    def _map_stop_reason(self, sr: str | None) -> str:
        return {
            "end_turn": "stop",
            "max_tokens": "length",
            "stop_sequence": "stop",
            "tool_use": "tool_use",
        }.get(sr or "", "stop")
```

### 7.2 OpenAI

```python
# clients/python/efe_agent/llm/openai.py

import os
from typing import AsyncIterator
import openai
from .base import LLMProvider
from ..types import LLMResponse, ProviderHealth, CostModel


_OPENAI_PRICING = {
    "gpt-5": (10.0, 40.0),
    "gpt-5-mini": (0.30, 1.20),
    "gpt-4o": (2.50, 10.0),
}

_OPENAI_MAX_CONTEXT = {
    "gpt-5": 256_000,
    "gpt-5-mini": 128_000,
    "gpt-4o": 128_000,
}


class OpenAIProvider:
    def __init__(
        self,
        *,
        model: str = "gpt-5-mini",
        api_key: str | None = None,
        base_url: str | None = None,
    ) -> None:
        self._model = model
        self._client = openai.AsyncOpenAI(
            api_key=api_key or os.environ["OPENAI_API_KEY"],
            base_url=base_url,
        )
    
    @property
    def name(self) -> str:
        return f"openai:{self._model}"
    
    @property
    def supports_tools(self) -> bool:
        return True
    
    @property
    def supports_streaming(self) -> bool:
        return True
    
    @property
    def max_context_tokens(self) -> int:
        return _OPENAI_MAX_CONTEXT.get(self._model, 128_000)
    
    @property
    def cost_model(self) -> CostModel:
        in_, out_ = _OPENAI_PRICING.get(self._model, (0.0, 0.0))
        return CostModel(
            input_per_1m_tokens_usd=in_,
            output_per_1m_tokens_usd=out_,
        )
    
    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse:
        msgs: list[dict] = []
        if system_prompt:
            msgs.append({"role": "system", "content": system_prompt})
        msgs.extend(messages)
        
        kwargs = {
            "model": self._model,
            "messages": msgs,
            "max_completion_tokens": max_tokens,
            "temperature": temperature,
            "timeout": timeout_s,
        }
        if tools:
            kwargs["tools"] = tools
        if seed is not None:
            kwargs["seed"] = seed
        
        resp = await self._client.chat.completions.create(**kwargs)
        choice = resp.choices[0]
        
        return LLMResponse(
            text=choice.message.content or "",
            tokens_in=resp.usage.prompt_tokens,
            tokens_out=resp.usage.completion_tokens,
            cost_usd=self._compute_cost(resp.usage.prompt_tokens, resp.usage.completion_tokens),
            finish_reason=self._map_finish_reason(choice.finish_reason),
            tool_calls=[
                {"id": tc.id, "name": tc.function.name, "input": tc.function.arguments}
                for tc in (choice.message.tool_calls or [])
            ] or None,
            provider_name=self.name,
            model_name=self._model,
            raw_response=resp,
        )
    
    async def stream(self, **kwargs):
        resp = await self.complete(**kwargs)
        yield resp
    
    async def health(self) -> ProviderHealth:
        try:
            await self._client.chat.completions.create(
                model=self._model,
                messages=[{"role": "user", "content": "ping"}],
                max_completion_tokens=10,
            )
            return ProviderHealth(available=True, last_check_ts=__import__("time").time())
        except Exception:
            return ProviderHealth(available=False, last_check_ts=__import__("time").time())
    
    def _compute_cost(self, in_, out_) -> float:
        cm = self.cost_model
        return in_ * cm.input_per_1m_tokens_usd / 1e6 + out_ * cm.output_per_1m_tokens_usd / 1e6
    
    def _map_finish_reason(self, fr: str | None) -> str:
        return {
            "stop": "stop",
            "length": "length",
            "tool_calls": "tool_use",
            "content_filter": "safety",
        }.get(fr or "", "stop")
```

### 7.3 AWS Bedrock

(Sketch — full implementation in `efe_agent/llm/bedrock.py` following same Protocol.)

```python
class BedrockProvider:
    """AWS Bedrock — supports Claude models via converse() API."""
    
    def __init__(
        self,
        *,
        model_id: str = "anthropic.claude-sonnet-4-5-v1:0",
        region: str | None = None,
    ) -> None:
        import boto3
        self._model_id = model_id
        self._client = boto3.client(
            "bedrock-runtime",
            region_name=region or os.environ.get("AWS_REGION", "us-east-1"),
        )
        # Use sync client wrapped in run_in_executor for asyncio.
    
    # complete(), stream(), health() implemented similarly.
```

**Auth chain** : IRSA → IAM role → AWS_PROFILE → static creds (M19
BYOK carry-over pattern).

### 7.4 Azure OpenAI

```python
class AzureOpenAIProvider:
    def __init__(
        self,
        *,
        deployment_name: str,
        endpoint: str,
        api_version: str = "2024-08-01-preview",
        api_key: str | None = None,
    ) -> None:
        from azure.identity import DefaultAzureCredential
        if api_key:
            self._client = openai.AsyncAzureOpenAI(
                api_key=api_key,
                api_version=api_version,
                azure_endpoint=endpoint,
            )
        else:
            credential = DefaultAzureCredential()
            self._client = openai.AsyncAzureOpenAI(
                azure_ad_token_provider=lambda: credential.get_token(
                    "https://cognitiveservices.azure.com/.default"
                ).token,
                api_version=api_version,
                azure_endpoint=endpoint,
            )
        self._deployment = deployment_name
    
    # ...
```

### 7.5 Google Vertex AI

```python
class VertexProvider:
    def __init__(
        self,
        *,
        model: str = "gemini-2.5-flash",
        project: str,
        location: str = "us-central1",
    ) -> None:
        from google.cloud import aiplatform
        aiplatform.init(project=project, location=location)
        # Use Vertex AI SDK ; wrap in async via run_in_executor.
```

### 7.6 vLLM (self-host)

```python
class VLLMProvider:
    """vLLM exposes an OpenAI-compatible HTTP API.
    
    Reuses OpenAIProvider with a custom base_url.
    """
    
    def __init__(
        self,
        *,
        server_url: str = "http://localhost:8000/v1",
        model: str,
        api_key: str = "EMPTY",  # vLLM ignores
    ) -> None:
        self._inner = OpenAIProvider(
            model=model,
            api_key=api_key,
            base_url=server_url,
        )
    
    @property
    def name(self) -> str:
        return f"vllm:{self._inner._model}"
    
    @property
    def cost_model(self) -> CostModel:
        # Self-host = $0 marginal cost
        return CostModel(
            input_per_1m_tokens_usd=0.0,
            output_per_1m_tokens_usd=0.0,
        )
    
    # Delegate complete/stream/health to inner.
```

### 7.7 llama.cpp (local)

```python
class LlamaCppProvider:
    """llama.cpp HTTP server (OpenAI-compatible mode)."""
    
    def __init__(
        self,
        *,
        server_url: str = "http://localhost:8080/v1",
        model: str = "local-model",
    ) -> None:
        self._inner = OpenAIProvider(
            model=model, api_key="sk-no-key-required",
            base_url=server_url,
        )
    
    @property
    def name(self) -> str:
        return f"llama-cpp:{self._inner._model}"
    
    @property
    def cost_model(self) -> CostModel:
        return CostModel(
            input_per_1m_tokens_usd=0.0,
            output_per_1m_tokens_usd=0.0,
        )
```

### 7.8 Apple MLX (Mac local)

```python
class MLXProvider:
    """Apple MLX local models (Mac M-series only)."""
    
    def __init__(self, *, model_path: str) -> None:
        from mlx_lm import load, generate
        self._model, self._tokenizer = load(model_path)
        self._generate = generate
    
    @property
    def name(self) -> str:
        return f"mlx:{self._model}"
    
    @property
    def supports_tools(self) -> bool:
        return False  # MLX doesn't support tools natively in M20
    
    @property
    def cost_model(self) -> CostModel:
        return CostModel(
            input_per_1m_tokens_usd=0.0,
            output_per_1m_tokens_usd=0.0,
        )
    
    async def complete(self, **kwargs) -> LLMResponse:
        import asyncio
        text = await asyncio.to_thread(
            self._generate,
            self._model,
            self._tokenizer,
            kwargs["messages"][-1]["content"],
            max_tokens=kwargs.get("max_tokens", 1024),
        )
        # Token counting approximate — len(text) // 4.
        return LLMResponse(
            text=text,
            tokens_in=len(kwargs["messages"][-1]["content"]) // 4,
            tokens_out=len(text) // 4,
            cost_usd=0.0,
            finish_reason="stop",
            provider_name=self.name,
            model_name="mlx",
        )
```

### 7.9 Provider extras

```bash
pip install efe-agent                  # base, no providers
pip install efe-agent[anthropic]       # + anthropic SDK
pip install efe-agent[openai]          # + openai SDK
pip install efe-agent[bedrock]         # + boto3
pip install efe-agent[azure-openai]    # + openai + azure-identity
pip install efe-agent[vertex]          # + google-cloud-aiplatform
pip install efe-agent[vllm]            # no extra dep (uses openai-compat)
pip install efe-agent[llama-cpp]       # no extra dep
pip install efe-agent[mlx]             # + mlx-lm (Mac only)
pip install efe-agent[all]             # everything
```

---

## 8. Tool 1 — `efe_explain_classification`

### 8.1 Mission

Donné une classification Mode-1 (label, confidence, anchors_used),
demander au LLM (côté client via sampling) une explication
structurée en 3-5 sentences. Use case principal : opérateur reçoit
un classify "support routing → hardware_issue, conf=0.87" et veut
comprendre pourquoi avant de décider d'overrider ou non.

### 8.2 INPUT_SCHEMA

```python
INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeExplainClassificationInput",
    "description": "Get an LLM-generated explanation for a Mode-1 classification result.",
    "properties": {
        "catalog": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "Catalog name the classification was performed against.",
        },
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
            "description": "Bundle version (semver, e.g. 'v1.2.0').",
        },
        "sentence": {
            "type": "string",
            "minLength": 1,
            "maxLength": 4096,
        },
        "classification_result": {
            "type": "object",
            "properties": {
                "label": {"type": "string", "minLength": 1, "maxLength": 100},
                "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                "anchors_used": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 10,
                },
            },
            "required": ["label", "confidence"],
            "additionalProperties": False,
        },
        "language_hint": {
            "type": "string",
            "enum": ["en", "fr", "de", "es", "pt", "it"],
            "default": "en",
            "description": "Preferred output language.",
        },
    },
    "required": ["catalog", "sentence", "classification_result"],
    "additionalProperties": False,
}
```

### 8.3 Handler

```python
# mcp-server/mcp_server/tools/m20_explain_classification.py

from typing import Any
import json

from mcp_server.tools.types import Tool, ToolContext, ToolError
from mcp_server.tools.m20_prompts import render_prompt
from mcp_server.tools.m20_validation import validate_explain_classification_output


INPUT_SCHEMA: dict[str, Any] = {  # ... see § 8.2 ...}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.feature_enabled("m20_helpers"):
        raise ToolError(
            "M20 helpers not enabled for this tenant",
            code="feature_disabled",
        )
    
    catalog = args["catalog"]
    sentence = args["sentence"]
    cls_result = args["classification_result"]
    version = args.get("version")
    lang = args.get("language_hint", "en")
    
    # 1. Optionally fetch class definitions for richer context
    class_definitions = None
    if version:
        try:
            class_definitions = await ctx.backend.get_class_definitions(
                catalog=catalog, version=version, auth=ctx.auth,
            )
        except Exception:
            class_definitions = None  # non-fatal
    
    # 2. Render prompt template
    system_prompt, user_prompt = render_prompt(
        "explain_classification_v1",
        catalog=catalog,
        sentence=sentence,
        classification_result_json=json.dumps(cls_result),
        class_definitions_json=json.dumps(class_definitions or {}),
        language=lang,
    )
    
    # 3. Issue sampling request via MCP
    sampling_resp = await ctx.sampling.create_message(
        system_prompt=system_prompt,
        messages=[{"role": "user", "content": user_prompt}],
        max_tokens=600,
        temperature=0.0,
        # Hint to client: quality matters, latency tolerable
        model_preferences={
            "intelligence_priority": 0.7,
            "speed_priority": 0.3,
            "cost_priority": 0.0,
        },
    )
    
    # 4. Validate output (Pydantic + retry on hallucination)
    output = await validate_explain_classification_output(
        raw_text=sampling_resp.content,
        max_retries=2,
        retry_callback=lambda errs: _retry_with_errors(ctx, system_prompt, user_prompt, errs),
    )
    
    # 5. Format & return
    return {
        "explanation": output.explanation,
        "key_factors": output.key_factors,
        "confidence_assessment": output.confidence_assessment,
        "suggested_alternatives": output.suggested_alternatives,
        "metadata": {
            "tokens_in": sampling_resp.tokens_in,
            "tokens_out": sampling_resp.tokens_out,
            "model_used": sampling_resp.model_name,
            "language": lang,
        },
    }


TOOL = Tool(
    name="efe_explain_classification",
    description=(
        "Get a human-readable explanation of why a Mode-1 classification "
        "produced a given label. Uses MCP sampling — the LLM call is "
        "delegated to the client. READ-ONLY."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="m20_helpers",
)
```

### 8.4 Prompt template `explain_classification_v1.md`

```markdown
---
name: explain_classification_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, llama-cpp]
min_context_tokens: 4000
max_response_tokens: 600
created: 2026-08-15
author: tech_lead_m20
eval_pass: true
eval_dataset: gold_dataset_v1.0
eval_accuracy: 0.92
notes: |
  Tested against 200 explanation prompts. 92% pass strict validation
  (key_factors >=1, explanation 20-1000 chars, confidence_assessment
  in enum). Fail cases mostly long explanations >1000 chars.
---

# System
You are an expert in {{ catalog }} classification. You explain why
a sentence was classified as a particular class, focusing on the
linguistic and semantic features that drove the decision.

Rules:
- Output VALID JSON matching this exact schema:
  {
    "explanation": "<20-1000 char summary>",
    "key_factors": ["<factor 1>", "<factor 2>", ...],
    "confidence_assessment": "high" | "medium" | "low",
    "suggested_alternatives": ["<class 1>", ...]
  }
- Use the language: {{ language }}.
- Do NOT include markdown code fences in the output.
- Keep explanation concise, actionable.
- key_factors: 1-5 items, each ≤ 50 chars.
- suggested_alternatives: 0-3 plausible alternative classes from
  the catalog if confidence is medium/low.

# User
Sentence: "{{ sentence }}"

Classification result:
{{ classification_result_json }}

Class definitions (if available):
{{ class_definitions_json }}

Produce the JSON explanation.
```

### 8.5 Output validation

```python
# mcp-server/mcp_server/tools/m20_validation.py

import json
import re
from pydantic import ValidationError
from .types import ToolError
from efe_agent.types import ExplainClassificationOutput


async def validate_explain_classification_output(
    raw_text: str,
    max_retries: int,
    retry_callback,
) -> ExplainClassificationOutput:
    last_err = None
    for attempt in range(max_retries + 1):
        try:
            cleaned = _strip_markdown_fences(raw_text)
            parsed = json.loads(cleaned)
            return ExplainClassificationOutput(**parsed)
        except (json.JSONDecodeError, ValidationError) as e:
            last_err = e
            if attempt < max_retries:
                raw_text = await retry_callback([str(e)])
                continue
    raise ToolError(
        f"LLM output failed validation after {max_retries + 1} attempts",
        code="validation_failed",
        detail=str(last_err),
    )


def _strip_markdown_fences(text: str) -> str:
    """Remove ```json ... ``` if present."""
    m = re.match(r"```(?:json)?\s*(.*?)\s*```\s*$", text.strip(), re.DOTALL)
    return m.group(1) if m else text.strip()
```

### 8.6 Examples

**Input** :
```json
{
  "catalog": "support-routing",
  "version": "v1.2.0",
  "sentence": "My laptop won't boot after the update",
  "classification_result": {
    "label": "hardware_issue",
    "confidence": 0.87,
    "anchors_used": ["hardware", "boot", "device"]
  }
}
```

**Output** :
```json
{
  "explanation": "Le ticket décrit un problème de démarrage matériel après une mise à jour, ce qui correspond fortement à la classe 'hardware_issue' via les ancres 'hardware' et 'boot'. Cependant, 'after the update' suggère un possible 'software_regression' qui n'a pas été déclenché.",
  "key_factors": ["hardware-related verb 'boot'", "physical device 'laptop'", "post-update timing"],
  "confidence_assessment": "high",
  "suggested_alternatives": ["software_regression"],
  "metadata": {
    "tokens_in": 412, "tokens_out": 156,
    "model_used": "claude-sonnet-4-5", "language": "fr"
  }
}
```

### 8.7 Pin tests (7)

- `test_explain_classification_basic_path`
- `test_explain_classification_validates_output_schema`
- `test_explain_classification_retries_on_invalid_json`
- `test_explain_classification_fails_after_max_retries`
- `test_explain_classification_includes_class_definitions_when_available`
- `test_explain_classification_emits_audit_event`
- `test_explain_classification_blocks_when_feature_disabled`

---

## 9. Tool 2 — `efe_compare_bundles_quality`

### 9.1 Mission

Comparer la qualité de classification de **deux bundles** sur un
échantillon de sentences, pour aider l'opérateur à décider de la
promotion. Le tool :
1. Récupère les manifests des deux bundles via resources.
2. Si `test_sentences` non fourni, sample N sentences depuis le gold
   dataset du catalog.
3. Run classify Mode-1 sur les deux bundles.
4. Demande au LLM de comparer les divergences et faire une
   recommendation.

### 9.2 INPUT_SCHEMA

```python
INPUT_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCompareBundlesQualityInput",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version_a": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+$"},
        "version_b": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+$"},
        "test_sentences": {
            "type": "array",
            "items": {"type": "string", "maxLength": 4096},
            "minItems": 1,
            "maxItems": 200,
        },
        "sample_size": {
            "type": "integer",
            "minimum": 10,
            "maximum": 500,
            "default": 100,
        },
    },
    "required": ["catalog", "version_a", "version_b"],
    "additionalProperties": False,
    "anyOf": [
        {"required": ["test_sentences"]},
        {"required": ["sample_size"]},
    ],
}
```

### 9.3 Handler skeleton

```python
async def handler(args, ctx):
    if not ctx.feature_enabled("m20_helpers"):
        raise ToolError("...", code="feature_disabled")
    
    catalog = args["catalog"]
    va, vb = args["version_a"], args["version_b"]
    
    # 1. Get test sentences
    sentences = args.get("test_sentences")
    if sentences is None:
        sample_size = args.get("sample_size", 100)
        sentences = await ctx.backend.sample_gold_sentences(
            catalog=catalog, n=sample_size, auth=ctx.auth,
        )
    
    # 2. Classify on both versions
    results_a = await ctx.backend.classify_batch(
        catalog=catalog, version=va, sentences=sentences, auth=ctx.auth,
    )
    results_b = await ctx.backend.classify_batch(
        catalog=catalog, version=vb, sentences=sentences, auth=ctx.auth,
    )
    
    # 3. Compute divergences
    divergences = []
    for i, s in enumerate(sentences):
        if results_a[i].label != results_b[i].label:
            divergences.append({
                "sentence": s,
                "label_a": results_a[i].label,
                "confidence_a": results_a[i].confidence,
                "label_b": results_b[i].label,
                "confidence_b": results_b[i].confidence,
            })
    
    # 4. Build prompt
    system_prompt, user_prompt = render_prompt(
        "compare_bundles_quality_v1",
        catalog=catalog,
        version_a=va, version_b=vb,
        n_total=len(sentences),
        n_divergent=len(divergences),
        divergences_json=json.dumps(divergences[:50]),
        # truncate to 50 in prompt to avoid context blowup
    )
    
    # 5. Sampling
    sampling_resp = await ctx.sampling.create_message(
        system_prompt=system_prompt,
        messages=[{"role": "user", "content": user_prompt}],
        max_tokens=1500,
        temperature=0.0,
    )
    
    # 6. Validate
    output = await validate_compare_bundles_output(
        raw_text=sampling_resp.content,
        max_retries=2,
    )
    
    return {
        "n_sentences_compared": len(sentences),
        "n_divergent": len(divergences),
        "accuracy_delta_pp": output.accuracy_delta_pp,
        "divergence_examples": output.divergence_examples,
        "summary": output.summary,
        "recommendation": output.recommendation,
        "metadata": {...},
    }
```

### 9.4 Prompt template (excerpt)

```markdown
# System
You are a quality assurance expert for text classification systems.
Compare two bundle versions of a classification model, focusing on
divergent classifications and their implications.

Rules:
- Output JSON: { accuracy_delta_pp, divergence_examples, summary, recommendation }
- recommendation must be exactly one of: "promote" | "investigate" | "do_not_promote"
- Be conservative: if any divergence appears systematic or concerning, prefer "investigate" over "promote"

# User
Catalog: {{ catalog }}
Version A (current): {{ version_a }}
Version B (candidate): {{ version_b }}

Total sentences compared: {{ n_total }}
Divergent classifications: {{ n_divergent }}

Sample of divergences:
{{ divergences_json }}

Produce the JSON comparison.
```

### 9.5 Pin tests (7)

- `test_compare_bundles_basic`
- `test_compare_bundles_no_divergence_recommends_promote`
- `test_compare_bundles_systematic_divergence_recommends_investigate`
- `test_compare_bundles_uses_provided_test_sentences`
- `test_compare_bundles_samples_from_gold_when_no_sentences`
- `test_compare_bundles_validates_recommendation_enum`
- `test_compare_bundles_blocks_when_feature_disabled`

---

## 10. Tool 3 — `efe_suggest_test_sentences`

### 10.1 Mission

Générer N sentences synthétiques pour augmenter le coverage d'une
classe spécifique dans un catalog. Use case : opérateur a class
`payment_dispute` avec 8 sentences gold, veut booster à 64
(D12 cap). LLM génère des candidats, opérateur valide.

### 10.2 INPUT_SCHEMA

```python
INPUT_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSuggestTestSentencesInput",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+$"},
        "target_class": {"type": "string", "minLength": 1, "maxLength": 100},
        "count": {"type": "integer", "minimum": 1, "maximum": 64, "default": 16},
        "difficulty": {
            "type": "string",
            "enum": ["easy", "medium", "hard", "edge", "mixed"],
            "default": "mixed",
        },
        "language": {
            "type": "string",
            "enum": ["en", "fr", "de", "es", "pt", "it"],
            "default": "en",
        },
        "avoid_redundancy_with_existing": {
            "type": "boolean",
            "default": True,
        },
    },
    "required": ["catalog", "target_class"],
    "additionalProperties": False,
}
```

### 10.3 Handler highlights

```python
async def handler(args, ctx):
    # ... feature gate ...
    
    # Fetch existing sentences for context (if avoid_redundancy)
    existing = []
    if args.get("avoid_redundancy_with_existing", True):
        existing = await ctx.backend.get_class_sentences(
            catalog=args["catalog"],
            version=args.get("version"),
            class_name=args["target_class"],
            auth=ctx.auth,
        )
    
    # Render prompt with existing sentences (anti-redundancy)
    system_prompt, user_prompt = render_prompt(
        "suggest_test_sentences_v1",
        catalog=args["catalog"],
        target_class=args["target_class"],
        count=args.get("count", 16),
        difficulty=args.get("difficulty", "mixed"),
        language=args.get("language", "en"),
        existing_sentences_json=json.dumps(existing[:30]),
    )
    
    sampling_resp = await ctx.sampling.create_message(...)
    output = await validate_suggest_test_sentences_output(...)
    
    return {
        "sentences": output.sentences,
        "difficulty_self_assessment": output.difficulty_self_assessment,
        "rationale": output.rationale,
    }
```

### 10.4 Prompt template (excerpt)

```markdown
# System
You generate synthetic test sentences for the class "{{ target_class }}"
in the catalog "{{ catalog }}". You produce sentences that:
- Are realistic — could be encountered in production.
- Cover the requested difficulty level.
- Avoid redundancy with existing sentences.
- Are written in {{ language }}.

Rules:
- Output JSON: { sentences: [...], difficulty_self_assessment: {sentence: difficulty}, rationale }
- Generate exactly {{ count }} sentences.
- Each sentence ≤ 256 chars.
- Difficulty levels:
  * easy: clear, unambiguous, common phrasing
  * medium: typical real-world phrasing with some context
  * hard: ambiguous, requires nuance, possible competing classes
  * edge: rare, adversarial, or boundary cases
- Mixed: 25% easy, 50% medium, 20% hard, 5% edge.

# User
Existing sentences (avoid these):
{{ existing_sentences_json }}

Generate {{ count }} new test sentences.
```

### 10.5 Pin tests (7)

- `test_suggest_sentences_basic`
- `test_suggest_sentences_count_respected`
- `test_suggest_sentences_difficulty_distribution_mixed`
- `test_suggest_sentences_avoids_redundancy_with_existing`
- `test_suggest_sentences_language_respected`
- `test_suggest_sentences_validates_output_schema`
- `test_suggest_sentences_blocks_when_feature_disabled`

---

## 11. Tool 4 — `efe_analyze_confusion_matrix`

### 11.1 Mission

Donné une confusion matrix EFE (statistiques agrégées), demander
au LLM une analyse sémantique des paires de classes les plus
confondues + propositions d'actions (split, merge, reword).

### 11.2 INPUT_SCHEMA

```python
INPUT_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeAnalyzeConfusionMatrixInput",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+$"},
        "confusion_matrix": {
            "type": "object",
            "description": "Map of true_class → {predicted_class → count}",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": {"type": "integer", "minimum": 0},
            },
        },
        "min_confusion_rate": {
            "type": "number",
            "minimum": 0.0,
            "maximum": 1.0,
            "default": 0.05,
            "description": "Only analyze pairs with rate ≥ this.",
        },
        "max_pairs": {
            "type": "integer",
            "minimum": 1,
            "maximum": 20,
            "default": 5,
        },
    },
    "required": ["catalog", "confusion_matrix"],
    "additionalProperties": False,
}
```

### 11.3 Pin tests (7)

- `test_analyze_confusion_basic`
- `test_analyze_confusion_filters_below_min_rate`
- `test_analyze_confusion_caps_at_max_pairs`
- `test_analyze_confusion_proposes_splits_for_internal_diversity`
- `test_analyze_confusion_proposes_merges_for_low_distinction`
- `test_analyze_confusion_validates_output_schema`
- `test_analyze_confusion_blocks_when_feature_disabled`

---

## 12. Tool 5 — `efe_draft_runbook`

### 12.1 Mission

Donné un événement d'incident (alerte SLO M19, log lines, contexte
metric), produire un draft de runbook markdown. Use case : SRE
on-call reçoit `M19-OBS-3` log volume breach, n'a pas de runbook
préparé. Helper produit un draft à raffiner.

### 12.2 INPUT_SCHEMA

```python
INPUT_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDraftRunbookInput",
    "properties": {
        "incident_event": {
            "type": "object",
            "properties": {
                "slo_id": {"type": "string", "maxLength": 50},
                "trigger_metric": {"type": "string", "maxLength": 200},
                "current_value": {"type": "number"},
                "threshold": {"type": "number"},
                "since": {"type": "string", "format": "date-time"},
            },
            "required": ["slo_id"],
        },
        "related_logs": {
            "type": "array",
            "items": {"type": "string", "maxLength": 1024},
            "maxItems": 20,
        },
        "context_metadata": {
            "type": "object",
            "description": "Free-form context (cluster_id, region, env)",
            "additionalProperties": {"type": "string"},
        },
    },
    "required": ["incident_event"],
    "additionalProperties": False,
}
```

### 12.3 Pin tests (7)

- `test_draft_runbook_basic`
- `test_draft_runbook_includes_diagnostics`
- `test_draft_runbook_estimates_resolution_time`
- `test_draft_runbook_includes_logs_when_provided`
- `test_draft_runbook_assesses_risk_level`
- `test_draft_runbook_validates_output_schema`
- `test_draft_runbook_blocks_when_feature_disabled`

---

## 13. Sampling delegation deep-dive

### 13.1 MCP sampling protocol

MCP standard sampling RPC :

```json
// Request from mcp-server to client
{
  "jsonrpc": "2.0",
  "id": "...",
  "method": "sampling/createMessage",
  "params": {
    "messages": [...],
    "systemPrompt": "...",
    "maxTokens": 1024,
    "temperature": 0.0,
    "modelPreferences": {
      "intelligencePriority": 0.7,
      "speedPriority": 0.3,
      "costPriority": 0.0
    }
  }
}

// Response
{
  "jsonrpc": "2.0",
  "id": "...",
  "result": {
    "model": "claude-sonnet-4-5",
    "stopReason": "end_turn",
    "role": "assistant",
    "content": {"type": "text", "text": "..."}
  }
}
```

### 13.2 Context selection (preferences hint)

Le mcp-server peut hinter le client (qui choisit le modèle) sur les
priorités :
- `intelligence_priority` : 0.0-1.0, importance qualité.
- `speed_priority` : 0.0-1.0, importance latence.
- `cost_priority` : 0.0-1.0, importance coût.

Convention M20 par tool :

| Tool | Intelligence | Speed | Cost |
|---|:---:|:---:|:---:|
| `efe_explain_classification` | 0.7 | 0.3 | 0.0 |
| `efe_compare_bundles_quality` | 0.9 | 0.0 | 0.1 |
| `efe_suggest_test_sentences` | 0.6 | 0.4 | 0.0 |
| `efe_analyze_confusion_matrix` | 0.9 | 0.1 | 0.0 |
| `efe_draft_runbook` | 0.5 | 0.5 | 0.0 |

### 13.3 Output validation safety

- **Pydantic models** strict pour chaque helper output.
- **Max retries** : 2 par défaut. Au-delà → `validation_failed` ToolError.
- **Retry prompt** : ré-injecte les erreurs Pydantic dans un suivi
  prompt: « Your previous output failed validation: [errors]. Please
  produce a valid response matching the schema. »
- **JSON cleanup** : strip `\`\`\`json fences\`\`\`` et trailing
  whitespace.

### 13.4 Privacy — no tenant data echoed

Convention :
- **`tenant_id`** jamais dans les prompts (utiliser hash si nécessaire).
- **PII regexes** (emails, phones) masqués via M14 carry-over avant
  inclusion prompt.
- **Idempotency_key** jamais dans prompts.
- **Audit ratio_len** seulement, pas le contenu free-form.

Implementation : helper `_redact_for_prompt(s: str) -> str` appelé
sur tous les inputs user-supplied avant template render.

---

## 14. Direct provider mode

### 14.1 When to use

| Use case | Sampling delegation | Direct provider |
|---|:---:|:---:|
| Operator using Claude Desktop | ✅ default | ❌ |
| Operator using Cursor | ✅ default | ❌ |
| CI batch eval (no client) | ❌ | ✅ |
| Python script automation | ❌ | ✅ |
| Self-hosted vLLM cluster | possible | ✅ |
| Cost-controlled per-team | possible | ✅ |

### 14.2 Provider config flow

```python
# CI batch script
import os
import asyncio
from efe_agent import Agent
from efe_agent.llm import (
    AnthropicProvider, OpenAIProvider, FallbackChain,
)


async def main():
    primary = AnthropicProvider(model="claude-sonnet-4-5")
    fallback = OpenAIProvider(model="gpt-5-mini")
    
    chain = FallbackChain(
        primary=primary,
        fallbacks=[fallback],
        per_provider_timeout_s=30.0,
    )
    
    agent = Agent(
        mcp_endpoint=os.environ["EFE_MCP_ENDPOINT"],
        oidc_token=os.environ["EFE_OIDC_TOKEN"],
        llm_provider=chain,
    )
    
    # Helper invocation in direct mode
    result = await agent.helper(
        "compare_bundles_quality",
        catalog="support-routing",
        version_a="v1.2.0",
        version_b="v1.2.1",
        sample_size=200,
    )
    print(f"Recommendation: {result.recommendation}")
    print(f"Accuracy delta: {result.accuracy_delta_pp:+.2f} pp")
    
    await agent.aclose()


asyncio.run(main())
```

### 14.3 Cost tracking client-side

```python
agent = Agent(...)
result = await agent.helper("explain_classification", ...)
print(f"Cost: ${result.metadata['cost_usd']:.4f}")
print(f"Tokens: in={result.metadata['tokens_in']}, out={result.metadata['tokens_out']}")
```

Cumulé sur N calls via `agent.cost_tracker.total_usd`.

---

## 15. Activation flags M20

### 15.1 Default state

**OFF par défaut**. Aucun helper enregistré sans flag explicite.

### 15.2 Server-level flags

| Flag | Default | Effect when OFF |
|---|:---:|---|
| `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` | `false` | 5 tools non enregistrés au boot dispatcher ; non listés dans `list_tools` ; appel direct → `tool_disabled` 403. |
| `EFE_FEATURE_M20_AGENT_SDK_BUNDLED` | `false` | `efe-agent` lib non incluse dans Docker image ; opérateur fait `pip install efe-agent` séparément. |

### 15.3 Tenant-level flag

Stocké dans Postgres `tenant_feature_flags` (cf. roadmap § F.3.3) :

```sql
INSERT INTO tenant_feature_flags (tenant_id, milestone, enabled, config_jsonb)
VALUES ('acme-corp', 'm20_helpers', true, '{}'::jsonb);
```

Flag tenant ON sans server-level ON ⇒ feature reste OFF (AND logic).

### 15.4 Test pin (default-off)

- `test_default_off_m20_no_helper_tools_listed`
- `test_default_off_m20_helper_call_returns_tool_disabled`
- `test_default_off_m20_pip_efe_agent_not_in_docker`
- `test_m20_requires_both_server_and_tenant_flag`

---

## 16. Threat model M20 (delta vs M19)

### 16.1 Vue d'ensemble

M20 ajoute **0 nouvelle menace HIGH/CRITICAL** (le sampling
delegation = pas de surface attaque server-side LLM). Quelques
menaces LOW à MED :

| ID | STRIDE | Threat | Sev | Mitigation |
|---|:---:|---|:---:|---|
| TM-M20-T-01 | Tamper | LLM produces malicious output (hallucination → wrong recommendation) | LOW | Pydantic validation + retry + recommendation enum |
| TM-M20-I-01 | InfoDisc | Prompt template leaks taxonomy structure to LLM provider | LOW | Tenant accepts via opt-in ; Anthropic/OpenAI no-training |
| TM-M20-I-02 | InfoDisc | PII in sentence echoed in prompt | MED | `_redact_for_prompt` regex M14 carry-over |
| TM-M20-D-01 | DoS | Helper called repeatedly without cost control | MED | Rate limit per-tenant (M19 carry-over weights) |

### 16.2 Cumulative

- M16-M19 cumulé : 37
- M20 delta : +4
- **Total post-M20 (si activé)** : 41

Si M20 OFF : 37 (M19 baseline).

---

## 17. Performance & SLOs M20

### 17.1 SLOs

| ID | SLI | Target |
|---|---|---|
| M20-LAT-1 | Helper p50 latency | < 500 ms (excluding sampling round-trip) |
| M20-LAT-2 | Helper p99 latency | < 1.5 s (excluding sampling round-trip) |
| M20-VAL-1 | Output validation success rate | ≥ 90 % first attempt |
| M20-VAL-2 | Validation failure rate after retries | ≤ 1 % |
| M20-AGT-1 | Agent SDK availability | n/a (client-side) |
| M20-PIN | Pin tests | +35 GREEN |

---

## 18. Compatibility matrix

### 18.1 Python versions

`efe-agent` supporte Python 3.11, 3.12, 3.13. CI matrix sur les trois.

### 18.2 mcp-server versions

mcp-server M20 (= mcp-server v0.20.0) requires :
- `mcp-server` ≥ 0.20.0
- Python ≥ 3.11
- Backend EFE 1.3 (M19 GA, no bump)

### 18.3 Provider SDK versions

| Provider | SDK version min |
|---|---|
| Anthropic | anthropic ≥ 0.45.0 |
| OpenAI | openai ≥ 1.50.0 |
| AWS Bedrock | boto3 ≥ 1.35.0 |
| Azure OpenAI | openai ≥ 1.50.0 + azure-identity ≥ 1.18.0 |
| Vertex AI | google-cloud-aiplatform ≥ 1.70.0 |
| MLX | mlx-lm ≥ 0.20.0 |

### 18.4 OS

- Linux (Ubuntu 22.04+, Debian 12+, RHEL 9+) : ✅ all features.
- macOS (Sequoia, Tahoe — M-series) : ✅ all features incl. MLX.
- Windows : ✅ except MLX.

---

## 19. Migration M19 → M20

### 19.1 No-action default

Un cluster mcp-server M19 GA upgradé vers M20 **sans flag activé** :
- Image Docker M20 build → identique en taille (extras non installés).
- Boot logs : aucun message M20 (helpers non enregistrés).
- `list_tools` output : 34 tools (M19 baseline), pas 39.
- Client behaviour : identique.
- Test : `test_default_off_compat.py::test_m19_to_m20_no_action_diff` passe.

### 19.2 Activation incrémentale

```bash
# Étape 1 : server-level
EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=true
# (redéploie / restart mcp-server)

# Étape 2 : tenant-level via mcp tool
mcp_tool(
    "efe_set_tenant_feature",
    {"milestone": "m20_helpers", "enabled": true, "confirm": true}
)

# Étape 3 : test
mcp_tool("efe_explain_classification", {...})
```

### 19.3 Rollback

```bash
EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=false
# restart → tools disparaissent
```

Données : aucune. M20 ne stocke rien côté EFE (sampling delegation).

---

## 20. Test plan

### 20.1 Pin tests inventory (~35 nouveaux)

| File | Tests | Sprint |
|---|---|---|
| `test_efe_explain_classification.py` | 7 | D55 |
| `test_efe_compare_bundles_quality.py` | 7 | D56 |
| `test_efe_suggest_test_sentences.py` | 7 | D56 |
| `test_efe_analyze_confusion_matrix.py` | 7 | D57 |
| `test_efe_draft_runbook.py` | 7 | D57 |
| `test_default_off_m20.py` | 4 | D55 |
| `test_audit_pass_m20.py` (post-livraison) | ~12 | D58 |

Total : **~51 nouveaux tests** (35 livraison + 16 audit).

### 20.2 Conditional tests

`tests/clients_python/test_provider_*.py` skipped sans creds :
- `test_provider_anthropic.py` (skipped si pas `ANTHROPIC_API_KEY`)
- `test_provider_openai.py` (skipped si pas `OPENAI_API_KEY`)
- `test_provider_bedrock.py` (skipped si pas AWS auth)
- (etc.)

CI conditionnelle dans GitHub Actions matrix.

### 20.3 Integration tests

- `test_e2e_anthropic_explain.py` : real Anthropic API, rate-limited.
- `test_e2e_vllm_local.py` : spin up vLLM in CI, run all helpers.

### 20.4 Audit pass M20

Pattern récurrent (M11-M19, ~6-8 défauts attendus). Phase D58.
Targets attendus : 1 MED (probablement prompt template variable
escaping), 3-5 LOW.

---

## 21. Deployment scenarios

### 21.1 Operator workstation (Claude Desktop)

```bash
# .claude/mcp_servers.json
{
  "efe": {
    "command": "/path/to/efe-mcp-server",
    "env": {
      "EFE_OIDC_TOKEN": "...",
      "EFE_ENDPOINT": "https://efe.example.com",
      "EFE_MCP_TOOL_PROFILE": "agent-native",
      "EFE_FEATURE_M20_HELPER_TOOLS_ENABLED": "true"
    }
  }
}
```

### 21.2 CI batch eval

```bash
# .github/workflows/bundle_eval.yml
- name: Compare bundles
  env:
    EFE_OIDC_TOKEN: ${{ secrets.EFE_TOKEN }}
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_KEY }}
  run: |
    pip install efe-agent[anthropic]
    efe-agent helper compare_bundles_quality \
      --catalog support-routing \
      --version-a v1.2.0 \
      --version-b v1.2.1 \
      --sample-size 200
```

### 21.3 Self-hosted vLLM

```bash
# Helm values for self-host vLLM cluster (operator side)
helm install vllm-cluster vllm/vllm \
  --set model=meta-llama/Llama-4-70B-Instruct \
  --set replicas=2

# Then in CI:
pip install efe-agent[vllm]
EFE_LLM_PROVIDER=vllm \
EFE_VLLM_URL=http://vllm-cluster:8000/v1 \
efe-agent helper explain_classification ...
```

### 21.4 Local llama.cpp

```bash
# Operator workstation, no API key:
./llama-server -m models/llama-4-70b-q4.gguf -c 8192 --port 8080

# Then:
pip install efe-agent[llama-cpp]
EFE_LLM_PROVIDER=llama-cpp \
EFE_LLAMA_CPP_URL=http://localhost:8080/v1 \
efe-agent helper draft_runbook --slo-id M19-OBS-3
```

---

## 22. Open questions

### 22.1 Should M20 helpers also be exposed via the `efe-agent` Python lib's CLI?

**Recommendation** : Yes, via `efe-agent helper <name> --json-input`.
Permits non-Python operators to use them.

### 22.2 Should we cache sampling outputs server-side?

**Recommendation** : NO in M20. Sampling = client-side, server has
no visibility. Caching would need M21 LLMRouter.

### 22.3 Should we support custom prompt templates per tenant?

**Recommendation** : NOT in M20. Templates are versioned in code,
proven via eval. Custom templates risk poor outputs.
Reconsider in M23 (curation milestone).

### 22.4 Which design partner pilots first?

**Recommendation** : 2-3 mid-tier customers (Pro plan, < 1k tickets/day)
to validate ROI before broader rollout. Internal team first (dogfood).

### 22.5 Streaming support for `efe_draft_runbook`?

**Recommendation** : NOT in M20 (simpler validation pipeline). Future
if SRE team strongly demands.

---

## 23. FAQ

**Q : EFE paye-t-il quelque chose en LLM tokens en M20 ?**
A : Non. Sampling delegation = client paye. Direct provider mode = client paye aussi (BYO).

**Q : Si je n'active pas M20, mon cluster est-il affecté ?**
A : Non. Bit-for-bit M19 GA. Les 5 tools n'apparaissent pas.

**Q : Quel provider recommandez-vous par défaut ?**
A : Pour qualité : Anthropic Claude Sonnet 4.5 ou GPT-5. Pour coût/local : llama.cpp ou vLLM Llama 4.

**Q : `efe-agent` lib est-elle requise ?**
A : Non. Les 5 tools fonctionnent via MCP sampling sans la lib. La lib est pour les use cases hors-MCP (CI, scripts).

**Q : Compatible avec EFE 1.2 backend ?**
A : Oui. M20 ne change pas le contract backend. EFE 1.2 et 1.3 supportés.

**Q : EU AI Act applies à M20 ?**
A : Non, le LLM est côté client. Le tenant assume sa propre conformité provider.

---

## 24. Glossary M20

- **BYO-LLM** : Bring Your Own LLM. Le client choisit son provider
  et paye ses tokens.
- **Helper tool** : tool MCP read-only qui aide l'opérateur (vs
  tool runtime qui sert le classify production).
- **Sampling delegation** : pattern MCP où le LLM call est exécuté
  côté client, pas côté server.
- **Direct provider mode** : pattern `efe-agent` lib où le LLM call
  est exécuté localement par la lib Python.
- **Fallback chain** : multi-provider failover avec circuit breaker.
- **Output validation** : Pydantic schema check on LLM response.
- **Anti-hallucination** : retry on validation failure with error
  feedback.

---

## 25. References

- **MCP specification (sampling)** :
  <https://modelcontextprotocol.io/specification#sampling>
- **Anthropic Python SDK** :
  <https://github.com/anthropics/anthropic-sdk-python>
- **OpenAI Python SDK** :
  <https://github.com/openai/openai-python>
- **AWS Bedrock Runtime** : boto3 docs.
- **vLLM OpenAI-compat server** : <https://docs.vllm.ai>
- **llama.cpp HTTP server** :
  <https://github.com/ggml-org/llama.cpp>
- **Apple MLX** : <https://github.com/ml-explore/mlx-lm>
- **Pydantic v2** : <https://docs.pydantic.dev>
- **Tenacity** (retry library) : <https://tenacity.readthedocs.io>
- **EFE M19 architecture** : `EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`.
- **EFE LLM roadmap** : `EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`.

---

## A — Full JSON Schemas

### A.1 `efe_explain_classification`

(See § 8.2 above.)

### A.2 `efe_compare_bundles_quality`

(See § 9.2 above.)

### A.3 `efe_suggest_test_sentences`

(See § 10.2 above.)

### A.4 `efe_analyze_confusion_matrix`

(See § 11.2 above.)

### A.5 `efe_draft_runbook`

(See § 12.2 above.)

---

## B — Provider integration details

### B.1 Anthropic

- API endpoint : `https://api.anthropic.com/v1/messages`.
- Auth : `Authorization: Bearer <api-key>` OR via Anthropic SDK env.
- BAA : Enterprise plan required (carry-over roadmap § A).
- EU residency : not yet (US-only).
- No-training : default true.
- Latency p50 : 300-800 ms.

### B.2 OpenAI

- API endpoint : `https://api.openai.com/v1/chat/completions`.
- Auth : `Authorization: Bearer <api-key>`.
- BAA : Enterprise plan required.
- No-training : Enterprise default true ; Standard requires opt-out.
- Latency p50 : 200-600 ms.

### B.3 AWS Bedrock

- Endpoint : `https://bedrock-runtime.<region>.amazonaws.com`.
- Auth : IRSA / IAM role / static (carry-over M19 BYOK).
- BAA : ✅ AWS-wide.
- Models 2026 : Claude 4 family + Llama 4 + Titan.
- Latency p50 : 350-900 ms.

### B.4 Azure OpenAI

- Endpoint : `https://<resource>.openai.azure.com/openai/deployments/<dep>/chat/completions`.
- Auth : Managed Identity / SP / API key.
- BAA : ✅ Azure-wide.
- No-training : default true.
- Latency p50 : 250-700 ms.

### B.5 Vertex AI

- Endpoint : Vertex AI predict / generateContent.
- Auth : Workload Identity / SA / static.
- BAA : ✅ GCP-wide.
- Models : Gemini 2.5 Pro / Flash, Claude (via partner).
- Latency p50 : 200-700 ms.

### B.6 vLLM

- OpenAI-compatible HTTP API.
- Self-host on H100/A100 cluster.
- Cost : GPU rental (~$3-5/h H100).
- Latency p50 : 80-100 ms (Llama 4 70B Q5 on 1 H100).

### B.7 llama.cpp

- HTTP server with OpenAI-compat mode.
- Local CPU/GPU, supports Mac M-series, RTX, etc.
- Cost : $0 (existing hardware).
- Latency p50 : 400-800 ms (depends on model + hardware).

### B.8 MLX

- Apple Silicon native (M1-M4).
- Python lib `mlx-lm`.
- Cost : $0.
- Latency p50 : 600-800 ms (Llama 4 70B Q4 on M3 Max).

---

## C — Agent loop sequence diagrams

### C.1 Sampling delegation flow

```
operator       mcp-client      mcp-server    LLM (client-side)    backend
    │              │               │                 │                │
    │ "explain..." │               │                 │                │
    │ ──────────►  │               │                 │                │
    │              │ tool/call     │                 │                │
    │              │ ────────────► │                 │                │
    │              │               │ get_class_def   │                │
    │              │               │ ──────────────────────────────►  │
    │              │               │ ◄──────────────────────────────  │
    │              │               │ render prompt   │                │
    │              │               │ sampling/createMessage           │
    │              │ ◄──────────── │                 │                │
    │              │ ──────────────────────────────► │                │
    │              │               │                 │ (LLM exec)     │
    │              │ ◄──────────────────────────────                  │
    │              │ ────────────► │                 │                │
    │              │               │ validate JSON   │                │
    │              │               │ audit emit      │                │
    │              │ ◄──────────── │                 │                │
    │ ◄──────────  │               │                 │                │
```

### C.2 Direct provider flow (Python script)

```
script          Agent           LLM provider            mcp-server      backend
   │             │                  │                       │              │
   │ helper(...) │                  │                       │              │
   │ ───────────►│                  │                       │              │
   │             │ get_class_def    │                       │              │
   │             │ ─────────────────────────────────────►   │              │
   │             │                  │                       │ ──────────►  │
   │             │                  │                       │ ◄──────────  │
   │             │ ◄─────────────────────────────────────   │              │
   │             │ render prompt    │                       │              │
   │             │ provider.complete()                      │              │
   │             │ ────────────────►│                       │              │
   │             │                  │ (LLM exec)            │              │
   │             │ ◄────────────────│                       │              │
   │             │ validate JSON    │                       │              │
   │             │ optional audit emit                      │              │
   │             │ ─────────────────────────────────────►   │              │
   │ ◄───────────│                  │                       │              │
```

### C.3 Fallback chain on primary failure

```
Agent           Anthropic        OpenAI         vLLM
  │                │                │             │
  │ complete()     │                │             │
  │ ─────────────► │                │             │
  │   (timeout)    │                │             │
  │ ─────────────► │                │             │
  │      X         │                │             │
  │ ◄───────────── │                │             │
  │                │                │             │
  │ (circuit-open Anthropic)        │             │
  │                │                │             │
  │ complete()     │                │             │
  │ ──────────────────────────────► │             │
  │ ◄────────────────────────────── │             │
  │  ✓                              │             │
```

---

## D — Pin tests inventory M20

### D.1 `test_efe_explain_classification.py`

```python
@pytest.mark.asyncio
async def test_explain_classification_basic_path(
    agent_native_dispatcher_with_m20, fake_backend, fake_sampling,
):
    fake_sampling.set_response('{"explanation": "...", "key_factors": ["a"], '
                               '"confidence_assessment": "high", '
                               '"suggested_alternatives": []}')
    result = await agent_native_dispatcher_with_m20.dispatch(
        "efe_explain_classification",
        {
            "catalog": "support-routing",
            "sentence": "My laptop won't boot",
            "classification_result": {
                "label": "hardware_issue",
                "confidence": 0.87,
            },
        },
    )
    assert result["confidence_assessment"] == "high"
    assert "explanation" in result


@pytest.mark.asyncio
async def test_explain_classification_validates_output_schema(...):
    # Pydantic strict
    pass


@pytest.mark.asyncio
async def test_explain_classification_retries_on_invalid_json(...):
    # First sampling response broken JSON, second OK → retry succeeds
    pass


@pytest.mark.asyncio
async def test_explain_classification_fails_after_max_retries(...):
    # 3 broken responses → ValidationFailed
    pass


@pytest.mark.asyncio
async def test_explain_classification_includes_class_definitions(...):
    # When backend returns class defs, prompt includes them
    pass


@pytest.mark.asyncio
async def test_explain_classification_emits_audit_event(...):
    # Audit chain HMAC includes tool_name, tool_args sumary
    pass


@pytest.mark.asyncio
async def test_explain_classification_blocks_when_feature_disabled(...):
    # Without M20 flag, dispatch returns feature_disabled
    pass
```

### D.2 `test_efe_compare_bundles_quality.py`

(7 tests, similar structure.)

### D.3 `test_efe_suggest_test_sentences.py`

(7 tests, similar structure.)

### D.4 `test_efe_analyze_confusion_matrix.py`

(7 tests, similar structure.)

### D.5 `test_efe_draft_runbook.py`

(7 tests, similar structure.)

### D.6 `test_default_off_m20.py`

```python
def test_default_off_m20_no_helper_tools_listed(default_dispatcher):
    """Without M20 flag, list_tools doesn't include any M20 helper."""
    tools = default_dispatcher.list_tools()
    names = {t["name"] for t in tools}
    for helper in (
        "efe_explain_classification",
        "efe_compare_bundles_quality",
        "efe_suggest_test_sentences",
        "efe_analyze_confusion_matrix",
        "efe_draft_runbook",
    ):
        assert helper not in names


@pytest.mark.asyncio
async def test_default_off_m20_helper_call_returns_tool_disabled(...):
    """Direct invocation by name → tool_disabled."""
    pass


def test_default_off_m20_pip_efe_agent_not_in_docker():
    """pip show efe-agent in Docker image → not installed."""
    pass


def test_m20_requires_both_server_and_tenant_flag():
    """Server ON + tenant OFF → still OFF."""
    pass
```

### D.7 `test_audit_pass_m20.py`

(Filled post-audit ~D58, ~12 tests.)

---

## E — Operator workflow examples

### E.1 « Pourquoi cette classification ? »

```
Opérateur (via Claude Desktop) :
"Le ticket #42189 a été classifié comme 'hardware_issue' avec
confidence 0.87. Peux-tu m'expliquer pourquoi ?"

Claude :
[appelle efe_explain_classification(catalog="support-routing",
 sentence="My laptop won't boot after the update",
 classification_result={...})]

Réponse efe_explain_classification :
{
  "explanation": "Le ticket décrit un problème de démarrage matériel
  après une mise à jour. Les ancres 'hardware', 'boot', 'device'
  ont fortement contribué. La mention 'after the update' suggère
  une cause logicielle, mais le sujet principal reste matériel.",
  "key_factors": [
    "verbe physique 'boot'",
    "objet matériel 'laptop'",
    "contexte post-update"
  ],
  "confidence_assessment": "high",
  "suggested_alternatives": ["software_regression"]
}

Claude présente l'explication à l'opérateur, qui décide de garder
la classification.
```

### E.2 « Mon nouveau bundle est-il meilleur ? »

```python
# CI eval avant promotion
import asyncio
from efe_agent import Agent
from efe_agent.llm import AnthropicProvider

async def main():
    agent = Agent(
        mcp_endpoint=os.environ["EFE_MCP_ENDPOINT"],
        oidc_token=os.environ["EFE_OIDC_TOKEN"],
        llm_provider=AnthropicProvider(model="claude-sonnet-4-5"),
    )
    
    result = await agent.helper(
        "compare_bundles_quality",
        catalog="support-routing",
        version_a="v1.2.0",   # current
        version_b="v1.3.0",   # candidate
        sample_size=200,
    )
    
    print(f"Compared {result['n_sentences_compared']} sentences")
    print(f"Divergent: {result['n_divergent']}")
    print(f"Accuracy delta: {result['accuracy_delta_pp']:+.2f} pp")
    print(f"Recommendation: {result['recommendation']}")
    
    if result["recommendation"] == "promote":
        print("✅ Safe to promote")
        # Trigger M18 efe_promote_bundle
    elif result["recommendation"] == "investigate":
        print("⚠️  Investigation required:")
        for ex in result["divergence_examples"][:5]:
            print(f"  - {ex}")
    else:
        print("❌ Do not promote")
        sys.exit(1)

asyncio.run(main())
```

### E.3 « Booster les sentences de cette classe »

```
Opérateur :
"Ma classe 'payment_dispute' n'a que 8 sentences gold. Génère-moi
56 sentences pour atteindre la cap D12 de 64."

Claude :
[appelle efe_suggest_test_sentences(catalog="billing",
 target_class="payment_dispute", count=56, difficulty="mixed",
 avoid_redundancy_with_existing=true)]

Réponse :
{
  "sentences": [
    "I never authorized this charge on my credit card",
    "The amount billed doesn't match what was agreed",
    ...  // 56 total
  ],
  "difficulty_self_assessment": {
    "I never authorized this charge on my credit card": "easy",
    ...
  },
  "rationale": "Generated 56 sentences spanning easy (14),
  medium (28), hard (12), edge (2). Avoided phrasings already in
  the existing 8 sentences. Coverage includes credit card,
  bank transfer, subscription, refund cases."
}

Opérateur valide via efe_register_bundle (M16) le nouveau training set.
```

### E.4 « Analyse mes confusions »

```python
result = await agent.helper(
    "analyze_confusion_matrix",
    catalog="support-routing",
    confusion_matrix={
        "billing": {"billing": 95, "refund": 5},
        "refund": {"billing": 12, "refund": 88},
        "tech_support": {"tech_support": 82, "billing": 3, "refund": 15},
    },
    min_confusion_rate=0.05,
)

# Output:
# top_confusion_pairs: [
#   {pair: "tech_support → refund", rate: 0.15, ...},
#   {pair: "refund → billing", rate: 0.12, ...},
# ]
# proposed_actions: [
#   {action: "split", classes: ["tech_support"],
#    rationale: "tech_support has 15% leak to refund — likely
#                 contains 'refund my repair' style cases"},
#   {action: "merge", classes: ["billing", "refund"],
#    rationale: "12% bidirectional leak suggests fundamental overlap"},
# ]
```

### E.5 « Drafter ce runbook »

```python
# SRE on-call receives M19-OBS-3 alert
incident = {
    "slo_id": "M19-OBS-3",
    "trigger_metric": "log_volume_kbytes_per_minute",
    "current_value": 247,
    "threshold": 100,
    "since": "2026-08-15T14:22:11Z",
}
related_logs = [
    "ERROR ... bundle pull retried 3 times",
    "WARN ... oci registry latency p99 = 8.2s",
    # ...
]

result = await agent.helper(
    "draft_runbook",
    incident_event=incident,
    related_logs=related_logs,
    context_metadata={
        "cluster_id": "prod-us-east-1",
        "region": "us-east-1",
        "env": "production",
    },
)

# Save the draft :
Path("docs/runbooks/m19_obs3_log_volume_DRAFT.md").write_text(
    result["runbook_markdown"]
)
```

---

**FIN**

Document M20 architecture, **opt-in everywhere edition**. ~2 500
lignes / ~95 KB. Prêt à passer en livraison sprint D55 (kick-off
1 senior Python lead + 0.5 senior Python provider intégrations).

**Garanties cardinales** :
1. **Default OFF** : un cluster M20 sans aucun flag M20 activé est
   bit-for-bit M19 GA.
2. **Aucun changement backend Rust** : EFE 1.3 reste GA.
3. **Aucun coût LLM EFE-side** : sampling delegation = client paye.
4. **Compliance M19 préservée** : SOC 2 / ISO 27001 / GDPR Art. 32 /
   NIS2 / EU CRA inchangés.

Toute revue chirurgicale : ouvrir issue dans le repo et taguer
`[m20-architecture]`. Décisions § 22 doivent être adressées avant
kick-off D55.
