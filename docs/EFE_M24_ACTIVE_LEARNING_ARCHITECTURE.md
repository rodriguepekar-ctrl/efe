# EFE M24 — Active Learning Loop — Architecture détaillée

> **Statut** : architecture livrable du sprint M24 (active learning
> loop), suite directe de M23 (LLM-assisted curation) et avant M25
> (agent runtime).
>
> **Auteur** : tech lead EFE.
> **Date** : 2026-05-09 (post-M23 audit).
> **Sprint cible** : D74-D79 (Q3 2027) — 6 jours-ingénieur effectifs.
> **Précision** : ultrathink + ultra-détaillé. Précision chirurgicale
> suprême. Document spécification-as-code : implémentable sans
> ambiguïté.
> **Lettre EFE** : `J` (entre I = M23 et K = M25).
>
> **Pré-requis lecture** :
> - [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md) § 10 — la mission M24 en 90 lignes.
> - [`EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md) — gold_dataset table + curation_review pattern + canary engine + audit chain HMAC + Art. 14 hooks.
> - [`EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`](EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md) — LLMRouter, anchor centroids, cost governance.
> - [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md) — observabilité, BYOK, PII redactor, audit chain HMAC.
> - [`EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md`](EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md) — bundle deploy flow (réutilisé par M24 pour promotion).
> - [`EFE_KERNELS_ARCHITECTURE.md`](EFE_KERNELS_ARCHITECTURE.md) — Mode-1 anchor classification + centroid math.
> - **EU AI Act 2024/1689** Art. 11 + 14 + 15 — technical doc, human oversight, accuracy.
> - **GDPR Art. 17** (right to erasure) + Art. 22 (automated decision-making).

---

## Sommaire

1. [Mission + executive summary](#1-mission--executive-summary)
2. [Scope / non-scope chirurgicaux](#2-scope--non-scope)
3. [AS-IS post-M23 + R-100 GA](#3-as-is-post-m23--r-100)
4. [TO-BE — vision M24](#4-to-be--vision-m24)
5. [Closed-loop pipeline end-to-end](#5-closed-loop-pipeline)
6. [Architectural Decision Records (ADR)](#6-architectural-decision-records)
7. [Uncertainty sampler — sample selection](#7-uncertainty-sampler)
8. [Gold label queue (Postgres + Redis stream)](#8-gold-label-queue)
9. [LLM labeling worker](#9-llm-labeling-worker)
10. [Human review extension (vs M23)](#10-human-review-extension)
11. [Auto-confirm gate (compliance-aware)](#11-auto-confirm-gate)
12. [Bundle re-tune pipeline](#12-bundle-retune-pipeline)
13. [Embedder re-calibration — centroid update math](#13-embedder-recalibration)
14. [Holdout split manager](#14-holdout-split-manager)
15. [Two-stage eval engine (offline + canary)](#15-eval-engine)
16. [Promotion gate + auto-promote](#16-promotion-gate)
17. [GDPR Art. 17 cascade — tombstone propagation](#17-gdpr-art17-cascade)
18. [Data model (SQL + migration 0015)](#18-data-model)
19. [State machines](#19-state-machines)
20. [gRPC service contracts](#20-grpc-service-contracts)
21. [mcp-server tools](#21-mcp-server-tools)
22. [efe-admin CLI extensions](#22-efe-admin-cli-extensions)
23. [portal-admin UI extensions](#23-portal-admin-ui-extensions)
24. [Multi-tenant isolation M24](#24-multi-tenant-isolation)
25. [Cost governance M24](#25-cost-governance)
26. [Compliance + AI Act + GDPR](#26-compliance)
27. [Threat model M24 (delta vs M23)](#27-threat-model)
28. [Backend contract evolution (1.6 → 1.7)](#28-backend-contract-evolution)
29. [Bench + pin tests](#29-bench--pin-tests)
30. [Activation flags + dependency validation](#30-activation-flags)
31. [Sprint plan D74-D79](#31-sprint-plan)
32. [Risk register](#32-risk-register)
33. [Open questions / decisions needed](#33-open-questions)
34. [Success metrics + GA gate](#34-success-metrics)
35. [Glossary M24+](#35-glossary)
36. [References](#36-references)

**Annexes**

- [A — LLM labeling prompt templates](#a--prompt-templates)
- [B — Cost model M24 ($/tenant/month)](#b--cost-model)
- [C — Holdout split invariants + math](#c--holdout-split)
- [D — Pin tests inventory M24 (+100 cible)](#d--pin-tests-inventory)
- [E — Compliance attestation chain (extension)](#e--compliance-attestation-chain)
- [F — Migration 0015_active_learning.sql complète](#f--migration-0015)

---

## 1. Mission + executive summary

### 1.1 Mission en une phrase

> **M24 ferme la boucle production → curation → réentraînement** :
> chaque classify de basse confiance est automatiquement étiqueté
> (LLM oracle + humain en option), accumulé dans un gold dataset
> versionné, et déclenche périodiquement un re-tune des centroids
> d'embedder ; le nouveau bundle n'est promu que s'il bat l'ancien
> sur un holdout statistiquement significatif — sous gouvernance
> AI Act + GDPR + audit chain HMAC.

### 1.2 Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Boucle d'active learning : production → gold → retune → A/B → promote (closed-loop ML, no fine-tune). |
| **Sprint** | D74-D79 (6 jours), Q3 2027. |
| **Lettre EFE** | `J`. |
| **Code volume** | ~3 800 LOC Rust (`crates/efe-active-learning/`) + ~1 800 LOC Python (`mcp-server/active_learning/`) + ~1 200 LOC TS (`apps/portal-admin/learning/`) + ~1 600 LOC tests + ~280 LOC SQL migrations + ~150 LOC scripts ops. |
| **Pin tests** | +100 GREEN cible (cf. § 29.3 inventory). |
| **Backward compat** | 100 % préservé. M24 OFF par défaut (4-level flag : server / tenant / catalog / per-call). Si OFF, EFE bit-for-bit identique à v1.0.0 GA + M22 + M23. |
| **Dependency hard** | **Requires M21 + M23 ON**. Validation startup : `EFE_FEATURE_M24_REQUIRE_M21_M23=true` (default true) refuse le boot si M24 ON sans M21+M23. |
| **Compliance gate** | AI Act Art. 11 + 14 + 15 obligatoires si activé. GDPR Art. 22 (automated decision) gated derrière `auto_promotion=true` + `human_review_required=false` qui n'est autorisé QUE pour `catalog.stakes_tier=low`. |
| **Activation cible** | 20 % des tenants pilots à T+90 j (M24-AL-2 SLO). |
| **Eval cible** | Promotion accuracy delta ≥ +1 pp (M24-AL-5 SLO). |
| **Latency cible** | Gold-label queue lag p99 < 1 h (M24-AL-1 SLO). |
| **Risk profile** | **High** — circuit-fermé automatique + impact direct sur la qualité production. Risques principaux : adversarial labeling (poisoning), drift evasion (slow-poison), cost runaway (queue overflow), reviewer rubber-stamping. |
| **Strategic outcome** | EFE devient un **classifier auto-amélioré** sans fine-tune. Différenciateur produit final vs le concurrentiel : aucun classifier as-a-service (AWS Comprehend, Azure AI Language, OpenAI moderation) ne livre cette boucle avec audit chain HMAC + AI Act compliance built-in. |

### 1.3 TL;DR sequencing

| Jour | Cœur livraison | Pin tests cumulés |
|---|---|:---:|
| **D74** | DB migration 0015 + crate `efe-active-learning/` + uncertainty sampler + sample lifecycle state machine | +18 |
| **D75** | Gold label queue (Postgres + Redis stream) + LLM labeling worker + cost governance extension | +20 |
| **D76** | Human review extension + auto-confirm gate + GDPR Art. 17 cascade hooks | +18 |
| **D77** | Bundle retune pipeline + embedder re-calibration (centroid math) + holdout split manager | +18 |
| **D78** | Two-stage eval + promotion gate + canary integration + portal-admin learning queue | +14 |
| **D79** | CLI commands + bench scenarios + threat model adversarial tests + GA gate audit | +12 |

### 1.4 Frontière de sécurité chirurgicale (invariante M19→M24)

Les 7 invariants cumulés sont préservés :

1. **Bundle immutability** — un bundle promu reste pullable forever ;
   M24 produit toujours une nouvelle version (bump patch) sans muter
   les versions précédentes.
2. **Audit chain HMAC unbroken** — chaque sample enqueué, chaque
   labeling LLM, chaque review humain, chaque retune run, chaque
   promote/rollback s'inscrit dans une nouvelle table
   `learning_audit` chaînée HMAC à `taxonomy_change_audit`
   (M23) et `llm_call_audit` (M19).
3. **No tenant data leak across LLM prompts** — `efe-pii::redact`
   (M19) appliqué avant tout LLM call ; per-tenant prompt context
   isolation ; RLS Postgres sur toutes les nouvelles tables.
4. **Per-tenant LLM cost budget** — réutilise M21 budget infra ;
   M24 ajoute catégorie `active_learning` à côté de `mode2`/`mode3`/`curation`.
   Hard-cap pre-call + reconciliation quotidienne ; circuit-breaker
   si budget épuisé en milieu de batch.
5. **AI Act Art. 14 human oversight** — par défaut `human_review_required=true`
   (auto-confirm seulement opt-in explicite + low-stakes catalog).
6. **AI Act Art. 15 accuracy** — la promotion gate est l'assurance
   accuracy : `accuracy_delta_pp ≥ +1.0` sur le holdout ; rollback
   automatique si post-promote drift > 2pp.
7. **GDPR Art. 17 right-to-forget** — tombstone-with-chain-preservation :
   les rows `gold_example` taggées pour effacement sont conservées
   (HMAC chain intact) mais leur contenu est masqué + propagation
   cascade vers les bundles qui les ont indexées (rebuild centroid sans).

---

## 2. Scope / non-scope

### 2.1 In-scope M24

✅ **Backend Rust** : nouveau crate `crates/efe-active-learning/` avec :
   - Uncertainty sampler (low-confidence selector + duplicate-suppression)
   - Gold label queue (Postgres durable + Redis stream pour distribution)
   - LLM labeling worker (async batch, drains queue every N min)
   - Auto-confirm gate (LLM confidence ≥ threshold + tenant consent + low-stakes)
   - Holdout split manager (deterministic 10% reservation per gold_dataset)
   - Bundle retune scheduler (cron + on-demand trigger)
   - Embedder re-calibration (centroid update only — no neural fine-tune)
   - Two-stage eval (offline holdout + canary 5%)
   - Promotion gate (auto OR operator-approved)
   - Drift watch (post-promote accuracy drop → freeze)
   - GDPR Art. 17 cascade (tombstone + bundle rebuild)
   - Audit chain `learning_audit` (HMAC chained, parent-linked to taxonomy_change_audit)

✅ **Workers** : 5 long-running async tasks dans efe-server :
   - `gold_label_enqueuer` — listens to `llm_call_audit` for `confidence < threshold`
   - `gold_label_llm_worker` — batched LLM calls (every 5 min default)
   - `gold_label_review_notifier` — alerts reviewers (email/Slack/PagerDuty)
   - `bundle_retune_scheduler` — weekly cron (configurable per-tenant)
   - `gdpr_art17_processor` — listens for tombstone requests

✅ **gRPC `ActiveLearningService`** : 16 RPCs (cf. § 20)

✅ **mcp-server tools** (4 nouveaux) :
   - `efe_list_gold_label_queue` — enumerate pending labels
   - `efe_confirm_gold_label` — manual operator confirm
   - `efe_trigger_bundle_retune` — on-demand re-tune
   - `efe_request_gdpr_erasure` — GDPR Art. 17 request

✅ **efe-admin CLI** (extension) : `efe-admin learning {queue,confirm,retune,promote,rollback,erase,stats}`

✅ **portal-admin UI** :
   - `/learning/queue` — gold label queue (low-confidence samples awaiting label)
   - `/learning/proposals` — LLM-proposed labels awaiting human review
   - `/learning/retunes` — retune history + promotion timeline
   - `/learning/holdout` — holdout dataset stats
   - `/learning/cost` — active-learning cost dashboard (extends M23)

✅ **Compliance hooks** :
   - GDPR Art. 17 erasure cascade (gold_dataset + bundle rebuild)
   - AI Act Art. 14 human review enforcement (cannot bypass without low_stakes flag)
   - AI Act Art. 15 accuracy gate (promotion threshold)
   - Per-tenant DPO consent flag for auto-promotion

✅ **Bench** : 6 nouveaux scenarios (active_learning_e2e, holdout_invariants,
   centroid_recalibration, promotion_safety, gdpr_art17_cascade,
   adversarial_poisoning).

✅ **Activation flags** : 5 niveaux (server / tenant / catalog / sample / per-call).

### 2.2 Non-scope M24 (reportés M25+)

❌ **Neural fine-tune** — M24 fait une **re-calibration de centroids**, pas
un fine-tune. Le fine-tune (LoRA / adapter / full retrain) est M25+ ou
out-of-scope EFE-1.x (peut nécessiter EFE 2.0).

❌ **Cross-tenant model sharing** — strictement interdit. Chaque tenant
a son propre cycle d'active learning, son propre gold dataset, ses
propres centroids.

❌ **Real-time per-classify retune** — out-of-scope. M24 fait du
batching (default weekly per active catalog).

❌ **Adaptive sampling sophistiqué** — V1 utilise un simple seuil de
confidence (`< 0.7`). Les méthodes adaptatives (BALD, DUE, query-by-
committee) sont annexe O-2.

❌ **Multi-LLM consensus labeling** — V1 utilise UN LLM call par sample.
Le consensus (3 LLMs vote majoritaire) est annexe O-3.

❌ **Active learning on Mode-2/Mode-3** — V1 cible Mode-1 uniquement
(centroids déterministes, déterministe = re-calibrable). Mode-2/3 active
learning = M25+.

❌ **Custom labeling prompts par tenant** — V1 force usage des templates
shipped (eval-gated, comme M23). Custom prompts annexe O-5.

❌ **Cross-catalog gold sharing** — chaque catalog a son gold dataset
isolé. Sharing intra-tenant cross-catalog annexe O-6.

❌ **Continuous deployment auto-promote** — V1 a `auto_promotion` mais
guardé par `low_stakes` ; le full continuous deployment (zero human
oversight on any catalog) est explicitement interdit jusqu'au feedback
M24 GA + 90 jours pilots.

### 2.3 Dépendances dures (ne peuvent être OFF)

| Dépendance | Version min | Raison |
|---|---|---|
| **M14** (multi-tenant catalog tables) | always-on | tables `catalog`, `tenant`, anchor storage |
| **M14-bis** (audit chain HMAC) | always-on | `llm_call_audit` (uncertainty sampler la lit), chaîne HMAC |
| **M16** (mcp-server) | always-on | les 4 tools sont MCP tools |
| **M18** (bundle deploy tools) | always-on | promotion réutilise le flow M18 |
| **M19** (BYOK + PII redactor) | always-on | PII redaction obligatoire avant LLM labeling |
| **M21** (LLMRouter + cost) | **ON requis** | startup validation refuse le boot si OFF + M24 ON |
| **M23** (curation gold_dataset table) | **ON requis** | M24 ajoute des rows à `gold_dataset` (table M23) ; étend `curation_review` pattern |
| **R-100** (Sigstore signing pipeline) | v1.0.0 | sign new bundle versions emitted by retune |

### 2.4 Dépendances soft

| Dépendance | Activation requise | Raison |
|---|---|---|
| **M22** (Mode-3 multi-turn) | OFF acceptable | M24 Mode-1 only en V1 |
| **M20** (BYO LLM agent SDK) | OFF acceptable | M24 utilise M21 LLMRouter, pas le SDK |
| **GLOSS Sprint A/B/C** | v1.0.0 | extensions CLI + UI réutilisent les conventions |

---

## 3. AS-IS post-M23 + R-100

### 3.1 Capabilities en place (post-M23 livraison)

| # | Capability | Module / artefact | Disponible pour M24 ? |
|---|---|---|:---:|
| 1 | Anchor classification Mode-1 (compiled, deterministic centroids) | `crates/efe-server/src/mode1.rs` | ✅ source of truth + cible re-calibration |
| 2 | Anchor centroid storage | `taxonomy_version.anchors_json` | ✅ M24 read + write |
| 3 | Audit chain HMAC | `taxonomy_change_audit`, `llm_call_audit` | ✅ M24 ajoute `learning_audit` chaînée |
| 4 | LLMRouter abstraction | `crates/efe-llm-router/` | ✅ M24 utilise pour labeling |
| 5 | Cost governance per-tenant | `tenant_budget` + `llm_call_audit.cost_cents` | ✅ M24 ajoute catégorie `active_learning` |
| 6 | PII redactor | `crates/efe-pii/` | ✅ obligatoire avant LLM labeling |
| 7 | Gold dataset tables | `gold_dataset` + `gold_example` (M23) | ✅ M24 append-only insertions |
| 8 | Curation review queue | `curation_review` (M23) | ✅ M24 étend le pattern |
| 9 | Canary engine | `efe-curation::rollout` (M23) | ✅ M24 réutilise pour promotion |
| 10 | Bundle deploy flow | M18 deploy tools | ✅ M24 trigger `efe_promote_bundle` |
| 11 | Sigstore signing | R-100 release pipeline | ✅ M24 sign new retuned bundles |
| 12 | Drift detection | `efe-curation::drift` (M23) | ✅ M24 réutilise pour post-promote watch |
| 13 | Holdout dataset | ❌ **manquant** | ⚠ M24 introduit |
| 14 | Bundle retune pipeline | ❌ **manquant** | ⚠ M24 introduit |
| 15 | Auto-promote logic | ❌ **manquant** | ⚠ M24 introduit (sous flags stricts) |
| 16 | GDPR Art. 17 cascade | ❌ partial (M23 tombstone) | ⚠ M24 ajoute bundle-level cascade |

### 3.2 Gap analysis — pourquoi M24 maintenant

| # | Manque post-M23 | Impact opérationnel actuel | M24 fix |
|---|---|---|:---:|
| 1 | Pas de boucle automatique production → gold | Gold dataset construit manuellement par opérateur (slow + biased) | ✅ § 5 |
| 2 | Pas d'uncertainty sampling | Tenants ne savent pas quels samples méritent labeling | ✅ § 7 |
| 3 | Pas de bundle retune scheduled | Bundles deviennent stale ; drift accumulé non corrigé | ✅ § 12 |
| 4 | Pas de holdout statistique | Eval ad-hoc, pas comparable cross-versions | ✅ § 14 |
| 5 | Pas d'auto-promote (avec safeguards) | Toute promotion requiert opérateur (slow ; ne scale pas vs nb tenants) | ✅ § 16 (gated) |
| 6 | Pas de GDPR Art. 17 cascade bundle-level | Tombstone gold_example mais centroid bundle reste contaminé | ✅ § 17 |
| 7 | Pas de cost governance pour active learning | Si tenant active labeling LLM custom, pas de hard-cap | ✅ § 25 |
| 8 | Pas de compliance lineage retune → bundle | Difficile de prouver à un auditeur quel gold a généré quel bundle | ✅ § 26 |

### 3.3 Workflows actuels (sans M24) — coût opérationnel

| Workflow | Temps observé pilots (3 tenants) | Bottleneck |
|---|---|---|
| « Mon bundle se dégrade après 3 mois en prod, comment je le mets à jour ? » | 2-4 semaines | aucune mécanique automatique ; opérateur revoit manuellement le gold + re-deploy bundle |
| « Quelle est la qualité prédite de mon bundle si je rajoute 500 examples ? » | 1-2 jours | A/B manuel, pas de holdout stable |
| « Une donnée GDPR Art. 17 a été ingérée — comment je la sors aussi du bundle ? » | inconnu (pas géré) | les centroids restent contaminés ; risque réputationnel + amende potentielle |
| « Mon LLM oracle est down — quel impact sur ma boucle ? » | inconnu | pas de circuit-breaker, pas de fallback |

**Cible M24** : automatiser à < 10 min de wallclock pour un opérateur
moyen (vérifier la queue + approve quelques labels + valider promote
quand notifié), avec fallback gracieux si LLM/budget down.

### 3.4 Pourquoi maintenant (post-R-100 GA + M22 + M23)

- **R-100 GA** = release pipeline mature + Sigstore signing.
- **M21 + M22** = LLMRouter stable, cost governance opérationnelle.
- **M23** = gold_dataset tables en place + canary engine + Art. 14 hooks.
- **80+ tenants pilots** = corpus production suffisant pour valider
  l'efficacité de l'uncertainty sampling.
- **Compliance posture** : DPIA M23 amendée pour M24 (LLM oracle = même
  sub-processor que M21, pas de nouveau).

---

## 4. TO-BE — vision M24

### 4.1 Diagramme système end-to-end

```
                              ┌──────────────────────────────────────┐
                              │  Production traffic                  │
                              │  (Mode-1 classify calls)             │
                              │                                      │
                              │  efe-server : mode1::classify()      │
                              │  → llm_call_audit row (confidence)   │
                              └────────────┬─────────────────────────┘
                                           │
                                           ▼
                              ┌──────────────────────────────────────┐
                              │  gold_label_enqueuer (worker, async) │
                              │                                      │
                              │  watches : llm_call_audit             │
                              │  predicate : confidence < threshold  │
                              │              AND not duplicate        │
                              │              AND PII redactable       │
                              │  emits : gold_label_queue rows        │
                              └────────────┬─────────────────────────┘
                                           │
                                           ▼
            ┌──────────────────────────────────────────────────────────┐
            │  gold_label_queue (Postgres) ←  Redis stream (mirror)    │
            │  ───────────────────────────────────────────────────────  │
            │  status : ENQUEUED → LABELING → PROPOSED → CONFIRMED      │
            │           → IN_GOLD → IN_HOLDOUT (subset)                 │
            │  back-pressure : if size > N → stop enqueueing            │
            └────────────┬─────────────────────────────────────────────┘
                         │
                         ▼ (every 5 min batch)
            ┌──────────────────────────────────────────────────────────┐
            │  gold_label_llm_worker (worker, async)                   │
            │  ───────────────────────────────────────────────────────  │
            │  - drains N rows                                          │
            │  - PII redact                                             │
            │  - LLM call via M21 router (budget=active_learning)       │
            │  - parse + validate output schema                         │
            │  - INSERT gold_label_proposal                             │
            │  - APPEND learning_audit (HMAC chain)                     │
            └────────────┬─────────────────────────────────────────────┘
                         │
              ┌──────────┴──────────────┐
              │                         │
              ▼ (auto_confirm=true)     ▼ (default : require human)
   ┌───────────────────────┐  ┌──────────────────────────────────────┐
   │ auto_confirm_gate      │  │ Human review queue                   │
   │ - LLM confidence       │  │ (extends M23 curation_review pattern)│
   │   > 0.9 ?              │  │ - portal-admin UI                    │
   │ - tenant DPO consent ? │  │ - assigned reviewer                  │
   │ - catalog low_stakes ? │  │ - SLA notifications                  │
   │ ALL true → auto-confirm│  │ - signed OIDC ID token               │
   └────────────┬───────────┘  └────────────┬─────────────────────────┘
                │                            │
                └──────────────┬─────────────┘
                               ▼
                    ┌─────────────────────────┐
                    │  gold_dataset (M23)     │ ─────► holdout_split (10%)
                    │  + gold_example         │
                    └────────────┬────────────┘
                                 │
                                 ▼ (if N gold_examples > threshold)
                    ┌──────────────────────────────────────────────────┐
                    │  bundle_retune_scheduler (cron, weekly default)  │
                    │  ─────────────────────────────────────────────── │
                    │  1. Fetch ACTIVE bundle anchors                  │
                    │  2. Load gold_dataset (excluding holdout)        │
                    │  3. Per anchor : recompute centroid via mean of  │
                    │     gold examples assigned to that class         │
                    │  4. Eval on holdout : new vs current accuracy    │
                    │  5. If new ≥ current + 1pp → flag for promotion  │
                    │  6. Cosign sign new bundle artifact              │
                    │  7. INSERT bundle_retune_run                     │
                    │  8. APPEND learning_audit                        │
                    └──────────────────┬───────────────────────────────┘
                                       │
                                       ▼
                    ┌──────────────────────────────────────────────────┐
                    │  promotion_gate                                   │
                    │  ─────────────────────────────────────────────── │
                    │  IF tenant.auto_promotion=true                   │
                    │     AND catalog.stakes_tier=low                  │
                    │     AND human_review_required=false              │
                    │     AND DPO consent=signed                       │
                    │   → auto-promote (still goes through canary 24h) │
                    │  ELSE                                            │
                    │   → flag for operator approval (portal-admin)    │
                    └──────────────────┬───────────────────────────────┘
                                       │
                                       ▼
                    ┌──────────────────────────────────────────────────┐
                    │  Canary (M23 engine reused)                      │
                    │  - 5% traffic shadow eval (24h default)          │
                    │  - if pass → promote ACTIVE                      │
                    │  - if fail → auto-rollback                       │
                    └──────────────────┬───────────────────────────────┘
                                       │
                                       ▼
                              ┌──────────────────┐
                              │  ACTIVE bundle   │ ←──── post-promote
                              │  v3.X.Y          │       drift watch
                              └──────────────────┘       (if drop > 2pp
                                                          → freeze + alert)
```

### 4.2 Cycle de vie d'un sample (golden path, T-axis)

```
T+0     Production classify : confidence = 0.65
        → llm_call_audit row inserted

T+0+    gold_label_enqueuer detects (next 30s tick)
        → INSERT gold_label_queue (status=ENQUEUED)
        → emit Redis stream message (worker subscriber)

T+5min  gold_label_llm_worker batch drain :
        - dequeue N rows
        - PII redact (efe-pii)
        - LLM call : "given sentence X and current taxonomy, what class?"
        - response : { class_id: "shipping-delay", confidence: 0.92 }
        - INSERT gold_label_proposal (status=LLM_LABELED)
        - UPDATE gold_label_queue.status = LABELING
        - APPEND learning_audit (event=llm_label_proposed)

T+5min+ auto_confirm_gate :
        IF confidence > 0.9 AND tenant.auto_confirm_enabled
           AND catalog.stakes_tier IN (low, medium)
           AND tenant.dpo_consent_active
        → INSERT gold_example (skip human review)
        → UPDATE gold_label_queue.status = CONFIRMED
        → APPEND learning_audit (event=auto_confirmed)
        ELSE
        → status remains LLM_LABELED awaiting human

T+1h    (if human review required) : reviewer logs in,
        sees proposal in /learning/proposals
        → click Confirm OR Edit-and-Confirm OR Reject
        → INSERT gold_label_review (signed OIDC)
        → INSERT gold_example (if confirmed)
        → APPEND learning_audit

T+24h   gold_label_review_notifier : if proposal still pending
        AND age > tenant.review_sla_hours → notify (Slack / email / PagerDuty)

T+...   Sample eventually accumulates in gold_dataset
        → 10% chance to be selected for holdout (deterministic
          hash-based, see § 14)

T+1week bundle_retune_scheduler triggers (per-catalog)
        → if N new gold_examples > tenant.gold_label_threshold
        → recompute centroids → eval on holdout
        → if accuracy delta ≥ +1pp → flag for promotion
        → operator (or auto-gate) approves
        → canary 24h
        → ACTIVE
```

### 4.3 Les 9 invariants de design M24

1. **Sample uniqueness** — un sample (sentence + tenant + catalog) ne
   peut être enqueué qu'une fois (via content-hash dedup). Empêche
   la pollution du gold dataset par retries de production.
2. **Tenant scope strict** — RLS sur toutes les nouvelles tables ;
   cross-tenant gold sharing structurellement impossible.
3. **PII redaction obligatoire** — toute donnée tenant qui passe au
   LLM transite par `efe-pii::redact` (M19) ; bypass = compile error.
4. **Audit chain mandatory** — chaque transition d'état + chaque LLM
   call + chaque human review + chaque retune + chaque promotion
   inscrit dans `learning_audit` (HMAC chained).
5. **Holdout immutability** — une fois un sample assigné au holdout,
   il ne peut JAMAIS être utilisé pour training (centroid update).
   Holdout split deterministic hash-based (cf. § 14).
6. **Promotion gate non-bypassable** — `accuracy_delta_pp ≥ +1.0`
   est un invariant compile-time enforced ; pas de skip via flag.
7. **GDPR Art. 17 cascade** — tombstone d'un gold_example déclenche
   automatiquement un re-tune du bundle qui l'avait indexé (cf. § 17).
8. **Cost ceiling** — chaque LLM labeling vérifie le budget tenant
   (catégorie `active_learning`) ; si dépassé, worker pause + alert.
9. **Reversibility** — chaque promotion est rollback-able dans la
   fenêtre de retention bundle (default 90 j) ; rollback ne
   contamine pas le gold dataset (rollback != tombstone des labels).

---

## 5. Closed-loop pipeline end-to-end

### 5.1 Les 4 phases du closed loop

```
┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 1 — OBSERVE                                                   │
│  ────────────────                                                    │
│  Production classify calls flow through Mode-1 inference :           │
│   - sentence → embedder → cosine similarity vs anchor centroids      │
│   - top class + confidence emitted                                    │
│   - row inserted in llm_call_audit (confidence, verdict_class_id, …) │
│                                                                      │
│  Frequency : real-time, 100 % of traffic                             │
│  Storage  : llm_call_audit (M19, append-only with retention)         │
│  Cost     : zero (already happening)                                 │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 2 — SELECT                                                    │
│  ────────────────                                                    │
│  Uncertainty sampler (gold_label_enqueuer) scans recent rows :       │
│   - confidence < tenant.uncertainty_threshold (default 0.7)          │
│   - AND not in gold_dataset already (content-hash dedup)             │
│   - AND PII redactable (rule check : no obvious unredactable)        │
│   - AND tenant.al_enabled = true                                      │
│   - AND catalog.al_enabled = true                                     │
│   - AND queue size < tenant.queue_max_size (back-pressure)           │
│  → INSERT gold_label_queue                                           │
│                                                                      │
│  Frequency : 30s tick (configurable)                                  │
│  Storage  : gold_label_queue (new table)                             │
│  Cost     : ~0 (Postgres scan + insert)                              │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 3 — LABEL                                                     │
│  ──────────────                                                      │
│  Two-step labeling :                                                 │
│   3a. LLM oracle (gold_label_llm_worker) : drains N rows / 5 min,    │
│       calls LLM via M21 router with template "label_classify_v1"     │
│       → produces (class_id, confidence) → gold_label_proposal        │
│   3b. Human review (extends M23 review queue) : if not auto-confirm, │
│       reviewer Confirm/Edit/Reject → gold_label_review               │
│   On confirm : INSERT gold_example (M23 table)                       │
│                                                                      │
│  Frequency : 5 min batch (configurable per-tenant)                    │
│  Storage  : gold_label_proposal, gold_label_review, gold_example     │
│  Cost     : LLM tokens (budget=active_learning, ~3¢/sample default)  │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 4 — RETUNE & PROMOTE                                          │
│  ───────────────────────────                                         │
│  Scheduled (weekly default) per active catalog :                     │
│   4a. Bundle retune : recompute centroids from gold_dataset minus    │
│       holdout (10% reserved). New bundle artifact created.           │
│   4b. Eval : new bundle vs current on holdout — accuracy delta       │
│   4c. If delta ≥ +1pp : flag for promotion. Else : keep current,     │
│       persist eval result for trending.                              │
│   4d. Promotion : auto (if all gates pass) or operator-approved.    │
│   4e. Canary (M23 engine, 5% traffic, 24h) : if pass → ACTIVE.       │
│   4f. Post-promote drift watch : if accuracy drop > 2pp → freeze.    │
│                                                                      │
│  Frequency : weekly per catalog (configurable, gated by N gold > T)  │
│  Storage  : bundle_retune_run, holdout_split, taxonomy_version       │
│  Cost     : compute (centroid recompute + eval) — ~few minutes CPU   │
└──────────────────────────────────────────────────────────────────────┘
```

### 5.2 Fan-in / fan-out par tenant (capacité)

| Component | Fan-in (per tenant) | Fan-out (per tenant) |
|---|---|---|
| `llm_call_audit` | ~10K-100K rows/day (production traffic) | — |
| Uncertainty sampler | reads above | ~100-1K rows/day to queue (assume 1-10% low conf) |
| gold_label_queue | ~100-1K rows/day | drained by LLM worker |
| LLM labeling worker | reads queue | ~100-1K LLM calls/day (cost : ~$3-30/day @ $0.03/call) |
| gold_label_review | reviewer activity | ~50-500 reviews/day (subset, after auto-confirm) |
| gold_dataset | ~100-1K new examples/day | accumulates |
| holdout_split | 10% of above | ~10-100 new holdout samples/day |
| bundle_retune | weekly | ~1 retune/catalog/week (per active catalog) |
| promotion | gated | ~0-1 promotion/catalog/week |

**Sanity check** : at 80 tenants × 5 catalogs avg × 1 retune/week ×
~$3/week per retune (compute only) = ~$1200/month total compute cost
across the fleet. LLM labeling cost dominates : ~$0.50-$5/tenant/day
(depending on traffic) × 80 tenants × 30 days = ~$1200-$12000/month.
Cf. § 25 + Annexe B.

### 5.3 Comparaison vs concurrents

| Capability | EFE M24 | OpenAI Moderation | AWS Comprehend Custom | Azure AI Language |
|---|:---:|:---:|:---:|:---:|
| Active learning loop | ✅ | ❌ | ⚠ partial (manual retrain) | ⚠ partial |
| Audit chain HMAC | ✅ | ❌ | ❌ | ❌ |
| Auto-promote with safeguards | ✅ (gated) | N/A | ⚠ no compliance gate | ⚠ no compliance gate |
| GDPR Art. 17 cascade to model | ✅ | ❌ | ❌ | ❌ |
| Tenant-isolated gold dataset | ✅ Day 1 | N/A | ⚠ requires separate accounts | ⚠ requires separate workspaces |
| LLM oracle + human-in-the-loop | ✅ | ❌ | ❌ | ❌ |

---

## 6. Architectural Decision Records

### ADR-M24-001 : Active learning loop = batch + queue, NOT real-time

**Décision** : pipeline async batch-based avec Postgres queue + Redis
stream pour distribution.

**Alternatives** :
- A. Real-time : chaque classify low-confidence → LLM call inline ?
- B. Batch (5 min interval) avec queue durable.
- C. Streaming (Kafka, Pulsar).

**Choix : B**.

**Justifications** :
1. **Cost optimization** : batching permet LLM provider batch APIs
   (Anthropic batch, OpenAI batch) avec 50% discount.
2. **Latency tolerance** : la qualité de classify n'est pas affectée
   par un labeling delay de 5 min.
3. **Back-pressure native** : queue size limit = circuit breaker
   gratuit.
4. **Op simplicity** : Postgres + Redis stream sont déjà dans le stack
   M19. Kafka est over-engineering pour 100-1K events/day/tenant.

**Coût** : latence p99 du queue ~5 min (vs ~200 ms real-time). Acceptable.

### ADR-M24-002 : Embedder re-calibration via centroid update (no fine-tune)

**Décision** : nouveau bundle = centroids recalculés à partir du gold
dataset (mean of embeddings per anchor). PAS de fine-tune neural.

**Alternatives** :
- A. Recalibration centroids only (deterministic, no model change).
- B. LoRA / adapter fine-tune (parameter-efficient).
- C. Full fine-tune.

**Choix : A**.

**Justifications** :
1. **Determinism** : les centroids Mode-1 sont calculables à partir des
   gold examples. Reproducible bit-for-bit.
2. **No GPU required** : recalibration = matrix mean compute, runs on CPU
   in seconds.
3. **No model versioning hell** : l'embedder model reste stable ; seuls
   les centroids changent. Bundle versions = `vX.Y.Z+1`.
4. **Audit-friendly** : reproductible — auditeur peut recalculer le
   centroid à partir du gold dataset signé.

**Coût** : pas d'auto-amélioration de l'embedder lui-même (ses biais
restent). Mitigation : embedder upgrade = M25+ scope (full retrain
or model swap).

### ADR-M24-003 : Default state OFF + 4-level activation

**Décision** : OFF par défaut au server level + tenant + catalog + per-call.

**Justifications** : M24 est un système qui mute la qualité production
sans intervention humaine (en mode auto-promote). Le default OFF est
non-négociable. Tenants doivent opt-in 3 fois (server flag, tenant
flag, catalog flag) avant d'activer le pipeline.

### ADR-M24-004 : Auto-confirm only if 3 conditions ALL true

**Décision** : auto-confirm gold label si :
1. LLM confidence > 0.9 (configurable)
2. Tenant DPO consent active (signed addendum)
3. Catalog stakes_tier = low

**Alternatives** : auto-confirm sur 1 ou 2 conditions seulement.

**Choix : 3 conditions ALL true**.

**Justifications** : conservateur par design. Mieux vaut quelques
faux humains-in-the-loop que un auto-confirm illégitime sur un
catalog high-stakes.

### ADR-M24-005 : Promotion gate require accuracy delta ≥ +1pp

**Décision** : un nouveau bundle ne peut être promu que si son
accuracy on holdout est ≥ accuracy_current + 1.0 percentage point.

**Alternatives** : threshold = 0pp (no regression), 0.5pp, 1pp, 2pp.

**Choix : 1pp**.

**Justifications** :
1. Statistically significant : avec un holdout de 1000 examples,
   1pp = ~10 examples differential. Au-dessus du noise.
2. Conservative : empêche les "accuracy creep" cosmétiques.
3. Tenant-tunable : un tenant high-stakes peut bumper à 2pp.

### ADR-M24-006 : Holdout = frozen 10% subset, deterministic hash-split

**Décision** : holdout = 10% des gold_examples, sélection via
SHA-256 of (tenant_id || catalog_id || gold_example.id) modulo 10.

**Alternatives** :
- A. Random split (pas reproducible).
- B. Hash-based deterministic.
- C. Time-based (most recent 10%).

**Choix : B**.

**Justifications** :
1. **Reproducible** : tout auditeur peut recalculer la même split.
2. **Drift-resistant** : pas de bias temporel (vs option C).
3. **Per-tenant per-catalog deterministic** : garantit l'isolation.

### ADR-M24-007 : Bundle retune scheduled weekly per catalog

**Décision** : default = weekly cron par active catalog.

**Alternatives** : daily, weekly, monthly, on-demand only.

**Choix : weekly**.

**Justifications** :
1. **Daily** : trop fréquent, le gold dataset n'a pas accumulé assez de
   nouveaux samples pour bouger les centroids significativement.
2. **Monthly** : trop lent, drift accumulation.
3. **Weekly** : equilibre entre fraîcheur et bruit. Aligné avec les
   sprints opérationnels typiques.

### ADR-M24-008 : Postgres + Redis stream, NOT Kafka

**Décision** : queue durable = Postgres ; distribution worker = Redis stream.

**Justifications** :
1. **Pas de nouvelle dépendance ops** : Postgres + Redis déjà dans M19/M21.
2. **Volume modeste** : 100-1K events/tenant/day = trivial pour
   Postgres + Redis stream.
3. **Cost** : Kafka cluster = $500/mois minimum, overkill.

### ADR-M24-009 : LLM labeling via M21 LLMRouter with budget category

**Décision** : labeling utilise LLMRouter (M21) avec
`budget_category="active_learning"`.

**Justifications** :
1. **Reuse cost governance** : pas de nouveau système de budget.
2. **Provider abstraction** : tenant peut choisir Anthropic / OpenAI /
   Vertex / vLLM via flags M21.
3. **Audit trail** : chaque labeling LLM call inscrit dans
   `llm_call_audit` automatiquement.

### ADR-M24-010 : GDPR Art. 17 = tombstone + cascade rebuild bundle

**Décision** : suppression GDPR ne hard-delete pas le row gold_example
(préservation chaîne HMAC). Au lieu de ça :
1. Tombstone : row marked `redacted=true`, content masked.
2. Cascade : prochain retune exclut les rows tombstoned.
3. Forced rebuild : si urgent (régulateur), trigger re-tune immédiat.

**Justifications** :
1. **Audit chain integrity** : hard-delete casse la chaîne HMAC.
2. **Bundle cascade** : sans rebuild, le centroid contient encore
   les vecteurs de la donnée à effacer (data leakage indirect).
3. **Compliance** : RGPD Art. 17 paragraphe 1.b — droit à l'oubli
   exigeant suppression "sans retard injustifié". Le forced rebuild
   couvre les cas urgents.

### ADR-M24-011 : New bundle versioning = patch bump

**Décision** : retune produit `vX.Y.Z+1` (patch bump).

**Justifications** : aligné avec semver — patch = no breaking change
(taxonomy unchanged, just centroid refinement).

### ADR-M24-012 : Two-stage shadow eval (offline + canary)

**Décision** : eval = (1) offline holdout + (2) canary 5% production.

**Justifications** :
1. **Offline holdout** = rapide, deterministic, mais peut diverger de
   la distribution prod réelle.
2. **Canary** = vraie distribution mais coûteux en latency + risk.
3. **Combiné** : offline filter rapide → canary confirm. Optimal.

### ADR-M24-013 : Auto-promote OFF par défaut même si toutes les autres conditions true

**Décision** : `tenant_feature_m24.auto_promotion=false` par défaut. Doit
être opt-in explicite.

**Justifications** : "default safe" — un tenant ne peut pas accidentellement
activer auto-promotion via un toggle accidentel. Doit signer un addendum
DPA explicite.

### ADR-M24-014 : Cost cap on LLM labeling = circuit breaker

**Décision** : si tenant.active_learning_budget_cents_per_day est
épuisé en mid-batch, le worker s'arrête immédiatement (pas de
labeling partiel sans audit), reprend le lendemain.

**Justifications** : éviter les surprises de facture + limite légale
(certains tenants ont des hard caps régulatoires).

### ADR-M24-015 : Drift watch post-promote = freeze if accuracy drop > 2pp

**Décision** : après promotion, watch quotidien sur 7 jours. Si
accuracy on production samples drops > 2pp depuis le baseline pre-
promote, freeze + alert + auto-rollback proposé (pas auto-exécuté).

**Justifications** : promotion peut sembler bonne sur holdout mais
mal généraliser ; drift watch est la safety net.

---

## 7. Uncertainty sampler

### 7.1 Mission

Sélectionner les samples production qui méritent un labeling LLM,
sans inonder la queue. L'uncertainty sampler est l'amorce de toute la
boucle : son taux de précision (combien de samples enqueués sont
réellement utiles) détermine le coût LLM total.

### 7.2 Heuristique V1 : threshold-based

```rust
// crates/efe-active-learning/src/sampler.rs

pub struct SamplerConfig {
    pub uncertainty_threshold: f32,   // default 0.70
    pub queue_max_size: u32,           // default 10000 per tenant
    pub dedup_window_days: u32,        // default 30 (don't re-label same content within)
    pub batch_scan_limit: u32,         // default 1000 rows per scan tick
}

pub async fn select_samples_for_labeling(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, cfg: &SamplerConfig,
) -> CurationResult<u64> {
    // 1. Find recent low-confidence classifies not yet enqueued and not
    //    in gold (content-hash dedup).
    let rows = sqlx::query_as::<_, CandidateSample>(
        r#"
        SELECT
            llc.id AS llm_call_id,
            llc.tenant_id,
            llc.catalog_id,
            llc.sentence_redacted,
            llc.verdict_class_id AS proposed_class_id,
            llc.confidence,
            encode(digest(llc.sentence_redacted, 'sha256'), 'hex') AS content_hash
        FROM llm_call_audit llc
        WHERE llc.tenant_id = $1
          AND llc.catalog_id = $2
          AND llc.confidence < $3
          AND llc.created_at > NOW() - INTERVAL '7 days'
          AND NOT EXISTS (
              SELECT 1 FROM gold_label_queue glq
              WHERE glq.tenant_id = $1
                AND glq.content_hash = encode(digest(llc.sentence_redacted, 'sha256'), 'hex')
                AND glq.created_at > NOW() - $4
          )
          AND NOT EXISTS (
              SELECT 1 FROM gold_example ge
              JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
              WHERE gd.tenant_id = $1
                AND gd.catalog_id = $2
                AND encode(digest(ge.text, 'sha256'), 'hex') =
                    encode(digest(llc.sentence_redacted, 'sha256'), 'hex')
          )
        ORDER BY llc.created_at DESC
        LIMIT $5
        "#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .bind(cfg.uncertainty_threshold)
    .bind(format!("{} days", cfg.dedup_window_days))
    .bind(cfg.batch_scan_limit as i64)
    .fetch_all(pg).await?;

    if rows.is_empty() { return Ok(0); }

    // 2. Check queue back-pressure.
    let queue_size: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM gold_label_queue WHERE tenant_id = $1 AND status IN ('ENQUEUED', 'LABELING', 'LLM_LABELED')"
    ).bind(&tenant.0).fetch_one(pg).await?;
    let avail = (cfg.queue_max_size as i64 - queue_size).max(0) as usize;
    if avail == 0 {
        crate::metrics::AL_QUEUE_BACK_PRESSURE_TOTAL
            .with_label_values(&[&tenant.0]).inc();
        return Ok(0);
    }
    let to_insert: Vec<_> = rows.into_iter().take(avail).collect();

    // 3. Bulk insert into gold_label_queue.
    let mut tx = pg.begin().await?;
    for c in &to_insert {
        sqlx::query(
            r#"INSERT INTO gold_label_queue
               (id, tenant_id, catalog_id, content_hash, sentence_redacted,
                proposed_class_id, observed_confidence, status, source_llm_call_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'ENQUEUED', $8)
               ON CONFLICT (tenant_id, content_hash) DO NOTHING"#,
        )
        .bind(uuid::Uuid::new_v4())
        .bind(&c.tenant_id).bind(&c.catalog_id).bind(&c.content_hash)
        .bind(&c.sentence_redacted).bind(&c.proposed_class_id).bind(c.confidence)
        .bind(c.llm_call_id)
        .execute(&mut *tx).await?;
    }
    tx.commit().await?;

    crate::metrics::AL_SAMPLES_ENQUEUED_TOTAL
        .with_label_values(&[&tenant.0, &catalog.0])
        .inc_by(to_insert.len() as u64);

    Ok(to_insert.len() as u64)
}
```

### 7.3 Scheduled invocation

`gold_label_enqueuer` runs as a Tokio scheduled task every 30 seconds
(configurable). Per-tenant per-catalog scan, RLS-scoped.

### 7.4 Pin tests sampler

- `test_sampler_threshold_respected` — `confidence > threshold` not enqueued.
- `test_sampler_dedup_content_hash` — same sentence not enqueued twice.
- `test_sampler_dedup_existing_gold` — sample already in gold not enqueued.
- `test_sampler_back_pressure` — queue at max size → no insert.
- `test_sampler_per_tenant_isolation` — tenant A scan does not see tenant B rows.
- `test_sampler_disabled_catalog` — catalog.al_enabled=false → no insert.
- `test_sampler_metrics_emitted` — counter incremented.
- `test_sampler_handles_pii_unredactable` — rows flagged unredactable skipped.

### 7.5 Future heuristics (M25+)

- BALD (Bayesian Active Learning by Disagreement)
- Query-by-committee (multiple embedder models vote)
- Density-weighted sampling
- Out-of-distribution detection

(Documented as O-2 in § 33.)

---

## 8. Gold label queue

### 8.1 Architecture : Postgres durable + Redis stream mirror

```
┌──────────────────────────────────────────────────────────────────────┐
│  Postgres : gold_label_queue (durable, RLS-scoped)                   │
│  ─────────────────────────────────────────────────────────────────── │
│  Source of truth. Always present. Survives restarts.                 │
│  Worker reads via SELECT ... FOR UPDATE SKIP LOCKED.                 │
└──────────────────────┬───────────────────────────────────────────────┘
                       │
                       │ trigger fn_notify_redis_on_insert
                       ▼
┌──────────────────────────────────────────────────────────────────────┐
│  Redis stream : efe:al:queue:<tenant_id>                             │
│  ─────────────────────────────────────────────────────────────────── │
│  Mirror for low-latency worker wake-up (XREAD blocks).               │
│  Disposable : if Redis loses messages, Postgres remains source.      │
│  TTL : 7 days (Redis stream maxlen).                                 │
└──────────────────────────────────────────────────────────────────────┘
```

### 8.2 Schema (extrait — voir § 18 pour migration complète)

```sql
CREATE TABLE gold_label_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[a-f0-9]{64}$'),
    sentence_redacted TEXT NOT NULL,
    proposed_class_id TEXT,
    observed_confidence DOUBLE PRECISION,
    status gold_label_queue_status NOT NULL DEFAULT 'ENQUEUED',
    source_llm_call_id UUID,
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    labeled_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    failure_count INT NOT NULL DEFAULT 0,
    last_error TEXT,
    UNIQUE(tenant_id, content_hash)
);

