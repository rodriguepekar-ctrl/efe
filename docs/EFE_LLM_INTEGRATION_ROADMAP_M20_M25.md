# EFE × LLM — Roadmap d'intégration M20 → M25

> **Statut** : roadmap stratégique post-M19 GA (production hardening livré).
> Document de planification — **PAS** une architecture livrée. Chaque
> sprint M20-M25 fera l'objet d'un `EFE_M{XX}_*_ARCHITECTURE.md`
> dédié au moment du kick-off.
>
> **Auteur** : tech lead EFE.
> **Date** : 2026-05-02 (J+0 post-M19).
> **Horizon** : Q3 2026 → Q4 2027 (~18 mois).
> **Précision** : ultrathink. Précision chirurgicale suprême.
>
> **Pré-requis** : avoir lu :
> - [`PRODUCT_COMPLETION_ARCHITECTURE.md`](PRODUCT_COMPLETION_ARCHITECTURE.md)
>   — vision produit + Mode-1/2/3 terminology + AS-IS audit (D8-D18).
> - [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md)
>   — observability + cosign + BYOK + HTTP+SSE multi-replica (juste livré).
> - [`EFE_CORE_EXTENSION_ARCHITECTURE.md`](EFE_CORE_EXTENSION_ARCHITECTURE.md)
>   — backend Rust + multi-tenant + audit chain.
> - **EU AI Act 2024/1689** (entrée en vigueur progressive 2025-2027) :
>   <https://eur-lex.europa.eu/eli/reg/2024/1689/oj>
> - **GDPR Art. 22** (automated individual decision-making + profiling).
> - **NIST AI RMF 1.0** : <https://www.nist.gov/itl/ai-risk-management-framework>
>
> ---
>
> ## Mission de cette roadmap
>
> EFE est aujourd'hui (post-M19) un **moteur de classification sémantique
> production-grade** :
>
> - **Mode-1** (anchors compilés, current MVP) : embedder + swarm consensus,
>   p99 ≤ 200 ms GPU, ~0.5 ¢/classify, multi-tenant, bundles signés
>   Ed25519 + Sigstore, observabilité OTel, BYOK, HTTP+SSE multi-replica.
> - **MCP server** (M16-M19) : 34 tools exposant EFE aux LLM clients
>   externes (Claude Desktop, Cursor, agents custom), 11 resources,
>   10 prompts, 5 profile tiers.
> - **Mode-2 / Mode-3** : ❌ **manquants**. Documentés dans
>   `PRODUCT_COMPLETION_ARCHITECTURE.md` mais reportés (priorité GA
>   compliance EFE 1.0 → 1.3).
>
> Le gap : **EFE traite les LLMs comme des CLIENTS** (M16-M19 MCP). Mais
> le produit EFE ne *contient* aucun LLM. Pour atteindre l'ambition
> initiale (Mode-2 + Mode-3 + active learning + agent runtime), il faut
> **brancher les LLMs comme COMPOSANTS internes** — sous gouvernance,
> avec budget, multi-vendor, multi-tenant isolation, et compliance EU AI
> Act / GDPR Art. 22 explicite.
>
> Cette roadmap couvre **6 milestones (M20 → M25)** sur ~18 mois pour
> déplacer EFE d'un "classifier seul" vers une "plateforme classifier
> + LLM gouverné, multi-pattern".

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Contexte post-M19](#2-contexte-post-m19)
3. [Le paysage LLM 2026 — analyse](#3-le-paysage-llm-2026)
4. [Vision stratégique — 4 patterns d'intégration](#4-vision-stratégique)
5. [Roadmap M20→M25 (vue d'ensemble)](#5-roadmap-m20m25)
6. [M20 — BYO-LLM Agent SDK + helper tools](#6-m20--byo-llm-agent-sdk)
7. [M21 — Mode-2 (LLM-routed anchor generation)](#7-m21--mode-2)
8. [M22 — Mode-3 (multi-turn reasoning)](#8-m22--mode-3)
9. [M23 — LLM-assisted taxonomy curation](#9-m23--taxonomy-curation)
10. [M24 — Active learning loop](#10-m24--active-learning)
11. [M25 — Agent runtime (experimental)](#11-m25--agent-runtime)
12. [Cross-cutting — LLMRouter abstraction](#12-llmrouter-abstraction)
13. [Cross-cutting — Cost governance](#13-cost-governance)
14. [Cross-cutting — Privacy & compliance](#14-privacy--compliance)
15. [Cross-cutting — Multi-tenant isolation](#15-multi-tenant-isolation)
16. [Cross-cutting — Fallback patterns](#16-fallback-patterns)
17. [Cross-cutting — Vendor lock-in mitigation](#17-vendor-lock-in)
18. [Threat model M20+ (delta vs M19)](#18-threat-model-m20)
19. [Backend contract evolution (EFE 1.4 → 2.0)](#19-backend-contract-evolution)
20. [Risk register](#20-risk-register)
21. [Open questions / decisions needed](#21-open-questions)
22. [Success metrics](#22-success-metrics)
23. [Glossary M20+](#23-glossary-m20)
24. [References](#24-references)

**Annexes**

- [A — LLM provider matrix (10+ providers)](#a--llm-provider-matrix)
- [B — Cost model (concrete $/month estimates)](#b--cost-model)
- [C — Prompt library skeleton](#c--prompt-library-skeleton)
- [D — Pin tests inventory roadmap](#d--pin-tests-inventory-roadmap)
- [E — Compliance attestation chain](#e--compliance-attestation-chain)
- [F — Feature flag specification (opt-in everywhere)](#f--feature-flag-specification)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Brancher les LLMs comme **composants internes** d'EFE (au-delà du pattern M16-M19 où ils sont clients externes). 4 patterns : LLM-as-classifier, LLM-as-curator, LLM-as-helper, LLM-as-agent. |
| **Horizon** | 6 milestones (M20-M25), Q3 2026 → Q4 2027, ~18 mois. |
| **Investissement total** | ~$2.4 M (équipe + LLM costs production) sur 18 mois. |
| **Équipe pic** | 6 ingénieurs (2 Python, 2 Rust, 1 ML, 1 SRE) + 0.5 PM + 0.25 sécurité + 0.25 compliance + 0.25 doc. |
| **Volumétrie code total** | ~12 000 LOC Rust (Mode-2/3 backend) + ~8 000 LOC Python (mcp-server + agent SDK + curation) + ~6 000 LOC tests + ~1 500 LOC infra (terraform / helm). |
| **Pin tests cumulés post-M25** | ~900-1000 (vs ~520 post-M19 GA). |
| **Backward compat** | 100 % Mode-1 préservé. Mode-2/3 opt-in. Aucun breaking change forcé. EFE 1.x stable jusqu'à M25 ; **EFE 2.0** (semver MAJOR) seulement si M25 agent runtime livré. |
| **Activation** | **OPT-IN EVERYWHERE — DEFAULT OFF**. Chaque capability M20-M25 est gardée par un feature flag à 4 niveaux (server / tenant / catalog / request). Tant qu'aucun flag n'est activé, l'instance EFE est **bit-for-bit identique** à un EFE 1.3 (post-M19) GA : pas de LLM dans le critical path, pas de coût LLM, pas d'applicabilité AI Act, pas de service `efe-llm-router` déployé, pas de dépendance Python `[llm]` extras installée. Voir § 4.6 + § 12.bis + Annexe F pour la spécification complète des flags. |
| **Compliance** | EU AI Act 2024/1689 **n'est applicable QUE** si Mode-2 ou Mode-3 sont activés (M21+). Sinon EFE conserve la posture M19 (SOC 2 + ISO 27001 + GDPR Art. 32 + NIS2 + EU CRA), inchangée. GDPR Art. 22 (automated decision) human-review obligatoire pour Mode-3 si activé. NIS2 Art. 21 reste préservé. |
| **Risk profile** | **High** — chaque milestone introduit de nouvelles dépendances (LLM providers), de nouvelles surfaces compliance (AI Act), de nouveaux coûts opex (LLM tokens). Mitigation : opt-in everywhere, multi-provider abstraction, hard budget caps, kill-switch par tenant, audit pass post-livraison comme M11-M19. |
| **Strategic outcome** | EFE devient une **plateforme de classification sémantique LLM-augmented** ; concurrentielle vs OpenAI moderation, AWS Comprehend Custom, Azure AI Language. Différenciateurs : multi-tenant Day-1, audit chain HMAC, compliance EU/SOC2 par défaut, BYO-LLM (pas d'enfermement vendor), zero-shot (pas de fine-tuning requis). |

**TL;DR sequencing** :

| Milestone | Lettre EFE | Sprints | Trimestre | Cœur livraison |
|---|:---:|---|:---:|---|
| **M20** | F | D55-D58 | Q3 2026 | BYO-LLM agent SDK + LLM-helper mcp-server tools (no backend changes) |
| **M21** | G | D59-D64 | Q4 2026 | **Mode-2** — backend Rust LLM endpoint abstraction + anchor dynamics + LLMRouter + cost governance |
| **M22** | H | D65-D69 | Q1 2027 | **Mode-3** — multi-turn iterative refinement + concept stack |
| **M23** | I | D70-D73 | Q2 2027 | LLM-assisted taxonomy curation (operator workflows) |
| **M24** | J | D74-D79 | Q3 2027 | Active learning loop (production → gold labels → training) |
| **M25** | K | D80-D85 | Q4 2027 | Agent runtime experimental (sandboxed in-cluster agents) |

---

## 2. Contexte post-M19

### 2.1 Acquis M11-M19

EFE 1.3 GA (post-M19) inclut :

| Capability | Statut | Module / artefact |
|---|:---:|---|
| Mode-1 backend (anchors compilés) | ✅ Production | `crates/efe-server/src/mode1.rs` |
| gRPC server multi-tenant | ✅ | `crates/efe-server/services/swarm.rs` |
| Postgres catalog + audit chain HMAC | ✅ | M14, M14-bis |
| OIDC bearer auth (D15) | ✅ | `crates/efe-server/auth/` |
| Bundles Ed25519 + SBOM CycloneDX 1.5 + OCI | ✅ | M13 + M16-M19 |
| Sigstore Bundle v0.3 + Rekor inclusion proof | ✅ | M19 D52 |
| BYOK keystore (7 backends) | ✅ | M19 D53 |
| MCP server (34 tools, 11 resources, 10 prompts) | ✅ | M16-M18 |
| Observability OTel + Prometheus + structlog | ✅ | M19 D51 |
| HTTP+SSE transport + multi-replica Redis | ✅ | M19 D54 |
| **Mode-2 (anchors dynamiques LLM-generated)** | ❌ | **manquant** |
| **Mode-3 (multi-turn reasoning)** | ❌ | **manquant** |
| **LLM cost governance** | ❌ | **manquant** |
| **LLM-as-curator tooling** | ❌ | **manquant** |
| **Active learning pipeline** | ❌ | **manquant** |
| **Agent runtime in-cluster** | ❌ | **manquant** |

Les **6 capabilities manquantes** sont ciblées par M20-M25.

### 2.2 Le gap LLM

**Aujourd'hui (post-M19)** : un client (Claude Desktop, agent custom)
appelle l'mcp-server, reçoit des tools, fait son propre raisonnement,
appelle EFE backend pour un classify Mode-1. Le **LLM est extérieur à
EFE** ; EFE ne sait rien sur quel modèle est utilisé, à quelle
température, avec quels prompts.

**Conséquences** :
- EFE ne peut **pas garantir** le coût end-to-end d'une requête (le
  client paye ses propres tokens, EFE ne sait pas).
- EFE ne peut **pas auditer** les décisions LLM (le LLM tourne hors
  audit chain HMAC).
- EFE ne peut **pas optimiser** pour cas d'usage : le LLM choisit son
  modèle, ses prompts ; EFE n'a pas de visibilité.
- EFE ne peut **pas escalader** intelligemment : un cas de faible
  confiance ne déclenche pas automatiquement un LLM ; le client doit
  le faire lui-même.

**Ce que les design partners demandent** (sources : 5 entretiens
post-M18) :

1. **« Donnez-moi un escalation auto vers LLM pour cas de faible
   confiance »** — Mode-2.
2. **« Aidez-moi à découvrir des classes manquantes dans ma taxonomie »**
   — Curation (M23).
3. **« Je veux un fallback déterministe si mon LLM principal est down »**
   — Multi-vendor + circuit breaker (M20).
4. **« Combien je vais payer en LLM tokens ce mois ? »** — Cost
   governance (M21+).
5. **« Mon DPO veut un audit complet des décisions automatisées »** —
   GDPR Art. 22 audit (M21+).

### 2.3 Pourquoi maintenant

**Trois conditions sont réunies** :

1. **Production hardening livré (M19)** : observability, BYOK, multi-replica.
   Ajouter du LLM (qui est *intrinsèquement* coûteux + lent + vendor-dépendant)
   sans observabilité serait suicidaire. M19 a précisément créé les
   fondations pour mesurer/contrôler/observer les LLM calls.

2. **Marché LLM stabilisé (2026)** : les frontier providers (Anthropic
   Claude 4, OpenAI GPT-5, Google Gemini 2.5) ont des SLAs prod-grade,
   des BAAs (HIPAA), des contrats data-residency (GDPR), et des
   API stables. Les open-source (Llama 4 70B, Mistral Large 3) ont
   atteint la parité de qualité sur classification.

3. **EU AI Act fully in force (2025-2026)** : compliance par défaut
   est non-négociable. Mode-2/3 doivent **naître compliant** plutôt
   que retrofit (cas Mode-1 où GDPR a été ajouté en M14).

### 2.4 Non-objectifs explicites

EFE ne deviendra **PAS** :
- Un LLM provider (pas de modèles propres entraînés).
- Un fine-tuning service (LLM = zero-shot, EFE-side).
- Un orchestrateur de prompts général (pas un LangChain-killer).
- Un copilot UI (Claude Desktop / Cursor restent les clients).
- Un foundation model trainer (jamais).

EFE **EST** :
- Un classifier zero-shot multi-tenant production-grade.
- Un MCP-exposed agent toolkit.
- Un cost-governed LLM router pour classification (Mode-2/3).
- Un compliance-attested decision engine (GDPR Art. 22, AI Act).

---

## 3. Le paysage LLM 2026

### 3.1 Frontier providers

| Provider | Top model 2026 | Strengths | Weaknesses |
|---|---|---|---|
| **Anthropic** | Claude Opus 4.7 (1M ctx) | Quality, safety, tool use, BAA | Highest $ / token, US-only data residency |
| **OpenAI** | GPT-5 / GPT-5-mini | Latency, ecosystem, multilingual | Privacy concerns (training opt-out manual), data residency limited |
| **Google** | Gemini 2.5 Pro / Flash | Latency, low cost on Flash, EU data residency, GCP integration | API maturity vs Anthropic/OpenAI |
| **Mistral** | Mistral Large 3 / Codestral | EU sovereign, lower $, open-weight option | Smaller ecosystem |
| **AWS Bedrock** | Hosts Claude / Titan / Llama | BAA + AWS-native + IAM, no separate vendor | Marginal markup vs direct |
| **Azure OpenAI** | GPT-4o + GPT-5 (Azure-hosted) | BAA + Azure-native + AAD auth | Throttle quotas |

### 3.2 Open-weight / self-host

| Family | Top open model 2026 | Inference engine | Hardware target |
|---|---|---|---|
| **Llama** | Llama 4 70B / 405B | vLLM, TensorRT-LLM | A100/H100 cluster |
| **Mistral** | Mistral Large 3 (open) | vLLM | A100/H100 |
| **Qwen** | Qwen 3 72B | vLLM | A100/H100 |
| **DeepSeek** | DeepSeek R1.2 | vLLM | H100 |
| **Quantized** | Llama 4 70B Q4 / Q5 | llama.cpp | M3/M4 Max, 5090 |
| **Edge** | Phi-4 small, Gemma 2 9B | MLX, llama.cpp | Mac mini, edge GPU |

**Key 2026 fact** : le gap qualité frontier vs open-weight 70B+ est
< 5 % sur classification. Pour Mode-2/3, **un open-weight 70B sur
self-host est viable** comme primary, frontier comme fallback haute
qualité.

### 3.3 Specialized "small-LMs" (1B-13B)

Cas d'usage : **escalation cheap** (Mode-2 first-tier) avant frontier.

| Modèle | Taille | Use case | Latency CPU |
|---|---|---|---|
| Llama Guard 3 8B | 8B | Safety classification | ~150 ms p50 |
| Mistral 7B Instruct v0.3 | 7B | General classification | ~200 ms p50 |
| Phi-4 14B | 14B | Quality reasoning, low-cost | ~500 ms p50 |
| Gemma 2 9B IT | 9B | Multilingual | ~250 ms p50 |
| Granite Guardian 3 | 8B | Toxicity | ~200 ms p50 |

**Tier strategy** : embedder (1B equivalent, 50 ms) → small-LM
(7-14B, 150-500 ms) → frontier (200B+, 500 ms-2 s).

### 3.4 Pricing trends 2026

| Tier | Model | $/1M input tokens | $/1M output tokens |
|---|---|---|---|
| Frontier | Claude Opus 4.7 | $15 | $75 |
| Frontier | GPT-5 | $10 | $40 |
| Mid | Claude Sonnet 4.5 | $3 | $15 |
| Mid | GPT-5-mini | $0.30 | $1.20 |
| Mid | Gemini 2.5 Flash | $0.075 | $0.30 |
| Self-host (vLLM 70B) | Llama 4 70B Q5 | ~$0.50/h GPU | (incl) |
| Self-host (llama.cpp) | Llama 4 70B Q4 | ~$0/h (existing hw) | (incl) |

Coût moyen par classification Mode-2 (typical 200 input tokens, 50
output tokens) :
- Claude Opus 4.7 : $0.0067
- Claude Sonnet 4.5 : $0.0014
- GPT-5-mini : $0.00012
- Gemini Flash : $0.000030
- vLLM Llama 4 70B (1 H100 sustained) : ~$0.000015 / classify

**Comparaison** : Mode-1 EFE = $0.000005 / classify (50 µs CPU
embedder + ~10 µs swarm). Mode-2 escalation à Gemini Flash = ~6× plus
cher ; à Claude Opus = ~1300× plus cher.

→ **Cost governance critique** : Mode-2 doit être *escalation* (≤ 5 %
des cas), pas default. Pour 100 % cas en Mode-2 vs 100 % Mode-1, le
coût X 1000.

---

## 4. Vision stratégique — 4 patterns d'intégration LLM

### 4.1 Le grid 2D

```
                    │ Runtime classify  │ Curation         │ Eval / RT     │ Agent
────────────────────┼───────────────────┼──────────────────┼───────────────┼──────────
LLM-as-client       │ M16-M19 ✅        │ —                │ —             │ ext only
(LLM externe)       │  (MCP tool surf.) │                  │               │
                    │                   │                  │               │
LLM-as-component    │ M21 (Mode-2)      │ M23              │ M24 active    │ M25
(LLM intra-cluster, │ M22 (Mode-3)      │  (taxonomy AI)   │  learning     │  agent
 routé EFE)         │                   │                  │               │  runner
                    │                   │                  │               │
LLM-as-helper       │ M20 (BYO-LLM SDK) │ M20 + M23        │ —             │ —
(LLM côté operator/ │  agent SDK        │  operator tools  │               │
 dev)               │                   │                  │               │
```

### 4.2 Pattern A — LLM-as-client (acquis M16-M19)

LLM externe (Claude Desktop, agent custom) parle à mcp-server via
JSON-RPC stdio ou HTTP+SSE. EFE expose des tools / resources /
prompts. **Le LLM décide** : quel tool appeler, dans quel ordre,
avec quels args. Le LLM **n'est PAS** dans le critical path d'une
classify Mode-1.

**Limites** : EFE n'a pas la main sur le LLM (cost, model, prompt).
Voir § 2.2.

### 4.3 Pattern B — LLM-as-classifier (Mode-2 + Mode-3)

LLM **devient un composant** d'EFE. Backend Rust appelle un endpoint
LLM (provider abstraction) pour générer des anchors dynamiques (Mode-2)
ou faire du multi-turn iterative refinement (Mode-3). Chaque appel
est tracké dans audit chain, soumis à cost governance, multi-tenant
isolated.

**Avantages** :
- EFE garantit le coût per classify (cost cap par tenant).
- Audit chain inclut LLM decisions (compliance Art. 22).
- Caching cross-tenant (sentence hash) impossible (tenant isolation),
  mais caching intra-tenant possible.
- Choix provider gouverné centralement, pas par client.

**Risques** : vendor dépendance (mitigée par LLMRouter), coût mensuel
inconnu (mitigé par hard cap), latence p99 augmente (Mode-1 : 200 ms ;
Mode-2 : 500-1000 ms ; Mode-3 : 2-5 s).

### 4.4 Pattern C — LLM-as-curator (M23)

LLM aide les **opérateurs** à curer leur taxonomie : proposer des
splits/merges, expliquer une misclassification, générer des examples
synthétiques pour boost la diversité du training set. Cela passe par
des **mcp-server tools** (continuation pattern M16-M19) appelés par
le client de l'opérateur (Claude Desktop, agent custom).

Le LLM peut être :
- **Externe** (sampling delegation au client MCP, pattern M16-M17)
- **Interne** (LLMRouter EFE, pattern Mode-2)

L'opérateur choisit via flag (`use_internal_llm: bool`).

### 4.5 Pattern D — LLM-as-agent (M25 expérimental)

Long-running agent qui tourne **dans le cluster EFE** plutôt que côté
client. Use cases : audit batch ("audit all bundles for SLSA L3
compliance"), refactor large taxonomies, generate documentation.

**Sandboxed** : agent runner en namespace isolé, sans accès cluster
state, sans network egress sauf LLM endpoint, audit chain HMAC pour
chaque action. Compliance-heavy (AI Act high-risk système).

**Statut M25** : expérimental. Pas en prod par défaut. Single design
partner pilot.

### 4.6 Activation philosophy — opt-in everywhere

**Principe non-négociable** : chaque capability M20→M25 est livrée
**OFF par défaut**, à activer via feature flag explicite. Ce n'est pas
une option de configuration parmi d'autres ; c'est la **posture
architecturale** qui rend la roadmap acceptable.

#### 4.6.1 Pourquoi opt-in everywhere

1. **Préservation compliance M19** : EFE 1.3 GA est attestée SOC 2 +
   ISO 27001 + GDPR Art. 32 + NIS2 + EU CRA **sans LLM dans le
   critical path**. Activer M20-M25 par défaut ferait basculer
   *tous* les tenants existants dans le périmètre EU AI Act
   2024/1689, ce qui exigerait DPIA + conformity assessment côté
   tenant — un breaking change compliance imposé. Inacceptable.

2. **Coût opex prévisible** : un tenant qui n'a pas activé Mode-2
   paye **$0 LLM tokens**. Activer par défaut introduirait des coûts
   LLM cachés ; un coût-cap qui se déclenche silencieusement
   dégraderait l'UX. Opt-in = facturation prévisible.

3. **Surface d'attaque minimale** : 12 nouvelles menaces M20-M25 (TM-M21-S-01
   prompt injection CRITICAL, TM-M25-E-01 sandbox escape CRITICAL,
   etc.) **n'existent que** si la feature correspondante est activée.
   Un tenant qui n'a pas Mode-2 activé n'est pas exposé à
   prompt injection sur Mode-2 par construction.

4. **Performance Mode-1 préservée** : un tenant Mode-1-only ne paye
   aucune latence supplémentaire pour la présence de code Mode-2 dans
   le binaire. Le service `efe-llm-router` (M21) n'est **pas démarré**
   si flag server-level OFF.

5. **Migration incrémentale** : un tenant peut activer M20 (helpers)
   sans engager M21 (Mode-2). Ou activer M21 read-only Mode-2 sans
   M22 (Mode-3 multi-turn). Granularité fine = adoption progressive.

6. **Sunset / rollback simple** : flag → OFF, le système revient au
   comportement M19 sans data migration. Pas de "no going back" qui
   bloque les décisions business.

#### 4.6.2 Hiérarchie à 4 niveaux

```
   ┌──────────────────────────────────────────────────────┐
   │ Niveau 1 — Server (Helm value / env-var)            │
   │   EFE_FEATURE_MODE2_ENABLED=false  (default)        │
   │   contrôle binaire : feature compilée / déployée /  │
   │   service router démarré ou non                     │
   └─────────────────┬────────────────────────────────────┘
                     │ AND
                     ▼
   ┌──────────────────────────────────────────────────────┐
   │ Niveau 2 — Tenant (DB row : tenant_feature_flags)   │
   │   tenant_feature_mode2.enabled=false (default)      │
   │   isolation par tenant ; un tenant peut être ON     │
   │   pendant que d'autres sont OFF                     │
   └─────────────────┬────────────────────────────────────┘
                     │ AND
                     ▼
   ┌──────────────────────────────────────────────────────┐
   │ Niveau 3 — Catalog (DB column : catalog.mode2_*)    │
   │   catalog.mode2_enabled=false (default)             │
   │   un tenant peut activer Mode-2 sur catalog A et    │
   │   le laisser OFF sur catalog B                      │
   └─────────────────┬────────────────────────────────────┘
                     │ AND
                     ▼
   ┌──────────────────────────────────────────────────────┐
   │ Niveau 4 — Request (RPC param : Mode enum)          │
   │   Classify(mode=MODE_1|MODE_2|MODE_3)               │
   │   le client choisit explicitement par appel ;       │
   │   default = MODE_1 si non spécifié                  │
   └──────────────────────────────────────────────────────┘
```

**Règle d'évaluation** : `is_active = N1 AND N2 AND N3 AND N4`.
Un seul niveau OFF ⇒ feature OFF. Le client reçoit une erreur
explicite (`feature_disabled` HTTP 403) si N4 demande une feature
que les niveaux supérieurs n'autorisent pas — pas de fallback
silencieux pour éviter surprises.

#### 4.6.3 Default state matrix

| Niveau | M19 GA (current) | M20-M25 livrés (futur) |
|---|:---:|:---:|
| Server (binaire / Helm) | N/A | **OFF** (extras pas installés, services non démarrés) |
| Tenant (DB row) | N/A | **OFF** (row absente = OFF) |
| Catalog (DB column) | N/A | **OFF** (default value `false`) |
| Request (RPC param) | mode=MODE_1 (Mode-1 only) | mode=MODE_1 (default si non spécifié) |

**Conclusion** : un tenant existant en M19 GA, après upgrade vers
EFE 1.4+, **continue à fonctionner exactement comme avant** sans
aucune action. Pas de migration forcée. Pas de surprise de
facturation. Pas de checkbox compliance à cocher.

#### 4.6.4 Mécanisme d'activation pour le tenant

Workflow opérateur (admin tenant) :

1. **Lecture state actuel** : tool MCP `efe_list_active_features`
   retourne la matrice 4-niveaux pour ce tenant.
2. **Vérification prérequis** : si server-level flag OFF, opérateur
   sait que sa modif tenant-level n'aura aucun effet — il doit
   demander à l'opérateur EFE (cluster admin) d'activer côté server.
3. **Activation tenant-level** : tool MCP `efe_set_tenant_feature` avec
   `confirm:true` (M16 carry-over write pattern). Audit chain HMAC
   journalise l'événement.
4. **Activation catalog-level** : optionnel, via
   `efe_set_catalog_feature`.
5. **Activation request-level** : explicit dans chaque `Classify` call.

L'audit chain conserve l'historique complet des toggles ; le
support technique peut reconstituer "qui a activé quoi quand".

#### 4.6.5 Préservation compliance EFE 1.3 si tout OFF

**Garantie formelle** : si **AUCUN** flag M20-M25 n'est activé sur
un cluster EFE 1.4+, le système est **fonctionnellement et
compliance-équivalent** à un cluster EFE 1.3 GA. Cette garantie est
testée en CI via `test_default_off_compat.py` :

- `test_default_off_no_llm_router_service` : pas de processus
  `efe-llm-router` running.
- `test_default_off_mode2_rpc_returns_unimplemented` : appel
  `Classify(mode=MODE_2)` → `UNIMPLEMENTED` gRPC.
- `test_default_off_no_llm_costs_in_audit` : audit chain ne contient
  aucun event `llm_call_*`.
- `test_default_off_compliance_attestations_unchanged` : SOC 2 / ISO
  27001 evidence packs identiques à M19 GA.
- `test_default_off_threat_model_count` : `len(active_threats) == 37`
  (= cumulé M19, **pas** 49 = cumulé M25).
- `test_default_off_pip_extras_not_installed` : `pip show
  efe-mcp-server[llm]` retourne not-installed.

---

## 5. Roadmap M20→M25

### 5.1 Vue d'ensemble (table dense)

| Milestone | Sprints (Dxx) | Lettre | Quarter | Effort PM-W | Default state | Headline |
|---|---|:---:|:---:|---:|:---:|---|
| **M20** | D55-D58 | F | Q3 2026 | 30 | **OFF** (extras pas installés) | BYO-LLM Agent SDK + 5 LLM-helper tools (mcp-server side, no backend) |
| **M21** | D59-D64 | G | Q4 2026 | 60 | **OFF** (router non démarré) | Mode-2 backend Rust + LLMRouter + 8 providers + cost governance + EFE 1.4 |
| **M22** | D65-D69 | H | Q1 2027 | 50 | **OFF** (requires M21 ON) | Mode-3 multi-turn + concept stack + EFE 1.5 + audit chain Art. 22 |
| **M23** | D70-D73 | I | Q2 2027 | 35 | **OFF** (requires M21 ON) | Curation tools (5 new mcp-server tools) + prompt library + A/B infra |
| **M24** | D74-D79 | J | Q3 2027 | 60 | **OFF** (requires M21+M23 ON) | Active learning : low-conf → LLM gold → training pipeline → bundle promotion gates |
| **M25** | D80-D85 | K | Q4 2027 | 50 | **OFF** (requires Helm CRD opt-in + M21 ON) | Agent runtime expérimental (sandboxed in-cluster) |

**Toutes les capabilities sont OFF par défaut** (cf. § 4.6 +
Annexe F). Les sponsors business peuvent suivre l'adoption via
`efe_list_active_features` per-tenant + dashboard
`M19-FEATURES`.

**Total** : 285 PM-W ≈ 24 PM-mois équipe pic 6 ETP, ~14 mois calendaires.

### 5.2 Dépendances inter-milestones

```
M19 (✅ livré) ── M20 (BYO-LLM SDK) ── parallèle
   │                  │
   │                  ↓
   └──────── M21 (Mode-2) ── M22 (Mode-3)
                  │              │
                  └──── M23 (curation, dépend de M21 LLMRouter)
                                 │
                                 ↓
                          M24 (active learning, dépend M22 + M23)
                                 │
                                 ↓
                          M25 (agent runtime, dépend M22)
```

**Critical path** : M21 (Mode-2) bloque M22, M23, M24. Si M21 dérape
de 1 trimestre, tout le reste glisse.

**Parallélisable** : M20 (Python-only mcp-server) peut commencer
parallèlement à M21 spec.

### 5.3 Gates entre milestones

Chaque transition milestone passe par **3 gates** :

1. **Gate technique** : audit pass post-livraison (~6-8 défauts
   attendus, M11-M19 pattern), pin tests GREEN, SLOs ≥ targets.
2. **Gate compliance** : DPIA mise à jour (GDPR Art. 35), AI Act
   conformity assessment ré-évalué, SOC 2 evidence pack incrémenté.
3. **Gate produit** : 2-3 design partners pilot validation,
   acceptance criteria signés.

Aucun milestone ne se déclenche sans gates précédent CLOS. Sponsor
business + tech lead arbitrent.

---

## 6. M20 — BYO-LLM Agent SDK

### 6.1 Mission

Livrer une **librairie Python `efe-agent`** qui :
1. Embarque l'mcp-server EFE comme dépendance.
2. Wrappe une boucle agent (sampling → tool call → sampling → ...).
3. Abstrait les LLM providers (Anthropic, OpenAI, vLLM local,
   llama.cpp local, Bedrock, Azure OpenAI, Vertex).
4. Fournit des **5 nouveaux mcp-server tools** opérateur-facing
   (LLM-helper category).

**Pas de backend changes**. Pas de changement EFE contract. Pure
Python, opt-in.

### 6.2 Architecture

```
clients/python/efe_agent/
├── __init__.py
├── agent.py              # Agent loop : sampling → tool → sampling
├── llm/                  # Provider abstraction
│   ├── base.py           # LLMProvider Protocol
│   ├── anthropic.py      # Anthropic SDK wrapper
│   ├── openai.py         # OpenAI SDK wrapper
│   ├── bedrock.py        # AWS Bedrock (boto3)
│   ├── azure_openai.py   # Azure OpenAI
│   ├── vertex.py         # Google Vertex AI
│   ├── vllm.py           # vLLM HTTP API (self-hosted)
│   ├── llama_cpp.py      # llama.cpp HTTP server (local)
│   └── mlx.py            # Apple MLX (local Mac)
├── prompts/              # Prompt templates for agent loops
│   ├── classify_with_reasoning.md
│   ├── debug_misclassification.md
│   └── propose_taxonomy_split.md
├── tools/                # mcp-server tools registered for agent loops
│   └── (re-uses M16-M19 mcp-server tools)
└── examples/
    ├── batch_classify.py
    ├── operator_curation.py
    └── incident_response.py
```

### 6.3 LLMProvider Protocol (Python)

```python
from typing import Protocol, AsyncIterator

class LLMResponse:
    text: str
    tokens_in: int
    tokens_out: int
    cost_usd: float
    finish_reason: str   # stop | length | tool_use | safety
    tool_calls: list[ToolCall] | None
    cache_hit: bool

class ProviderHealth:
    available: bool
    latency_p50_ms: float | None
    latency_p99_ms: float | None
    error_rate_30d: float

class LLMProvider(Protocol):
    name: str
    supports_tools: bool
    supports_streaming: bool
    max_context: int
    
    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None,
        max_tokens: int,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse: ...
    
    async def stream(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict],
        tools: list[dict] | None,
        max_tokens: int,
    ) -> AsyncIterator[LLMResponse]: ...
    
    async def health(self) -> ProviderHealth: ...
    
    @property
    def cost_model(self) -> CostModel: ...
```

### 6.4 5 nouveaux mcp-server tools (M20 livraison)

| Tool name | Profile | Write? | Description |
|---|---|:---:|---|
| `efe_explain_classification` | agent-native | No | Given (sentence, predicted_label, confidence), ask LLM (via sampling delegation) to produce a 3-line rationale. Useful for operator dashboards. |
| `efe_compare_bundles_quality` | agent-native | No | Given two bundles + a sample of test sentences, ask LLM to compare classification outputs and highlight divergences. |
| `efe_suggest_test_sentences` | agent-native | No | Given a class definition + 5 examples, ask LLM to generate 20 hard/edge test sentences. Used during bundle eval. |
| `efe_analyze_confusion_matrix` | agent-native | No | Given a confusion matrix (M14 catalog stats), ask LLM to identify likely class confusions + propose splits. |
| `efe_draft_runbook` | agent-native | No | Given a SLO breach event from M19 observability, ask LLM to draft a runbook section. |

**Toutes utilisent le pattern MCP sampling** (M16-M17 carry-over) — le
LLM call est délégué au client MCP. **Pas** de LLM call EFE-side en
M20. Cela fait de M20 une livraison Python-pure, low-risk, pré-Mode-2.

### 6.5 Configuration

```python
# Direct usage
from efe_agent import Agent
from efe_agent.llm import AnthropicProvider, FallbackChain

provider = FallbackChain(
    primary=AnthropicProvider(model="claude-sonnet-4-5"),
    fallbacks=[
        OpenAIProvider(model="gpt-5-mini"),
        LlamaCppProvider(server_url="http://localhost:8081"),
    ],
)

agent = Agent(
    mcp_endpoint="http://efe-mcp.example.com:8800",
    oidc_token=os.environ["EFE_OIDC_TOKEN"],
    llm_provider=provider,
    profile="agent-native",
)

result = await agent.run("Audit catalog 'support-routing' for confusion patterns")
```

### 6.6 SLOs M20

| ID | SLO | Cible |
|---|---|---|
| M20-AGENT-1 | Agent loop overhead vs raw LLM call | < 10 % p99 |
| M20-AGENT-2 | Provider failover latency | < 500 ms (between primary fail + fallback success) |
| M20-AGENT-3 | Pin tests | +35 GREEN |

### 6.7 Risques M20

| Risk | Sev | Mitigation |
|---|:---:|---|
| Provider SDKs divergent (Anthropic vs OpenAI tool format) | MED | Adapter layer in `llm/base.py` ; tool schemas converted at boundary |
| Pip dependency creep | LOW | Each provider is `extras_require` (`efe-agent[anthropic]`) |
| Customers run insecure local LLM (RCE risk) | MED | Doc explicit security warnings ; `llama.cpp`/`vllm` recommended over raw exec |

### 6.8 Activation flags M20

**Default state** : OFF par défaut. Aucun helper tool n'est exposé
par le mcp-server tant que le flag server-level est OFF.

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Server | `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` | `false` | Si `false`, les 5 tools (`efe_explain_classification`, etc.) ne sont pas enregistrés au démarrage de mcp-server. Pas dans `list_tools` output. |
| Server | `EFE_FEATURE_M20_AGENT_SDK_BUNDLED` | `false` | Si `true`, l'image Docker mcp-server inclut `efe-agent` Python lib pour facilité ; sinon pull séparé `pip install efe-agent`. |
| Tenant | `tenant_feature_m20_helpers.enabled` | `false` | Si OFF, tenant ne voit pas les helpers même si server-level ON. |
| Catalog | N/A | — | M20 helpers sont read-only multi-catalog ; pas de scope catalog. |
| Request | (per-tool args) | — | Chaque tool a son propre opt-in via args (e.g. `use_internal_llm: false`). |

**Activation client (Python)** :
```bash
pip install efe-agent              # pas pris par défaut
pip install efe-agent[anthropic]   # provider opt-in
pip install efe-agent[openai]
pip install efe-agent[bedrock]
pip install efe-agent[vertex]
pip install efe-agent[vllm]
```

**Pas de migration nécessaire** : un tenant qui n'active jamais M20
ne voit aucun changement. `efe-agent` n'est pas installé par
défaut dans l'image mcp-server.

**Test pin** : `test_default_off_m20_no_helper_tools_listed` —
vérifie que `dispatcher.list_tools()` ne contient pas les 5 helpers
quand server-flag OFF.

---

## 7. M21 — Mode-2

### 7.1 Mission

Livrer **Mode-2** (anchors dynamiques LLM-routed) côté backend Rust :

1. Un nouveau RPC `Classify` mode `MODE_2` qui :
   - Embed la sentence (Mode-1 carry-over).
   - Génère **anchors dynamiques** via LLM call (top-K anchors candidats
     pour cette sentence spécifique).
   - Run swarm Mode-1 sur les anchors générés.
   - Return label + confidence + anchors_used.
2. Un nouveau service `LLMRouter` (Rust) qui :
   - Multi-provider (Anthropic / OpenAI / Bedrock / Azure / Vertex /
     vLLM / Mistral).
   - Cost governance per-tenant (USD/month cap, hard).
   - Caching exact-match + semantic-match (M19 Redis carry-over).
   - Circuit breaker per-provider.
   - Fallback chain.
3. **EFE 1.4 contract bump** (semver-mineur, additive RPC).

### 7.2 Architecture (delta vs M19)

```
crates/
├── efe-server/                   (existing)
│   ├── services/
│   │   ├── swarm.rs              ← extend pour Mode-2
│   │   └── mode2.rs              ← NEW : anchors dynamiques
│   └── (existing)
│
├── efe-llm-router/               ← NEW crate (M21)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── provider/
│   │   │   ├── mod.rs            (LLMProvider trait)
│   │   │   ├── anthropic.rs
│   │   │   ├── openai.rs
│   │   │   ├── bedrock.rs
│   │   │   ├── azure_openai.rs
│   │   │   ├── vertex.rs
│   │   │   ├── vllm.rs
│   │   │   └── mistral.rs
│   │   ├── router.rs             (LLMRouter)
│   │   ├── cost_governor.rs      (per-tenant budget enforcement)
│   │   ├── cache.rs              (Redis-backed)
│   │   ├── circuit_breaker.rs    (per-provider)
│   │   ├── prompt_registry.rs    (versioned templates)
│   │   └── audit.rs              (M14-bis HMAC chain integration)
│   └── tests/
│       ├── test_router_integration.rs
│       ├── test_cost_governor.rs
│       ├── test_cache.rs
│       └── test_circuit_breaker.rs
│
└── efe-prompts/                  ← NEW crate (M21)
    ├── templates/
    │   ├── mode2_anchor_gen_v1.md
    │   ├── mode2_anchor_gen_v2.md
    │   └── mode3_multi_turn_v1.md  (← M22)
    └── src/lib.rs                (parsing + variable substitution)
```

### 7.3 Mode-2 flow (deep-dive)

```
                     ┌─────────────────────────────────────────┐
                     │  client.Classify(mode=MODE_2,           │
                     │       sentence="My laptop won't boot",  │
                     │       catalog="support-routing")        │
                     └────────────────────────┬────────────────┘
                                              │
                                              ▼
                                ┌─────────────────────────┐
                                │ tenant_quota.check()    │
                                │  - tokens budget OK?    │
                                │  - rate limit OK?       │
                                └─────────┬───────────────┘
                                          │
                       ┌──────────────────┴───────────────────┐
                       │ (if budget exhausted) → Mode-1 fallback
                       ▼
            ┌──────────────────────┐
            │ embed(sentence)      │ ← Mode-1 carry-over
            │  → 384-dim vector    │
            └──────┬───────────────┘
                   │
                   ▼
            ┌──────────────────────┐
            │ retrieve top-K       │
            │ candidate anchors    │ ← from Mode-1 catalog
            │  via embedding kNN   │
            └──────┬───────────────┘
                   │
                   ▼
       ┌────────────────────────────┐
       │ LLMRouter.complete(        │
       │   prompt=mode2_anchor_gen, │
       │   sentence, candidates,    │
       │   tenant=acme              │
       │ )                          │
       │  → top_3_anchors           │
       │     dynamiques             │
       └────────┬───────────────────┘
                │
                ▼
       ┌────────────────────────────┐
       │ swarm_run(                 │
       │   sentence,                │
       │   anchors=top_3,           │
       │   mode=MODE_1              │ ← réutilise Mode-1 engine
       │ )                          │
       └────────┬───────────────────┘
                │
                ▼
       ┌────────────────────────────┐
       │ audit.append({             │
       │   tool: classify_mode2,    │
       │   tenant_hash, slug,       │
       │   tokens_in, tokens_out,   │
       │   cost_usd, anchors_used,  │
       │   provider, model          │
       │ })                         │
       └────────┬───────────────────┘
                │
                ▼
        ClassifyResponse{
          label, confidence,
          anchors_used,
          mode: MODE_2,
          tokens_used, cost_usd
        }
```

### 7.4 LLMRouter (Rust)

```rust
#[derive(Clone)]
pub struct LLMRouter {
    providers: Vec<Arc<dyn LLMProvider>>,
    cost_governor: Arc<CostGovernor>,
    cache: Arc<LLMCache>,
    circuit: Arc<CircuitBreaker>,
    audit: Arc<AuditEmitter>,
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn name(&self) -> &str;
    fn supports_tools(&self) -> bool;
    fn cost_per_1m_in(&self) -> f64;
    fn cost_per_1m_out(&self) -> f64;
    
    async fn complete(
        &self,
        request: CompleteRequest,
    ) -> Result<CompleteResponse, ProviderError>;
    
    async fn health(&self) -> ProviderHealth;
}

impl LLMRouter {
    pub async fn complete(
        &self,
        request: RouterRequest,
    ) -> Result<RouterResponse, RouterError> {
        // 1. Cost governance pre-flight
        self.cost_governor
            .check_budget(&request.tenant_id, request.estimated_cost())
            .await?;
        
        // 2. Cache lookup
        let cache_key = compute_cache_key(&request);
        if let Some(cached) = self.cache.get(&cache_key).await? {
            return Ok(RouterResponse::cached(cached));
        }
        
        // 3. Provider selection (cost tier + circuit breaker state)
        let provider_chain = self.select_providers(&request);
        
        // 4. Try in order
        for provider in provider_chain {
            if !self.circuit.is_closed(provider.name()) {
                continue; // skip if circuit open
            }
            match provider.complete(request.to_provider_request()).await {
                Ok(resp) => {
                    self.cache.put(cache_key, &resp).await?;
                    self.cost_governor.commit_actual_cost(
                        &request.tenant_id, resp.cost_usd,
                    ).await?;
                    self.audit.append_llm_call(&request, &resp).await?;
                    return Ok(RouterResponse::from(resp));
                }
                Err(e) => {
                    self.circuit.record_failure(provider.name(), &e);
                    tracing::warn!("provider {} failed: {}", provider.name(), e);
                }
            }
        }
        
        Err(RouterError::AllProvidersFailed)
    }
}
```

### 7.5 Cost governance

#### 7.5.1 Per-tenant budget

Stocké dans Postgres `tenant_llm_budget` table :

```sql
CREATE TABLE tenant_llm_budget (
    tenant_id           TEXT PRIMARY KEY,
    monthly_cap_usd     NUMERIC(10,4) NOT NULL,
    soft_threshold_pct  INT DEFAULT 80,
    fail_open           BOOLEAN DEFAULT false,
    -- if true: when cap hit, fall back to Mode-1 (degraded UX, no error)
    -- if false: hard error 429
    rollover_dom        INT DEFAULT 1,  -- day of month to reset
    last_rollover       DATE NOT NULL,
    spent_this_month    NUMERIC(10,4) DEFAULT 0
);
```

#### 7.5.2 Pre-flight check

```rust
pub async fn check_budget(
    &self,
    tenant_id: &str,
    estimated_cost_usd: f64,
) -> Result<(), CostError> {
    let row = sqlx::query!(
        "SELECT monthly_cap_usd, spent_this_month, fail_open, soft_threshold_pct
         FROM tenant_llm_budget WHERE tenant_id = $1",
        tenant_id,
    ).fetch_one(&self.pool).await?;
    
    let after = row.spent_this_month + estimated_cost_usd;
    let cap_pct = (after / row.monthly_cap_usd) * 100.0;
    
    if cap_pct >= 100.0 && !row.fail_open {
        return Err(CostError::BudgetExhausted {
            tenant_id: tenant_id.to_string(),
            cap_usd: row.monthly_cap_usd,
            spent_usd: row.spent_this_month,
        });
    }
    
    if cap_pct >= row.soft_threshold_pct as f64 {
        // emit warning audit event but allow
        self.audit.append_budget_warning(tenant_id, cap_pct).await?;
    }
    
    Ok(())
}
```

#### 7.5.3 Commit actual cost (post-call)

```rust
pub async fn commit_actual_cost(
    &self,
    tenant_id: &str,
    actual_cost_usd: f64,
) -> Result<(), CostError> {
    sqlx::query!(
        "UPDATE tenant_llm_budget
         SET spent_this_month = spent_this_month + $1
         WHERE tenant_id = $2",
        actual_cost_usd,
        tenant_id,
    ).execute(&self.pool).await?;
    Ok(())
}
```

#### 7.5.4 Monthly rollover

Cron job daily à 02:00 UTC :

```rust
pub async fn rollover_expired_tenants(&self) -> Result<usize, CostError> {
    let n = sqlx::query!(
        "UPDATE tenant_llm_budget
         SET spent_this_month = 0, last_rollover = CURRENT_DATE
         WHERE EXTRACT(DAY FROM CURRENT_DATE) = rollover_dom
           AND last_rollover < CURRENT_DATE - INTERVAL '25 days'"
    ).execute(&self.pool).await?.rows_affected();
    Ok(n as usize)
}
```

### 7.6 Caching strategy

#### 7.6.1 Exact-match cache

Key : `sha256(tenant_id || catalog || version || sentence || prompt_template_version)`.

TTL : per-bundle (default 1h, configurable up to 7 days).

Stocké dans Redis (M19 carry-over) avec namespace `efe:llm:cache:`.

#### 7.6.2 Semantic-match cache

Key : embedding(sentence) → kNN top-3 in tenant cache, if cosine
similarity > 0.95, reuse the cached anchors.

Implémentation : Redis Stack (RediSearch + RedisVL) avec
`HNSW` index.

**Trade-off** : semantic cache + 5-15 ms latency overhead per
classify, mais cache hit rate passe de ~30 % (exact) à ~60-70 %
(semantic). À évaluer avec design partners.

#### 7.6.3 Per-tenant isolation

Chaque tenant a son propre namespace Redis (M19 carry-over). Aucun
cache cross-tenant. Pas de cache global.

### 7.7 EFE 1.4 contract diff

**New gRPC RPC** (additive) :

```protobuf
service EfeSwarmService {
  // existing M16-M19 RPCs unchanged...
  
  // M21 NEW
  rpc ClassifyMode2(ClassifyMode2Request) returns (ClassifyMode2Response);
  rpc StreamClassifyMode2(ClassifyMode2Request) returns (stream ClassifyMode2Update);
}

message ClassifyMode2Request {
  string sentence = 1;
  string catalog = 2;
  optional string version = 3;
  optional uint32 top_k_anchors = 4;     // default 3
  optional bool fail_open_on_budget = 5; // default tenant config
}

message ClassifyMode2Response {
  string label = 1;
  float confidence = 2;
  repeated string anchors_used = 3;
  Mode mode = 4;                         // = MODE_2 (or MODE_1 if budget fallback)
  uint32 tokens_in = 5;
  uint32 tokens_out = 6;
  double cost_usd = 7;
  string provider_used = 8;
  string model_used = 9;
  bool cache_hit = 10;
}
```

**New REST endpoints** (admin-only) :

| Method | Path | Purpose |
|---|---|---|
| GET | `/admin/llm/budget/{tenant_id}` | Get current budget state |
| PUT | `/admin/llm/budget/{tenant_id}` | Set monthly cap |
| GET | `/admin/llm/providers` | List available providers + health |
| GET | `/admin/llm/cache/stats` | Hit rate, eviction count |
| POST | `/admin/llm/cache/invalidate` | Invalidate cache for catalog/tenant |

### 7.8 New mcp-server tools (M21 livraison MCP-side)

| Tool name | Profile | Write? | Description |
|---|---|:---:|---|
| `efe_classify_mode2` | agent-native | No | Run a Mode-2 classify (LLM-routed anchors). Returns label + cost. |
| `efe_get_llm_budget` | core | No | Get current LLM budget for tenant. |
| `efe_set_llm_budget` | agent-native-deploy | **Yes** | Set monthly cap. `confirm` required. |
| `efe_list_llm_providers` | advanced | No | List available providers + health snapshot. |
| `efe_invalidate_llm_cache` | agent-native | **Yes** | Invalidate LLM cache for a catalog. `confirm` required. |

### 7.9 SLOs M21

| ID | SLO | Cible |
|---|---|---|
| M21-MODE2-1 | Mode-2 p50 latency | < 600 ms |
| M21-MODE2-2 | Mode-2 p99 latency | < 2 s |
| M21-MODE2-3 | Mode-2 cache hit rate (semantic) | ≥ 60 % |
| M21-MODE2-4 | Cost governance budget enforcement | 100 % (no over-cap calls) |
| M21-MODE2-5 | Provider failover success | ≥ 99 % when primary fails |
| M21-MODE2-6 | Audit completeness (Art. 22) | 100 % LLM calls audited |
| M21-PIN | Pin tests | +120 GREEN |

### 7.10 Risques M21

| Risk | Sev | Mitigation |
|---|:---:|---|
| Provider price hike (Anthropic 2× tomorrow) | MED | Multi-provider abstraction + fail-over to cheaper |
| Cache poisoning (malicious cached entry) | LOW | Cache is per-tenant + signed entries |
| LLM hallucination produces invalid anchors | MED | Validation : anchors must be in candidate set OR rejected |
| Cost cap bypass via concurrent calls | MED | Cost governor uses Postgres `SELECT FOR UPDATE` for race safety |
| Latency spike pulls Mode-1 SLO down | HIGH | Mode-2 in dedicated thread pool ; Mode-1 unaffected |
| EU AI Act non-conformity | HIGH | DPIA + conformity assessment Q4 2026 |

### 7.11 Activation flags M21 (le plus chargé)

**Default state** : OFF total. Le service `efe-llm-router` n'est pas
démarré, l'image Docker ne le contient pas (séparé via Helm chart),
les RPCs `ClassifyMode2` retournent `UNIMPLEMENTED`.

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Helm chart | `llmRouter.enabled` | `false` | Si `false`, deployment `efe-llm-router` non créé. Si `true`, deployment + service + RBAC + secrets créés. |
| Server | `EFE_FEATURE_MODE2_ENABLED` | `false` | Si `false`, backend Rust enregistre `ClassifyMode2` RPC qui retourne `UNIMPLEMENTED`. Aucun appel router. |
| Server | `EFE_LLM_ROUTER_URL` | `""` | Si vide ET feature enabled = error startup. Force config explicite. |
| Server | `EFE_FEATURE_MODE2_REQUIRE_TENANT_OPT_IN` | `true` | Si `true` (default), opérateur EFE ne peut PAS pousser Mode-2 à un tenant qui n'a pas activé tenant-level. Garde-fou contre l'abus admin. |
| Tenant | `tenant_feature_mode2.enabled` | `false` | Default OFF même si server ON. Tenant doit signer un avenant DPA pour Mode-2. |
| Tenant | `tenant_feature_mode2.allowed_providers` | `[]` | Liste vide = aucun provider autorisé même si flag enabled. Tenant doit choisir explicitement (Anthropic / OpenAI / Bedrock / etc.). |
| Tenant | `tenant_feature_mode2.monthly_cap_usd` | `0` | Cap = 0 → tous appels rejetés `BUDGET_EXHAUSTED`. Tenant doit définir cap > 0 pour activer effectivement. |
| Tenant | `tenant_feature_mode2.fail_open` | `false` | Si OFF (default), budget exhausted = erreur. Si ON, fall-back silencieux Mode-1 avec flag `degraded=true`. |
| Tenant | `tenant_feature_mode2.require_baa` | `false` | Si `true`, restreint aux providers BAA (Bedrock, Azure, Anthropic Ent). |
| Tenant | `tenant_feature_mode2.require_eu_residency` | `false` | Si `true`, restreint aux providers EU (Mistral, Anthropic Bedrock eu-, Vertex EU). |
| Tenant | `tenant_feature_mode2.require_no_training` | `true` | Si `true` (default), exclut providers sans clause no-training (default protective). |
| Tenant | `tenant_feature_mode2.prompt_redaction_level` | `aggressive` | `none` / `mild` / `aggressive`. Default le plus protecteur. |
| Catalog | `catalog.mode2_enabled` | `false` | Per-catalog opt-in. Un tenant peut activer Mode-2 sur catalog A et le laisser OFF sur catalog B (e.g. catalog low-stakes vs high-stakes). |
| Catalog | `catalog.mode2_threshold_confidence` | `0.0` | Seuil sous lequel Mode-1 escalade vers Mode-2. `0.0` = jamais (effectivement OFF même si flags ON). Tenant fixe à `0.7` typiquement. |
| Catalog | `catalog.mode2_max_anchor_candidates` | `10` | Hard cap sur top-K anchors envoyés au LLM. Plus = + cher. |
| Request | `Classify(mode=MODE_1)` (default) | — | Default si non spécifié. Mode-2 ignoré. |
| Request | `Classify(mode=MODE_2)` | — | Demande explicite. Si flags supérieurs OFF → 403 `feature_disabled`. |
| Request | `Classify(mode=MODE_AUTO)` | — | Backend décide : Mode-1 essayé d'abord, escalation Mode-2 si confidence < threshold AND flags ON. Sinon stay Mode-1. |

**Vérification d'activation effective** (5 conditions AND) :

```rust
fn is_mode2_active_for_request(req: &ClassifyRequest, ctx: &TenantContext) -> bool {
    // Lecture cache 60s pour minimiser DB roundtrips :
    let server = config.feature_mode2_enabled();         // env-var
    let helm   = config.llm_router_reachable();          // probe efe-llm-router /health
    let tenant = ctx.feature_mode2_enabled();            // DB row
    let cap    = ctx.feature_mode2_monthly_cap_usd() > 0.0;
    let provs  = !ctx.feature_mode2_allowed_providers().is_empty();
    let cat    = ctx.catalog(req.catalog).mode2_enabled();
    let req_ok = matches!(req.mode, Mode::MODE_2 | Mode::MODE_AUTO);
    
    server && helm && tenant && cap && provs && cat && req_ok
}
```

Si **n'importe lequel** des 7 flags est OFF, Mode-2 est considéré OFF
pour cette requête. Sinon Mode-2 enabled.

**Audit** : chaque toggle est event `feature_toggled` HMAC-chained
avec :
```json
{
  "event_type": "feature_toggled",
  "feature": "mode2",
  "level": "tenant" | "catalog",
  "previous_value": false,
  "new_value": true,
  "actor": "<jwt_sub>",
  "tenant_id": "<tenant_id>",
  "timestamp": "...",
  "ip_address": "...",
  "rationale": "<optional free-form>",
  "hmac": "..."
}
```

**Rollback** : tenant peut OFF en `<60 s` (TTL cache config).
Aucune perte de data. Cache LLM invalidé pour ce tenant. Calls
en cours fail-fast avec `feature_disabled` ou completent normally
selon `EFE_FEATURE_TOGGLE_DRAIN_INFLIGHT` (default `drain`).

**Test pin** :
- `test_default_off_mode2_returns_unimplemented` : sans flag, RPC
  retourne `UNIMPLEMENTED` gRPC code.
- `test_default_off_no_llm_router_in_helm` : `helm template` sans
  `llmRouter.enabled=true` → 0 ressources router.
- `test_mode2_requires_all_4_levels` : 4 niveaux testés
  individuellement OFF → tous donnent `feature_disabled`.
- `test_mode2_partial_activation_still_safe` : tenant ON mais
  catalog OFF → pas d'appel LLM, retombe Mode-1 si MODE_AUTO ou
  rejette si MODE_2 explicite.

---

## 8. M22 — Mode-3

### 8.1 Mission

Livrer **Mode-3** : multi-turn iterative reasoning. Le classify peut
boucler 2-5 fois pour affiner le label, en construisant une "concept
stack" intermédiaire.

Cas d'usage typique : sentence ambiguë où Mode-2 confidence < 0.7. Le
LLM décide d'un follow-up : "this looks like billing OR returns ; let
me check for keywords X, Y, Z" → second pass avec narrower candidates
→ confidence rise.

### 8.2 Mode-3 flow

```
turn=0 :
  embed(sentence) → top_K=10 anchors candidates
  LLM(sentence, candidates) → narrow to 5 + reasoning trace
  
turn=1 (if confidence < threshold) :
  swarm(sentence, narrowed_5) → label_attempt + conf
  LLM(sentence, label_attempt, narrowed_5) → "is this right?"
    → "no, actually retry with anchors X, Y" (concept refinement)
  
turn=2 :
  swarm(sentence, [X, Y]) → final_label + conf
  
return : final_label, concept_stack=[turn0_5, turn1_2, turn2_X+Y]
```

Bornage : `MAX_TURNS=5` (configurable, default 3). Coût scale
linéaire avec turns.

### 8.3 Concept stack tracking

Chaque turn contribue une ligne dans la concept_stack :

```rust
pub struct ConceptStackEntry {
    turn: u32,
    candidates_in: Vec<String>,
    candidates_out: Vec<String>,
    llm_reasoning: String,    // truncated to 1KB
    swarm_label_attempt: Option<String>,
    swarm_confidence: Option<f32>,
    cost_usd: f64,
    duration_ms: u64,
}

pub struct ClassifyMode3Response {
    label: String,
    confidence: f32,
    concept_stack: Vec<ConceptStackEntry>,
    total_turns: u32,
    total_cost_usd: f64,
    converged: bool, // true if final conf > threshold
}
```

### 8.4 Compliance — GDPR Art. 22

Mode-3 = automated decision-making with significant effect (catalog
classification can route customer support, deny insurance claim,
etc.). **GDPR Art. 22 explicit** :
- Data subject has right to human review.
- EFE must log full reasoning trace (concept_stack).
- DPIA required.

**Implementation** :
1. Every Mode-3 call audited with full concept_stack (HMAC-chained).
2. Per-tenant config : `human_review_required: bool`. If true, the
   response includes `requires_human_review=true` flag and a
   `review_token` for the operator's UI.
3. Backend exposes `/admin/mode3/reviews/{review_token}` endpoint
   for the operator to attest the decision.

### 8.5 EFE 1.5 contract diff

```protobuf
service EfeSwarmService {
  // M22 NEW
  rpc ClassifyMode3(ClassifyMode3Request) returns (ClassifyMode3Response);
}

message ClassifyMode3Request {
  string sentence = 1;
  string catalog = 2;
  optional uint32 max_turns = 3;          // default 3
  optional float confidence_threshold = 4; // default 0.85
}

message ClassifyMode3Response {
  string label = 1;
  float confidence = 2;
  repeated ConceptStackEntry concept_stack = 3;
  uint32 total_turns = 4;
  double total_cost_usd = 5;
  bool converged = 6;
  bool requires_human_review = 7;
  optional string review_token = 8;
}
```

### 8.6 SLOs M22

| ID | SLO | Cible |
|---|---|---|
| M22-MODE3-1 | Mode-3 p50 latency (3 turns) | < 3 s |
| M22-MODE3-2 | Mode-3 convergence rate | ≥ 80 % within max_turns |
| M22-MODE3-3 | Cost vs Mode-2 | 2-3× higher acceptable |
| M22-MODE3-4 | Human-review SLA (operator response) | < 24 h p95 |
| M22-PIN | Pin tests | +80 GREEN |

### 8.7 Activation flags M22

**Default state** : OFF. **Requires M21 ON** (Mode-3 = Mode-2 +
multi-turn). Si flags M21 OFF, M22 flags ne peuvent même pas être
activés (validation côté admin endpoint).

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Server | `EFE_FEATURE_MODE3_ENABLED` | `false` | Si `false`, RPC `ClassifyMode3` retourne `UNIMPLEMENTED`. |
| Server | `EFE_FEATURE_MODE3_REQUIRE_MODE2` | `true` | Validation : on ne peut pas activer Mode-3 sans Mode-2 actif (sinon orphan dependency). |
| Tenant | `tenant_feature_mode3.enabled` | `false` | Per-tenant. Default OFF même si server ON. |
| Tenant | `tenant_feature_mode3.human_review_required` | `true` | Si `true` (default protective), réponses Mode-3 incluent `requires_human_review=true` + review_token. |
| Tenant | `tenant_feature_mode3.max_turns` | `3` | Hard cap sur multi-turn (GDPR Art. 22 — limiter coût + latence). |
| Catalog | `catalog.mode3_enabled` | `false` | Per-catalog. Tenant peut activer M3 sur catalog A only. |
| Catalog | `catalog.stakes_tier` | `low` | `low` / `medium` / `high`. `high` force `human_review_required=true`. |
| Request | `Classify(mode=MODE_3, max_turns=N)` | — | Demande explicite, N capped par tenant config. |

**Compliance gate** : si `catalog.stakes_tier=high` AND
`tenant_feature_mode3.human_review_required=false`, le RPC d'activation
**rejette** avec `compliance_violation: high-stakes catalogs require
human review`. Garde-fou anti-mauvaise-config.

**Test pin** :
- `test_default_off_mode3_returns_unimplemented`.
- `test_mode3_requires_mode2_active`.
- `test_mode3_high_stakes_forces_human_review`.
- `test_mode3_max_turns_enforced_at_runtime`.

---

## 9. M23 — Taxonomy curation

### 9.1 Mission

Livrer 5+ mcp-server tools qui aident l'opérateur à **curer sa
taxonomie** via LLM. Pattern hybride :
- Si tenant a Mode-2/3 enabled : utilise LLMRouter (M21).
- Sinon : sampling delegation au client MCP.

### 9.2 Workflows ciblés

1. **« Mes classes ne couvrent pas tous les cas »** → `efe_propose_taxonomy_split`
2. **« Cette classify est wrong, pourquoi »** → `efe_explain_misclassification`
3. **« Mon training set manque de diversité »** → `efe_generate_synthetic_examples`
4. **« Ces deux classes se ressemblent trop »** → `efe_suggest_class_merge`
5. **« Quelle est la qualité de mon bundle vs gold »** → `efe_audit_bundle_vs_gold`

### 9.3 Prompt library

Versionned templates dans `crates/efe-prompts/templates/` :

```
mode2_anchor_gen_v1.md
mode2_anchor_gen_v2.md             ← improved May 2027 after eval
mode3_multi_turn_v1.md
mode3_multi_turn_v2.md
curation_propose_split_v1.md       ← M23
curation_explain_miss_v1.md
curation_generate_synth_v1.md
curation_suggest_merge_v1.md
curation_audit_vs_gold_v1.md
```

Chaque template :

```yaml
---
name: curation_propose_split_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, vllm]
min_context_tokens: 4000
estimated_output_tokens: 500
created: 2027-04-15
author: tech_lead
eval_pass: true   # ran against gold dataset
eval_accuracy: 0.87
---

# System
You are a taxonomy expert helping a customer support team improve...

# User
Catalog: {{ catalog }}
Current classes: {{ classes_json }}
Confusion matrix: {{ confusion_csv }}

Question: which 2-3 classes should be split, and why?
```

### 9.4 A/B testing infra

Each prompt version is shadow-evaluated against gold labels :

```
when curation_propose_split_v2 deployed :
  - 50% of calls use v2 (canary)
  - 50% v1 (control)
  - eval against gold : if v2 ≥ v1 + 2pp, promote v2 to 100%
```

Backed by `crates/efe-prompts/src/ab_test.rs`.

### 9.5 SLOs M23

| ID | SLO | Cible |
|---|---|---|
| M23-CURATE-1 | Curation tool p99 latency | < 5 s |
| M23-CURATE-2 | Operator adoption (DAU/MAU) | ≥ 30 % across pilots |
| M23-CURATE-3 | Eval accuracy delta vs human-only baseline | ≥ +5 pp |
| M23-PIN | Pin tests | +60 GREEN |

### 9.6 Activation flags M23

**Default state** : OFF. M23 fonctionne en deux modes (sampling
delegation au client MCP OU LLMRouter interne). Le mode "interne"
**requires M21 ON**.

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Server | `EFE_FEATURE_M23_CURATION_ENABLED` | `false` | Si `false`, 5 curation tools non enregistrés. |
| Server | `EFE_FEATURE_M23_USE_INTERNAL_LLM` | `false` | Si `false`, curation tools utilisent sampling delegation (pattern M16-M17, no internal LLM). Si `true`, utilisent LLMRouter (requires M21). |
| Tenant | `tenant_feature_m23_curation.enabled` | `false` | Per-tenant. |
| Tenant | `tenant_feature_m23_curation.allow_internal_llm` | `false` | Si `false`, force sampling delegation côté client même si server ON. |
| Catalog | N/A | — | M23 tools sont catalog-scoped via args, pas via flag. |

**Test pin** :
- `test_default_off_m23_curation_tools_not_listed`.
- `test_m23_internal_llm_requires_mode2_active`.

---

## 10. M24 — Active learning

### 10.1 Mission

Boucler la production : low-confidence classifies → LLM gold-label →
training queue → bundle re-tune → A/B vs current bundle → promote if
better.

### 10.2 Pipeline

```
[production traffic]
       │
       ▼ (classify_mode1 or _mode2)
  confidence < 0.7 ?
       │ yes
       ▼
  enqueue to gold_label_queue (Postgres + Redis stream)
       │
       ▼ (async batch worker)
  LLM(sentence, candidates) → proposed_gold_label
       │
       ▼
  (if human_review_required) → operator UI
       │
       ▼ (signed off OR auto-confirmed if confidence > 0.9)
  insert into gold_dataset
       │
       ▼ (nightly batch)
  if N new gold labels > THRESHOLD :
      trigger bundle_retune_pipeline
      → embedder re-calibration (no fine-tune)
      → new bundle version (e.g. v3.2.1)
      → eval vs current vs new on holdout
      → if new ≥ current + 1pp accuracy : flag for promotion
       │
       ▼
  operator decides : promote new bundle (M16-M18 deploy flow)
```

### 10.3 Compliance gates

- **Human review** : configurable per-tenant. Default ON for
  high-stakes catalogs (`stakes_tier=high|medium|low`).
- **Audit** : every step logged in M14-bis HMAC chain.
- **Drift detection** : if accuracy drops > 2 pp on holdout, alert
  + freeze (manual override required).
- **GDPR** : right to forget — data subject can request deletion of
  any sentence in gold_dataset traceable to them.

### 10.4 SLOs M24

| ID | SLO | Cible |
|---|---|---|
| M24-AL-1 | Gold-label queue lag | < 1 h p99 |
| M24-AL-2 | Bundle retune frequency | weekly per active catalog |
| M24-AL-3 | Drift detection MTTR | < 24 h |
| M24-AL-4 | Human-review SLA | < 48 h p95 |
| M24-AL-5 | Promotion accuracy delta | ≥ +1 pp before promote |
| M24-PIN | Pin tests | +100 GREEN |

### 10.5 Activation flags M24

**Default state** : OFF. **Requires M21 + M23 ON**. Active learning
sans cost governance (M21) et sans curation tools (M23) est techniquement
impossible.

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Server | `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED` | `false` | Si `false`, pas de gold_label_queue worker démarré, pas de pipeline. |
| Server | `EFE_FEATURE_M24_REQUIRE_M21_M23` | `true` | Validation startup : refuse boot si M24 ON sans M21 + M23. |
| Tenant | `tenant_feature_m24.enabled` | `false` | Per-tenant. |
| Tenant | `tenant_feature_m24.auto_promotion` | `false` | Si `true`, bundles candidate auto-promu si accuracy delta ≥ threshold. Si `false` (default), opérateur valide manuellement (recommended). |
| Tenant | `tenant_feature_m24.human_review_required` | `true` | Si `true`, gold labels validés par humain avant entrée dataset. Si `false`, auto-confirm si LLM confidence > 0.9. |
| Tenant | `tenant_feature_m24.gold_label_threshold` | `1000` | N gold labels min avant retune trigger. |
| Catalog | `catalog.al_enabled` | `false` | Per-catalog. |
| Catalog | `catalog.al_drift_threshold_pp` | `2.0` | Si accuracy drop ≥ X pp, freeze catalog. |

**Compliance gate** : `auto_promotion=true` AND `human_review_required=false`
n'est autorisé que si `catalog.stakes_tier=low`. Garde-fou GDPR.

**Test pin** :
- `test_default_off_m24_no_workers_running`.
- `test_m24_requires_m21_and_m23`.
- `test_m24_auto_promotion_blocked_for_high_stakes`.

---

## 11. M25 — Agent runtime

### 11.1 Mission

Permettre à un LLM agent de **tourner DANS le cluster EFE** (vs les
M16-M19 où l'agent est côté client). Use cases :
- Long-running batch audits ("audit all bundles for SLSA L3").
- Refactor large taxonomies (multi-step, 1+ hour).
- Auto-generate runbook drafts from M19 incident logs.

### 11.2 Architecture

Service `efe-agent-runner` (Rust ou Python wrapper d'un Python core)
deployé en Kubernetes :

```
                    ┌────────────────────────────┐
                    │ AgentRun CRD (K8s custom)  │
                    │  - prompt                  │
                    │  - tenant_id               │
                    │  - timeout_min: 60         │
                    │  - max_cost_usd: 50        │
                    │  - allowed_tools: [...]    │
                    └─────────┬──────────────────┘
                              │
                              ▼ (k8s controller)
                ┌─────────────────────────────┐
                │ AgentRunPod (Job)           │
                │  - sidecar: efe-agent-runner│
                │  - image: efe-agent:M25     │
                │  - networkPolicy: deny      │
                │      egress except LLM API  │
                │  - resourceLimits: 2GB RAM  │
                └─────────────────────────────┘
                              │
                              ▼
                    LLMRouter ──► provider
                              │
                              ▼
                    mcp-server (read-only profile)
                              │
                              ▼
                    EFE backend
```

### 11.3 Sandboxing

- **Network policy** : egress autorisé seulement vers
  - mcp-server (read-only profile)
  - LLM provider endpoints (whitelist)
  - Audit chain backend
  - Pas d'autre.
- **Filesystem** : ephemeral, /tmp wiped post-run.
- **Resource limits** : CPU 1 core, RAM 2 GB, timeout 60 min,
  max_cost_usd 50.
- **Tool permissions** : whitelist obligatoire.
- **Audit** : chaque tool call HMAC-chained.

### 11.4 EFE 2.0 (semver MAJOR)

Justifications du semver MAJOR :
- New service `efe-agent-runner` requires K8s CRDs
- Helm chart ships new RBAC roles
- Backend audit chain extended for agent_run events
- New `/admin/agents/runs/` endpoints

EFE 1.x clients restent compat. EFE 2.0 = OPT-IN du nouveau service.

### 11.5 SLOs M25

| ID | SLO | Cible |
|---|---|---|
| M25-AGENT-1 | Agent run p99 duration | < timeout (60 min default) |
| M25-AGENT-2 | Agent cost cap enforcement | 100 % |
| M25-AGENT-3 | Sandbox escape attempts detected | 100 % alerted |
| M25-PIN | Pin tests | +90 GREEN |

### 11.6 Statut "expérimental"

M25 = **experimental** = :
- Off par défaut (`agentRunner.enabled=false` dans Helm).
- Single design partner pilot (Q4 2027).
- Pas de SLA contract.
- Audit pass annuel obligatoire.
- Sunset si adoption < 2 tenants à 12 mois.

### 11.7 Activation flags M25

**Default state** : OFF total. Triple opt-in : Helm chart, server,
tenant. **Requires M21 ON**. Le service `efe-agent-runner` n'est pas
inclus dans l'image Docker mcp-server ; c'est un image et un Helm
chart séparés (`efe-agent-runner` chart).

| Niveau | Flag | Default | Description |
|---|---|:---:|---|
| Helm | `agentRunner.enabled` | `false` | Si `false`, **CRDs non installées**, deployment non créé. Pas de footprint K8s. |
| Helm | `agentRunner.namespaceIsolation` | `strict` | `strict` / `relaxed`. `strict` (default) = NetworkPolicy deny-all egress sauf whitelist, seccomp, readonly rootfs. |
| Helm | `agentRunner.image.allowList` | `[official]` | Whitelist d'images autorisées. Default = uniquement image officielle EFE. |
| Server | `EFE_FEATURE_AGENT_RUNTIME_ENABLED` | `false` | Si `false`, RPC `CreateAgentRun` retourne `UNIMPLEMENTED`. |
| Server | `EFE_FEATURE_AGENT_RUNTIME_REQUIRE_M21` | `true` | Validation startup : refuse boot si M25 ON sans M21. |
| Tenant | `tenant_feature_m25.enabled` | `false` | Per-tenant. |
| Tenant | `tenant_feature_m25.cost_cap_per_run_usd` | `0` | Cap = 0 → tous AgentRuns rejetés. Tenant doit définir cap > 0. |
| Tenant | `tenant_feature_m25.timeout_minutes_max` | `60` | Hard cap sur durée max d'un run. |
| Tenant | `tenant_feature_m25.allowed_tools` | `[]` | Whitelist obligatoire. Vide = aucun tool callable par l'agent. |
| Tenant | `tenant_feature_m25.allowed_providers` | `[]` | Quels LLM providers le sandbox peut appeler. Empty = none. |
| Per-run | K8s `AgentRun` resource | — | Création explicite par opérateur via API ou UI. Pas d'auto-création. |

**Validation startup** : si `EFE_FEATURE_AGENT_RUNTIME_ENABLED=true`
sans CRDs installées dans le cluster, le mcp-server log un FATAL et
exit. Pas d'état dégradé silencieux.

**Test pin** :
- `test_default_off_m25_no_crd_installed`.
- `test_m25_requires_helm_chart_separate`.
- `test_m25_requires_m21_active`.
- `test_m25_empty_tools_whitelist_rejects_agent_run`.

---

## 12. LLMRouter abstraction

### 12.1 Hierarchy

```
LLMRouter (M21 livré)
   ├── Provider Pool
   │    ├── AnthropicProvider
   │    ├── OpenAIProvider
   │    ├── BedrockProvider
   │    ├── AzureOpenAIProvider
   │    ├── VertexProvider
   │    ├── VLLMProvider (self-host)
   │    ├── MistralProvider
   │    └── (M22+) HuggingFaceTGIProvider
   │
   ├── CostGovernor (per-tenant budget)
   ├── Cache (Redis exact + semantic)
   ├── CircuitBreaker (per-provider)
   ├── PromptRegistry (versioned templates)
   ├── AuditEmitter (M14-bis HMAC chain)
   └── ObservabilityHooks (M19 OTel + Prometheus)
```

### 12.2 Provider selection algorithm

```rust
fn select_providers(&self, request: &RouterRequest) -> Vec<Arc<dyn LLMProvider>> {
    let tenant_config = self.tenant_config(&request.tenant_id);
    
    // 1. Filter by tenant policy (allowed providers)
    let allowed: Vec<_> = self.providers.iter()
        .filter(|p| tenant_config.allowed_providers.contains(p.name()))
        .collect();
    
    // 2. Filter by capability (tool support, context size)
    let capable: Vec<_> = allowed.into_iter()
        .filter(|p| p.supports_tools() >= request.requires_tools)
        .filter(|p| p.max_context() >= request.estimated_tokens)
        .collect();
    
    // 3. Filter by circuit state
    let healthy: Vec<_> = capable.into_iter()
        .filter(|p| self.circuit.is_closed(p.name()))
        .collect();
    
    // 4. Sort by tenant preference :
    //   - cost-optimized : ascending cost
    //   - quality-optimized : pre-defined quality rank
    //   - latency-optimized : p50 latency
    let sorted = match tenant_config.optimize_for {
        OptimizeFor::Cost => sort_by_cost(healthy),
        OptimizeFor::Quality => sort_by_quality_rank(healthy),
        OptimizeFor::Latency => sort_by_p50(healthy),
        OptimizeFor::Balanced => sort_by_blend(healthy, 0.5, 0.5),
    };
    
    sorted
}
```

### 12.3 Per-tenant config

```sql
CREATE TABLE tenant_llm_config (
    tenant_id              TEXT PRIMARY KEY,
    allowed_providers      TEXT[] NOT NULL,
    optimize_for           TEXT NOT NULL DEFAULT 'balanced',
    -- 'cost' | 'quality' | 'latency' | 'balanced'
    primary_provider       TEXT,
    fallback_providers     TEXT[] NOT NULL,
    max_context_tokens     INT DEFAULT 16384,
    require_baa            BOOLEAN DEFAULT false,
    require_eu_residency   BOOLEAN DEFAULT false,
    require_no_training    BOOLEAN DEFAULT true,
    -- providers must guarantee they don't train on customer data
    prompt_redaction_level TEXT DEFAULT 'aggressive',
    -- 'none' | 'mild' | 'aggressive' (PII scrubbing)
    audit_full_prompts     BOOLEAN DEFAULT false
    -- if true, prompts (after redaction) stored in audit chain
);
```

---

## 13. Cost governance

### 13.1 Budget tiers (recommended defaults)

| Tier | Monthly cap | Mode-2 calls allowed | Use case |
|---|---|---|---|
| **Free** | $0 | 0 (Mode-1 only) | Dev/POC |
| **Starter** | $50 | ~5 000 (Gemini Flash) | Small teams |
| **Pro** | $500 | ~50 000 (Sonnet 4.5) | Mid market |
| **Enterprise** | custom (>$5k) | unlimited within cap | Enterprise |
| **Custom self-host** | infra-only | unlimited | Compliance-heavy (BFSI, gov) |

### 13.2 Hard cap enforcement

Dès que `spent_this_month >= monthly_cap`, comportement selon
`fail_open` :
- `fail_open=false` (default) : tous les calls Mode-2/3 retournent
  `BUDGET_EXHAUSTED` HTTP 429. Mode-1 reste disponible.
- `fail_open=true` : Mode-2/3 calls fall back silently à Mode-1
  avec un `degraded_mode_warning` flag dans la response.

### 13.3 Soft threshold (80 %)

À 80 % du cap mensuel, audit event `budget_warning_emitted` :
- `efe_get_llm_budget` retourne `warning: true`
- Slack/email notification au tenant admin (via tenant
  notification settings)
- Dashboard widget `M19-LLM-BUDGET` passe orange

### 13.4 Cost prediction

Pre-flight : `estimated_cost = (estimated_tokens_in × provider.cost_per_1m_in / 1e6)
                                + (estimated_tokens_out × provider.cost_per_1m_out / 1e6)`.

Tokens estimés via `tiktoken` (OpenAI) ou `anthropic.count_tokens`
(Anthropic) ou approximation `len(text) / 4` (fallback).

### 13.5 Cost reporting

Per-tenant monthly report (auto-generated, M19 observability ingest) :
- Total spent
- Breakdown by provider
- Breakdown by tool (`classify_mode2`, `propose_split`, ...)
- Cache hit rate (effective savings)
- Top 10 most expensive calls

Stocké dans `crates/efe-server/src/reports/monthly_llm_*.rs`,
exposé via `/admin/reports/llm/{tenant_id}/{YYYY-MM}.json`.

---

## 14. Privacy & compliance

### 14.1 GDPR Art. 22 (automated decision-making)

**Trigger** : Mode-2/3 = "automated processing including profiling
which produces legal effects". Conditions :
- Decision based solely on automated processing → likely yes (route a
  support ticket = potentially significant).
- Includes profiling → no (we don't profile users, we classify content).

**Obligations** :
1. Data subject must be informed (via tenant's privacy notice).
2. Data subject has right to obtain human intervention.
3. EFE must log full reasoning trace (concept_stack for Mode-3).
4. DPIA required for high-risk processing.

**Implementation hooks** :
- `human_review_required` flag per tenant.
- `review_token` returned for Mode-3 responses requiring review.
- Audit chain HMAC includes full concept_stack.
- `efe_export_decision_log` mcp-server tool for Subject Access Requests.

### 14.2 EU AI Act 2024/1689

Mode-2/3 classify : likely **limited risk** if used for general
content classification, **high risk** if used for :
- Education / vocational training scoring.
- Critical infrastructure (healthcare triage).
- Justice / asylum processing.

**Customer-side determination**. EFE provides :
- Conformity assessment template (`docs/compliance/ai_act_assessment_template.md`).
- Data sheet (`docs/compliance/ai_act_data_sheet.md`).
- Risk classification helper (`efe_assess_ai_act_risk` mcp-server tool).
- Logs sufficient for traceability requirement (Art. 12).

### 14.3 Data residency

- **EU residency** : tenants with `require_eu_residency=true` only
  use EU-hosted providers : Anthropic EU (Paris/Frankfurt), Mistral
  (FR), Azure OpenAI EU, Vertex EU, vLLM self-host EU.
- **US residency** : default.
- **No residency constraint** : best price/latency provider wins.

LLMRouter enforces at provider selection time (§ 12.2 step 1
extended).

### 14.4 No training (data minimization)

Tous les contrats LLM provider doivent stipuler : "customer data NOT
used for training". Status par provider :

| Provider | Default no-training | Contract clause |
|---|:---:|---|
| Anthropic | ✅ (default API) | Anthropic API Terms § Data |
| OpenAI Standard | ⚠️ (opt-out via dashboard) | Org-level setting |
| OpenAI Enterprise | ✅ (default) | EA Terms |
| AWS Bedrock | ✅ (no training) | AWS Service Terms § Bedrock |
| Azure OpenAI | ✅ (no training) | Azure Service Terms |
| Vertex AI | ✅ (default) | Google Cloud Terms |
| Mistral La Plateforme | ✅ (default) | Mistral Terms |
| vLLM self-host | ✅ (you control) | N/A |

`require_no_training=true` (default) → router disables non-compliant
providers automatically.

### 14.5 BAA (HIPAA)

Pour tenants healthcare :

| Provider | BAA available | Region required |
|---|:---:|---|
| Anthropic via AWS Bedrock | ✅ | us-east-1, us-west-2 |
| Anthropic direct | ✅ (Enterprise contract) | us only |
| OpenAI Enterprise | ✅ | us only |
| Azure OpenAI | ✅ | configurable |
| Vertex AI | ✅ | configurable |
| Mistral | ❌ (en discussion) | — |

`require_baa=true` → only BAA-covered providers.

### 14.6 PII redaction (prompt scrubbing)

Trois niveaux :

| Level | Behavior |
|---|---|
| `none` | Send sentence as-is to LLM |
| `mild` | Redact email, phone, SSN via regex (M14 carry-over) |
| `aggressive` | Redact + use Microsoft Presidio (NER-based) |

`aggressive` adds ~30 ms latency but strongest compliance posture.

---

## 15. Multi-tenant isolation

### 15.1 Hard isolations

| Aspect | Mechanism |
|---|---|
| Cache namespace | `efe:llm:cache:{tenant_id}:*` (M19 carry-over) |
| Budget tracking | `tenant_llm_budget` per-tenant row |
| Audit chain | per-tenant chain (M14-bis carry-over) |
| Provider config | `tenant_llm_config` per-tenant row |
| Prompt logs | per-tenant retention policy |
| Concurrent requests | per-tenant rate limit |

### 15.2 Cross-tenant leakage attack vectors

| Vector | Mitigation |
|---|---|
| Cache key collision | Cache key includes `tenant_id` always |
| Audit chain mix-up | M14-bis HMAC keyed per-tenant |
| Provider context bleeding | Each call is a fresh context, no session state |
| Prompt template variable leakage | Templates validated for tenant-only variables |
| Log aggregation cross-tenant | M19 metrics use `tenant_hash`, not `tenant_id` |

### 15.3 Compliance test pin

`test_isolation_m21_cross_tenant_cache.py` :
- Tenant A classifies sentence X via Mode-2.
- Tenant B classifies same sentence X.
- Assert : two distinct LLM calls (no cache hit) AND results may differ.
- Assert : audit chains separate.

---

## 16. Fallback patterns

### 16.1 Cascade fallback

```
LLM primary → fail
   ↓
LLM fallback 1 → fail
   ↓
LLM fallback 2 → fail
   ↓
Mode-1 (embedder) → always available
   ↓
(if M21 cache available) → cached Mode-2 response
   ↓
Error 503 (last resort)
```

### 16.2 Circuit breaker per-provider

```rust
pub struct CircuitBreaker {
    state: HashMap<String, CircuitState>,
    failure_threshold: u32,  // 5 consecutive failures
    open_duration: Duration,  // 60s before trying half-open
}

pub enum CircuitState {
    Closed { success_count: u32 },
    Open { opened_at: Instant },
    HalfOpen,
}
```

### 16.3 Timeout strategy

| Scenario | Timeout |
|---|---|
| Mode-2 single call | 5 s |
| Mode-3 single turn | 5 s × max_turns |
| Provider HTTP | 10 s |
| Cache lookup | 50 ms |

Beyond timeout → fail, trigger fallback.

### 16.4 Graceful degradation

- LLM router fully down → all Mode-2/3 calls fall back to Mode-1.
- Mode-1 latency goes from 200 ms → 200 ms (no impact).
- `efe_classify_mode2` returns `mode=MODE_1` + `degraded=true` flag.
- Operator dashboard shows red banner.

---

## 17. Vendor lock-in mitigation

### 17.1 Stratégies anti-lock-in

| Strategy | Implementation |
|---|---|
| Multi-provider abstraction | LLMProvider Protocol + 8 implementations |
| No vendor-specific prompt features | Templates avoid Anthropic-only or OpenAI-only syntax |
| Self-host fallback | vLLM + llama.cpp always supported |
| Standard formats only | OpenAI tool format → adapter to vendor format |
| Cost transparency | Customer can see per-provider cost, switch trivially |

### 17.2 Provider migration runbook

`docs/runbooks/m21_migrate_llm_provider.md` :
1. Add new provider config in `tenant_llm_config.allowed_providers`.
2. Set `primary_provider` to new.
3. Run shadow eval : 5 % traffic A/B for 1 week.
4. Verify SLOs maintained.
5. Promote to 100 %.

Migration time : 1-2 weeks (eval-bound). Total operator effort : ~4 h.

### 17.3 Open-weight escape hatch

If all commercial providers become unacceptable :
1. Provision GPU cluster (H100 × 4 minimum).
2. Deploy vLLM + Llama 4 70B Q5.
3. Set `primary_provider=vllm-local`.
4. Migration time : 1 week (infra) + shadow eval.

Cost : ~$3-5k/month GPU vs ~$5-15k/month frontier API for same volume.
Break-even ~10k Mode-2 calls/day.

---

## 18. Threat model M20+ (delta vs M19)

### 18.1 New threats (~12 over M20-M25)

| ID | STRIDE | Threat | Sev | Phase |
|---|:---:|---|:---:|:---:|
| TM-M21-S-01 | Spoof | Tenant impersonation via tenant_id forge in budget | HIGH | M21 |
| TM-M21-T-01 | Tamper | Cache poisoning (malicious entry hits multiple users) | MED | M21 |
| TM-M21-R-01 | Repud. | Tenant denies LLM call cost ("I never made that request") | HIGH | M21 |
| TM-M21-I-01 | InfoDisc | Prompt injection extracts tenant data via LLM | HIGH | M21 |
| TM-M21-D-01 | DoS | Tenant spams Mode-2 to drain budget DOS competitors | MED | M21 |
| TM-M21-E-01 | Elev. | Prompt injection extracts router secrets / KMS access | CRITICAL | M21 |
| TM-M22-T-01 | Tamper | Concept stack replay attack | MED | M22 |
| TM-M22-R-01 | Repud. | Subject denies decision ("I never agreed to autom.") | HIGH | M22 |
| TM-M23-I-01 | InfoDisc | Curation prompt leaks taxonomy IP cross-tenant | HIGH | M23 |
| TM-M24-T-01 | Tamper | Active learning loop poisoned by adversarial examples | HIGH | M24 |
| TM-M25-E-01 | Elev. | Agent runtime sandbox escape | CRITICAL | M25 |
| TM-M25-D-01 | DoS | Long-running agent exhausts cluster budget | HIGH | M25 |

### 18.2 Mitigations résumées

| Threat | Mitigation |
|---|---|
| TM-M21-S-01 | Tenant_id derived from JWT claim, never user-supplied |
| TM-M21-T-01 | Cache entries signed HMAC ; entries rejected if HMAC invalid |
| TM-M21-R-01 | Audit chain HMAC + monthly statement signed by EFE |
| TM-M21-I-01 | System prompt enforces "do not echo data from previous calls" + redaction |
| TM-M21-D-01 | Per-tenant rate limit on Mode-2 calls (separate from budget) |
| TM-M21-E-01 | Router runs in separate service process ; no KMS creds in router context |
| TM-M22-T-01 | Concept_stack signed at each turn |
| TM-M22-R-01 | Human review tokens unforgeable + audit |
| TM-M23-I-01 | Per-tenant prompt scope + log review |
| TM-M24-T-01 | Anomaly detection on gold_label distribution + drift alerts |
| TM-M25-E-01 | NetworkPolicy + seccomp + readonly rootfs + no privileged |
| TM-M25-D-01 | Hard timeout 60 min + cost cap 50$/run |

### 18.3 Cumulative threat model post-M25

| Milestone | New | Cumulated |
|---|---:|---:|
| M16-M18 | 28 | 28 |
| M19 | +9 | 37 |
| M20 | +0 | 37 |
| M21 | +6 | 43 |
| M22 | +2 | 45 |
| M23 | +1 | 46 |
| M24 | +1 | 47 |
| M25 | +2 | 49 |

49 STRIDE threats post-M25 GA. Pen-test annuel obligatoire.

---

## 19. Backend contract evolution

### 19.1 Version sequence

| Version | Date | Milestone | Type | Highlights |
|---|---|---|:---:|---|
| EFE 1.0 | Q1 2026 | M1-M14 GA | — | Mode-1, multi-tenant, OIDC |
| EFE 1.1 | Q2 2026 | M15 | minor | Bench v1, observability dashboards |
| EFE 1.2 | Q2 2026 | M16-M18 | minor | MCP server tools surface |
| EFE 1.3 | Q3 2026 | M19 | minor | Sigstore + BYOK + 3 admin endpoints |
| **EFE 1.4** | **Q4 2026** | **M21** | **minor** | **Mode-2 + LLMRouter** |
| EFE 1.5 | Q1 2027 | M22 | minor | Mode-3 + concept_stack |
| EFE 1.6 | Q2 2027 | M23 | minor | Curation prompts + A/B infra |
| EFE 1.7 | Q3 2027 | M24 | minor | Active learning + gold_dataset |
| **EFE 2.0** | **Q4 2027** | **M25** | **MAJOR** | **agent-runner service + CRDs** |

### 19.2 EFE 1.4 diff (Mode-2)

```protobuf
// crates/efe-proto/v1.4/swarm.proto
service EfeSwarmService {
  // existing ...
  rpc ClassifyMode2(ClassifyMode2Request) returns (ClassifyMode2Response);
  rpc StreamClassifyMode2(ClassifyMode2Request) returns (stream ClassifyMode2Update);
}

service EfeAdminService {
  // existing ...
  rpc GetTenantLLMBudget(GetBudgetRequest) returns (LLMBudget);
  rpc SetTenantLLMBudget(SetBudgetRequest) returns (LLMBudget);
  rpc ListLLMProviders(google.protobuf.Empty) returns (LLMProviderList);
  rpc GetLLMCacheStats(google.protobuf.Empty) returns (LLMCacheStats);
  rpc InvalidateLLMCache(InvalidateCacheRequest) returns (google.protobuf.Empty);
}
```

### 19.3 Backward compat policy

- EFE 1.x clients work against EFE 2.0 server (1.x RPCs preserved).
- New features = new RPCs, never breaking change to existing.
- Server detects client version via `EFE-Client-Version` header
  (carry-over D14).
- Server can warn if client uses deprecated RPC (none in 1.x).
- EFE 2.0 = MAJOR only because agent-runner is fundamentally a new
  service shape (CRDs, RBAC).

### 19.4 Sunset policy

- Bug fixes : N-2 versions (e.g. when 1.6 ships, 1.4 still supported).
- Security fixes : all currently-deployed versions.
- New features : only N (latest).

---

## 20. Risk register

| ID | Risk | Sev | Likelihood | Phase | Mitigation |
|---|---|:---:|:---:|:---:|---|
| R-01 | LLM provider price hike | MED | HIGH | All | Multi-provider abstraction ; renegotiate every 6 months |
| R-02 | EU AI Act non-compliance | HIGH | MED | M21+ | DPIA + conformity assessment + dedicated compliance role |
| R-03 | Cost overrun (tenant-level) | MED | HIGH | M21+ | Hard budget caps + monitoring |
| R-04 | Vendor lock-in to one provider | HIGH | LOW | All | Multi-provider abstraction + self-host fallback ready |
| R-05 | Performance regression Mode-1 from Mode-2 noise | HIGH | MED | M21 | Dedicated thread pool ; SLO M19-MODE1-* preserved |
| R-06 | Prompt injection extracts data | HIGH | MED | M21+ | System prompt hardening + redaction + audit log review |
| R-07 | Cache poisoning | MED | LOW | M21+ | HMAC-signed cache entries + per-tenant namespace |
| R-08 | LLM hallucination produces wrong labels | MED | HIGH | M21+ | Confidence threshold + Mode-1 fallback |
| R-09 | Active learning poisoned (M24) | HIGH | LOW | M24 | Anomaly detection + drift alerts + human review |
| R-10 | Agent runtime sandbox escape (M25) | CRITICAL | LOW | M25 | NetworkPolicy + seccomp + readonly rootfs |
| R-11 | GDPR Art. 22 lawsuit | HIGH | LOW | M21+ | Human review path + audit chain + DPIA |
| R-12 | Compliance officer turnover during AI Act review | MED | MED | M21 | Documentation + 2 backups |
| R-13 | LLM provider goes bankrupt | LOW | LOW | All | Multi-provider always ; data not stuck with one provider |
| R-14 | Open-weight catches up to frontier (good news) | LOW | HIGH | M22+ | Self-host becomes default → cost drops 10× |
| R-15 | EU AI Act tightens (Acts 2.0 in 2028) | MED | MED | M22+ | Architecture flexible enough to absorb new requirements |

---

## 21. Open questions / decisions needed

### 21.1 Stratégique

1. **Q-01** : Mode-2 default ON or OFF for new tenants ?
   - Pro ON : better UX, attracts customers.
   - Pro OFF : safer, lower cost surprises, simpler compliance.
   - **Recommendation** : OFF, opt-in. Aligned with M19 pattern.

2. **Q-02** : Self-host vLLM as official supported product or
   "best-effort community" ?
   - Pro official : differentiates EFE vs API-only competitors.
   - Pro community : less support burden.
   - **Recommendation** : official Q1 2027 (M22 timing) once
     open-weight quality confirmed.

3. **Q-03** : Active learning (M24) — opt-in per tenant or always-on
   for tenants with > X gold labels ?
   - **Recommendation** : opt-in, always-on for paid tiers post-pilot.

4. **Q-04** : Agent runtime (M25) — bundled with EFE 2.0 server image
   or separate Helm chart ?
   - **Recommendation** : separate Helm chart, opt-in. Follows M19
     pattern.

### 21.2 Architecturales

5. **Q-05** : LLMRouter in same service as classify, or separate
   `efe-llm-router` microservice ?
   - Pro same : simpler deploy.
   - Pro separate : isolation, independent scaling, easier to audit.
   - **Recommendation** : separate (the security argument wins).

6. **Q-06** : Prompt registry — versioned in code (Cargo crate) or
   in Postgres for hot-reload ?
   - Pro code : reproducible, signed bundles, auditable.
   - Pro Postgres : faster iteration.
   - **Recommendation** : code (signed in bundles), Postgres only for
     A/B test ratios.

7. **Q-07** : Audit chain for LLM calls — extend M14-bis HMAC chain
   or separate `llm_audit_chain` ?
   - **Recommendation** : extend M14-bis with new event types,
     consistent posture.

8. **Q-08** : Caching semantic (kNN) — Redis Stack (RediSearch) or
   pgvector ?
   - Pro Redis Stack : low-latency, M19 carry-over.
   - Pro pgvector : transactional with audit.
   - **Recommendation** : Redis Stack for hot cache, pgvector for
     post-mortem analytics.

### 21.3 Compliance / business

9. **Q-09** : Default provider for new tenants ?
   - Anthropic (quality, BAA) vs Gemini Flash (cost) vs Bedrock (AWS-native).
   - **Recommendation** : Bedrock Claude Sonnet (BAA + balance).

10. **Q-10** : EFE pricing model adjustment for Mode-2/3 ?
    - Existing : flat per-classify $0.005 (Mode-1).
    - Mode-2 : passthrough cost + 20 % EFE margin ?
    - Mode-3 : passthrough × turns + 20 % ?
    - **Recommendation** : passthrough + 30 % for Mode-2, passthrough
      + 50 % for Mode-3 (covers concept_stack overhead).

---

## 22. Success metrics

### 22.1 Per-milestone

| Milestone | Primary KPI | Target |
|---|---|---|
| M20 | Agent SDK adoption | 5+ pilot teams using `efe-agent` Python lib |
| M21 | Mode-2 traffic share | ≥ 5 % of total classifies (across all tenants) |
| M22 | Mode-3 conversion | ≥ 20 % of low-confidence Mode-2 cases escalate to Mode-3 |
| M23 | Curation tool DAU | ≥ 30 % of operators use curation tools weekly |
| M24 | Active learning impact | ≥ 2 pp accuracy improvement on top-10 catalogs |
| M25 | Agent runs successful | ≥ 80 % runs complete within timeout/budget |

### 22.2 Cumulative business

| Metric | Pre-M20 | Post-M25 target |
|---|---|---|
| Active customers | 50 | 200 |
| Monthly classifies | 100 M | 1 B |
| Mode-2/3 share | 0 % | 15 % |
| Avg revenue per customer | $5k/month | $15k/month |
| EU AI Act customers | 0 (no use case) | 30+ |
| Pin tests cumulative | 522 | ~1000 |

### 22.3 Quality

| Metric | Target |
|---|---|
| Mode-2 accuracy vs human | ≥ 95 % agreement on holdout |
| Mode-3 convergence rate | ≥ 80 % within max_turns |
| Provider availability blended | ≥ 99.5 % |
| Cost predictability | ≤ 10 % variance vs forecast |

---

## 23. Glossary M20+

- **Mode-2** : EFE classify avec anchors LLM-routed (vs Mode-1 anchors compilés).
- **Mode-3** : Mode-2 + multi-turn iterative refinement avec concept_stack.
- **LLMRouter** : service d'abstraction multi-provider qui gère cost,
  cache, circuit breaker, audit.
- **CostGovernor** : per-tenant budget enforcer, hard cap.
- **Concept stack** : trace d'un Mode-3 (turn-by-turn reasoning).
- **Prompt registry** : catalogue de templates versionnés.
- **A/B test infra** : promotion canary sur prompts.
- **Active learning** : boucle production → low-conf → LLM gold → training.
- **Agent runtime** : service tournant des LLM agents in-cluster, sandbox.
- **BYO-LLM** : Bring Your Own LLM ; le client fournit son provider.
- **Pattern A/B/C/D** : LLM-as-client / classifier / curator / agent.
- **DPIA** : Data Protection Impact Assessment (GDPR Art. 35).
- **BAA** : Business Associate Agreement (HIPAA).
- **Conformity assessment** : EU AI Act self-attestation by provider.
- **Stakes tier** : per-catalog risk classification (low/medium/high)
  driving human-review requirements.

---

## 24. References

- **EU AI Act 2024/1689** :
  <https://eur-lex.europa.eu/eli/reg/2024/1689/oj>
- **GDPR Art. 22** :
  <https://gdpr-info.eu/art-22-gdpr/>
- **NIST AI RMF 1.0** :
  <https://www.nist.gov/itl/ai-risk-management-framework>
- **Google SRE Workbook** (SLO error budget patterns).
- **Anthropic API Terms of Service** (Sonnet 4.5 / Opus 4.7 reference).
- **OpenAI Enterprise Terms** (no-training default).
- **AWS Bedrock Service Terms** (BAA reference).
- **Sigstore Sigstore-Python** (M19 carry-over).
- **Postgres `SELECT FOR UPDATE`** (cost governor concurrency).
- **Redis Stack RediSearch + RedisVL** (semantic cache).
- **vLLM project** : <https://github.com/vllm-project/vllm>.
- **llama.cpp** : <https://github.com/ggerganov/llama.cpp>.

---

## A — LLM provider matrix

### A.1 Hosted (commercial)

| Provider | Models 2026 | $/1M in | $/1M out | BAA | EU residency | No-training default | Latency p50 | SLA |
|---|---|---:|---:|:---:|:---:|:---:|---:|---:|
| Anthropic (direct) | Claude Opus 4.7, Sonnet 4.5, Haiku 4 | $3-15 | $15-75 | ✅ Ent | US only | ✅ | 300-800 ms | 99.5 % |
| OpenAI (direct) | GPT-5, GPT-5-mini | $0.30-10 | $1.20-40 | ✅ Ent | Limited | ✅ Ent | 200-600 ms | 99.5 % |
| AWS Bedrock | Claude family, Titan, Llama | passthrough | passthrough | ✅ | configurable | ✅ | 350-900 ms | 99.9 % |
| Azure OpenAI | GPT-5 (regional) | passthrough | passthrough | ✅ | configurable | ✅ | 250-700 ms | 99.9 % |
| Vertex AI | Gemini 2.5 Pro/Flash, Claude (via partner) | $0.075-15 | $0.30-75 | ✅ | configurable | ✅ | 200-700 ms | 99.5 % |
| Mistral La Plateforme | Mistral Large 3 | $2 | $6 | ❌ | EU only | ✅ | 250-700 ms | 99.5 % |

### A.2 Self-host

| Provider | Model | Hardware | $/h | Latency p50 | Pros | Cons |
|---|---|---|---:|---:|---|---|
| vLLM | Llama 4 70B Q5 | 1× H100 | $3-5 | 80 ms | Speed, cost | GPU shortage |
| vLLM | Mistral Large 3 (open) | 2× H100 | $6-10 | 100 ms | EU sovereign | Heavier |
| vLLM | Qwen 3 72B | 1× H100 | $3-5 | 80 ms | Multilingual | License terms |
| llama.cpp | Llama 4 70B Q4 | M3/M4 Max 128GB | $0 (existing) | 800 ms | Cheap, local | Slow |
| llama.cpp | Phi-4 14B Q5 | RTX 4090 | $0 (existing) | 400 ms | Edge | Lower quality |
| MLX | Llama 4 70B Q4 | M4 Max | $0 | 600 ms | Mac native | Mac only |
| HuggingFace TGI | Any HF model | configurable | varies | varies | HF ecosystem | Complexity |

### A.3 Selection guidelines

| Use case | Recommended primary | Recommended fallback |
|---|---|---|
| Cost-sensitive | Gemini 2.5 Flash | OpenAI GPT-5-mini |
| Quality-sensitive | Anthropic Claude Sonnet 4.5 | OpenAI GPT-5 |
| BAA required | AWS Bedrock Claude | Azure OpenAI GPT-5 |
| EU residency | Mistral Large 3 (Mistral) | Anthropic EU (Bedrock eu-) |
| Self-host (cost-driven) | vLLM Llama 4 70B | llama.cpp Llama 4 70B Q4 |
| Edge/local dev | llama.cpp Phi-4 | MLX Llama 4 70B |

---

## B — Cost model

### B.1 Mode-2 cost per classify (all providers, 200 in / 50 out tokens)

| Provider | $/classify | Annual cost @ 1M classifies |
|---|---:|---:|
| Anthropic Opus 4.7 | $0.0067 | $6,700 |
| Anthropic Sonnet 4.5 | $0.0014 | $1,400 |
| Anthropic Haiku 4 | $0.00040 | $400 |
| OpenAI GPT-5 | $0.0040 | $4,000 |
| OpenAI GPT-5-mini | $0.00012 | $120 |
| Vertex Gemini 2.5 Pro | $0.0023 | $2,300 |
| Vertex Gemini 2.5 Flash | $0.000030 | $30 |
| Mistral Large 3 | $0.00070 | $700 |
| vLLM Llama 4 70B (1× H100, sustained) | $0.000015 | $15 |

### B.2 Mode-3 cost (3 turns, 600 in / 150 out tokens × 3)

Multiplied by 3-5× depending on convergence.

| Provider | $/Mode-3 (3 turns) | Annual @ 100k Mode-3 calls |
|---|---:|---:|
| Anthropic Sonnet 4.5 | $0.0042 | $420 |
| Vertex Gemini 2.5 Flash | $0.000090 | $9 |
| vLLM Llama 4 70B | $0.000045 | $4.50 |

### B.3 Self-host break-even (vLLM)

Hardware : 1× H100 + 96 GB RAM + ~50 GB SSD = ~$3-5/h cloud or
~$30k CapEx + $300/month colo.

Break-even (vs API @ Sonnet 4.5) :
- $3/h × 24 × 30 = $2,160/month
- @ $0.0014 / classify : $2,160 / $0.0014 = **1,540,000 classifies/month**
- ≥ 50k/day → self-host is cheaper.

### B.4 Total platform LLM cost projection

Hypothèses :
- 200 customers, average 50k Mode-2 calls/month each = 10M Mode-2/mo.
- 5 % of Mode-2 escalate to Mode-3 = 500k Mode-3/mo.
- Provider mix : 60 % Sonnet 4.5, 30 % Gemini Flash, 10 % vLLM.

| Item | Volume | Unit cost | Monthly |
|---|---:|---:|---:|
| Mode-2 Sonnet | 6 M | $0.0014 | $8,400 |
| Mode-2 Gemini Flash | 3 M | $0.000030 | $90 |
| Mode-2 vLLM | 1 M | $0.000015 | $15 |
| Mode-3 (3 turns avg) | 500 k | avg $0.002 | $1,000 |
| Cache savings (60 % hit) | -60 % | — | -$5,700 |
| **Total LLM opex** | — | — | **~$3,800/mo** |

Per customer : ~$19/month LLM opex. Marginal vs subscription.

---

## C — Prompt library skeleton

### C.1 Naming convention

`{purpose}_{phase}_{version}.md` :
- `mode2_anchor_gen_v1.md`
- `mode3_multi_turn_v1.md`
- `curation_propose_split_v1.md`

### C.2 Header schema

```yaml
---
name: mode2_anchor_gen_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm]
min_context_tokens: 4000
max_response_tokens: 200
estimated_cost_per_call_usd: 0.0014  # Sonnet 4.5
created: 2026-10-15
author: tech_lead
eval_pass: true
eval_dataset: gold_dataset_v1.0
eval_accuracy: 0.93
eval_avg_latency_ms: 480
notes: |
  Improvement over v0 : explicit constraint that anchors must come
  from candidate set ; reduces hallucination 12 pp.
---
```

### C.3 Body skeleton

```markdown
# System
You are an expert in {{ catalog_domain }}. You select the most
appropriate anchor concepts from a candidate set to classify
customer support sentences.

Rules :
- You MUST output exactly 3 anchors.
- Each anchor MUST come from the candidate set verbatim.
- Reasoning is optional, ≤ 50 words.

# User
Sentence : "{{ sentence }}"
Candidate anchors : {{ candidates_json }}

Output JSON :
{
  "anchors": ["...", "...", "..."],
  "reasoning": "..."
}
```

### C.4 Validation post-call

```python
def validate_mode2_output(raw: str, candidates: list[str]) -> Mode2Output:
    parsed = json.loads(raw)
    
    # 1. Schema check
    assert "anchors" in parsed and len(parsed["anchors"]) == 3
    
    # 2. Anchors must be from candidate set
    for a in parsed["anchors"]:
        if a not in candidates:
            raise InvalidLLMOutput(
                f"hallucinated anchor not in candidate set: {a}"
            )
    
    # 3. No duplicates
    assert len(set(parsed["anchors"])) == 3
    
    # 4. Reasoning truncated
    parsed["reasoning"] = parsed.get("reasoning", "")[:200]
    
    return Mode2Output(**parsed)
```

---

## D — Pin tests inventory roadmap

| Sprint | New tests | Cumulative |
|---|---:|---:|
| Pre-M20 (post-M19) | 0 | 522 |
| M20 D55-D58 | +35 | 557 |
| M21 D59-D64 | +120 | 677 |
| M22 D65-D69 | +80 | 757 |
| M23 D70-D73 | +60 | 817 |
| M24 D74-D79 | +100 | 917 |
| M25 D80-D85 | +90 | 1007 |

### D.1 New test files M20

- `test_efe_agent_loop.py` (10 tests)
- `test_llm_provider_anthropic.py` (cond., 5)
- `test_llm_provider_openai.py` (cond., 5)
- `test_llm_provider_fallback_chain.py` (8)
- `test_mcp_explain_classification_tool.py` (4)
- `test_mcp_compare_bundles_tool.py` (3)

### D.2 New test files M21

- `crates/efe-llm-router/tests/test_router_basic.rs` (10)
- `test_provider_anthropic.rs` (cond., 8)
- `test_provider_openai.rs` (cond., 8)
- `test_provider_bedrock.rs` (cond., 6)
- `test_provider_azure.rs` (cond., 6)
- `test_provider_vertex.rs` (cond., 6)
- `test_provider_vllm.rs` (cond., 6)
- `test_provider_mistral.rs` (cond., 6)
- `test_cost_governor.rs` (12)
- `test_cache_exact.rs` (8)
- `test_cache_semantic.rs` (10)
- `test_circuit_breaker.rs` (8)
- `test_prompt_registry.rs` (6)
- `test_audit_pass_m21.rs` (post-livraison, ~20)

### D.3 New test files M22

- `test_mode3_classifier.rs` (15)
- `test_concept_stack.rs` (10)
- `test_human_review_flow.rs` (12)
- `test_audit_art22_chain.rs` (8)
- `test_audit_pass_m22.rs` (post-livraison, ~15)

### D.4 New test files M23

- `test_curation_propose_split.py` (10)
- `test_curation_explain_miss.py` (8)
- `test_curation_generate_synth.py` (8)
- `test_curation_audit_vs_gold.py` (10)
- `test_prompt_ab_test.rs` (12)
- `test_audit_pass_m23.rs` (post-livraison, ~12)

### D.5 New test files M24

- `test_active_learning_pipeline.py` (15)
- `test_gold_label_queue.rs` (10)
- `test_drift_detection.rs` (8)
- `test_promotion_gates.rs` (10)
- `test_human_review_sla.rs` (6)
- `test_gdpr_right_to_forget.rs` (8)
- `test_audit_pass_m24.rs` (post-livraison, ~25)

### D.6 New test files M25

- `test_agent_run_lifecycle.rs` (15)
- `test_agent_sandbox_isolation.rs` (12)
- `test_agent_cost_cap.rs` (8)
- `test_agent_timeout_enforcement.rs` (6)
- `test_agent_tool_whitelist.rs` (10)
- `test_audit_pass_m25.rs` (post-livraison, ~20)

---

## E — Compliance attestation chain

### E.1 Per-milestone compliance artifacts

| Milestone | Artifact | Evaluator |
|---|---|---|
| M20 | DPIA addendum (BYO-LLM data flow) | DPO |
| M21 | EU AI Act conformity assessment v1 | External counsel |
| M21 | Updated `soc2_evidence.md` § LLM | Auditor |
| M22 | GDPR Art. 22 implementation note | DPO + counsel |
| M23 | Prompt library content review (no PII leakage) | DPO |
| M24 | Active learning ethical review | Ethics committee (external) |
| M25 | Agent runtime safety case | External red team |

### E.2 Annual recurring

- AI Act conformity reassessment (annually).
- DPIA review (annually OR upon material change).
- Pen-test (annually).
- Provider contract review (every 6 months).
- Threat model refresh (semi-annually).
- SLO review (quarterly).

### E.3 Sub-processor list (post-M25)

EFE 2.0 supported sub-processors (customer-choice, opt-in) :

| Sub-processor | Service | Purpose | DPA available |
|---|---|---|:---:|
| AWS | Bedrock, KMS, S3, SQS | Hosting + LLM access | ✅ |
| GCP | Vertex, KMS, Cloud Storage | Hosting + LLM access | ✅ |
| Azure | OpenAI, Key Vault, Blob | Hosting + LLM access | ✅ |
| Anthropic | Claude API | LLM provider | ✅ |
| OpenAI Enterprise | GPT API | LLM provider | ✅ |
| Mistral AI | La Plateforme | LLM provider | ✅ |
| Sigstore PGI | Rekor / Fulcio | Transparency | Public good (no DPA needed) |

EFE customers select which sub-processors apply via tenant config.

---

## F — Feature flag specification

> **Référence normative** pour toute la roadmap. Si un § M20-M25
> diffère de cette annexe, c'est cette annexe qui fait foi.

### F.1 Principes d'architecture

1. **Opt-in everywhere, default OFF**. Aucune capability M20-M25
   n'est activée silencieusement.
2. **Hiérarchie 4-niveaux** : server → tenant → catalog → request.
   AND-évalué : un seul niveau OFF ⇒ feature OFF.
3. **Audit complet** : chaque toggle est event HMAC-chained.
4. **Réversible** : OFF immédiat (TTL ≤ 60 s). Pas de "no going back".
5. **Discoverable** : tools MCP + resources + métriques exposent
   l'état flag pour observabilité.
6. **Préservation M19** : test pin formel `test_default_off_*`
   garantit qu'aucun flag activé ⇒ comportement EFE 1.3 GA bit-for-bit.
7. **Dépendances déclaratives** : M22 requires M21, M24 requires
   M21+M23, M25 requires Helm-CRDs + M21. Validation startup +
   API enforcement.
8. **Sécurité par défaut** : caps initiaux = 0 / [] / false ; tenant
   doit explicitement définir des valeurs non-triviales.

### F.2 Naming convention

| Niveau | Pattern | Exemple |
|---|---|---|
| Helm chart value | `{component}.{aspect}` | `llmRouter.enabled`, `agentRunner.namespaceIsolation` |
| Server env-var | `EFE_FEATURE_{MILESTONE}_{ASPECT}` | `EFE_FEATURE_MODE2_ENABLED`, `EFE_FEATURE_AGENT_RUNTIME_ENABLED` |
| Tenant DB row | `tenant_feature_{milestone}.{aspect}` | `tenant_feature_mode2.enabled`, `tenant_feature_m24.auto_promotion` |
| Catalog DB column | `catalog.{milestone}_{aspect}` | `catalog.mode2_enabled`, `catalog.al_drift_threshold_pp` |
| Request RPC param | (per-RPC contract) | `Classify(mode=MODE_2)`, `CreateAgentRun(...)` |

### F.3 Inventaire complet (tous flags M20-M25 cumulés)

#### F.3.1 Flags Helm

| Flag | Default | Milestone | Description |
|---|:---:|:---:|---|
| `llmRouter.enabled` | `false` | M21 | Deploy `efe-llm-router` service |
| `llmRouter.replicas` | `2` | M21 | HA si `llmRouter.enabled=true` |
| `llmRouter.cacheBackend` | `redis` | M21 | `redis` / `none` |
| `m20Helpers.bundled` | `false` | M20 | Inclure `efe-agent` lib dans image mcp-server |
| `agentRunner.enabled` | `false` | M25 | Deploy `efe-agent-runner` + CRDs |
| `agentRunner.namespaceIsolation` | `strict` | M25 | `strict` / `relaxed` |
| `agentRunner.image.allowList` | `[official]` | M25 | Whitelist images |
| `activeLearning.workerEnabled` | `false` | M24 | Deploy gold_label_queue worker |
| `activeLearning.retunePipelineEnabled` | `false` | M24 | Deploy retune cron |

#### F.3.2 Flags server (env-vars)

| Flag | Default | Milestone | Validation |
|---|:---:|:---:|---|
| `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` | `false` | M20 | — |
| `EFE_FEATURE_M20_AGENT_SDK_BUNDLED` | `false` | M20 | — |
| `EFE_FEATURE_MODE2_ENABLED` | `false` | M21 | requires `EFE_LLM_ROUTER_URL` non-empty |
| `EFE_LLM_ROUTER_URL` | `""` | M21 | required if Mode-2 ON |
| `EFE_FEATURE_MODE2_REQUIRE_TENANT_OPT_IN` | `true` | M21 | — |
| `EFE_FEATURE_MODE3_ENABLED` | `false` | M22 | requires Mode-2 ON |
| `EFE_FEATURE_MODE3_REQUIRE_MODE2` | `true` | M22 | — |
| `EFE_FEATURE_M23_CURATION_ENABLED` | `false` | M23 | — |
| `EFE_FEATURE_M23_USE_INTERNAL_LLM` | `false` | M23 | requires Mode-2 ON if `true` |
| `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED` | `false` | M24 | requires M21 + M23 |
| `EFE_FEATURE_M24_REQUIRE_M21_M23` | `true` | M24 | — |
| `EFE_FEATURE_AGENT_RUNTIME_ENABLED` | `false` | M25 | requires Helm CRDs + M21 |
| `EFE_FEATURE_AGENT_RUNTIME_REQUIRE_M21` | `true` | M25 | — |
| `EFE_FEATURE_TOGGLE_DRAIN_INFLIGHT` | `drain` | All | `drain` / `abort` |
| `EFE_FEATURE_AUDIT_TOGGLE_EVENTS` | `true` | All | toujours auditer toggles |

#### F.3.3 Flags tenant (DB rows)

Schema Postgres :

```sql
CREATE TABLE tenant_feature_flags (
    tenant_id           TEXT NOT NULL,
    milestone           TEXT NOT NULL,
    -- 'mode2' | 'mode3' | 'm20_helpers' | 'm23_curation' | 'm24' | 'm25'
    enabled             BOOLEAN NOT NULL DEFAULT false,
    config_jsonb        JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_toggled_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_toggled_by     TEXT,  -- JWT sub
    PRIMARY KEY (tenant_id, milestone)
);

CREATE INDEX ix_tenant_feature_flags_milestone
    ON tenant_feature_flags(milestone, enabled);
```

`config_jsonb` schema per-milestone (validé via JSON Schema) :

**M21 (mode2)** :
```json
{
  "allowed_providers": ["anthropic", "bedrock"],
  "monthly_cap_usd": 500,
  "fail_open": false,
  "require_baa": true,
  "require_eu_residency": false,
  "require_no_training": true,
  "prompt_redaction_level": "aggressive",
  "primary_provider": "bedrock",
  "fallback_providers": ["anthropic"],
  "optimize_for": "balanced"
}
```

**M22 (mode3)** :
```json
{
  "human_review_required": true,
  "max_turns": 3
}
```

**M23 (curation)** :
```json
{
  "allow_internal_llm": false
}
```

**M24 (active learning)** :
```json
{
  "auto_promotion": false,
  "human_review_required": true,
  "gold_label_threshold": 1000
}
```

**M25 (agent runtime)** :
```json
{
  "cost_cap_per_run_usd": 0,
  "timeout_minutes_max": 60,
  "allowed_tools": [],
  "allowed_providers": []
}
```

#### F.3.4 Flags catalog (DB columns)

Migrations additives sur `catalog` table :

```sql
-- M21 (Mode-2)
ALTER TABLE catalog ADD COLUMN mode2_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE catalog ADD COLUMN mode2_threshold_confidence REAL NOT NULL DEFAULT 0.0;
ALTER TABLE catalog ADD COLUMN mode2_max_anchor_candidates INT NOT NULL DEFAULT 10;

-- M22 (Mode-3)
ALTER TABLE catalog ADD COLUMN mode3_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE catalog ADD COLUMN stakes_tier TEXT NOT NULL DEFAULT 'low';
-- 'low' | 'medium' | 'high'

-- M24 (Active learning)
ALTER TABLE catalog ADD COLUMN al_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE catalog ADD COLUMN al_drift_threshold_pp REAL NOT NULL DEFAULT 2.0;
```

Toutes ces colonnes ont **default false / 0.0 / null** ⇒ ajout sans
break de compat schema.

#### F.3.5 Flags request (RPC params)

Already documented per-RPC :
- `Classify(mode: Mode = MODE_1)` — Mode enum default MODE_1.
- `Classify(mode: MODE_2 | MODE_AUTO)` for Mode-2 opt-in.
- `Classify(mode: MODE_3, max_turns: u32 = 3)` for Mode-3.
- `CreateAgentRun(...)` for M25.

### F.4 Évaluation logique (pseudo-code Rust)

```rust
pub struct FeatureContext {
    server_flags: ServerFlags,
    tenant_flags: TenantFlags,
    catalog_flags: CatalogFlags,
    request_mode: Mode,
}

pub fn is_mode2_active(ctx: &FeatureContext) -> Result<bool, FeatureError> {
    // 1. Server level
    if !ctx.server_flags.mode2_enabled {
        return Ok(false);
    }
    if ctx.server_flags.llm_router_url.is_empty() {
        return Err(FeatureError::ServerMisconfigured("llm_router_url empty"));
    }
    
    // 2. Tenant level
    if !ctx.tenant_flags.mode2.enabled {
        return Ok(false);
    }
    let cfg = &ctx.tenant_flags.mode2.config;
    if cfg.monthly_cap_usd <= 0.0 {
        return Ok(false);
    }
    if cfg.allowed_providers.is_empty() {
        return Ok(false);
    }
    
    // 3. Catalog level
    if !ctx.catalog_flags.mode2_enabled {
        return Ok(false);
    }
    if ctx.catalog_flags.mode2_threshold_confidence <= 0.0 {
        return Ok(false);
    }
    
    // 4. Request level
    let req_ok = matches!(
        ctx.request_mode,
        Mode::MODE_2 | Mode::MODE_AUTO,
    );
    if !req_ok {
        return Ok(false);
    }
    
    Ok(true)
}
```

Note : la fonction retourne `Result` pour distinguer "feature OFF
volontairement" (`Ok(false)`) de "feature mal configurée"
(`Err(...)`).

### F.5 Hiérarchie des dépendances inter-features

```
                  M19 (Mode-1) ─── always ON, base layer
                       │
            ┌──────────┼──────────┐
            ▼          ▼          ▼
          M20      M21 (Mode-2) ── (no dependency)
        (helpers)     │
                      │
          ┌───────────┼───────────┬─────────┐
          ▼           ▼           ▼         ▼
        M22        M23         M24       M25
       (Mode-3) (curation)  (active-l)  (agent)
                                │           │
                                ▼           ▼
                         requires M21+   requires Helm-
                         M23+M22 (or     CRDs + M21
                         direct gold     (independent of
                         labels)         M22/M23/M24)
```

**Validation API** : impossible d'activer un flag dont les deps
ne sont pas activées. `efe_set_tenant_feature` retourne
`dependency_not_active` si pré-requis manquant.

### F.6 Discoverability — observability flags

#### F.6.1 Tool MCP `efe_list_active_features`

Profile : `core` (read-only, accessible à tous).

```json
// Output schema
{
  "tenant_id_hash": "abc123de",
  "server_features": {
    "m20_helper_tools_enabled": false,
    "mode2_enabled": true,
    "mode3_enabled": false,
    ...
  },
  "tenant_features": {
    "mode2": { "enabled": true, "config": {...} },
    "mode3": { "enabled": false, "config": {} },
    ...
  },
  "active_resolution": {
    "mode2_active": true,
    "mode3_active": false,
    ...
  },
  "dependencies_satisfied": {
    "mode3": false  // why : tenant_flag mode3.enabled = false
  }
}
```

#### F.6.2 Resource `efe://server/features`

Read-only, profile `core`. Server-level flags only (no tenant data).

#### F.6.3 Resource `efe://tenant/{tenant_id}/features`

Read-only, profile `agent-native`. Tenant-scoped, requires
`tenant_id` match JWT claim.

#### F.6.4 Métrique Prometheus

```
# HELP efe_mcp_feature_active 1 if feature active for tenant
# TYPE efe_mcp_feature_active gauge
efe_mcp_feature_active{feature="mode2", tenant_hash="abc123de"} 1
efe_mcp_feature_active{feature="mode3", tenant_hash="abc123de"} 0
```

Cardinality : N tenants × M features. Avec 200 tenants × 6 features
= 1 200 series (sous le cap 10K M19-OBS-6).

#### F.6.5 Audit event `feature_toggled`

```json
{
  "event_type": "feature_toggled",
  "feature": "mode2",
  "level": "tenant",
  "tenant_id": "<tenant_id>",
  "previous_value": false,
  "new_value": true,
  "config_diff": {
    "monthly_cap_usd": [0, 500],
    "allowed_providers": [[], ["bedrock", "anthropic"]]
  },
  "actor": "<jwt_sub>",
  "ip_address": "203.0.113.42",
  "rationale": "<optional>",
  "timestamp": "2027-03-15T14:22:11.547Z",
  "hmac": "..."
}
```

Retention : 7 ans (compliance carry-over M14).

### F.7 Sécurité — guards anti-mauvaise-config

| Guard | Trigger | Behavior |
|---|---|---|
| Server flag ON, dependency OFF | startup validation | FATAL log + exit |
| Tenant flag ON, server flag OFF | runtime classify | `feature_disabled_at_server` 503 |
| Tenant flag ON, deps not satisfied | `efe_set_tenant_feature` API | reject `dependency_not_active` |
| Catalog flag ON, tenant flag OFF | runtime | feature OFF, no error (catalog opt is moot) |
| `auto_promotion=true` AND `stakes_tier=high` | `efe_set_tenant_feature` | reject `compliance_violation` |
| `cost_cap_per_run_usd=0` (M25) AND tenant flag ON | runtime CreateAgentRun | reject `cost_cap_zero` |
| Empty `allowed_providers` AND tenant flag ON | runtime | reject `no_providers_configured` |

### F.8 Migration & rollback

#### F.8.1 ON → OFF (rollback)

1. Opérateur appelle `efe_set_tenant_feature(milestone, enabled=false)`.
2. Cache config invalidé (TTL ≤ 60 s effective).
3. Calls inflight selon `EFE_FEATURE_TOGGLE_DRAIN_INFLIGHT` :
   - `drain` (default) : completer normally, no new calls accepted.
   - `abort` : annuler immédiatement, retourne `feature_disabled`.
4. Audit event émis.
5. Pas de data migration — données existantes (cache, audit, gold
   labels) restent en place mais inutilisées.

#### F.8.2 OFF → ON (activation)

1. Opérateur EFE active server-level (Helm value ou env-var).
2. Si M21 : start `efe-llm-router` service ; vérifier health.
3. Tenant admin appelle `efe_set_tenant_feature` avec config valid.
4. Validation : deps satisfaites, config schema OK, sécurité
   guards passent.
5. Catalog admin active per-catalog si applicable.
6. Test : `efe_classify_mode2(catalog="x", mode=MODE_2,
   sentence="test")` → succès.
7. Adoption progressive : 1 catalog → 1 tenant → fleet.

#### F.8.3 Re-activation après OFF

Données conservées :
- Tenant config jsonb persisté.
- Audit events de toggles précédents préservés.
- Cache LLM expirée (TTL standard).

Re-activer = re-enable flag. Aucun re-onboarding nécessaire.

### F.9 Sunset policy

| Condition | Action |
|---|---|
| Server flag défaut OFF, jamais activé en 12 mois sur la fleet | Marqué deprecated dans CHANGELOG ; sunset si confirmé inutilisé en 24 mois |
| Tenant flag activé < 5 % fleet en 12 mois | Pas de sunset (capability rare est OK) |
| Configuration jsonb schema bump | additive only ; old schemas restent valides via versioning |

### F.10 Test pin reference

`test_default_off_compat.py` (à livrer M20 D55) :

```python
def test_default_off_no_llm_router_service():
    """Sans EFE_LLM_ROUTER_URL, mcp-server doit fonctionner identique
    à M19 GA. Pas de tentative de connexion router."""
    # ...

def test_default_off_no_llm_costs_in_audit():
    """100 classify Mode-1 → audit chain ne contient aucun event
    `llm_call_*`."""
    # ...

def test_default_off_pip_extras_not_installed():
    """`pip show efe-agent` retourne not-installed dans l'image
    Docker mcp-server par défaut."""
    # ...

def test_default_off_threat_model_count():
    """Sans flag activé, le threat model actif = 37 (M19 cumulé)
    pas 49 (M25 cumulé)."""
    # ...

def test_default_off_compliance_attestations_unchanged():
    """Evidence packs SOC 2 / ISO 27001 identiques à M19 GA."""
    # ...

def test_default_off_mode2_returns_unimplemented():
    """Classify(mode=MODE_2) sans flag → UNIMPLEMENTED gRPC."""
    # ...

def test_default_off_no_mcp_server_features_listed():
    """`list_tools()` ne contient aucun tool M20-M25."""
    # ...

def test_default_off_helm_template_no_router_resources():
    """`helm template` sans flags → 0 ressources `efe-llm-router`."""
    # ...

def test_default_off_helm_template_no_agent_runner_crds():
    """`helm template` sans flags → 0 CRDs `agentRunner`."""
    # ...

def test_default_off_test_count_invariant():
    """Invariant : un cluster avec tous flags OFF passe les 522
    tests M19 GA + N test_default_off_* tests, total ≥ 530."""
    # ...
```

---

**FIN**

Document M20-M25 LLM integration roadmap, **opt-in everywhere
edition**. ~2 700 lignes. Pré-livraison ready : prêt à passer en
architecture détaillée M20 (`EFE_M20_BYO_LLM_AGENT_SDK_ARCHITECTURE.md`)
sur signal du sponsor.

**Garantie cardinale** : tant qu'aucun flag M20-M25 n'est activé sur
un cluster, le système est **fonctionnellement et compliance-équivalent
à un cluster EFE 1.3 GA M19**. Cette équivalence est attestée par
le pin test suite `test_default_off_*` (cf. § F.10).

Toute revue chirurgicale : ouvrir issue dans le repo et taguer
`[roadmap-m20-m25]`. Décisions Q-01 à Q-10 (§ 21) doivent être
adressées avant kick-off M21.
