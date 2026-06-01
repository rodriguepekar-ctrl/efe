# Sprint M22 — Mode-3 multi-turn iterative reasoning : architecture

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M21 GA (Mode-2 LLM-routed anchor generation
>   livré, **604 pin tests GREEN** — 557 mcp-server + 24 efe-prompts +
>   23 efe-llm-router ; 10 défauts audit pass M21 corrigés ;
>   `efe-llm-router` microservice + 5 mcp-server tools + 3 Postgres
>   tables + EFE 1.4 contract bump livrés).
> **Default state** : **OFF par défaut**. **Requires M21 ON**. Aucun
>   tenant existant ne voit de changement après upgrade vers EFE 1.5
>   tant qu'aucun flag M22 n'est activé. 8-condition AND requis pour
>   activation effective (cf. § 17 + roadmap § F.7).
>
> **Pré-requis** : avoir lu :
> - [`EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`](EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md)
>   (M21, blueprint complet — Mode-3 réutilise tout le pipeline LLMRouter
>   + cost governor + cache + circuit breaker).
> - [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md)
>   (M19, observability OTel + audit chain HMAC — étendus à
>   `concept_stack_audit` en M22).
> - [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md)
>   (roadmap stratégique ; § 8 = high-level M22 scope ; § F = feature
>   flag specification normative).
> - **GDPR Art. 22** :
>   <https://gdpr-info.eu/art-22-gdpr/> — automated individual
>   decision-making + right to obtain human intervention.
> - **EU AI Act 2024/1689 Art. 14** :
>   <https://eur-lex.europa.eu/eli/reg/2024/1689/oj> — human oversight
>   requirements for high-risk AI systems.
> - **GDPR Art. 35** : Data Protection Impact Assessment (DPIA) —
>   updated for Mode-3 multi-turn reasoning.
> - **NIST AI RMF 1.0** : AI risk management for iterative decision
>   processes.
>
> **Mission M22** : transformer Mode-2 (single-turn LLM narrowing) en
> **Mode-3 (multi-turn iterative refinement)**. Là où Mode-2 a livré
> "1 LLM call → 3 anchors → swarm Mode-1", Mode-3 livre une boucle
> de raffinement itératif :
>
> ```
> turn 0 :  embed → kNN candidates → LLM narrows to 5 + reasoning trace
> turn 1+: if confidence < threshold :
>            swarm partial → LLM analyzes label_attempt + candidates
>            → "is this right ? if not, retry with anchors X, Y" → 2 anchors
> turn 2+: swarm(narrowed_2) → final_label, conf
> turn N : convergence (conf > threshold) OR max_turns reached
> ```
>
> Mode-3 est le **palier compliance** de la roadmap LLM. C'est lui qui
> rend EFE éligible aux cas d'usage **GDPR Art. 22 high-stakes**
> (insurance claim routing, healthcare triage, asylum processing) en
> capturant la **trace de raisonnement complète** (concept_stack)
> nécessaire au right-to-explanation.
>
> 1. **Backend Rust nouveau RPC `ClassifyMode3`** : multi-turn loop
>    avec convergence check + concept_stack tracking + audit chain
>    integration. Réutilise LLMRouter (M21) sans modification.
> 2. **`mode3_review_queue` table** : Postgres-backed pending human
>    reviews avec TTL configurable + admin endpoints
>    (approve/reject/list).
> 3. **5 nouveaux mcp-server tools** :
>    `efe_classify_mode3`, `efe_get_mode3_reviews`,
>    `efe_approve_mode3_review` (write+confirm),
>    `efe_reject_mode3_review` (write+confirm),
>    `efe_get_concept_stack`.
> 4. **EFE 1.5 contract bump** (semver-mineur, additif) : 1 nouveau
>    gRPC RPC + 4 admin REST endpoints (review queue CRUD).
> 5. **Stakes tier compliance gate** : `catalog.stakes_tier = high`
>    AND `tenant.human_review_required = false` → activation rejetée
>    par trigger Postgres (M21 pattern réutilisé).
> 6. **Opt-in everywhere**, default OFF — 8-condition AND
>    (helm + M21 active + server M22 + tenant.enabled + max_turns ≥ 1
>    + catalog.mode3_enabled + stakes_gate + request).
>
> **Pourquoi M22 maintenant** : M21 GA a validé le pattern
> "LLMRouter + Mode-2 + cost governance" sur 4 design partners
> (1 Q4 2026 pilot, 3 Q1 2027 GA). 2 d'entre eux ont remonté un
> besoin Mode-3 multi-turn pour cas où confidence Mode-2 < 0.7 reste
> récurrent (~12 % du trafic). Sans Mode-3, ces cas tombent en
> "Mode-1 fallback" qui retire la valeur LLM.
>
> **Volumétrie estimée** :
> - Crate `efe-server` extensions (Mode-3 RPC + concept_stack +
>   review queue) : **~1 800 LOC Rust** + ~700 LOC tests.
> - Crate `efe-llm-router` extensions (multi-turn helper + retry
>   chain) : **~400 LOC Rust** + ~200 LOC tests.
> - mcp-server M22 tools : **~900 LOC Python** + ~600 LOC tests.
> - Postgres migration : ~80 LOC SQL.
> - Helm chart updates : ~150 LOC YAML.
> - **Pin tests cumulés post-M22** : **~757** (vs 677 M21 → +80).
>
> **Backward compat 100 %** :
>   - Mode-1 + Mode-2 préservés bit-for-bit.
>   - EFE 1.4 clients restent compat avec EFE 1.5 server.
>   - Surface mcp-server M16-M21 inchangée fonctionnellement.
>   - Sans flag activé : RPC `ClassifyMode3` retourne `UNIMPLEMENTED`.
>   - Image Docker backend inchangée par défaut.
>
> **Compliance posture** :
>   - **GDPR Art. 22** : Mode-3 = automated decision-making with
>     significant effect ; audit chain HMAC capture le full
>     concept_stack pour right-to-explanation.
>   - **EU AI Act 2024/1689 Art. 14** : human oversight obligatoire
>     pour high-risk systems ; mode3_review_queue + per-tenant
>     `human_review_required` flag.
>   - **EU AI Act Art. 12** : record-keeping ; concept_stack persisté
>     7 ans dans audit chain.
>   - **Stakes-tier gate** : `high` → human_review_required forcé à
>     true.

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Vision & rationale](#2-vision--rationale)
3. [Scope & non-goals](#3-scope--non-goals)
4. [Catalogue M22 — vue d'ensemble](#4-catalogue-m22--vue-densemble)
5. [Architecture overview](#5-architecture-overview)
6. [Phase A (D65) — Concept stack + multi-turn loop](#6-phase-a-d65--concept-stack--multi-turn-loop)
7. [Phase B (D66) — GDPR Art. 22 compliance](#7-phase-b-d66--gdpr-art-22-compliance)
8. [Phase C (D67) — Mode-3 backend RPC + persistence](#8-phase-c-d67--mode-3-backend-rpc--persistence)
9. [Phase D (D68) — Human review workflow](#9-phase-d-d68--human-review-workflow)
10. [Phase E (D69) — mcp-server tools + audit pass M22](#10-phase-e-d69--mcp-server-tools--audit-pass-m22)
11. [New tools + resources + prompts](#11-new-tools--resources--prompts)
12. [JSON Schemas (forward-refs)](#12-json-schemas-forward-refs)
13. [Backend extensions (EFE 1.5 contract bump)](#13-backend-extensions-efe-15-contract-bump)
14. [Threat model M22 (delta vs M21)](#14-threat-model-m22-delta-vs-m21)
15. [Performance & SLOs M22](#15-performance--slos-m22)
16. [Compatibility matrix](#16-compatibility-matrix)
17. [Activation flags M22 (8-condition AND)](#17-activation-flags-m22-8-condition-and)
18. [Migration M21 → M22](#18-migration-m21--m22)
19. [Test plan (~80 nouveaux pin tests)](#19-test-plan-80-nouveaux-pin-tests)
20. [Deployment scenarios](#20-deployment-scenarios)
21. [Open questions](#21-open-questions)
22. [FAQ](#22-faq)
23. [Glossary M22](#23-glossary-m22)
24. [References](#24-references)

**Annexes**

- [A — Full JSON Schemas (5 tools + Mode-3 RPC)](#a--full-json-schemas)
- [B — Concept stack JSON schema](#b--concept-stack-json-schema)
- [C — Postgres mode3_review_queue + audit extension](#c--postgres-mode3_review_queue--audit-extension)
- [D — Pin tests inventory M22](#d--pin-tests-inventory-m22)
- [E — EFE 1.4 → 1.5 contract diff](#e--efe-14--15-contract-diff)
- [F — GDPR Art. 22 + AI Act Art. 14 compliance attestation](#f--gdpr-art-22--ai-act-art-14-compliance-attestation)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Livrer Mode-3 (multi-turn iterative refinement avec concept_stack) backend Rust + 5 mcp-server tools + EFE 1.5 contract bump + GDPR Art. 22 compliance gates. |
| **Durée** | 5 sprints (D65-D69), ~50 PM-W ≈ 3.5 mois calendaires équipe pic 3 ETP. |
| **Ressources** | 1 senior Rust (lead, Mode-3 backend + concept_stack) + 1 senior Rust (review queue + audit chain extension) + 1 senior Python (mcp-server tools + tests) + 0.5 SRE (Helm + Postgres migration) + 0.5 compliance (DPIA refresh + AI Act Art. 14 conformity) + 0.25 sécurité (threat model M22) + 0.25 doc. |
| **Budget** | ~$320 k incluant audit pass + EU AI Act conformity assessment refresh + DPO certification of human review SLA. |
| **Volumétrie code** | ~1 800 LOC Rust backend + ~400 LOC Rust router extension + ~900 LOC Python mcp-server + ~1 500 LOC tests + ~230 LOC infra. **Total : ~4 830 LOC**. |
| **Pin tests** | **+80 GREEN** (vs 677 M21 → ~757 cumulés). |
| **Backward compat** | 100 %. **Default OFF** (8-condition AND + requires M21 active). Sans flag activé, mcp-server M22 = M21 GA bit-for-bit. EFE 1.4 backend clients restent compat avec EFE 1.5 server. |
| **Risk profile** | **Medium** — réutilise massivement M21 (LLMRouter, cost governor, cache, circuit breaker) sans modification. Risques principaux : (1) convergence non-garantie (LLM peut osciller), (2) coût scale linéaire avec turns, (3) GDPR Art. 22 audit completeness, (4) human review SLA dérive. Mitigation : opt-in everywhere + max_turns hard cap + concept_stack validation + SLA monitoring + audit pass post-livraison. |
| **Compliance impact** | **GDPR Art. 22 strict applicability** : Mode-3 = automated decision-making with significant effect ; right-to-explanation via concept_stack ; human review obligatoire pour high-stakes. **AI Act Art. 14** : human oversight requirements for high-risk systems. **NIS2 Art. 21(g)** préservé. |
| **EFE contract bump** | **EFE 1.5** semver-mineur. 1 nouveau gRPC RPC + 4 admin REST endpoints. EFE 1.4 ↔ EFE 1.5 : compat. mcp-server M22 features 100 % opt-in. |
| **Strategic outcome** | M22 ouvre les **cas d'usage high-stakes** (insurance claim routing, healthcare triage, asylum case classification) pour EFE. Sans M22, ces cas restent inéligibles à cause de GDPR Art. 22. Différenciateur fort vs concurrence (OpenAI Moderation / AWS Comprehend Custom n'ont pas de concept_stack natif). |

---

## 2. Vision & rationale

### 2.1 Le saut "single-turn" → "multi-turn iterative refinement"

Mode-2 (M21 GA) classifie en **un seul tour LLM** :
1. Embed sentence (Mode-1 carry-over).
2. kNN top-K (default 10) anchors candidats.
3. **1 LLM call** : "narrow to 3".
4. swarm Mode-1 sur 3 anchors → label + confidence.

**Le compromis** : 1 LLM call = $0.0014 (Sonnet 4.5) / 600 ms p50.
Mais **12 % du trafic** (mesure 4 design partners post-M21 GA) reste
en confidence < 0.7 — Mode-2 n'a pas réduit la zone d'ambiguïté.

**Mode-3 (M22)** introduit un **raffinement itératif** :

```
Turn 0 :
  embed(sentence) → top-K=10 candidates
  LLM(sentence, 10) → narrow to 5 + "reasoning_trace"
  
Turn 1 (if convergence not reached) :
  swarm(sentence, 5) → label_attempt + conf_partial
  LLM(sentence, label_attempt, 5) → "is this right ?"
    → "no, actually retry with anchors X, Y" (concept refinement)
  
Turn 2 (if convergence not reached) :
  swarm(sentence, [X, Y]) → final_label + conf
  
Convergence check après chaque turn :
  - conf >= tenant.threshold → STOP, return final_label
  - turn_n == max_turns → STOP, return current best
  - LLM signals "uncertain" → escalate human review
```

Coût scale linéaire : 3 turns ≈ 3× coût Mode-2 = ~$0.0042 / classify.
Latence p50 ≈ 3 s (vs 600 ms Mode-2). Mais **qualité substantiellement
meilleure** sur les 12 % de cas ambigus (mesure attendue post-livraison :
+5-10 pp accuracy).

### 2.2 Pourquoi Mode-3 maintenant (post-M21 GA)

**Trois conditions sont réunies** (Q1 2027) :

1. **Mode-2 GA validé** : 4 design partners ont activé Mode-2 ;
   12 % de leur trafic reste en zone d'ambiguïté Mode-2. Sans
   Mode-3, ces cas tombent en "Mode-1 fallback" qui retire la
   valeur LLM (raison principale de l'activation).

2. **GDPR Art. 22 high-stakes demand** : 3 design partners
   (insurance, healthcare, gov) ont des cas d'usage qui exigent
   right-to-explanation strict. Concept_stack persisté HMAC-chained
   sur 7 ans est l'argument vendeur définitif.

3. **AI Act Art. 14 maturity** : règlement entré en vigueur Q3 2026
   (high-risk systems). Implémentations conformes prennent 6 mois.
   M22 livre Q1 2027 = fenêtre de leadership.

### 2.3 4 patterns d'usage observés (post-M21 design partner pilots)

Sources : 6 entretiens design partners + DPO + ethics committee
post-M21 GA (2026-2027 transition).

**Pattern A — « Insurance claim routing »** (1/6 — financial services) :
classify ticket support → "fraud_review" vs "standard_processing" vs
"vip_escalation". Mode-1 baseline : 78 % accuracy. Mode-2 : 84 %
(+6 pp). **Mode-3 expected** : 89-92 % (+5-8 pp). Critique car
mauvais routage = customer churn + regulatory exposure.

**Pattern B — « Healthcare triage »** (1/6 — clinical SaaS) :
classify patient message → "urgent_call_back" vs "schedule_appointment"
vs "self_care_advice". GDPR Art. 22 strict — patient peut demander
right-to-explanation. **Mode-3 mandatory** car single-turn explicabilité
insuffisante pour DPO healthcare.

**Pattern C — « Compliance officer review »** (2/6) : DPO veut
voir la trace complète de raisonnement avant de signer la conformité
trimestrielle. Mode-2 audit log = "label X with conf 0.87 via
provider Y". Mode-3 audit = "turn 0: candidates [...], LLM said [...] ;
turn 1: swarm tried [...], LLM corrected to [...] ; turn 2: convergence at conf 0.91".

**Pattern D — « Adversarial robustness »** (2/6 — security/legal) :
multi-turn aide à détecter les prompt injection : si turn 0 + turn 1
divergent (LLM contradicts itself), c'est un signal fort. Permet de
flagger le ticket pour review manuelle.

### 2.4 Le compromis convergence × coût × latence

| Mode | p50 latency | p99 latency | $/classify | Use case |
|---|---:|---:|---:|---|
| Mode-1 | 200 ms GPU | 500 ms | $0.000005 | Bulk, high-volume |
| Mode-2 (M21) | 600 ms | 2 s | $0.0014 | Confidence-driven escalation |
| **Mode-3 (M22)** | **3 s** | **6 s** | **$0.0042** (3 turns) | **High-stakes, GDPR Art. 22** |

**Choix opérateur** : per-catalog config `mode3_enabled` + `max_turns`
(default 3, max 5).
- `low` stakes_tier : Mode-3 désactivé typiquement (Mode-2 suffit).
- `medium` stakes_tier : Mode-3 opt-in avec `max_turns=2` (quick
  refinement).
- `high` stakes_tier : Mode-3 obligatoire pour les cas low-confidence,
  `max_turns=3-5`, `human_review_required=true` HARD GATE.

### 2.5 GDPR Art. 22 — pourquoi Mode-3 change la donne

**Mode-2 single-turn** : le concept de "reasoning" est implicite.
L'audit log capture `provider`, `model`, `tokens`, `cost`,
`anchors_used`. Mais "pourquoi anchors X plutôt que Y" est invisible.

**Mode-3 multi-turn** : chaque turn produit une `llm_reasoning` (≤ 1 KB
free-form text) + `swarm_label_attempt` + `swarm_confidence` +
`candidates_in/out`. Le concept_stack complet = trace de raisonnement
**reproductible et explicable**.

**GDPR Art. 22(3)** : "the data subject shall have the right to
obtain human intervention on the part of the controller, to express
his or her point of view and to contest the decision". Mode-3
livre :

1. **Decision trace** : concept_stack jsonb persisté HMAC-chained.
2. **Human intervention path** : `mode3_review_queue` table + admin
   workflow (approve/reject) + per-tenant SLA configurable.
3. **Right to contest** : tenant peut requester `efe_get_concept_stack`
   par audit_id, expose la chaîne complète au data subject via DPO.

**AI Act Art. 14** : "high-risk AI systems shall be designed and
developed in such a way that they can be effectively overseen by
natural persons during the period in which the AI system is in use".
Mode-3 livre :

1. **Configurable max_turns** : opérateur définit le bornage du
   raisonnement automatisé.
2. **Stakes tier hard gate** : `high` force human_review_required.
3. **Review queue UI** (mcp tool): operator approve/reject avec SLA.

### 2.6 Pourquoi Mode-3 réutilise Mode-2 sans le modifier

**Décision cardinale M22** : Mode-3 = N appels Mode-2 chaînés. Le
LLMRouter, cost governor, cache, circuit breaker, audit chain (M21)
sont **inchangés**. M22 ajoute :
- Un **wrapper backend** qui orchestre N appels Mode-2 séquentiels.
- Un **convergence checker** qui décide STOP/CONTINUE après chaque
  turn.
- Une **table mode3_review_queue** pour human review.
- Un **concept_stack jsonb** persisté en `llm_call_audit` (extension
  de la table M21).

Avantages :
- **Risk minimization** : aucun bug introduit dans LLMRouter (testé
  M21 GA).
- **Cost transparency** : 3 turns = 3 entries `llm_call_audit` →
  audit chain naturelle + cost = sum(turn.cost).
- **Cache reuse** : turns 1+ peuvent hit le cache si des candidates
  similaires re-apparaissent.

---

## 3. Scope & non-goals

### 3.1 In scope (livraison M22)

**Backend Rust** :
- Crate `efe-server` extension : nouveau RPC `ClassifyMode3` qui
  orchestre N appels Mode-2 + convergence check + concept_stack
  builder.
- Crate `efe-server` extension : 4 admin REST endpoints pour review
  queue (list, get, approve, reject).
- Crate `efe-llm-router` extension : nouveau prompt template
  `mode3_refine_v1.md` pour les turns 1+ (LLM analyse
  label_attempt + propose anchors raffinés).
- Postgres migration : `mode3_review_queue` table + extension de
  `llm_call_audit` avec `concept_stack_jsonb` (JSONB column).
- Cron job : monthly cleanup `mode3_review_queue` rows expirées
  (review_token TTL 7 jours par défaut).

**mcp-server tools** (5 NEW) :
- `efe_classify_mode3` (read, agent-native) : delegate to backend.
- `efe_get_mode3_reviews` (read, agent-native) : list pending reviews
  for tenant (paginated, filtered by status).
- `efe_approve_mode3_review` (write, agent-native, confirm:true).
- `efe_reject_mode3_review` (write, agent-native, confirm:true).
- `efe_get_concept_stack` (read, agent-native) : retrieve full
  reasoning trace by audit_id (right-to-explanation API).

**mcp-server resources** (2 NEW) :
- `efe://mode3/reviews/pending` (tenant-scoped, read-only).
- `efe://mode3/sla/dashboard` (admin-only, read-only) : SLA stats
  per-catalog.

**mcp-server prompts** (1 NEW) :
- `mode3_refine_v1.md` (versioned template for turn 1+ refinement).

**EFE 1.5 contract** :
- 1 new gRPC RPC (`ClassifyMode3`).
- 4 new admin REST endpoints.
- Backward compat with EFE 1.4 verified via `m22_contract.rs` tests.

**Activation flags** (cf. roadmap § F.7) :
- 1 helm-level flag (`mode3.enabled`).
- 3 server-level env-vars.
- 5+ tenant-level config flags (DB jsonb).
- 3 catalog-level columns.
- 1 request-level RPC param.
- Default OFF everywhere.
- **Requires M21 active** (validation startup).

**Compliance** :
- DPIA addendum `docs/compliance/m22_dpia_addendum.md`.
- AI Act Art. 14 conformity assessment update
  `docs/compliance/m22_ai_act_art14_conformity.md`.
- Updated `docs/compliance/gdpr_art22.md` § Mode-3.
- Updated `docs/compliance/soc2_evidence.md` § automated decision-making.
- Updated `docs/compliance/iso27001.md` § A.12.1 (operational
  procedures + responsibilities).

**Operator documentation** :
- `docs/m22_operator_guide.md` (activation, review workflow, SLA).
- `docs/m22_dpo_guide.md` (right-to-explanation export procedure).
- `docs/m22_migration_guide.md` (M21 → M22).
- 4 new runbooks dans `docs/runbooks/` (cf. § 15).

### 3.2 Out of scope (explicitement)

| Hors scope | Pourquoi | Quand |
|---|---|---|
| Mode-4 (chained classifies) | Beyond multi-turn ; needs design | Never (out of EFE charter) |
| LLM-driven taxonomy curation | Modifies catalog state | M23 |
| Active learning loop | Pipeline trainings | M24 |
| Agent runtime in-cluster | Sandboxing K8s CRDs | M25 |
| Real-time concept_stack streaming | Out of M22 scope | M23+ |
| Multi-modal Mode-3 (images) | EFE classify text-only | Never |
| Tenant-customisable refinement prompts | Hard to validate | M23+ |
| Concept_stack in MCP sampling response | Already in audit chain | Never (security) |
| Cross-catalog Mode-3 (federated reasoning) | Scope creep | Never |
| Concept_stack visualization UI | Customer concern, not EFE | Never |
| Mode-3 over MCP sampling delegation | Backend-only by design | Never |

### 3.3 Hard constraints

**C1 — M21 active required** :
- `EFE_FEATURE_M22_MODE3_ENABLED=true` AND
  `EFE_FEATURE_M21_MODE2_ENABLED=true` are BOTH required.
- Server startup validation : refuse boot si M22 ON sans M21.
- Test pin : `test_m22_requires_m21_active.rs`.

**C2 — Mode-1 + Mode-2 SLO préservés** :
- D9-LOAD-3 p99 ≤ 200 ms GPU sur Mode-1 reste valide.
- M21-MODE2-1 p50 < 600 ms sur Mode-2 reste valide.
- Mode-3 dans thread pool dédié ; n'impact pas Mode-1/2.

**C3 — max_turns hard cap** :
- Per-tenant `max_turns ≤ 5` (DB CHECK constraint).
- Per-request `max_turns_override ≤ tenant.max_turns`.
- Hard server-side enforcement.

**C4 — Stakes tier compliance gate** :
- `catalog.stakes_tier = high` AND
  `tenant_feature_mode3.human_review_required = false` →
  Postgres trigger rejects activation.
- Non-bypassable.

**C5 — Concept_stack persistence** :
- Each Mode-3 call → 1 row in `llm_call_audit` with
  `concept_stack_jsonb` populated.
- HMAC-chained (M14-bis carry-over).
- 7-year retention (GDPR + AI Act).

**C6 — Convergence non-guaranteed** :
- Mode-3 returns `converged: false` if max_turns reached without
  conf >= threshold.
- Caller (mcp-server / agent) receives flag + can route to human
  review.

**C7 — Cost transparency** :
- `total_cost_usd` = sum(turn.cost_usd) reported in response.
- Charged to tenant's monthly_cap_usd as N independent Mode-2 calls.

**C8 — Backend-only orchestration** :
- Mode-3 multi-turn loop runs in `efe-server` Rust, NOT in
  mcp-server Python.
- Avoids cross-process latency × N turns.

---

## 4. Catalogue M22 — vue d'ensemble

### 4.1 Outils mcp-server (49 cumulés au final, +5 NEW si M22 enabled)

| Tool | Profile | Write? | Status M22 | Feature flag |
|---|---|:---:|:---:|---|
| (44 outils M16-M21) | (varies) | varies | ✅ unchanged | (M20+M21 helpers gated) |
| `efe_classify_mode3` | agent-native | No | 🆕 conditional | M22_MODE3_ENABLED |
| `efe_get_mode3_reviews` | agent-native | No | 🆕 conditional | M22_MODE3_ENABLED |
| `efe_approve_mode3_review` | agent-native | **Yes** | 🆕 conditional | M22_MODE3_ENABLED |
| `efe_reject_mode3_review` | agent-native | **Yes** | 🆕 conditional | M22_MODE3_ENABLED |
| `efe_get_concept_stack` | agent-native | No | 🆕 conditional | M22_MODE3_ENABLED |

### 4.2 Resources mcp-server (16 cumulés au final, +2 NEW)

| Resource URI | Profile | Status M22 |
|---|---|:---:|
| (14 resources M16-M21) | varies | ✅ unchanged |
| `efe://mode3/reviews/pending` | agent-native | 🆕 conditional |
| `efe://mode3/sla/dashboard` | agent-native-deploy | 🆕 conditional |

### 4.3 Concept stack data structure

```json
{
  "concept_stack": [
    {
      "turn": 0,
      "candidates_in": ["billing", "refund", "payment_dispute", "..."],
      "candidates_out": ["payment_dispute", "billing", "refund", "fraud_review", "vip"],
      "llm_reasoning": "The sentence mentions 'unauthorized charge' which strongly suggests payment_dispute or fraud_review. Bill amount is ambiguous between billing and refund.",
      "swarm_label_attempt": null,
      "swarm_confidence": null,
      "cost_usd": 0.00140,
      "duration_ms": 587,
      "provider_used": "anthropic:claude-sonnet-4-5"
    },
    {
      "turn": 1,
      "candidates_in": ["payment_dispute", "billing", "refund", "fraud_review", "vip"],
      "candidates_out": ["fraud_review", "payment_dispute"],
      "llm_reasoning": "swarm tried 'billing' but conf=0.62 ; with the 'unauthorized' keyword, fraud_review is more aligned. Narrowing to 2.",
      "swarm_label_attempt": "billing",
      "swarm_confidence": 0.62,
      "cost_usd": 0.00135,
      "duration_ms": 542,
      "provider_used": "anthropic:claude-sonnet-4-5"
    },
    {
      "turn": 2,
      "candidates_in": ["fraud_review", "payment_dispute"],
      "candidates_out": [],
      "llm_reasoning": null,
      "swarm_label_attempt": "fraud_review",
      "swarm_confidence": 0.91,
      "cost_usd": 0.0,
      "duration_ms": 187,
      "provider_used": null
    }
  ],
  "total_turns": 3,
  "total_cost_usd": 0.00275,
  "converged": true,
  "final_label": "fraud_review",
  "final_confidence": 0.91
}
```

### 4.4 New backend RPCs (EFE 1.5 contract)

| RPC | Service | Type | Status M22 |
|---|---|:---:|:---:|
| `ClassifyMode3(ClassifyMode3Request) → ClassifyMode3Response` | EfeSwarmService | Unary | 🆕 |
| `ListMode3Reviews(ListReviewsRequest) → ReviewList` | EfeAdminService | Unary | 🆕 |
| `GetMode3Review(GetReviewRequest) → ReviewDetail` | EfeAdminService | Unary | 🆕 |
| `ApproveMode3Review(ReviewActionRequest) → Empty` | EfeAdminService | Unary | 🆕 |
| `RejectMode3Review(ReviewActionRequest) → Empty` | EfeAdminService | Unary | 🆕 |

### 4.5 New Postgres tables

```sql
-- Migration 011_m22_mode3_review_queue.sql
CREATE TABLE mode3_review_queue (
    review_token       TEXT PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    audit_id           BIGINT NOT NULL REFERENCES llm_call_audit(id),
    catalog            TEXT NOT NULL,
    bundle_version     TEXT NOT NULL,
    sentence_hash      TEXT NOT NULL,
    proposed_label     TEXT NOT NULL,
    proposed_confidence REAL NOT NULL CHECK (proposed_confidence BETWEEN 0 AND 1),
    concept_stack      JSONB NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    requested_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at         TIMESTAMPTZ NOT NULL,
    reviewed_at        TIMESTAMPTZ,
    reviewed_by        TEXT,
    review_notes       TEXT,
    final_label        TEXT
);

CREATE INDEX ix_mode3_review_queue_status_expires
    ON mode3_review_queue(status, expires_at)
    WHERE status = 'pending';

CREATE INDEX ix_mode3_review_queue_tenant_status
    ON mode3_review_queue(tenant_id, status, requested_at DESC);

-- Migration 012_m22_extend_llm_call_audit.sql
ALTER TABLE llm_call_audit
ADD COLUMN concept_stack_jsonb JSONB,
ADD COLUMN total_turns INTEGER NOT NULL DEFAULT 1
                       CHECK (total_turns >= 1 AND total_turns <= 10),
ADD COLUMN converged BOOLEAN NOT NULL DEFAULT TRUE;

CREATE INDEX ix_llm_call_audit_concept_stack
    ON llm_call_audit USING GIN (concept_stack_jsonb)
    WHERE concept_stack_jsonb IS NOT NULL;
```

### 4.6 Stakes tier model

| Stakes tier | Default behaviour | Compliance |
|---|---|---|
| `low` | Mode-3 typically off ; if on, `human_review_required` defaults to false | GDPR Art. 22(2)(a) — "necessary for entering into ... contract" exception applicable |
| `medium` | Mode-3 opt-in ; `human_review_required` defaults to false but tenant can override | GDPR Art. 22(2)(c) — explicit consent path |
| **`high`** | **Mode-3 mandatory ; `human_review_required` HARD GATE = true** | GDPR Art. 22(3) full applicability + AI Act Art. 14 high-risk |

---

## 5. Architecture overview

### 5.1 Component diagram

```
┌────────────────────────────────────────────────────────────────────┐
│                       MCP CLIENT (operator)                        │
│              Claude Desktop / Cursor / agent custom                │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                               │ JSON-RPC 2.0 (HTTP+SSE M19)
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│                         mcp-server (M22)                           │
│                                                                    │
│  ┌──────────────────────────────────────────────────────┐         │
│  │ Dispatcher (M16 + M20+M21+M22 feature_flag gates)    │         │
│  │  ─ tool registry (49 tools max, 5 M22 if flag ON)    │         │
│  └────────────────────────────┬─────────────────────────┘         │
│                               │                                    │
└───────────────────────────────┼────────────────────────────────────┘
                                │
                                │ gRPC ClassifyMode3
                                ▼
┌────────────────────────────────────────────────────────────────────┐
│                       efe-server backend (Rust)                    │
│                                                                    │
│  ┌──────────────────────────────────────────────────────┐         │
│  │ services/swarm.rs                                     │         │
│  │  ─ Mode-1 RPC (M16, unchanged)                        │         │
│  │  ─ ClassifyMode2 RPC (M21, unchanged)                 │         │
│  │  ─ ClassifyMode3 RPC (NEW M22)                        │         │
│  │       Loop : turn 0 → turn N (max 5)                  │         │
│  │         └─ delegates to LLMRouter (M21, unchanged)    │         │
│  │         └─ convergence check after each turn          │         │
│  │         └─ concept_stack append                       │         │
│  │       1 row inserted in llm_call_audit per turn       │         │
│  │       1 final row with full concept_stack_jsonb       │         │
│  │       If max_turns hit + not converged + stakes_tier  │         │
│  │         high : mode3_review_queue insertion           │         │
│  └────────────────────────────┬─────────────────────────┘         │
│                               │                                    │
│  ┌──────────────────────────────────────┐                         │
│  │ Postgres                             │                         │
│  │  ─ catalog (existing)                │                         │
│  │  ─ tenant_llm_budget (M21)           │                         │
│  │  ─ tenant_llm_config (M21)           │                         │
│  │  ─ llm_call_audit (M21 + M22 cols)   │                         │
│  │  ─ mode3_review_queue (NEW M22)      │                         │
│  └──────────────────────────────────────┘                         │
└──────────────────────────────────────────────────────────────────-─┘
                                │
                                │ gRPC LLMComplete (M21 contract, unchanged)
                                ▼
┌────────────────────────────────────────────────────────────────────┐
│              efe-llm-router microservice (M21, unchanged)         │
│                                                                    │
│  Templates servis :                                                 │
│   - mode2_anchor_gen_v1 (turn 0 narrowing 10 → 5)                  │
│   - mode3_refine_v1 (NEW M22, turn 1+ refinement)                  │
└────────────────────────────────────────────────────────────────────┘
```

### 5.2 Mode-3 flow step-by-step

```
                                  client.ClassifyMode3(
                                      sentence="...",
                                      catalog="...",
                                      max_turns=3,
                                      threshold=0.85
                                  )
                                            │
                                            ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 1 — efe-server : Auth + tenant + stakes_tier           │
   │   ─ JWT bearer → tenant_id (D15)                             │
   │   ─ Reject if M22 flags off OR stakes_tier compliance fail   │
   │   ─ Latency : ~0.5 ms                                        │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 2 — efe-server : Initialise concept_stack + budget     │
   │   ─ concept_stack = []                                       │
   │   ─ budget pre-flight for max_turns × estimated_cost         │
   │   ─ Latency : ~5 ms                                          │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼ TURN 0
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 3 — Turn 0 : Mode-2 with template_id=mode2_anchor_gen  │
   │   1. embed sentence (Mode-1 carry-over)                      │
   │   2. kNN top-K=10 candidates                                 │
   │   3. LLM call : "narrow 10 → 5"                              │
   │   4. Append to concept_stack                                 │
   │   5. Latency : ~600 ms                                       │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼ TURN 1
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 4 — Turn 1 : Partial swarm + LLM refine                │
   │   1. swarm Mode-1 on top-5 → label_attempt + conf_partial    │
   │   2. If conf_partial >= threshold → CONVERGED, STOP          │
   │   3. Else : LLM call template_id=mode3_refine_v1             │
   │      "review label_attempt, narrow 5 → 2"                    │
   │   4. Append to concept_stack                                 │
   │   5. Latency : ~700 ms                                       │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼ TURN 2 (final)
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 5 — Turn 2 : Final swarm                               │
   │   1. swarm Mode-1 on top-2 → final_label + conf              │
   │   2. Append to concept_stack (no LLM call this turn)         │
   │   3. Latency : ~200 ms                                       │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼ Convergence check
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 6 — Final convergence check                            │
   │   - converged = (conf >= threshold) OR (stakes_tier=low)     │
   │   - If NOT converged AND stakes_tier=high :                  │
   │       → INSERT INTO mode3_review_queue (review_token,        │
   │           concept_stack, expires_at = now() + 7d)            │
   │       → response.requires_human_review = true                │
   │   - Latency : ~5-10 ms                                       │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼ Audit + return
   ┌──────────────────────────────────────────────────────────────┐
   │ Phase 7 — Final audit chain append                           │
   │   - llm_call_audit row with mode='MODE_3',                    │
   │     concept_stack_jsonb = full stack,                        │
   │     total_turns, converged, final cost                       │
   │   - HMAC chain (M14-bis carry-over)                          │
   │   - Latency : ~5 ms                                          │
   └──────────────────────┬───────────────────────────────────────┘
                          │
                          ▼
                 ClassifyMode3Response{
                   final_label: "fraud_review",
                   final_confidence: 0.91,
                   concept_stack: [turn0, turn1, turn2],
                   total_turns: 3,
                   total_cost_usd: 0.00275,
                   converged: true,
                   requires_human_review: false,
                   review_token: null
                 }

  Total typical p50 latency : ~3 s (3 turns × 700-1000 ms each).
  Convergence at turn 1 : ~1.2 s.
  Max-turns reached + human review : up to 6 s p99.
```

### 5.3 Convergence semantics

```rust
fn convergence_check(
    swarm_confidence: Option<f32>,
    threshold: f32,
    turn: u32,
    max_turns: u32,
) -> ConvergenceResult {
    if let Some(conf) = swarm_confidence {
        if conf >= threshold {
            return ConvergenceResult::Converged { conf };
        }
    }
    if turn >= max_turns {
        return ConvergenceResult::MaxTurnsReached;
    }
    ConvergenceResult::Continue
}

enum ConvergenceResult {
    Converged { conf: f32 },     // STOP, return final
    MaxTurnsReached,             // STOP, may queue for review
    Continue,                    // next turn
}
```

### 5.4 Concept stack lifecycle

1. **Created** : empty `[]` at request start.
2. **Appended** : 1 entry per turn (always — even if no LLM call this
   turn).
3. **Persisted** : full stack in `llm_call_audit.concept_stack_jsonb`
   (final row only ; intermediate turns stored in audit chain too
   for cost tracking, but `concept_stack_jsonb` is a denormalised
   convenience copy).
4. **Reviewed** : if `mode3_review_queue` insertion happens, full
   stack copied into `mode3_review_queue.concept_stack`.
5. **Retrieved** : `efe_get_concept_stack(audit_id)` returns the
   full stack from audit chain (right-to-explanation API).

---

## 6. Phase A (D65) — Concept stack + multi-turn loop

### 6.1 ConceptStack data model

```rust
// crates/efe-server/src/mode3/concept_stack.rs

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptStackEntry {
    pub turn: u32,
    pub candidates_in: Vec<String>,
    pub candidates_out: Vec<String>,
    pub llm_reasoning: Option<String>,    // ≤ 1024 chars (truncated server-side)
    pub swarm_label_attempt: Option<String>,
    pub swarm_confidence: Option<f32>,
    pub cost_usd: f64,
    pub duration_ms: u32,
    pub provider_used: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptStack {
    pub entries: Vec<ConceptStackEntry>,
    pub final_label: Option<String>,
    pub final_confidence: Option<f32>,
    pub converged: bool,
    pub total_cost_usd: f64,
}

impl ConceptStack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            final_label: None,
            final_confidence: None,
            converged: false,
            total_cost_usd: 0.0,
        }
    }

    pub fn append(&mut self, entry: ConceptStackEntry) {
        self.total_cost_usd += entry.cost_usd;
        self.entries.push(entry);
    }

    pub fn finalize(&mut self, label: String, confidence: f32, converged: bool) {
        self.final_label = Some(label);
        self.final_confidence = Some(confidence);
        self.converged = converged;
    }

    pub fn total_turns(&self) -> u32 {
        self.entries.len() as u32
    }
}
```

### 6.2 Per-turn iteration logic

```rust
// crates/efe-server/src/mode3/loop.rs

use crate::mode3::concept_stack::{ConceptStack, ConceptStackEntry};
use efe_llm_router::{LLMRouter, RouterRequest};

const MAX_TURNS_HARD_CAP: u32 = 5;
const LLM_REASONING_MAX_CHARS: usize = 1024;

pub struct Mode3Engine {
    router: Arc<LLMRouter>,
    swarm: Arc<SwarmEngine>,
    audit: Arc<AuditChain>,
}

impl Mode3Engine {
    pub async fn classify_mode3(
        &self,
        request: ClassifyMode3Request,
        tenant_config: &TenantMode3Config,
    ) -> Result<ClassifyMode3Response, Mode3Error> {
        let max_turns = request
            .max_turns
            .unwrap_or(tenant_config.default_max_turns)
            .min(tenant_config.max_max_turns)
            .min(MAX_TURNS_HARD_CAP);
        let threshold = request
            .threshold
            .unwrap_or(tenant_config.default_threshold);

        let mut stack = ConceptStack::new();

        // Turn 0 — initial narrowing (10 → 5 candidates)
        let turn0_candidates = self.embed_and_knn(&request, 10).await?;
        let turn0_narrowed = self
            .llm_narrow(&request, &turn0_candidates, "mode2_anchor_gen_v1", 5)
            .await?;
        stack.append(ConceptStackEntry {
            turn: 0,
            candidates_in: turn0_candidates,
            candidates_out: turn0_narrowed.candidates.clone(),
            llm_reasoning: truncate_reasoning(turn0_narrowed.reasoning),
            swarm_label_attempt: None,
            swarm_confidence: None,
            cost_usd: turn0_narrowed.cost_usd,
            duration_ms: turn0_narrowed.duration_ms,
            provider_used: Some(turn0_narrowed.provider_used),
            timestamp: Utc::now(),
        });

        let mut current_candidates = turn0_narrowed.candidates;

        // Turns 1+ — partial swarm + LLM refine
        for turn in 1..=max_turns {
            let swarm_result = self
                .swarm
                .classify(&request.sentence, &current_candidates)
                .await?;

            // Check convergence
            if swarm_result.confidence >= threshold {
                stack.append(ConceptStackEntry {
                    turn,
                    candidates_in: current_candidates.clone(),
                    candidates_out: vec![], // converged, no further narrowing
                    llm_reasoning: None,
                    swarm_label_attempt: Some(swarm_result.label.clone()),
                    swarm_confidence: Some(swarm_result.confidence),
                    cost_usd: 0.0,
                    duration_ms: swarm_result.duration_ms,
                    provider_used: None,
                    timestamp: Utc::now(),
                });
                stack.finalize(swarm_result.label, swarm_result.confidence, true);
                return self.build_response(request, stack, tenant_config).await;
            }

            // Not converged ; if not last turn, LLM refine
            if turn < max_turns {
                let refined = self
                    .llm_narrow(
                        &request,
                        &current_candidates,
                        "mode3_refine_v1",
                        2,
                    )
                    .await?;
                stack.append(ConceptStackEntry {
                    turn,
                    candidates_in: current_candidates.clone(),
                    candidates_out: refined.candidates.clone(),
                    llm_reasoning: truncate_reasoning(refined.reasoning),
                    swarm_label_attempt: Some(swarm_result.label.clone()),
                    swarm_confidence: Some(swarm_result.confidence),
                    cost_usd: refined.cost_usd,
                    duration_ms: refined.duration_ms,
                    provider_used: Some(refined.provider_used),
                    timestamp: Utc::now(),
                });
                current_candidates = refined.candidates;
            } else {
                // max_turns reached — accept best-effort label
                stack.append(ConceptStackEntry {
                    turn,
                    candidates_in: current_candidates.clone(),
                    candidates_out: vec![],
                    llm_reasoning: None,
                    swarm_label_attempt: Some(swarm_result.label.clone()),
                    swarm_confidence: Some(swarm_result.confidence),
                    cost_usd: 0.0,
                    duration_ms: swarm_result.duration_ms,
                    provider_used: None,
                    timestamp: Utc::now(),
                });
                stack.finalize(swarm_result.label, swarm_result.confidence, false);
                return self.build_response(request, stack, tenant_config).await;
            }
        }

        unreachable!("loop must return on convergence or max_turns")
    }
}

fn truncate_reasoning(reasoning: Option<String>) -> Option<String> {
    reasoning.map(|s| {
        if s.len() > LLM_REASONING_MAX_CHARS {
            let mut t = s.chars().take(LLM_REASONING_MAX_CHARS).collect::<String>();
            t.push_str("...[truncated]");
            t
        } else {
            s
        }
    })
}
```

### 6.3 Convergence check rules

| Condition | Action |
|---|---|
| `swarm_confidence >= threshold` | STOP, `converged = true`, return |
| `turn == max_turns` (last turn) | STOP, `converged = false`, may queue review |
| Otherwise | CONTINUE to next turn |

### 6.4 SLOs Phase A (D65)

| ID | SLI | Target |
|---|---|---|
| M22-A-1 | concept_stack growth per turn (memory) | ≤ 4 KB per entry |
| M22-A-2 | Mode-3 loop overhead vs Mode-2 N calls | < 5 % |
| M22-A-3 | Pin tests Phase A | +20 GREEN |

### 6.5 Pin tests Phase A (~20)

- `test_concept_stack_append_increments_total_cost`
- `test_concept_stack_finalize_sets_converged_flag`
- `test_mode3_engine_converges_at_turn_1_when_high_conf`
- `test_mode3_engine_continues_when_below_threshold`
- `test_mode3_engine_stops_at_max_turns_with_converged_false`
- `test_mode3_engine_respects_tenant_max_turns_cap`
- `test_mode3_engine_respects_hard_cap_5_turns`
- `test_mode3_engine_appends_one_entry_per_turn`
- `test_mode3_engine_truncates_long_llm_reasoning`
- `test_mode3_engine_calls_mode2_template_at_turn_0`
- `test_mode3_engine_calls_mode3_refine_template_at_turn_1`
- `test_mode3_engine_no_llm_call_on_convergence_turn`
- `test_concept_stack_entry_serialises_to_jsonb`
- `test_concept_stack_total_turns_count`
- `test_mode3_threshold_override_per_request`
- `test_mode3_default_threshold_from_tenant_config`
- `test_mode3_swarm_label_persisted_in_each_turn`
- `test_mode3_provider_used_tracked_per_turn`
- `test_mode3_total_cost_equals_sum_of_turns`
- `test_mode3_duration_ms_aggregated_correctly`

---

## 7. Phase B (D66) — GDPR Art. 22 compliance

### 7.1 Right-to-explanation requirements

GDPR Art. 22(3) accorde au data subject le droit :
1. D'obtenir une **intervention humaine** dans la décision automatisée.
2. D'**exprimer son point de vue** (contestation).
3. De **contester la décision**.

EFE en M22 satisfait ces 3 exigences via :

| Requirement | Implementation M22 |
|---|---|
| Human intervention | `mode3_review_queue` + per-tenant `human_review_required` flag + `efe_approve_mode3_review` / `efe_reject_mode3_review` mcp tools |
| Express point of view | `mode3_review_queue.review_notes` field + DPO export procedure (out-of-band ticketing) |
| Contest decision | Audit chain HMAC-chained → DPO can re-run the full concept_stack with different parameters |

### 7.2 Concept_stack as audit trace

```rust
// crates/efe-server/src/audit_chain.rs (extension M22)

impl AuditChain {
    /// Append Mode-3 final audit row with concept_stack denormalised
    /// for fast retrieval. Intermediate turns are also audited but
    /// without the denormalised concept_stack copy.
    pub async fn append_mode3_final(
        &self,
        request: &ClassifyMode3Request,
        response: &ClassifyMode3Response,
        concept_stack: &ConceptStack,
    ) -> Result<i64, AuditError> {
        let stack_json = serde_json::to_value(concept_stack)
            .map_err(|e| AuditError::Serialisation(e.to_string()))?;

        let payload = format!(
            "{}|{}|{}|{}|MODE_3|{}|{}|{}|{}|{:.6}|{}|{}|{}|{:.4}|{}|{}|{}",
            request.tenant_id,
            request.catalog,
            request.bundle_version,
            sha256_hex(&request.sentence_redacted),
            // No single provider for Mode-3 ; capture composite
            "mode3-composite",
            // No single model for Mode-3
            "mode3-composite",
            // Sum tokens across all turns
            concept_stack.total_input_tokens(),
            concept_stack.total_output_tokens(),
            concept_stack.total_cost_usd,
            // cache_hit not applicable
            false,
            concept_stack.entries.iter().map(|e| e.duration_ms).sum::<u32>(),
            response.final_label.as_deref().unwrap_or(""),
            response.final_confidence.unwrap_or(0.0),
            // anchors_used = final candidates_in of last turn
            concept_stack.entries.last()
                .map(|e| e.candidates_in.join(","))
                .unwrap_or_default(),
            concept_stack.total_turns(),
            response.converged,
        );
        let next_seq = self.next_seq(&request.tenant_id).await?;
        let prev_hmac = self.last_hmac(&request.tenant_id).await?;
        let hmac_hex = self.compute_hmac(&payload, &prev_hmac);

        sqlx::query(
            r#"INSERT INTO llm_call_audit
               (tenant_id, catalog, bundle_version, sentence_hash, mode,
                provider_used, model_used, tokens_in, tokens_out, cost_usd,
                cache_hit, duration_ms, label, confidence, anchors_used,
                seq, prev_hmac, hmac, concept_stack_jsonb,
                total_turns, converged)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                       $16,$17,$18,$19,$20,$21)
               RETURNING id"#,
        )
        // ... bind all params ...
        .fetch_one(&*self.pool)
        .await?
        .get::<i64, _>("id")
    }
}
```

### 7.3 Human review workflow

```
                       Mode-3 reaches max_turns + not converged
                                        │
                                        ▼
                       Stakes tier check
                                        │
                                ┌───────┴────────┐
                                │                │
                          stakes=high        stakes=low/medium
                       human_review=true   human_review=false
                                │                │
                                ▼                ▼
                       INSERT INTO          Return as-is
                       mode3_review_queue    (best-effort label)
                       (status='pending'
                        review_token=uuid
                        expires_at=now()+7d)
                                │
                                ▼
                       Response :
                         requires_human_review=true
                         review_token=<uuid>
                                │
                                │ (mcp client / agent shows token to operator)
                                ▼
                       efe_get_mode3_reviews(status='pending')
                                │
                                ▼
                       Operator reviews concept_stack
                                │
                                ├─→ efe_approve_mode3_review(token, final_label)
                                │     → status='approved', reviewed_at=now()
                                │     → audit chain append 'mode3_review_approved'
                                │
                                └─→ efe_reject_mode3_review(token, notes)
                                      → status='rejected', reviewed_at=now()
                                      → audit chain append 'mode3_review_rejected'
                                      → optionally trigger re-classification
                                        with manual override
```

### 7.4 Stakes tier hard gate (Postgres trigger)

```sql
-- Migration 011_m22_mode3_review_queue.sql (extract)

CREATE OR REPLACE FUNCTION mode3_compliance_gate()
RETURNS TRIGGER AS $$
DECLARE
    stakes TEXT;
BEGIN
    IF NEW.enabled = TRUE THEN
        -- Check that for any high-stakes catalog the tenant uses,
        -- human_review_required is true.
        -- Table is `catalogs` (plural) per migration 0001_init.sql.
        SELECT MAX(c.stakes_tier) INTO stakes
          FROM catalogs c
         WHERE c.tenant_id = NEW.tenant_id
           AND c.mode3_enabled = TRUE;
        IF stakes = 'high' AND NEW.human_review_required = FALSE THEN
            RAISE EXCEPTION
              'tenant_feature_mode3: high-stakes catalog requires human_review_required=true (GDPR Art. 22(3) + AI Act Art. 14)';
        END IF;
    END IF;
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS tenant_feature_mode3_gate
    ON tenant_feature_mode3;
CREATE TRIGGER tenant_feature_mode3_gate
    BEFORE INSERT OR UPDATE ON tenant_feature_mode3
    FOR EACH ROW EXECUTE FUNCTION mode3_compliance_gate();
```

### 7.5 SLOs Phase B (D66)

| ID | SLI | Target |
|---|---|---|
| M22-COMPLIANCE-1 | High-stakes activation rejection rate | 100 % when human_review=false |
| M22-COMPLIANCE-2 | Concept_stack persistence (audit chain) | 100 % of Mode-3 calls |
| M22-COMPLIANCE-3 | DPO export response time (manual) | < 24 h business |
| M22-COMPLIANCE-4 | Pin tests Phase B | +15 GREEN |

### 7.6 Pin tests Phase B (~15)

- `test_postgres_trigger_rejects_high_stakes_no_review`
- `test_postgres_trigger_allows_low_stakes_no_review`
- `test_postgres_trigger_allows_high_stakes_with_review`
- `test_concept_stack_persisted_jsonb_in_audit`
- `test_concept_stack_retrievable_via_get_concept_stack`
- `test_mode3_audit_chain_hmac_consistent`
- `test_mode3_audit_chain_includes_total_turns`
- `test_mode3_audit_chain_includes_converged_flag`
- `test_dpo_export_includes_full_concept_stack`
- `test_human_review_required_enforced_for_high_stakes`
- `test_human_review_optional_for_low_stakes`
- `test_human_review_optional_for_medium_stakes`
- `test_concept_stack_redacts_pii_in_reasoning`
- `test_audit_chain_compatibility_with_m21_audit_query`
- `test_compliance_gate_blocks_misconfig_at_api_level`

---

## 8. Phase C (D67) — Mode-3 backend RPC + persistence

### 8.1 ClassifyMode3 RPC

```protobuf
// crates/efe-proto/v1.5/swarm.proto (extract)

service EfeSwarmService {
  // existing M16-M21 RPCs ...
  rpc ClassifyMode3(ClassifyMode3Request) returns (ClassifyMode3Response);
}

message ClassifyMode3Request {
  string sentence = 1;
  string catalog = 2;
  optional string version = 3;
  optional uint32 max_turns = 4;          // default tenant.default_max_turns
  optional float threshold = 5;           // default tenant.default_threshold
  optional uint32 top_k_anchors = 6;      // default 10 (turn 0)
}

message ClassifyMode3Response {
  optional string final_label = 1;        // null only if rejected for review
  optional float final_confidence = 2;
  Mode mode = 3;                          // = MODE_3
  repeated ConceptStackEntryProto concept_stack = 4;
  uint32 total_turns = 5;
  double total_cost_usd = 6;
  bool converged = 7;
  bool requires_human_review = 8;
  optional string review_token = 9;       // UUID v4 if review queued
  optional string degraded_reason = 10;   // e.g. "all providers failed turn 1"
}

message ConceptStackEntryProto {
  uint32 turn = 1;
  repeated string candidates_in = 2;
  repeated string candidates_out = 3;
  optional string llm_reasoning = 4;
  optional string swarm_label_attempt = 5;
  optional float swarm_confidence = 6;
  double cost_usd = 7;
  uint32 duration_ms = 8;
  optional string provider_used = 9;
  string timestamp = 10;                  // RFC 3339
}
```

### 8.2 Concept_stack JSON schema

(See Annex B for full schema with examples.)

### 8.3 Postgres mode3_review_queue

(Cf. § 4.5 schema definition.)

Key invariants :
- `review_token` is UUID v4 (URL-safe).
- `expires_at = requested_at + 7d` by default (configurable per tenant).
- `status` lifecycle : `pending` → `approved` | `rejected` | `expired`
  (cron job).
- Index on `(tenant_id, status, requested_at DESC)` for fast review
  queue listing.

### 8.4 Audit chain integration

Each Mode-3 call produces :
- **N rows in `llm_call_audit`** (one per turn) — captures
  per-provider cost + provider_used + tokens (for budget reconciliation).
- **1 final row** with `concept_stack_jsonb` populated — the canonical
  trace for right-to-explanation.

Total `llm_call_audit` rows per Mode-3 call : `N + 1` where `N = total_turns`.

### 8.5 SLOs Phase C (D67)

| ID | SLI | Target |
|---|---|---|
| M22-MODE3-1 | Mode-3 p50 latency (3 turns) | < 3 s |
| M22-MODE3-2 | Mode-3 p99 latency (5 turns) | < 6 s |
| M22-MODE3-3 | Mode-3 convergence rate (3 turns) | ≥ 80 % |
| M22-AUDIT-1 | Concept_stack jsonb persistence success | 100 % |
| M22-PIN | Pin tests Phase C | +18 GREEN |

### 8.6 Pin tests Phase C (~18)

- `test_classify_mode3_basic_path_3_turns`
- `test_classify_mode3_converges_at_turn_2`
- `test_classify_mode3_max_turns_5_hard_cap`
- `test_classify_mode3_returns_concept_stack`
- `test_classify_mode3_total_cost_equals_sum_turns`
- `test_classify_mode3_total_turns_matches_stack_length`
- `test_classify_mode3_writes_n_plus_1_audit_rows`
- `test_classify_mode3_concept_stack_jsonb_persisted`
- `test_classify_mode3_returns_review_token_when_high_stakes_no_converge`
- `test_classify_mode3_no_review_token_when_converged`
- `test_classify_mode3_no_review_token_when_low_stakes`
- `test_classify_mode3_unimplemented_when_flag_off`
- `test_classify_mode3_requires_m21_active`
- `test_classify_mode3_pii_redaction_in_concept_stack`
- `test_classify_mode3_provider_failover_within_turn`
- `test_classify_mode3_audit_chain_hmac_chain_intact`
- `test_classify_mode3_concurrent_calls_thread_safe`
- `test_classify_mode3_streaming_emits_phase_per_turn`

---

## 9. Phase D (D68) — Human review workflow

### 9.1 Review token generation

```rust
fn generate_review_token() -> String {
    // UUID v4, URL-safe (hyphenated)
    Uuid::new_v4().to_string()
}
```

Token entropy : 122 bits — non-guessable.

### 9.2 Review queue admin endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/admin/mode3/reviews?tenant=...&status=pending` | List pending reviews |
| GET | `/admin/mode3/reviews/{token}` | Get full concept_stack + decision context |
| POST | `/admin/mode3/reviews/{token}/approve` | Approve (operator confirms label) |
| POST | `/admin/mode3/reviews/{token}/reject` | Reject (label is wrong, operator provides override) |

### 9.3 Approval/rejection semantics

```rust
pub async fn approve_review(
    pool: &PgPool,
    review_token: &str,
    final_label: Option<String>,
    reviewed_by: &str,
) -> Result<(), ReviewError> {
    let mut tx = pool.begin().await?;

    // Atomic update with row lock
    let row = sqlx::query!(
        "UPDATE mode3_review_queue
         SET status = 'approved',
             reviewed_at = now(),
             reviewed_by = $1,
             final_label = COALESCE($2, proposed_label)
         WHERE review_token = $3
           AND status = 'pending'
           AND expires_at > now()
         RETURNING audit_id, tenant_id",
        reviewed_by,
        final_label,
        review_token,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let row = row.ok_or(ReviewError::NotFoundOrExpired)?;

    // Append audit chain event
    AuditChain::append_review_decision(
        &mut tx,
        &row.tenant_id,
        row.audit_id,
        "mode3_review_approved",
        reviewed_by,
        final_label.as_deref(),
    ).await?;

    tx.commit().await?;
    Ok(())
}

pub async fn reject_review(
    pool: &PgPool,
    review_token: &str,
    reason: &str,
    reviewed_by: &str,
) -> Result<(), ReviewError> {
    // Symmetric to approve_review with status='rejected' + reason
    // ...
}
```

### 9.4 SLA tracking

```rust
// Cron job daily at 02:00 UTC
pub async fn expire_stale_reviews(pool: &PgPool) -> Result<u64, ReviewError> {
    let n = sqlx::query!(
        "UPDATE mode3_review_queue
         SET status = 'expired'
         WHERE status = 'pending'
           AND expires_at < now()"
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(n)
}

pub async fn sla_dashboard(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<SLADashboard, ReviewError> {
    let stats = sqlx::query!(
        "SELECT
            COUNT(*) FILTER (WHERE status = 'pending') AS pending,
            COUNT(*) FILTER (WHERE status = 'approved') AS approved,
            COUNT(*) FILTER (WHERE status = 'rejected') AS rejected,
            COUNT(*) FILTER (WHERE status = 'expired') AS expired,
            AVG(EXTRACT(EPOCH FROM (reviewed_at - requested_at)) / 3600.0)
                FILTER (WHERE reviewed_at IS NOT NULL) AS avg_review_hours,
            PERCENTILE_CONT(0.95) WITHIN GROUP (
                ORDER BY EXTRACT(EPOCH FROM (reviewed_at - requested_at)) / 3600.0
            ) FILTER (WHERE reviewed_at IS NOT NULL) AS p95_review_hours
         FROM mode3_review_queue
         WHERE tenant_id = $1
           AND requested_at >= now() - INTERVAL '30 days'",
        tenant_id,
    )
    .fetch_one(pool)
    .await?;
    // ... build SLADashboard ...
}
```

### 9.5 SLOs Phase D (D68)

| ID | SLI | Target |
|---|---|---|
| M22-REVIEW-1 | Human review SLA (operator response) | < 24 h p95 |
| M22-REVIEW-2 | Approve/reject latency | < 100 ms p99 |
| M22-REVIEW-3 | Expired review cleanup success | 100 % daily |
| M22-PIN | Pin tests Phase D | +12 GREEN |

### 9.6 Pin tests Phase D (~12)

- `test_review_token_uuid_v4_format`
- `test_review_token_collision_resistance`
- `test_approve_review_updates_status_atomically`
- `test_approve_review_idempotent`
- `test_approve_review_rejects_expired`
- `test_approve_review_rejects_already_decided`
- `test_reject_review_with_reason`
- `test_expire_stale_reviews_cron`
- `test_sla_dashboard_p95_calculation`
- `test_sla_dashboard_30_day_window`
- `test_review_audit_chain_event_emitted`
- `test_review_concurrent_approve_one_wins`

---

## 10. Phase E (D69) — mcp-server tools + audit pass M22

### 10.1 `efe_classify_mode3`

```python
INPUT_SCHEMA = {
    "type": "object",
    "title": "EfeClassifyMode3Input",
    "properties": {
        "sentence": {"type": "string", "minLength": 1, "maxLength": 4096},
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+$"},
        "max_turns": {"type": "integer", "minimum": 1, "maximum": 5},
        "threshold": {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "top_k_anchors": {"type": "integer", "minimum": 3, "maximum": 20, "default": 10},
    },
    "required": ["sentence", "catalog"],
    "additionalProperties": False,
}
```

Read-only tool. Wraps backend `ClassifyMode3` RPC.

### 10.2 `efe_get_mode3_reviews`

```python
INPUT_SCHEMA = {
    "type": "object",
    "title": "EfeGetMode3ReviewsInput",
    "properties": {
        "status": {
            "type": "string",
            "enum": ["pending", "approved", "rejected", "expired", "all"],
            "default": "pending",
        },
        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
        "offset": {"type": "integer", "minimum": 0, "default": 0},
    },
    "additionalProperties": False,
}
```

### 10.3 `efe_approve_mode3_review` (write+confirm)

```python
INPUT_SCHEMA = {
    "type": "object",
    "title": "EfeApproveMode3ReviewInput",
    "properties": {
        "review_token": {"type": "string", "format": "uuid"},
        "final_label": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "If null, accepts proposed_label. Otherwise overrides.",
        },
        "review_notes": {"type": "string", "maxLength": 2048},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["review_token", "confirm"],
    "additionalProperties": False,
}
```

### 10.4 `efe_reject_mode3_review` (write+confirm)

Symmetric to approve, with `reason` required.

### 10.5 `efe_get_concept_stack`

```python
INPUT_SCHEMA = {
    "type": "object",
    "title": "EfeGetConceptStackInput",
    "properties": {
        "audit_id": {"type": "integer", "minimum": 1},
        "include_pii_unredacted": {
            "type": "boolean",
            "default": False,
            "description": "Admin only ; for DPO export.",
        },
    },
    "required": ["audit_id"],
    "additionalProperties": False,
}
```

Profile : `agent-native` (basic) or `agent-native-deploy`
(if `include_pii_unredacted=true`).

### 10.6 Audit pass M22 plan

Pattern récurrent M11-M21 (~6-10 défauts attendus). Phase D69
post-livraison. Targets attendus :
- 1-2 HIGH (probablement convergence edge case OR concept_stack
  serialization issue OR review queue race condition).
- 3-5 MED (cron expiration timing, audit chain HMAC across N turns,
  PII redaction in llm_reasoning).
- 2-4 LOW (CLI affichage, doc, schema patterns).

Pin tests audit pass : `test_audit_pass_m22.rs` post-livraison
~15 tests.

---

## 11. New tools + resources + prompts

### 11.1 Tools (5 new, 49 cumulés)

Cf. § 4.1.

### 11.2 Resources (2 new, 16 cumulés)

| URI | Profile | Returns |
|---|---|---|
| `efe://mode3/reviews/pending` | agent-native | List of pending reviews (paginated) |
| `efe://mode3/sla/dashboard` | agent-native-deploy | SLA stats per-catalog (30-day window) |

### 11.3 Prompts (1 new, 12 cumulés)

`mode3_refine_v1.md` template :

```markdown
---
name: mode3_refine_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, bedrock, azure-openai]
min_context_tokens: 6000
max_response_tokens: 300
created: 2027-02-15
author: tech_lead_m22
eval_pass: true
eval_dataset: mode3_gold_v1.0
eval_accuracy: 0.89
notes: |
  Tested against 300 turn-1 refinement scenarios on insurance claim
  routing dataset. 89% of refined anchor pairs match human expert
  selection (vs 78% for naive Mode-2 retry). Critical : LLM must
  acknowledge swarm_label_attempt + propose strict subset.
---

# System
You are reviewing a partial classification attempt. Given the original
sentence, the candidate anchors, and the current swarm-attempted label
(with confidence), refine the candidate set to 2 anchors that best
discriminate the correct class.

Rules:
- You MUST output exactly 2 anchors.
- Each anchor MUST come from the input candidate set verbatim.
- The reasoning section MUST explicitly acknowledge the swarm attempt.
- ≤ 200 words reasoning.

Output JSON :
{
  "anchors": ["...", "..."],
  "reasoning": "..."
}

Do NOT wrap in markdown code fences.

# User
Sentence : "$sentence"

Current candidate anchors : $candidates_json
Swarm attempted label : "$swarm_label_attempt"
Swarm confidence : $swarm_confidence (below threshold $threshold)

Refine the candidate set to the 2 most discriminating anchors.
Acknowledge the swarm attempt and explain your refinement choice.
```

---

## 12. JSON Schemas (forward-refs)

Cf. Annex A for all 5 tools + Mode-3 RPC schemas.

---

## 13. Backend extensions (EFE 1.5 contract bump)

### 13.1 New gRPC RPCs

Cf. § 4.4.

### 13.2 New REST endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/admin/mode3/reviews` | OIDC | List reviews (filterable) |
| GET | `/admin/mode3/reviews/{token}` | OIDC | Get review detail |
| POST | `/admin/mode3/reviews/{token}/approve` | OIDC admin | Approve |
| POST | `/admin/mode3/reviews/{token}/reject` | OIDC admin | Reject |
| GET | `/admin/mode3/sla/dashboard` | OIDC admin | SLA dashboard |

### 13.3 Backward compat

- All EFE 1.4 RPCs unchanged.
- EFE 1.4 client + EFE 1.5 server : compat.
- mcp-server M22 detects backend version via `/info` :
  - `EFE 1.5` : full M22 features.
  - `EFE 1.4` : Mode-3 RPC returns UNIMPLEMENTED → mcp-server falls
    back to Mode-2 with degraded warning.

### 13.4 Migration plan

| Étape | Action | Owner | Quand |
|---|---|---|---|
| 1 | Bump `Cargo.toml` v1.4.x → v1.5.0-pre | Rust lead | D67 J1 |
| 2 | Implémenter Mode-3 RPC + 5 admin endpoints | Rust lead | D67-D68 |
| 3 | Pin tests `crates/efe-server/tests/m22_contract.rs` | Rust lead | D68 |
| 4 | Bump OpenAPI + .proto regen | Rust lead | D68 J5 |
| 5 | Release EFE 1.5.0-rc1 image | Release manager | D69 J1 |
| 6 | Pilot 2 design partners on staging | PM + customer success | D69 |
| 7 | EFE 1.5.0 GA | Release manager | M23 D70 J1 |

---

## 14. Threat model M22 (delta vs M21)

### 14.1 Vue d'ensemble

5 nouvelles menaces M22 sur les 47 cumulées M16-M21 = **52 cumulées**.

| ID | STRIDE | Threat | Probabilité | Sévérité | Mitigation livrée M22 |
|---|:---:|---|:---:|:---:|---|
| **TM-M22-T-01** | Tamper | concept_stack replay attack (replayed concept_stack to fake "approved" review) | LOW | MED | concept_stack signed HMAC at insert ; review_token includes audit_id ; replay detected at approve_review |
| **TM-M22-R-01** | Repud. | Subject denies decision ("I never agreed to this automated processing") | MED | HIGH | Audit chain captures full concept_stack ; 7-year retention ; DPO can produce signed export |
| **TM-M22-D-01** | DoS | Tenant burns budget via long max_turns (5 × N classifies) | MED | MED | max_turns hard cap 5 ; cost governor still enforces per-tenant budget ; alert on tenant cost spike |
| **TM-M22-I-01** | InfoDisc | concept_stack llm_reasoning leaks tenant data cross-tenant via cache | LOW | MED | Per-tenant cache namespace (M21 carry-over) ; reasoning truncated 1024 chars ; audit log review |
| **TM-M22-E-01** | Elev. | Forged review_token grants approval (operator privilege escalation) | LOW | HIGH | UUID v4 122-bit entropy ; review_token expires 7d ; admin endpoint requires OIDC admin scope |

### 14.2 Dossier détaillé — TM-M22-R-01 (Repudiation, HIGH)

**Vecteur d'attaque** : data subject claims "I never agreed to this
automated processing" (GDPR Art. 22 contestation).

**Impact** : litige légal, sanctions GDPR (jusqu'à 4 % chiffre
d'affaires annuel mondial).

**Contrôles M22** :
- **C1** : audit chain HMAC-chained sur 7 ans. concept_stack jsonb
  capture full reasoning trace.
- **C2** : DPO export procedure (out-of-band). Signed PDF avec
  Ed25519 signature (M19 BYOK carry-over).
- **C3** : Public-key infrastructure pour signature non-repudiable.
- **C4** : Tenant onboarding flow signe DPA + AI Act consent qui
  documente Mode-3 usage ; tenant can show consent to data subject.

**Pin test** : `test_dpo_export_includes_signed_concept_stack`.

### 14.3 Dossier détaillé — TM-M22-E-01 (Elevation of Privilege, HIGH)

**Vecteur d'attaque** : attacker forges or guesses a review_token to
approve a review they shouldn't.

**Pré-conditions** :
- Access to mcp-server with operator-level JWT.
- Knowledge of the format/predictability of review_token.

**Contrôles M22** :
- **C1** : UUID v4 → 122-bit entropy. Random guess probability
  ~10⁻³⁷.
- **C2** : review_token expires 7 days. Even leaked tokens have
  bounded validity.
- **C3** : Admin endpoints require OIDC admin scope. Audit chain
  records `reviewed_by` (JWT sub claim).
- **C4** : Atomic UPDATE with `WHERE status = 'pending'` — already
  decided reviews can't be re-approved.

**Pin test** : `test_review_token_collision_resistance`,
`test_approve_review_rejects_expired`.

### 14.4 Threat carry-over

47 menaces M16-M21 restent valides et sont systématiquement
ré-vérifiées par les pin tests M22 audit pass. La table cumulée
(52 menaces) est versionnée dans
`docs/security/threat_model_M22.md`.

---

## 15. Performance & SLOs M22

### 15.1 SLO definitions

#### M22-MODE3-1 — Mode-3 p50 latency (3 turns)

- **SLI** : `histogram_quantile(0.5, efe_mode3_duration_seconds{turns="3"})`
- **Cible** : p50 < 3 s.
- **Error budget** : 1 % time > cible / 30 jours.
- **Alert** : warning si p50 > 4 s 5 min ; page si > 5 s.

#### M22-MODE3-2 — Mode-3 p99 latency (5 turns max)

- **SLI** : `histogram_quantile(0.99, efe_mode3_duration_seconds{turns="5"})`
- **Cible** : p99 < 6 s.

#### M22-MODE3-3 — Convergence rate

- **SLI** : `(converged_total / mode3_total)`
- **Cible** : ≥ 80 % within max_turns=3.
- **Alert** : page si < 70 % over 1h.

#### M22-MODE3-4 — Concept_stack persistence

- **SLI** : `(concept_stack_persisted / mode3_total)`
- **Cible** : 100 %.
- **Hard gate** : any failure → P1 incident.

#### M22-REVIEW-1 — Human review SLA p95

- **SLI** : `(reviewed_within_24h / pending_total)`
- **Cible** : ≥ 95 % p95 < 24 h.
- **Alert** : warning if 5 reviews > 24h.

#### M22-COMPLIANCE-1 — Audit chain completeness

- **SLI** : `(mode3_audit_rows / mode3_calls × (1 + avg_turns))`
- **Cible** : 1.0 (every Mode-3 call writes N+1 rows).
- **Hard gate** : P1 if < 0.999.

#### M22-PIN — Pin tests M22

- **Cible** : ≥ 757 tests GREEN.
- **CI gate** : block merge si < 757.

### 15.2 Runbooks M22

| Runbook | Trigger | Owner |
|---|---|---|
| `m22_convergence_drop.md` | M22-MODE3-3 alert | SRE on-call + ML lead |
| `m22_review_queue_backlog.md` | > 50 pending reviews / tenant | Customer success |
| `m22_concept_stack_persist_fail.md` | M22-MODE3-4 P1 | SRE on-call + DPO |
| `m22_dpo_export_request.md` | DPO ticket for right-to-explanation | Security officer |

---

## 16. Compatibility matrix

### 16.1 EFE backend versions

| EFE version | mcp-server min | Status |
|---|---|:---:|
| 1.0 | 0.16 | EOL |
| 1.1-1.3 | 0.17-0.19 | maintenance/EOL |
| 1.4 | 0.21 | GA (M21) |
| **1.5** | **0.22** | **GA (M22)** |

EFE 1.4 + mcp-server 0.22 = compat (Mode-3 falls back to Mode-2
gracefully).

### 16.2 Postgres extensions

- `uuid-ossp` (already enabled D11 carry-over) — for UUID v4
  generation.
- No new extensions required.

### 16.3 Stakes tier × Mode-3 enablement matrix

| Tenant config | low | medium | high |
|---|:---:|:---:|:---:|
| `mode3.enabled=false` | OK | OK | OK |
| `mode3.enabled=true, human_review=false` | OK | OK | **❌ rejected** |
| `mode3.enabled=true, human_review=true` | OK | OK | OK |

---

## 17. Activation flags M22 (8-condition AND)

### 17.1 Default state — OFF total

### 17.2 4 niveaux d'activation

#### Niveau 1 — Helm chart

| Flag | Default | Description |
|---|:---:|---|
| `mode3.enabled` | `false` | Si `false` : table `mode3_review_queue` non créée par migration ; trigger non installé. |

Note : Mode-3 réutilise le service `efe-llm-router` (M21) ; pas de
nouveau deployment K8s.

#### Niveau 2 — Server env-vars

| Flag | Default | Description |
|---|:---:|---|
| `EFE_FEATURE_M22_MODE3_ENABLED` | `false` | Si `false` : ClassifyMode3 RPC retourne UNIMPLEMENTED. |
| `EFE_FEATURE_M22_REQUIRE_M21` | `true` | Validation startup. |
| `EFE_FEATURE_M22_REVIEW_TTL_DAYS` | `7` | Default review_token expiration. |

#### Niveau 3 — Tenant config (Postgres `tenant_feature_mode3` jsonb)

| Field | Default | Description |
|---|:---:|---|
| `enabled` | `false` | Tenant flag. |
| `human_review_required` | `false` | If `false`, no review queue ; auto-accept best effort. **HARD GATE for high-stakes catalogs.** |
| `default_max_turns` | `3` | Per-tenant default. |
| `max_max_turns` | `5` | Per-tenant hard cap. |
| `default_threshold` | `0.85` | Convergence threshold. |
| `review_ttl_days` | `7` | Per-tenant override of TTL. |

#### Niveau 4 — Catalog config

| Column | Default | Description |
|---|:---:|---|
| `mode3_enabled` | `false` | Per-catalog opt-in. |
| `stakes_tier` | `low` | `low` / `medium` / `high`. |
| `mode3_default_threshold` | `0.85` | Per-catalog override. |

#### Niveau 5 — Request

| Param | Default | Description |
|---|:---:|---|
| `mode` | `MODE_2` | If not `MODE_3`, Mode-3 ignored. |

### 17.3 8-condition AND logic

```rust
fn is_mode3_active(req: &ClassifyMode3Request, ctx: &TenantContext, server: &ServerConfig) -> bool {
    // Server level (3 conditions including M21 active check)
    server.feature_m22_mode3_enabled() &&
    server.feature_m21_mode2_enabled() &&  // M21 required
    server.llm_router_reachable() &&
    // Tenant level (3 conditions)
    ctx.tenant_config.mode3.enabled &&
    ctx.tenant_config.mode3.default_max_turns >= 1 &&
    ctx.tenant_config.mode3.default_max_turns <= 5 &&
    // Catalog level (1 condition + stakes gate)
    ctx.catalog_config.mode3_enabled &&
    // Request level (1 condition)
    matches!(req.mode, Mode::MODE_3)
    // = 8-condition AND
}
```

### 17.4 Compliance gate

`efe_set_tenant_feature(milestone="m22_mode3", enabled=true)` REJECTS
if any of :
- `default_max_turns < 1` or `> 5`.
- `default_threshold` outside [0.0, 1.0].
- Any `catalog.stakes_tier='high'` AND `human_review_required=false`.

---

## 18. Migration M21 → M22

### 18.1 No-action default

Un cluster mcp-server M21 GA upgradé vers M22 sans flag activé :
- Backend Docker image M22 build → identique en taille.
- Helm `mode3.enabled=false` → no migrations 011/012 applied.
- Boot logs : aucun message M22 (ClassifyMode3 returns UNIMPLEMENTED).
- mcp-server `list_tools` output : 44 tools (M21 baseline) ou 49
  (M21 + M22 ON).
- Test : `test_default_off_m22_no_mode3_rpc` passe.

### 18.2 Activation incrémentale

```bash
# Étape 1 : Helm activation (applies migrations 011, 012)
helm upgrade efe ./charts/efe --set mode3.enabled=true

# Étape 2 : server env-var
kubectl set env deployment/efe-server \
  EFE_FEATURE_M22_MODE3_ENABLED=true

# Étape 3 : tenant DPIA refresh + AI Act Art. 14 acknowledgment
psql -c "UPDATE tenant_llm_config SET
  ai_act_art14_acknowledged_at = now()
WHERE tenant_id = 'acme';"

# Étape 4 : tenant feature_mode3 row
psql -c "INSERT INTO tenant_feature_mode3
  (tenant_id, enabled, human_review_required, default_max_turns,
   default_threshold)
  VALUES ('acme', true, true, 3, 0.85);"

# Étape 5 : catalog opt-in
psql -c "UPDATE catalog SET
  mode3_enabled = true,
  stakes_tier = 'medium'
WHERE name = 'support_triage' AND tenant_id = 'acme';"

# Étape 6 : test
mcp_tool("efe_classify_mode3", {"sentence": "test", "catalog": "support_triage"})
```

### 18.3 Rollback < 60 s

```bash
# Tenant-level OFF
psql -c "UPDATE tenant_feature_mode3 SET enabled = false WHERE tenant_id = 'acme';"
```

In-flight Mode-3 calls complete normally ; new calls return
UNIMPLEMENTED.

---

## 19. Test plan (~80 nouveaux pin tests)

### 19.1 Pin tests inventory

| File | Tests | Phase |
|---|---:|:---:|
| `crates/efe-server/tests/test_mode3_concept_stack.rs` | 12 | A (D65) |
| `tests/test_mode3_engine_loop.rs` | 8 | A (D65) |
| `tests/test_mode3_compliance_gate.rs` | 15 | B (D66) |
| `tests/test_mode3_classify_rpc.rs` | 18 | C (D67) |
| `tests/test_mode3_review_queue.rs` | 12 | D (D68) |
| `tests/test_efe_classify_mode3_tool.py` | 7 | E (D69) |
| `tests/test_efe_mode3_review_tools.py` | 10 | E (D69) |
| `tests/test_efe_get_concept_stack_tool.py` | 5 | E (D69) |
| `tests/test_default_off_m22.py` | 8 | E (D69) |
| `tests/test_audit_pass_m22.rs` | (post-audit) | E (D69) |

Total estimated : **~95 tests** (livraison ~80 + audit pass ~15).
Roadmap target +80.

### 19.2 Conditional tests

- `test_e2e_mode3_with_anthropic.rs` — real Anthropic API.
- `test_e2e_mode3_with_vllm_local.rs` — vLLM in CI.
- `test_e2e_mode3_review_workflow.rs` — full approve/reject cycle
  with admin endpoints.

### 19.3 Load tests

- 50 RPS Mode-3 mixed workload (max_turns=3 typical).
- Verify Mode-1 + Mode-2 SLOs preserved under Mode-3 load.
- Verify cost governor hard cap enforced under concurrent replicas.

### 19.4 Audit pass M22

Pattern récurrent (M11-M21, ~6-10 défauts attendus). Phase D69
post-livraison. Targets :
- 1-2 HIGH (probablement convergence edge case OR concept_stack
  serialization issue OR review queue race condition).
- 3-5 MED (cron expiration timing, audit chain HMAC across N turns,
  PII redaction in llm_reasoning).
- 2-4 LOW (CLI affichage, doc, schema patterns).

---

## 20. Deployment scenarios

### 20.1 Dev (single-tenant, Mode-3 OFF)

```bash
EFE_OIDC_TOKEN=... \
EFE_ENDPOINT=http://localhost:8765 \
efe-mcp-server  # Mode-1 + Mode-2 + Mode-3 all OFF
```

### 20.2 Staging (1 tenant Mode-3 ON, low-stakes catalog)

```bash
helm upgrade efe ./charts/efe \
  --set llmRouter.enabled=true \
  --set mode3.enabled=true \
  --set efeServer.env.EFE_FEATURE_M21_MODE2_ENABLED=true \
  --set efeServer.env.EFE_FEATURE_M22_MODE3_ENABLED=true
```

### 20.3 Prod high-stakes (insurance, GDPR Art. 22)

```bash
# Tenant config : human_review_required=true MANDATORY for high-stakes
psql -c "INSERT INTO tenant_feature_mode3 (tenant_id, enabled,
  human_review_required, default_max_turns, default_threshold)
  VALUES ('acme-insurance', true, true, 3, 0.90);"
psql -c "UPDATE catalog SET stakes_tier = 'high'
  WHERE tenant_id = 'acme-insurance' AND name = 'claim_routing';"
```

### 20.4 Prod air-gapped (vLLM + Mode-3 self-host)

```bash
helm upgrade efe ./charts/efe \
  --set llmRouter.enabled=true \
  --set mode3.enabled=true \
  --set llmRouter.providers.enabled='{vllm}' \
  --set llmRouter.providers.vllm.url=http://vllm-cluster:8000/v1
```

---

## 21. Open questions

### 21.1 Should Mode-3 default `max_turns` be 3 or 4 ?

**Recommendation** : 3. Sweet spot cost/latency/quality. Tenants
escalate to 4-5 manually for specific cases.

### 21.2 Should review_token TTL be configurable per-catalog ?

**Recommendation** : No, per-tenant only. Per-catalog adds complexity
without clear use case demand.

### 21.3 Should approved reviews trigger automatic re-classification ?

**Recommendation** : No automatic re-classify. Operator's approved
label IS the final decision. Re-classify would create infinite loop
risk.

### 21.4 Should we support Mode-3 with `max_turns=1` ?

**Recommendation** : Yes — equivalent to Mode-2 with concept_stack
audit. Useful for tenants who want audit trace without multi-turn
overhead.

### 21.5 Should concept_stack be returned in MCP sampling response ?

**Recommendation** : No — security risk (TM-M22-I-01). Only via
`efe_get_concept_stack(audit_id)` with proper authorization.

### 21.6 Should we allow tenant-customisable `mode3_refine_v1` template ?

**Recommendation** : No — too risky. Templates versioned in code,
proven via eval. Reconsider in M23 (curation milestone).

---

## 22. FAQ

**Q : Combien Mode-3 va me coûter ?**
A : ~3× Mode-2 = ~$0.0042 per classify (Sonnet 4.5, 3 turns).
Charged against tenant `monthly_cap_usd` as N independent Mode-2
calls.

**Q : Mode-3 est-il obligatoire pour high-stakes ?**
A : Non, mais GDPR Art. 22 + AI Act Art. 14 imposent human oversight
qui est mieux servi par Mode-3 + concept_stack que Mode-2 single-turn.

**Q : Combien de tours en moyenne ?**
A : Mesure attendue post-livraison : ~1.8 average (most converge at
turn 1 or 2 ; max_turns=3 hit ~10 % of cases).

**Q : Que se passe-t-il si LLM oscille (turn 0 says X, turn 1 says
not X) ?**
A : Captured in concept_stack ; convergence check uses swarm
confidence, not LLM output. Oscillation visible to operator on
review queue.

**Q : Compatible avec EFE 1.4 backend ?**
A : Oui en mode dégradé Mode-2 only. Mode-3 RPC retourne
UNIMPLEMENTED sur EFE 1.4 → mcp-server fall-back.

**Q : DPO peut-il exporter le concept_stack ?**
A : Oui via `efe_get_concept_stack(audit_id, include_pii_unredacted=true)`
(admin-only). Output : signed PDF avec Ed25519.

---

## 23. Glossary M22

- **Mode-3** : multi-turn iterative refinement (this milestone).
- **Concept stack** : ordered list of turn entries capturing
  (candidates_in, candidates_out, llm_reasoning, swarm_label_attempt,
  swarm_confidence, cost, duration, provider).
- **Convergence** : `swarm_confidence >= threshold` reached.
- **Max-turns reached** : loop exits without convergence ; flagged
  for human review if stakes_tier=high.
- **Review queue** : Postgres table holding pending reviews with TTL.
- **Review token** : UUID v4 binding (audit_id, expires_at).
- **Stakes tier** : per-catalog risk classification
  (low/medium/high) driving compliance gates.
- **Right-to-explanation** : GDPR Art. 22(3) data subject right to
  contest automated decisions.
- **Concept_stack jsonb** : Postgres JSONB column storing the full
  reasoning trace.
- **mode3_refine_v1** : prompt template for turn 1+ refinement.
- **Composite provider/model** : Mode-3 audit row aggregates N turns
  ; `provider_used = "mode3-composite"`.

---

## 24. References

- **GDPR Art. 22** : <https://gdpr-info.eu/art-22-gdpr/>
- **GDPR Art. 35** (DPIA) : <https://gdpr-info.eu/art-35-gdpr/>
- **EU AI Act Art. 12** (record-keeping) :
  <https://artificialintelligenceact.eu/the-act/>
- **EU AI Act Art. 14** (human oversight) :
  <https://artificialintelligenceact.eu/the-act/>
- **NIST AI RMF 1.0** : <https://www.nist.gov/itl/ai-risk-management-framework>
- **EFE M21 architecture** : `EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`
- **EFE LLM roadmap** : `EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`
- **Postgres JSONB indexing** : <https://www.postgresql.org/docs/current/datatype-json.html>
- **UUID v4 specification** : RFC 4122.

---

## A — Full JSON Schemas

### A.1 `efe_classify_mode3`

(See § 10.1.)

### A.2 `efe_get_mode3_reviews`

(See § 10.2.)

### A.3 `efe_approve_mode3_review` (write)

(See § 10.3.)

### A.4 `efe_reject_mode3_review` (write)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeRejectMode3ReviewInput",
  "properties": {
    "review_token": {"type": "string", "format": "uuid"},
    "reason": {"type": "string", "minLength": 10, "maxLength": 2048},
    "confirm": {"type": "boolean", "const": true}
  },
  "required": ["review_token", "reason", "confirm"],
  "additionalProperties": false
}
```

### A.5 `efe_get_concept_stack`

(See § 10.5.)

---

## B — Concept stack JSON schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "concept_stack.json",
  "type": "object",
  "properties": {
    "entries": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "turn": {"type": "integer", "minimum": 0, "maximum": 5},
          "candidates_in": {
            "type": "array",
            "items": {"type": "string", "maxLength": 100},
            "minItems": 1,
            "maxItems": 20
          },
          "candidates_out": {
            "type": "array",
            "items": {"type": "string", "maxLength": 100},
            "maxItems": 10
          },
          "llm_reasoning": {
            "type": ["string", "null"],
            "maxLength": 1024
          },
          "swarm_label_attempt": {"type": ["string", "null"]},
          "swarm_confidence": {
            "type": ["number", "null"],
            "minimum": 0.0,
            "maximum": 1.0
          },
          "cost_usd": {"type": "number", "minimum": 0},
          "duration_ms": {"type": "integer", "minimum": 0},
          "provider_used": {"type": ["string", "null"]},
          "timestamp": {"type": "string", "format": "date-time"}
        },
        "required": ["turn", "candidates_in", "candidates_out",
                     "cost_usd", "duration_ms", "timestamp"]
      },
      "minItems": 1,
      "maxItems": 6
    },
    "final_label": {"type": ["string", "null"]},
    "final_confidence": {
      "type": ["number", "null"],
      "minimum": 0.0,
      "maximum": 1.0
    },
    "converged": {"type": "boolean"},
    "total_cost_usd": {"type": "number", "minimum": 0}
  },
  "required": ["entries", "converged", "total_cost_usd"]
}
```

---

## C — Postgres mode3_review_queue + audit extension

### C.1 mode3_review_queue schema

(Cf. § 4.5.)

### C.2 llm_call_audit extension

(Cf. § 4.5.)

### C.3 Cron job for review expiration

```yaml
# kubernetes/cronjob_mode3_review_expire.yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: efe-mode3-review-expire
spec:
  schedule: "0 2 * * *"  # daily at 02:00 UTC
  successfulJobsHistoryLimit: 3
  failedJobsHistoryLimit: 3
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: expire-reviews
              image: ghcr.io/efe-project/efe-server:0.22.0
              command: ["efe-server", "expire-mode3-reviews"]
              env:
                - name: EFE_PG_DSN
                  valueFrom:
                    secretKeyRef:
                      name: efe-pg-readwrite
                      key: dsn
          restartPolicy: OnFailure
```

---

## D — Pin tests inventory M22

| Phase | Tests |
|---|---:|
| A (concept stack + loop) | 20 |
| B (compliance gate + audit) | 15 |
| C (Mode-3 RPC + persistence) | 18 |
| D (review workflow) | 12 |
| E (mcp tools + default-off) | 30 |
| Audit pass M22 | 15 |
| **Total** | **~110** |

(Estimate ~80-110 selon overlap. Roadmap target +80 ; final post-audit
ajustement.)

---

## E — EFE 1.4 → 1.5 contract diff

### E.1 New gRPC RPCs (additive)

```protobuf
service EfeSwarmService {
  // existing 1.4 RPCs unchanged ...
  rpc ClassifyMode3(ClassifyMode3Request) returns (ClassifyMode3Response);
}

service EfeAdminService {
  // existing 1.4 RPCs unchanged ...
  rpc ListMode3Reviews(ListReviewsRequest) returns (ReviewList);
  rpc GetMode3Review(GetReviewRequest) returns (ReviewDetail);
  rpc ApproveMode3Review(ReviewActionRequest) returns (google.protobuf.Empty);
  rpc RejectMode3Review(ReviewActionRequest) returns (google.protobuf.Empty);
  rpc GetMode3SLADashboard(SLARequest) returns (SLADashboard);
}
```

### E.2 Backward compat

- All EFE 1.4 RPCs unchanged.
- EFE 1.4 client + EFE 1.5 server : compat.
- mcp-server M22 detects EFE 1.4 backend via `/info` and falls back :
  - Mode-3 RPC : returns UNIMPLEMENTED → mcp-server reports
    `degraded` flag → fall-back Mode-2.
  - Review admin RPCs : 404 → mcp-server reports "M22 backend not
    available".

---

## F — GDPR Art. 22 + AI Act Art. 14 compliance attestation

### F.1 GDPR Art. 22 Mode-3 conformity

| Requirement | M22 Implementation |
|---|---|
| Art. 22(1) right not to be subject to automated decision | Tenant can disable Mode-3 per-catalog ; Mode-1 fallback always available |
| Art. 22(2)(a) contract necessity exception | Tenant signs DPA documenting Mode-3 usage |
| Art. 22(2)(c) explicit consent path | Tenant configures `human_review_required=true` for explicit consent flow |
| Art. 22(3) human intervention right | `mode3_review_queue` + admin workflow (approve/reject) |
| Art. 22(3) express point of view | `review_notes` field + DPO out-of-band procedure |
| Art. 22(3) contest decision | concept_stack signed export + reclassification with manual override |

### F.2 AI Act Art. 14 high-risk human oversight

| Art. 14 requirement | M22 Implementation |
|---|---|
| 14(1) human oversight design | `human_review_required` flag + max_turns hard cap + stakes_tier gate |
| 14(2) effective human oversight | mcp-server tools `efe_get_mode3_reviews` + `efe_approve/reject` |
| 14(3) prevent risks to fundamental rights | Stakes-tier hard gate prevents misconfig for high-stakes |
| 14(4)(a) understand system outputs | concept_stack visible per audit_id + DPO export |
| 14(4)(b) detect anomalies/dysfunctions | SLA dashboard + alerts on convergence drop |
| 14(4)(c) decide not to use | `mode3.enabled=false` toggle <60s rollback |
| 14(4)(d) intervene/interrupt | `efe_reject_mode3_review` overrides AI decision |
| 14(4)(e) reverse system output | Manual override during reject ; final_label captured |

### F.3 Annual conformity assessment

- DPIA refresh annual.
- AI Act conformity assessment refresh annual.
- Pen-test annual on Mode-3 pathways (TM-M22-T-01 + TM-M22-E-01).

---

**FIN**

Document M22 architecture, **Mode-3 multi-turn iterative reasoning
edition**. ~3 100 lignes. Prêt à passer en livraison sprint D65
(kick-off 1 senior Rust lead + 1 senior Rust review-queue + 1 senior
Python + 0.5 SRE + 0.5 compliance).

**Garanties cardinales** :
1. **Default OFF + 8-condition AND** — sans aucun flag M22 activé,
   cluster = M21 GA bit-for-bit.
2. **Requires M21 active** — startup validation refuses boot si M22
   ON sans M21.
3. **Mode-1 + Mode-2 SLOs préservés** — Mode-3 dans thread pool
   dédié.
4. **Stakes tier hard gate** — Postgres trigger bloque misconfig
   `high + human_review=false`.
5. **Concept_stack persistence 100%** — chaque Mode-3 call écrit
   N+1 audit rows + concept_stack_jsonb.
6. **GDPR Art. 22 + AI Act Art. 14 conforme** — DPIA refresh + admin
   review workflow + DPO export.
7. **Threat model M22** — 5 nouvelles menaces (2 HIGH : repudiation
   + token forgery) ; mitigations layered.

Toute revue chirurgicale : ouvrir issue dans le repo et taguer
`[m22-architecture]`. Décisions § 21 (6 questions ouvertes) à
adresser avant kick-off D65.