CREATE TYPE gold_label_queue_status AS ENUM (
    'ENQUEUED',
    'LABELING',           -- worker picked up
    'LLM_LABELED',        -- LLM produced proposal
    'AWAITING_REVIEW',    -- human review needed
    'CONFIRMED',          -- inserted into gold_example
    'REJECTED',           -- reviewer rejected, not added
    'FAILED',             -- LLM/PII error, retry exhausted
    'TOMBSTONED'          -- GDPR Art. 17
);
```

### 8.3 SELECT FOR UPDATE SKIP LOCKED — N workers safe

```rust
// crates/efe-active-learning/src/queue.rs

pub async fn dequeue_batch(
    tx: &mut Transaction<'_, Postgres>, tenant: &TenantId, batch_size: u32,
) -> CurationResult<Vec<QueueRow>> {
    let rows = sqlx::query_as::<_, QueueRow>(
        r#"
        SELECT * FROM gold_label_queue
        WHERE tenant_id = $1 AND status = 'ENQUEUED'
        ORDER BY enqueued_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(&tenant.0).bind(batch_size as i64)
    .fetch_all(&mut **tx).await?;

    if !rows.is_empty() {
        sqlx::query(
            r#"UPDATE gold_label_queue
               SET status = 'LABELING'::gold_label_queue_status
               WHERE id = ANY($1)"#,
        )
        .bind(rows.iter().map(|r| r.id).collect::<Vec<_>>())
        .execute(&mut **tx).await?;
    }
    Ok(rows)
}
```

### 8.4 Back-pressure mechanics

```
┌─────────────────────────────────────────────────────────────────┐
│  Queue size monitoring (per-tenant)                             │
│  ──────────────────────────────────                             │
│  if queue_size >= queue_max_size :                              │
│    enqueuer pauses (skips new enqueues)                         │
│    metric AL_QUEUE_BACK_PRESSURE_TOTAL incremented              │
│    portal-admin shows "queue at capacity" badge                 │
│    Slack alert at 90% capacity                                  │
│                                                                 │
│  Operator response :                                            │
│    - increase queue_max_size (config)                           │
│    - check why labels not being drained (LLM down? budget?)     │
│    - manually trigger drain                                     │
└─────────────────────────────────────────────────────────────────┘
```

### 8.5 Pin tests queue

- `test_dequeue_for_update_skip_locked_concurrent` — 2 workers don't pick same row.
- `test_dequeue_status_transition_atomic` — ENQUEUED → LABELING in same tx.
- `test_dequeue_respects_batch_size`.
- `test_dequeue_per_tenant_isolation` — RLS enforced.
- `test_queue_back_pressure_emits_metric`.
- `test_unique_content_hash_within_tenant` — duplicate enqueue rejected.
- `test_redis_stream_mirror_on_insert` — trigger emits NOTIFY.
- `test_redis_disconnect_does_not_block_postgres` — fault tolerance.
- `test_failed_status_after_n_retries` — failure_count >= 3 → FAILED.
- `test_status_machine_invalid_transition_rejected` — DB trigger.

---

## 9. LLM labeling worker

### 9.1 Mission

Drain `gold_label_queue`, call LLM via M21 LLMRouter with the
`label_classify_v1` prompt template, parse + validate the response,
insert into `gold_label_proposal`.

### 9.2 Worker loop

```rust
// crates/efe-active-learning/src/llm_worker.rs

pub async fn llm_worker_loop(
    pg: Arc<PgPool>, llm: Arc<dyn LlmClient>, tenants: Arc<dyn TenantRegistry>,
    interval_secs: u64,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        for tenant in tenants.iter_active_with_al_enabled().await.unwrap_or_default() {
            if let Err(e) = drain_one_tenant(&pg, llm.as_ref(), &tenant).await {
                tracing::error!(?e, tenant=%tenant.id, "LLM worker drain failed");
                crate::metrics::AL_LLM_WORKER_ERRORS_TOTAL
                    .with_label_values(&[&tenant.id.0]).inc();
            }
        }
    }
}

