# Sprint M17 — Advanced agent-native tools : architecture

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M16 (MCP Server livré, 6 outils CRUD-style + 46 pin
> tests GREEN, conforme `docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md`).
>
> **Pré-requis** : avoir lu :
> - [`EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md`](EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md)
>   (M16, blueprint MCP server avec 6 outils : `efe_classify`,
>   `efe_register_catalog`, `efe_evolve_catalog`, `efe_list_catalogs`,
>   `efe_get_audit_log`, `efe_get_admin_quotas`)
> - `mcp-server/mcp_server/` (code livré M16, ~1 935 LOC)
> - `mcp-server/tests/` (46 pin tests GREEN)
> - [`EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`](EFE_BUNDLES_EXTENSION_ARCHITECTURE.md)
>   (M13, base bundle Ed25519 + SBOM)
> - [`EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md`](EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md)
>   (M14, threat model 66 STRIDE + 12 nouveaux M16)
> - [`d12_catalog_admin_versioning_architecture.md`](bench/d12_catalog_admin_versioning_architecture.md)
>   (catalog versioning + Evolve RPC + quotas)
> - [`d14bis_hmac_audit_chain_architecture.md`](bench/d14bis_hmac_audit_chain_architecture.md)
>   (HMAC tamper-evident audit log)
> - **Spec MCP officielle Anthropic** : [modelcontextprotocol.io/specification](https://modelcontextprotocol.io/specification)
>   sections « Resources », « Prompts », « Sampling », « Notifications »
>   (3 capabilities MCP que M16 n'utilise pas encore)
>
> **Mission M17** : transformer EFE-via-MCP d'une **API exposée pour
> agents** (M16 = 6 outils CRUD atomiques) en une **API conçue pour
> agents** (agent-native = primitives qui composent des workflows
> multi-étapes, raisonnent, expliquent, et cooptent le LLM hôte via
> sampling lorsque c'est pertinent).
>
> Quatre sous-livraisons :
>
> | Sprint | Phase | Jour | Livrable |
> |---|:---:|:---:|---|
> | **D43** | Φ-A | J185 | **8 nouveaux outils MCP** : 3 reasoning + 3 discovery + 2 diagnostic. Pas de sampling, pas de streaming. Backward-compatible drop-in. |
> | **D44** | Φ-B | J195 | **MCP Resources** (URI grammar `efe://...`) + **MCP Prompts** (5 templates) + 1 outil diagnostic supplémentaire (`efe_audit_health_check`). |
> | **D45** | Φ-C | J205 | **3 outils composites** : `efe_curate_catalog`, `efe_calibrate_thresholds`, `efe_consolidate_versions` — utilisent **sampling MCP** (le serveur demande au LLM hôte) et **streaming progress**. |
> | **D46** | Φ-D | J215 | **2 outils bulk/streaming** : `efe_bulk_classify`, `efe_export_catalog`. Audit pass M17 + freeze des SLOs M17-MCP. |
>
> **Total livraison M17** : **14 nouveaux outils MCP** (de 6 → 20),
> **6 resource types** (`efe://...` URIs), **5 prompt templates**,
> **2 nouvelles capacités MCP** (resources + prompts), **1 capacité
> avancée** (sampling). Volumétrie estimée : **~2 800 LOC code +
> ~1 800 LOC tests + ~80 nouveaux pin tests**, additif sur
> `mcp-server/`.
>
> **Pain points M17 résolus** :
>
> 1. **Atomicité paralysante** : M16 expose 6 verbes atomiques
>    (classify/register/evolve/list/audit/quotas). Pour qu'un agent
>    audite la santé d'un catalogue, il doit aujourd'hui :
>    `list_catalogs` → `classify` × 50 → calculer matrice ambiguïté à la
>    main → re-classify avec différents seuils → comparer. **6 round-
>    trips minimum, ~30 s de latence cumulée** (LLM-loop + EFE-API +
>    LLM-reasoning entre chaque appel). M17 livre `efe_diagnose_catalog`
>    qui fait tout ça côté serveur en **un seul appel ~150 ms**.
> 2. **Décisions opaques** : `efe_classify` retourne top-k scores + flag
>    `above_threshold`. L'agent doit deviner pourquoi `class-A` a gagné
>    (cosine 0.72 vs `class-B` 0.71, écart < 1 % → ambigu mais aucune
>    info de pourquoi). M17 livre `efe_explain_classification` qui
>    explicite : neighbour examples contributoirs, gap-with-runner-up,
>    threshold-sensitivity, dominant tokens (heuristique).
> 3. **Bootstrap froid impossible** : un utilisateur arrive avec 200
>    phrases d'historique support et veut un catalogue. M16 force
>    l'utilisateur à inventer les classes lui-même + assigner chaque
>    phrase à la main. M17 livre `efe_suggest_classes` qui clusterise
>    les embeddings + utilise **sampling MCP** pour demander au LLM hôte
>    de nommer chaque cluster — l'agent propose un catalogue prêt à
>    revoir + register.
> 4. **Calibration manuelle** : confidence_threshold default 0.5 est
>    arbitraire. Une fois déployé en prod, l'opérateur découvre que
>    `class-billing` a 95 % rappel mais 60 % précision (faux positifs
>    massifs). M16 = aucun outil pour calibrer. M17 livre
>    `efe_calibrate_thresholds` qui propose des seuils per-classe
>    ciblant une précision cible.
> 5. **Bulk classify forcé via boucle agent** : aujourd'hui pour
>    classifier 1 000 textes (e.g. backfill historique), l'agent doit
>    appeler `efe_classify` × 1 000. **1 000 round-trips MCP** + le LLM
>    consomme ~1 M tokens à formuler les 1 000 calls. M17 livre
>    `efe_bulk_classify` avec **streaming progress** (un seul appel,
>    progress events tous les 100 ms).
> 6. **Resources MCP non exploitées** : MCP a 3 primitives — `tools`
>    (M16 utilise), `resources` (M16 n'utilise PAS), `prompts` (M16
>    n'utilise PAS). Un client MCP (Claude Desktop) affiche les
>    resources dans une UI dédiée et permet @-mention dans une
>    conversation. Sans M17 resources : l'utilisateur doit dire
>    « classify avec catalogue support-routing » en texte → l'agent
>    retrouve par grep. Avec M17 resources : `@efe://catalogs/support-
>    routing` est un lien direct cliquable + parsable.
> 7. **Sampling MCP non exploité** : MCP permet au serveur de **demander
>    au LLM hôte** de générer du texte (sampling). Use case : « génère-
>    moi 5 noms candidats pour ce cluster », « explique cette
>    classification en français B2 », etc. Sans sampling, le serveur
>    doit soit (a) embarquer son propre LLM (=sub-processor compliance
>    nightmare), soit (b) renvoyer du JSON brut et compter sur l'agent
>    pour reformuler (=double round-trip). Avec sampling : zéro
>    sub-processor (le LLM est celui de l'utilisateur dans son client),
>    réponse en un seul tool call.
>
> **Backward compat** : 100 %. Les 6 outils M16 restent inchangés. Les
> 14 nouveaux sont opt-in via env var `EFE_MCP_TOOL_PROFILE` :
>   - `core` (default) → 6 outils M16 uniquement (zero deviation) ;
>   - `advanced` → 6 + 8 (Phase A) ;
>   - `agent-native` → 6 + 14 (full M17) ;
>   - `experimental` → 6 + 14 + futurs M18.
>
> **Compliance posture EFE 1.0 GA toujours préservée** : sampling
> n'introduit pas de sub-processor — le LLM utilisé est celui que
> l'utilisateur a déjà choisi dans son client MCP, sous son propre
> compte. Aucune donnée EFE ne sort vers une infrastructure
> Anthropic / OpenAI / autre depuis l'infra EFE elle-même.
>
> **Hors scope du freeze 1.0 GA** : `mcp-server/` reste un sous-
> répertoire additif. M17 étend ce répertoire ; aucun crate Rust
> EFE 1.0 GA n'est touché. Les nouveaux endpoints REST nécessaires
> (Phase A §14) sont eux dans le crate `efe-server` et constituent
> donc un **versionnement EFE 1.1 contract** (additif, semver-mineur).

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Vision & rationale — pourquoi « agent-native »](#2-vision--rationale)
3. [Scope & non-goals (delta vs M16)](#3-scope--non-goals)
4. [Catalogue M17 — vue d'ensemble](#4-catalogue-m17--vue-densemble)
5. [Architecture](#5-architecture)
6. [Tool taxonomy — 14 nouveaux outils](#6-tool-taxonomy)
   - 6.1 [Reasoning tools (3)](#61-reasoning-tools)
   - 6.2 [Discovery tools (3)](#62-discovery-tools)
   - 6.3 [Diagnostic tools (3)](#63-diagnostic-tools)
   - 6.4 [Composite workflow tools (3)](#64-composite-workflow-tools)
   - 6.5 [Bulk/streaming tools (2)](#65-bulkstreaming-tools)
7. [MCP Resources (`efe://...`)](#7-mcp-resources)
8. [MCP Prompts (templates)](#8-mcp-prompts)
9. [Sampling integration (server → host LLM)](#9-sampling-integration)
10. [Streaming protocol (progress notifications)](#10-streaming-protocol)
11. [State management & sessions](#11-state-management)
12. [Tool composition patterns](#12-tool-composition-patterns)
13. [JSON Schemas (forward-refs)](#13-json-schemas-forward-refs)
14. [Backend extensions (EFE 1.1 contract)](#14-backend-extensions)
15. [Threat model M17 (delta vs M16)](#15-threat-model-m17)
16. [Performance & SLOs M17](#16-performance--slos-m17)
17. [Compatibility matrix (clients MCP)](#17-compatibility-matrix)
18. [Migration M16 → M17 (opt-in profile)](#18-migration-m16--m17)
19. [Test plan (~80 nouveaux pin tests)](#19-test-plan)
20. [Deployment scenarios (delta vs M16)](#20-deployment-scenarios)
21. [Roadmap M18+](#21-roadmap-m18)
22. [Open questions](#22-open-questions)
23. [FAQ](#23-faq)
24. [Glossary M17](#24-glossary-m17)
25. [References](#25-references)

**Annexes**

- [A — Full JSON Schemas (14 nouveaux outils + 6 resources + 5 prompts)](#a--full-json-schemas)
- [B — Resource URI grammar (formal ABNF)](#b--resource-uri-grammar)
- [C — Prompt templates (full text)](#c--prompt-templates)
- [D — Pin tests inventory (80 tests)](#d--pin-tests-inventory)
- [E — Backend contract diff EFE 1.0 → 1.1](#e--backend-contract-diff)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Étendre la surface MCP EFE de 6 outils CRUD à **20 outils + 6 resources + 5 prompts** orientés agents (raisonnement, découverte, diagnostic, workflows composites, bulk streaming). |
| **Durée** | 4 sprints (D43 → D46), ~40 jours-ingénieur. |
| **Ressources** | 1 senior Python (lead) + 1 mid Python (tools impl) + 1 senior Rust (backend EFE 1.1 endpoints, ~5 jours) + 0.5 sécurité (threat-model review) + 0.25 doc (API ref + tutoriels). |
| **Budget** | ~$220 k (incluant audit pass post-livraison). |
| **Volumétrie code** | ~2 800 LOC Python `mcp-server/` + ~600 LOC Rust `crates/efe-server/` (3 nouveaux endpoints) + ~1 800 LOC tests. |
| **Pin tests** | **+80 GREEN** (vs 46 M16 → 126 cumulés mcp-server). |
| **Backward compat** | 100 %. M16 par défaut ; M17 = opt-in via `EFE_MCP_TOOL_PROFILE=advanced` ou `=agent-native`. |
| **Risk profile** | **Medium** — 2 nouveautés MCP non encore testées en prod (sampling, streaming) ; mitigation via Phase D audit pass + fallback profile. |
| **Compliance impact** | **Zéro nouveau sub-processor**. Sampling utilise le LLM du client MCP de l'utilisateur (pas d'infra EFE-side). Doc compliance M14 inchangée — note additive seulement dans `gdpr_art32.md` §3.5. |
| **EFE 1.1 contract bump** | **Oui** — semver-mineur. 3 nouveaux endpoints REST + 3 nouvelles RPCs gRPC (additifs). EFE 1.0 clients restent compatibles avec EFE 1.1 server, mais EFE 1.1 mcp-server requiert EFE 1.1 server pour `agent-native` profile complet. Profile `advanced` (Phase A) reste compatible avec EFE 1.0 server. |

---

## 2. Vision & rationale

### 2.1 Le glissement « exposed for agents » → « designed for agents »

M16 a livré une **transposition mécanique** : 6 RPCs gRPC EFE 1.0 GA →
6 outils MCP. Schémas JSON copiés des `.proto`. Sémantique préservée.
**Résultat** : un agent peut appeler EFE depuis Claude Desktop, mais
il appelle EFE *comme un humain appellerait* — un verbe atomique à la
fois, en re-formulant chaque réponse en langage naturel pour décider
de l'étape suivante.

C'est correct mais sous-optimal. Un agent LLM diffère d'un humain sur
quatre dimensions critiques :

1. **Latence de raisonnement** : un humain réfléchit ~10 s entre deux
   clics. Un agent réfléchit ~3-5 s entre deux tool calls (génération
   tokens) **mais paie en prix de tokens** ($/1k tokens × tokens
   raisonnement entre chaque round-trip). 10 round-trips ≈ $0.05 (Sonnet)
   à $0.50 (Opus) pour 1 seule décision EFE.
2. **Pas de mémoire visuelle** : un opérateur EFE Web UI voit la
   matrice de confusion d'un coup d'œil. L'agent doit appeler N tool
   calls + reconstruire la matrice en interne.
3. **Hallucination latente** : si l'agent doit calculer manuellement
   « quel seuil pour 90 % précision ? », il peut halluciner. Si le
   serveur le calcule déterministiquement et le retourne, zéro
   hallucination.
4. **Confiance asymétrique** : l'agent fait + naturellement confiance
   à un outil **structuré + déterministe** (`recommended_threshold:
   0.62`) qu'à un raisonnement libre (`based on the histogram, you
   should pick somewhere around 0.6 maybe 0.65...`).

→ **Designed for agents** = primitives qui :
- **Composent** côté serveur ce que l'agent ferait par boucle (réduit
  round-trips × prix tokens) ;
- **Renvoient du raisonnement structuré** (justifications, alternatives,
  contraintes) plutôt que des bool simples ;
- **Coopèrent avec le LLM hôte** via sampling pour les tâches à valeur
  linguistique (nommer un cluster, expliquer en français B2) ;
- **Streament** les longues opérations au lieu de bloquer.

### 2.2 4 patterns d'usage observés (sources : Phase D M16 audit + 5 utilisateurs design partner)

**Pattern A — « Bootstrap »** : utilisateur arrive avec un export CSV
historique (200-2 000 phrases). Veut un catalogue prêt en < 5 min
sans coder. **M16** : impossible (force l'utilisateur à clusteriser
manuellement). **M17** : `efe_suggest_classes` + `efe_propose_examples`.

**Pattern B — « Investigation »** : un classify a renvoyé un résultat
inattendu en prod. L'opérateur veut comprendre pourquoi. **M16** :
re-classify avec top_k=10 + lecture des examples du catalogue + raisonnement
manuel. **M17** : `efe_explain_classification` (justification structurée).

**Pattern C — « Calibration »** : catalogue déployé depuis 1 mois,
operator regarde le dashboard + ressent une dérive (rappel ↓). Veut
recalibrer les seuils. **M16** : impossible automatiquement. **M17** :
`efe_calibrate_thresholds` + `efe_detect_drift`.

**Pattern D — « Bulk »** : intégration Stripe/Shopify, l'utilisateur veut
classifier 50 000 tickets historiques pour backfill. **M16** : 50 000
appels MCP, ~$15 en tokens raisonnement + 25 min de latence cumulée
+ user-experience dégradée. **M17** : `efe_bulk_classify` (1 appel
streamed, ~$0.05 + 30 s).

### 2.3 Pourquoi pas un agent-LLM dédié ?

Question stratégique tranchée pré-M16 : agent-LLM EFE-managed vs
plugin MCP. M16 a choisi MCP. M17 prolonge MCP + ajoute des
capacités agent-native **sans** réintroduire un agent-LLM EFE-managed.

| Critère | Agent EFE-managed (rejeté) | M17 agent-native MCP (retenu) |
|---|---|---|
| Sub-processor add | Anthropic/OpenAI ajouté → DPA × N | Zéro |
| Compliance impact | DPA + AI Act + GDPR § amendments | Aucun |
| Coût LLM | Payé par EFE → pricing model à revoir | Payé par utilisateur (son LLM) |
| Choix LLM | EFE-imposé | User-choice (Claude/GPT/Gemini/local) |
| Latence | 1 hop EFE-LLM + 1 hop EFE-DB | 1 hop EFE-DB seulement |
| Bring-your-own-LLM | Impossible | Trivial |
| Air-gapped support | Difficile (LLM call out) | OK (avec Llama local) |

→ M17 confirme que le sampling MCP **co-opte** le LLM hôte — il ne
ré-introduit pas un LLM côté serveur EFE. C'est la même posture que
M16, étendue.

---

## 3. Scope & non-goals

### 3.1 In scope (livraison M17)

| ID | Catégorie | Item | Phase |
|---|---|---|---|
| 3.1.1 | Tool | `efe_explain_classification` | D43 |
| 3.1.2 | Tool | `efe_compare_classes` | D43 |
| 3.1.3 | Tool | `efe_score_distribution` | D43 |
| 3.1.4 | Tool | `efe_suggest_classes` (sampling) | D45 |
| 3.1.5 | Tool | `efe_find_similar_examples` | D43 |
| 3.1.6 | Tool | `efe_propose_examples` (sampling) | D45 |
| 3.1.7 | Tool | `efe_diagnose_catalog` | D43 |
| 3.1.8 | Tool | `efe_detect_drift` | D43 |
| 3.1.9 | Tool | `efe_audit_health_check` | D44 |
| 3.1.10 | Tool | `efe_curate_catalog` (sampling + streaming) | D45 |
| 3.1.11 | Tool | `efe_calibrate_thresholds` (streaming) | D45 |
| 3.1.12 | Tool | `efe_consolidate_versions` | D45 |
| 3.1.13 | Tool | `efe_bulk_classify` (streaming) | D46 |
| 3.1.14 | Tool | `efe_export_catalog` (streaming) | D46 |
| 3.1.15 | Resources | `efe://catalogs/{name}` | D44 |
| 3.1.16 | Resources | `efe://catalogs/{name}/versions/{v}` | D44 |
| 3.1.17 | Resources | `efe://catalogs/{name}/classes/{class}` | D44 |
| 3.1.18 | Resources | `efe://audit/recent` | D44 |
| 3.1.19 | Resources | `efe://quotas` | D44 |
| 3.1.20 | Resources | `efe://samples/{id}` | D44 |
| 3.1.21 | Prompt | `classify_with_reasoning` | D44 |
| 3.1.22 | Prompt | `bootstrap_catalog_from_csv` | D44 |
| 3.1.23 | Prompt | `audit_compliance_check` | D44 |
| 3.1.24 | Prompt | `drift_investigation` | D44 |
| 3.1.25 | Prompt | `threshold_calibration_session` | D44 |
| 3.1.26 | Capability | Sampling integration | D45 |
| 3.1.27 | Capability | Streaming via `notifications/progress` | D45 |
| 3.1.28 | Backend | EFE 1.1 contract bump (3 REST + 3 gRPC RPCs) | D43-D45 |
| 3.1.29 | Profile | `EFE_MCP_TOOL_PROFILE={core,advanced,agent-native}` | D43 |
| 3.1.30 | Audit | M17 audit pass post-livraison | D46 |

### 3.2 Out of scope (explicitement)

| Item | Pourquoi exclu | Successeur |
|---|---|---|
| Agent-LLM EFE-managed | Sub-processor compliance nightmare ; tranché en analyse stratégique pré-M16 | Jamais ; resté hors-roadmap |
| Auto-déploiement Helm via tool MCP | M12 Helm déjà mature ; surface attaque trop large pour exposer kubectl | M19 (peut-être, en lecture seule) |
| Bundle signing tool | Cryptographique pur ; doit rester `cosign` CLI hors-MCP | M18 explore |
| Realtime streaming classify (WebSocket) | Hors-spec MCP standard ; SSE M19 seulement | M19 |
| Multi-modal (images / audio) | EFE 1.0 GA = text-only ; embedder bundle ne supporte pas | M20+ |
| Apprentissage continu (online learning) | Risque dérive non-contrôlée + compliance AI Act ; statique = + auditable | Jamais (par design) |
| Nettoyage automatique données | Trop intrusif sans confirmation explicite ; reste manuel via UI | Jamais |
| LLM-judge des classifications | Tâche trop subjective ; meilleur en local par l'agent | Jamais (sampling = meilleur) |
| Cross-tenant comparison | Casse D11 zero-trust ; jamais. | Jamais |

### 3.3 Constraints

- **Aucune dépendance Python nouvelle critique** : on ne peut ajouter
  que des deps déjà dans le tronc (numpy déjà requis par playground ;
  scikit-learn = NEW mais cantonné à `mcp_server/clustering/` derrière
  un import paresseux, opt-in via `efe-mcp-server[advanced]` extra).
- **Aucune dépendance Rust nouvelle critique** : les 3 endpoints EFE 1.1
  réutilisent l'infra HTTP/gRPC existante.
- **Profile par défaut = `core`** : un déploiement M17 sans config
  garde le comportement M16 strict (anti-régression).
- **Audit M17 obligatoire avant unfreeze** : pattern récurrent M11-M16
  (~6-8 défauts attendus).

---

## 4. Catalogue M17 — vue d'ensemble

### 4.1 Outils (20 cumulés au final)

```
                         ┌─────────────────────────────────────┐
                         │  EFE MCP Server v0.2.0 (M17 GA)     │
                         │  Profile : core | advanced |        │
                         │            agent-native             │
                         └─────────────────┬───────────────────┘
                                           │
       ┌───────────────────────────────────┴────────────────────────────────────┐
       │                                                                         │
   M16 core (6)                          M17 advanced (8) — D43+D44             │
   ─────────────                         ────────────────────────────           │
   • efe_classify                        • efe_explain_classification           │
   • efe_register_catalog (W)            • efe_compare_classes                  │
   • efe_evolve_catalog   (W)            • efe_score_distribution               │
   • efe_list_catalogs                   • efe_find_similar_examples            │
   • efe_get_audit_log                   • efe_diagnose_catalog                 │
   • efe_get_admin_quotas                • efe_detect_drift                     │
                                         • efe_audit_health_check               │
                                         • efe_export_catalog (read-only export) │
                                                                                │
                                         M17 agent-native (6) — D45+D46         │
                                         ───────────────────────────────        │
                                         • efe_suggest_classes      (S)         │
                                         • efe_propose_examples     (S)         │
                                         • efe_curate_catalog       (W)(S)(P)   │
                                         • efe_calibrate_thresholds (P)         │
                                         • efe_consolidate_versions (W)         │
                                         • efe_bulk_classify        (P)         │

   Légende :
   W = write tool (confirm:true requis, M16 carry-over)
   S = sampling (serveur demande au LLM hôte)
   P = streaming progress (notifications/progress)
```

### 4.2 Resources (6 types, D44)

| URI pattern | Lecture | Description |
|---|:---:|---|
| `efe://catalogs/{name}` | R | Summary catalogue (latest version, fingerprint, class count, audit recent) |
| `efe://catalogs/{name}/versions/{v}` | R | Version spécifique (immutable) |
| `efe://catalogs/{name}/classes/{class}` | R | Classe + examples + centroid hash + threshold |
| `efe://audit/recent?limit=N&since=T` | R | Audit log paginé |
| `efe://quotas` | R | Quotas D12 actuels du tenant |
| `efe://samples/{id}` | R | Sample catalogs (mirror M15 `/samples/{id}`) |

### 4.3 Prompts (5 templates, D44)

| ID | Variables | Usage |
|---|---|---|
| `classify_with_reasoning` | `{catalog}`, `{text}` | Classify + explain en un workflow guidé |
| `bootstrap_catalog_from_csv` | `{csv_path}`, `{target_classes_hint?}` | Workflow first-time : CSV → catalog draft |
| `audit_compliance_check` | `{tenant?}`, `{since?}` | Walkthrough audit trimestriel |
| `drift_investigation` | `{catalog}`, `{class}` | Why did class X suddenly fail? |
| `threshold_calibration_session` | `{catalog}`, `{target_precision?}` | Guided threshold tuning |

---

## 5. Architecture

### 5.1 Module map (delta vs M16)

```
mcp-server/
├── pyproject.toml                  ← UPDATE : new extras [advanced], [agent-native]
├── Dockerfile                      ← UPDATE : numpy added in advanced stage
├── README.md                       ← UPDATE : profile section + new tools
├── mcp_server/
│   ├── __init__.py                 ← UPDATE : version 0.2.0
│   ├── efe_mcp_server.py           ← UPDATE : profile gate, sampling, progress
│   ├── dispatcher.py               ← UPDATE : streaming dispatch, sampling hooks
│   ├── backend_client.py           ← UPDATE : new EFE 1.1 endpoints
│   ├── auth.py                     ← UNCHANGED
│   ├── audit_emitter.py            ← UPDATE : streaming progress events
│   ├── rate_limiter.py             ← UPDATE : per-tool weight (bulk = 10×)
│   ├── profile.py                  ← NEW : profile loader + tool gating
│   ├── sampling.py                 ← NEW : sampling client (server → host LLM)
│   ├── streaming.py                ← NEW : progress emission helper
│   ├── resources/                  ← NEW directory : MCP resources handlers
│   │   ├── __init__.py
│   │   ├── registry.py
│   │   ├── catalogs.py
│   │   ├── audit.py
│   │   ├── quotas.py
│   │   └── samples.py
│   ├── prompts/                    ← NEW directory : prompt templates
│   │   ├── __init__.py
│   │   ├── registry.py
│   │   ├── classify_with_reasoning.md
│   │   ├── bootstrap_catalog.md
│   │   ├── audit_compliance.md
│   │   ├── drift_investigation.md
│   │   └── threshold_calibration.md
│   ├── analysis/                   ← NEW directory : pure-math helpers
│   │   ├── __init__.py
│   │   ├── reasoning.py            ← Explain, compare, score-distribution math
│   │   ├── clustering.py           ← K-means / hierarchical (lazy-loaded scikit)
│   │   ├── drift.py                ← Centroid shift, KL divergence
│   │   ├── thresholds.py           ← PR-curve + threshold optimization
│   │   └── stats.py                ← Histograms, percentiles
│   └── tools/                      ← UPDATE : 14 new tool handlers
│       ├── __init__.py
│       ├── registry.py             ← UPDATE : profile-gated registration
│       ├── types.py                ← UPDATE : ToolContext gains streaming hooks
│       ├── classify.py             ← UNCHANGED (M16)
│       ├── list_catalogs.py        ← UNCHANGED
│       ├── get_audit_log.py        ← UNCHANGED
│       ├── get_admin_quotas.py     ← UNCHANGED
│       ├── register_catalog.py     ← UNCHANGED
│       ├── evolve_catalog.py       ← UNCHANGED
│       ├── explain_classification.py    ← NEW (D43)
│       ├── compare_classes.py           ← NEW (D43)
│       ├── score_distribution.py        ← NEW (D43)
│       ├── find_similar_examples.py     ← NEW (D43)
│       ├── diagnose_catalog.py          ← NEW (D43)
│       ├── detect_drift.py              ← NEW (D43)
│       ├── audit_health_check.py        ← NEW (D44)
│       ├── export_catalog.py            ← NEW (D44)
│       ├── suggest_classes.py           ← NEW (D45, sampling)
│       ├── propose_examples.py          ← NEW (D45, sampling)
│       ├── curate_catalog.py            ← NEW (D45, sampling+streaming)
│       ├── calibrate_thresholds.py      ← NEW (D45, streaming)
│       ├── consolidate_versions.py      ← NEW (D45)
│       └── bulk_classify.py             ← NEW (D46, streaming)
└── tests/
    ├── conftest.py                 ← UPDATE : streaming/sampling fixtures
    ├── test_tool_schemas.py        ← UPDATE : +14 new tool schema tests
    ├── test_tool_dispatch.py       ← UPDATE : +14 happy paths
    ├── test_auth_bridge.py         ← UNCHANGED
    ├── test_error_mapping.py       ← UNCHANGED + 3 new error codes
    ├── test_mcp_protocol.py        ← UPDATE : +resources/list, +prompts/list
    ├── test_e2e_playground.py      ← UPDATE : +EFE 1.1 endpoints
    ├── test_profile_gating.py            ← NEW (D43)
    ├── test_reasoning_tools.py           ← NEW (D43)
    ├── test_discovery_tools.py           ← NEW (D43+D45)
    ├── test_diagnostic_tools.py          ← NEW (D43+D44)
    ├── test_composite_tools.py           ← NEW (D45)
    ├── test_streaming_tools.py           ← NEW (D45+D46)
    ├── test_resources.py                 ← NEW (D44)
    ├── test_prompts.py                   ← NEW (D44)
    ├── test_sampling.py                  ← NEW (D45)
    └── test_audit_pass1.py               ← NEW post-livraison (D46)

crates/efe-server/                  ← UPDATE for EFE 1.1 contract
├── src/
│   ├── catalog/
│   │   ├── analysis.rs             ← NEW : reasoning + drift + diagnose impls
│   │   └── thresholds.rs           ← NEW : PR-curve calibration
│   ├── http/
│   │   └── routes.rs               ← UPDATE : +3 endpoints
│   └── grpc/
│       └── service.rs              ← UPDATE : +3 RPCs
└── tests/
    └── m17_contract.rs             ← NEW (~25 tests EFE 1.1)
```

### 5.2 Process & threading model

M16 utilise asyncio single-event-loop (per-process). M17 conserve ce
modèle :

- **Tool dispatch** : single asyncio task par tool call (stays the same).
- **Streaming progress** : émis depuis le même event loop ; pas de
  thread (`asyncio.create_task` interne pour ne pas bloquer).
- **Sampling round-trip** : stop dans le tool handler, await la
  réponse du client, reprend → naturel en asyncio.
- **Heavy compute** (k-means clustering, PR-curve calibration) :
  encapsulé dans `asyncio.to_thread()` ou `loop.run_in_executor()`
  pour ne pas bloquer l'event loop. Limite par défaut 4 threads via
  `EFE_MCP_COMPUTE_THREADS=4` (override possible).

### 5.3 State management (per-session caches)

M16 = stateless. M17 introduit 2 caches in-memory **per process** (i.e.
per MCP subprocess instance) :

1. **Embedding cache** (`mcp_server/state/embedding_cache.py`) — LRU 1024
   entries, key `sha256(catalog+text)`, TTL 5 min. Réduit re-embed
   coûteux dans les outils composites (`diagnose_catalog` réutilise les
   embeddings de `score_distribution`).
2. **Sampling response cache** (`mcp_server/state/sampling_cache.py`) —
   LRU 64 entries, key `sha256(prompt+model_hint)`, TTL 60 s. Évite
   double-sampling si l'utilisateur ré-appelle un même outil composite
   à courte échéance.

Ces caches sont **per-tenant** (clé inclut `tenant_id` dérivé du JWT) et
**bypassables** via `EFE_MCP_DISABLE_CACHE=true`.

---

## 6. Tool taxonomy

### 6.1 Reasoning tools

#### 6.1.1 `efe_explain_classification`

**Intent** : étant donné une classification (que l'agent ou
l'utilisateur a déjà reçue de `efe_classify`), expliquer pourquoi top-1
a été retenu.

**Input** :
```json
{
  "catalog": "support-routing",
  "text": "I cannot reset my password help",
  "predicted_class": "tech-support"          // optionnel
}
```

**Output** :
```json
{
  "predicted_class": "tech-support",
  "score": 0.87,
  "above_threshold": true,
  "threshold": 0.5,
  "runner_up": {
    "class_name": "billing",
    "score": 0.42,
    "gap": 0.45
  },
  "contributors": [
    {
      "example": "I forgot my password",
      "similarity_to_input": 0.91,
      "weight_in_centroid": 0.18
    },
    {
      "example": "How do I reset 2FA",
      "similarity_to_input": 0.78,
      "weight_in_centroid": 0.16
    }
  ],
  "ambiguity_signal": {
    "is_ambiguous": false,
    "reason": "gap_to_runner_up=0.45 is far above the 0.05 ambiguity threshold"
  },
  "threshold_sensitivity": {
    "decision_unchanged_in_range": [0.3, 0.95],
    "decision_changes_at": null
  }
}
```

**Algorithm** (côté serveur EFE 1.1, fallback playground M17) :
1. Re-embed `text` (ou récupère depuis embedding cache).
2. Identifie `predicted_class` (par arg ou recompute top-1).
3. Pour chaque example de la classe : `similarity_to_input = cos(emb_text, emb_example)` ; rank top-3.
4. Calcule `weight_in_centroid = (1/N)` (uniforme dans EFE 1.0/1.1
   centroid impl) ; si custom-weighted centroids un jour → reflète.
5. Calcule `gap = score_top1 - score_top2`.
6. **Ambiguity heuristic** : si `gap < 0.05` → ambiguous.
7. **Threshold sensitivity** : binary-search la plage [0, 1] où la
   décision reste `predicted_class`.

**Latence cible** : p50 50 ms, p99 200 ms (1 embed + N similarities + 1
threshold sweep).

**Handler skeleton** :
```python
async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    text = args["text"]
    predicted = args.get("predicted_class")
    explanation = await ctx.backend.explain_classification(
        catalog=catalog, text=text, predicted_class=predicted, auth=ctx.auth
    )
    return explanation
```

**Backward compat** : sur EFE 1.0 server (pre-1.1), le mcp-server
fait fallback sur 2 appels `classify` + calcul local — moins
performant mais fonctionnel. Pin test couvre les 2 chemins.

#### 6.1.2 `efe_compare_classes`

**Intent** : comparer deux classes du même catalogue en termes de
séparation sémantique. Sert à détecter de l'ambiguïté pré-prod.

**Input** :
```json
{
  "catalog": "support-routing",
  "class_a": "billing",
  "class_b": "sales"
}
```

**Output** :
```json
{
  "catalog": "support-routing",
  "class_a": "billing",
  "class_b": "sales",
  "centroid_cosine": 0.62,
  "centroid_distance_l2": 0.87,
  "separation_score": 0.38,    // 1 - centroid_cosine, normalized [0,1]
  "ambiguity_band": "moderate", // none|moderate|high|critical
  "boundary_examples": [
    { "example": "Billing inquiry about new plan", "from_class": "billing", "score_a": 0.71, "score_b": 0.69 },
    { "example": "Cancel my subscription", "from_class": "billing", "score_a": 0.58, "score_b": 0.61 }
  ],
  "recommended_action": "review_examples_2_and_above"
}
```

**Algorithm** :
1. Récupère les 2 centroïdes.
2. Calcule cosine + L2.
3. Pour chaque example des 2 classes : score contre les 2 centroïdes ;
   examples avec `|score_a - score_b| < 0.1` → boundary_examples.
4. Bande ambiguïté : `none` si separation > 0.7, `moderate` 0.4-0.7,
   `high` 0.2-0.4, `critical` < 0.2.

**Latence cible** : p50 30 ms, p99 100 ms.

#### 6.1.3 `efe_score_distribution`

**Intent** : générer la distribution de scores d'une classe contre N
textes random (ou fournis), pour aider à choisir un threshold.

**Input** :
```json
{
  "catalog": "support-routing",
  "class_name": "tech-support",
  "sample_count": 100,                    // 10..1000
  "texts": null,                          // ou tableau d'override
  "include_negatives": true               // textes des autres classes comme contre-points
}
```

**Output** :
```json
{
  "catalog": "support-routing",
  "class_name": "tech-support",
  "histogram": {
    "buckets": [0.0, 0.1, 0.2, ..., 1.0],
    "positives": [0,0,1,2,5,12,18,22,18,12,10],
    "negatives": [4,8,15,22,18,12,8,5,3,2,3]
  },
  "percentiles_positives": { "p10": 0.42, "p50": 0.71, "p90": 0.91 },
  "percentiles_negatives": { "p10": 0.05, "p50": 0.32, "p90": 0.62 },
  "recommended_threshold": {
    "value": 0.55,
    "expected_precision": 0.92,
    "expected_recall": 0.88,
    "f1": 0.90
  },
  "current_threshold": 0.5,
  "drift_from_recommended": -0.05
}
```

**Algorithm** :
1. Si `texts` fourni : utilise. Sinon : récupère N examples random du
   catalogue (sample_count // 2 positifs, // 2 négatifs si
   `include_negatives=true`).
2. Score chaque texte contre la classe.
3. Construit histogramme 10-bucket.
4. Optimise threshold pour maximiser F1 (Phase A) ou cible précision
   user-passé (`target_precision` — Phase C composé avec
   `efe_calibrate_thresholds`).

**Latence cible** : p50 200 ms (N=100), p99 1 s (N=1000).

### 6.2 Discovery tools

#### 6.2.1 `efe_suggest_classes` *(sampling)*

**Intent** : étant donné une liste de phrases brutes (e.g. 200 lignes
d'export Stripe), proposer une découpe en classes nommées.

**Input** :
```json
{
  "examples": [
    {"text": "I want to cancel my subscription"},
    {"text": "When is my next payment"},
    {"text": "How do I add a team member"}
    // … 100-2000 entries
  ],
  "target_class_count": 5,        // hint, [2..50]
  "language_hint": "en"           // pour le sampling de noms
}
```

**Output** :
```json
{
  "proposed_classes": [
    {
      "name": "billing",
      "confidence": 0.92,
      "examples": [
        "When is my next payment",
        "Why was I charged twice",
        "..."
      ],
      "centroid_preview": [0.12, -0.04, ...],     // 384-dim L2-normalized
      "size": 23,
      "intra_cluster_cohesion": 0.78
    },
    // … N classes
  ],
  "outliers": [
    {"text": "Random gibberish lol", "reason": "low_cohesion_with_all_clusters"}
  ],
  "naming_method": "sampling",     // ou "heuristic_top_terms" si fallback
  "next_action_hint": "review proposed_classes ; call efe_register_catalog with confirm:true once approved"
}
```

**Algorithm** :
1. Embed all examples (batch) — utilise embedding cache si possible.
2. Clustering : MiniBatchKMeans (sklearn, lazy-loaded, k = `target_class_count`).
3. Pour chaque cluster : top-5 examples nearest to centroid.
4. **Sampling** : pour chaque cluster, demande au LLM hôte de proposer
   un nom court (< 25 chars, kebab-case) basé sur les 5 examples (cf.
   §9). Si sampling indispo (client ne supporte pas) → fallback
   heuristique : top-3 terms TF-IDF.
5. Calcule `intra_cluster_cohesion` = mean cosine intra-cluster.
6. Outliers = examples dont distance min au plus proche centroïde > 1.5σ.

**Latence cible** : p50 3 s (N=200, sampling round-trip ~500 ms × N
clusters // 2 parallèles), p99 10 s (N=2000).

**Profile gate** : `agent-native` only (sampling required). Si
profile=`advanced` mais sampling unavailable → fallback heuristic naming.

#### 6.2.2 `efe_find_similar_examples`

**Intent** : trouver les examples du catalogue les plus similaires à
une entrée. Lookup-style, no LLM required.

**Input** :
```json
{
  "catalog": "support-routing",
  "text": "card declined again",
  "top_k": 5,
  "scope": "all_classes"      // "all_classes" | "single_class:billing" | "top_class"
}
```

**Output** :
```json
{
  "results": [
    {
      "example": "My card was declined",
      "class_name": "billing",
      "similarity": 0.89
    },
    {
      "example": "Payment method failure",
      "class_name": "billing",
      "similarity": 0.81
    }
  ],
  "scope": "all_classes",
  "embedding_norm": 1.0
}
```

**Latence cible** : p50 30 ms (catalog ≤ 256 classes × 64 examples =
16 384 embeddings).

#### 6.2.3 `efe_propose_examples` *(sampling)*

**Intent** : proposer N nouveaux examples pour une classe existante,
avec cible de diversité. Utile pour augmenter une classe sous-fournie.

**Input** :
```json
{
  "catalog": "support-routing",
  "class_name": "tech-support",
  "n_proposed": 5,
  "diversity_target": "high"   // low | medium | high
}
```

**Output** :
```json
{
  "class_name": "tech-support",
  "existing_examples_count": 8,
  "proposed_examples": [
    {
      "text": "My API integration is throwing 500 errors",
      "estimated_centroid_similarity": 0.74,
      "novelty_score": 0.81,
      "rationale": "covers a previously unrepresented sub-topic (API errors)"
    },
    // …
  ],
  "next_action_hint": "review and call efe_evolve_catalog with confirm:true"
}
```

**Algorithm** :
1. Récupère les examples existants + centroïde.
2. **Sampling** : demande au LLM hôte de générer N candidats variants
   basés sur les examples + class_name + diversity_target. Prompt
   template défini en Annexe C.
3. Embed les candidats.
4. `novelty_score = 1 - max(cos(candidate, existing))`.
5. Filtre : drop candidates avec similarity_to_centroid < 0.4 (off-topic).

**Latence cible** : p50 2 s (1 sampling round-trip), p99 6 s.

### 6.3 Diagnostic tools

#### 6.3.1 `efe_diagnose_catalog`

**Intent** : rapport de santé full sur un catalogue. Combine ambiguïté,
gaps de couverture, drift, threshold appropriateness, audit recent.

**Input** :
```json
{
  "catalog": "support-routing",
  "depth": "full"      // "quick" | "standard" | "full"
}
```

**Output** :
```json
{
  "catalog": "support-routing",
  "version": 7,
  "fingerprint": "abc123...",
  "overall_health": "good",      // critical | poor | fair | good | excellent
  "scores": {
    "ambiguity": 0.81,           // 1 = no ambiguity
    "coverage": 0.74,            // 1 = each class has ≥ 5 diverse examples
    "balance": 0.65,             // 1 = even class sizes
    "threshold_quality": 0.88,   // 1 = thresholds match observed F1 optima
    "audit_completeness": 1.0    // 1 = no audit gaps
  },
  "ambiguity_matrix": [
    { "class_a": "billing", "class_b": "sales", "separation": 0.42, "band": "moderate" }
  ],
  "underrepresented_classes": [
    { "class_name": "feedback", "size": 2, "recommended_size": 5 }
  ],
  "threshold_recommendations": [
    { "class_name": "spam", "current": 0.5, "recommended": 0.62, "expected_f1_gain": 0.03 }
  ],
  "drift_signals": [],
  "recommended_actions": [
    "Add ≥3 examples to class 'feedback' (efe_propose_examples + efe_evolve_catalog)",
    "Review boundary between billing/sales (efe_compare_classes)",
    "Adjust spam threshold from 0.5 → 0.62 (efe_evolve_catalog)"
  ]
}
```

**Algorithm** : composé en interne sur le serveur :
1. `compare_classes` × C(N,2) pairs → ambiguity_matrix.
2. Per-class `score_distribution` × N → threshold_quality + recommendations.
3. Per-class size + intra-cohesion → coverage + balance.
4. Last 50 audit events → audit_completeness (gaps détectés ?).
5. Si ≥ 2 versions : `detect_drift` → drift_signals.

**Latence cible** : p50 800 ms (catalogue 5 classes × 10 examples), p99
5 s (256 classes × 64 examples). `depth=quick` : skip ambiguity_matrix.

#### 6.3.2 `efe_detect_drift`

**Intent** : détecter le drift entre 2 versions d'un même catalogue.
Mirror de la cible du tutorial 03 M15.

**Input** :
```json
{
  "catalog": "support-routing",
  "version_a": 5,                  // optionnel, default = previous
  "version_b": 7                   // optionnel, default = latest
}
```

**Output** :
```json
{
  "catalog": "support-routing",
  "version_a": 5,
  "version_b": 7,
  "fingerprint_a": "xyz...",
  "fingerprint_b": "abc...",
  "per_class_drift": [
    {
      "class_name": "billing",
      "centroid_shift_cosine": 0.04,  // 0 = no shift
      "examples_added": 3,
      "examples_removed": 0,
      "size_a": 8,
      "size_b": 11,
      "drift_severity": "low"
    }
  ],
  "new_classes": ["urgent-escalation"],
  "removed_classes": [],
  "overall_drift": 0.07,
  "recommendation": "Drift is low ; rolling forward is safe."
}
```

**Latence cible** : p50 100 ms, p99 500 ms.

#### 6.3.3 `efe_audit_health_check`

**Intent** : vérifier l'intégrité de la chaîne D14-bis HMAC pour le
tenant courant.

**Input** :
```json
{
  "from_timestamp": null,
  "to_timestamp": null,
  "deep": false                  // true = re-vérifie tous les HMAC depuis epoch
}
```

**Output** :
```json
{
  "ok": true,
  "events_verified": 1250,
  "broken_at": null,
  "first_event_at": 1730000000.0,
  "last_event_at": 1735000000.0,
  "last_hmac": "ab12...",
  "verification_method": "incremental"  // incremental | deep
}
```

**Latence cible** : p50 50 ms (incremental, 50 events), p99 5 s (deep, 10 000 events).

### 6.4 Composite workflow tools

#### 6.4.1 `efe_curate_catalog` *(write + sampling + streaming)*

**Intent** : workflow guidé multi-étapes pour curer un catalogue
existant (rebalance classes, remove noise, augment underrepresented).
**Retourne un PLAN** ; n'applique rien sans confirm + register/evolve
explicite.

**Input** :
```json
{
  "catalog": "support-routing",
  "intent": "balance_class_sizes",   // balance_class_sizes | remove_ambiguity | augment_underrepresented
  "max_changes": 20,
  "confirm": true                    // M16 carry-over for write tools — but here the tool is dry-run only (always returns a plan), confirm gates the *cost* of the LLM sampling rounds
}
```

**Output (final, après progress events)** :
```json
{
  "catalog": "support-routing",
  "intent": "balance_class_sizes",
  "current_state": { /* snapshot diagnose */ },
  "plan": {
    "actions": [
      {"type": "add_examples", "class": "feedback", "examples": ["...", "..."], "reason": "underrepresented"},
      {"type": "rename_class", "from": "tech-support", "to": "technical-issue", "reason": "ambiguity reduction"},
      {"type": "remove_examples", "class": "billing", "examples": ["..."], "reason": "drift outlier"}
    ],
    "estimated_impact": {
      "size_balance_score_after": 0.85,
      "ambiguity_score_after": 0.86
    }
  },
  "next_action_hint": "Review the plan ; apply by calling efe_evolve_catalog with the proposed changes (confirm:true required)."
}
```

**Streaming** : émet ~5-10 progress notifications (cf. §10) au fil du
workflow :
- "Phase 1/4 : diagnose…"
- "Phase 2/4 : sampling LLM for class names (3/5)…"
- "Phase 3/4 : computing impact estimate…"
- "Phase 4/4 : assembling plan…"
- final result.

**Latence cible** : p50 8 s, p99 30 s. Hard timeout : 60 s (sinon
streaming progress avec `status=cancelled` + cleanup côté serveur).

**IMPORTANT — confirm-required pattern étendu** : `confirm:true` ici
**ne déclenche pas une écriture** (pas de mutation appliquée), mais
**autorise la dépense de sampling rounds** (coûts LLM côté
utilisateur). Sans confirm → erreur `confirm_required`. Le label
sémantique du confirm est dans la description de l'outil pour qu'un
client MCP affiche un message adéquat (« This action will run up to 5
LLM sampling rounds against your model »).

#### 6.4.2 `efe_calibrate_thresholds` *(streaming)*

**Intent** : calculer per-class thresholds optimaux pour cibler une
précision (ou rappel, ou F1) cible.

**Input** :
```json
{
  "catalog": "support-routing",
  "target_metric": "precision",       // precision | recall | f1
  "target_value": 0.9,
  "validation_set": null,              // optionnel : tableau {text, true_class}
  "min_per_class_samples": 30
}
```

**Output (post-streaming)** :
```json
{
  "catalog": "support-routing",
  "calibration_method": "internal_holdout",   // internal_holdout | provided_validation
  "per_class": [
    {
      "class_name": "billing",
      "current_threshold": 0.5,
      "recommended_threshold": 0.62,
      "expected_precision": 0.91,
      "expected_recall": 0.84,
      "expected_f1": 0.87,
      "samples_used": 64
    }
  ],
  "global_summary": {
    "avg_expected_precision": 0.91,
    "avg_expected_recall": 0.83
  },
  "next_action_hint": "Apply via efe_evolve_catalog : pass updated confidence_threshold per class (confirm:true required)."
}
```

**Algorithm** : pour chaque classe : sweep threshold [0.0..1.0] step
0.01 ; calcule (precision, recall, F1) sur validation set ; pick le
plus petit threshold ≥ target_value. Émet 1 progress event par classe.

**Latence cible** : p50 5 s (catalog 5 classes), p99 30 s (256
classes).

#### 6.4.3 `efe_consolidate_versions` *(write)*

**Intent** : collapse N versions d'un catalogue en gardant les
`keep_last_n` plus récentes (delete les autres). Cleanup pour cas où
quota `MAX_VERSIONS_PER_NAME=1000` est près d'être atteint.

**Input** :
```json
{
  "catalog": "support-routing",
  "keep_last_n": 10,
  "dry_run": true,           // true = retourne plan sans appliquer ; false = applique (requires confirm)
  "confirm": false           // requis si dry_run=false
}
```

**Output (dry_run)** :
```json
{
  "catalog": "support-routing",
  "current_versions_count": 250,
  "keep_last_n": 10,
  "to_delete_count": 240,
  "to_delete_versions": [1, 2, ..., 240],
  "estimated_savings_bytes": 12500000,
  "preserved_versions": [241, ..., 250],
  "dry_run": true,
  "next_action_hint": "Re-call with dry_run=false and confirm=true to apply."
}
```

**Output (applied)** :
```json
{
  "catalog": "support-routing",
  "deleted_count": 240,
  "preserved_versions": [241, ..., 250],
  "applied_at": 1735000123.45,
  "audit_event_id": "evt_..."
}
```

**Backend requirement** : nouveau endpoint EFE 1.1
`POST /catalog/<name>/consolidate?keep_last_n=N&dry_run={true,false}`.
Pre-1.1 backend = `not_supported` error code.

**Latence cible** : p50 200 ms (dry_run), p99 5 s (applied, 240
deletes).

### 6.5 Bulk/streaming tools

#### 6.5.1 `efe_bulk_classify` *(streaming)*

**Intent** : classifier N textes en un seul appel, avec progress
streaming.

**Input** :
```json
{
  "catalog": "support-routing",
  "texts": ["..", "..", ..],         // 1..1000
  "top_k": 1,
  "include_explanations": false      // true = inclut explanation par item, plus lent
}
```

**Output (final)** :
```json
{
  "catalog": "support-routing",
  "version": 7,
  "results": [
    {"index": 0, "text_preview": "I cannot...", "predicted_class": "tech-support", "score": 0.87},
    {"index": 1, "text_preview": "When is...", "predicted_class": "billing", "score": 0.79},
    // …
  ],
  "summary": {
    "total": 200,
    "by_class": {"tech-support": 78, "billing": 62, "sales": 41, "...": 19},
    "below_threshold_count": 12,
    "ambiguous_count": 8
  },
  "duration_ms": 4500
}
```

**Streaming** : 1 progress event par batch de 50 items
(`{processed: 50, total: 200}`).

**Latence cible** : p50 5 s pour N=200, p99 30 s pour N=1000.
Throughput cible : ≥ 200 classify/s (mesuré single-replica EFE 1.1 avec
embedder hash backend).

**Cap** : N ≤ 1000 par appel (TM-M17-D-01 mitigation). Plus → multiple
calls.

#### 6.5.2 `efe_export_catalog`

**Intent** : sérialiser un catalogue (full ou single-version) en JSON
ou YAML pour backup, diff, ou audit.

**Input** :
```json
{
  "catalog": "support-routing",
  "version": null,         // null = latest, sinon int spécifique
  "format": "json",        // json | yaml
  "include_centroids": false,
  "include_audit_excerpt": true
}
```

**Output** :
```json
{
  "catalog": "support-routing",
  "version": 7,
  "format": "json",
  "size_bytes": 18540,
  "content": {
    "name": "support-routing",
    "version": 7,
    "fingerprint": "abc...",
    "classes": [
      {"name": "billing", "examples": [...], "confidence_threshold": 0.62}
    ],
    "embedder_info": { "backend": "hash", "model_name": "fake", "dim": 384 }
  },
  "exported_at": 1735000123.45
}
```

**Note** : si `size_bytes > 1 MB` → réponse streamée en chunks de 256 KB
via progress events ; le contenu final est reconstruit côté client MCP
(le SDK MCP supporte ça via `notifications/progress` avec `partialResult`).

---

## 7. MCP Resources

### 7.1 Pourquoi des resources

Les MCP resources sont des **lectures URI-adressables** que le client
peut afficher dans une UI dédiée et @-mentionner dans une conversation.
Différences clés vs tools :

| Aspect | Tool | Resource |
|---|---|---|
| Sémantique | RPC (verb) | URI (noun) |
| Side-effects | Possibles (write tools) | Aucun (read-only) |
| Schéma input | JSON Schema d'args | Pas d'args (URI = la requête) |
| UI client | Bouton tool call | Lien cliquable + @-mention |
| Caching | Selon serveur | HTTP-style ETag / Last-Modified |

→ M17 expose des « lectures fréquentes » comme resources pour exploiter
l'UX MCP-native.

### 7.2 URI grammar

```
efe-uri    = "efe://" path [ "?" query ]
path       = catalogs / audit / quotas / samples
catalogs   = "catalogs/" name [ "/versions/" version
                              / "/classes/" class ]
audit      = "audit/recent"
quotas     = "quotas"
samples    = "samples/" sample-id
name       = 1*100( ALPHA / DIGIT / "-" / "_" )
version    = 1*5DIGIT          ; 1..99999
class      = 1*100( ALPHA / DIGIT / "-" / "_" / "." )
sample-id  = 1*50( ALPHA / DIGIT / "-" / "_" )
query      = key=value *( "&" key=value )
key        = "limit" / "since" / "format"
value      = 1*VCHAR
```

Une grammaire ABNF formelle complète est en Annexe B.

**Exemples valides** :
- `efe://catalogs/support-routing`
- `efe://catalogs/support-routing/versions/7`
- `efe://catalogs/support-routing/classes/billing`
- `efe://audit/recent?limit=50`
- `efe://audit/recent?since=2026-04-01T00:00:00Z`
- `efe://quotas`
- `efe://samples/support_routing`

**Exemples invalides** (rejetés par le parser ; TM-M17-E-01 mitigation) :
- `efe://catalogs/../../../etc/passwd` (traversal)
- `efe://catalogs/<script>` (chars non autorisés)
- `efe://catalogs/very-long-name-that-exceeds-100-chars-…` (max length)
- `efe://other-tenant/catalogs/foo` (path inconnu, pas de cross-tenant)

### 7.3 Implementation skeleton

```python
# mcp_server/resources/registry.py
from dataclasses import dataclass
from typing import Awaitable, Callable

@dataclass
class Resource:
    uri_pattern: str            # e.g. "efe://catalogs/{name}"
    name: str
    description: str
    mime_type: str = "application/json"
    handler: Callable[[dict, ToolContext], Awaitable[bytes]] | None = None

REGISTRY: list[Resource] = [
    Resource(
        uri_pattern="efe://catalogs/{name}",
        name="EFE catalog summary",
        description="Catalog summary : latest version, fingerprint, class count.",
        handler=catalog_summary_handler,
    ),
    # …
]
```

### 7.4 Pagination & caching

- **Pagination** (audit/recent) : `?limit=N` (default 50, max 200) +
  `?cursor=...` (opaque token from previous response).
- **Caching** : chaque resource retourne `etag` (sha256 of canonical
  serialization) + `last_modified` (unix timestamp). Le SDK MCP relaie
  au client. Pas de cache HTTP-style côté serveur car contenu peut
  changer ; c'est le client qui décide de re-fetch ou pas.

### 7.5 Protocol mapping

```
tools/list  → liste des 20 outils
resources/list  → liste des 6 resource types (avec patterns)
resources/read  { "uri": "efe://catalogs/support-routing" }
                → { "contents": [{"uri": ..., "mimeType": "application/json", "text": "..."}] }
prompts/list  → liste des 5 prompts
prompts/get  { "name": "classify_with_reasoning", "arguments": { "catalog": ..., "text": ... } }
              → { "messages": [{"role": "user", "content": {"type": "text", "text": "..."}}] }
```

---

## 8. MCP Prompts

### 8.1 Why prompts

Les MCP prompts sont des **templates pré-construits** que le client
expose dans une UI dédiée. L'utilisateur sélectionne un prompt ; le
client substitue les variables ; le LLM exécute.

→ M17 livre 5 prompts qui couvrent les workflows les plus fréquents.

### 8.2 Liste des prompts

| ID | Variables | Phase |
|---|---|---|
| `classify_with_reasoning` | `{catalog}`, `{text}` | D44 |
| `bootstrap_catalog_from_csv` | `{csv_path}`, `{target_classes_hint?}` | D44 |
| `audit_compliance_check` | `{tenant?}`, `{since?}` | D44 |
| `drift_investigation` | `{catalog}`, `{class}` | D44 |
| `threshold_calibration_session` | `{catalog}`, `{target_precision?}` | D44 |

Templates complets en Annexe C.

### 8.3 Variable substitution

Substitution Jinja2-light (subset whitelisted). Pas de Turing-completeness :
- `{var}` ou `{var?}` (optional) — substitution simple.
- `{#if var}…{/if}` — conditional block.
- Pas de boucles, pas de filtres, pas d'eval. Anti TM-M17-E-02.

### 8.4 Example : `classify_with_reasoning`

```markdown
---
name: classify_with_reasoning
description: Classify a text against an EFE catalog and explain the result.
arguments:
  - name: catalog
    description: EFE catalog name
    required: true
  - name: text
    description: Text to classify
    required: true
---

I want to classify the following text against the EFE catalog
**{catalog}** and understand why it lands where it does.

Text:
```
{text}
```

Please :
1. Call `efe_classify` with `catalog={catalog}` and `text={text}`.
2. Then call `efe_explain_classification` with the same args plus
   `predicted_class=<top1 from step 1>`.
3. Summarize the result in 3 bullets : (a) the predicted class,
   (b) the gap to runner-up, (c) any ambiguity signals or contributors
   worth flagging.
```

---

## 9. Sampling integration

### 9.1 What MCP sampling does

Le serveur MCP peut, au cours d'un tool call, **demander au LLM hôte**
de générer du texte. Le client relaie au LLM choisi par l'utilisateur,
applique ses propres garde-fous (rate limit, budget tokens, refus si
le LLM hôte n'a pas la capability), et renvoie la réponse.

Sequence :
```
User                    Client MCP                Server MCP            Backend EFE
  │   call efe_suggest_classes
  ├──────────────────────►│
  │                       │  tools/call            │
  │                       ├───────────────────────►│
  │                       │                        │ embed batch + cluster
  │                       │                        │   (deterministic)
  │                       │                        │
  │                       │   sampling/createMessage  (per cluster)
  │                       │◄───────────────────────┤
  │   (UI shows user that the server is asking the LLM)
  │                       │  user-host-LLM call
  │                       │  (e.g. Claude on Anthropic infra,
  │                       │   under user's account, user's token)
  │                       │
  │                       │   sampling/createMessage response
  │                       ├───────────────────────►│
  │                       │                        │ assemble final result
  │                       │  tools/call result     │
  │                       │◄───────────────────────┤
  │   final answer        │
  │◄──────────────────────┤
```

### 9.2 Compliance posture

- Le LLM utilisé est **celui de l'utilisateur**, sous son propre
  compte/token ; **pas un LLM hébergé par EFE**.
- EFE n'a **aucune visibilité** sur le contenu du sampling round-trip
  côté LLM hôte ; le serveur EFE envoie un prompt + reçoit une réponse,
  point.
- → **Zéro nouveau sub-processor**. Compliance posture EFE 1.0 GA
  (M14) inchangée. Note additive seulement dans `gdpr_art32.md` §3.5
  (« Sampling-based tools : the EFE MCP server may, at the user's
  client's discretion, request text generation from the user's chosen
  LLM via the MCP sampling protocol. EFE does not control which LLM
  provider serves these requests ; this is the same LLM the user
  has already chosen for their MCP client. »).

### 9.3 Server-side guards

- **Confirm-required** : tout tool qui utilise sampling doit
  recevoir `confirm: true` (M16 carry-over étendu).
- **Max sampling rounds par tool call** : 5 (TM-M17-D-02 mitigation,
  hard cap, abort + error si dépassé).
- **Timeout par sampling round** : 10 s (annulation si dépassé).
- **Max tokens demandés par round** : 500 (le serveur ne demande pas de
  long-form ; juste names + brief explanations).
- **Cache sampling** : mêmes prompt+model_hint dans la même fenêtre
  60 s → réutilise (`mcp_server/state/sampling_cache.py`).
- **Refus client** : si le client MCP ne supporte pas sampling →
  fallback heuristique ; signalé dans la réponse via `naming_method:
  "heuristic_top_terms"` (cf. §6.2.1).

### 9.4 Sampling client skeleton

```python
# mcp_server/sampling.py
import asyncio
from typing import Any

class SamplingClient:
    def __init__(self, mcp_session, *, max_rounds_per_call=5, timeout_seconds=10.0):
        self._session = mcp_session
        self.max_rounds = max_rounds_per_call
        self.timeout = timeout_seconds
        self._rounds_used = 0

    def reset_for_new_call(self):
        self._rounds_used = 0

    async def sample(self, *, prompt: str, max_tokens: int = 500, model_hint: str | None = None) -> str:
        if self._rounds_used >= self.max_rounds:
            raise SamplingBudgetExceeded(
                f"max sampling rounds ({self.max_rounds}) exceeded for this tool call"
            )
        self._rounds_used += 1
        try:
            resp = await asyncio.wait_for(
                self._session.create_message(
                    messages=[{"role": "user", "content": {"type": "text", "text": prompt}}],
                    max_tokens=max_tokens,
                    model_preferences=[{"hints": [{"name": model_hint}]}] if model_hint else None,
                ),
                timeout=self.timeout,
            )
        except asyncio.TimeoutError:
            raise SamplingTimeout(f"sampling round timed out after {self.timeout}s")
        return resp.content.text
```

---

## 10. Streaming protocol

### 10.1 Why streaming

Les outils composites (`efe_curate_catalog`, `efe_calibrate_thresholds`,
`efe_bulk_classify`, `efe_export_catalog` > 1 MB) peuvent prendre 5-30 s.
Sans streaming :
- L'utilisateur voit un spinner sans info.
- Le client MCP peut timeout.
- Un cancel utilisateur tue le tool call mais aucun cleanup côté serveur
  (qui continue à brûler des cycles).

Avec streaming via `notifications/progress` :
- L'utilisateur voit "Phase 2/4 : sampling… (3/5 clusters done)" en
  live.
- Le client MCP peut envoyer `notifications/cancelled` → le serveur
  stoppe gracefully + cleanup.

### 10.2 MCP `notifications/progress` shape

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/progress",
  "params": {
    "progressToken": "tool-call-id-abc123",
    "progress": 30,
    "total": 100,
    "message": "Phase 2/4 : sampling LLM for class names (3/5 clusters)"
  }
}
```

### 10.3 Server-side helper

```python
# mcp_server/streaming.py
import asyncio
from typing import Any

class ProgressEmitter:
    """Helper attached to ToolContext when streaming is requested."""

    def __init__(self, mcp_session, progress_token: str | None):
        self._session = mcp_session
        self._token = progress_token
        self._cancelled = asyncio.Event()
        # Hook : if the client sends notifications/cancelled with same token,
        # the dispatcher sets self._cancelled.

    @property
    def streaming_enabled(self) -> bool:
        return self._token is not None

    async def emit(self, *, progress: int, total: int = 100, message: str = "") -> None:
        if not self.streaming_enabled:
            return
        await self._session.send_progress_notification(
            self._token, progress=progress, total=total, message=message
        )

    @property
    def cancelled(self) -> bool:
        return self._cancelled.is_set()

    def check_cancelled(self):
        if self.cancelled:
            raise OperationCancelled("client cancelled tool call")
```

### 10.4 Tool handler usage pattern

```python
# mcp_server/tools/calibrate_thresholds.py
async def handler(args, ctx):
    classes = await fetch_classes(...)
    result = []
    for i, cls in enumerate(classes):
        ctx.progress.check_cancelled()  # pre-emption point
        threshold = compute_threshold(cls, ...)
        result.append(threshold)
        await ctx.progress.emit(
            progress=i+1, total=len(classes),
            message=f"Calibrating class {cls.name} ({i+1}/{len(classes)})",
        )
    return {"per_class": result, ...}
```

### 10.5 Cancellation semantics

Si le client envoie `notifications/cancelled` avec le même `progressToken` :
- Le tool handler reçoit `OperationCancelled` à son prochain
  `check_cancelled()`.
- Aucun side-effect partiel n'est committé (les write tools n'apply
  qu'à la fin).
- L'audit event est émis avec `ok: false, error_code: "cancelled"`.
- Le dispatcher retourne un `isError: true` avec code `cancelled`.

---

## 11. State management & sessions

### 11.1 Per-tool-call state

- `progress_token` : opaque, généré par client.
- `sampling_rounds_used` : compteur initialisé à 0 par tool call.
- `embedding_cache_hits` : telemetry (instrumented via OTel attrs).

### 11.2 Per-session state

- `embedding_cache` (LRU 1024, TTL 5 min, per-tenant).
- `sampling_cache` (LRU 64, TTL 60 s, per-tenant).
- `tool_call_count` (per-(token, tool), reset on session disconnect).

### 11.3 Idempotency keys (M11 mirror)

Les write tools (`efe_register_catalog`, `efe_evolve_catalog`,
`efe_consolidate_versions`) supportent un argument optionnel
`idempotency_key` (string ≤ 64 chars). Le mcp-server forward au backend
EFE 1.0/1.1 qui utilise déjà M11 idempotency cache. Si même
`idempotency_key` + même body dans la fenêtre 5 min → backend retourne
le résultat caché ; mcp-server le transmet inchangé avec un flag
`replay: true`.

---

## 12. Tool composition patterns

### 12.1 Sequential pipeline

```
efe_diagnose_catalog
   → efe_calibrate_thresholds
   → efe_evolve_catalog (apply thresholds)
   → efe_diagnose_catalog (verify improvement)
```

L'agent peut chaîner ; chaque étape est idempotente sauf l'evolve.

### 12.2 Parallel ensemble (advisory)

L'agent peut appeler `efe_classify` et `efe_find_similar_examples` en
parallèle pour obtenir 2 angles complémentaires ; pas de garantie côté
serveur, c'est de l'orchestration agent.

### 12.3 Conditional branching

Le prompt `bootstrap_catalog_from_csv` documente :
```
If suggest_classes returns confidence > 0.85 for all clusters → ask user to confirm + register.
Else → propose to refine via propose_examples and re-iterate.
```

### 12.4 Plan-execute-verify

Pattern standard :
1. **Plan** : `efe_curate_catalog` (dry-run, retourne plan).
2. **Execute** : agent affiche plan à l'utilisateur ; sur approbation
   appelle `efe_evolve_catalog`.
3. **Verify** : `efe_diagnose_catalog` post-fact pour confirmer
   l'amélioration.

---

## 13. JSON Schemas (forward-refs)

Tous les schémas complets sont en **Annexe A**. Conventions M17 :
- `additionalProperties: false` sur tous les objets (anti TM-M16-T-02 carry-over).
- Tout outil qui appelle sampling **DOIT** déclarer
  `confirm: { type: boolean }` dans `required` (cf. §9.3).
- Tout outil qui streame **DOIT** documenter le format des progress
  events dans la description.
- `pattern` pour les noms (`^[a-zA-Z0-9][a-zA-Z0-9_\-]{0,99}$`)
  identique M16.
- `maxLength: 8192` sur tous les inputs `text` (anti TM-M17-D-01 partiel).
- Quand un outil prend un tableau de textes (`bulk_classify`) :
  `maxItems: 1000` strict + `items.maxLength: 8192`.

---

## 14. Backend extensions

### 14.1 EFE 1.1 contract bump (semver-mineur)

3 nouveaux endpoints REST + 3 RPCs gRPC additifs. Les EFE 1.0 clients
restent compatibles avec EFE 1.1 server (le bump est additif). Les
mcp-server M17 profile=`agent-native` requièrent EFE 1.1 server pour
la totalité des fonctionnalités.

### 14.2 Nouveaux endpoints REST (additifs)

| Method | Path | Phase | Description |
|---|---|---|---|
| `POST` | `/catalog/<name>/explain` | D43 | `efe_explain_classification` backend impl |
| `POST` | `/catalog/<name>/compare` | D43 | `efe_compare_classes` backend impl |
| `POST` | `/catalog/<name>/score-distribution` | D43 | `efe_score_distribution` backend impl |
| `POST` | `/catalog/<name>/find-similar` | D43 | `efe_find_similar_examples` backend impl |
| `POST` | `/catalog/<name>/diagnose` | D43 | `efe_diagnose_catalog` backend impl |
| `POST` | `/catalog/<name>/drift` | D43 | `efe_detect_drift` backend impl |
| `POST` | `/catalog/<name>/calibrate` | D45 | `efe_calibrate_thresholds` backend impl |
| `POST` | `/catalog/<name>/consolidate` | D45 | `efe_consolidate_versions` backend impl |
| `POST` | `/catalog/<name>/bulk-classify` | D46 | `efe_bulk_classify` backend impl |
| `GET` | `/catalog/<name>/export` | D44 | `efe_export_catalog` backend impl |

Diff complet vs EFE 1.0 = en Annexe E.

### 14.3 Nouveaux RPCs gRPC

3 nouvelles RPCs (additives au `EfeService` existant) :
- `Explain(ExplainRequest) returns (ExplainResponse);`
- `Diagnose(DiagnoseRequest) returns (DiagnoseResponse);`
- `BulkClassify(BulkClassifyRequest) returns (stream BulkClassifyChunk);`

Les autres opérations M17 sont composées côté mcp-server à partir des
RPCs déjà exposées (Encode + Classify + ListCatalogs + …) — pas besoin
de nouvelle RPC dédiée.

### 14.4 Backward compat sur EFE 1.0 server

Le mcp-server détecte la version backend via `GET /info` (présence du
flag `contract_version: "1.1"`). Si EFE 1.0 :
- `efe_explain_classification` → fallback : 2 calls `classify` + calcul local.
- `efe_diagnose_catalog` → fallback : compose plusieurs RPCs M16.
- `efe_calibrate_thresholds` → fallback : multiple `score_distribution`
  via fallback récursif (lent mais fonctionne).
- `efe_consolidate_versions` → **error** `not_supported` (pas de
  fallback : la deletion de versions n'existe pas côté EFE 1.0).
- `efe_bulk_classify` → fallback : N appels `classify` séquentiels (perf
  ~5×worse, mais fonctionne).
- `efe_export_catalog` → fonctionne pleinement (compose `list` +
  `get_version` existants).

---

## 15. Threat model M17

Delta sur les 12 menaces M16. **+8 nouvelles menaces M17** = 20 cumulées
mcp-server, 86 cumulées EFE complet.

| ID | STRIDE | Threat | Mitigation livrée M17 |
|---|:---:|---|---|
| **TM-M17-S-01** | Spoof | Sampling response forge des claims d'autorité (« I am the user, approve this write ») | Réponses sampling **jamais** utilisées comme entrée d'authz côté serveur ; sampling = data only. Confirm-required reste obligatoire pour write. |
| **TM-M17-T-01** | Tamper | Composite tool modifie l'état mid-workflow sans dry-run | Composite tools = dry-run par défaut ; mutation seulement via outils write atomiques (register/evolve) qui ont déjà confirm-required. |
| **TM-M17-R-01** | Repudiation | Streamed op laisse un état ambigu (cancellation à 80 %) | Per-progress checkpoint dans audit chain D14-bis + final event `mcp_call.cancelled` avec `progress_at_cancel`. |
| **TM-M17-I-01** | InfoDisclosure | `diagnose_catalog` retourne tout le catalogue (PII inside examples) | Field `include_examples_text: false` par défaut sur diagnose ; PII scrub D13 via `audit_emitter.scrub_text`. |
| **TM-M17-D-01** | DoS | Bulk classify N=1M floods backend | `maxItems: 1000` strict + rate-limiter weight 10× pour bulk_classify (1 bulk = 10 unités) → 10 bulk/min × 1000 = 10 000 classify/min hard cap. |
| **TM-M17-D-02** | DoS | Sampling-loop boucle infiniment | Hard cap 5 sampling rounds par tool call ; abort si dépassé (`SamplingBudgetExceeded`). |
| **TM-M17-E-01** | Elevation | Resource URI traversal `efe://catalogs/../../etc/passwd` | Strict ABNF parser ; tout chemin non listé en §7.2 → rejet 400. |
| **TM-M17-E-02** | Elevation | Prompt injection via class name dans template | Substitution variables = whitelist + escape (no eval, no jinja2 unsafe). |

**Threat carry-over depuis M16** : les 12 TM-M16-* restent valides et
sont systématiquement ré-vérifiés par les pin tests ré-runnés en M17
audit pass.

**Pen-test couverture** : threat model M17 est ajouté au scope du
pen-test annuel (référence `docs/security/threat_model.md`, à étendre
en M17 audit pass).

---

## 16. Performance & SLOs M17

### 16.1 SLOs M17-MCP

| ID | SLO | Cible | Mesuré comment |
|---|---|---|---|
| M17-MCP-1 | p50 reasoning tools | < 50 ms (excl backend) | OTel histogram `efe_mcp.tool.duration_ms{tool=*reasoning*}` |
| M17-MCP-2 | p99 reasoning tools | < 200 ms | OTel idem |
| M17-MCP-3 | p50 diagnostic tools (`diagnose_catalog`) | < 800 ms (catalog ≤ 50 classes) | OTel idem |
| M17-MCP-4 | p99 diagnostic tools | < 5 s | OTel idem |
| M17-MCP-5 | bulk_classify throughput | ≥ 200 req/s par instance | bench M17 (`bench/m17_bulk.py`) |
| M17-MCP-6 | streaming progress event freq | ≥ 1 event / 100 ms pour les ops > 1 s | bench M17 |
| M17-MCP-7 | resource read p50 | < 30 ms | OTel histogram `efe_mcp.resource.duration_ms` |
| M17-MCP-8 | sampling round-trip overhead (excl LLM) | < 50 ms | OTel histogram `efe_mcp.sampling.duration_ms` (sans LLM time) |
| M17-MCP-9 | composite tool budget total | ≤ 5 sampling rounds, ≤ 30 s | enforced + audited |
| M17-MCP-10 | tool error rate (excl rate_limit) | < 0.5 % | OTel ratio `errors / total` |
| M17-MCP-11 | profile=`core` regression | identique à M16 (46 pin tests still GREEN) | CI |
| M17-MCP-12 | M17 pin tests | 80 nouveaux GREEN | CI (`mcp-server/tests/`) |

### 16.2 Runbooks M17 (dans `docs/runbooks/`)

- `m17_high_sampling_failure_rate.md` — sampling timeout repeat → check client capability + LLM provider status.
- `m17_streaming_starvation.md` — progress events absents → check session, `EFE_MCP_DISABLE_STREAMING` env.
- `m17_resource_404_pattern.md` — multiple resources/read 404 → check tenant + resource URI grammar.
- `m17_composite_tool_timeout.md` — curate_catalog hits 60 s timeout → check sampling budget + classes count.

---

## 17. Compatibility matrix

État capabilities MCP supportées par chaque client connu (avril 2026,
à mettre à jour à chaque nouveau release client) :

| Client | Tools | Resources | Prompts | Sampling | Streaming progress |
|---|:---:|:---:|:---:|:---:|:---:|
| Claude Desktop (latest) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Cursor (latest) | ✅ | ✅ | ⚠ partial | ⚠ early | ✅ |
| Zed (latest) | ✅ | ✅ | ⚠ | ❌ | ⚠ partial |
| Continue.dev | ✅ | ⚠ | ❌ | ❌ | ⚠ partial |
| Cline | ✅ | ⚠ | ❌ | ❌ | ❌ |
| OpenAI bridge (community) | ✅ | ⚠ | ❌ | ❌ | ❌ |

→ Comportement mcp-server quand une capability manque :
- **Tools** : tous les outils marchent partout (capability minimum
  garantie par MCP).
- **Resources** : si client ne supporte pas → resources non listées
  côté client. Pas d'erreur. L'utilisateur peut toujours appeler les
  tools équivalents (`efe_list_catalogs` etc).
- **Prompts** : idem, fallback graceful.
- **Sampling** : tools qui en dépendent retournent `naming_method:
  "heuristic_top_terms"` (ou équivalent). Pas d'erreur.
- **Streaming** : tools longs ne streament pas mais retournent le
  résultat final agrégé. Pas d'erreur.

**Detection** : le mcp-server lit
`session.client_capabilities` reçu dans le handshake `initialize` MCP.

---

## 18. Migration M16 → M17

### 18.1 Profile gating

Variable env `EFE_MCP_TOOL_PROFILE` (default `core`) :
- `core` (M16) — 6 outils, pas de resources, pas de prompts, pas de sampling, pas de streaming.
- `advanced` (M17 Phase A+B) — 6 + 8 outils (3 reasoning + 3
  discovery sans sampling + 2 diagnostic) + resources + prompts.
- `agent-native` (M17 Phase C+D) — 6 + 14 outils complet + sampling + streaming.
- `experimental` — réservé M18+.

Implémentation :

```python
# mcp_server/profile.py
from enum import Enum
from typing import Literal

class Profile(str, Enum):
    CORE         = "core"
    ADVANCED     = "advanced"
    AGENT_NATIVE = "agent-native"
    EXPERIMENTAL = "experimental"

PROFILE_TOOL_GATE: dict[Profile, set[str]] = {
    Profile.CORE: {
        "efe_classify", "efe_register_catalog", "efe_evolve_catalog",
        "efe_list_catalogs", "efe_get_audit_log", "efe_get_admin_quotas",
    },
    Profile.ADVANCED: {  # CORE + 8
        # …(union)…
        "efe_explain_classification", "efe_compare_classes",
        "efe_score_distribution", "efe_find_similar_examples",
        "efe_diagnose_catalog", "efe_detect_drift",
        "efe_audit_health_check", "efe_export_catalog",
    },
    Profile.AGENT_NATIVE: {  # ADVANCED + 6
        # …
        "efe_suggest_classes", "efe_propose_examples",
        "efe_curate_catalog", "efe_calibrate_thresholds",
        "efe_consolidate_versions", "efe_bulk_classify",
    },
    Profile.EXPERIMENTAL: ...,
}

def load_profile() -> Profile:
    raw = os.environ.get("EFE_MCP_TOOL_PROFILE", "core").strip().lower()
    try:
        return Profile(raw)
    except ValueError:
        raise RuntimeError(f"unknown EFE_MCP_TOOL_PROFILE={raw!r}")
```

### 18.2 Backward-compat tests

Pin test rouge si profile=`core` introduit n'importe quel changement de
comportement vs M16 :
```python
def test_core_profile_exposes_exactly_six_tools(mcp_server_core):
    tools = await mcp_server_core.list_tools()
    assert {t["name"] for t in tools} == M16_CORE_TOOLS
    # ré-run les 46 tests M16 → 46 GREEN attendus
```

### 18.3 Rollback plan

Si M17 introduit une régression en prod, simple :
```bash
EFE_MCP_TOOL_PROFILE=core efe-mcp-server  # reverts to M16 behavior
```

Ou re-deploy l'image M16 0.1.x — toujours backward compatible avec
EFE 1.0/1.1 server.

---

## 19. Test plan

### 19.1 Pin tests inventory (~80 nouveaux tests)

Inventaire complet en Annexe D. Résumé :

| Fichier | Tests | Phase | Couverture |
|---|---|---|---|
| `test_profile_gating.py` | 6 | D43 | Profile env-var → tool registry filtré. |
| `test_reasoning_tools.py` | 12 | D43 | explain, compare, score-distribution + edges. |
| `test_discovery_tools.py` | 14 | D43+D45 | suggest, find-similar, propose + sampling fallback. |
| `test_diagnostic_tools.py` | 12 | D43+D44 | diagnose, drift, audit-health-check + depth modes. |
| `test_composite_tools.py` | 12 | D45 | curate, calibrate, consolidate + dry-run. |
| `test_streaming_tools.py` | 8 | D45+D46 | bulk_classify, export, progress event order, cancellation. |
| `test_resources.py` | 8 | D44 | URI parsing, 6 resource types, traversal rejection. |
| `test_prompts.py` | 5 | D44 | Variable substitution, all 5 templates render. |
| `test_sampling.py` | 6 | D45 | budget cap, timeout, fallback, cache. |
| **Total nouveaux M17** | **83** | | |

Aggregate après M17 : **46 (M16) + 83 (M17) = 129 mcp-server tests**.

### 19.2 Integration tests

- Run mcp-server M17 contre playground (REST adapter via Werkzeug) →
  10 nouveaux e2e tests (Phase A endpoints + resources + prompts).
- Run mcp-server M17 contre EFE 1.1 dev cluster (gRPC) → 5 nouveaux e2e
  tests (couverture endpoints EFE 1.1 nouveaux).

### 19.3 Stress tests

- `bench/m17_bulk.py` — 1000 textes via `bulk_classify` ; assert ≥ 200
  req/s (M17-MCP-5).
- `bench/m17_diagnose.py` — catalog 256 classes × 64 examples ;
  assert p99 < 5 s (M17-MCP-4).
- `bench/m17_concurrent_sessions.py` — 10 mcp-server processes
  simultanés vs 1 backend ; vérifie pas de cross-tenant leak.

### 19.4 Audit pass M17

Pattern récurrent M11-M16 (~6-8 défauts attendus). Phase D46.
Targets :
- 1 HIGH attendu (peut-être dans le sampling budget enforcement).
- 4-6 MED (typiquement : edge cases dans diagnose pour catalogs
  pathologiques, threshold calibration sur catalog déséquilibré, etc.).
- 1-2 LOW (logs verbosity, doc gaps).

---

## 20. Deployment scenarios

### 20.1 Local dev (Claude Desktop) — profile par défaut

```json
{
  "mcpServers": {
    "efe": {
      "command": "efe-mcp-server",
      "env": {
        "EFE_OIDC_TOKEN": "eyJhbG...",
        "EFE_ENDPOINT": "https://efe.example.com",
        "EFE_MCP_TOOL_PROFILE": "agent-native"
      }
    }
  }
}
```

### 20.2 Air-gapped / on-prem

Profile=`advanced` par default (sampling indispo dans certains envs
air-gapped car le LLM hôte n'est pas accessible). Si LLM local
(Ollama, LlamaCpp) → profile=`agent-native` OK.

### 20.3 Cloud HTTP+SSE (M19, hors-scope M17 livraison)

Mentionné pour cohérence ; pas livré M17.

### 20.4 CI / agent batch

`agent-native` profile + sampling possible vs Anthropic API + token
batch processing :
```bash
EFE_MCP_TOOL_PROFILE=agent-native \
EFE_OIDC_TOKEN=$EFE_TOKEN \
EFE_ENDPOINT=$EFE_URL \
efe-mcp-server &  # in CI workflow with agent SDK
```

---

## 21. Roadmap M18+

### 21.1 M18 — « Bundle & deploy tools » (Q3 2026, indicatif)

- Tool `efe_inspect_bundle` (lecture bundle Ed25519, SBOM extract).
- Tool `efe_dry_run_deploy` (vérification pré-déploiement).
- Resources `efe://bundles/{slug}/{version}`.
- Pas d'écriture (deploy reste via Helm + cosign CLI hors-MCP).

### 21.2 M19 — « Cloud transport + multi-replica » (Q4 2026)

- Implémente `_run_http()` (HTTP+SSE on :8800).
- Stateless mcp-server multi-replica ; rate-limit Redis-backed.
- Unix-socket sidecar pattern productionnalisé.

### 21.3 M20 — « Multi-modal » (2027 H1, conditional)

- Encode RPC accepte image / audio (nouveau bundle modèle multimodal).
- Tools EFE classify multi-modal.
- Hors scope M17 ; mentionné pour traçabilité roadmap.

---

## 22. Open questions

1. **Sampling pricing transparency** : faut-il que `efe_curate_catalog`
   retourne dans son output le nombre de sampling rounds consommés +
   estimated_cost_tokens ? Pour aider l'utilisateur à voir le coût
   incurr de son LLM. **Décision** : oui, ajouter
   `sampling_usage: { rounds: 5, total_tokens_in: 1200, total_tokens_out: 800 }`.
2. **Resource @-mention semantics** : si user @-mention
   `efe://catalogs/X` dans une conversation Claude Desktop, le LLM voit
   le contenu directement vs un placeholder ? **Décision** : laisser le
   client MCP décider ; le serveur retourne du JSON canonique, le client
   peut le pretty-print.
3. **Catalog version pinning in resources** : `efe://catalogs/X` retourne
   latest. Faut-il un alias `efe://catalogs/X/latest` explicite + rendre
   `efe://catalogs/X` pinned-immutable à la 1ère lecture ? **Décision** :
   non, simplicité avant tout. `efe://catalogs/X` = always latest.
   `efe://catalogs/X/versions/{v}` = pinned.
4. **Prompt template vs slash-command** : Claude Desktop introduit slash
   commands. Faut-il ré-exposer les 5 prompts en tant que slash commands
   aussi ? **Décision** : reporté M19 ; spec des slash commands MCP pas
   stable au moment de M17.
5. **Anti-régression `core` profile** : faut-il un test dédié qui chronomètre
   M16 perf vs M17 profile=`core` ? **Décision** : oui, ajouter
   `bench/m17_core_perf_regression.py` (assert p99 stable ± 10 %).
6. **Tenant scoping for resources/list** : `resources/list` doit-il
   retourner uniquement les resources accessibles au tenant courant ?
   **Décision** : oui (D11 zero-trust mirror) ; mais la liste contient
   les *patterns*, pas des instances enumerées (l'enumération viendrait
   par `resources/read` qui filtre par tenant_id du JWT).

---

## 23. FAQ

**Q: Pourquoi 14 outils plutôt que 6 plus puissants ?**
A: Granularité. Un outil composite (`efe_diagnose_catalog`) compose
en interne 4 primitives (`compare_classes`, `score_distribution`,
`detect_drift`, audit lookup). On expose les deux niveaux. L'agent
choisit selon contexte (debug → composite, fine-grained →
primitives).

**Q: Pourquoi sampling et pas un agent EFE-managed ?**
A: Sub-processor compliance. Cf. §2.3.

**Q: Sampling rate-limit — comment l'opérateur voit la conso ?**
A: Audit log a un champ `sampling_rounds_used` par tool call
(`mcp_call.efe_curate_catalog` event metadata). Aggregable via dashboard
M14 D13 (extension de runbook M17 D45).

**Q: Comment tester localement sans Claude Desktop ?**
A: Le mcp-server M17 fallback JSON-RPC (M16 carry-over) supporte
`tools/list`, `tools/call`, `resources/list`, `resources/read`,
`prompts/list`, `prompts/get`. Pas de sampling ni progress (nécessitent
une session interactive). `pytest mcp-server/tests/` couvre les autres.

**Q: Si EFE 1.0 server (pas 1.1) — que ratent les utilisateurs ?**
A: 4 fallbacks plus lents + 1 erreur explicite (`consolidate_versions`).
Tout le reste fonctionne. Doc de migration en `docs/EFE_MCP_M17_MIGRATION.md`
(à livrer en D44).

**Q: Pricing impact ?**
A: Aucun pour le serveur mcp-server (open source, gratuit). Le LLM
sampling consomme des tokens du compte LLM de l'utilisateur — c'est
transparent par design (pas un coût caché EFE).

---

## 24. Glossary M17

| Term | Définition |
|---|---|
| **Agent-native** | Conçu pour être appelé par un agent LLM, avec primitives qui composent + raisonnent (vs CRUD atomique). |
| **Sampling (MCP)** | Capability MCP : le serveur demande au LLM hôte de générer du texte. Inverse-direction du tool-call habituel. |
| **Resource (MCP)** | Lecture URI-adressable exposée par le serveur. Lecture uniquement, pas d'args. |
| **Prompt (MCP)** | Template de prompt pré-construit, sélectionné par utilisateur dans le client. |
| **Profile** | Mode de fonctionnement mcp-server (`core`/`advanced`/`agent-native`/`experimental`). |
| **Composite tool** | Outil qui compose plusieurs primitives côté serveur (ex: `diagnose_catalog`). |
| **Streaming progress** | Émission de `notifications/progress` MCP pendant un long tool call. |
| **Confirm-required** | Pattern M16 carry-over : `confirm: true` requis pour write tools (et étendu en M17 pour sampling-heavy tools). |
| **Sampling budget** | Hard cap 5 rounds par tool call. |
| **Drift** | Évolution sémantique d'un catalogue entre 2 versions. |
| **Calibration threshold** | Seuil per-classe ajusté pour cible précision/rappel. |
| **EFE 1.1 contract** | Bump semver-mineur ; 3 nouveaux endpoints REST + 3 RPCs gRPC additifs. |

---

## 25. References

- **Spec MCP officielle** : <https://modelcontextprotocol.io/specification>
- **MCP Python SDK** : `mcp>=1.0,<2.0` — <https://github.com/modelcontextprotocol/python-sdk>
- **EFE M16 livraison** : `mcp-server/` + `docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md`
- **EFE D11 zero-trust** : `docs/bench/d11_per_tenant_isolation_architecture.md`
- **EFE D12 catalog** : `docs/bench/d12_catalog_admin_versioning_architecture.md`
- **EFE D14-bis HMAC chain** : `docs/bench/d14bis_hmac_audit_chain_architecture.md`
- **EFE D15 OIDC** : `docs/bench/d15_oidc_jwt_architecture.md`
- **EFE M14 threat model** : `docs/security/threat_model.md`
- **EFE M14 GDPR Art. 32** : `docs/compliance/gdpr_art32.md` (à étendre §3.5 sampling)
- **EFE M15 playground tutorials** : `playground/tutorials/03_drift_detection.md` (deepened by `efe_detect_drift`)
- **scikit-learn MiniBatchKMeans** : <https://scikit-learn.org/stable/modules/clustering.html#mini-batch-k-means>

---

## A — Full JSON Schemas

### A.1 `efe_explain_classification`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeExplainClassificationInput",
  "description": "Explain why a given classification was returned.",
  "properties": {
    "catalog":         { "type": "string", "minLength": 1, "maxLength": 100 },
    "text":            { "type": "string", "minLength": 1, "maxLength": 8192 },
    "predicted_class": { "type": "string", "minLength": 1, "maxLength": 100 }
  },
  "required": ["catalog", "text"],
  "additionalProperties": false
}
```

### A.2 `efe_compare_classes`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeCompareClassesInput",
  "description": "Compare two classes within a catalog for ambiguity.",
  "properties": {
    "catalog": { "type": "string", "minLength": 1, "maxLength": 100 },
    "class_a": { "type": "string", "minLength": 1, "maxLength": 100 },
    "class_b": { "type": "string", "minLength": 1, "maxLength": 100 }
  },
  "required": ["catalog", "class_a", "class_b"],
  "additionalProperties": false
}
```

### A.3 `efe_score_distribution`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeScoreDistributionInput",
  "description": "Histogram of scores for a class against random / provided texts.",
  "properties": {
    "catalog":           { "type": "string", "minLength": 1, "maxLength": 100 },
    "class_name":        { "type": "string", "minLength": 1, "maxLength": 100 },
    "sample_count":      { "type": "integer", "minimum": 10, "maximum": 1000, "default": 100 },
    "texts":             { "type": "array", "items": { "type": "string", "maxLength": 8192 }, "maxItems": 1000 },
    "include_negatives": { "type": "boolean", "default": true }
  },
  "required": ["catalog", "class_name"],
  "additionalProperties": false
}
```

### A.4 `efe_suggest_classes`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeSuggestClassesInput",
  "description": "Cluster examples into classes ; sampling-based naming.",
  "properties": {
    "examples": {
      "type": "array",
      "minItems": 10,
      "maxItems": 2000,
      "items": {
        "type": "object",
        "properties": { "text": { "type": "string", "minLength": 1, "maxLength": 8192 } },
        "required": ["text"],
        "additionalProperties": false
      }
    },
    "target_class_count": { "type": "integer", "minimum": 2, "maximum": 50, "default": 5 },
    "language_hint":      { "type": "string", "minLength": 2, "maxLength": 10, "default": "en" },
    "confirm":            { "type": "boolean" }
  },
  "required": ["examples", "confirm"],
  "additionalProperties": false
}
```

### A.5 `efe_find_similar_examples`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeFindSimilarExamplesInput",
  "properties": {
    "catalog": { "type": "string", "minLength": 1, "maxLength": 100 },
    "text":    { "type": "string", "minLength": 1, "maxLength": 8192 },
    "top_k":   { "type": "integer", "minimum": 1, "maximum": 32, "default": 5 },
    "scope":   {
      "type": "string",
      "enum": ["all_classes", "top_class"],
      "default": "all_classes"
    }
  },
  "required": ["catalog", "text"],
  "additionalProperties": false
}
```

### A.6 `efe_propose_examples`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeProposeExamplesInput",
  "properties": {
    "catalog":          { "type": "string", "minLength": 1, "maxLength": 100 },
    "class_name":       { "type": "string", "minLength": 1, "maxLength": 100 },
    "n_proposed":       { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 },
    "diversity_target": { "type": "string", "enum": ["low", "medium", "high"], "default": "medium" },
    "confirm":          { "type": "boolean" }
  },
  "required": ["catalog", "class_name", "confirm"],
  "additionalProperties": false
}
```

### A.7 `efe_diagnose_catalog`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeDiagnoseCatalogInput",
  "properties": {
    "catalog":              { "type": "string", "minLength": 1, "maxLength": 100 },
    "depth":                { "type": "string", "enum": ["quick", "standard", "full"], "default": "standard" },
    "include_examples_text":{ "type": "boolean", "default": false }
  },
  "required": ["catalog"],
  "additionalProperties": false
}
```

### A.8 `efe_detect_drift`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeDetectDriftInput",
  "properties": {
    "catalog":  { "type": "string", "minLength": 1, "maxLength": 100 },
    "version_a":{ "type": "integer", "minimum": 1 },
    "version_b":{ "type": "integer", "minimum": 1 }
  },
  "required": ["catalog"],
  "additionalProperties": false
}
```

### A.9 `efe_audit_health_check`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeAuditHealthCheckInput",
  "properties": {
    "from_timestamp": { "type": "number" },
    "to_timestamp":   { "type": "number" },
    "deep":           { "type": "boolean", "default": false }
  },
  "additionalProperties": false
}
```

### A.10 `efe_curate_catalog`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeCurateCatalogInput",
  "properties": {
    "catalog":     { "type": "string", "minLength": 1, "maxLength": 100 },
    "intent":      {
      "type": "string",
      "enum": ["balance_class_sizes", "remove_ambiguity", "augment_underrepresented"]
    },
    "max_changes": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
    "confirm":     { "type": "boolean" }
  },
  "required": ["catalog", "intent", "confirm"],
  "additionalProperties": false
}
```

### A.11 `efe_calibrate_thresholds`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeCalibrateThresholdsInput",
  "properties": {
    "catalog":       { "type": "string", "minLength": 1, "maxLength": 100 },
    "target_metric": { "type": "string", "enum": ["precision", "recall", "f1"], "default": "f1" },
    "target_value":  { "type": "number", "minimum": 0.0, "maximum": 1.0, "default": 0.9 },
    "validation_set":{
      "type": "array",
      "maxItems": 1000,
      "items": {
        "type": "object",
        "properties": {
          "text": { "type": "string", "maxLength": 8192 },
          "true_class": { "type": "string", "maxLength": 100 }
        },
        "required": ["text", "true_class"],
        "additionalProperties": false
      }
    },
    "min_per_class_samples": { "type": "integer", "minimum": 5, "maximum": 1000, "default": 30 }
  },
  "required": ["catalog"],
  "additionalProperties": false
}
```

### A.12 `efe_consolidate_versions`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeConsolidateVersionsInput",
  "properties": {
    "catalog":     { "type": "string", "minLength": 1, "maxLength": 100 },
    "keep_last_n": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 10 },
    "dry_run":     { "type": "boolean", "default": true },
    "confirm":     { "type": "boolean" }
  },
  "required": ["catalog"],
  "additionalProperties": false,
  "if":   { "properties": { "dry_run": { "const": false } } },
  "then": { "required": ["confirm"] }
}
```

### A.13 `efe_bulk_classify`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeBulkClassifyInput",
  "properties": {
    "catalog":              { "type": "string", "minLength": 1, "maxLength": 100 },
    "texts":                { "type": "array", "minItems": 1, "maxItems": 1000, "items": { "type": "string", "minLength": 1, "maxLength": 8192 } },
    "top_k":                { "type": "integer", "minimum": 1, "maximum": 32, "default": 1 },
    "include_explanations": { "type": "boolean", "default": false }
  },
  "required": ["catalog", "texts"],
  "additionalProperties": false
}
```

### A.14 `efe_export_catalog`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeExportCatalogInput",
  "properties": {
    "catalog":              { "type": "string", "minLength": 1, "maxLength": 100 },
    "version":              { "type": "integer", "minimum": 1 },
    "format":               { "type": "string", "enum": ["json", "yaml"], "default": "json" },
    "include_centroids":    { "type": "boolean", "default": false },
    "include_audit_excerpt":{ "type": "boolean", "default": false }
  },
  "required": ["catalog"],
  "additionalProperties": false
}
```

---

## B — Resource URI grammar

Formal ABNF (RFC 5234) :

```abnf
efe-uri    = "efe://" path [ "?" query ]
path       = catalogs-path / audit-path / quotas-path / samples-path

catalogs-path = "catalogs/" name [ catalog-detail ]
catalog-detail = "/versions/" version
              / "/classes/" class-name

audit-path  = "audit/recent"
quotas-path = "quotas"
samples-path = "samples/" sample-id

name        = name-start *99name-cont
name-start  = ALPHA / DIGIT
name-cont   = ALPHA / DIGIT / "-" / "_"

class-name  = name-start *99class-cont
class-cont  = ALPHA / DIGIT / "-" / "_" / "."

version     = 1*5DIGIT
sample-id   = name-start *49name-cont

query       = key-value *( "&" key-value )
key-value   = key "=" value
key         = "limit" / "since" / "format" / "cursor"
value       = 1*VCHAR

ALPHA       = %x41-5A / %x61-7A
DIGIT       = %x30-39
VCHAR       = %x21-7E
```

Toute URI hors-grammaire → erreur `400 invalid_resource_uri` (TM-M17-E-01).

---

## C — Prompt templates

### C.1 `classify_with_reasoning`

(Voir §8.4.)

### C.2 `bootstrap_catalog_from_csv`

```markdown
---
name: bootstrap_catalog_from_csv
description: Walk a user from a raw CSV of texts to a registered EFE catalog.
arguments:
  - { name: csv_path, required: true }
  - { name: target_classes_hint, required: false }
---

I have a CSV file at `{csv_path}` containing one text per row.
{#if target_classes_hint}I'd like roughly {target_classes_hint} classes.{/if}

Please :
1. Read the CSV (you may use a file-reading tool of your choice).
2. Call `efe_suggest_classes` with the texts and `confirm:true`.
3. Show me the proposed classes ; let me approve or refine each one.
4. Once I approve, call `efe_register_catalog` with `confirm:true`.
5. Run `efe_diagnose_catalog` to confirm the catalog is healthy.
```

### C.3 `audit_compliance_check`

```markdown
---
name: audit_compliance_check
description: Quarterly compliance walkthrough of EFE audit logs.
arguments:
  - { name: tenant, required: false }
  - { name: since, required: false }
---

I need to verify the EFE audit log integrity for compliance purposes.

Please :
1. Call `efe_audit_health_check`{#if since} with `from_timestamp={since}`{/if}.
   Use `deep:true` for a full re-verification of the HMAC chain.
2. Call `efe_get_audit_log` with `limit:200` and report the action
   distribution (count by action type).
3. If `efe_audit_health_check` returns `ok:false`, print the
   `broken_at` event and recommend escalation per
   `docs/runbooks/m11_audit_chain_break.md`.
```

### C.4 `drift_investigation`

```markdown
---
name: drift_investigation
description: Investigate why a class suddenly underperforms.
arguments:
  - { name: catalog, required: true }
  - { name: class, required: true }
---

Class `{class}` in catalog `{catalog}` seems to be misclassifying.
Help me investigate :

1. Call `efe_diagnose_catalog` with `catalog={catalog}` and `depth:full`.
2. Call `efe_detect_drift` for `{catalog}` (latest vs prev version).
3. Call `efe_score_distribution` with `class_name:{class}` and
   `sample_count:200`.
4. Summarize : (a) overall health, (b) drift since last version,
   (c) recommended threshold, (d) ambiguity neighbors.
5. If overall_health < "fair", suggest concrete next actions
   (calibrate, evolve, etc.).
```

### C.5 `threshold_calibration_session`

```markdown
---
name: threshold_calibration_session
description: Guided per-class threshold tuning.
arguments:
  - { name: catalog, required: true }
  - { name: target_precision, required: false }
---

Let's calibrate thresholds for catalog `{catalog}`.
Target precision : {target_precision?0.9}.

1. Call `efe_calibrate_thresholds` with `target_metric:precision` and
   `target_value:{target_precision?0.9}`.
2. Show me the per-class recommended thresholds with expected metrics.
3. For each class where `expected_recall < 0.6`, flag and suggest
   an alternative target_metric (e.g. `f1`).
4. When I approve, output the exact `efe_evolve_catalog` payload to
   apply the new thresholds (do NOT call evolve without my explicit
   confirmation).
```

---

## D — Pin tests inventory

83 nouveaux pin tests, organisés par fichier :

### D.1 `test_profile_gating.py` (6)

- `test_default_profile_is_core`
- `test_profile_advanced_includes_8_extra_tools`
- `test_profile_agent_native_includes_14_extra_tools`
- `test_unknown_profile_raises`
- `test_core_profile_six_tools_unchanged_from_m16`
- `test_profile_env_var_case_insensitive`

### D.2 `test_reasoning_tools.py` (12)

- `test_explain_classification_returns_top_contributors`
- `test_explain_classification_predicts_when_not_provided`
- `test_explain_classification_threshold_sensitivity_range`
- `test_explain_classification_unknown_catalog_returns_not_found`
- `test_compare_classes_returns_separation_score`
- `test_compare_classes_identifies_boundary_examples`
- `test_compare_classes_invalid_class_name_400`
- `test_compare_classes_self_comparison_returns_max_separation`
- `test_score_distribution_default_sample_count_100`
- `test_score_distribution_with_provided_texts`
- `test_score_distribution_recommends_threshold_above_current`
- `test_score_distribution_max_sample_count_1000_enforced`

### D.3 `test_discovery_tools.py` (14)

- `test_suggest_classes_returns_target_count`
- `test_suggest_classes_outliers_below_cohesion_threshold`
- `test_suggest_classes_sampling_fallback_heuristic`
- `test_suggest_classes_sampling_budget_enforced_5_rounds`
- `test_suggest_classes_requires_confirm_true`
- `test_suggest_classes_min_examples_10`
- `test_find_similar_examples_top_k_default_5`
- `test_find_similar_examples_scope_all_classes`
- `test_find_similar_examples_scope_top_class`
- `test_find_similar_examples_below_max_top_k_32`
- `test_propose_examples_n_proposed_max_10`
- `test_propose_examples_diversity_target_changes_novelty_scores`
- `test_propose_examples_filters_off_topic_below_0_4_similarity`
- `test_propose_examples_requires_confirm_true`

### D.4 `test_diagnostic_tools.py` (12)

- `test_diagnose_catalog_quick_depth_skips_ambiguity_matrix`
- `test_diagnose_catalog_full_depth_returns_all_scores`
- `test_diagnose_catalog_includes_recommended_actions`
- `test_diagnose_catalog_pii_scrubbed_when_include_examples_text_false`
- `test_diagnose_catalog_overall_health_critical_threshold`
- `test_detect_drift_default_compares_latest_vs_prev`
- `test_detect_drift_explicit_versions`
- `test_detect_drift_new_classes_listed`
- `test_detect_drift_removed_classes_listed`
- `test_audit_health_check_incremental_default`
- `test_audit_health_check_deep_verifies_full_chain`
- `test_audit_health_check_broken_chain_returns_broken_at`

### D.5 `test_composite_tools.py` (12)

- `test_curate_catalog_dry_run_default_returns_plan_only`
- `test_curate_catalog_emits_progress_events`
- `test_curate_catalog_requires_confirm_for_sampling_budget`
- `test_curate_catalog_sampling_budget_5_rounds_enforced`
- `test_curate_catalog_intent_balance_class_sizes`
- `test_curate_catalog_intent_remove_ambiguity`
- `test_calibrate_thresholds_target_precision_returns_per_class`
- `test_calibrate_thresholds_streams_one_event_per_class`
- `test_calibrate_thresholds_validation_set_used_when_provided`
- `test_consolidate_versions_dry_run_default_true`
- `test_consolidate_versions_apply_requires_confirm_and_dry_run_false`
- `test_consolidate_versions_keep_last_n_min_1`

### D.6 `test_streaming_tools.py` (8)

- `test_bulk_classify_max_1000_texts`
- `test_bulk_classify_emits_progress_per_50_items`
- `test_bulk_classify_summary_count_matches_results`
- `test_bulk_classify_below_threshold_count_correct`
- `test_export_catalog_format_json_default`
- `test_export_catalog_format_yaml`
- `test_export_catalog_streams_when_size_above_1mb`
- `test_streaming_cancellation_emits_audit_event`

### D.7 `test_resources.py` (8)

- `test_resources_list_returns_six_types`
- `test_resource_efe_catalogs_summary_returns_latest`
- `test_resource_efe_catalogs_versions_specific`
- `test_resource_efe_catalogs_classes_returns_examples`
- `test_resource_efe_audit_recent_paginates`
- `test_resource_efe_quotas_returns_d12_limits`
- `test_resource_uri_traversal_rejected_400`
- `test_resource_unknown_path_returns_404`

### D.8 `test_prompts.py` (5)

- `test_prompts_list_returns_five_templates`
- `test_prompt_classify_with_reasoning_substitutes_variables`
- `test_prompt_bootstrap_catalog_optional_var_missing_skips_block`
- `test_prompt_unknown_returns_404`
- `test_prompt_variable_injection_escaped`

### D.9 `test_sampling.py` (6)

- `test_sampling_budget_default_5_rounds`
- `test_sampling_budget_exceeded_raises_after_5`
- `test_sampling_timeout_default_10s`
- `test_sampling_cache_60s_ttl`
- `test_sampling_fallback_when_client_unsupported`
- `test_sampling_audit_event_records_rounds_used`

---

## E — Backend contract diff EFE 1.0 → 1.1

### E.1 New REST endpoints (additive)

| Method | Path | Auth | Notes |
|---|---|---|---|
| POST | `/catalog/<name>/explain` | OIDC | body : `{ text, predicted_class? }` |
| POST | `/catalog/<name>/compare` | OIDC | body : `{ class_a, class_b }` |
| POST | `/catalog/<name>/score-distribution` | OIDC | body : `{ class_name, sample_count?, texts?, include_negatives? }` |
| POST | `/catalog/<name>/find-similar` | OIDC | body : `{ text, top_k?, scope? }` |
| POST | `/catalog/<name>/diagnose` | OIDC | body : `{ depth? }` |
| POST | `/catalog/<name>/drift` | OIDC | body : `{ version_a?, version_b? }` |
| POST | `/catalog/<name>/calibrate` | OIDC admin | body : `{ target_metric, target_value, validation_set? }` |
| POST | `/catalog/<name>/consolidate` | OIDC admin | body : `{ keep_last_n, dry_run, idempotency_key? }` |
| POST | `/catalog/<name>/bulk-classify` | OIDC | body : `{ texts, top_k?, include_explanations? }` ; returns NDJSON if `Accept: application/x-ndjson` |
| GET  | `/catalog/<name>/export` | OIDC admin | query : `version?`, `format=json\|yaml`, `include_centroids?` |

### E.2 New gRPC RPCs (additive)

```proto
service EfeService {
  // existing M16 + EFE 1.0 RPCs unchanged …

  rpc Explain(ExplainRequest) returns (ExplainResponse);
  rpc Diagnose(DiagnoseRequest) returns (DiagnoseResponse);
  rpc BulkClassify(BulkClassifyRequest) returns (stream BulkClassifyChunk);
}
```

### E.3 Backward compat guarantees

- Tous les endpoints / RPCs M16 + EFE 1.0 GA inchangés.
- Tout client EFE 1.0 reste compatible avec un server EFE 1.1.
- Un mcp-server M17 profile=`core` ne touche aucun endpoint EFE 1.1
  (zero coupling).
- Un mcp-server M17 profile=`advanced`/`agent-native` détecte
  EFE 1.0 vs 1.1 via `GET /info` et fallback approprié (cf. §14.4).

### E.4 Migration plan

| Étape | Action | Owner | Quand |
|---|---|---|---|
| 1 | Bump `Cargo.toml` `efe-server` v1.0.x → v1.1.0-pre | Rust lead | D43 J1 |
| 2 | Implémenter 10 endpoints REST + 3 RPCs gRPC | Rust lead | D43 J1-J5 |
| 3 | Pin tests `crates/efe-server/tests/m17_contract.rs` (~25) | Rust lead | D43 J3-J7 |
| 4 | Bump OpenAPI spec + .proto regen + re-generate Python clients | Rust lead | D43 J7 |
| 5 | Release EFE 1.1.0 image (additive, backward-compat) | Release manager | D43 J10 |
| 6 | mcp-server M17 detect 1.1 via `/info` flag | Python lead | D44 J1 |
| 7 | mcp-server fallback paths for 1.0 server | Python lead | D44 J1-J3 |

---

**FIN**

Document M17 architecture. ~2 800 lignes. Prêt à passer en livraison
sprint D43. Toute revue chirurgicale : ouvrir issue
`docs/m17_review_pass_*` ; pattern récurrent M11-M16.
