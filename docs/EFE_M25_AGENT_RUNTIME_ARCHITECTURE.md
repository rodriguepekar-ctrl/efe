# EFE M25 — Agent Runtime (experimental) — Architecture détaillée

> **Statut** : architecture livrable du sprint M25 (in-cluster agent
> runtime), suite directe de M24 (active learning loop). **Cette
> milestone ferme la roadmap LLM M20-M25**.
>
> **Auteur** : tech lead EFE.
> **Date** : 2026-05-09 (post-M24 audit).
> **Sprint cible** : D80-D85 (Q4 2027) — 6 jours-ingénieur effectifs.
> **Précision** : ultrathink + ultra-détaillé. Précision chirurgicale
> suprême. Document spécification-as-code.
> **Lettre EFE** : `K` (clôture roadmap M20-M25).
> **Statut produit** : **EXPERIMENTAL** — single design-partner pilot
> Q4 2027 ; sunset si adoption < 2 tenants à 12 mois post-livraison.
> **Bump semver** : EFE **2.0.0** (MAJOR — nouveau service `efe-agent-
> runner` + nouveau Helm chart + CRDs Kubernetes).
>
> **Pré-requis lecture** :
> - [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md) § 11 — la mission M25 en 90 lignes.
> - [`EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`](EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md) — closed-loop ML + retune + GDPR Art. 17.
> - [`EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md) — review queue + audit chain extension.
> - [`EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md`](EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md) — multi-turn pattern (réutilisé pour les boucles agent).
> - [`EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`](EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md) — LLMRouter, cost governance.
> - [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md) — observability, BYOK, PII redactor.
> - [`EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`](EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md) — Helm structure (M25 = **chart séparé**).
> - **EU AI Act 2024/1689 Art. 9** — risk management for high-risk autonomous AI systems (M25 self-classifies as high-risk).
> - **EU AI Act Art. 26** — provider obligations on output monitoring.
> - **GDPR Art. 22 + Art. 35** — automated decision-making + DPIA.
> - **NIS2 Article 21** — supply chain (LLM provider becomes critical sub-processor).
> - **NIST AI RMF 1.0** — Govern, Map, Measure, Manage functions.
> - **Firecracker microVM security model** — https://firecracker-microvm.github.io
> - **Kubernetes Pod Security Standards (restricted profile)**.

---

## Sommaire

1. [Mission + executive summary](#1-mission--executive-summary)
2. [Scope / non-scope chirurgicaux](#2-scope--non-scope)
3. [AS-IS post-M24](#3-as-is-post-m24)
4. [TO-BE — vision M25](#4-to-be--vision-m25)
5. [Sandboxing architecture (Firecracker microVM)](#5-sandboxing-architecture)
6. [Agent runtime architecture (control plane + data plane)](#6-agent-runtime)
7. [Tool access control — profiles + whitelist](#7-tool-access-control)
8. [Step approval workflow (in-band human-in-the-loop)](#8-step-approval)
9. [Long-running pause / resume / checkpointing](#9-pause-resume)
10. [Agent state + memory model](#10-state--memory)
11. [Cost governance M25 (extension M21)](#11-cost-governance)
12. [Architectural Decision Records (ADR)](#12-adrs)
13. [Data model (SQL + migration 0016)](#13-data-model)
14. [State machines](#14-state-machines)
15. [gRPC + Kubernetes CRD contracts](#15-grpc--crd)
16. [mcp-server tools M25](#16-mcp-server-tools)
17. [efe-admin CLI extensions](#17-efe-admin-cli)
18. [portal-admin UI extensions](#18-portal-admin-ui)
19. [Multi-tenant isolation M25 (defense in depth)](#19-multi-tenant-isolation)
20. [Compliance + AI Act Art. 9 + DPIA M25](#20-compliance--ai-act-art-9)
21. [Threat model M25 (delta vs M24)](#21-threat-model)
22. [Backend contract evolution (1.7 → 2.0 — semver MAJOR)](#22-contract-evolution)
23. [Bench + pin tests](#23-bench--pin-tests)
24. [Activation flags + dependency validation](#24-activation-flags)
25. [Sprint plan D80-D85](#25-sprint-plan)
26. [Risk register](#26-risk-register)
27. [Open questions / decisions needed](#27-open-questions)
28. [Success metrics + GA gate (experimental gate)](#28-success-metrics)
29. [Glossary M25+](#29-glossary)
30. [References](#30-references)

**Annexes**

- [A — Sandbox security profile (seccomp + AppArmor + SECCOMP_FILTER_FLAG_TSYNC)](#a--sandbox-security-profile)
- [B — Cost model M25 ($ / agent run / month)](#b--cost-model)
- [C — Agent task template specification (signed YAML)](#c--agent-task-template)
- [D — Pin tests inventory M25 (+90 cible)](#d--pin-tests-inventory)
- [E — DPIA delta M25 (post-market monitoring obligations)](#e--dpia-delta)
- [F — Migration 0016_agent_runtime.sql](#f--migration-0016)
- [G — Helm chart `efe-agent-runner` skeleton](#g--helm-chart)

---

## 1. Mission + executive summary

### 1.1 Mission en une phrase

> **M25 livre un runtime sandboxé permettant à un agent LLM de
> *tourner dans le cluster EFE*** (vs M16-M19 où l'agent est externe),
> avec isolation Firecracker microVM par tenant, whitelist obligatoire
> de tools, in-band step approval pour les opérations à fort impact,
> cost cap hard-limit par run, audit chain HMAC end-to-end, et
> conformité EU AI Act Art. 9 (système IA high-risk autonome) +
> Art. 14 (human oversight) + GDPR Art. 22 + Art. 35 (DPIA).

### 1.2 Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Agent runtime sandboxé in-cluster, multi-tenant isolé, audit chain end-to-end. |
| **Sprint** | D80-D85 (6 jours), Q4 2027. |
| **Lettre EFE** | `K` (close roadmap M20-M25). |
| **Bump semver** | EFE **2.0.0** (MAJOR). Nouveau service + nouveau chart + CRDs K8s. |
| **Statut** | **EXPERIMENTAL**. Single design-partner pilot. Sunset si < 2 tenants à 12 mois. |
| **Code volume** | ~5 200 LOC Rust (`crates/efe-agent-runner/` + crate sandbox driver) + ~2 400 LOC Python (mcp-server tools + agent core lib) + ~1 600 LOC TS (portal-admin) + ~2 400 LOC tests + ~360 LOC SQL + ~800 LOC YAML (Helm + CRDs). |
| **Pin tests** | +120 GREEN cible (oversubscribed vs +90 minimum roadmap). |
| **Backward compat** | EFE 1.x clients restent fonctionnels. M25 = service séparé (`efe-agent-runner`) shippé via Helm chart distinct. Si M25 OFF, EFE 2.0 ≡ EFE 1.7 + M24. |
| **Dependency hard** | **Requires M21 ON** (LLMRouter pour les LLM calls de l'agent). M22, M23, M24 acceptables OFF (l'agent runtime est composable). |
| **Compliance gate critique** | EU AI Act Art. 9 obligatoire — DPIA + post-market monitoring + human oversight + risk management system documenté. |
| **Activation cible** | 1 tenant pilot à T+90 j. ≥ 2 tenants pilots à T+12 mois (gate sunset). |
| **Eval cible** | Sandbox escape attempts détectés ≥ 100 % ; cost cap respecté 100 %. |
| **Risk profile** | **EXTRÊME** — première surface où un LLM exécute du code in-cluster. Worst-case : agent compromise → tenant data exfil OR compute resource abuse OR cross-tenant escape. Mitigation : Firecracker microVM + tool whitelist + step approval + DPIA + adversarial bench. |
| **Strategic outcome** | EFE devient une **plateforme classifier + LLM gouverné + agent runtime tenant-isolé**. Différenciateur produit ultime — aucun concurrent (AWS Bedrock Agents, OpenAI Assistants, Google Vertex AI Agents) ne ship avec audit chain HMAC + GDPR Art. 17 cascade + Firecracker isolation Day-1. |

### 1.3 TL;DR sequencing

| Jour | Cœur livraison | Pin tests cumulés |
|---|---|:---:|
| **D80** | DB migration 0016 + crate `efe-agent-runner/` skeleton + AgentRun CRD + Helm chart séparé `efe-agent-runner` | +18 |
| **D81** | Sandbox driver Firecracker (microVM provisioning + lifecycle) + network policy + filesystem ephemeral | +20 |
| **D82** | Agent core (LLM-driven loop) + tool whitelist enforcement + per-step audit | +20 |
| **D83** | Step approval workflow + pause/resume + checkpoint/restore + cost circuit breaker | +18 |
| **D84** | mcp-server tools (4 nouveaux) + portal-admin agent run UI + CLI commands | +18 |
| **D85** | Bench (sandbox escape adversarial) + DPIA documentation + threat model adversarial tests + GA gate (experimental gate) | +26 |

### 1.4 Frontière de sécurité chirurgicale (invariante M19→M25)

Les 13 invariants cumulés sont préservés ET **étendus** par 8 nouveaux M25-spécifiques :

**Hérités (préservés)** :
1. Bundle immutability
2. Audit chain HMAC unbroken
3. No tenant data leak across LLM prompts
4. Per-tenant LLM cost budget
5. AI Act Art. 14 human oversight (étendu en step approval)
6. AI Act Art. 15 accuracy gate
7. GDPR Art. 17 right-to-forget (cascade through agent runs)
8. Sample uniqueness (M24)
9. Holdout immutability (M24)
10. Centroid drift safety net (M24)
11. Promotion gate non-bypassable (M24)
12. DPO consent expiry (M24)
13. Sigstore signing of artifacts

**Nouveaux M25** :
14. **Sandbox isolation hard guarantee** — agent code ne peut JAMAIS s'évader du Firecracker microVM. Vérifié par adversarial bench + audit annuel obligatoire.
15. **Tool whitelist enforcement** — agent ne peut appeler QUE les tools listés dans son `AgentRun.allowed_tools`. Empty list = aucun tool callable.
16. **Step approval mandatory pour high-stakes tools** — toute écriture (curation, retune trigger, GDPR erasure, deploy) requiert approval humain inline avant exécution.
17. **Cost cap hard-limit** — chaque AgentRun a un `max_cost_usd` ; dépassement = SIGTERM immédiat du microVM.
18. **Network policy deny-all** — egress autorisé UNIQUEMENT vers : LLMRouter (M21), mcp-server (read-only profile), audit chain backend. Pas d'autre.
19. **Filesystem ephemeral** — `/tmp` wiped post-run. Aucune persistance disque hors agent_state DB rows.
20. **Image whitelist** — seules les images `efe-agent:*` officielles signées Sigstore (R-100) sont autorisées comme runtime image. Pas de custom user images.
21. **Tenant context binding immutable** — tenant_id injecté à la création du microVM ; toute tentative d'altération mid-run = SIGTERM.

---

## 2. Scope / non-scope

### 2.1 In-scope M25

✅ **Service backend Rust `efe-agent-runner`** :
   - Crate `crates/efe-agent-runner/` (control plane + data plane)
   - K8s controller pour AgentRun CRD reconciliation
   - Sandbox driver Firecracker (provisioning + lifecycle)
   - Agent core (LLM-driven multi-step loop)
   - Tool whitelist enforcer
   - Step approval gateway
   - Cost circuit breaker
   - Audit chain HMAC (étend M19)

✅ **Helm chart `efe-agent-runner`** (séparé, OPT-IN total) :
   - Default `agentRunner.enabled=false`
   - CRDs (AgentRun, AgentRunTemplate)
   - RBAC (ClusterRole strictement scoped)
   - NetworkPolicy (deny-all egress + whitelist)
   - PodSecurityPolicy / Pod Security Standards (restricted)

✅ **Kubernetes CRDs** :
   - `AgentRun` — une exécution
   - `AgentRunTemplate` — un template signé Sigstore réutilisable

✅ **gRPC `AgentRuntimeService`** : 14 RPCs (cf. § 15)

✅ **mcp-server tools** (4 nouveaux) :
   - `efe_create_agent_run` — submit a new AgentRun
   - `efe_get_agent_run` — fetch run state + step history
   - `efe_approve_agent_step` — operator approval inline
   - `efe_terminate_agent_run` — SIGTERM the microVM

✅ **efe-admin CLI** (extension) : `efe-admin agent {create,list,get,approve,terminate,template,logs,cost}`

✅ **portal-admin UI** :
   - `/agent/runs` — runs list + status
   - `/agent/runs/[id]` — live tail (SSE) + step approval
   - `/agent/templates` — template catalog
   - `/agent/cost` — cost dashboard
   - `/agent/dpia` — DPIA evidence viewer

✅ **Compliance hooks** :
   - DPIA template + completion workflow (per-tenant)
   - Post-market monitoring metrics emit (AI Act Art. 26)
   - Step approval audit lineage
   - Sandbox escape attempt alerting (PagerDuty)
   - Annual security audit checklist

✅ **Bench** : 6 scénarios dont **adversarial sandbox escape** (à 100 % détection cible, hard gate).

✅ **Activation flags** : 6 niveaux (Helm / server / CRD / tenant / template / per-run).

### 2.2 Non-scope M25 (out-of-scope V1)

❌ **Multi-LLM agent collaboration** — un seul LLM par agent run. Multi-agent orchestration = M26+ ou EFE 3.0.

❌ **Network egress vers tools custom (HTTP API tenant)** — V1 limite egress à LLMRouter + mcp-server + audit. Custom HTTP tools = annexe O-2.

❌ **Custom Python code execution sans sandbox** — strictement interdit. Toute logique custom doit passer par tools whitelist.

❌ **Live editing du prompt agent en cours de run** — agent prompt freezed à create-time. Edit = create new run.

❌ **GPU compute** — V1 : CPU only. GPU sandboxing nécessite encore plus d'isolation (pas mature pour V1).

❌ **Streaming output partial vers caller** — V1 : poll-based. Streaming via gRPC server-streaming = M26+.

❌ **Cross-cluster agent runs** — V1 : single-cluster. Federated = out-of-scope.

❌ **Auto-resumption après crash agent** — V1 : crash = run = FAILED final. Resume manuel via operator. Auto-resume = M26+.

❌ **Vector DB / RAG inside agent** — V1 : agent peut appeler les tools mcp-server `efe_classify` / `efe_explain_misclassification` mais pas faire vector search arbitraire. RAG agent = M26+.

❌ **Image whitelist élargie (custom user images)** — V1 : uniquement images officielles `efe-agent:*` signées. Custom = M26+.

### 2.3 Dépendances dures (ne peuvent être OFF)

| Dépendance | Version min | Raison |
|---|---|---|
| **Kubernetes** | 1.27+ | CRD API + Pod Security Standards (restricted) + NetworkPolicy v2 |
| **Firecracker** | 1.5+ | microVM driver, KVM-based |
| **M14** (multi-tenant) | always-on | tenant + catalog tables |
| **M14-bis** (audit chain HMAC) | always-on | foundation audit |
| **M16** (mcp-server) | always-on | les 4 tools sont MCP tools + agent appelle mcp-server |
| **M19** (BYOK + PII redactor) | always-on | secrets management + PII redaction |
| **M21** (LLMRouter + cost) | **ON requis** | agent uses M21 for ALL LLM calls |
| **R-100** (Sigstore signing pipeline) | v1.0.0 | sign agent task templates + agent images |

### 2.4 Dépendances soft

| Dépendance | Activation requise | Raison |
|---|---|---|
| **M22** (Mode-3 multi-turn) | OFF acceptable | M25 réutilise concept stack pattern, pas Mode-3 directement |
| **M23** (curation tools) | OFF acceptable | mais souhaité — curation tools = excellent agent profile |
| **M24** (active learning) | OFF acceptable | mais souhaité — agent peut trigger retunes |
| **M20** (BYO LLM agent SDK) | OFF acceptable | M25 utilise ses propres prompts / patterns |

---

## 3. AS-IS post-M24

### 3.1 Capabilities en place (post-M24)

| # | Capability | Module / artefact | Disponible pour M25 ? |
|---|---|---|:---:|
| 1 | LLM Router (M21) | `crates/efe-llm-router/` | ✅ tous LLM calls passent par lui |
| 2 | Cost governance multi-categories | `tenant_budget` + categories `mode2`/`mode3`/`curation`/`active_learning` | ✅ M25 ajoute catégorie `agent_runtime` |
| 3 | mcp-server (43 tools après M24) | `mcp-server/` | ✅ agent peut appeler tools whitelistés |
| 4 | Audit chain HMAC | `llm_call_audit`, `taxonomy_change_audit`, `learning_audit` | ✅ M25 ajoute `agent_audit` chaîné |
| 5 | Sigstore signing pipeline | `.github/workflows/release-v1-ga.yml` | ✅ M25 sign agent templates + images |
| 6 | PII redactor | `crates/efe-pii/` | ✅ obligatoire avant tout tool call ou LLM call |
| 7 | BYOK keystore | `crates/efe-keystore/` | ✅ secrets agent (LLM API keys) via BYOK |
| 8 | Curation tools (M23) | `crates/efe-curation/` | ✅ agent peut être profile-bound to curation tools |
| 9 | Active learning tools (M24) | `crates/efe-active-learning/` | ✅ agent peut trigger retunes via mcp tools |
| 10 | Helm chart `efe-stack` | `charts/efe-stack/` | ⚠ M25 ship un chart **séparé** `efe-agent-runner` |
| 11 | Sandbox runtime | ❌ **manquant** | ⚠ M25 introduit Firecracker driver |
| 12 | K8s CRD machinery | partial (utilise déjà K8s) | ⚠ M25 introduit AgentRun + AgentRunTemplate CRDs |
| 13 | Step approval workflow | partial (M23 review queue pattern) | ⚠ M25 étend pour in-band agent step gating |
| 14 | Per-tenant network policies | ⚠ partial (cluster-level) | ⚠ M25 ajoute deny-all egress par AgentRunPod |

### 3.2 Gap analysis — pourquoi M25 maintenant

| # | Manque post-M24 | Impact opérationnel actuel | M25 fix |
|---|---|---|:---:|
| 1 | Pas d'agent in-cluster | Tout l'agent autonomy doit être client-side (M16-M19 pattern) — bottleneck pour use cases long-running multi-step | ✅ § 6 |
| 2 | Pas de sandboxing pour code LLM-generated | LLMs ne peuvent pas exécuter de code arbitraire dans EFE — limite use cases | ✅ § 5 |
| 3 | Pas de step-level approval workflow | Tools writes (curation promote, M24 retune) requièrent operator action explicite hors-bande ; pas inline avec un agent run | ✅ § 8 |
| 4 | Pas de pause/resume long runs | Si un run prend 1h, doit rester live — impossible de schédule des batch audits nocturnes opérateur-driven | ✅ § 9 |
| 5 | Pas de DPIA workflow tooling | DPIA construit ad-hoc par tenant ; pas de template versionné ni d'evidence chain | ✅ § 20 |
| 6 | Pas de bench adversarial agent | Aucun garantie sandbox escape detection | ✅ § 23 |

### 3.3 Pourquoi maintenant (post-M19→M24)

- **M19** : observabilité + BYOK + PII redactor matures.
- **M21** : LLMRouter + cost governance opérationnel.
- **M22** : Mode-3 multi-turn = pattern précurseur pour agent loops.
- **M23** : review queue pattern réutilisable pour step approval.
- **M24** : closed-loop active learning fournit les tools que l'agent peut orchestrer.
- **R-100** : Sigstore pipeline mature pour signer agent images + templates.
- **Compliance posture** : DPIA + AI Act Art. 9 framework prêt pour le step le plus risqué.

---

## 4. TO-BE — vision M25

### 4.1 Diagramme système end-to-end

```
                    ┌─────────────────────────────────────────────┐
                    │  Operator (tenant_admin)                    │
                    │  via portal-admin OR efe-admin CLI          │
                    │                                             │
                    │  Submits AgentRun :                         │
                    │   - prompt                                  │
                    │   - allowed_tools : [efe_audit_health, ...] │
                    │   - max_cost_usd : 50                       │
                    │   - timeout_min : 60                        │
                    │   - template_id (signed) OR raw_prompt      │
                    └────────────┬────────────────────────────────┘
                                 │
                                 ▼ HTTPS + OIDC + tenant_admin role
                    ┌─────────────────────────────────────────────┐
                    │  efe-server gRPC AgentRuntimeService        │
                    │  CreateAgentRun handler                     │
                    │                                             │
                    │  1. Validate tenant + RBAC                  │
                    │  2. Validate cost cap + tool whitelist      │
                    │  3. Verify DPO consent + DPIA               │
                    │  4. Create AgentRun K8s CRD                 │
                    │  5. INSERT agent_run row (status=PENDING)   │
                    │  6. APPEND agent_audit (event=run_created)  │
                    └────────────┬────────────────────────────────┘
                                 │
                                 ▼ K8s API server
                    ┌─────────────────────────────────────────────┐
                    │  efe-agent-runner controller                │
                    │  (StatefulSet, leader-elected)              │
                    │                                             │
                    │  Reconcile loop :                           │
                    │   - watches AgentRun CRDs                   │
                    │   - status PENDING → spawn Firecracker VM   │
                    │   - status RUNNING → monitor + stream       │
                    │   - status PAUSED  → checkpoint + freeze    │
                    │   - status TERMINATED → cleanup             │
                    └────────────┬────────────────────────────────┘
                                 │
                                 ▼
              ┌──────────────────────────────────────────────────┐
              │  AgentRunPod (K8s Job, microVM-backed)           │
              │                                                  │
              │  ┌────────────────────────────────────────────┐ │
              │  │  Firecracker microVM                       │ │
              │  │   - kernel : minimal Linux 6.x             │ │
              │  │   - rootfs : readonly /                    │ │
              │  │   - tmpfs : /tmp (wiped post-run)          │ │
              │  │   - resource limits : 1 vCPU, 2 GB RAM     │ │
              │  │   - network : virtio-net w/ bridge         │ │
              │  │                                            │ │
              │  │  ┌──────────────────────────────────────┐  │ │
              │  │  │  Agent core (Python)                 │  │ │
              │  │  │   - prompt                           │  │ │
              │  │  │   - LLM call (via M21 router)        │  │ │
              │  │  │   - parse tool call request          │  │ │
              │  │  │   - check tool in whitelist          │  │ │
              │  │  │   - if write tool → request approval │  │ │
              │  │  │   - call mcp-server tool             │  │ │
              │  │  │   - feed result back to LLM          │  │ │
              │  │  │   - loop until done OR timeout       │  │ │
              │  │  └──────────────────────────────────────┘  │ │
              │  └────────────────────────────────────────────┘ │
              │                                                  │
              │  NetworkPolicy : deny-all egress except :        │
              │   - LLMRouter ClusterIP (port 50061)             │
              │   - mcp-server ClusterIP (port 8081)             │
              │   - audit-backend ClusterIP (port 8082)          │
              └──────────────────────────────────────────────────┘
                                 │
                                 │ tool calls (gRPC, mTLS)
                                 ▼
                    ┌─────────────────────────────────────────────┐
                    │  mcp-server (with agent profile filter)     │
                    │                                             │
                    │  - reads X-EFE-Agent-Run-Id header          │
                    │  - looks up AgentRun.allowed_tools          │
                    │  - rejects calls to unlisted tools          │
                    │  - rejects writes without approval token    │
                    │  - streams progress to portal-admin SSE     │
                    └────────────┬────────────────────────────────┘
                                 │
                                 ▼
                    ┌─────────────────────────────────────────────┐
                    │  EFE backend (efe-server)                   │
                    │   - mode1 / mode2 / mode3 classify          │
                    │   - curation (M23) tools                    │
                    │   - active learning (M24) tools             │
                    │   - audit chain HMAC writes                 │
                    └─────────────────────────────────────────────┘
                                 │
                                 ▼ (if write requires approval)
                    ┌─────────────────────────────────────────────┐
                    │  Approval queue → portal-admin              │
                    │  Operator inline approves OR rejects        │
                    │  → unblocks agent step execution            │
                    └─────────────────────────────────────────────┘
```

### 4.2 Cycle de vie d'un AgentRun (golden path)

```
T+0     Operator submits AgentRun via portal-admin or CLI :
        - prompt = "audit all bundles for SLSA L3 compliance and
                    write a runbook for non-compliant ones"
        - allowed_tools = [efe_list_available_bundles,
                           efe_audit_bundle_vs_gold (read),
                           efe_get_sbom, efe_get_audit_log]
        - max_cost_usd = 50, timeout_min = 60
        - template_id = "audit-runbook-author-v1" (signed)
        → INSERT agent_run (status=PENDING)
        → CREATE K8s AgentRun CRD
        → APPEND agent_audit (run_created, signed by submitter)

T+0+    efe-agent-runner controller picks up the CRD :
        → Validate template signature (cosign verify)
        → Allocate Firecracker microVM from pool
        → Inject tenant_id + run_id + LLM credentials (BYOK fetch)
        → Apply NetworkPolicy (deny-all except whitelist)
        → Mount readonly rootfs + tmpfs /tmp
        → Spawn agent_core process
        → status PENDING → RUNNING

T+1m    Agent core boots in microVM :
        → Read prompt + tool whitelist from env
        → Call LLM via M21 router (tenant scope)
        → LLM returns : "Step 1 : list bundles. Use efe_list_available_bundles"
        → Agent emits ToolCall event
        → mcp-server receives : verify tool in whitelist ✓ + read-only ✓
        → call efe-server, get bundle list
        → Return result to agent
        → APPEND agent_audit (step_1_completed)

T+5m    Agent processes 12 bundles in parallel via efe_audit_bundle_vs_gold :
        → 12 tool calls, all read-only, all in whitelist
        → APPEND 12 agent_audit rows (HMAC chained)
        → Cost so far : $4.20

T+30m   Agent has identified 3 non-compliant bundles. Now wants to
        write runbook drafts. Tries efe_create_runbook_draft (NOT in
        whitelist).
        → mcp-server rejects with PERMISSION_DENIED
        → Agent retries with efe_get_audit_log (in whitelist) to gather
          more data
        → After 5 more steps : agent says "I have enough data, here's
          the final runbook draft text"
        → Agent emits FINAL_OUTPUT event with markdown text

T+45m   Agent run completes :
        → status RUNNING → COMPLETED
        → Total cost : $18.40 (well under cap)
        → Total steps : 23
        → Final output emitted to portal-admin
        → Firecracker microVM destroyed
        → tmpfs /tmp wiped
        → APPEND agent_audit (run_completed)

T+45m+  Operator reviews output :
        → Approves : output markdown saved as draft in portal
        → Rejects : output discarded, run marked LEARNED_FROM
        → APPEND agent_audit (output_reviewed)

T+1d    Compliance pack regen includes the agent run lineage :
        - Agent template version + signature
        - All steps with timestamps + tool calls
        - Cost breakdown
        - DPIA evidence
        - Approval decisions
```

### 4.3 Comparaison vs concurrents

| Capability | EFE M25 | AWS Bedrock Agents | OpenAI Assistants | Google Vertex AI Agents |
|---|:---:|:---:|:---:|:---:|
| Tenant-isolated sandbox | ✅ Firecracker | ⚠ shared infra | ⚠ shared infra | ⚠ shared infra |
| Tool whitelist enforcement | ✅ per-run | ⚠ via Lambda IAM | ⚠ via custom code | ⚠ via custom code |
| In-band step approval | ✅ HITL inline | ❌ | ❌ | ❌ |
| Audit chain HMAC | ✅ end-to-end | ❌ CloudTrail only | ❌ | ❌ Cloud Audit only |
| GDPR Art. 17 cascade | ✅ inherits M24 | ❌ | ❌ | ❌ |
| AI Act Art. 9 DPIA workflow | ✅ first-class | ❌ | ❌ | ❌ |
| Cost cap hard-limit | ✅ per-run | ⚠ AWS Budget alerts | ⚠ usage limits | ⚠ |
| Sandbox escape detection | ✅ adversarial bench | ❌ | ❌ | ❌ |

### 4.4 Les 11 invariants de design M25

1. **Sandbox immutability** — la microVM ne peut PAS être modifiée pendant le run. Aucun mount additionnel, aucun network policy patch.
2. **Tool whitelist immutable** — `AgentRun.allowed_tools` freezed at create-time. Toute tentative de modification = run TERMINATED.
3. **Tenant binding immutable** — tenant_id injecté à VM-create, vérifié à chaque tool call. Discrepancy = SIGTERM immediate.
4. **Image whitelist** — runtime image MUST be in `agentRunner.image.allowList` (Helm config) AND signature-verified.
5. **Cost cap hard** — circuit-breaker SIGTERM le microVM si cost dépasse `max_cost_usd`. Aucun grace period.
6. **Time cap hard** — `timeout_min` enforced par K8s + runtime. Past deadline = SIGTERM.
7. **Step approval pour writes** — toute mutation (curation_promote, retune_trigger, gdpr_erase, deploy) bloque jusqu'à approval explicite.
8. **PII redaction obligatoire** — agent inputs et outputs passent par `efe-pii::redact` avant audit log.
9. **Network deny-all par défaut** — egress whitelist explicite en NetworkPolicy ; tout autre traffic = blocked + alerted.
10. **Audit chain HMAC unbroken** — agent_audit chained avec parent reference ; verify_chain doit retourner OK pour every promoted run output.
11. **DPIA gate** — chaque tenant doit avoir signé le DPIA M25 + addendum DPA avant tout AgentRun submitted.

### 4.5 Différenciateurs stratégiques

M25 livre **3 capacités uniques** vs concurrentiel :

1. **Tenant-isolated sandboxing avec audit chain HMAC** — aucun fournisseur n'offre ça en commercial.
2. **In-band step approval** — opérateur peut pause un agent run mid-flight, voir l'état, approuver step-par-step.
3. **DPIA workflow + AI Act Art. 9 compliance built-in** — pour les régulateurs EU, c'est une posture que les concurrents US ne peuvent pas matcher facilement.

---

## 5. Sandboxing architecture (Firecracker microVM)

### 5.1 Pourquoi Firecracker (vs alternatives)

| Approche | Isolation | Startup | Compute overhead | Choix M25 |
|---|---|---|---|---|
| Container + seccomp + AppArmor | Faible (shared kernel) | < 50 ms | Minimal | ❌ trop faible vs hostile LLM |
| gVisor user-space kernel | Moyenne | ~150 ms | ~3-10 % | ⚠ alternative considérée |
| **Firecracker microVM** | **Élevée (KVM hypervisor)** | **~125 ms** | **~5 %** | ✅ **CHOIX** |
| QEMU full VM | Très élevée | ~5-10 s | ~10-20 % | ❌ trop lent |
| WASM (WASI) runtime | Moyenne (deterministic) | < 10 ms | minimal mais limité | ❌ pas de full Linux LLM clients |

**Justification Firecracker** :
- Conçu pour serverless multi-tenant (AWS Lambda).
- Hardware-virtualization-based isolation (KVM).
- Startup ~125 ms.
- Memory footprint ~5 MB per microVM (jeevon-base).
- Well-supported : OSS, Apache 2.0.
- Battle-tested at AWS scale.

### 5.2 microVM provisioning lifecycle

```
1. Pre-warmed pool : N microVMs idle per tenant (default N=2)
   - Fresh kernel + readonly rootfs + tmpfs /tmp
   - No tenant context yet (generic agent base image)

2. AgentRun submitted : controller picks a microVM from pool
   - Inject tenant_id + run_id + LLM API key (via vsock)
   - Apply NetworkPolicy (Calico CNI)
   - Mount readonly /efe-templates volume (signed templates)
   - Start agent_core process

3. Run executes : agent loops LLM ↔ tools

4. Run completes (success or failure or timeout) :
   - SIGTERM agent_core (graceful 30s grace period)
   - If SIGKILL needed (cost or escape) : immediate SIGKILL
   - Capture final state + agent_state checkpoint to DB
   - Destroy microVM (memory wipe, vCPU cleanup)
   - Pool replenishes one slot

5. Adversarial scan post-run :
   - Inspect microVM exit logs for anomalies
   - Check for escape attempts (NetworkPolicy denied logs)
   - Alert if suspicious patterns
```

### 5.3 Resource limits (per microVM)

| Resource | Default | Tenant max override |
|---|---:|---:|
| vCPU | 1 | 4 |
| Memory | 2 GiB | 8 GiB |
| /tmp tmpfs | 256 MiB | 1 GiB |
| Network egress bandwidth | 10 Mbps | 50 Mbps |
| Open file descriptors | 1024 | 4096 |
| Max processes | 64 | 256 |
| `timeout_min` | 60 | 240 |

Limits enforced via K8s ResourceQuota + Firecracker config + cgroup v2.

### 5.4 Network policy (deny-all + whitelist)

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: agent-run-deny-all-except-whitelist
  namespace: efe-agent-runs-{{tenant}}
spec:
  podSelector:
    matchLabels:
      app: efe-agent-runner
      run-id: {{run_id}}
  policyTypes:
    - Egress
    - Ingress
  egress:
    # Allow LLMRouter ClusterIP only.
    - to:
        - namespaceSelector:
            matchLabels:
              app: efe-llm-router
          podSelector:
            matchLabels:
              app: efe-llm-router
      ports:
        - protocol: TCP
          port: 50061
    # Allow mcp-server ClusterIP only.
    - to:
        - namespaceSelector:
            matchLabels:
              app: efe-mcp-server
          podSelector:
            matchLabels:
              app: efe-mcp-server
      ports:
        - protocol: TCP
          port: 8081
    # Allow audit backend.
    - to:
        - namespaceSelector:
            matchLabels:
              app: efe-server
          podSelector:
            matchLabels:
              component: audit-backend
      ports:
        - protocol: TCP
          port: 8082
    # DNS resolution (kube-system DNS only).
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
  ingress: []  # Deny all ingress (agent doesn't expose services).
```

### 5.5 Filesystem layout

```
/                  ← readonly (rootfs from official efe-agent image)
  /usr             ← readonly
  /etc             ← readonly
  /var             ← readonly
  /opt             ← readonly
/tmp               ← tmpfs, 256 MiB, ephemeral, wiped post-run
/etc/efe/          ← readonly mount of run-specific config
  prompt.txt       ← agent prompt
  allowed_tools.json  ← whitelist
  api_endpoints.json  ← LLMRouter + mcp-server URLs
/var/run/efe/secrets/  ← readonly mount, in-memory tmpfs from BYOK
  llm_api_key      ← injected at VM-create
```

No persistent storage volumes mounted. Agent state durability via DB only (cf. § 10).

### 5.6 Pin tests sandbox

- `test_microvm_starts_under_200ms` (perf)
- `test_microvm_cannot_modify_rootfs` (immutability)
- `test_microvm_cannot_egress_outside_whitelist` (network)
- `test_microvm_tmpfs_wiped_post_run`
- `test_microvm_resource_limits_enforced` (cgroup)
- `test_microvm_image_whitelist_rejects_unknown` (signature)
- `test_microvm_tenant_id_immutable` (rejection on tampering)
- `test_microvm_destroyed_on_sigterm`
- `test_microvm_pool_replenishes`
- `test_microvm_escape_attempt_detected_and_alerted` (adversarial)

---

## 6. Agent runtime architecture

### 6.1 Control plane vs data plane

```
┌─────────────────────────────────────────────────────────────────┐
│                      CONTROL PLANE                              │
│                                                                 │
│  efe-agent-runner-controller (StatefulSet, 3 replicas, leader   │
│  election via Kubernetes lease)                                  │
│                                                                 │
│  Responsibilities :                                             │
│   - Watch AgentRun CRDs                                         │
│   - Reconcile : PENDING → spawning → RUNNING → terminated       │
│   - Manage microVM pool                                         │
│   - Enforce cost circuit breakers                               │
│   - Stream agent state to portal-admin                          │
│   - Cleanup : evict completed/failed runs                       │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                       DATA PLANE                                │
│                                                                 │
│  AgentRunPod × N (one per active AgentRun)                      │
│                                                                 │
│  Each Pod hosts a Firecracker microVM running :                 │
│   - agent_core (Python) — LLM-driven loop                       │
│   - sidecar : metrics/log shipping (read-only mounted)          │
│                                                                 │
│  Pods are created by the controller, never directly by users.  │
│  Pods run for the lifetime of the AgentRun (max timeout_min).   │
└─────────────────────────────────────────────────────────────────┘
```

### 6.2 Agent core loop (Python)

```python
# crates/efe-agent-runner/agent_core/main.py

async def main():
    cfg = load_config_from_env()  # tenant_id, run_id, prompt, allowed_tools
    audit_chain = AuditChain.connect(cfg.audit_backend, cfg.tenant_id, cfg.run_id)
    llm_client = LLMRouterClient(cfg.llm_router_url, cfg.api_key)
    mcp_client = McpClient(cfg.mcp_server_url, cfg.run_id, cfg.allowed_tools)
    cost_tracker = CostTracker(cfg.max_cost_usd)

    conversation = [{"role": "system", "content": SYSTEM_PROMPT}]
    conversation.append({"role": "user", "content": cfg.prompt})

    step_n = 0
    while step_n < MAX_STEPS:  # safety cap
        step_n += 1
        # 1. LLM call with tool definitions
        try:
            llm_resp = await llm_client.call_with_tools(
                conversation,
                tools=mcp_client.tool_definitions(),
                budget_category="agent_runtime",
                cost_tracker=cost_tracker,
            )
        except BudgetExceeded:
            await audit_chain.append("budget_exceeded", {"step": step_n})
            sys.exit(EXIT_BUDGET_EXCEEDED)

        await audit_chain.append("llm_call_completed", {
            "step": step_n,
            "input_tokens": llm_resp.input_tokens,
            "output_tokens": llm_resp.output_tokens,
            "cost_cents": llm_resp.cost_cents,
        })

        # 2. Process LLM response
        if llm_resp.is_final_output():
            await audit_chain.append("final_output", {
                "step": step_n,
                "output_redacted": pii_redact(llm_resp.text),
            })
            await emit_to_portal(cfg.run_id, "completed", llm_resp.text)
            sys.exit(EXIT_OK)

        if llm_resp.has_tool_call():
            tool_call = llm_resp.tool_call
            # 3. Verify whitelist
            if tool_call.name not in cfg.allowed_tools:
                await audit_chain.append("tool_call_rejected_whitelist", {
                    "tool": tool_call.name, "step": step_n,
                })
                conversation.append({
                    "role": "tool",
                    "tool_call_id": tool_call.id,
                    "content": f"ERROR : tool '{tool_call.name}' not in whitelist",
                })
                continue

            # 4. If write tool, request approval
            if mcp_client.is_write_tool(tool_call.name):
                approval = await request_approval(cfg.run_id, step_n, tool_call)
                if not approval.granted:
                    await audit_chain.append("tool_call_rejected_approval", {
                        "tool": tool_call.name, "approver": approval.actor,
                    })
                    conversation.append({
                        "role": "tool",
                        "tool_call_id": tool_call.id,
                        "content": f"ERROR : approval denied — {approval.reason}",
                    })
                    continue

            # 5. Execute tool call via mcp-server
            tool_result = await mcp_client.call(tool_call)
            await audit_chain.append("tool_call_completed", {
                "tool": tool_call.name, "step": step_n,
                "input_redacted": pii_redact(json.dumps(tool_call.arguments)),
                "output_redacted": pii_redact(json.dumps(tool_result.content)),
            })
            conversation.append({
                "role": "tool",
                "tool_call_id": tool_call.id,
                "content": tool_result.content,
            })

    # MAX_STEPS reached
    await audit_chain.append("max_steps_reached", {"max": MAX_STEPS})
    sys.exit(EXIT_MAX_STEPS)
```

### 6.3 SYSTEM_PROMPT (versioned)

```
You are an EFE agent. Your tenant is '{{tenant_id}}', your run id is
'{{run_id}}'. You operate inside a sandboxed Firecracker microVM with
strict tool whitelist.

CRITICAL CONSTRAINTS :
1. You can ONLY call tools listed in your whitelist. Other tool names
   will be rejected.
2. Write tools require human approval ; expect a delay of seconds-to-
   hours when calling them.
3. Your time budget is {{timeout_min}} minutes. Plan accordingly.
4. Your cost budget is ${{max_cost_usd}}. Each LLM call deducts from it.
5. Do not attempt to access external APIs, read filesystem outside /tmp,
   or modify your runtime environment. These will fail and may
   trigger termination.
6. Output PII is redacted before audit logging — assume your responses
   are stored.
7. When you have completed the task, emit a final output via the
   designated end-of-run signal (no special tool call ; the LLM
   "is_final_output" handler detects via end markers).

Your task :
{{prompt}}
```

### 6.4 Pin tests agent core

- `test_agent_loop_completes_within_max_steps`
- `test_agent_rejects_non_whitelist_tool`
- `test_agent_pauses_on_write_approval_request`
- `test_agent_aborts_on_budget_exceeded`
- `test_agent_aborts_on_timeout`
- `test_agent_emits_final_output_correctly`
- `test_agent_audit_chain_unbroken_full_run`
- `test_agent_pii_redacted_before_audit`

---

## 7. Tool access control

### 7.1 Whitelist enforcement points

```
┌─────────────────────────────────────────────────────────┐
│  Layer 1 — AgentRun creation (control plane)            │
│  ─────────────────────────────────────────────          │
│  - tenant.allowed_tools must be subset of               │
│    AgentRun.allowed_tools                               │
│  - REJECT if AgentRun.allowed_tools is empty            │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│  Layer 2 — Agent core (in microVM)                      │
│  ─────────────────────────────────                       │
│  - LLM tool call → check name in cfg.allowed_tools      │
│  - if not in list → return error to LLM                 │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│  Layer 3 — mcp-server (server-side)                     │
│  ────────────────────────────────                        │
│  - reads X-EFE-Agent-Run-Id from gRPC metadata          │
│  - looks up AgentRun.allowed_tools from DB              │
│  - if tool not in list → PERMISSION_DENIED              │
│  - if write tool → check approval token                 │
└─────────────────────────────────────────────────────────┘
```

3-layer defense-in-depth. Any single bypass requires breaking 2 other layers + audit chain detection.

### 7.2 Profiles (predefined whitelist sets)

```yaml
# config/agent-profiles.yaml (signed Sigstore)

profiles:
  audit-readonly:
    description: "Audit operations, no writes."
    allowed_tools:
      - efe_classify
      - efe_audit_health_check
      - efe_get_audit_log
      - efe_list_available_bundles
      - efe_inspect_bundle
      - efe_get_sbom
      - efe_diff_bundles
      - efe_audit_bundle_vs_gold      # M23 (read-only audit mode)

  taxonomy-refactor:
    description: "Curation tools, all writes require approval."
    allowed_tools:
      - efe_classify
      - efe_propose_taxonomy_split    # M23 write
      - efe_suggest_class_merge        # M23 write
      - efe_explain_misclassification  # M23 read
      - efe_compare_classes
      - efe_suggest_classes
    requires_approval: ["efe_propose_taxonomy_split", "efe_suggest_class_merge"]

  runbook-author:
    description: "Read-only access to incident logs + write runbook drafts."
    allowed_tools:
      - efe_get_audit_log
      - efe_get_deployment_history
      - efe_score_distribution
      - efe_diagnose_catalog

  experimental:
    description: "Full mcp-server access (NOT for production)."
    allowed_tools: ["*"]
    requires_approval: ["*"]
    feature_flag_required: "EFE_FEATURE_M25_EXPERIMENTAL_PROFILE"
```

### 7.3 Pin tests tool ACL

- `test_tool_call_outside_whitelist_rejected_layer1`
- `test_tool_call_outside_whitelist_rejected_layer2`
- `test_tool_call_outside_whitelist_rejected_layer3`
- `test_audit_logs_all_3_layers_rejection`
- `test_profile_inheritance_correct`
- `test_experimental_profile_requires_explicit_flag`

---

## 8. Step approval

### 8.1 Workflow

```
Agent (in microVM)                     mcp-server               Operator (portal-admin)
       │                                     │                          │
       │  ToolCall(efe_propose_taxonomy_split,│                          │
       │           args={...})                │                          │
       │ ────────────────────────────────────►│                          │
       │                                     │                          │
       │                                     │ check whitelist ✓         │
       │                                     │ is_write=true → request  │
       │                                     │ approval                 │
       │                                     │ INSERT agent_approval_   │
       │                                     │   request (status=PENDING)│
       │                                     │                          │
       │                                     │ status PAUSED            │
       │                                     │  ──── (HTTP 202 Accepted)│
       │  PENDING_APPROVAL response           │                          │
       │ ◄────────────────────────────────────│                          │
       │                                     │  ──────► SSE notify ────►│
       │                                     │                          │  see approval
       │  agent core polls every 30s         │                          │  in UI
       │                                     │                          │
       │                                     │                          │  Click ✓ Approve
       │                                     │                          │  + rationale
       │                                     │ ◄────── grpc Approve ───│
       │                                     │ approval_token issued    │
       │                                     │ status APPROVED          │
       │                                     │                          │
       │  on next poll, sees APPROVED         │                          │
       │  retries the tool call with token   │                          │
       │ ────────────────────────────────────►│                          │
       │                                     │ verify token, execute    │
       │                                     │ tool                     │
       │                                     │ APPEND audit             │
       │                                     │ return result            │
       │ ◄────────────────────────────────────│                          │
       │                                     │                          │
       │  agent continues loop                │                          │
```

### 8.2 Approval request schema

```rust
pub struct AgentApprovalRequest {
    pub id: Uuid,
    pub agent_run_id: Uuid,
    pub step_n: u32,
    pub tool_name: String,
    pub tool_arguments_redacted: serde_json::Value,
    pub status: ApprovalStatus,         // PENDING, APPROVED, REJECTED, EXPIRED
    pub requested_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
    pub responder_oidc_sub: Option<String>,
    pub responder_role: Option<String>,
    pub rationale: Option<String>,
    pub approval_token: Option<String>,  // single-use, expires in 10 min
    pub expires_at: DateTime<Utc>,        // default : NOW() + 1h
}
```

### 8.3 Approval expiry + agent fallback

If approval not given within `expires_at` (default 1h) :
- Approval status → EXPIRED
- Agent receives "TOOL_DENIED_EXPIRED" on next poll
- Agent can choose to abort or retry with different approach
- audit row appended

### 8.4 Approval bypass : NEVER

There is no bypass for write tool approval. Even tenant_admin must
explicitly approve each step. No "approve all writes" toggle. This is
the AI Act Art. 14 hard constraint.

### 8.5 Pin tests step approval

- `test_write_tool_blocks_until_approval_or_expiry`
- `test_approval_token_single_use_only`
- `test_approval_expires_after_1h_if_no_response`
- `test_approval_rationale_min_10_chars`
- `test_approval_records_responder_oidc_sub`
- `test_no_bypass_via_tenant_admin_role`
- `test_approval_audit_chain_link`

---

## 9. Long-running pause / resume / checkpointing

### 9.1 Use cases pour pause/resume

- **Operator interruption** : opérateur pause un run pour ajuster context, ré-évaluer
- **Cost throttle** : run approche `max_cost_usd`, pause + extend cap if approved
- **Maintenance window** : pause during cluster upgrade
- **Long approval wait** : run paused while awaiting approval > 1h (rare ; usually times out)

### 9.2 Checkpoint mechanism

Au pause request :
1. Agent core reçoit SIGUSR1
2. Serializes : conversation history, tool call results, intermediate variables, step counter, cost spent so far
3. Writes to `agent_state` row (Postgres, encrypted via KMS per-tenant)
4. Emits `agent_audit` row (event=run_paused)
5. Process exits with EXIT_PAUSED
6. microVM destroyed (memory reclaimed)
7. AgentRun.status → PAUSED

Au resume :
1. Operator clicks "Resume" in portal-admin
2. Controller spawns NEW microVM (fresh)
3. Inject tenant_id + run_id + saved checkpoint state via vsock
4. Agent core deserializes, resumes loop from saved step
5. Emits `agent_audit` row (event=run_resumed)
6. Status → RUNNING

### 9.3 State serialization format

```json
{
  "version": 1,
  "run_id": "uuid-here",
  "checkpoint_at": "2027-12-01T10:30:45Z",
  "step_n": 17,
  "cost_spent_cents": 1240,
  "conversation": [
    { "role": "system", "content": "..." },
    { "role": "user", "content": "..." },
    { "role": "assistant", "content": "..." },
    ...
  ],
  "pending_approval_id": null,
  "user_data": { /* opaque agent-specific */ }
}
```

Encrypted at rest via per-tenant KMS key (BYOK M19). Stored in `agent_state.encrypted_blob` column (bytea).

### 9.4 Resume safety invariants

- Resume only re-spawns same `agent_run_id` — never duplicates
- Cost spent so far carried forward
- Tool whitelist and timeout cap unchanged
- `step_n` continues from checkpoint (not reset)
- Approval requests pending at pause-time remain valid (expiry recomputed against resume time)

### 9.5 Pin tests pause/resume

- `test_pause_serializes_conversation_history`
- `test_resume_restores_step_counter`
- `test_resume_carries_cost_spent`
- `test_resume_uses_fresh_microvm`
- `test_resume_immutable_tool_whitelist`
- `test_pause_resume_audit_chain_link`
- `test_pause_during_pending_approval_preserves_request`
- `test_resume_after_24h_still_works`

---

## 10. Agent state + memory

### 10.1 Two storage tiers

```
┌─────────────────────────────────────────────────────────────────┐
│  Tier 1 — Per-run state (always)                                │
│                                                                 │
│  Table : agent_state                                            │
│  - one row per AgentRun id                                      │
│  - holds checkpoint blob (encrypted)                            │
│  - retention : 90 days post-run completion (configurable)       │
│  - GDPR Art. 17 cascade : tombstone on subject erasure          │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  Tier 2 — Cross-run memory (opt-in)                             │
│                                                                 │
│  Table : agent_memory                                           │
│  - tenant-scoped, NOT cross-tenant                              │
│  - keys : (tenant_id, memory_namespace, key)                    │
│  - tenant_admin OPT-IN explicitly                               │
│  - retention : tenant-controlled (default 30 days)              │
│  - encryption : per-tenant KMS                                  │
│  - access controlled : agent_run.allowed_memory_namespaces      │
└─────────────────────────────────────────────────────────────────┘
```

### 10.2 Cross-run memory schema

```sql
CREATE TABLE agent_memory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    memory_namespace TEXT NOT NULL CHECK (length(memory_namespace) <= 64),
    key TEXT NOT NULL CHECK (length(key) <= 256),
    value_encrypted BYTEA NOT NULL,
    written_by_agent_run_id UUID NOT NULL REFERENCES agent_run(id),
    written_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    UNIQUE(tenant_id, memory_namespace, key)
);
```

### 10.3 Tools to access memory (whitelisted per profile)

- `efe_agent_memory_read(namespace, key)` — read-only
- `efe_agent_memory_write(namespace, key, value, ttl_days)` — write (requires approval if cross-run)
- `efe_agent_memory_list(namespace)` — list keys

### 10.4 Pin tests memory

- `test_per_run_state_isolated_from_other_runs`
- `test_cross_run_memory_tenant_scoped`
- `test_cross_run_memory_namespace_isolation`
- `test_memory_encrypted_at_rest`
- `test_memory_gdpr_art17_cascade`
- `test_memory_expires_at_ttl`
- `test_memory_write_requires_approval_token`

---

## 11. Cost governance M25

### 11.1 Budget hierarchy extension

```
Tenant total budget : $X / month (hard cap)
├── classify (Mode-1 + Mode-2) : $A
├── multi-turn (Mode-3) : $B
├── curation (M23) : $C
├── active_learning (M24) : $D
└── agent_runtime (M25) : $E
    ├── per agent run : `max_cost_usd` (default $50, max $500)
    ├── compute (microVM hours) : tenant-priced
    └── network egress : tenant-priced
```

### 11.2 Cost components per run

| Component | Tracking |
|---|---|
| LLM tokens (via M21 router) | Real-time, dedicated to `agent_runtime` category |
| microVM compute | Pre-allocated based on `timeout_min` × cpu_per_min rate |
| Tool call overhead | Negligible per call ; aggregated |
| Network egress | Bytes-based metering |

### 11.3 Circuit breaker

```
Cost spent reaches 80 % of max_cost_usd :
  → emit warning to portal-admin
  → tagged metric AL_COST_NEAR_CAP_TOTAL
  → operator can choose to extend cap (requires admin role + audit)

Cost spent reaches 100 % of max_cost_usd :
  → SIGTERM the microVM immediately (no grace period for LLM calls)
  → Run status → TERMINATED_COST_CAP
  → Final state checkpoint to DB
  → audit row appended
```

### 11.4 Pin tests cost

- `test_cost_tracker_decrements_per_llm_call`
- `test_cost_warning_at_80pct`
- `test_cost_circuit_breaker_at_100pct`
- `test_cost_extension_requires_admin_audit`
- `test_cost_per_category_isolated`

---

## 12. Architectural Decision Records

### ADR-M25-001 : Firecracker over gVisor

**Décision** : Firecracker microVM.

**Justifications** : kernel-level isolation, hardware virtualization,
mature, AWS Lambda-proven, bonne perf (~125 ms startup, 5% overhead).

### ADR-M25-002 : Helm chart séparé `efe-agent-runner` (vs. inclus dans efe-stack)

**Décision** : chart séparé, OPT-IN total. Default OFF.

**Justifications** :
1. **Surface attack** : tenants qui ne veulent pas M25 ne payent pas l'overhead K8s.
2. **Compliance posture** : tenants peuvent installer EFE sans M25 et conserver leur posture compliance non-AI-Act-Art.9.
3. **Sunset path** : si M25 expérimental sunset, pas de migration EFE 2.0 → 1.x à faire.

### ADR-M25-003 : Step approval mandatory pour writes

**Décision** : aucune mutation sans approval humain inline.

**Justifications** : AI Act Art. 14 — high-risk autonomous AI systems
must have human oversight. Step approval = enforcement.

### ADR-M25-004 : Tool whitelist (vs. blacklist)

**Décision** : whitelist obligatoire ; empty = no tool callable.

**Justifications** : default-deny. Add new tool = explicit decision.

### ADR-M25-005 : Cost cap hard SIGTERM (vs. graceful)

**Décision** : SIGTERM immediate at 100 % cap. Aucun grace period.

**Justifications** : LLM peut empiler les calls dans un loop 1s → 1000$
en quelques minutes. SIGTERM = seule garantie hard.

### ADR-M25-006 : Bump semver MAJOR (1.x → 2.0)

**Décision** : EFE 2.0.0 obligatoire pour M25.

**Justifications** : nouveau service + chart + CRDs = changement de
contract product. Customers EFE 1.x peuvent rester 1.x indéfiniment.

### ADR-M25-007 : DPIA gate per-tenant

**Décision** : tenant doit avoir signé DPIA M25 avant tout AgentRun.

**Justifications** : AI Act Art. 35 GDPR + Art. 9 AI Act explicit
requirement. Nous ne pouvons pas activer le runtime sans cela.

### ADR-M25-008 : Image whitelist (vs. tenant custom images)

**Décision** : seules images officielles `efe-agent:*` (signées).

**Justifications** : tenant custom code = nouveau threat surface ;
out-of-scope V1.

### ADR-M25-009 : Pre-warmed microVM pool (vs. on-demand)

**Décision** : pool de N microVMs pré-spawned par tenant.

**Justifications** : startup ~125 ms est OK mais pour use-case
batch-many-runs, pool évite re-init overhead. Default N=2.

### ADR-M25-010 : Adversarial bench obligatoire au GA gate

**Décision** : sandbox escape detection ≥ 100 % bloque GA.

**Justifications** : tout escape = catastrophic incident risk. Pas de
warn-only — c'est un fail.

### ADR-M25-011 : Cross-run memory OPT-IN explicite

**Décision** : tenants doivent activer `tenant_feature.m25_memory_enabled` séparément.

**Justifications** : memory cross-run = surface fuite plus large.
Default OFF.

### ADR-M25-012 : Annual security audit obligatoire

**Décision** : tenants utilisant M25 doivent re-confirmer via
addendum DPA annuel.

**Justifications** : statut "experimental" + AI Act Art. 9 require
ongoing risk management ; annual cadence aligned with SOC 2 + ISO 27001.

### ADR-M25-013 : Sunset clause

**Décision** : si < 2 tenants pilots à 12 mois, M25 sunset.

**Justifications** : maintenance burden d'un module experimental sans
adoption ne se justifie pas.

### ADR-M25-014 : Streaming output via SSE (vs. WebSocket vs. gRPC streaming)

**Décision** : Server-Sent Events (SSE) pour live tail UI.

**Justifications** : SSE = simple, bidirectional pas requis, fonctionne
natif HTTP/2, déjà utilisé par M19 progress events.

### ADR-M25-015 : Encryption per-tenant pour agent_state

**Décision** : KMS per-tenant key (BYOK M19) pour chiffrer agent_state.encrypted_blob.

**Justifications** : agent state contient conversation history avec
potentiellement des extracts de production data. KMS isolé par tenant.

---

## 13. Data model

### 13.1 Migration `0016_agent_runtime.sql` (extrait — full version § F)

```sql
BEGIN;

-- ─── ENUM types ──────────────────────────────────────────────────

CREATE TYPE agent_run_status AS ENUM (
    'PENDING', 'PROVISIONING', 'RUNNING', 'PAUSED', 'AWAITING_APPROVAL',
    'COMPLETED', 'FAILED', 'TIMED_OUT', 'TERMINATED_COST_CAP',
    'TERMINATED_USER', 'TERMINATED_SECURITY'
);

CREATE TYPE agent_approval_status AS ENUM (
    'PENDING', 'APPROVED', 'REJECTED', 'EXPIRED'
);

-- ─── agent_run_template (signed Sigstore) ──────────────────────────

CREATE TABLE agent_run_template (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL CHECK (version ~ '^[0-9]+\.[0-9]+\.[0-9]+$'),
    prompt_markdown TEXT NOT NULL,
    allowed_tools TEXT[] NOT NULL CHECK (cardinality(allowed_tools) >= 0),
    profile_name TEXT NOT NULL,
    cosign_signature_url TEXT,
    rekor_log_index BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deprecated_at TIMESTAMPTZ,
    UNIQUE(tenant_id, name, version)
);

-- ─── agent_run ─────────────────────────────────────────────────────

CREATE TABLE agent_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    template_id UUID REFERENCES agent_run_template(id),
    raw_prompt TEXT,
    allowed_tools TEXT[] NOT NULL,
    max_cost_usd DOUBLE PRECISION NOT NULL CHECK (max_cost_usd > 0 AND max_cost_usd <= 500),
    timeout_minutes INT NOT NULL CHECK (timeout_minutes > 0 AND timeout_minutes <= 240),
    submitted_by TEXT NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    status agent_run_status NOT NULL DEFAULT 'PENDING',
    cost_cents_spent BIGINT NOT NULL DEFAULT 0,
    step_count INT NOT NULL DEFAULT 0,
    final_output_redacted TEXT,
    failure_reason TEXT,
    microvm_id TEXT,
    k8s_pod_name TEXT,
    profile_name TEXT NOT NULL,
    -- DPIA evidence : signed at submission
    dpia_addendum_hash TEXT NOT NULL,
    CONSTRAINT raw_prompt_or_template CHECK (
        (raw_prompt IS NOT NULL AND template_id IS NULL)
        OR (raw_prompt IS NULL AND template_id IS NOT NULL)
    )
);

CREATE INDEX agent_run_active_idx
    ON agent_run(tenant_id, status, submitted_at)
    WHERE status IN ('PENDING', 'PROVISIONING', 'RUNNING', 'PAUSED', 'AWAITING_APPROVAL');

-- ─── agent_step ────────────────────────────────────────────────────

CREATE TABLE agent_step (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_run_id UUID NOT NULL REFERENCES agent_run(id) ON DELETE RESTRICT,
    step_n INT NOT NULL CHECK (step_n > 0),
    kind TEXT NOT NULL CHECK (kind IN ('llm_call', 'tool_call', 'approval_request', 'approval_response', 'final_output', 'pause', 'resume')),
    tool_name TEXT,
    arguments_redacted JSONB,
    result_redacted JSONB,
    cost_cents BIGINT,
    duration_ms INT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'IN_PROGRESS',
    UNIQUE(agent_run_id, step_n)
);

CREATE INDEX agent_step_run_idx ON agent_step(agent_run_id, step_n);

-- ─── agent_approval_request ────────────────────────────────────────

CREATE TABLE agent_approval_request (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_run_id UUID NOT NULL REFERENCES agent_run(id) ON DELETE RESTRICT,
    step_n INT NOT NULL,
    tool_name TEXT NOT NULL,
    arguments_redacted JSONB NOT NULL,
    status agent_approval_status NOT NULL DEFAULT 'PENDING',
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    responded_at TIMESTAMPTZ,
    responder_oidc_sub TEXT,
    responder_role TEXT,
    rationale TEXT CHECK (rationale IS NULL OR length(rationale) >= 10),
    approval_token_hash TEXT,
    approval_token_used_at TIMESTAMPTZ
);

CREATE INDEX agent_approval_pending_idx
    ON agent_approval_request(status, expires_at)
    WHERE status = 'PENDING';

-- ─── agent_state (encrypted checkpoint) ────────────────────────────

CREATE TABLE agent_state (
    agent_run_id UUID PRIMARY KEY REFERENCES agent_run(id) ON DELETE CASCADE,
    encrypted_blob BYTEA NOT NULL,
    kms_key_alias TEXT NOT NULL,
    checkpoint_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    blob_sha256 TEXT NOT NULL CHECK (blob_sha256 ~ '^[a-f0-9]{64}$')
);

-- ─── agent_memory (cross-run, opt-in) ──────────────────────────────

CREATE TABLE agent_memory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    memory_namespace TEXT NOT NULL CHECK (length(memory_namespace) <= 64),
    key TEXT NOT NULL CHECK (length(key) <= 256),
    value_encrypted BYTEA NOT NULL,
    kms_key_alias TEXT NOT NULL,
    written_by_agent_run_id UUID NOT NULL REFERENCES agent_run(id),
    written_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    UNIQUE(tenant_id, memory_namespace, key)
);

-- ─── agent_audit (HMAC chained, mirrors learning_audit) ────────────

CREATE TABLE agent_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    parent_id UUID,
    agent_run_id UUID REFERENCES agent_run(id),
    agent_step_id UUID REFERENCES agent_step(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'run_created', 'microvm_provisioned', 'run_started',
        'llm_call_completed', 'tool_call_attempted', 'tool_call_completed',
        'tool_call_rejected_whitelist', 'tool_call_rejected_approval',
        'approval_requested', 'approval_granted', 'approval_rejected',
        'approval_expired',
        'pause_requested', 'run_paused', 'run_resumed',
        'cost_warning_80pct', 'cost_cap_exceeded', 'budget_exceeded',
        'sandbox_escape_attempt_detected',
        'final_output', 'run_completed', 'run_terminated',
        'run_failed', 'run_timed_out',
        'gdpr_art17_cascade'
    )),
    actor_id TEXT NOT NULL,
    actor_role TEXT,
    payload_json JSONB NOT NULL,
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[a-f0-9]{64}$'),
    hmac TEXT NOT NULL CHECK (hmac ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT agent_audit_parent_fk
        FOREIGN KEY (parent_id) REFERENCES agent_audit(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX agent_audit_chain_idx ON agent_audit(tenant_id, created_at);
CREATE INDEX agent_audit_run_idx ON agent_audit(agent_run_id) WHERE agent_run_id IS NOT NULL;

-- ─── tenant_feature M25 columns ───────────────────────────────────

ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS m25_agent_runtime_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m25_dpia_signed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m25_dpia_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m25_cost_cap_per_run_usd DOUBLE PRECISION DEFAULT 0,
    ADD COLUMN IF NOT EXISTS m25_timeout_minutes_max INT DEFAULT 60,
    ADD COLUMN IF NOT EXISTS m25_allowed_profiles TEXT[] DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS m25_allowed_providers TEXT[] DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS m25_memory_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m25_microvm_pool_size INT DEFAULT 2,
    ADD COLUMN IF NOT EXISTS m25_max_concurrent_runs INT DEFAULT 5;

-- ─── RLS ──────────────────────────────────────────────────────────

ALTER TABLE agent_run_template ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_step ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_approval_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_memory ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_audit ENABLE ROW LEVEL SECURITY;

-- ... policies similar to M23/M24 patterns ...

COMMIT;
```

### 13.2 Storage estimate

| Table | Rows/tenant/an (estim. pilot) | Total/tenant/an |
|---|---:|---:|
| `agent_run_template` | 5-50 | ~1 MB |
| `agent_run` | 100-1K | ~5 MB |
| `agent_step` | 5K-100K | ~200 MB |
| `agent_approval_request` | 500-10K | ~10 MB |
| `agent_state` | 100-1K | ~50 MB |
| `agent_memory` | 0-10K (opt-in) | ~50 MB |
| `agent_audit` | 50K-1M | ~2 GB |
| **Total/tenant/an** | | **~2.3 GB** |

À 5 tenants pilot × 5 GB compliance buffer = ~12 GB/an. Negligible.

---

## 14. State machines

### 14.1 AgentRun lifecycle

```
                ┌──────────┐
                │ PENDING  │
                └────┬─────┘
                     │ controller picks up
                     ▼
            ┌──────────────────┐
            │ PROVISIONING     │ (microVM spawning, ≤ 30s)
            └────┬─────────────┘
                 │
        ┌────────┼─────────┐
        │ ok     │ fail    │
        ▼        │         ▼
┌───────────┐    │   ┌────────┐
│ RUNNING   │    │   │ FAILED │ (terminal)
└──┬─────┬──┘    │   └────────┘
   │     │       │
   │     │       │
   │     ▼       │
   │ ┌───────────────────┐
   │ │ AWAITING_APPROVAL │
   │ └─────┬─────────────┘
   │       │ approved → resume RUNNING
   │       │ expired → continue (with deny)
   │       │ rejected → continue (with deny)
   │       │ user pause → PAUSED
   │
   │ ┌──────────┐
   │ │ PAUSED   │ ← user-initiated pause
   │ └────┬─────┘
   │      │ resume → RUNNING
   │      │ terminate → TERMINATED_USER
   │
   ├─→ COMPLETED (terminal)
   ├─→ TIMED_OUT (terminal, hit timeout_minutes)
   ├─→ TERMINATED_COST_CAP (terminal, cost circuit breaker)
   ├─→ TERMINATED_USER (terminal, operator action)
   ├─→ TERMINATED_SECURITY (terminal, sandbox escape detected)
   └─→ FAILED (terminal, microVM crash etc.)
```

### 14.2 Approval state machine

```
PENDING ──► APPROVED (granted, token issued, not yet used)
        ├─► REJECTED (operator denied)
        └─► EXPIRED (timed out at expires_at)

APPROVED ──► CONSUMED (token used, agent unblocked)
```

Token is single-use ; once consumed, cannot be replayed.

---

## 15. gRPC + Kubernetes CRD

### 15.1 gRPC `AgentRuntimeService` (14 RPCs)

```protobuf
service AgentRuntimeService {
    // ── Run management ──────────────────────────────────────────────
    rpc CreateAgentRun(CreateAgentRunRequest) returns (CreateAgentRunResponse);
    rpc GetAgentRun(GetAgentRunRequest) returns (GetAgentRunResponse);
    rpc ListAgentRuns(ListAgentRunsRequest) returns (ListAgentRunsResponse);
    rpc TerminateAgentRun(TerminateAgentRunRequest) returns (TerminateAgentRunResponse);
    rpc PauseAgentRun(PauseAgentRunRequest) returns (PauseAgentRunResponse);
    rpc ResumeAgentRun(ResumeAgentRunRequest) returns (ResumeAgentRunResponse);

    // ── Step inspection (live tail) ─────────────────────────────────
    rpc StreamAgentSteps(StreamAgentStepsRequest)
        returns (stream AgentStepEvent);
    rpc GetAgentStep(GetAgentStepRequest) returns (GetAgentStepResponse);

    // ── Approval ────────────────────────────────────────────────────
    rpc ListPendingApprovals(ListPendingApprovalsRequest)
        returns (ListPendingApprovalsResponse);
    rpc ApproveAgentStep(ApproveAgentStepRequest) returns (ApproveAgentStepResponse);
    rpc RejectAgentStep(RejectAgentStepRequest) returns (RejectAgentStepResponse);

    // ── Templates ───────────────────────────────────────────────────
    rpc CreateAgentRunTemplate(CreateAgentRunTemplateRequest)
        returns (CreateAgentRunTemplateResponse);
    rpc ListAgentRunTemplates(ListAgentRunTemplatesRequest)
        returns (ListAgentRunTemplatesResponse);

    // ── DPIA ────────────────────────────────────────────────────────
    rpc RegisterDpiaEvidence(RegisterDpiaEvidenceRequest)
        returns (RegisterDpiaEvidenceResponse);
}
```

### 15.2 AgentRun K8s CRD

```yaml
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: agentruns.efe.example.com
spec:
  group: efe.example.com
  names:
    kind: AgentRun
    listKind: AgentRunList
    plural: agentruns
    singular: agentrun
    shortNames: [ar]
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openAPIV3Schema:
          type: object
          required: [spec]
          properties:
            spec:
              type: object
              required: [tenantId, allowedTools, maxCostUsd, timeoutMinutes, profileName, dpiaAddendumHash]
              properties:
                tenantId: { type: string, pattern: '^[a-z0-9][a-z0-9-]*[a-z0-9]$' }
                rawPrompt: { type: string, maxLength: 8192 }
                templateRef:
                  type: object
                  properties:
                    name: { type: string }
                    version: { type: string }
                allowedTools:
                  type: array
                  items: { type: string }
                  minItems: 0
                maxCostUsd: { type: number, minimum: 0.01, maximum: 500 }
                timeoutMinutes: { type: integer, minimum: 1, maximum: 240 }
                profileName: { type: string }
                dpiaAddendumHash: { type: string, pattern: '^[a-f0-9]{64}$' }
                submittedBy: { type: string }
                memoryNamespaces: { type: array, items: { type: string } }
            status:
              type: object
              properties:
                phase: { type: string, enum: [Pending, Provisioning, Running, Paused, AwaitingApproval, Completed, Failed, TimedOut, Terminated] }
                podName: { type: string }
                microvmId: { type: string }
                stepCount: { type: integer }
                costCentsSpent: { type: integer }
                lastTransitionTime: { type: string, format: date-time }
                finalOutput: { type: string }
                failureReason: { type: string }
      subresources:
        status: {}
      additionalPrinterColumns:
        - name: Tenant, type: string, jsonPath: .spec.tenantId
        - name: Phase, type: string, jsonPath: .status.phase
        - name: Steps, type: integer, jsonPath: .status.stepCount
        - name: Cost-USD, type: number, jsonPath: .status.costCentsSpent
        - name: Age, type: date, jsonPath: .metadata.creationTimestamp
```

### 15.3 AgentRunTemplate CRD

Similar structure ; signed via Sigstore (cosign verify before any
AgentRun referencing the template can be created).

---

## 16. mcp-server tools M25

### 16.1 4 nouveaux tools

| Tool | Purpose | Write |
|---|---|:---:|
| `efe_create_agent_run` | Submit a new AgentRun | ✅ (requires DPIA + approval workflow) |
| `efe_get_agent_run` | Fetch run state + last 100 steps | ❌ |
| `efe_approve_agent_step` | Operator inline approval | ✅ |
| `efe_terminate_agent_run` | SIGTERM the microVM | ✅ |

### 16.2 Spec : `efe_create_agent_run`

```python
_INPUT_SCHEMA: dict[str, Any] = {
    "type": "object",
    "title": "EfeCreateAgentRunInput",
    "description": (
        "Submit a new AgentRun. Requires tenant_admin role + signed DPIA "
        "addendum + non-empty allowed_tools whitelist. Output is the "
        "created run id ; status starts at PENDING."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "raw_prompt": {"type": "string", "minLength": 10, "maxLength": 8192},
        "template_id": {"type": "string", "description": "Use this OR raw_prompt, not both"},
        "allowed_tools": {
            "type": "array",
            "items": {"type": "string"},
            "minItems": 1, "maxItems": 50,
            "uniqueItems": True,
        },
        "max_cost_usd": {"type": "number", "minimum": 0.01, "maximum": 500, "default": 50},
        "timeout_minutes": {"type": "integer", "minimum": 1, "maximum": 240, "default": 60},
        "profile_name": {"type": "string"},
        "dpia_addendum_hash": {"type": "string", "pattern": "^[a-f0-9]{64}$"},
    },
    "required": ["tenant_id", "allowed_tools", "profile_name", "dpia_addendum_hash"],
    "additionalProperties": False,
    "oneOf": [
        {"required": ["raw_prompt"]},
        {"required": ["template_id"]},
    ],
}
```

### 16.3 Pin tests mcp tools M25

- `test_create_run_requires_tenant_admin`
- `test_create_run_requires_dpia_signed`
- `test_create_run_rejects_empty_tools`
- `test_create_run_rejects_disabled_profile`
- `test_get_run_per_tenant_isolated`
- `test_approve_step_records_oidc_sub`
- `test_terminate_emits_audit`
- `test_register_tools_off_when_m25_disabled`

---

## 17. efe-admin CLI extensions

### 17.1 Command tree

```
efe-admin agent
├── create               # submit new AgentRun
│   --template <name@version>
│   --raw-prompt <file>
│   --tools <comma-list>
│   --profile <name>
│   --max-cost 50
│   --timeout 60
├── list                 # list runs
│   --tenant <slug>
│   --status <phase>
├── get <run-id>         # detail + recent steps
├── tail <run-id>        # live SSE tail
├── approve <step-id> --reason "..."
├── reject <step-id>  --reason "..."
├── pause <run-id>
├── resume <run-id>
├── terminate <run-id> --reason "..." --confirm
├── template
│   ├── create --file <yaml>
│   ├── list
│   └── show <name@version>
├── cost
│   └── report --period last-30d
├── dpia
│   ├── register --signer <name> --addendum-hash <h>
│   └── show
└── audit
    └── verify --tenant <slug> --module=agent
```

---

## 18. portal-admin UI extensions

### 18.1 New pages

```
apps/portal-admin/src/app/agent/
├── runs/page.tsx              # list of runs + filters
├── runs/[id]/page.tsx         # run detail + live tail SSE
├── runs/[id]/approval/[step]/page.tsx  # in-band approval modal
├── templates/page.tsx         # template catalog
├── templates/[id]/page.tsx    # template detail + signature verify
├── cost/page.tsx              # cost dashboard
└── dpia/page.tsx              # DPIA evidence viewer
```

### 18.2 Live tail (SSE)

Critical UX feature : operator can watch agent runs in real-time.

```typescript
// apps/portal-admin/src/components/agent/LiveTail.tsx
"use client";

import { useEffect, useState } from "react";

export function LiveTail({ runId }: { runId: string }) {
  const [events, setEvents] = useState<AgentStepEvent[]>([]);
  useEffect(() => {
    const es = new EventSource(`/api/agent/runs/${runId}/tail`);
    es.onmessage = (e) => {
      const event = JSON.parse(e.data) as AgentStepEvent;
      setEvents((prev) => [...prev.slice(-99), event]);
    };
    es.onerror = () => es.close();
    return () => es.close();
  }, [runId]);
  return (
    <div className="font-mono text-xs space-y-1 bg-slate-950 text-slate-100 p-3 rounded h-96 overflow-y-auto">
      {events.map((ev, i) => (
        <div key={i} className={`whitespace-pre-wrap ${eventColor(ev.kind)}`}>
          [{new Date(ev.timestamp).toLocaleTimeString()}] {ev.kind} {ev.summary}
        </div>
      ))}
    </div>
  );
}
```

### 18.3 In-band approval modal

When agent reaches AWAITING_APPROVAL :
- Run detail page shows yellow banner
- Banner has button "Approve / Reject Step N"
- Click → modal with tool name + arguments + rationale field
- Operator approves with rationale ≥ 10 chars
- Modal submits to gRPC ApproveAgentStep
- SSE event triggers UI refresh

---

## 19. Multi-tenant isolation M25 (defense in depth)

| Boundary | Mechanism | Pin test |
|---|---|---|
| Tenant ↔ tenant K8s | per-tenant namespace `efe-agent-runs-{tenant}` | `test_namespace_per_tenant` |
| microVM ↔ host | Firecracker/KVM hardware virtualization | `test_microvm_kvm_isolation` |
| microVM ↔ microVM | distinct vsock + memory | `test_microvm_no_shared_memory` |
| Network egress | NetworkPolicy deny-all + whitelist | `test_egress_whitelist_enforced` |
| Tenant context | injected at vm-create, immutable | `test_tenant_id_immutable_runtime` |
| RLS Postgres | all M25 tables RLS-enforced | `test_rls_no_session_var_returns_empty` |
| audit chain | per-tenant HMAC key | `test_audit_chain_per_tenant_isolated` |
| Memory namespace | tenant-scoped, no cross-tenant | `test_memory_no_cross_tenant_read` |
| LLM API keys | per-tenant BYOK injection | `test_llm_keys_isolated_per_microvm` |

---

## 20. Compliance + AI Act Art. 9 + DPIA M25

### 20.1 EU AI Act Art. 9 — Risk management for high-risk AI

M25 self-classifies as **high-risk autonomous AI system** under
Annex III. Art. 9 requires :

1. **Risk management system** : continuous, iterative, documented
2. **Identification of risks** : sandbox escape, prompt injection, autonomous mis-action
3. **Mitigation** : sandboxing, whitelist, approval, audit chain
4. **Post-market monitoring** : metrics emit + alerting + quarterly review
5. **Documentation** : maintained throughout lifecycle

### 20.2 DPIA M25 template (livré comme part du sprint)

Format : Markdown document signed by DPO, tenant_admin, EFE compliance.
Sections :

- 1. Description of the processing
- 2. Necessity + proportionality
- 3. Risks identified (sandbox escape, output injection, etc.)
- 4. Mitigations applied
- 5. Residual risk assessment
- 6. DPO opinion
- 7. Reviewed by board / stakeholders
- 8. Renewal cadence (annual)

Signed PDF SHA-256 hash stored in `tenant_feature.m25_dpia_signed_at` + `m25_dpia_addendum_hash` (TBD column to add).

### 20.3 Post-market monitoring metrics (Art. 26)

Required metrics emitted continuously :

```
efe_agent_runs_total{tenant, status}
efe_agent_step_count{tenant, kind}
efe_agent_cost_cents_total{tenant}
efe_agent_sandbox_escape_attempts{tenant}        # MUST be 0 in prod
efe_agent_approval_response_latency_seconds{tenant}
efe_agent_run_duration_seconds{tenant}
efe_agent_failure_rate_per_tenant{tenant}
```

PagerDuty alerts on :
- `sandbox_escape_attempts > 0` (P0)
- `failure_rate > 10%` for 1h sliding window (P1)
- `approval_response_latency_p99 > 4h` (P2)

### 20.4 Quarterly review meeting (mandatory)

Per Art. 9.2 :
- Reviewer : DPO + CISO + Head of Engineering
- Cadence : quarterly
- Output : signed minutes, retained 7 years
- Decision : continue / pause / sunset

### 20.5 GDPR Art. 17 cascade

When subject erasure submitted :
1. Identify all `agent_audit` rows referencing the data
2. Tombstone (preserve HMAC chain)
3. Identify `agent_state` blobs containing the data
4. Re-encrypt with placeholder + zero out original blob
5. Notify subject + record in `gdpr_art17_log`

### 20.6 Pin tests compliance

- `test_dpia_required_for_create_run`
- `test_dpia_expiry_blocks_create_run`
- `test_post_market_metrics_emitted`
- `test_quarterly_review_cron_emits_alert`
- `test_gdpr_art17_cascades_to_agent_state`
- `test_gdpr_art17_preserves_audit_chain`

---

## 21. Threat model M25 (delta vs M24)

### 21.1 New attack surfaces

| # | Attack | Severity | Mitigation |
|---|---|---|---|
| TM-25-01 | Sandbox escape (microVM → host) | CRITICAL | Firecracker isolation + adversarial bench (100% detect) + PagerDuty alert |
| TM-25-02 | Tool whitelist bypass | CRITICAL | 3-layer enforcement (control plane + agent core + mcp-server) |
| TM-25-03 | Approval token replay | HIGH | Single-use tokens + expiry < 10 min after grant |
| TM-25-04 | Network egress to non-whitelisted endpoint | CRITICAL | NetworkPolicy deny-all + Calico CNI enforcement |
| TM-25-05 | LLM prompt injection redirecting agent | HIGH | tenant scope + write tools require approval + audit chain detects anomalies |
| TM-25-06 | Cost runaway | HIGH | Hard SIGTERM at 100 % cap |
| TM-25-07 | Resource exhaustion (forkbomb) | MEDIUM | cgroup limits + max process count |
| TM-25-08 | Cross-tenant data via shared LLM provider | MEDIUM | per-tenant API keys + dedicated context per call |
| TM-25-09 | DPIA forgery | HIGH | signed PDF SHA-256 verified at create-time |
| TM-25-10 | Insider malicious agent submission | MEDIUM | tenant_admin role + audit + step approval for writes |
| TM-25-11 | Image swap (custom image instead of official) | CRITICAL | image whitelist + cosign verify in admission webhook |
| TM-25-12 | Memory poisoning across runs | MEDIUM | memory namespace isolation + opt-in per tenant |

### 21.2 Adversarial bench — sandbox escape (100% detection cible)

**Test fixture** : 50 known sandbox-escape techniques (Firecracker CVEs disclosed, generic Linux exploits, side-channel attempts). Run each in a sandbox-test mode and verify :
1. Exploit fails to break out
2. NetworkPolicy logs the attempt
3. Audit chain captures the anomaly
4. Alert fires within 5 seconds

GA gate : 50/50 detection.

### 21.3 Pin tests threat model

8+ adversarial tests dedicated.

---

## 22. Backend contract evolution (1.7 → 2.0 — semver MAJOR)

### 22.1 Breaking changes (justifying MAJOR bump)

- New service `efe-agent-runner` (separate process + Helm chart)
- New CRDs : `AgentRun`, `AgentRunTemplate`
- New gRPC service `AgentRuntimeService`
- Backend audit chain extended (new event_type values)
- New `/admin/agents/runs/` REST endpoints
- New tenant_feature columns + DPIA flag

### 22.2 Backward compat

- EFE 1.x clients continue to function (no removed RPCs)
- Helm chart `efe-stack` unchanged at 1.7 ; M25 ships as `efe-agent-runner` chart 2.0.0
- Tenants who don't deploy M25 → see no behaviour change
- Migration 0016 idempotent, rollback safe

### 22.3 Versioning matrix

| Component | Pre-M25 version | Post-M25 version |
|---|---|---|
| `efe-server` | 1.7 (M24) | 2.0 |
| `mcp-server` | 1.7 (43 tools) | 2.0 (47 tools : +4 M25) |
| `efe-admin` | 1.7 | 2.0 |
| `portal-admin` | 1.7 | 2.0 |
| Helm `efe-stack` | 1.7 | 1.7 (unchanged) |
| Helm `efe-agent-runner` | N/A | **2.0.0** (new) |

---

## 23. Bench + pin tests

### 23.1 Bench scenarios M25 (6 nouveaux)

```
crates/efe-bench/scenarios/agent_runtime/
├── e2e_agent_run.rs           # full audit-readonly run end-to-end
├── sandbox_escape.rs          # 50 known exploit techniques
├── tool_whitelist_bypass.rs   # try to call unlisted tools 3 layers
├── cost_cap_enforcement.rs    # cost runaway scenarios (forkbomb LLM calls)
├── approval_token_replay.rs   # single-use enforcement
└── gdpr_art17_cascade.rs      # erasure cascades to agent_state + audit
```

### 23.2 Bench thresholds (GA gate — experimental)

| Bench | Threshold | Direction |
|---|---|---|
| sandbox_escape_detection_rate | ≥ 100 % (50/50) | = (HARD GATE) |
| tool_whitelist_bypass_rate | 0 / 100 | = (HARD GATE) |
| cost_cap_enforcement | 100 % | = (HARD GATE) |
| approval_token_replay_blocked | 100 % | = |
| e2e_run_duration_p99_ms | < 10 min for audit-readonly | ≤ |
| gdpr_art17_audit_chain_intact | 100 % | = (HARD GATE) |

### 23.3 Pin tests inventory M25 (target +120, oversubscribed vs +90 minimum)

| Module | Count |
|---|:---:|
| Sandbox / microVM | 10 |
| Agent core | 8 |
| Tool ACL (3 layers) | 6 |
| Step approval | 7 |
| Pause / resume | 8 |
| Memory | 7 |
| Cost governance | 5 |
| Migration 0016 | 4 |
| RLS / multi-tenant | 8 |
| State machines | 8 |
| Compliance + DPIA | 7 |
| Threat adversarial | 8 |
| mcp-server tools | 8 |
| CLI commands | 6 |
| portal-admin UI | 8 |
| K8s CRD validation | 6 |
| Helm chart smoke | 4 |
| Network policy | 6 |
| Image whitelist | 4 |
| **Total** | **+128** |

---

## 24. Activation flags + dependency validation

### 24.1 6-niveaux flags

```
LEVEL 1 — Helm chart
  agentRunner.enabled                       (default false ; CRDs not installed)
  agentRunner.namespaceIsolation            (strict | relaxed ; default strict)
  agentRunner.image.allowList               (default ['efe-agent:*'])
  agentRunner.firecracker.enabled           (default true)
  agentRunner.networkPolicyDenyEgressDefault(default true)

LEVEL 2 — Server (env vars)
  EFE_FEATURE_AGENT_RUNTIME_ENABLED         (default false)
  EFE_FEATURE_AGENT_RUNTIME_REQUIRE_M21     (default true ; refuse boot if M25 ON without M21)
  EFE_FEATURE_M21_LLMROUTER                 (must be true)

LEVEL 3 — CRD validation (admission webhook)
  AgentRun.spec.tenantId                    (must match caller's namespace)
  AgentRun.spec.profileName                 (must be in tenant.allowed_profiles)
  AgentRun.spec.dpiaAddendumHash            (must match tenant.m25_dpia_addendum_hash)

LEVEL 4 — Tenant (DB)
  tenant_feature.m25_agent_runtime_enabled       (default false)
  tenant_feature.m25_dpia_signed_at              (required NOT NULL)
  tenant_feature.m25_dpia_expires_at             (must be > NOW())
  tenant_feature.m25_cost_cap_per_run_usd        (default 0 → all rejected)
  tenant_feature.m25_timeout_minutes_max         (default 60)
  tenant_feature.m25_allowed_profiles            (default empty → none allowed)
  tenant_feature.m25_allowed_providers           (default empty)
  tenant_feature.m25_memory_enabled              (default false)
  tenant_feature.m25_microvm_pool_size           (default 2)
  tenant_feature.m25_max_concurrent_runs         (default 5)

LEVEL 5 — Template (signed)
  agent_run_template.cosign_signature_url        (must verify)
  agent_run_template.deprecated_at               (NULL or future)

LEVEL 6 — Per-run (CRD spec)
  AgentRun.spec.maxCostUsd ≤ tenant.m25_cost_cap_per_run_usd
  AgentRun.spec.timeoutMinutes ≤ tenant.m25_timeout_minutes_max
  AgentRun.spec.allowedTools ⊆ profile.allowed_tools
```

### 24.2 Startup validation (panic if mis-configured)

```rust
pub fn validate_m25_startup() -> Result<(), StartupError> {
    let m25 = env_bool("EFE_FEATURE_AGENT_RUNTIME_ENABLED");
    if !m25 { return Ok(()); }

    // Verify CRD installed
    if !crd_exists("agentruns.efe.example.com") {
        return Err(StartupError::CrdMissing(
            "EFE_FEATURE_AGENT_RUNTIME_ENABLED=true but AgentRun CRD not installed in cluster ; \
             install via Helm chart efe-agent-runner first".into()
        ));
    }

    // Verify Firecracker available on node
    if !firecracker_kvm_available() {
        return Err(StartupError::SandboxUnavailable(
            "KVM not available ; M25 requires hardware virtualization".into()
        ));
    }

    // Verify M21 dependency
    if !env_bool("EFE_FEATURE_M21_LLMROUTER") {
        return Err(StartupError::DependencyMissing(
            "M25 requires M21 LLMRouter to be ON".into()
        ));
    }
    Ok(())
}
```

### 24.3 Pin tests activation

- `test_m25_off_default`
- `test_m25_on_without_crd_panics`
- `test_m25_on_without_kvm_panics`
- `test_m25_on_without_m21_panics`
- `test_tenant_dpia_expired_blocks_create_run`
- `test_tenant_empty_profiles_blocks_create_run`
- `test_create_run_cost_above_tenant_cap_rejected`
- `test_image_whitelist_admission_webhook_rejects_unknown`

---

## 25. Sprint plan D80-D85 (6 jours)

| Jour | Focus | Engineers | LOC | Pin tests |
|---|---|---|---:|---:|
| D80 | DB migration 0016 + crate skeleton + AgentRun CRD + Helm chart skeleton | 2 Rust + 1 SRE | ~1500 | +18 |
| D81 | Sandbox driver Firecracker + network policy + filesystem | 1 Rust + 1 SRE | ~1800 | +20 |
| D82 | Agent core (Python) + tool whitelist enforcement + audit | 1 Python + 1 ML | ~1400 | +20 |
| D83 | Step approval + pause/resume + cost circuit breaker | 1 Rust + 1 Python | ~1300 | +18 |
| D84 | mcp tools + portal-admin live tail + CLI | 1 Python + 1 TS | ~1600 | +18 |
| D85 | Bench (sandbox escape adversarial) + DPIA template + GA gate | 2 Security + lead | ~1100 | +34 |

### 25.1 Audit pass post-D85 (½ jour)

Same chirurgical pattern as M19→M24 :
- LOC final inventory
- Cross-cutting check : audit chain HMAC, sandbox escape detection,
  tool whitelist 3-layer enforcement, approval workflow
- Pin test count verified on CI (target +128)
- Adversarial bench green (50/50 sandbox escape detection)
- Compliance pack regen + DPIA template signed off
- CHANGELOG + tag `M25/v2.0.0`

---

## 26. Risk register

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|:---:|:---:|---|
| R-25-01 | Sandbox escape (microVM → host) | LOW | CRITICAL | Firecracker isolation + adversarial bench + annual audit |
| R-25-02 | Tool whitelist bypass | LOW | CRITICAL | 3-layer enforcement + audit anomaly detection |
| R-25-03 | Cost runaway by malicious agent | MEDIUM | HIGH | Hard SIGTERM cost cap |
| R-25-04 | Approval fatigue (operator rubber-stamps) | HIGH | HIGH | UI shows full context + min rationale + 4-eyes optional |
| R-25-05 | Prompt injection redirecting agent intent | HIGH | HIGH | Whitelist + audit anomaly detection + step approval for writes |
| R-25-06 | LLM provider compromise (data exfil through API) | LOW | CRITICAL | Per-tenant API keys + provider whitelist + audit |
| R-25-07 | DPIA expiry not enforced | LOW | HIGH | Daily cron + auto-block create_run |
| R-25-08 | Resource exhaustion (forkbomb) | LOW | MEDIUM | cgroup limits + max processes + adversarial bench |
| R-25-09 | Sunset risk (< 2 tenants at 12mo) | MEDIUM | LOW | Monitor adoption ; clean sunset path documented |
| R-25-10 | Helm chart drift (tenant has stale chart) | MEDIUM | MEDIUM | chart version compat matrix in docs |
| R-25-11 | KVM not available on tenant cluster | LOW | HIGH | startup validation + clear error message |
| R-25-12 | Image whitelist tampered (admission webhook bypass) | LOW | CRITICAL | Webhook signature verify + signed Helm chart |
| R-25-13 | AI Act audit failure (regulator inquiry) | LOW | CRITICAL | DPIA + post-market monitoring + quarterly review minutes |
| R-25-14 | Cross-run memory leak (poisoning) | LOW | HIGH | Tenant scope + namespace isolation + opt-in |

---

## 27. Open questions / decisions needed

### O-1 : GPU support for agents ?

**Question** : autoriser GPU compute pour agents ?

**Status** : NON en V1. GPU sandboxing nécessite plus de maturité.

**Decision attendue** : EFE 3.0+ ou architecture revisited.

### O-2 : Custom tenant tools (HTTP egress to tenant API) ?

**Question** : agent peut-il appeler des HTTP API custom du tenant ?

**Status** : NON en V1. Seulement mcp-server + LLMRouter.

**Decision attendue** : T+12 mois, basé sur demand.

### O-3 : Multi-LLM agent collaboration ?

**Question** : multi-agent orchestration ?

**Status** : NON en V1. EFE 3.0+ ou out-of-scope.

### O-4 : Streaming output partial ?

**Status** : NON en V1. SSE pour observation seulement.

### O-5 : Auto-resume after crash ?

**Status** : NON en V1. Manual resume by operator.

### O-6 : RAG / vector DB inside agent ?

**Status** : NON en V1. Tools-only access pattern.

### O-7 : Custom user images ?

**Status** : NON en V1. Whitelist obligatoire.

---

## 28. Success metrics + GA gate (experimental)

### 28.1 SLOs M25

| ID | SLO | Cible |
|---|---|---|
| M25-AGENT-1 | AgentRun p99 duration | < timeout (60 min default) |
| M25-AGENT-2 | Cost cap enforcement | 100 % |
| M25-AGENT-3 | Sandbox escape attempts detected | 100 % alerted |
| M25-AGENT-4 | Operator adoption | ≥ 1 pilot tenant at T+90 j ; ≥ 2 at T+12 mo |
| M25-AGENT-5 | Eval acceptance rate (operator approves output) | ≥ 60 % |
| M25-AGENT-PIN | Pin tests | +90 GREEN minimum (target +128) |
| M25-AGENT-DPIA | DPIA completion rate | 100 % of active tenants |
| M25-AGENT-COST | Avg cost per run (pilots) | ≤ $20 |

### 28.2 Experimental GA gate

- [ ] +90 pin tests GREEN minimum (target +128 oversubscribed)
- [ ] Sandbox escape adversarial bench : 50/50 detection
- [ ] Tool whitelist bypass : 0/100 successful bypasses
- [ ] Cost cap enforcement : 100 % SIGTERM at threshold
- [ ] DPIA template signed off by EFE DPO + CISO
- [ ] AI Act Art. 9 risk management system documented
- [ ] At least one pilot tenant signed addendum DPA
- [ ] Helm chart smoke install on test cluster passes
- [ ] CRD admission webhook deployed and verified
- [ ] PagerDuty alerts wired for sandbox escape + cost cap
- [ ] Quarterly review meeting #1 scheduled
- [ ] Sunset clause documented

### 28.3 Post-GA observability

Pendant 90 j post-GA :
- Daily : sandbox escape attempts (must be 0)
- Daily : cost cap hits + responses
- Weekly : DPIA expiry tracking
- Monthly : adoption metrics
- Quarterly : risk management review (Art. 9 mandatory)

---

## 29. Glossary M25+

| Term | Definition |
|---|---|
| **Agent runtime** | In-cluster sandboxed environment for LLM-driven autonomous execution |
| **AgentRun** | Single execution instance ; lifecycle PENDING → RUNNING → COMPLETED/FAILED |
| **AgentRunTemplate** | Reusable signed template (prompt + profile + tools whitelist) |
| **microVM** | Firecracker-based KVM virtual machine, lightweight, fast-spawning |
| **Profile** | Predefined whitelist of tools (e.g. `audit-readonly`, `taxonomy-refactor`) |
| **Step approval** | In-band human-in-the-loop check before write tool execution |
| **DPIA** | Data Protection Impact Assessment — required per GDPR Art. 35 + AI Act Art. 9 |
| **Pause / Resume** | Operator can pause an in-flight run ; state checkpointed and restored |
| **Cross-run memory** | Tenant-scoped opt-in storage that persists across AgentRuns |
| **Sandbox escape** | An attempt by an agent to exit the microVM boundary |
| **Cost cap** | Hard `max_cost_usd` per-run ; SIGTERM enforced |
| **Image whitelist** | Approved set of `efe-agent:*` runtime images, signature-verified |

---

## 30. References

### Standards & regulations
- **EU AI Act 2024/1689** Art. 9, 14, 26 — risk management, human oversight, post-market monitoring
- **GDPR (EU 2016/679)** Art. 17, 22, 35 — erasure, automated decision, DPIA
- **NIS2 Directive 2022/2555** Art. 21 — supply chain
- **NIST AI RMF 1.0** — Govern, Map, Measure, Manage
- **ISO/IEC 42001:2023** — AI management systems
- **SOC 2** CC8 — change management
- **ISO 27001:2022** A.8.32 — change management

### Sandboxing tech
- **Firecracker microVM** — https://firecracker-microvm.github.io
- **KVM** Kernel-based Virtual Machine — Linux
- **Calico CNI** — NetworkPolicy enforcement
- **Pod Security Standards (restricted)** — Kubernetes 1.27+

### EFE internal docs
- [`EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md)
- [`EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`](EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md)
- [`EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md)
- [`EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md`](EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md)
- [`EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`](EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md)
- [`EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md`](EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md)
- [`EFE_RELEASE_V1_GA_ARCHITECTURE.md`](EFE_RELEASE_V1_GA_ARCHITECTURE.md)
- [`EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`](EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md)

### Research
- "Constitutional AI : Harmlessness from AI Feedback" (Anthropic, 2022)
- "ReAct : Synergizing Reasoning and Acting in Language Models" (Yao et al., 2022)
- "Toolformer : Language Models Can Teach Themselves to Use Tools" (Meta, 2023)
- "Sandboxing AI Agents : Survey of Approaches" (industry pre-print, 2027)

---

## Annexe A — Sandbox security profile

### A.1 Seccomp profile (subset)

Allowed syscalls : read, write, openat (readonly), close, mmap (anonymous),
munmap, brk, exit_group, futex, clock_gettime, getrandom, fstat, lseek,
poll, ppoll, dup2, fcntl (limited), socket (TCP only, IPv4),
connect (whitelist endpoints), recvmsg, sendmsg.

Denied : ptrace, prctl, setuid, setgid, mount, umount, kexec_load,
init_module, finit_module, delete_module, accept (no inbound),
listen (no inbound), execve (only for agent_core init), …

### A.2 AppArmor profile

Confines agent_core process : readonly /, tmpfs /tmp, no /dev access except
/dev/null + /dev/urandom + /dev/kvm (Firecracker host-side), no /proc
escape.

### A.3 cgroup v2 limits

- pids.max = 64
- memory.max = 2 GiB
- cpu.max = 100000 100000 (1 vCPU)
- io.max — read/write 50 MB/s

---

## Annexe B — Cost model

### B.1 Per-run cost components

| Component | Unit cost | Run cost (60 min) |
|---|---:|---:|
| Firecracker microVM (1 vCPU + 2 GB) | $0.001/min | ~$0.06 |
| LLM tokens (sonnet, ~30K i + 10K o) | varies | ~$8 |
| Network egress (LLM API) | included | – |
| Audit chain writes | included | – |
| **Total per audit-readonly run** | | **~$8** |
| **Total per taxonomy-refactor run** | | **~$25** (more LLM calls) |

### B.2 Per-tenant monthly estimate (pilot)

- 30 runs/month avg × $15 avg = $450/month
- Plus DPIA review overhead (one-time)

---

## Annexe C — Agent task template specification

### C.1 Template format (YAML, signed cosign)

```yaml
apiVersion: efe.example.com/v1
kind: AgentRunTemplate
metadata:
  name: audit-runbook-author
  version: 1.0.0
spec:
  description: |
    Audit all bundles for SLSA L3 compliance and write a runbook draft.
  prompt: |
    You are an EFE compliance auditor. Your task is to check every active
    bundle for SLSA L3 compliance signs : ...
  allowed_tools:
    - efe_list_available_bundles
    - efe_inspect_bundle
    - efe_get_sbom
    - efe_audit_bundle_vs_gold
  profile_name: audit-readonly
  max_cost_usd_default: 50
  timeout_minutes_default: 60
  estimated_steps: 25
  estimated_cost_usd: 18
  documentation_url: https://docs.efe.example.com/agents/audit-runbook
  cosign_signature_url: registry.efe.example.com/templates@sha256:...
```

### C.2 Signature verification

Required at template registration AND at AgentRun create-time. Cosign
verify via R-100 release pipeline identity.

---

## Annexe D — Pin tests inventory M25

(Cf. § 23.3 pour le tableau global +128. Liste canonique dans
`bench_baselines/v2.0.0/baseline.json`.)

128 tests groupés par module. Top-level highlights :
- 50 sandbox escape adversarial scenarios
- 100 tool whitelist bypass attempts
- 8 cost cap enforcement scenarios
- 7 step approval workflow tests
- 8 pause/resume tests
- ...

Full list shipped in baseline.json à D85.

---

## Annexe E — DPIA delta M25

### E.1 Sections impactées vs DPIA M19/M24

| Section | M24 | M25 |
|---|---|---|
| 2. Necessity + proportionality | active learning | + agent autonomy |
| 3. Risks identified | + sandbox escape, autonomous mis-action |
| 4. Mitigations | + Firecracker, tool whitelist, approval, image whitelist |
| 6. DPO opinion | M24 sign-off | + addendum M25 sign-off requis |
| 7. Reviewed by | DPO + CISO | + Board (high-risk autonomous → governance level) |

### E.2 Annual renewal cadence

Each tenant's DPIA expires 12 months after `m25_dpia_signed_at`. Auto-revert
of `m25_agent_runtime_enabled` to false on expiry. Tenant must re-sign
addendum.

---

## Annexe F — Migration `0016_agent_runtime.sql`

(Cf. § 13.1 pour le schéma SQL complet. Full file livré au sprint D80 :
~370 LOC SQL.)

Includes :
- 7 tables, 2 ENUMs
- 11 tenant_feature columns
- 7 RLS policies
- 2 state-machine triggers
- Schema migration tracking

---

## Annexe G — Helm chart `efe-agent-runner`

### G.1 Chart structure

```
charts/efe-agent-runner/
├── Chart.yaml                # version 2.0.0
├── values.yaml                # default agentRunner.enabled=false
├── templates/
│   ├── crd-agentrun.yaml
│   ├── crd-agentruntemplate.yaml
│   ├── deployment-controller.yaml
│   ├── deployment-microvm-host.yaml
│   ├── networkpolicy-base.yaml
│   ├── rbac.yaml
│   ├── service.yaml
│   ├── admission-webhook.yaml
│   └── psp-restricted.yaml
└── README.md
```

### G.2 Default values

```yaml
agentRunner:
  enabled: false                    # MUST be opt-in
  namespaceIsolation: strict
  image:
    allowList:
      - "ghcr.io/efe-ai/efe-agent:*"
    requireSignature: true
  firecracker:
    enabled: true
    pool:
      defaultSize: 2
      maxSize: 10
  resources:
    controller:
      requests: { cpu: "200m", memory: "512Mi" }
      limits: { cpu: "1", memory: "1Gi" }
  networkPolicy:
    denyEgressDefault: true
    allowedEndpoints:
      - efe-llm-router:50061
      - efe-mcp-server:8081
      - efe-server-audit:8082
  rbac:
    create: true
  admission:
    enabled: true
    cosignVerify: true
  metrics:
    enabled: true
    pagerdutyOnSandboxEscape: true
```

### G.3 Compatibility matrix

| EFE version | Helm chart `efe-stack` | Helm chart `efe-agent-runner` |
|---|---|---|
| 1.0.0 (R-100 GA) | 1.0.0 | N/A |
| 1.5 (M22 GA) | 1.5.0 | N/A |
| 1.6 (M23 GA) | 1.6.0 | N/A |
| 1.7 (M24 GA) | 1.7.0 | N/A |
| 2.0.0 (M25 GA) | 1.7.0 (unchanged) | **2.0.0** (new) |

---

**FIN — EFE M25 Architecture détaillée. Roadmap LLM M20-M25 fermée.**

> Ce document est exécutable. Toute ambiguïté restante est listée en
> § 27 (Open questions). Le sprint D80-D85 peut démarrer sur cette base.
> **Statut produit : EXPERIMENTAL** — sunset clause à 12 mois si
> adoption < 2 tenants.