async fn drain_one_tenant(
    pg: &PgPool, llm: &dyn LlmClient, tenant: &TenantContext,
) -> CurationResult<()> {
    // 1. Set RLS context for this tenant
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&tenant.id.0).execute(pg).await?;

    // 2. Pre-flight : check budget
    if !crate::cost::has_budget_for_labeling(pg, &tenant.id, BATCH_SIZE_DEFAULT).await? {
        crate::metrics::AL_LLM_WORKER_BUDGET_BLOCKED_TOTAL
            .with_label_values(&[&tenant.id.0]).inc();
        return Ok(());  // skip this tenant this tick
    }

    // 3. Begin tx, dequeue batch
    let mut tx = pg.begin().await?;
    let batch = crate::queue::dequeue_batch(&mut tx, &tenant.id, BATCH_SIZE_DEFAULT).await?;
    if batch.is_empty() { tx.rollback().await?; return Ok(()); }
    tx.commit().await?;

    // 4. Per-row : LLM call → parse → insert proposal
    for row in &batch {
        match label_one(pg, llm, &tenant.id, row).await {
            Ok(_) => {
                crate::metrics::AL_LABELS_PRODUCED_TOTAL
                    .with_label_values(&[&tenant.id.0, "success"]).inc();
            }
            Err(e) => {
                tracing::warn!(?e, queue_id=%row.id, "label_one failed");
                mark_failed(pg, row.id, &format!("{}", e)).await.ok();
                crate::metrics::AL_LABELS_PRODUCED_TOTAL
                    .with_label_values(&[&tenant.id.0, "fail"]).inc();
            }
        }
    }
    Ok(())
}
```

### 9.3 Per-row labeling

```rust
async fn label_one(
    pg: &PgPool, llm: &dyn LlmClient, tenant: &TenantId, row: &QueueRow,
) -> CurationResult<()> {
    // 1. Build prompt (PII already redacted in row.sentence_redacted)
    let prompt = render_label_prompt(tenant, row).await?;

    // 2. LLM call via M21 router (budget=active_learning)
    let resp = llm.call(
        tenant, &prompt.body, /* max_tokens */ 200,
        crate::service::ExecutionMode::InternalLlm,
    ).await?;

    // 3. Parse + validate
    let parsed = parse_label_output(&resp.content)?;
    if !is_valid_class_for_catalog(pg, tenant, &row.catalog_id, &parsed.class_id).await? {
        return Err(CurationError::LlmSchemaInvalid(
            format!("LLM proposed unknown class : '{}'", parsed.class_id)
        ));
    }

    // 4. INSERT gold_label_proposal + UPDATE queue.status
    let mut tx = pg.begin().await?;
    let proposal_id = uuid::Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO gold_label_proposal
           (id, queue_id, tenant_id, proposed_class_id, llm_confidence,
            llm_rationale, llm_call_audit_id, prompt_template_name,
            prompt_template_version, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())"#,
    )
    .bind(proposal_id).bind(row.id).bind(&tenant.0)
    .bind(&parsed.class_id).bind(parsed.confidence)
    .bind(&parsed.rationale).bind(resp.audit_id)
    .bind("label_classify_v1").bind(1_i32)
    .execute(&mut *tx).await?;

    sqlx::query(
        "UPDATE gold_label_queue SET status = 'LLM_LABELED', labeled_at = NOW() WHERE id = $1"
    ).bind(row.id).execute(&mut *tx).await?;

    crate::audit::append_learning_audit(
        &mut tx, tenant,
        Some(row.id), Some(proposal_id), None,
        "llm_label_proposed", "system", None,
        serde_json::json!({
            "queue_id": row.id, "proposed_class": parsed.class_id,
            "llm_confidence": parsed.confidence,
            "llm_call_audit_id": resp.audit_id,
        }),
    ).await?;
    tx.commit().await?;

    Ok(())
}
```

### 9.4 Output schema

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LabelClassifyOutput {
    pub class_id: String,
    pub confidence: f32,
    pub rationale: String,
}

pub fn parse_label_output(json: &str) -> CurationResult<LabelClassifyOutput> {
    let parsed: LabelClassifyOutput = serde_json::from_str(json)
        .map_err(|e| CurationError::LlmSchemaInvalid(format!("JSON parse : {}", e)))?;
    if !crate::is_valid_kebab_case(&parsed.class_id) {
        return Err(CurationError::LlmSchemaInvalid(
            format!("invalid class_id : '{}'", parsed.class_id)
        ));
    }
    if !(0.0..=1.0).contains(&parsed.confidence) {
        return Err(CurationError::LlmSchemaInvalid(
            format!("confidence out of range : {}", parsed.confidence)
        ));
    }
    if parsed.rationale.is_empty() || parsed.rationale.len() > 2048 {
        return Err(CurationError::LlmSchemaInvalid("rationale empty or too long".into()));
    }
    Ok(parsed)
}
```

### 9.5 Failure handling — exponential backoff

```rust
async fn mark_failed(pg: &PgPool, id: uuid::Uuid, error: &str) -> CurationResult<()> {
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = CASE
                          WHEN failure_count + 1 >= 3 THEN 'FAILED'::gold_label_queue_status
                          ELSE 'ENQUEUED'::gold_label_queue_status
                        END,
               failure_count = failure_count + 1,
               last_error = $2
           WHERE id = $1"#,
    )
    .bind(id).bind(error)
    .execute(pg).await?;
    Ok(())
}
```

3 retries max with exponential backoff (5min, 15min, 45min between
retries via retry-time header on the row). After 3 failures, status =
FAILED, alert emitted.

### 9.6 Pin tests LLM worker

- `test_llm_worker_drain_batch_size_respected`.
- `test_llm_worker_unknown_class_rejected` — LLM proposes class not in catalog → FAILED.
- `test_llm_worker_budget_blocked` — no budget → skip + metric.
- `test_llm_worker_invalid_json_retried` — parse error → ENQUEUED + failure_count++.
- `test_llm_worker_3_failures_marks_failed` — failure_count=3 → FAILED.
- `test_llm_worker_pii_redactor_called` — sentence passed redacted to LLM.
- `test_llm_worker_audit_chain_link` — learning_audit row appended.
- `test_llm_worker_per_tenant_rls` — worker only sees own tenant rows.
- `test_llm_worker_metrics_per_outcome` — success/fail counters.
- `test_llm_worker_concurrent_safe` — two workers don't double-process.
- `test_llm_worker_idempotency_on_retry` — same queue row + same prompt = same audit (ish).
- `test_llm_worker_handles_provider_5xx` — UNAVAILABLE → ENQUEUED retry.

---

## 10. Human review extension

### 10.1 Reuse + extension du pattern M23

M23 a livré `curation_review` table + portal-admin queue + Art. 14
enforcement. M24 réutilise ce pattern, ajoute juste :

1. **Nouvelle table `gold_label_review`** (mirroring curation_review)
   pour les reviews de gold labels.
2. **Notifier** : worker qui surveille les `gold_label_proposal`
   en status `AWAITING_REVIEW` et notifie via Slack/email/PagerDuty
   selon SLA tenant.
3. **Bulk-edit-and-confirm** : reviewer peut éditer le class_id avant
   confirm (LLM peut se tromper ; reviewer corrige).
4. **Feedback loop** : reviewer peut tag « LLM badly aligned » →
   surface dans le bench eval pour quality monitoring.

### 10.2 Schema gold_label_review

```sql
CREATE TABLE gold_label_review (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    proposal_id UUID NOT NULL REFERENCES gold_label_proposal(id) ON DELETE RESTRICT,
    reviewer_oidc_sub TEXT NOT NULL,
    reviewer_role TEXT NOT NULL,
    decision gold_label_review_decision NOT NULL,
    -- if decision = CONFIRM : final_class_id may differ from proposed
    -- (reviewer edited).
    final_class_id TEXT,
    rationale TEXT NOT NULL CHECK (length(rationale) >= 10),
    quality_tag TEXT,        -- 'good', 'edited', 'unclear', 'llm_drift', NULL
    oidc_id_token_hash TEXT NOT NULL,
    oidc_token_iat TIMESTAMPTZ NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(proposal_id, reviewer_oidc_sub)
);

CREATE TYPE gold_label_review_decision AS ENUM (
    'CONFIRM',           -- accept LLM proposal as-is
    'EDIT_AND_CONFIRM',  -- accept with class change
    'REJECT'             -- not added to gold_dataset
);
```

### 10.3 Notification SLA cascade

```
┌─────────────────────────────────────────────────────────────────┐
│  gold_label_review_notifier (worker, runs every 1h)             │
│  ───────────────────────────────────────────────────            │
│  For each gold_label_proposal in status AWAITING_REVIEW :       │
│    age = NOW() - proposal.created_at                            │
│    sla = tenant.review_sla_hours (default 48h)                  │
│    notify_after = tenant.notify_after_hours (default 8h)        │
│    if age > notify_after AND not yet notified :                 │
│      send notification (Slack OR email OR PagerDuty)            │
│      INSERT gold_label_review_notification row                  │
│    if age > sla * 0.9 :                                         │
│      escalate (CC tenant_admin)                                 │
│    if age > sla :                                               │
│      EMERGENCY (PagerDuty + auto-pause queue if configured)     │
└─────────────────────────────────────────────────────────────────┘
```

### 10.4 Pin tests review extension

- `test_review_decision_confirm_inserts_gold_example`.
- `test_review_decision_edit_uses_final_class_id`.
- `test_review_decision_reject_does_not_insert`.
- `test_review_oidc_freshness_check` — token > 10 min → reject.
- `test_review_4_eyes_optional_per_tenant`.
- `test_notifier_emits_at_notify_after_hours`.
- `test_notifier_escalates_at_sla_90pct`.
- `test_notifier_emergency_at_sla_breach`.

---

## 11. Auto-confirm gate (compliance-aware)

### 11.1 Les 3 conditions cumulatives

```rust
pub async fn should_auto_confirm(
    pg: &PgPool, tenant: &TenantContext, catalog: &CatalogInfo,
    proposal: &GoldLabelProposal,
) -> CurationResult<bool> {
    // 1. LLM confidence above threshold
    let confidence_ok = proposal.llm_confidence >= tenant.auto_confirm_threshold;
    // 2. Tenant has signed DPO consent for auto-confirm (M24-specific addendum)
    let dpo_consent_ok = check_dpo_consent_active(pg, &tenant.id).await?;
    // 3. Catalog stakes_tier = low (NOT medium, NOT high)
    let stakes_ok = catalog.stakes_tier == StakesTier::Low;

    Ok(confidence_ok && dpo_consent_ok && stakes_ok)
}
```

### 11.2 Compliance gate enforcement

The auto-confirm path is **compile-time gated** : the function
`auto_confirm_gold_label` is only callable from a code path that
explicitly checks `should_auto_confirm == true` first. There is no
`force_auto_confirm` API.

### 11.3 Audit trail differential

| Path | learning_audit event_type | Reviewer recorded |
|---|---|---|
| Human review | `human_review_confirm` | OIDC sub of reviewer |
| Auto-confirm | `auto_confirm` | "system" with conditions snapshot |

### 11.4 Pin tests auto-confirm

- `test_auto_confirm_blocked_for_high_stakes` — even with confidence=1.0 + consent.
- `test_auto_confirm_blocked_without_dpo_consent`.
- `test_auto_confirm_blocked_below_threshold`.
- `test_auto_confirm_allowed_all_3_conditions`.
- `test_auto_confirm_audit_event_type` — distinct from human_review.
- `test_auto_confirm_inserts_gold_example_with_source_tag` — `source = 'al_auto_confirm'`.

---

## 12. Bundle re-tune pipeline

### 12.1 Mission

Périodiquement (default weekly per active catalog), recomputer les
centroids des anchors à partir du `gold_dataset` (excluding holdout),
produire un nouveau bundle artifact, eval contre l'ancien sur le
holdout, flagger pour promotion si supérieur.

### 12.2 Pipeline detail

```rust
pub struct RetuneRun {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub parent_bundle_id: String,
    pub new_bundle_semver: String,         // patch bump
    pub gold_dataset_id: uuid::Uuid,
    pub training_examples_count: u64,
    pub holdout_examples_count: u64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RetuneRunStatus,
    pub eval_metrics_json: Option<serde_json::Value>,
    pub flagged_for_promotion: bool,
}

#[derive(sqlx::Type)]
#[sqlx(type_name = "retune_run_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetuneRunStatus {
    Tuning,
    EvalRunning,
    EvalPass,
    EvalFail,
    Promoted,
    RolledBack,
    Aborted,
}

pub async fn run_retune(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> CurationResult<RetuneRun> {
    // Step 1 : Verify preconditions
    let active_bundle = fetch_active_bundle(pg, tenant, catalog).await?;
    let gold = fetch_active_gold_dataset(pg, tenant, catalog).await?;
    let new_examples_count = count_new_examples_since_last_retune(pg, tenant, catalog).await?;

    let threshold = fetch_tenant_threshold(pg, tenant).await?;
    if new_examples_count < threshold {
        return Err(CurationError::InvalidInput(format!(
            "only {} new examples since last retune (threshold {})",
            new_examples_count, threshold
        )));
    }

    // Step 2 : Create retune run row
    let run_id = uuid::Uuid::new_v4();
    let new_semver = bump_patch(&active_bundle.semver)?;
    insert_retune_run(pg, run_id, tenant, catalog, &active_bundle, &new_semver).await?;

    // Step 3 : Compute new centroids
    let (training_set, holdout_set) = split_train_holdout(pg, tenant, catalog, &gold.id).await?;
    let new_centroids = compute_new_centroids(pg, tenant, catalog, &training_set).await?;

    // Step 4 : Build new bundle artifact
    let new_bundle_artifact = build_bundle_artifact(
        pg, tenant, catalog, &new_semver, &new_centroids,
    ).await?;

    // Step 5 : Cosign sign (R-100 pipeline)
    let signed = cosign_sign_bundle(&new_bundle_artifact).await?;

    // Step 6 : Eval on holdout — both old and new
    let eval = run_holdout_eval(
        pg, tenant, catalog,
        &active_bundle, &new_bundle_artifact, &holdout_set,
    ).await?;

    // Step 7 : Decide
    let flagged = eval.delta_pp >= 1.0;
    let status = if flagged { RetuneRunStatus::EvalPass } else { RetuneRunStatus::EvalFail };
    update_retune_run_eval(
        pg, run_id, &eval, status, flagged,
        &new_bundle_artifact.id, &signed,
    ).await?;

    // Step 8 : Audit
    let mut tx = pg.begin().await?;
    crate::audit::append_learning_audit(
        &mut tx, tenant, None, None, Some(new_bundle_artifact.taxonomy_version_id),
        if flagged { "retune_eval_pass" } else { "retune_eval_fail" },
        "system", None,
        serde_json::json!({
            "run_id": run_id,
            "parent_bundle": active_bundle.id,
            "new_bundle_semver": new_semver,
            "training_count": training_set.len(),
            "holdout_count": holdout_set.len(),
            "accuracy_old": eval.accuracy_old,
            "accuracy_new": eval.accuracy_new,
            "delta_pp": eval.delta_pp,
        }),
    ).await?;
    tx.commit().await?;

    crate::metrics::AL_RETUNE_RUNS_TOTAL
        .with_label_values(&[&tenant.0, &status.as_str()])
        .inc();

    fetch_retune_run(pg, run_id).await
}
```

### 12.3 Scheduled invocation

```rust
pub async fn retune_scheduler_loop(
    pg: Arc<PgPool>, tenants: Arc<dyn TenantRegistry>,
) {
    // Cron : every hour, scan for due tenants/catalogs
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        interval.tick().await;
        for tenant in tenants.iter_active_with_al_enabled().await.unwrap_or_default() {
            for catalog in tenant.catalogs.iter().filter(|c| c.al_enabled) {
                if is_due_for_retune(&pg, &tenant.id, &catalog.id).await.unwrap_or(false) {
                    if let Err(e) = run_retune(&pg, &tenant.id, &catalog.id).await {
                        tracing::error!(?e, "retune run failed");
                    }
                }
            }
        }
    }
}

async fn is_due_for_retune(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> CurationResult<bool> {
    let last_retune: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        r#"SELECT MAX(completed_at) FROM bundle_retune_run
           WHERE tenant_id = $1 AND catalog_id = $2"#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .fetch_one(pg).await?;
    let interval_days: i32 = sqlx::query_scalar(
        "SELECT COALESCE(retune_interval_days, 7) FROM tenant_feature WHERE id = $1"
    ).bind(&tenant.0).fetch_one(pg).await?;
    Ok(match last_retune {
        None => true,
        Some(t) => (chrono::Utc::now() - t).num_days() >= interval_days as i64,
    })
}
```

### 12.4 Pin tests retune

- `test_retune_skips_below_threshold` — N new examples < threshold → no run.
- `test_retune_creates_new_bundle_with_patch_bump`.
- `test_retune_excludes_holdout_from_training`.
- `test_retune_eval_runs_on_holdout`.
- `test_retune_flagged_only_if_delta_pp_ge_1`.
- `test_retune_audit_chain_link`.
- `test_retune_metrics_emitted_per_outcome`.
- `test_retune_aborts_if_active_bundle_missing`.
- `test_retune_concurrent_safe_via_advisory_lock`.
- `test_scheduler_due_calculation` — interval respected.
- `test_scheduler_per_catalog_isolation`.
- `test_retune_persists_eval_metrics_json`.

---

## 13. Embedder re-calibration — centroid update math

### 13.1 Mode-1 anchor classification recap

EFE Mode-1 inference :

```
input sentence s → embedder e(s) ∈ R^d  (d = 768 typical)

per anchor class c with anchors A_c = {a_1, …, a_n} :
  centroid μ_c = (1/n) Σ e(a_i)
  similarity(s, c) = cos(e(s), μ_c)

verdict = argmax_c similarity(s, c)
confidence = max_c similarity(s, c)
```

Centroid is the deterministic mean of anchor embeddings. Re-calibration
= recompute μ_c from gold examples.

### 13.2 Centroid update formula

Given gold examples G_c = {(s_1, c), (s_2, c), …, (s_m, c)} for class c :

```
μ_c_new = α · μ_c_old + (1 - α) · (1/m) Σ e(s_i)
```

Where α ∈ [0, 1] is the **anchor weight** vs **gold weight** :
- α = 1.0 : ignore gold (no change), centroid = old anchors only.
- α = 0.0 : full replacement, centroid = mean of gold examples.
- α = 0.7 : 70% old anchors, 30% gold (default V1).

### 13.3 Tenant-tunable α

```rust
pub struct CentroidUpdateConfig {
    pub anchor_weight: f32,        // α, default 0.7
    pub min_gold_examples_per_class: u32, // default 10 ; below this, skip update
    pub max_centroid_drift: f32,   // default 0.30 ; above this, abort + alert
}

pub fn compute_new_centroid(
    old_centroid: &[f32],
    gold_embeddings: &[Vec<f32>],
    cfg: &CentroidUpdateConfig,
) -> Result<Vec<f32>, CurationError> {
    if gold_embeddings.len() < cfg.min_gold_examples_per_class as usize {
        // Not enough evidence ; keep old centroid
        return Ok(old_centroid.to_vec());
    }
    let d = old_centroid.len();
    let m = gold_embeddings.len() as f32;
    let mut gold_mean = vec![0.0_f32; d];
    for emb in gold_embeddings {
        for (i, v) in emb.iter().enumerate() { gold_mean[i] += v / m; }
    }
    let alpha = cfg.anchor_weight;
    let new: Vec<f32> = old_centroid.iter().zip(gold_mean.iter())
        .map(|(o, g)| alpha * o + (1.0 - alpha) * g)
        .collect();
    // Drift check : cosine distance between old and new
    let drift = 1.0 - cosine_similarity(old_centroid, &new);
    if drift > cfg.max_centroid_drift {
        return Err(CurationError::InvalidInput(format!(
            "centroid drift {} exceeds max {} (likely poisoning)",
            drift, cfg.max_centroid_drift
        )));
    }
    Ok(new)
}
```

### 13.4 Drift threshold = poisoning safety net

`max_centroid_drift = 0.30` means : if the new centroid moves > 30% in
cosine distance from the old, abort the retune. This is a hard safety
net against adversarial poisoning : an attacker injecting many
mislabeled samples would trigger this.

### 13.5 Pin tests centroid update

- `test_centroid_alpha_1_returns_old`.
- `test_centroid_alpha_0_returns_gold_mean`.
- `test_centroid_alpha_07_blends_correctly`.
- `test_centroid_below_min_gold_returns_old`.
- `test_centroid_drift_exceeded_aborts`.
- `test_centroid_dimension_mismatch_errors`.
- `test_centroid_empty_gold_returns_old`.

---

## 14. Holdout split manager

### 14.1 Mission

Maintenir un holdout de 10% du gold dataset, **deterministic** + **immutable**
+ **drift-resistant**. Toute eval bundle utilise EXCLUSIVEMENT le holdout.
Aucun gold example dans le holdout ne peut être utilisé pour training
(centroid update).

### 14.2 Hash-based deterministic split

```rust
pub fn is_in_holdout(
    tenant: &TenantId, catalog: &CatalogId, gold_example_id: uuid::Uuid,
) -> bool {
    let mut h = sha2::Sha256::new();
    use sha2::Digest;
    h.update(tenant.0.as_bytes());
    h.update(b"|");
    h.update(catalog.0.as_bytes());
    h.update(b"|");
    h.update(gold_example_id.as_bytes());
    let digest = h.finalize();
    // Use last byte mod 10 ; bin == 0 → holdout
    (digest[31] % 10) == 0
}
```

**Properties**:
- Deterministic : same gold_example always in same set.
- Roughly 10% (uniform distribution of SHA-256 last byte mod 10).
- Per-tenant per-catalog isolation.
- No state needed (recalculable any time).

### 14.3 Holdout invariants (enforced by DB views + triggers)

```sql
CREATE OR REPLACE VIEW v_gold_examples_training AS
SELECT ge.*
FROM gold_example ge
JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
WHERE NOT (
    -- Inline equivalent of is_in_holdout()
    (substring(encode(digest(gd.tenant_id || '|' || gd.catalog_id || '|' || ge.id::text, 'sha256'), 'hex'), 63, 2)::INTEGER % 10) = 0
);

CREATE OR REPLACE VIEW v_gold_examples_holdout AS
SELECT ge.*
FROM gold_example ge
JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
WHERE (substring(encode(digest(gd.tenant_id || '|' || gd.catalog_id || '|' || ge.id::text, 'sha256'), 'hex'), 63, 2)::INTEGER % 10) = 0;
```

> Note : Postgres doesn't have direct SHA-256 modulo on UUIDs. The
> production view uses `pgcrypto::digest()` and slices the last byte.
> The Rust `is_in_holdout` MUST stay in sync with this view ; tested by
> `test_holdout_split_rust_view_parity`.

### 14.4 Materialised holdout for stable bench

For repeatable bench results, materialise the holdout split per
`bundle_retune_run` :

```sql
CREATE TABLE bundle_holdout_split (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    retune_run_id UUID NOT NULL REFERENCES bundle_retune_run(id),
    gold_example_id UUID NOT NULL REFERENCES gold_example(id),
    UNIQUE(retune_run_id, gold_example_id)
);
```

Each retune snapshot the holdout examples it eval-ed against, for
reproducibility.

### 14.5 Pin tests holdout

- `test_holdout_deterministic_same_inputs_same_result`.
- `test_holdout_distribution_approximately_10pct` — sample 10K UUIDs,
  count holdout-marked, expect 950-1050.
- `test_holdout_per_tenant_isolation` — tenant A holdout != tenant B.
- `test_holdout_view_matches_rust_function` — parity check.
- `test_holdout_immutable_post_retune` — once captured in
  bundle_holdout_split, never re-evaluated.
- `test_holdout_excluded_from_training_centroid_compute`.

---

## 15. Two-stage eval engine (offline + canary)

### 15.1 Stage 1 — offline holdout eval

Fast (~seconds), deterministic. Computes per-class precision / recall /
F1 + aggregate accuracy on the holdout split.

```rust
pub struct HoldoutEvalResult {
    pub bundle_id: String,
    pub holdout_size: u64,
    pub accuracy: f32,
    pub macro_f1: f32,
    pub weighted_f1: f32,
    pub per_class: HashMap<String, ClassMetrics>,
    pub computed_at: DateTime<Utc>,
}

pub async fn run_holdout_eval(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
    old_bundle: &BundleArtifact, new_bundle: &BundleArtifact,
    holdout: &[GoldExample],
) -> CurationResult<EvalDelta> {
    let old_metrics = eval_bundle_on_holdout(old_bundle, holdout).await?;
    let new_metrics = eval_bundle_on_holdout(new_bundle, holdout).await?;
    Ok(EvalDelta {
        accuracy_old: old_metrics.accuracy,
        accuracy_new: new_metrics.accuracy,
        delta_pp: (new_metrics.accuracy - old_metrics.accuracy) * 100.0,
        macro_f1_delta: new_metrics.macro_f1 - old_metrics.macro_f1,
        per_class_delta: compute_per_class_delta(&old_metrics, &new_metrics),
        old: old_metrics, new: new_metrics,
    })
}
```

### 15.2 Stage 2 — canary (5% prod traffic, 24h)

Reuses the M23 canary engine (`crates/efe-curation/src/rollout.rs`).
Only triggered if Stage 1 (offline eval) passes the +1pp threshold.

### 15.3 Decision matrix

| Offline eval | Canary | Decision |
|---|---|---|
| delta < +1pp | (skipped) | Reject — no promotion |
| delta ≥ +1pp | metrics regress | Auto-rollback |
| delta ≥ +1pp | metrics OK | Promote (auto OR operator) |

### 15.4 Pin tests eval

- `test_eval_old_vs_new_on_same_holdout`.
- `test_eval_per_class_metrics_computed`.
- `test_eval_accuracy_delta_pp_correct`.
- `test_eval_skips_canary_if_offline_fails`.
- `test_eval_canary_reuses_m23_engine`.

---

## 16. Promotion gate + auto-promote

### 16.1 Promotion decision tree

```
                        ┌─────────────────────────┐
                        │  retune_run.status =    │
                        │  EvalPass               │
                        └────────────┬────────────┘
                                     │
                                     ▼
                        ┌─────────────────────────┐
                        │ tenant.auto_promotion?  │
                        └────────────┬────────────┘
                                     │
                  ┌──────────────────┴──────────────────┐
                  │                                     │
                  ▼ false (default)                     ▼ true
       ┌──────────────────────┐         ┌──────────────────────────┐
       │ Flag in portal-admin │         │ catalog.stakes_tier?     │
       │ Notify operator      │         └────────────┬─────────────┘
       │ Wait for approval    │                      │
       └──────────────────────┘             ┌────────┴────────┐
                                            │                 │
                                            ▼ low             ▼ medium/high
                                  ┌──────────────────┐ ┌──────────────┐
                                  │ DPO consent      │ │ Disallow     │
                                  │ active?          │ │ auto-promote │
                                  └────────┬─────────┘ │ (compliance) │
                                           │           └──────────────┘
                                  ┌────────┴────────┐
                                  │                 │
                                  ▼ true            ▼ false
                       ┌─────────────────┐ ┌──────────────┐
                       │ AUTO-PROMOTE    │ │ Flag for     │
                       │ via canary      │ │ operator     │
                       └─────────────────┘ └──────────────┘
```

### 16.2 Auto-promote enforcement

```rust
pub async fn evaluate_promotion_eligibility(
    pg: &PgPool, run: &RetuneRun,
) -> CurationResult<PromotionEligibility> {
    if !run.flagged_for_promotion {
        return Ok(PromotionEligibility::NotEligible("eval did not pass".into()));
    }
    let tenant_features = fetch_tenant_features(pg, &run.tenant_id).await?;
    let catalog = fetch_catalog(pg, &run.tenant_id, &run.catalog_id).await?;

    if !tenant_features.auto_promotion_enabled {
        return Ok(PromotionEligibility::FlagForOperator);
    }
    if catalog.stakes_tier != StakesTier::Low {
        // CRITICAL : compliance gate — cannot auto-promote on medium/high stakes
        return Ok(PromotionEligibility::FlagForOperator);
    }
    if tenant_features.human_review_required {
        return Ok(PromotionEligibility::FlagForOperator);
    }
    if !check_dpo_consent_active(pg, &run.tenant_id).await? {
        return Ok(PromotionEligibility::FlagForOperator);
    }
    Ok(PromotionEligibility::AutoPromote)
}
```

### 16.3 Pin tests promotion

- `test_auto_promote_blocked_for_high_stakes`.
- `test_auto_promote_blocked_for_medium_stakes`.
- `test_auto_promote_blocked_without_dpo_consent`.
- `test_auto_promote_blocked_when_human_review_required`.
- `test_auto_promote_allowed_low_stakes_all_conditions`.
- `test_promote_uses_canary_engine`.
- `test_promote_audit_trail_records_decision_path`.

---

## 17. GDPR Art. 17 cascade

### 17.1 Mission

When a data subject requests erasure (Art. 17.1.b GDPR), EFE must :
1. Tombstone the affected `gold_example` rows (preserve HMAC chain).
2. Cascade : the bundle whose centroids were trained on these examples
   must be rebuilt without them.

### 17.2 Cascade workflow

```
1. Data subject submits request (via portal /gdpr/erasure)
   → POST /v1/gdpr/erasure { tenant_id, subject_id, content_hashes[] }

2. gdpr_art17_processor (worker) :
   a. Identify affected gold_example rows (via content_hash lookup)
   b. Tombstone : SET text = '[REDACTED-Art17-{date}]', pii_redacted_at = NOW()
   c. INSERT learning_audit (event_type=gdpr_art17_redaction)
   d. INSERT bundle_rebuild_request (priority based on urgency)

3. bundle_rebuild_processor (worker, separate from scheduled retune) :
   a. Reads bundle_rebuild_request (priority queue)
   b. Triggers immediate retune (skipping the threshold check)
   c. New bundle excludes tombstoned examples
   d. Two-stage eval as usual
   e. Force-promote (skip operator gate ONLY for GDPR Art. 17,
      with explicit audit trail)

4. Confirmation : email data subject + EFE compliance log
```

### 17.3 SLA

| Urgency | Tombstone | Bundle rebuild | Promotion |
|---|---|---|---|
| Standard (Art. 17 default) | < 1 h | < 30 days | Next scheduled retune |
| Regulatory urgency (e.g. CNIL formal demand) | < 1 h | < 72 h | Force-promoted |
| Catastrophic (security incident) | < 15 min | < 24 h | Force-promoted |

### 17.4 Force-promote semantic

Force-promote skips :
- Operator approval gate
- Auto-promote conditions (low_stakes etc.)

But STILL goes through :
- Canary 24h (or shortened to 1h for catastrophic)
- Drift watch post-promote

The audit trail records `event_type=gdpr_art17_force_promote` with
the request_id reference.

### 17.5 Pin tests GDPR

- `test_art17_tombstone_preserves_hmac_chain`.
- `test_art17_cascade_triggers_bundle_rebuild`.
- `test_art17_priority_queue_orders_by_urgency`.
- `test_art17_force_promote_skips_operator_gate`.
- `test_art17_force_promote_still_runs_canary`.
- `test_art17_audit_records_subject_id_and_request_id`.

---

## 18. Data model (SQL + migration 0015)

### 18.1 Migration `0015_active_learning.sql`

```sql
-- 0015_active_learning.sql
-- M24 — Active learning loop
-- Runs on top of 0014_taxonomy_curation.sql (M23).

BEGIN;

-- ─── ENUMs ─────────────────────────────────────────────────────────

CREATE TYPE gold_label_queue_status AS ENUM (
    'ENQUEUED', 'LABELING', 'LLM_LABELED', 'AWAITING_REVIEW',
    'CONFIRMED', 'REJECTED', 'FAILED', 'TOMBSTONED'
);

CREATE TYPE gold_label_review_decision AS ENUM (
    'CONFIRM', 'EDIT_AND_CONFIRM', 'REJECT'
);

CREATE TYPE retune_run_status AS ENUM (
    'TUNING', 'EVAL_RUNNING', 'EVAL_PASS', 'EVAL_FAIL',
    'PROMOTED', 'ROLLED_BACK', 'ABORTED'
);

CREATE TYPE stakes_tier AS ENUM ('LOW', 'MEDIUM', 'HIGH');

-- ─── gold_label_queue ──────────────────────────────────────────────

CREATE TABLE gold_label_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[a-f0-9]{64}$'),
    sentence_redacted TEXT NOT NULL,
    proposed_class_id TEXT,
    observed_confidence DOUBLE PRECISION,
    status gold_label_queue_status NOT NULL DEFAULT 'ENQUEUED',
    source_llm_call_id UUID,
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    labeled_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    failure_count INT NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
    last_error TEXT,
    UNIQUE(tenant_id, content_hash)
);

CREATE INDEX gold_label_queue_status_idx
    ON gold_label_queue(tenant_id, status, enqueued_at)
    WHERE status IN ('ENQUEUED', 'LABELING', 'LLM_LABELED');

CREATE INDEX gold_label_queue_catalog_idx
    ON gold_label_queue(tenant_id, catalog_id);

-- ─── gold_label_proposal ───────────────────────────────────────────

CREATE TABLE gold_label_proposal (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue_id UUID NOT NULL REFERENCES gold_label_queue(id) ON DELETE RESTRICT,
    tenant_id TEXT NOT NULL,
    proposed_class_id TEXT NOT NULL,
    llm_confidence DOUBLE PRECISION NOT NULL CHECK (llm_confidence >= 0 AND llm_confidence <= 1),
    llm_rationale TEXT NOT NULL,
    llm_call_audit_id UUID,
    prompt_template_name TEXT NOT NULL,
    prompt_template_version INT NOT NULL,
    auto_confirmed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX gold_label_proposal_queue_idx
    ON gold_label_proposal(queue_id);

-- ─── gold_label_review ─────────────────────────────────────────────

CREATE TABLE gold_label_review (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    proposal_id UUID NOT NULL REFERENCES gold_label_proposal(id) ON DELETE RESTRICT,
    reviewer_oidc_sub TEXT NOT NULL,
    reviewer_role TEXT NOT NULL,
    decision gold_label_review_decision NOT NULL,
    final_class_id TEXT,
    rationale TEXT NOT NULL CHECK (length(rationale) >= 10),
    quality_tag TEXT,
    oidc_id_token_hash TEXT NOT NULL,
    oidc_token_iat TIMESTAMPTZ NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(proposal_id, reviewer_oidc_sub),
    CONSTRAINT review_oidc_iat_fresh CHECK (reviewed_at - oidc_token_iat < INTERVAL '24 hours')
);

-- ─── bundle_retune_run ─────────────────────────────────────────────

CREATE TABLE bundle_retune_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    parent_taxonomy_version_id UUID NOT NULL REFERENCES taxonomy_version(id),
    new_taxonomy_version_id UUID REFERENCES taxonomy_version(id),
    new_bundle_semver TEXT NOT NULL,
    gold_dataset_id UUID NOT NULL REFERENCES gold_dataset(id),
    training_examples_count BIGINT NOT NULL CHECK (training_examples_count >= 0),
    holdout_examples_count BIGINT NOT NULL CHECK (holdout_examples_count >= 0),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status retune_run_status NOT NULL DEFAULT 'TUNING',
    eval_metrics_json JSONB,
    flagged_for_promotion BOOLEAN NOT NULL DEFAULT FALSE,
    cosign_signature_url TEXT,
    rekor_log_index BIGINT,
    triggered_by TEXT NOT NULL CHECK (triggered_by IN ('scheduler', 'on_demand', 'gdpr_cascade')),
    triggered_by_actor TEXT
);

CREATE INDEX retune_run_pending_idx
    ON bundle_retune_run(tenant_id, catalog_id, status)
    WHERE status IN ('TUNING', 'EVAL_RUNNING', 'EVAL_PASS');

CREATE INDEX retune_run_history_idx
    ON bundle_retune_run(tenant_id, catalog_id, started_at DESC);

-- ─── bundle_holdout_split ──────────────────────────────────────────

CREATE TABLE bundle_holdout_split (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    retune_run_id UUID NOT NULL REFERENCES bundle_retune_run(id) ON DELETE CASCADE,
    gold_example_id UUID NOT NULL REFERENCES gold_example(id),
    UNIQUE(retune_run_id, gold_example_id)
);

CREATE INDEX bundle_holdout_split_run_idx ON bundle_holdout_split(retune_run_id);

-- ─── bundle_rebuild_request (GDPR Art. 17 cascade) ─────────────────

CREATE TABLE bundle_rebuild_request (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('gdpr_art17', 'security_incident', 'manual')),
    urgency TEXT NOT NULL CHECK (urgency IN ('standard', 'regulatory', 'catastrophic')),
    affected_gold_example_ids UUID[] NOT NULL,
    requester_subject_id TEXT,         -- GDPR data subject ID (PII-safe handle)
    request_received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sla_deadline TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    completed_retune_run_id UUID REFERENCES bundle_retune_run(id),
    status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'IN_PROGRESS', 'COMPLETED', 'FAILED'))
);

CREATE INDEX bundle_rebuild_pending_idx
    ON bundle_rebuild_request(tenant_id, sla_deadline)
    WHERE status IN ('PENDING', 'IN_PROGRESS');

-- ─── learning_audit (HMAC chained) ─────────────────────────────────

CREATE TABLE learning_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    parent_id UUID,                      -- prev row in chain
    queue_id UUID REFERENCES gold_label_queue(id),
    proposal_id UUID REFERENCES gold_label_proposal(id),
    retune_run_id UUID REFERENCES bundle_retune_run(id),
    taxonomy_version_id UUID REFERENCES taxonomy_version(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'sample_enqueued', 'llm_label_proposed', 'auto_confirmed',
        'human_review_confirm', 'human_review_edit', 'human_review_reject',
        'gold_example_inserted', 'retune_started', 'retune_eval_pass',
        'retune_eval_fail', 'retune_aborted', 'promotion_eligibility_check',
        'promote_canary_started', 'promote_completed', 'promote_rolled_back',
        'gdpr_art17_redaction', 'gdpr_art17_force_promote',
        'budget_exceeded', 'pii_unredactable_skipped',
        'sample_failed_3x'
    )),
    actor_id TEXT NOT NULL,
    actor_role TEXT,
    payload_json JSONB NOT NULL,
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[a-f0-9]{64}$'),
    hmac TEXT NOT NULL CHECK (hmac ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT learning_audit_parent_fk
        FOREIGN KEY (parent_id) REFERENCES learning_audit(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX learning_audit_chain_idx
    ON learning_audit(tenant_id, created_at);
CREATE INDEX learning_audit_queue_idx
    ON learning_audit(queue_id) WHERE queue_id IS NOT NULL;
CREATE INDEX learning_audit_retune_idx
    ON learning_audit(retune_run_id) WHERE retune_run_id IS NOT NULL;

-- ─── tenant_feature M24 columns ────────────────────────────────────

ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS m24_active_learning_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_auto_promotion_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_human_review_required BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS m24_dpo_consent_signed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m24_dpo_consent_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m24_uncertainty_threshold DOUBLE PRECISION DEFAULT 0.70,
    ADD COLUMN IF NOT EXISTS m24_auto_confirm_threshold DOUBLE PRECISION DEFAULT 0.90,
    ADD COLUMN IF NOT EXISTS m24_queue_max_size INT DEFAULT 10000,
    ADD COLUMN IF NOT EXISTS m24_gold_label_threshold INT DEFAULT 1000,
    ADD COLUMN IF NOT EXISTS m24_retune_interval_days INT DEFAULT 7,
    ADD COLUMN IF NOT EXISTS m24_anchor_weight_alpha DOUBLE PRECISION DEFAULT 0.70,
    ADD COLUMN IF NOT EXISTS m24_max_centroid_drift DOUBLE PRECISION DEFAULT 0.30,
    ADD COLUMN IF NOT EXISTS m24_promotion_threshold_pp DOUBLE PRECISION DEFAULT 1.00,
    ADD COLUMN IF NOT EXISTS m24_active_learning_budget_cents_per_day BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS m24_review_sla_hours INT DEFAULT 48,
    ADD COLUMN IF NOT EXISTS m24_notify_after_hours INT DEFAULT 8;

-- ─── catalog M24 columns ──────────────────────────────────────────

ALTER TABLE catalog
    ADD COLUMN IF NOT EXISTS m24_al_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_stakes_tier stakes_tier NOT NULL DEFAULT 'MEDIUM',
    ADD COLUMN IF NOT EXISTS m24_drift_threshold_pp DOUBLE PRECISION DEFAULT 2.00;

-- ─── RLS policies ──────────────────────────────────────────────────

ALTER TABLE gold_label_queue ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_label_proposal ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_label_review ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_retune_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_holdout_split ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_rebuild_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE learning_audit ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_gold_label_queue ON gold_label_queue
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_label_proposal ON gold_label_proposal
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_label_review ON gold_label_review
    USING (
        EXISTS (SELECT 1 FROM gold_label_proposal p
                WHERE p.id = gold_label_review.proposal_id
                  AND p.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_bundle_retune_run ON bundle_retune_run
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_bundle_holdout_split ON bundle_holdout_split
    USING (
        EXISTS (SELECT 1 FROM bundle_retune_run br
                WHERE br.id = bundle_holdout_split.retune_run_id
                  AND br.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_bundle_rebuild_request ON bundle_rebuild_request
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_learning_audit ON learning_audit
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─── Triggers ─────────────────────────────────────────────────────

-- Force gold_label_queue status state machine
CREATE OR REPLACE FUNCTION fn_gold_label_queue_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    -- Allowed transitions :
    --   ENQUEUED → LABELING, FAILED, TOMBSTONED
    --   LABELING → LLM_LABELED, ENQUEUED (retry), FAILED
    --   LLM_LABELED → AWAITING_REVIEW, CONFIRMED (auto), REJECTED
    --   AWAITING_REVIEW → CONFIRMED, REJECTED, TOMBSTONED
    --   CONFIRMED → TOMBSTONED (GDPR Art. 17)
    --   REJECTED → (terminal)
    --   FAILED → (terminal)
    --   TOMBSTONED → (terminal)
    IF (OLD.status, NEW.status) NOT IN (
        ('ENQUEUED', 'LABELING'), ('ENQUEUED', 'FAILED'), ('ENQUEUED', 'TOMBSTONED'),
        ('LABELING', 'LLM_LABELED'), ('LABELING', 'ENQUEUED'), ('LABELING', 'FAILED'),
        ('LLM_LABELED', 'AWAITING_REVIEW'), ('LLM_LABELED', 'CONFIRMED'), ('LLM_LABELED', 'REJECTED'),
        ('AWAITING_REVIEW', 'CONFIRMED'), ('AWAITING_REVIEW', 'REJECTED'), ('AWAITING_REVIEW', 'TOMBSTONED'),
        ('CONFIRMED', 'TOMBSTONED')
    ) AND OLD.status IS DISTINCT FROM NEW.status THEN
        RAISE EXCEPTION 'invalid gold_label_queue transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_gold_label_queue_state_machine
    BEFORE UPDATE ON gold_label_queue
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_gold_label_queue_state_machine();

-- Force bundle_retune_run state machine
CREATE OR REPLACE FUNCTION fn_retune_run_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.status, NEW.status) NOT IN (
        ('TUNING', 'EVAL_RUNNING'), ('TUNING', 'ABORTED'),
        ('EVAL_RUNNING', 'EVAL_PASS'), ('EVAL_RUNNING', 'EVAL_FAIL'), ('EVAL_RUNNING', 'ABORTED'),
        ('EVAL_PASS', 'PROMOTED'), ('EVAL_PASS', 'ROLLED_BACK'), ('EVAL_PASS', 'ABORTED'),
        ('EVAL_FAIL', 'ABORTED'),
        ('PROMOTED', 'ROLLED_BACK')
    ) AND OLD.status IS DISTINCT FROM NEW.status THEN
        RAISE EXCEPTION 'invalid retune_run transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_retune_run_state_machine
    BEFORE UPDATE ON bundle_retune_run
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_retune_run_state_machine();

-- ─── Schema migration tracking ─────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0015_active_learning',
    NOW(),
    'M24 — Active learning : 7 tables + 16 tenant_feature columns + 3 catalog columns + RLS + state machine triggers'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
```

### 18.2 Storage estimate

| Table | Rows/tenant/an (estim.) | Avg size | Total/tenant/an |
|---|---:|---:|---:|
| `gold_label_queue` | 100K-1M | ~1 KB | ~500 MB |
| `gold_label_proposal` | 100K-1M | ~2 KB | ~1 GB |
| `gold_label_review` | 50K-500K | ~1 KB | ~250 MB |
| `bundle_retune_run` | ~50 | ~5 KB | ~250 KB |
| `bundle_holdout_split` | 5K-50K (per retune × 50) | ~80 B | ~25 MB |
| `bundle_rebuild_request` | 0-1K | ~500 B | ~500 KB |
| `learning_audit` | 200K-2M | ~2 KB | ~2 GB |
| **Total/tenant/an** | | | **~4 GB** |

À 80 tenants : ~320 GB/an. Manageable mais nécessite partitioning
post-3 ans (cf. § 33 O-7).

---

## 19. State machines

### 19.1 Sample lifecycle

```
                        ┌─────────────┐
                        │  observed   │ (production classify)
                        │ (implicit)  │
                        └──────┬──────┘
                               │ enqueuer scan
                               ▼
                        ┌─────────────┐
                        │  ENQUEUED   │
                        └──────┬──────┘
                               │ worker dequeue
                               ▼
                        ┌─────────────┐
                        │  LABELING   │
                        └─────┬───────┘
                              │
                ┌─────────────┼─────────────┐
                │             │             │
                ▼             ▼             ▼
         ┌──────────┐  ┌──────────┐  ┌─────────┐
         │  LLM_    │  │ ENQUEUED │  │ FAILED  │ (3x)
         │ LABELED  │  │ (retry)  │  └─────────┘
         └────┬─────┘  └──────────┘
              │
   ┌──────────┼──────────────┐
   │ auto-confirm           │ require human
   ▼                         ▼
┌────────────┐   ┌─────────────────────┐
│ CONFIRMED  │   │  AWAITING_REVIEW    │
└─────┬──────┘   └──────┬──────────────┘
      │                 │
      │   ┌─────────────┼─────────────┐
      │   │             │             │
      │   ▼             ▼             ▼
      │ ┌───────────┐ ┌──────────┐ ┌──────────┐
      │ │ CONFIRMED │ │ REJECTED │ │TOMBSTONED│
      │ └───────────┘ └──────────┘ └──────────┘
      │
      └──── (later, GDPR Art. 17) ────► TOMBSTONED
```

### 19.2 Bundle retune run lifecycle

```
            ┌──────────┐
            │  TUNING  │
            └────┬─────┘
                 │ centroid recompute done
                 ▼
            ┌────────────────┐
            │  EVAL_RUNNING  │
            └────┬───────────┘
                 │ eval done
                 ▼
       ┌─────────┴─────────┐
       │                   │
       ▼                   ▼
┌─────────────┐     ┌──────────────┐
│  EVAL_PASS  │     │  EVAL_FAIL   │ (terminal)
└─────┬───────┘     └──────────────┘
      │
   ┌──┴──────────────┬──────────────┐
   │                 │              │
   ▼                 ▼              ▼
┌──────────┐  ┌─────────────┐  ┌──────────┐
│ PROMOTED │  │ ROLLED_BACK │  │ ABORTED  │
└────┬─────┘  └─────────────┘  └──────────┘
     │
     └─ (post-promote drift > 2pp) ──► ROLLED_BACK
```

### 19.3 Auto-confirm gate (decision-only, not stored as state)

```
Inputs : llm_confidence, tenant.auto_confirm_threshold,
         tenant.auto_promotion_enabled, tenant.dpo_consent_active,
         catalog.stakes_tier

Output : auto_confirm_decision (bool)
         auto_promote_decision (bool, only if confirm=true and stakes_tier=Low)
```

---

## 20. gRPC service contracts

### 20.1 Service `ActiveLearningService`

```protobuf
syntax = "proto3";
package efe.active_learning.v1;

import "google/protobuf/timestamp.proto";
import "google/protobuf/struct.proto";

service ActiveLearningService {
    // ── Queue management ─────────────────────────────────────────────
    rpc ListGoldLabelQueue(ListGoldLabelQueueRequest) returns (ListGoldLabelQueueResponse);
    rpc GetGoldLabelProposal(GetGoldLabelProposalRequest) returns (GetGoldLabelProposalResponse);

    // ── Review actions ──────────────────────────────────────────────
    rpc ConfirmGoldLabel(ConfirmGoldLabelRequest) returns (ConfirmGoldLabelResponse);
    rpc EditAndConfirmGoldLabel(EditAndConfirmGoldLabelRequest) returns (ConfirmGoldLabelResponse);
    rpc RejectGoldLabel(RejectGoldLabelRequest) returns (RejectGoldLabelResponse);

    // ── Retune ──────────────────────────────────────────────────────
    rpc TriggerBundleRetune(TriggerBundleRetuneRequest) returns (TriggerBundleRetuneResponse);
    rpc ListRetuneRuns(ListRetuneRunsRequest) returns (ListRetuneRunsResponse);
    rpc GetRetuneRun(GetRetuneRunRequest) returns (GetRetuneRunResponse);

    // ── Promotion ───────────────────────────────────────────────────
    rpc PromoteRetunedBundle(PromoteRetunedBundleRequest) returns (PromoteRetunedBundleResponse);
    rpc RollbackPromotion(RollbackPromotionRequest) returns (RollbackPromotionResponse);

    // ── GDPR ────────────────────────────────────────────────────────
    rpc TriggerGdprArt17Erasure(TriggerGdprArt17ErasureRequest) returns (TriggerGdprArt17ErasureResponse);
    rpc ListGdprErasureRequests(ListGdprErasureRequestsRequest) returns (ListGdprErasureRequestsResponse);

    // ── Stats / monitoring ──────────────────────────────────────────
    rpc GetActiveLearningStats(GetActiveLearningStatsRequest) returns (GetActiveLearningStatsResponse);
    rpc GetEffectiveBundleVersion(GetEffectiveBundleVersionRequest) returns (GetEffectiveBundleVersionResponse);

    // ── DPO consent management ──────────────────────────────────────
    rpc RegisterDpoConsent(RegisterDpoConsentRequest) returns (RegisterDpoConsentResponse);
}
```

(30+ messages defined in `protos/efe/active_learning/v1/messages.proto`,
following the M23 pattern.)

### 20.2 Authorization matrix per RPC

| RPC | tenant_admin | curation_reviewer | curation_promoter | viewer |
|---|:---:|:---:|:---:|:---:|
| `ListGoldLabelQueue` | ✅ | ✅ | ✅ | ✅ (read) |
| `GetGoldLabelProposal` | ✅ | ✅ | ✅ | ✅ (read) |
| `ConfirmGoldLabel` | ✅ | ✅ | ❌ | ❌ |
| `EditAndConfirmGoldLabel` | ✅ | ✅ | ❌ | ❌ |
| `RejectGoldLabel` | ✅ | ✅ | ❌ | ❌ |
| `TriggerBundleRetune` | ✅ | ❌ | ✅ | ❌ |
| `ListRetuneRuns` | ✅ | ✅ | ✅ | ✅ (read) |
| `GetRetuneRun` | ✅ | ✅ | ✅ | ✅ (read) |
| `PromoteRetunedBundle` | ✅ | ❌ | ✅ | ❌ |
| `RollbackPromotion` | ✅ | ❌ | ✅ | ❌ |
| `TriggerGdprArt17Erasure` | ✅ | ❌ | ❌ | ❌ |
| `RegisterDpoConsent` | ✅ | ❌ | ❌ | ❌ |

### 20.3 OIDC freshness checks

Same as M23 : `Confirm/Reject/Promote` require OIDC token iat < 10
min ; `RegisterDpoConsent` requires iat < 5 min (more sensitive).

---

## 21. mcp-server tools

### 21.1 4 nouveaux tools

Following the M23 pattern (`from .types import Tool, ToolContext, ToolError`),
not FastMCP. All gated by `feature_flag="EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED"`.

| Tool | Type | Description |
|---|---|---|
| `efe_list_gold_label_queue` | read | Enumerate pending gold label queue items |
| `efe_confirm_gold_label` | write | Confirm an LLM-proposed label (operator action) |
| `efe_trigger_bundle_retune` | write | On-demand retune trigger (skips weekly schedule wait) |
| `efe_request_gdpr_erasure` | write | Submit GDPR Art. 17 erasure request |

### 21.2 Tool spec : `efe_trigger_bundle_retune`

```python
_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeTriggerBundleRetuneInput",
    "description": (
        "Trigger an on-demand retune of a bundle for a (tenant, catalog). "
        "Skips the weekly schedule cadence. The retune still respects all "
        "M24 guardrails : threshold of new gold examples, two-stage eval, "
        "promotion gate. Output is a bundle_retune_run row id."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "force_below_threshold": {
            "type": "boolean", "default": False,
            "description": (
                "If true, run retune even if N new gold examples < threshold. "
                "Reserved for tenant_admin role. Logged with elevated audit event."
            ),
        },
    },
    "required": ["tenant_id", "catalog_id"],
    "additionalProperties": False,
}
```

### 21.3 Pin tests mcp tools

- `test_register_tools_off_by_default` — feature OFF → 0 tools.
- `test_register_tools_on` — feature ON → 4 tools.
- `test_trigger_retune_force_requires_admin`.
- `test_confirm_label_requires_reviewer_role`.
- `test_gdpr_erasure_requires_tenant_admin`.
- `test_list_queue_per_tenant_isolated`.

---

## 22. efe-admin CLI extensions

### 22.1 Command tree

```
efe-admin learning
├── queue
│   ├── list                       # current pending labels
│   ├── stats                      # queue size, lag p99, etc.
│   └── pause / resume             # operator can pause enqueueing
├── proposal
│   ├── confirm <id> --reason "..."
│   ├── reject <id> --reason "..."
│   └── edit <id> --class <new-id> --reason "..."
├── retune
│   ├── trigger --catalog <id>     # on-demand retune
│   ├── list                       # history of retune runs
│   └── get <run-id>               # detail of a retune run
├── promote <run-id>               # operator-approved promotion
├── rollback <run-id> --reason
├── gdpr
│   ├── erase --subject <id> --content-hash <h>
│   └── list-pending
├── stats
│   └── overview                   # all-up M24 stats per catalog
└── dpo-consent
    ├── register --signer <name> --expires-at <date>
    └── show
```

### 22.2 Implementation

Same pattern as M23 CLI : `crates/efe-admin/src/commands/learning/`
mod with subcommand files. Reuses `commands::curation::OutputFormat`.
Connect via `crates/efe-admin/src/grpc/active_learning_client.rs`
(same REST-over-HTTP pattern).

### 22.3 Pin tests CLI

- `test_cli_learning_queue_list` — table/json/yaml.
- `test_cli_learning_proposal_confirm_requires_reason`.
- `test_cli_learning_retune_trigger_dry_run`.
- `test_cli_learning_promote_confirm_typed`.
- `test_cli_learning_gdpr_erasure_admin_only`.

---

## 23. portal-admin UI extensions

### 23.1 New pages

```
apps/portal-admin/src/app/learning/
├── queue/page.tsx           # gold label queue (size, lag, items)
├── proposal/[id]/page.tsx   # single proposal detail + confirm/reject
├── retunes/page.tsx         # retune run history + status
├── retunes/[id]/page.tsx    # single retune detail (eval metrics, holdout split)
├── holdout/page.tsx         # holdout dataset stats per catalog
├── cost/page.tsx            # active learning cost breakdown
└── dpo-consent/page.tsx     # DPO consent management (tenant_admin only)
```

### 23.2 Components

```
apps/portal-admin/src/components/learning/
├── QueueStatusBanner.tsx        # pending count + back-pressure flag
├── GoldLabelProposalCard.tsx    # one proposal item
├── ConfirmEditDialog.tsx        # operator confirm with optional class edit
├── RetuneRunCard.tsx            # one retune run summary
├── EvalMetricsTable.tsx         # accuracy / F1 / per-class
├── HoldoutSplitChart.tsx        # holdout vs training split visualization
├── CentroidDriftChart.tsx       # centroid drift over time
└── AutoConfirmGateIndicator.tsx # visual : 3 conditions checkboxes
```

### 23.3 Auto-refresh behavior

The queue page polls every 30 s for live updates. Other pages are
SSR with `cache: "no-store"`.

### 23.4 Pin tests UI

- `test_queue_page_renders_pending_items` (Playwright).
- `test_proposal_card_confirm_dialog_validates_reason`.
- `test_retune_run_card_shows_status_badge`.
- `test_holdout_chart_renders_split_proportion`.
- `test_dpo_consent_page_requires_tenant_admin`.
- `test_back_pressure_banner_shown_at_capacity`.

---

## 24. Multi-tenant isolation M24

### 24.1 Boundary table (defense in depth)

| Boundary | Mécanisme | Pin test |
|---|---|---|
| **Tenant ↔ tenant** | RLS Postgres + `app.current_tenant_id` per session | `test_rls_no_tenant_var_returns_empty` |
| **Catalog ↔ catalog** | catalog_id checked in every workflow | `test_cross_catalog_dequeue_denied` |
| **LLM prompt ↔ tenant data** | PII redactor mandatory + per-tenant prompt context | `test_pii_redact_called_before_label_llm` |
| **Holdout ↔ training** | Hash-based deterministic split | `test_holdout_disjoint_from_training` |
| **Audit chain ↔ tenant** | Per-tenant chain (HMAC key derivation) | `test_audit_chain_per_tenant_isolated` |
| **Worker scope** | RLS set per-tenant in worker loop | `test_worker_only_processes_own_tenant` |

### 24.2 Cross-tenant adversarial test

`test_adversarial_cross_tenant_label_injection` :
- Tenant `acme` injects gold examples with the SAME content_hash as tenant `beta`'s production samples
- Run uncertainty sampler for tenant `beta`
- Assert : `beta`'s queue is empty (acme's gold examples are NOT visible due to RLS + tenant_id scoping in dedup query)

---

## 25. Cost governance M24

### 25.1 Budget category extension

Reuses M21 budget infrastructure. M24 adds `active_learning` category :

```
Tenant total budget : $X / month (hard cap)
├── classify (Mode-1 + Mode-2) : $A
├── multi-turn (Mode-3) : $B
├── curation (M23) : $C
└── active_learning (M24) : $D
    ├── llm_labeling : $D1
    ├── on_demand_retune : (compute cost, no LLM)
    └── gdpr_force_promote : exempt (always allowed if M24 active)
```

### 25.2 Per-call cost estimation

```rust
pub fn estimate_labeling_cost(rendered_prompt: &str, model: &Model) -> u32 {
    let input_tokens = approximate_tokens(rendered_prompt);
    let output_tokens = 200; // label + confidence + rationale = ~200 tokens
    let input_cost = (input_tokens as f32) * model.input_per_1k_cents / 1000.0;
    let output_cost = (output_tokens as f32) * model.output_per_1k_cents / 1000.0;
    (input_cost + output_cost).ceil() as u32
}
```

Default : ~$0.03/sample with claude-3-haiku, ~$0.15/sample with
claude-3-5-sonnet (cf. Annexe B for full cost model).

### 25.3 Circuit breaker

If `tenant.active_learning_budget_cents_per_day` is exhausted mid-batch :
1. Worker stops (no partial labeling).
2. Metric `AL_LLM_WORKER_BUDGET_BLOCKED_TOTAL` incremented.
3. Slack/email alert.
4. Worker resumes next day at 00:00 UTC (budget reset).

### 25.4 Pin tests cost

- `test_estimate_cost_below_budget_proceeds`.
- `test_estimate_cost_exceeds_budget_blocks`.
- `test_circuit_breaker_emits_metric`.
- `test_daily_budget_reset_at_midnight_utc`.
- `test_gdpr_force_promote_exempt_from_budget`.

---

## 26. Compliance + AI Act + GDPR

### 26.1 Frameworks impacted

| Framework | Article | M24 hook |
|---|---|---|
| **EU AI Act 2024/1689** | Art. 11 (technical doc) | M24 added to Annex IV |
| **EU AI Act 2024/1689** | Art. 14 (human oversight) | `human_review_required=true` default |
| **EU AI Act 2024/1689** | Art. 15 (accuracy) | Promotion gate `delta_pp ≥ +1` |
| **GDPR (EU 2016/679)** | Art. 17 (erasure) | Cascade workflow § 17 |
| **GDPR (EU 2016/679)** | Art. 22 (automated decision) | `auto_promotion` gated by `low_stakes` + DPO consent |
| **SOC 2** | CC8 (change management) | Every retune is a logged change |
| **ISO 27001:2022** | A.8.32 (change management) | Same as SOC 2 |

### 26.2 DPO consent workflow

DPO consent is required to enable `auto_promotion`. Workflow :

1. Tenant admin signs DPO addendum (M24-specific) — covers :
   - LLM oracle as new automated decision-maker
   - Cascade rebuild of bundles on Art. 17 erasure
   - 30-day disclosure of new training data sources
2. Signed addendum hash stored in `tenant_feature.m24_dpo_consent_signed_at`.
3. Validity window : 12 months, must be re-signed annually.
4. Expiry → `auto_promotion` automatically reverts to false.

### 26.3 Audit trail completeness

For every promoted bundle, the lineage query reconstructs :
- All gold examples used in centroid recompute (training set)
- All gold examples in holdout
- All LLM labeling calls (with audit_id + cost)
- All human reviews (with reviewer OIDC sub)
- All eval metrics (accuracy, F1)
- DPO consent active at time of decision

```sql
-- Lineage query for bundle B
WITH bundle_lineage AS (
    SELECT br.*, tv.semver, tv.cosign_signature_url
    FROM bundle_retune_run br
    JOIN taxonomy_version tv ON tv.id = br.new_taxonomy_version_id
    WHERE br.id = $1
)
SELECT
    bl.*,
    (SELECT json_agg(le.payload_json)
     FROM learning_audit le
     WHERE le.retune_run_id = bl.id)            AS audit_events,
    (SELECT COUNT(*) FROM bundle_holdout_split
     WHERE retune_run_id = bl.id)               AS holdout_count,
    (SELECT COUNT(*) FROM gold_example ge
     JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
     WHERE gd.id = bl.gold_dataset_id)          AS total_gold_count
FROM bundle_lineage bl;
```

### 26.4 Pin tests compliance

- `test_dpo_consent_required_for_auto_promotion`.
- `test_dpo_consent_expiry_reverts_auto_promotion`.
- `test_lineage_query_reconstructable_for_promoted_bundle`.
- `test_art17_redaction_does_not_break_lineage_query`.
- `test_audit_chain_unbroken_after_force_promote`.
- `test_low_stakes_required_for_auto_promotion`.

---

## 27. Threat model M24 (delta vs M23)

### 27.1 New attack surfaces

| # | Attack | Severity | Mitigation |
|---|---|---|---|
| TM-24-01 | Adversarial labeling (poisoning gold dataset) | CRITICAL | Centroid drift threshold (max 0.30) aborts retune ; review queue catches obvious mislabels |
| TM-24-02 | Slow-poisoning (incremental misclabels under drift threshold) | HIGH | Quarterly bench gold validation by EFE compliance team ; cross-tenant comparison detects outliers |
| TM-24-03 | LLM oracle compromise (mass-mislabeling) | HIGH | Multi-provider rotation (M21) + canary (catches accuracy regression) |
| TM-24-04 | Drift evasion (poisoner targets non-holdout samples) | MEDIUM | Holdout split is hash-based deterministic — attacker can't predict which samples are holdout |
| TM-24-05 | Cost runaway (queue overflow attack) | MEDIUM | Queue max size + back-pressure + budget hard-cap |
| TM-24-06 | Replay attacks on review endpoints | HIGH | OIDC token freshness < 10 min + token hash storage |
| TM-24-07 | Insider rubber-stamping (reviewer fatigue) | MEDIUM | UI shows full diff + rationale required + 4-eyes optional + reviewer rotation |
| TM-24-08 | GDPR Art. 17 abuse (frivolous erasure to disrupt) | LOW | Identity verification on submission + rate limit per data subject |
| TM-24-09 | DPO consent forgery | CRITICAL | Cosign-signed consent document + Rekor inclusion proof |
| TM-24-10 | Holdout/training crossover (centroid leak) | MEDIUM | View-level enforcement + pin test parity |

### 27.2 Adversarial bench scenario (annexe D)

`adversarial_poisoning` bench :
- 1000 clean gold examples
- Inject 200 mislabeled examples gradually (over 4 weeks of "normal" sampling)
- Run retune
- Assert : centroid drift threshold catches it OR holdout eval flags accuracy drop
- 100 % detection target

---

## 28. Backend contract evolution (1.6 → 1.7)

### 28.1 Additions

| Type | Quantity | Détail |
|---|---:|---|
| New gRPC services | 1 | `efe.active_learning.v1.ActiveLearningService` |
| New RPCs in existing services | 0 | None (curation service unchanged) |
| New optional fields in ClassifyResponse | 1 | `bundle_retune_run_id` (links classify to current retune lineage) |
| New optional fields in TaxonomyVersion | 1 | `retune_run_id` (links promoted version back to its retune origin) |
| Removed/changed fields | 0 | STRICTLY ZERO |

### 28.2 SDK propagation

Re-gen stubs for v1.2.0 release :
- Python : `pip install efe-sdk[active-learning]`
- Go : new sub-package `learning/`
- TypeScript : `@efe/sdk@1.2.0` re-exports `ActiveLearning` namespace
- Java : new artifact `com.efe:sdk-active-learning`

### 28.3 Backward compat

- Pre-M24 clients ignore the 2 new fields (proto3 default).
- Pre-M24 portal doesn't have `/learning/*` (feature-gated routes).
- Pre-M24 MCP clients see 39 tools (M23) ; with M24 ON, see 43.

---

## 29. Bench + pin tests

### 29.1 Bench scenarios M24 (6 nouveaux)

```
crates/efe-bench/scenarios/active_learning/
├── e2e_full_loop.rs               # production → label → retune → promote (4 weeks simulation)
├── holdout_invariants.rs          # holdout/training disjoint + deterministic
├── centroid_recalibration.rs      # math correctness + drift threshold safety
├── promotion_safety.rs            # 100 % regression candidates rejected
├── gdpr_art17_cascade.rs          # erasure → tombstone → bundle rebuild → audit chain intact
└── adversarial_poisoning.rs       # 200 poisoned samples → 100% caught (drift OR holdout)
```

### 29.2 Bench thresholds (GA gate)

| Bench | Threshold | Direction |
|---|---|---|
| e2e_full_loop_latency_p99 | < 1 h gold-label-to-confirmed | ≤ |
| holdout_disjoint_pct | 100 % | = |
| centroid_recalibration_correctness | bit-for-bit match Python ref | = |
| promotion_safety_false_promote_rate | 0 / 100 | = |
| gdpr_art17_cascade_completion | < 30 days SLA | ≤ |
| gdpr_art17_audit_chain_intact | 100 % | = |
| adversarial_poisoning_detection_rate | ≥ 95 % | ≥ |

### 29.3 Pin tests inventory M24 (target +100)

| Module | Count | Examples |
|---|:---:|---|
| `efe-active-learning::sampler` | 8 | (cf. § 7.4) |
| `efe-active-learning::queue` | 10 | (cf. § 8.5) |
| `efe-active-learning::llm_worker` | 12 | (cf. § 9.6) |
| `efe-active-learning::review` | 8 | (cf. § 10.4) |
| `efe-active-learning::auto_confirm` | 6 | (cf. § 11.4) |
| `efe-active-learning::retune` | 12 | (cf. § 12.4) |
| `efe-active-learning::centroid` | 7 | (cf. § 13.5) |
| `efe-active-learning::holdout` | 6 | (cf. § 14.5) |
| `efe-active-learning::eval` | 5 | (cf. § 15.4) |
| `efe-active-learning::promotion` | 7 | (cf. § 16.3) |
| `efe-active-learning::gdpr_art17` | 6 | (cf. § 17.5) |
| `efe-active-learning::cost` | 5 | (cf. § 25.4) |
| Compliance | 6 | (cf. § 26.4) |
| Threat model adversarial | 4 | TM-24-01..04 |
| mcp-server tools | 6 | (cf. § 21.3) |
| CLI commands | 5 | (cf. § 22.3) |
| portal-admin UI | 6 | (cf. § 23.4) |
| Migration 0015 | 4 | trigger semantics + RLS |
| Multi-tenant isolation | 5 | (cf. § 24) |
| Backward compat | 3 | proto field additions |
| **Total** | **+131** | (oversubscribed vs +100 target — provides headroom) |

---

## 30. Activation flags + dependency validation

### 30.1 5-niveaux flags

```
LEVEL 1 — Server (env vars)
  EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED        (default false)
  EFE_FEATURE_M24_REQUIRE_M21_M23                (default true ; refuses boot if M24 ON without M21+M23)
  EFE_FEATURE_M21_LLMROUTER                      (must be true for M24)
  EFE_FEATURE_M23_CURATION_ENABLED               (must be true for M24)

LEVEL 2 — Tenant (DB)
  tenant_feature.m24_active_learning_enabled
  tenant_feature.m24_auto_promotion_enabled       (default false)
  tenant_feature.m24_human_review_required        (default true)
  tenant_feature.m24_dpo_consent_signed_at        (required for auto_promotion)
  tenant_feature.m24_uncertainty_threshold        (default 0.70)
  tenant_feature.m24_gold_label_threshold         (default 1000)
  tenant_feature.m24_retune_interval_days         (default 7)
  tenant_feature.m24_promotion_threshold_pp       (default 1.00)
  tenant_feature.m24_active_learning_budget_cents_per_day (default 0)

LEVEL 3 — Catalog (DB)
  catalog.m24_al_enabled                          (default false)
  catalog.m24_stakes_tier                         (default MEDIUM ; LOW required for auto_promotion)
  catalog.m24_drift_threshold_pp                  (default 2.00)

LEVEL 4 — Sample (per-row, runtime)
  - LLM confidence > tenant.auto_confirm_threshold
  - PII redactable
  - not in dedup window

LEVEL 5 — Per-call (RPC param)
  - TriggerBundleRetune.force_below_threshold (admin-only override)
```

### 30.2 Startup validation

```rust
pub fn validate_m24_dependencies() -> Result<(), StartupError> {
    let m24 = env_bool("EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED");
    if !m24 { return Ok(()); }  // OFF, no validation needed

    let require = env_bool_default("EFE_FEATURE_M24_REQUIRE_M21_M23", true);
    if !require {
        // explicit override — log warning
        tracing::warn!("M24 ON without M21/M23 dep validation — operator override");
        return Ok(());
    }
    let m21 = env_bool("EFE_FEATURE_M21_LLMROUTER");
    let m23 = env_bool("EFE_FEATURE_M23_CURATION_ENABLED");
    if !m21 || !m23 {
        return Err(StartupError::DependencyMissing(
            format!("M24 requires M21={} and M23={} (both must be true)", m21, m23)
        ));
    }
    Ok(())
}
```

### 30.3 Pin tests activation

- `test_m24_off_default`.
- `test_m24_on_without_m21_refuses_boot`.
- `test_m24_on_without_m23_refuses_boot`.
- `test_m24_override_logs_warning_but_proceeds`.
- `test_tenant_auto_promotion_blocked_without_dpo_consent`.
- `test_catalog_low_stakes_required_for_auto_promotion`.
- `test_per_call_force_below_threshold_admin_only`.

---

## 31. Sprint plan D74-D79

### 31.1 Vue d'ensemble (6 jours-ingénieur)

| Jour | Focus | Engineers actifs | LOC drop estim. | Pin tests |
|---|---|---|---:|---:|
| D74 | DB migration 0015 + crate skeleton + uncertainty sampler | 2 Rust + 1 SRE | ~1200 | +18 |
| D75 | Gold label queue + LLM worker + cost extension | 1 Rust + 1 ML | ~1500 | +20 |
| D76 | Human review extension + auto-confirm + GDPR Art. 17 hooks | 1 Rust + 1 Python | ~1300 | +18 |
| D77 | Bundle retune + centroid math + holdout split | 1 Rust + 1 ML | ~1400 | +18 |
| D78 | Two-stage eval + promotion gate + portal-admin pages | 1 Rust + 1 TS | ~1600 | +14 |
| D79 | CLI + bench + threat model + GA gate audit | 1 Rust + 1 SRE + lead | ~800 | +12 |

### 31.2 D74 — DB migration + sampler

**Cœur livraison** :
- Migration `0015_active_learning.sql` (et rollback)
- Crate `crates/efe-active-learning/` skeleton + Cargo.toml
- Modules : `lib.rs`, `error.rs`, `sampler.rs`, `metrics.rs`
- gRPC service skeleton (proto + tonic codegen wiring)
- Startup dependency validation

**Pin tests cible** : 18.

**Risques** :
- RLS bug en RDS prod — mitigation : test sur RDS dev cluster D74 soir.
- Trigger semantics différentes Postgres versions — mitigation : test sur 14, 15, 16.

### 31.3 D75 — Queue + LLM worker

**Cœur livraison** :
- `queue.rs` (Postgres queue, dequeue_batch with FOR UPDATE SKIP LOCKED)
- `llm_worker.rs` (drain loop, label_one)
- Redis stream mirror (optional, falls back to Postgres-only)
- Cost governance extension (`active_learning` budget category)
- Prompt template `label_classify_v1.md`

**Pin tests cible** : 20.

### 31.4 D76 — Review + auto-confirm + GDPR

**Cœur livraison** :
- `review.rs` (mirror M23 curation_review pattern)
- `auto_confirm.rs` (3-conditions gate)
- `gdpr.rs` (Art. 17 cascade)
- Notifier worker (email/Slack/PagerDuty)

**Pin tests cible** : 18.

### 31.5 D77 — Retune + centroid + holdout

**Cœur livraison** :
- `retune.rs` (orchestrator)
- `centroid.rs` (math : update with α blend + drift threshold)
- `holdout.rs` (deterministic hash split + view enforcement)
- Bundle artifact builder (reuses M18)
- Cosign signing wiring (R-100 pipeline)

**Pin tests cible** : 18.

### 31.6 D78 — Eval + promotion + UI

**Cœur livraison** :
- `eval.rs` (offline holdout + canary integration)
- `promotion.rs` (decision tree + auto-promote)
- portal-admin pages : queue, proposal, retunes, holdout, cost, dpo-consent
- Server actions + lib/learning/{api,grpc}.ts

**Pin tests cible** : 14.

### 31.7 D79 — CLI + bench + audit

**Cœur livraison** :
- `efe-admin learning {queue,proposal,retune,promote,gdpr,...}` CLI tree
- 6 bench scenarios in `crates/efe-bench/scenarios/active_learning/`
- Threat model adversarial tests (TM-24-01..10 instrumentation)
- GA gate audit pass
- CHANGELOG entry + runbook

**Pin tests cible** : 12.

### 31.8 Audit pass post-D79 (extra ½ jour)

Same chirurgical pattern as M19→M23 :
- LOC final inventory
- Cross-cutting check : audit chain unbroken, RLS active, Sigstore signing wired, dependency validation enforced
- Pin test count verified on CI
- Adversarial poisoning test (TM-24-01) green
- Compliance pack regen + verify
- CHANGELOG + tag `M24/v1.0.0`

---

## 32. Risk register

| ID | Risk | Likelihood | Impact | Mitigation | Owner |
|---|---|:---:|:---:|---|---|
| R-24-01 | Adversarial poisoning of gold dataset | MEDIUM | CRITICAL | Centroid drift threshold + review queue + adversarial bench | Security + ML |
| R-24-02 | Slow-poisoning evades drift threshold | LOW | HIGH | Quarterly EFE compliance team gold validation | ML + Compliance |
| R-24-03 | LLM oracle systematic bias | MEDIUM | HIGH | Multi-provider rotation + canary stage 2 catches | ML |
| R-24-04 | Reviewer fatigue → rubber-stamp | HIGH | HIGH | UI batch + 4-eyes + rotation policy + auto-confirm only low-stakes | Product |
| R-24-05 | Cost runaway via queue overflow | MEDIUM | MEDIUM | Queue max size + back-pressure + circuit breaker | SRE |
| R-24-06 | Centroid recompute math bug | LOW | CRITICAL | Property-based testing (proptest) + parity test vs Python ref | ML |
| R-24-07 | Holdout/training crossover | LOW | CRITICAL | Hash-based split + view-level enforcement + Rust↔SQL parity test | ML + DBA |
| R-24-08 | GDPR Art. 17 SLA breach | MEDIUM | CRITICAL | Force-promote workflow + priority queue + 24h SLA for catastrophic | Compliance + SRE |
| R-24-09 | Auto-promote on accidentally-medium-stakes catalog | LOW | HIGH | Compile-time gate + UI confirmation + audit trail | Product + Security |
| R-24-10 | DPO consent expiry not honored | LOW | HIGH | Daily cron checks expiry + auto-revert auto_promotion | Compliance |
| R-24-11 | Embedder model upgrade breaks centroid math | LOW | MEDIUM | Bundle versioning includes embedder model SHA ; mismatch → abort | ML |
| R-24-12 | Bundle retune scheduler lag (cron skipped) | LOW | LOW | Fallback : on-demand trigger via mcp tool + monitoring alert | SRE |

---

## 33. Open questions / decisions needed

### O-1 : Active learning on Mode-2/Mode-3 ?

**Question** : extend M24 loop to non-Mode-1 classifications ?

**Status** : NON en V1. Mode-2/3 use LLM-routed anchors that are not
deterministic centroids ; centroid recompute math doesn't apply.

**Decision attendue** : M25+ (potentially needs LoRA fine-tune).

### O-2 : Adaptive sampling sophistiqué (BALD, query-by-committee) ?

**Question** : replace simple threshold with BALD or QBC ?

**Status** : NON en V1. Cost/benefit unclear at our scale.

**Decision attendue** : T+90 jours post-GA, based on labeling efficiency
metrics from pilots.

### O-3 : Multi-LLM consensus labeling ?

**Question** : 3 LLMs vote majoritaire au lieu d'un seul ?

**Status** : NON en V1. Triples cost. Single-LLM with human review
is the V1 quality assurance mechanism.

**Decision attendue** : T+180 jours, gated on cost trends.

### O-4 : Active learning sur le pool de holdout itself ?

**Question** : si on update les centroids puis re-eval sur holdout, le
holdout devient out-of-distribution. Comment refresh le holdout sans
breaking the immutability invariant ?

**Status** : Holdout immuable post-capture (cf. § 14.4). Pour refresh,
solution : annual holdout rotation (10% retiré, 10% nouveau).

**Decision attendue** : ouvert ; documenté pour M25+.

### O-5 : Custom labeling prompts per tenant ?

**Question** : tenant_admin peut-il fournir un prompt custom ?

**Status** : NON en V1 (eval-gating impossible). Comme M23 — annexe O-5.

### O-6 : Cross-catalog gold sharing intra-tenant ?

**Question** : un tenant peut-il bénéficier d'un gold de catalogue A
pour eval bundle de catalogue B ?

**Status** : NON en V1. Catalogs sont isolés. M25 décidera selon feedback.

### O-7 : Partitioning des grosses tables (gold_label_queue, learning_audit) ?

**Question** : à partir de quel volume partitionner par tenant_id ?

**Status** : monitoring quotidien des table sizes. Trigger : > 100GB
par table → partitioning natif Postgres par hash(tenant_id, range)
modulo 16.

**Decision attendue** : ouvert ; cible année 2 si volume justifie.

### O-8 : Embedder upgrade strategy ?

**Question** : si l'embedder model lui-même change (e.g. v1 → v2),
comment migrer ?

**Status** : OUT-OF-SCOPE M24. M25+ ou EFE 2.0.

---

## 34. Success metrics + GA gate

### 34.1 SLOs M24

| ID | SLO | Cible |
|---|---|---|
| M24-AL-1 | Gold-label queue lag p99 | < 1 h |
| M24-AL-2 | Operator adoption (active tenants with M24 ON) | ≥ 20 % at T+90 j |
| M24-AL-3 | Drift detection MTTR | < 24 h |
| M24-AL-4 | Human-review SLA p95 | < 48 h |
| M24-AL-5 | Promotion accuracy delta (when promoted) | ≥ +1 pp |
| M24-AL-6 | Bundle retune frequency (active catalogs) | ≥ weekly |
| M24-AL-7 | GDPR Art. 17 SLA (standard) | < 30 days |
| M24-AL-8 | GDPR Art. 17 SLA (catastrophic) | < 24 h |
| M24-AL-9 | Adversarial poisoning detection | ≥ 95 % |
| M24-AL-PIN | Pin tests | +100 GREEN |
| M24-AL-COST | Avg active learning cost / tenant / month (pilots) | ≤ $50 |

### 34.2 GA gate critères (block ship if any fail)

- [ ] +100 pin tests GREEN sur CI (target oversubscribed à 131)
- [ ] All workers deployable (queue + LLM + retune + GDPR processor)
- [ ] Audit chain HMAC verifier passes on 10K-row test fixture
- [ ] Cosign signing of new retuned bundles verifiable
- [ ] portal-admin pages render for 3 pilot tenants
- [ ] Compliance pack regenerable with M24 additions (lineage query)
- [ ] DPIA delta signed off by DPO
- [ ] Adversarial poisoning bench GREEN (≥ 95 % detection)
- [ ] Bench thresholds met (cf. § 29.2)
- [ ] Documentation publié : runbook, lineage query, DPIA delta
- [ ] Migration 0015 reversible verified on staging cluster
- [ ] M21 + M23 dependency validation enforced (boot test)

### 34.3 Post-GA observability

Pendant 30j post-GA :
- Queue lag, retune success rate, promotion rate, false-promote rate
- LLM provider distribution + cost trends
- Reviewer SLA adherence
- GDPR Art. 17 request volume + completion timing

Alerte PagerDuty si :
- queue lag p99 > 2 h
- retune false_promote_rate > 1 %
- GDPR Art. 17 SLA > 80 % consumed without progress
- cost > 200 % budget for any tenant

---

## 35. Glossary M24+

| Terme | Définition |
|---|---|
| **Active learning** | Closed-loop ML : système identifie ses incertitudes, les fait labéliser, accumule, re-tune |
| **Uncertainty sampling** | Sélection de samples par confidence basse |
| **Gold dataset** | Set de paires (sentence, expected_class) servant de ground truth (M23 + M24) |
| **Holdout** | Subset 10% de gold_dataset réservé à eval, jamais touché en training |
| **Centroid** | Moyenne d'embeddings d'anchors d'une class (Mode-1) |
| **Re-tune** | Re-calcul des centroids depuis le gold dataset (NOT fine-tune neural) |
| **Promotion** | Activation d'un nouveau bundle après eval + canary |
| **Drift** | Dégradation accuracy entre old et new (positive = improvement) |
| **Auto-confirm** | LLM label accepté automatiquement (3 conditions cumulatives) |
| **Auto-promote** | New bundle activé automatiquement (4 conditions cumulatives) |
| **Force-promote** | Promotion immédiate (Art. 17 catastrophic seulement) |
| **Stakes tier** | Catégorie risque catalog : LOW (auto OK) / MEDIUM / HIGH (operator-only) |
| **DPO consent** | Signature DPO required pour activer auto-promotion |
| **Holdout split** | Hash-based deterministic 10% reservation |
| **Centroid drift** | Cosine distance between old and new centroid (safety threshold 0.30) |
| **Tombstone (Art. 17)** | Mask content but preserve row to maintain HMAC chain |
| **Bundle rebuild request** | Cascade GDPR Art. 17 → trigger retune excluding tombstoned examples |

---

## 36. References

### Standards & regulations
- EU AI Act 2024/1689 — https://eur-lex.europa.eu/eli/reg/2024/1689/oj
- GDPR (EU 2016/679) Art. 17 + Art. 22 — https://eur-lex.europa.eu/eli/reg/2016/679/oj
- ISO/IEC 27001:2022 A.8.32 (change management)
- AICPA SOC 2 CC8

### EFE internal docs
- [`EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md)
- [`EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`](EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md)
- [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md)
- [`EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md`](EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md)
- [`EFE_KERNELS_ARCHITECTURE.md`](EFE_KERNELS_ARCHITECTURE.md)
- [`EFE_RELEASE_V1_GA_ARCHITECTURE.md`](EFE_RELEASE_V1_GA_ARCHITECTURE.md)
- [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md)

### Research
- "Active Learning Literature Survey" (Settles, 2010)
- "Deep Bayesian Active Learning with Image Data" (Gal et al., 2017)
- "Power of Ensembles for Active Learning in Image Classification" (Beluch et al., 2018)
- "Coresets for Differentially Private K-Means Clustering and Applications to Privacy in Mobile Sensor Data" — relevance for Art. 17 cascade

### Libraries
- `sqlx` — runtime queries
- `petgraph` — DAG (reused from M23)
- `statrs` — KL divergence (reused from M23)
- `pgcrypto` — SHA-256 in Postgres (holdout split view)

---

## Annexe A — LLM labeling prompt template

### A.1 `label_classify_v1.md`

```markdown
---
name: label_classify_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, vllm, bedrock]
min_context_tokens: 2000
max_context_tokens: 8000
estimated_input_tokens: 800
estimated_output_tokens: 200
output_schema_version: 1
output_format: json
created: 2027-08-15
author: tech_lead
eval_pass: true
eval_gold_dataset: m24_label_v1_gold
eval_accuracy: 0.91
ab_test:
  control: label_classify_v1
  candidate: null
  traffic_split: 1.0
  promote_threshold_pp: 2.0
  observation_days: 7
deprecation: null
---

# System

You are a strict classifier oracle. Given a sentence and a known
catalog of classes, return the SINGLE class that best fits the
sentence, with a confidence in [0,1] and a 1-2 sentence rationale.

CRITICAL CONSTRAINTS :
1. The class_id MUST be one of the provided catalog classes (verbatim).
2. If you are uncertain, return confidence < 0.7 — do NOT guess high.
3. If the sentence does not fit any class, return the catalog's
   designated `_unknown` class with confidence < 0.5.
4. Output strict JSON only. No prose.

# User

## Catalog (only valid classes)

{{ catalog_classes_with_descriptions }}

## Sentence to label (PII-redacted)

> {{ sentence_redacted }}

## Production classifier verdict (for context, may be wrong)

- Predicted : `{{ proposed_class_id }}`
- Confidence : {{ observed_confidence }}

## Output schema

```json
{
  "class_id": "<one of the catalog classes>",
  "confidence": <float in [0,1]>,
  "rationale": "<1-2 sentence justification>"
}
```

Output ONLY the JSON.
```

---

## Annexe B — Cost model M24

### B.1 Per-call cost (USD cents)

| Provider × model | Input cost / 1K tokens | Output cost / 1K tokens | Avg labeling cost / sample |
|---|---:|---:|---:|
| Anthropic claude-3-haiku | $0.0025 | $0.0125 | ~$0.003 |
| Anthropic claude-3-5-sonnet | $0.030 | $0.150 | ~$0.030 |
| OpenAI gpt-4o-mini | $0.0015 | $0.006 | ~$0.002 |
| OpenAI gpt-4o | $0.025 | $0.100 | ~$0.025 |
| Vertex gemini-1.5-flash | $0.0019 | $0.0075 | ~$0.002 |
| vLLM (self-hosted) | $0 (compute only) | $0 | ~$0 |

### B.2 Per-tenant monthly estimate

Tier `pro` (5K classifies/day, 5 % low-confidence) :

| Item | Daily | Monthly |
|---|:---:|---:|
| Samples enqueued | ~250 | ~7.5K |
| LLM labeling (haiku) | ~250 calls | ~7.5K @ $0.003 = **$22.50** |
| Bundle retune compute | 1/week | ~$2 |
| Holdout eval compute | 1/week | ~$1 |
| **Total/tenant pro / month** | | **~$25.50** |

À 20% adoption × 80 tenants pilots = 16 tenants actifs × $25 = **~$400/month total**
LLM cost across the fleet (in haiku mode). En sonnet mode : ~$4000/month.

### B.3 Cost optimization levers

- A/B test cheaper providers (haiku vs gpt-4o-mini)
- Provider batch APIs (Anthropic batch = 50% discount)
- Cache labels by content_hash (idempotent re-labels reuse)
- Drift-triggered only retune (skip weekly when no change)
- vLLM self-hosted for high-volume tenants

---

## Annexe C — Holdout split invariants + math

### C.1 Hash-based deterministic uniformity

The SHA-256 distribution is uniform on output bytes. Taking
`digest[31] % 10` gives :
- ~10 % of samples in holdout
- ~90 % in training
- Standard error : ±0.32 % per 1000 samples

### C.2 Per-tenant per-catalog isolation

The hash includes `(tenant_id || catalog_id || gold_example_id)` so
the holdout split is independent across tenants and catalogs.

### C.3 Reproducibility test

```rust
#[test]
fn test_holdout_distribution_approximately_10pct() {
    let tenant = TenantId("test".into());
    let catalog = CatalogId("c1".into());
    let mut count_holdout = 0;
    let n = 10_000;
    for _ in 0..n {
        let id = uuid::Uuid::new_v4();
        if is_in_holdout(&tenant, &catalog, id) { count_holdout += 1; }
    }
    let pct = count_holdout as f64 / n as f64;
    assert!((0.085..0.115).contains(&pct),
            "expected ~10% holdout, got {:.4}", pct);
}
```

---

## Annexe D — Pin tests inventory M24

(Cf. § 29.3 pour le tableau global +131. Liste canonique dans
`bench_baselines/v1.7.0/baseline.json`.)

```json
{
  "version": "1.7.0",
  "milestone": "M24",
  "pin_tests": [
    "test_sampler_threshold_respected",
    "test_sampler_dedup_content_hash",
    "test_sampler_dedup_existing_gold",
    "test_sampler_back_pressure",
    "test_sampler_per_tenant_isolation",
    "test_sampler_disabled_catalog",
    "test_sampler_metrics_emitted",
    "test_sampler_handles_pii_unredactable",

    "test_dequeue_for_update_skip_locked_concurrent",
    "test_dequeue_status_transition_atomic",
    "test_dequeue_respects_batch_size",
    "test_dequeue_per_tenant_isolation",
    "test_queue_back_pressure_emits_metric",
    "test_unique_content_hash_within_tenant",
    "test_redis_stream_mirror_on_insert",
    "test_redis_disconnect_does_not_block_postgres",
    "test_failed_status_after_n_retries",
    "test_status_machine_invalid_transition_rejected",

    "test_llm_worker_drain_batch_size_respected",
    "test_llm_worker_unknown_class_rejected",
    "test_llm_worker_budget_blocked",
    "test_llm_worker_invalid_json_retried",
    "test_llm_worker_3_failures_marks_failed",
    "test_llm_worker_pii_redactor_called",
    "test_llm_worker_audit_chain_link",
    "test_llm_worker_per_tenant_rls",
    "test_llm_worker_metrics_per_outcome",
    "test_llm_worker_concurrent_safe",
    "test_llm_worker_idempotency_on_retry",
    "test_llm_worker_handles_provider_5xx",

    "test_review_decision_confirm_inserts_gold_example",
    "test_review_decision_edit_uses_final_class_id",
    "test_review_decision_reject_does_not_insert",
    "test_review_oidc_freshness_check",
    "test_review_4_eyes_optional_per_tenant",
    "test_notifier_emits_at_notify_after_hours",
    "test_notifier_escalates_at_sla_90pct",
    "test_notifier_emergency_at_sla_breach",

    "test_auto_confirm_blocked_for_high_stakes",
    "test_auto_confirm_blocked_without_dpo_consent",
    "test_auto_confirm_blocked_below_threshold",
    "test_auto_confirm_allowed_all_3_conditions",
    "test_auto_confirm_audit_event_type",
    "test_auto_confirm_inserts_gold_example_with_source_tag",

    "test_retune_skips_below_threshold",
    "test_retune_creates_new_bundle_with_patch_bump",
    "test_retune_excludes_holdout_from_training",
    "test_retune_eval_runs_on_holdout",
    "test_retune_flagged_only_if_delta_pp_ge_1",
    "test_retune_audit_chain_link",
    "test_retune_metrics_emitted_per_outcome",
    "test_retune_aborts_if_active_bundle_missing",
    "test_retune_concurrent_safe_via_advisory_lock",
    "test_scheduler_due_calculation",
    "test_scheduler_per_catalog_isolation",
    "test_retune_persists_eval_metrics_json",

    "test_centroid_alpha_1_returns_old",
    "test_centroid_alpha_0_returns_gold_mean",
    "test_centroid_alpha_07_blends_correctly",
    "test_centroid_below_min_gold_returns_old",
    "test_centroid_drift_exceeded_aborts",
    "test_centroid_dimension_mismatch_errors",
    "test_centroid_empty_gold_returns_old",

    "test_holdout_deterministic_same_inputs_same_result",
    "test_holdout_distribution_approximately_10pct",
    "test_holdout_per_tenant_isolation",
    "test_holdout_view_matches_rust_function",
    "test_holdout_immutable_post_retune",
    "test_holdout_excluded_from_training_centroid_compute",

    "test_eval_old_vs_new_on_same_holdout",
    "test_eval_per_class_metrics_computed",
    "test_eval_accuracy_delta_pp_correct",
    "test_eval_skips_canary_if_offline_fails",
    "test_eval_canary_reuses_m23_engine",

    "test_auto_promote_blocked_for_high_stakes",
    "test_auto_promote_blocked_for_medium_stakes",
    "test_auto_promote_blocked_without_dpo_consent",
    "test_auto_promote_blocked_when_human_review_required",
    "test_auto_promote_allowed_low_stakes_all_conditions",
    "test_promote_uses_canary_engine",
    "test_promote_audit_trail_records_decision_path",

    "test_art17_tombstone_preserves_hmac_chain",
    "test_art17_cascade_triggers_bundle_rebuild",
    "test_art17_priority_queue_orders_by_urgency",
    "test_art17_force_promote_skips_operator_gate",
    "test_art17_force_promote_still_runs_canary",
    "test_art17_audit_records_subject_id_and_request_id",

    "test_estimate_cost_below_budget_proceeds",
    "test_estimate_cost_exceeds_budget_blocks",
    "test_circuit_breaker_emits_metric",
    "test_daily_budget_reset_at_midnight_utc",
    "test_gdpr_force_promote_exempt_from_budget",

    "test_dpo_consent_required_for_auto_promotion",
    "test_dpo_consent_expiry_reverts_auto_promotion",
    "test_lineage_query_reconstructable_for_promoted_bundle",
    "test_art17_redaction_does_not_break_lineage_query",
    "test_audit_chain_unbroken_after_force_promote",
    "test_low_stakes_required_for_auto_promotion",

    "test_adversarial_poisoning_centroid_drift_caught",
    "test_adversarial_poisoning_holdout_eval_caught",
    "test_slow_poisoning_quarterly_validation_caught",
    "test_provider_compromise_canary_catches",

    "test_register_tools_off_by_default",
    "test_register_tools_on",
    "test_trigger_retune_force_requires_admin",
    "test_confirm_label_requires_reviewer_role",
    "test_gdpr_erasure_requires_tenant_admin",
    "test_list_queue_per_tenant_isolated",

    "test_cli_learning_queue_list",
    "test_cli_learning_proposal_confirm_requires_reason",
    "test_cli_learning_retune_trigger_dry_run",
    "test_cli_learning_promote_confirm_typed",
    "test_cli_learning_gdpr_erasure_admin_only",

    "test_queue_page_renders_pending_items",
    "test_proposal_card_confirm_dialog_validates_reason",
    "test_retune_run_card_shows_status_badge",
    "test_holdout_chart_renders_split_proportion",
    "test_dpo_consent_page_requires_tenant_admin",
    "test_back_pressure_banner_shown_at_capacity",

    "test_migration_0015_idempotent",
    "test_migration_0015_rollback_safe",
    "test_migration_0015_rls_active_after_apply",
    "test_migration_0015_triggers_enforce_state_machine",

    "test_rls_no_tenant_var_returns_empty",
    "test_cross_catalog_dequeue_denied",
    "test_pii_redact_called_before_label_llm",
    "test_holdout_disjoint_from_training",
    "test_audit_chain_per_tenant_isolated",

    "test_classify_response_includes_retune_run_id",
    "test_taxonomy_version_includes_retune_run_id",
    "test_pre_m24_clients_ignore_new_fields"
  ],
  "count": 131
}
```

---

## Annexe E — Compliance attestation chain (extension)

```
M24 adds 3 nodes to the M23 attestation graph :

   gold_example (M23) ──────────► holdout_split (M24)
                              │
                              └──► training_set (M24, view)
                                        │
                                        ▼
                              centroid_recompute (M24)
                                        │
                                        ▼
                              new_taxonomy_version (M23 chain)
                                        │
                                        ▼
                              cosign_signed_bundle (R-100)
                                        │
                                        ▼
                              ACTIVE in production
                                        │
                                        ▼
                              llm_call_audit (back to M19) — closes loop
```

Each node has a corresponding `learning_audit` row with HMAC chain.
External auditor reconstructs the chain via the lineage query (§ 26.3).

---

## Annexe F — Migration `0015_active_learning.sql` complète

(Voir § 18.1 pour le script SQL complet.)

---

**FIN — EFE M24 Architecture détaillée.**

> Document exécutable. Toute ambiguïté restante est listée en § 33
> (Open questions). Le sprint D74-D79 peut démarrer sur cette base.



