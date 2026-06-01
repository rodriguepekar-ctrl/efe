# EFE M26 — Agent Runtime Production Graduation Architecture

**Status** : ARCHITECTURE — design phase, pre-implementation.
**Predecessor** : M25 (`docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`) shipped EXPERIMENTAL.
**Target** : graduate M25 from `EXPERIMENTAL` → `STABLE` ; close all production gaps identified in the M25 chirurgical audit + assembly verification.
**Codename** : "Caliper" (caliper = the precision instrument used to measure the dimensions of an object — connoting graduation, fitting, fitness-for-purpose).
**EFE version** : `2.0.0` → `2.1.0` (semver MINOR ; backward-compatible with M25).
**Sprint window** : D86 → D92 (7 design-days, ~3 calendar weeks at 4-day weeks).
**Hard dependencies** : M21 (LLMRouter), M25 (agent runtime experimental), `cert-manager` ≥ 1.16, `Firecracker` ≥ 1.10.0, KVM-enabled hypervisor, OPA Gatekeeper (optional), `cosign` ≥ 2.4, Sigstore Rekor public log.
**Default activation** : OFF for new tenants ; ON-PER-OPT-IN with DPIA + production graduation acknowledgement (separate addendum).
**Sunset clause** : LIFTED for tenants on STABLE channel ; M25 EXPERIMENTAL channel retains the < 2 tenants @ 12 months sunset clause until M26 promotes them.

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Where M25 stood — gap inventory](#2-where-m25-stood--gap-inventory)
3. [Graduation criteria — what makes us "STABLE"](#3-graduation-criteria--what-makes-us-stable)
4. [Goals + non-goals](#4-goals--non-goals)
5. [Architectural overview](#5-architectural-overview)
6. [Component map — what changes vs M25](#6-component-map--what-changes-vs-m25)
7. [REST gateway `/v1/agent/*` (efe-server)](#7-rest-gateway-v1agent-efe-server)
8. [Admission webhook server](#8-admission-webhook-server)
9. [Firecracker driver — real implementation](#9-firecracker-driver--real-implementation)
10. [vsock-relayer DaemonSet](#10-vsock-relayer-daemonset)
11. [Next.js streaming routes (portal-admin)](#11-nextjs-streaming-routes-portal-admin)
12. [Orphan slot cleanup + reconcile resilience](#12-orphan-slot-cleanup--reconcile-resilience)
13. [Cosign signature pipeline finalization](#13-cosign-signature-pipeline-finalization)
14. [SLO framework + alerting](#14-slo-framework--alerting)
15. [Bench at production scale](#15-bench-at-production-scale)
16. [Disaster recovery procedures](#16-disaster-recovery-procedures)
17. [Penetration test integration](#17-penetration-test-integration)
18. [Compliance pack integration (R-100)](#18-compliance-pack-integration-r-100)
19. [Multi-tenant isolation hardening](#19-multi-tenant-isolation-hardening)
20. [Data lifecycle management](#20-data-lifecycle-management)
21. [Cost model — production](#21-cost-model--production)
22. [Performance budget](#22-performance-budget)
23. [Migration plan : EXPERIMENTAL → STABLE](#23-migration-plan-experimental--stable)
24. [Sunset clause review + reconfirmation](#24-sunset-clause-review--reconfirmation)
25. [AI Act + GDPR compliance updates](#25-ai-act--gdpr-compliance-updates)
26. [Threat model update](#26-threat-model-update)
27. [Activation flags + tenant enablement](#27-activation-flags--tenant-enablement)
28. [Sprint plan D86–D92](#28-sprint-plan-d86d92)
29. [Architecture decision records (ADRs)](#29-architecture-decision-records-adrs)
30. [Risk register](#30-risk-register)
31. [Open questions](#31-open-questions)
32. [Success metrics](#32-success-metrics)
33. [Documentation deliverables](#33-documentation-deliverables)
34. [Rollback strategy](#34-rollback-strategy)
35. [Contract evolution + SDK changes](#35-contract-evolution--sdk-changes)

### Annexes

- [A. REST gateway endpoint catalogue](#annex-a-rest-gateway-endpoint-catalogue)
- [B. Firecracker REST API call sequence](#annex-b-firecracker-rest-api-call-sequence)
- [C. vsock-relayer wire protocol](#annex-c-vsock-relayer-wire-protocol)
- [D. SLO catalogue (full table)](#annex-d-slo-catalogue-full-table)
- [E. Pen test scope + threat scenarios](#annex-e-pen-test-scope--threat-scenarios)
- [F. DPIA addendum diff (M25 → M26)](#annex-f-dpia-addendum-diff-m25--m26)
- [G. Operator readiness checklist](#annex-g-operator-readiness-checklist)
- [H. Kernel hardening profile](#annex-h-kernel-hardening-profile)
- [I. Sigstore policy](#annex-i-sigstore-policy)
- [J. Glossary](#annex-j-glossary)

---

## 1. Executive summary

M25 shipped the agent runtime as **EXPERIMENTAL** with the explicit understanding that production-blocking gaps would be closed in a follow-on milestone. The chirurgical audit + assembly verification surfaced six gaps that prevent end-to-end deployability :

1. **REST gateway `/v1/agent/*`** is referenced by `efe-admin` and portal-admin but not implemented in `efe-server`.
2. **Admission webhook** is wired in the Helm `ValidatingWebhookConfiguration` but has no HTTPS server in the controller binary.
3. **`FirecrackerUdsDriver::spawn`** stamps a handle without making the actual REST calls (PUT `/boot-source`, `/drives`, `/machine-config`, `/network-interfaces`, `/vsock`, `/actions InstanceStart`) ; the production stub.
4. **`vsock-relayer`** — the DaemonSet that bridges in-microVM `httpx` calls to the LLM router and mcp-server pods — does not exist as a deliverable.
5. **Next.js streaming endpoint `/api/agent/runs/[id]/steps`** referenced by the `LiveTail` React component is missing.
6. **Orphan microVM slot leak** : if `apply_network_policy` fails after `pool.lease()` succeeds, the microVM stays in `Running` state with no run attached.

Beyond the "hard six" gaps, **production graduation** requires closing softer but equally important issues : SLO definitions, alerting wired to PagerDuty rotations, pen test coverage, compliance pack integration, DR drills, and a tenant-graduation workflow that distinguishes EXPERIMENTAL early adopters from STABLE production users.

M26 ("Caliper") closes all six hard gaps, adds the SLO + alerting framework, integrates a 30-scenario pen test bundle, finalizes the cosign + Rekor signature pipeline for both controller and agent-core images, runs a 50-tenant × 5-concurrent-runs bench, and ships a tenant migration workflow. **EFE 2.1.0** is a semver MINOR : the M25 wire formats, SQL schema, gRPC protos, and CRDs are all backward-compatible. The migration is a single `efe-admin agent graduate <tenant> --confirm` command per tenant.

The sunset clause baked into M25 (`< 2 tenants at 12 months → archive`) is **lifted** for tenants on the STABLE channel ; the EXPERIMENTAL channel keeps the clause until all early adopters have migrated. A new "**Caliper Steering Committee**" (CSC) — owners : Eng, SRE, DPO, Security, Product — meets monthly during the 12-month observation window post-graduation to review SLO performance, security incident counts, and tenant feedback.

---

## 2. Where M25 stood — gap inventory

### 2.1 Hard production gaps (the "blocking six")

| # | Gap | Surface | Scope of fix |
|---|---|---|---|
| 1 | REST gateway `/v1/agent/*` missing | `efe-server` | New axum router module ; 23 endpoints ; OIDC bearer auth ; tenant-scoped RLS ; rate-limit per OIDC subject ; integration tests |
| 2 | Admission webhook server missing | `efe-agent-runner` controller | New HTTPS sidecar ; cert-manager-issued cert ; validates `AgentRun` + `AgentRunTemplate` CRDs ; rejects forbidden tools / cost > tenant cap / unknown profiles |
| 3 | FirecrackerUdsDriver stub | `efe-agent-runner` sandbox | Replace stub with hyper UDS client : 6 PUT calls + 1 GET poll + InstanceHalt ; metric histogram per call ; chaos test for partial failure recovery |
| 4 | vsock-relayer DaemonSet missing | new artifact | Privileged DaemonSet + Go binary ; vsock CID 2 listener ; HTTP→TCP forwarder to LLM router + mcp-server with mTLS ; rate-limit per microVM |
| 5 | Next.js streaming route missing | `apps/portal-admin` | `app/api/agent/runs/[id]/steps/route.ts` ; SSE or polling ; auth via OIDC session |
| 6 | Orphan microVM slot leak | `efe-agent-runner` controller | `Drop` impl on `MicrovmLease` guard ; release-on-error in `drive_pending_to_running` ; metric + reaper goroutine for stale slots |

### 2.2 Soft production gaps

| # | Gap | Sev | Fix in M26 |
|---|---|:--:|---|
| 7 | SLO definitions absent | HIGH | Catalogue 14 SLOs (run lifecycle, approval latency, sandbox metrics, cost cap promptness, …) ; PrometheusRule manifests ; SLO burn-rate alerts |
| 8 | PagerDuty integration not wired | HIGH | `pagerduty-routing.yaml` ConfigMap ; alertmanager receivers ; rotation : `efe-agent-oncall-prod` (P0/P1) + `efe-agent-oncall-business-hours` (P2/P3) |
| 9 | Pen test bundle absent | HIGH | `bench/m26_pentest/` — 30 scenarios across sandbox escape vectors, RBAC, audit chain, approval token replay, NetworkPolicy bypass, image swap |
| 10 | Compliance pack auto-export | MEDIUM | R-100 cron extension : `M25_AGENT_RUN_AUDIT.json`, `M25_DPIA_LOG.json`, `M25_TOOL_REJECTION_LOG.json`, `M25_APPROVAL_LOG.json`, `M25_INCIDENT_LOG.json` (security incidents from PD) |
| 11 | DR drills not scheduled | MEDIUM | Game day schedule (4 per year) ; runbook for : DB region failover, Firecracker fleet loss, KMS rotation, cosign key rotation, audit chain HMAC re-key |
| 12 | Tenant graduation workflow absent | MEDIUM | `efe-admin agent graduate <tenant>` + DB column `m25_graduated_at` + admission webhook split (EXPERIMENTAL = relaxed, STABLE = strict) |
| 13 | Cosign signature pipeline not finalized | MEDIUM | `cosign sign` GH Action ; Rekor inclusion proof verified at controller boot ; image whitelist auto-rotated on Rekor entry expiry |
| 14 | Streaming + observability for runs > 30 min | MEDIUM | OpenTelemetry traces end-to-end ; runs > 30 min get sampling at 1 % ; long-tail latency histogram |
| 15 | Multi-region readiness | LOW | Document data residency model (NOT implemented in M26 ; deferred to M27 if multi-region adopted) ; tag every artifact with `efe.region` label |
| 16 | Multi-cloud Firecracker availability | LOW | AWS Bottlerocket + GCP COS already supported by Firecracker upstream ; document required AMI/COS image versions |
| 17 | M23/M24 portal pages at wrong path | INHERITED | M26 includes a "GLOSS-B" mini audit pass to fix M23/M24 portal pages (inherited from M25 audit findings) |

### 2.3 Cosmetic / cleanup items

- Remove `unused mut` warnings from `pool.rs::warm_loop` (already done in audit).
- Remove dead `append_agent_audit_owned` if unused after R-100 integration (decide based on usage).
- Rename ambiguous SQL column `agent_run.template_id` (which actually stores a slug) to `template_slug` ; add migration `0017_agent_run_rename_template_id.sql` with view-based backward compat for 1 release.
- Document the M25 → M26 wire format diff in `docs/EFE_M26_WIRE_DIFF.md`.

### 2.4 What's intentionally OUT of scope for M26

- **Multi-region failover** — deferred to M27 if business case emerges.
- **GPU integration for agent-core** — deferred ; current agents are CPU-bound (small LLM models via M21 router).
- **Multi-modal agents** (vision / audio inputs) — deferred to M28+.
- **Agent-to-agent orchestration** — explicitly out (a single agent run is the atomic unit ; orchestration is the operator's responsibility).
- **Federated learning** between agents across tenants — privacy implications too deep, deferred indefinitely.
- **WebAssembly sandbox alternative** — Firecracker is the chosen primitive ; WASM evaluated and rejected (no syscall isolation parity with KVM).

---

## 3. Graduation criteria — what makes us "STABLE"

A tenant cannot move from EXPERIMENTAL to STABLE until **all of the following** are true. The graduation criteria are codified in `efe-admin agent graduate --check` which runs the verification automatically.

### 3.1 Operational criteria

| Criterion | Threshold | Verification |
|---|---|---|
| Tenant has run M25 in EXPERIMENTAL for ≥ 30 calendar days | 30 d | `agent_run.created_at` MIN < NOW() - 30d |
| Tenant has run ≥ 100 agent runs successfully | 100 | `COUNT(agent_run WHERE status='COMPLETED' AND tenant=...)` |
| Tenant has zero `sandbox_escape_attempt_detected` events in last 90 d | 0 | `agent_audit WHERE event_type='sandbox_escape_attempt_detected'` |
| Tenant has < 5 % failure rate in last 30 d | < 5 % | `efe_agent_runs_total{status="FAILED"} / total` |
| Tenant has a current DPIA (signed within 11 months) | ≤ 335 d to expiry | `m25_dpia_expires_at > NOW() + 30d` |
| Tenant operator has acknowledged the production graduation addendum | YES | new column `m26_graduation_acknowledged_at` |

### 3.2 Compliance criteria

| Criterion | Threshold | Verification |
|---|---|---|
| Audit chain verified end-to-end | OK | `efe-admin audit verify <tenant> --module=agent_runtime` returns OK |
| At least 1 R-100 compliance pack regen succeeded for the tenant | YES | `compliance_pack_run` row with `module='m25_agent_runtime'` and `status='OK'` |
| DPO has reviewed the previous quarter's `M25_DPIA_LOG.json` | YES | `dpo_review` row with `module='m25_agent_runtime'` and `quarter=PREVIOUS_QUARTER` |
| AI Act post-market monitoring report submitted (Art. 26) | YES | `ai_act_report` row with `quarter=PREVIOUS_QUARTER` |

### 3.3 Technical criteria (per cluster, not per tenant)

| Criterion | Threshold | Verification |
|---|---|---|
| `efe-agent-runner` Helm chart at version ≥ 0.3.0 (bundles M26 webhook) | ≥ 0.3.0 | `helm list -n efe-system \| grep efe-agent-runner` |
| `agent-core` image cosign-verified at boot | YES | controller logs `image_whitelist_verified=true` |
| Admission webhook `failurePolicy: Fail` | YES | `kubectl get validatingwebhookconfigurations efe-agent-runner-admission -o yaml` |
| KVM available on every worker node where AgentRuns scheduled | YES | DaemonSet reports `/dev/kvm` accessible on all targeted nodes |
| `vsock-relayer` DaemonSet healthy on every targeted node | YES | DS readiness probe |
| OpenTelemetry collector receives spans from controller + agent-core | YES | Grafana dashboard `M25 Trace Sampling Health` shows ≥ 1 span/min |
| 14 SLOs deployed and burn-rate alerts firing correctly in staging dry-run | YES | Alertmanager `silences` ≥ 1 successful test fire |

### 3.4 Sign-off criteria

| Stakeholder | Sign-off |
|---|---|
| **Eng lead** | Architecture review accepted, implementation merged |
| **SRE on-call lead** | 14 SLOs accepted, runbook tested via game day |
| **Security lead** | Pen test report (30 scenarios) reviewed, no P0/P1 unresolved |
| **DPO** | DPIA addendum reviewed, AI Act compliance accepted |
| **Product lead** | Tenant pilot feedback incorporated, GA announcement drafted |
| **Legal** | Master agreement clause updated to reflect STABLE tier ; DPA addendum in place |

A tenant cannot graduate without **all six sign-offs at the cluster level** PLUS **all 6 operational + 4 compliance criteria at the tenant level**. The graduation acknowledgement document captures both layers.

---

## 4. Goals + non-goals

### 4.1 Goals

1. **Close all six hard production gaps** identified in the M25 audit.
2. **Define and deploy 14 SLOs** with burn-rate alerts and PagerDuty integration.
3. **Land a 30-scenario pen test bundle** ; require all P0/P1 findings resolved before GA.
4. **Finalize the cosign + Rekor signature pipeline** for both `efe-agent-runner` and `agent-core` images ; controller verifies at boot.
5. **Implement tenant graduation workflow** (`efe-admin agent graduate`) ; track `m26_graduation_acknowledged_at`.
6. **Bench at production scale** : 50 tenants × 5 concurrent runs (= 250 simultaneous microVMs) sustained for 4 h with cost-cap simulation, approval workflows, sandbox tests interleaved.
7. **Schedule + run 1 DR drill** before GA (full game day with simulated DB region failover).
8. **Lift the EXPERIMENTAL sunset clause** for STABLE tenants ; document the new sunset criteria for STABLE (3+ tenants @ 24 months).
9. **Cross-cutting GLOSS-B mini audit** : fix M23/M24 portal pages location + lib paths inherited issues.
10. **Update CHANGELOG, runbook, DPIA template, profiles config** to reflect M26 changes.

### 4.2 Non-goals

1. **No new tenant-facing features** — graduation is about hardening, not new functionality.
2. **No new sandbox primitives** — Firecracker stays.
3. **No new LLM provider integrations** — providers ship via M21.
4. **No new tool kinds** — the M25 7-tool catalogue is frozen for M26 ; new tools land in M27.
5. **No multi-region** — explicit deferral.
6. **No protocol breaking changes** — wire formats remain v1 ; M26 wire fields are additive.
7. **No CRD schema changes that break M25 manifests** — additive only.
8. **No mandatory tenant migration** — early adopters can stay on EXPERIMENTAL until they choose to graduate (or hit the sunset date).

---

## 5. Architectural overview

### 5.1 High-level diagram (M25 → M26 deltas)

```
┌──────────────────────────────────────────────────────────────────────┐
│                        portal-admin (Next.js)                        │
│  • app/(authenticated)/agent/                                        │
│  • NEW : app/api/agent/runs/[id]/steps/route.ts (SSE)               │
│  • NEW : OpenTelemetry browser instrumentation                       │
└─────────────────────────────┬────────────────────────────────────────┘
                              │ HTTPS (OIDC bearer)
                              ▼
┌──────────────────────────────────────────────────────────────────────┐
│                          efe-server (Rust)                           │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   NEW : axum module v1::agent — REST gateway 23 endpoints   │   │
│  │   - run lifecycle, approvals, templates, cost, dpia          │   │
│  │   - SSE on /v1/agent/runs/:id/steps/stream                    │   │
│  └─────────────────┬────────────────────────────────────────────┘   │
│                    │ in-process call                                 │
│                    ▼                                                 │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   service::AgentRuntimeService (M25, ~unchanged)             │   │
│  │   + new RPC : graduate_tenant, post_market_monitoring_export │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────┬────────────────────────────────────────────────────┘
                  │ tonic gRPC
                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                  efe-agent-runner controller (Rust)                  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   NEW : axum admission webhook server (HTTPS, port 8443)     │   │
│  │   - cert-manager-issued cert                                  │   │
│  │   - /validate-agentrun, /validate-agentruntemplate            │   │
│  │   - rejects unknown tools, cost > tenant cap, profile mismatch│   │
│  └──────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   sandbox::FirecrackerUdsDriver (REWORKED — real REST calls) │   │
│  │   - hyper UDS client                                          │   │
│  │   - 6 PUT + 1 GET + InstanceHalt                              │   │
│  │   - histograms per call ; chaos test rollback                 │   │
│  └──────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   controller::reconcile (REWORKED — orphan slot cleanup)     │   │
│  │   - Drop impl on MicrovmLease releases on error               │   │
│  │   - reaper goroutine for stale slots (> 5 min Running         │   │
│  │     without active agent_run row)                             │   │
│  └──────────────────────────────────────────────────────────────┘   │
└──────────┬───────────────────────────────────────────────────────────┘
           │ vsock (CID-pinned)
           ▼
┌──────────────────────────────────────────────────────────────────────┐
│         NEW : vsock-relayer DaemonSet (Go binary, privileged)        │
│   - vsock CID 2 port 9999 listener                                   │
│   - per-microVM rate-limit, mTLS to LLM router + mcp-server          │
│   - audit relay : every cross-boundary call logged with HMAC chain   │
└──────────┬───────────────────────────────────────────────────────────┘
           │ TCP/HTTPS (mTLS)
           ├──────────────────────────► efe-llm-router (M21)
           └──────────────────────────► efe-mcp-server
```

### 5.2 What changed structurally

- **3 new components** : REST gateway module in efe-server, admission webhook server in controller binary, vsock-relayer DaemonSet.
- **3 reworked components** : FirecrackerUdsDriver (stub → real), controller reconcile (orphan slot cleanup), portal-admin (SSE route).
- **0 component removals** : everything M25 shipped survives M26.

### 5.3 New inter-service boundaries

| Boundary | Auth | Transport | Notes |
|---|---|---|---|
| portal-admin → efe-server REST gateway | OIDC bearer (user) + service token (`EFE_BACKEND_TOKEN`) | HTTPS | Existing pattern ; M26 adds 23 new endpoints under `/v1/agent/` |
| efe-server → AgentRuntimeService | in-process (Rust function call) | n/a | Already in M25 |
| efe-server → controller | tonic gRPC over mTLS | HTTPS/2 | NEW for M26 cross-pod call ; service mesh handles mTLS |
| K8s API server → admission webhook | mTLS via cert-manager | HTTPS | NEW ; cert auto-rotates every 90 d |
| controller → Firecracker (host node) | UDS file permissions (only `efe-agent-runner` SA) | UDS HTTP | NEW (real impl) |
| agent-core → vsock-relayer | vsock authentication via JWT in HTTP header | vsock + HTTP | NEW |
| vsock-relayer → LLM router / mcp-server | mTLS + JWT | HTTPS | NEW ; mTLS via cert-manager |

---

## 6. Component map — what changes vs M25

### 6.1 Source tree additions

```
crates/efe-server/
├── src/
│   ├── v1/
│   │   ├── agent/                      ← NEW (M26 — 23 endpoints)
│   │   │   ├── mod.rs
│   │   │   ├── runs.rs                 ← run CRUD + SSE
│   │   │   ├── approvals.rs            ← approve / reject / list pending
│   │   │   ├── templates.rs            ← template CRUD
│   │   │   ├── cost.rs                 ← daily / per-run cost reports
│   │   │   ├── dpia.rs                 ← sign / status / list / refresh
│   │   │   ├── graduate.rs             ← NEW M26 graduation endpoint
│   │   │   ├── post_market.rs          ← NEW M26 AI Act post-market export
│   │   │   ├── auth.rs                 ← OIDC bearer middleware
│   │   │   ├── errors.rs               ← REST → gRPC error mapping
│   │   │   └── tests/                  ← integration tests (testcontainers)
│   │   └── …existing routers
│   └── …existing files

crates/efe-agent-runner/
├── src/
│   ├── webhook/                        ← NEW (M26 — admission webhook server)
│   │   ├── mod.rs
│   │   ├── server.rs                   ← axum HTTPS server, port 8443
│   │   ├── validators.rs               ← AgentRun + AgentRunTemplate validation
│   │   ├── tls.rs                      ← cert-manager cert reload watcher
│   │   └── tests/
│   ├── sandbox/
│   │   ├── firecracker_real.rs         ← NEW (real UDS client, replaces stub)
│   │   ├── firecracker_chaos.rs        ← NEW (chaos test harness)
│   │   └── …existing
│   ├── controller_reaper.rs            ← NEW (orphan slot reaper)
│   ├── post_market.rs                  ← NEW (AI Act Art. 26 export builder)
│   └── …existing
└── tests/
    ├── webhook_integration.rs          ← NEW
    └── firecracker_real_integration.rs ← NEW (skipped without /dev/kvm)

cmd/vsock-relayer/                      ← NEW (Go binary, daemonset)
├── go.mod
├── main.go
├── pkg/
│   ├── vsock/                          ← vsock CID 2 listener
│   ├── proxy/                          ← HTTP→TCP forwarder with mTLS
│   ├── ratelimit/                      ← per-microVM token bucket
│   └── audit/                          ← audit relay client
├── Dockerfile
└── README.md

charts/efe-agent-runner/templates/
├── webhook-service.yaml                ← NEW
├── webhook-cert.yaml                   ← NEW (cert-manager Certificate)
├── vsock-relayer-daemonset.yaml        ← NEW
├── vsock-relayer-rbac.yaml             ← NEW
├── slo-prometheusrule.yaml             ← NEW (14 SLO burn-rate rules)
├── alertmanager-routing.yaml           ← NEW (PagerDuty integration)
└── …existing M25 templates

apps/portal-admin/
├── app/api/agent/                      ← NEW
│   └── runs/[id]/
│       ├── steps/route.ts              ← SSE endpoint (M26)
│       └── tail/route.ts               ← polling fallback
└── …existing

bench/m26_pentest/                      ← NEW
├── README.md
├── 01_sandbox_escape_ptrace.py         ← deeper scenarios than M25 had
├── 02_sandbox_escape_seccomp_bypass.py
├── 03_sandbox_escape_dns_exfil.py
├── …                                   ← 30 total scenarios
└── 30_audit_chain_tamper_detection.py

docs/
├── EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md  ← THIS DOC
├── EFE_M26_WIRE_DIFF.md                ← M25 → M26 wire format diff
├── runbooks/
│   ├── M26_AGENT_RUNTIME_PRODUCTION_RUNBOOK.md  ← supersedes M25 runbook
│   └── M26_DR_PLAYBOOK.md              ← NEW (game day procedures)
└── release/
    ├── DPIA_M26_ADDENDUM.md            ← supplements M25 DPIA template
    └── M26_TENANT_GRADUATION_AGREEMENT.md  ← legal addendum
```

### 6.2 SQL migrations

| Migration | Purpose | Reversible |
|---|---|---|
| `0017_agent_run_rename_template_id.sql` | Rename `agent_run.template_id` → `template_slug` ; create view `agent_run_v25` for backward compat | YES (rollback drops view + reverts column) |
| `0018_m26_graduation.sql` | Add `tenant_feature.m26_graduation_acknowledged_at` + `m26_graduation_revoked_at` columns ; add `compliance_pack_run` table if not exist ; add `dpo_review` table | YES |
| `0019_m26_pen_test_run.sql` | Add `pen_test_run` table for tracking quarterly pen test results ; per-tenant scoped | YES |
| `0020_m26_post_market_report.sql` | Add `ai_act_post_market_report` table for Art. 26 quarterly exports | YES |

### 6.3 Helm chart changes

| Chart change | Purpose |
|---|---|
| `Chart.yaml` version bump 0.1.0 → 0.3.0 | M26 ships 0.3.0 (skipping 0.2.x reserved for hotfixes) |
| `values.yaml` : add `webhook.enabled: true` (was false in M25) | Webhook is the production gate |
| `values.yaml` : add `vsockRelayer.enabled: true` | Required for STABLE channel |
| `values.yaml` : add `slo.enabled: true` | Activates PrometheusRule manifest |
| `values.yaml` : add `pagerduty.routingKey` | Required for alert routing |
| Add `webhook-service.yaml`, `webhook-cert.yaml` | Service + cert for admission webhook |
| Add `vsock-relayer-daemonset.yaml`, `vsock-relayer-rbac.yaml` | vsock-relayer deployment |
| Add `slo-prometheusrule.yaml` | 14 SLO burn-rate rules |
| Add `alertmanager-routing.yaml` | PagerDuty + Slack routing |

---

## 7. REST gateway `/v1/agent/*` (efe-server)

### 7.1 Why REST and not pure gRPC

Both portal-admin (Next.js) and `efe-admin` (Rust CLI calling reqwest) use HTTPS+JSON. A pure tonic gRPC surface would force tonic-web on the browser side and a bigger client crate in the CLI. The M25 admin client already established the REST pattern ; M26 implements the matching server side.

**Decision** : axum REST is the public-facing transport ; the underlying `AgentRuntimeService` Rust struct is called in-process from the axum handler. tonic gRPC is reserved for service-mesh internal calls only (e.g. controller ↔ efe-server).

### 7.2 Endpoint catalogue (23 endpoints)

See [Annex A](#annex-a-rest-gateway-endpoint-catalogue) for the exhaustive list with paths, methods, request/response shapes. Summary :

| Group | Endpoints |
|---|---|
| Run lifecycle | POST `/v1/agent/runs`, GET `/v1/agent/runs`, GET `/v1/agent/runs/:id`, POST `/v1/agent/runs/:id/pause`, POST `/v1/agent/runs/:id/resume`, POST `/v1/agent/runs/:id/terminate`, GET `/v1/agent/runs/:id/steps/stream` (SSE) |
| Approvals | POST `/v1/agent/approvals/approve`, POST `/v1/agent/approvals/reject`, GET `/v1/agent/approvals/pending` |
| Templates | POST `/v1/agent/templates`, GET `/v1/agent/templates`, GET `/v1/agent/templates/:tenant/:slug`, DELETE `/v1/agent/templates/:tenant/:slug` (soft delete = deprecated) |
| Cost | GET `/v1/agent/cost/daily`, GET `/v1/agent/cost/run/:id` |
| DPIA | POST `/v1/agent/dpia/sign`, GET `/v1/agent/dpia/status`, GET `/v1/agent/dpia/list`, POST `/v1/agent/dpia/refresh` |
| Graduation (NEW) | POST `/v1/agent/graduate/check`, POST `/v1/agent/graduate/confirm`, POST `/v1/agent/graduate/revoke` |
| Post-market (NEW) | GET `/v1/agent/post-market/quarterly` (AI Act Art. 26 export) |

### 7.3 Authentication + authorization

```rust
// crates/efe-server/src/v1/agent/auth.rs

use axum::{extract::FromRequestParts, http::request::Parts};
use jsonwebtoken::{decode, DecodingKey, Validation};

pub struct AuthCtx {
    pub subject: String,
    pub tenant: String,
    pub roles: Vec<String>,
    pub token_age_seconds: i64,
}

#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthCtx {
    type Rejection = AgentApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or(AgentApiError::Unauthenticated)?;

        let key = DecodingKey::from_ec_pem(
            std::env::var("EFE_OIDC_PUBLIC_KEY_PEM")
                .map_err(|_| AgentApiError::Internal("OIDC key missing".into()))?
                .as_bytes(),
        )
        .map_err(|e| AgentApiError::Internal(e.to_string()))?;

        let mut validation = Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.set_audience(&["efe-agent-runtime"]);
        validation.set_issuer(&["https://oidc.efe.example.com"]);

        let claims: OidcClaims = decode(bearer, &key, &validation)
            .map_err(|_| AgentApiError::InvalidToken)?
            .claims;

        Ok(AuthCtx {
            subject: claims.sub,
            tenant: claims.tenant,
            roles: claims.roles,
            token_age_seconds: chrono::Utc::now().timestamp() - claims.iat,
        })
    }
}
```

The middleware extracts an OIDC JWT, validates signature + audience + issuer, and surfaces a typed `AuthCtx` to handlers. Each handler then enforces role + tenant via `AgentRuntimeService::CallerIdentity` (already implemented in M25).

### 7.4 Rate limiting

Per `(subject, endpoint_class)` token bucket :

| Endpoint class | Budget | Refill |
|---|---|---|
| **Read** (GET) | 100 / min | 100 / min |
| **Write** (POST/DELETE) | 30 / min | 30 / min |
| **Streaming** (SSE) | 5 concurrent | n/a (concurrency) |
| **Admin** (graduate, dpia/sign, dpia/refresh) | 10 / hour | 10 / hour |

Implementation : `tower-governor` middleware with a Redis-backed key store keyed by `${subject}:${class}`. On exceed → HTTP 429 with `Retry-After` header.

### 7.5 SSE endpoint for live tail

The portal-admin `LiveTail` component connects to `GET /v1/agent/runs/:id/steps/stream`. This is the production replacement for the polling stub.

```rust
// crates/efe-server/src/v1/agent/runs.rs (excerpt)

pub async fn stream_steps(
    auth: AuthCtx,
    Path(run_id): Path<Uuid>,
    State(svc): State<Arc<AgentRuntimeService>>,
) -> impl IntoResponse {
    let stream = async_stream::stream! {
        let mut last_step_index = -1i32;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let detail = match svc
                .get_agent_run(&auth.into(), run_id)
                .await
            {
                Ok(d) => d,
                Err(e) => {
                    yield Err::<Event, _>(axum::Error::new(e));
                    break;
                }
            };
            for step in detail.recent_steps {
                if step.step_index > last_step_index {
                    last_step_index = step.step_index;
                    yield Ok(Event::default()
                        .event("agent_step")
                        .json_data(&step)
                        .unwrap());
                }
            }
            if detail.run.status.is_terminal() {
                yield Ok(Event::default()
                    .event("run_terminal")
                    .json_data(&serde_json::json!({"status": detail.run.status}))
                    .unwrap());
                break;
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

The 500 ms tick is well within the 2 s polling experience that M25 used, but bound to a single open connection ; far more efficient at scale.

### 7.6 Error mapping

REST handlers return `AgentApiError` which maps to standard HTTP status codes :

| `AgentRunnerError` | HTTP status | Body |
|---|---|---|
| `InvalidInput` | 400 | `{"code":"invalid_input","message":...}` |
| `RunNotFound` | 404 | `{"code":"not_found"}` |
| `M25NotEnabledForTenant` / `DpiaNotSigned` / `DpiaExpired` | 412 | `{"code":"failed_precondition", "details":{...}}` |
| `PermissionDenied` / `TenantMismatch` | 403 | `{"code":"forbidden"}` |
| `OidcTokenStale` | 401 | `{"code":"token_stale"}` |
| `CostCapExceeded` / `CostAboveTenantCap` / `MaxConcurrentRunsReached` | 429 | `{"code":"resource_exhausted","retry_after":N}` |
| `MicroVmProvisioningFailed` / `MicroVmNotFound` | 503 | `{"code":"unavailable"}` |
| `SandboxEscapeAttempt` / `AuditChainBroken` / `AuditHmacFailure` / `Database` / `Internal` | 500 | `{"code":"internal"}` ; full details NEVER returned |

The 500 path triggers a structured tracing event with a correlation ID returned to the caller for support escalation. Sandbox escape errors additionally trigger PagerDuty P0.

### 7.7 Integration tests

Tests run under `testcontainers` with PostgreSQL + a Wiremock for the AgentRuntimeService dependency. Each endpoint has at least one test :

- **happy path** (valid request, valid auth, expected 2xx)
- **unauthorized** (no bearer or invalid bearer → 401)
- **forbidden** (wrong tenant or role → 403)
- **rate-limited** (exceed budget → 429)
- **failed precondition** (DPIA expired → 412)

A consolidated test for SSE verifies that 5 sequential steps emit 5 events plus a final `run_terminal` event.

---

## 8. Admission webhook server

### 8.1 Why an admission webhook

K8s `AgentRun` and `AgentRunTemplate` CRs are user-facing. Without an admission gate, an operator could submit :
- A run with `tool_whitelist: [evil_tool]` and the controller would fail later, but only after the CR is persisted.
- A template with `default_max_cost_cents: 999999` (above tenant cap) and the run creation would fail at intake, but the template lives on.
- A run with a deprecated profile.
- A run targeting a tenant the operator doesn't have RBAC for (cross-tenant attack).

The admission webhook rejects malformed manifests **before** they enter etcd. This prevents storage of invalid state and enforces tenant boundaries at the K8s layer, not just at the application layer.

### 8.2 Architecture

```
┌──────────────────────────────────┐
│  K8s API server                  │
│  CREATE/UPDATE on AgentRun CR    │
└─────────────┬────────────────────┘
              │
              │ admission review v1
              ▼
┌──────────────────────────────────┐
│  ValidatingWebhookConfiguration  │
│  efe-agent-runner-admission      │
│  failurePolicy: Fail             │
│  timeout: 5s                     │
└─────────────┬────────────────────┘
              │
              │ HTTPS, mTLS, port 8443
              ▼
┌────────────────────────────────────────────────────┐
│  efe-agent-runner controller pod                   │
│  ┌──────────────────────────────────────────────┐  │
│  │  webhook server (axum, port 8443)            │  │
│  │  cert-manager-issued cert (auto-rotate 90d)  │  │
│  └──────────────────────────────────────────────┘  │
│       │                                            │
│       │ in-process call                            │
│       ▼                                            │
│  ┌──────────────────────────────────────────────┐  │
│  │  webhook::validators                         │  │
│  │  - validate_agentrun(cr) → AdmissionResponse │  │
│  │  - validate_agentruntemplate(cr) → ...       │  │
│  └──────────────────────────────────────────────┘  │
│       │                                            │
│       │ DB read (tenant flags + template lookup)   │
│       ▼                                            │
│  ┌──────────────────────────────────────────────┐  │
│  │  Postgres (RLS-scoped)                       │  │
│  └──────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────┘
```

The webhook server runs in the same pod as the controller — co-tenancy is fine because both share the DB connection pool and the audit chain HMAC key. The webhook is read-only (it only reads tenant flags, profile registry, image whitelist) ; it doesn't insert into agent_audit (audit happens later when the controller actually reconciles).

### 8.3 Validation rules

For `AgentRun` :

1. **Tenant exists + agent runtime enabled** : `tenant_feature.m25_agent_runtime_enabled = TRUE`.
2. **DPIA valid** : `m25_dpia_signed_at IS NOT NULL AND m25_dpia_expires_at > NOW()`.
3. **Profile in `m25_allowed_profiles`** : reject if not.
4. **Provider in `m25_allowed_providers`** : reject if not.
5. **Tool whitelist is subset of template's** : every requested tool exists in the template's whitelist.
6. **Tool whitelist contains only known tools** : every tool name is in the global registry (the 7-tool catalogue).
7. **Cost cap ≤ tenant cap** : `max_cost_usd ≤ m25_cost_cap_per_run_usd`.
8. **Timeout ≤ tenant cap** : `timeout_seconds ≤ m25_timeout_minutes_max * 60`.
9. **Concurrent runs < tenant cap** : `COUNT(active runs) < m25_max_concurrent_runs`.
10. **Image whitelist** : if the manifest references an `image` field (rare ; usually inherits from profile), it must match `EFE_M25_IMAGE_WHITELIST`.
11. **STABLE-only checks** (when tenant has graduated) :
    - `m26_graduation_acknowledged_at IS NOT NULL AND m26_graduation_revoked_at IS NULL`
    - `template.deprecated = FALSE`

For `AgentRunTemplate` :

1. **Slug shape** : `^[A-Za-z0-9_-]{1,64}$`.
2. **System prompt non-empty** + ≤ 32 KiB.
3. **Tool whitelist non-empty** + every tool is in the global registry.
4. **Profile in registry**.
5. **Default provider in tenant's `m25_allowed_providers`**.
6. **Max steps + timeout + cost** within absolute bounds (1..200, 60..14400, 1..50000).
7. **Cosign signature** (if URL provided) — the lightweight check ; full Rekor verification happens at run admission.

### 8.4 Implementation skeleton

```rust
// crates/efe-agent-runner/src/webhook/server.rs

use axum::{
    extract::State,
    response::Json,
    routing::post,
    Router,
};
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};

pub async fn run(pg: PgPool, profiles: Arc<ProfileRegistry>, tls: TlsConfig) -> Result<(), Error> {
    let state = WebhookState { pg, profiles };
    let app = Router::new()
        .route("/validate-agentrun", post(validate_agentrun))
        .route("/validate-agentruntemplate", post(validate_agentruntemplate))
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8443").await?;
    axum_server::from_tcp(listener.into_std()?)
        .acceptor(RustlsAcceptor::new(tls))
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

async fn validate_agentrun(
    State(state): State<WebhookState>,
    Json(review): Json<AdmissionReview<AgentRun>>,
) -> Json<AdmissionReview<AgentRun>> {
    let req = match review.request {
        Some(r) => r,
        None => return Json(AdmissionReview {
            response: Some(AdmissionResponse::invalid("no request").into_review()),
            ..Default::default()
        }),
    };

    let resp = match state.run_validate(&req).await {
        Ok(()) => AdmissionResponse::from(&req),
        Err(e) => {
            tracing::warn!(?e, "admission rejected agent run");
            AdmissionResponse::from(&req).deny(e.to_string())
        }
    };
    Json(resp.into_review())
}
```

### 8.5 cert-manager integration

```yaml
# charts/efe-agent-runner/templates/webhook-cert.yaml
{{- if .Values.admissionWebhook.enabled }}
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe-agent-runner.fullname" . }}-webhook-cert
  namespace: {{ .Release.Namespace }}
spec:
  secretName: {{ include "efe-agent-runner.fullname" . }}-webhook-tls
  issuerRef:
    name: {{ .Values.admissionWebhook.certManager.issuerRef }}
    kind: ClusterIssuer
  duration: 2160h     # 90 d
  renewBefore: 720h   # 30 d
  dnsNames:
    - {{ include "efe-agent-runner.fullname" . }}-webhook
    - {{ include "efe-agent-runner.fullname" . }}-webhook.{{ .Release.Namespace }}
    - {{ include "efe-agent-runner.fullname" . }}-webhook.{{ .Release.Namespace }}.svc
  privateKey:
    algorithm: ECDSA
    size: 256
{{- end }}
```

The controller reloads the cert via `inotify` on `/var/run/webhook-tls/tls.crt` modifications. Reload is graceful : new connections use the new cert ; in-flight requests complete on the old cert.

### 8.6 Failure mode handling

`failurePolicy: Fail` means : if the webhook is down, ALL `AgentRun` CR creation/updates are rejected. This is the safe default.

To avoid a single-pod-crash → cluster-wide AgentRun freeze :
- The Helm chart sets `replicas: 2` for the controller deployment in production (HPA optional based on QPS).
- A PodDisruptionBudget ensures at least 1 webhook is always available during voluntary disruptions.
- The webhook has its own readiness probe at `/healthz` that returns 200 only if : DB is reachable, profile registry loaded, TLS cert valid for ≥ 1 day.

---

## 9. Firecracker driver — real implementation

### 9.1 Boot sequence target

Each microVM provisioning is a 6-step REST call sequence to the per-node Firecracker instance, accessed via UDS at `/var/run/firecracker/<vm_id>.socket` :

1. **PUT `/boot-source`** — kernel image + boot args.
2. **PUT `/drives/rootfs`** — the cosign-verified agent-core OCI image, mounted as ext4 read-only.
3. **PUT `/machine-config`** — vCPU + memory.
4. **PUT `/network-interfaces/eth0`** — TAP interface name (pre-created by the host CNI).
5. **PUT `/vsock`** — vsock CID assignment.
6. **PUT `/actions` `{"action_type": "InstanceStart"}`** — kick off the kernel boot.

A 7th call follows at run termination :

7. **PUT `/actions` `{"action_type": "SendCtrlAltDel"}`** — graceful shutdown (or `InstanceHalt` for kill).

### 9.2 Real driver implementation

```rust
// crates/efe-agent-runner/src/sandbox/firecracker_real.rs

use hyper::{client::conn::http1, Method, Request};
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct FirecrackerRealDriver {
    uds_dir: PathBuf,
    kernel_path: PathBuf,
    spawn_timeout: Duration,
    client: hyper_util::client::legacy::Client<UnixConnector, http_body_util::Full<bytes::Bytes>>,
}

impl FirecrackerRealDriver {
    pub fn from_env() -> AgentRunnerResult<Self> {
        Ok(Self {
            uds_dir: std::env::var("EFE_FC_UDS_DIR")
                .unwrap_or_else(|_| "/var/run/firecracker".into())
                .into(),
            kernel_path: std::env::var("EFE_FC_KERNEL")
                .unwrap_or_else(|_| "/opt/efe/firecracker/vmlinux".into())
                .into(),
            spawn_timeout: Duration::from_secs(30),
            client: hyper_util::client::legacy::Client::builder(
                hyper_util::rt::TokioExecutor::new(),
            )
            .build(UnixConnector),
        })
    }

    fn socket_path(&self, vm_id: &str) -> PathBuf {
        self.uds_dir.join(format!("{}.socket", vm_id))
    }

    async fn put<T: Serialize>(&self, vm_id: &str, path: &str, body: &T) -> AgentRunnerResult<()> {
        let started = Instant::now();
        let uri: hyper::Uri = Uri::new(self.socket_path(vm_id), path).into();
        let body_json = serde_json::to_vec(body)?;
        let req = Request::builder()
            .method(Method::PUT)
            .uri(uri)
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(body_json)))
            .map_err(|e| AgentRunnerError::Internal(format!("build req : {}", e)))?;

        let resp = tokio::time::timeout(self.spawn_timeout, self.client.request(req))
            .await
            .map_err(|_| AgentRunnerError::Internal("Firecracker REST timed out".into()))?
            .map_err(|e| AgentRunnerError::Internal(format!("Firecracker REST : {}", e)))?;

        crate::metrics::AGENT_FIRECRACKER_API_DURATION
            .with_label_values(&[path, &resp.status().as_u16().to_string()])
            .observe(started.elapsed().as_secs_f64());

        if !resp.status().is_success() {
            let status = resp.status();
            let body = http_body_util::BodyExt::collect(resp.into_body())
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            let body_str = String::from_utf8_lossy(&body);
            return Err(AgentRunnerError::MicroVmProvisioningFailed(
                format!("PUT {} returned {}: {}", path, status, body_str),
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl FirecrackerDriver for FirecrackerRealDriver {
    async fn spawn(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle> {
        let started = Instant::now();

        // 1. boot-source
        self.put(&config.vm_id, "/boot-source", &serde_json::json!({
            "kernel_image_path": self.kernel_path,
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off random.trust_cpu=on",
        })).await?;

        // 2. rootfs
        self.put(&config.vm_id, "/drives/rootfs", &serde_json::json!({
            "drive_id": "rootfs",
            "path_on_host": format!("/var/lib/efe/agent-runner/rootfs/{}.ext4", config.vm_id),
            "is_root_device": true,
            "is_read_only": true,
        })).await?;

        // 3. machine-config
        self.put(&config.vm_id, "/machine-config", &serde_json::json!({
            "vcpu_count": config.resources.vcpus,
            "mem_size_mib": config.resources.memory_mib,
            "smt": false,
            "track_dirty_pages": false,
        })).await?;

        // 4. network-interfaces/eth0
        self.put(&config.vm_id, "/network-interfaces/eth0", &serde_json::json!({
            "iface_id": "eth0",
            "host_dev_name": config.network_iface,
            "guest_mac": format!("AA:FC:00:00:{:02X}:{:02X}",
                (config.vsock_cid >> 8) & 0xFF, config.vsock_cid & 0xFF),
        })).await?;

        // 5. vsock
        self.put(&config.vm_id, "/vsock", &serde_json::json!({
            "vsock_id": "primary",
            "guest_cid": config.vsock_cid,
            "uds_path": format!("/var/run/efe-vsock-{}.sock", config.vm_id),
        })).await?;

        // 6. InstanceStart
        self.put(&config.vm_id, "/actions", &serde_json::json!({
            "action_type": "InstanceStart",
        })).await?;

        let handle = FirecrackerHandle {
            vm_id: config.vm_id.clone(),
            spawned_at: started,
            vsock_cid: config.vsock_cid,
        };

        crate::metrics::AGENT_MICROVM_PROVISION_DURATION
            .with_label_values(&[&config.vm_id, "ok"])
            .observe(started.elapsed().as_secs_f64());

        tracing::info!(
            vm_id = %config.vm_id,
            took_ms = started.elapsed().as_millis() as u64,
            "Firecracker microVM started",
        );
        Ok(handle)
    }

    async fn kill(&self, vm_id: &str) -> AgentRunnerResult<()> {
        self.put(vm_id, "/actions", &serde_json::json!({
            "action_type": "SendCtrlAltDel",
        })).await?;
        // After 10s grace period, send InstanceHalt.
        tokio::time::sleep(Duration::from_secs(10)).await;
        let _ = self.put(vm_id, "/actions", &serde_json::json!({
            "action_type": "InstanceHalt",
        })).await; // best-effort
        // Cleanup socket files.
        let _ = tokio::fs::remove_file(self.socket_path(vm_id)).await;
        Ok(())
    }

    async fn status(&self, vm_id: &str) -> AgentRunnerResult<VmStatus> {
        let uri: hyper::Uri = Uri::new(self.socket_path(vm_id), "/").into();
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(http_body_util::Full::new(bytes::Bytes::new()))
            .map_err(|e| AgentRunnerError::Internal(format!("build req : {}", e)))?;
        let resp = self.client.request(req).await
            .map_err(|e| AgentRunnerError::Internal(format!("status : {}", e)))?;
        if resp.status().is_success() { Ok(VmStatus::Running) } else { Ok(VmStatus::Crashed) }
    }
}
```

### 9.3 Failure handling + partial state cleanup

The 6-step boot sequence is **not atomic** : if step 5 succeeds but step 6 (InstanceStart) fails, the VM is configured but not running. The driver wraps each `spawn` in a guard that calls `kill(vm_id)` on any error path.

```rust
async fn spawn_with_cleanup(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle> {
    match self.spawn(config).await {
        Ok(handle) => Ok(handle),
        Err(e) => {
            tracing::warn!(vm_id = %config.vm_id, ?e, "spawn failed ; cleaning up");
            let _ = self.kill(&config.vm_id).await;
            Err(e)
        }
    }
}
```

### 9.4 Chaos test harness

`firecracker_chaos.rs` injects faults at each step :

| Fault | Test |
|---|---|
| Step 1 returns 500 | spawn errors at boot-source ; no socket file remains |
| Step 3 returns 400 (bad memory size) | spawn errors at machine-config ; cleanup invoked |
| Step 5 hangs > timeout | spawn errors with timeout ; kill(vm_id) attempts to reclaim |
| Step 6 returns 500 | spawn errors after vsock setup ; cleanup invoked |
| UDS socket disappears mid-spawn | spawn errors with connect refused ; metric recorded |
| Concurrent spawn + kill | race-tested with 100 iterations ; no leaked sockets |

Tests run only when `EFE_FC_CHAOS_TEST=true` and `/dev/kvm` is present.

### 9.5 Production node requirements

| Requirement | Spec |
|---|---|
| Kernel | ≥ 5.10 with KVM module loaded |
| `/dev/kvm` permissions | `0660 root:efe-agent-runner` |
| Firecracker binary | ≥ 1.10.0, at `/usr/local/bin/firecracker` |
| Kernel image (vmlinux) | ≥ 5.10, hardened (see [Annex H](#annex-h-kernel-hardening-profile)) |
| Disk | ≥ 100 GiB free under `/var/lib/efe/agent-runner/rootfs/` |
| Network | TAP interface support ; CNI plugin (Calico / Cilium) configured for vsock pinning |
| Memory | ≥ 16 GiB ; per-VM memory budget : 512 MiB default, up to 2 GiB per profile |
| CPU | ≥ 8 vCPU per node ; per-VM vCPU 1-2 |

---

## 10. vsock-relayer DaemonSet

### 10.1 Why a relayer

microVM agent-core processes need to talk to :
- The M21 LLM router (`efe-llm-router` pod, port 50051).
- The `efe-mcp-server` pod (port 8080) for tool calls + audit emission.

Constraints :
- microVM has **no network** in the K8s sense (no Pod CIDR, no DNS).
- vsock is the only comm channel ; the host process is the bridge.
- TLS termination + mTLS to upstream is required.
- Per-microVM rate limits to prevent runaway agent abuse.

The vsock-relayer is a **Go binary** running as a privileged DaemonSet (one pod per K8s node). It listens on vsock CID 2 (host CID) port 9999 and forwards HTTP requests to the appropriate upstream service via mTLS.

### 10.2 Wire protocol

The agent-core sends :
```
GET vsock://2:9999/v1/llm/complete HTTP/1.1
Host: efe-llm-router.local:50051
Authorization: Bearer <per-run JWT>
X-EFE-Run-Id: <uuid>
X-EFE-Tenant: <slug>
X-EFE-Step: <step-index>
Content-Type: application/json

{...}
```

The relayer :
1. Reads the request from vsock.
2. Validates the JWT (signed by the controller's key, audience `efe-agent-relay`).
3. Looks up the upstream service from the `Host` header (whitelisted set).
4. Applies per-microVM rate limit (token bucket keyed by `run_id`).
5. Forwards the request via mTLS to the upstream.
6. Streams the response back over vsock.
7. Emits an audit event (`tool_call_attempted` / `llm_call_completed`) to the audit relay endpoint.

### 10.3 Upstream whitelist

| Host header | Upstream service | Port | mTLS cert |
|---|---|---|---|
| `efe-llm-router.local:50051` | `efe-llm-router.efe-system.svc.cluster.local` | 50051 | `vsock-relayer-llm-client` |
| `efe-mcp-server.local:8080` | `efe-mcp-server.efe-system.svc.cluster.local` | 8080 | `vsock-relayer-mcp-client` |

Any other Host → 403.

### 10.4 Per-microVM rate limit

Per `(run_id, endpoint_class)` token bucket :

| Endpoint class | Budget | Refill |
|---|---|---|
| LLM call | 30 / min | 30 / min (matches typical step rate) |
| Tool call (read) | 60 / min | 60 / min |
| Tool call (write) | 10 / min | 10 / min |
| Audit emission | 200 / min | 200 / min (high to allow rich tracing) |
| Checkpoint | 5 / min | 5 / min |

On exceed → HTTP 429 returned over vsock to agent-core which retries via tenacity.

### 10.5 mTLS for upstream calls

cert-manager issues two `Certificate` resources mounted into the relayer pod :
- `vsock-relayer-llm-client.tls` : client cert for LLM router calls (CN: `vsock-relayer.efe-system`).
- `vsock-relayer-mcp-client.tls` : client cert for mcp-server calls.

The LLM router and mcp-server have admission policies that accept requests only from these specific CNs.

### 10.6 Implementation skeleton

```go
// cmd/vsock-relayer/main.go

package main

import (
    "context"
    "log/slog"
    "net/http"
    "os"
    "os/signal"
    "syscall"

    "github.com/mdlayher/vsock"
    "github.com/efe-ai/efe/cmd/vsock-relayer/pkg/proxy"
    "github.com/efe-ai/efe/cmd/vsock-relayer/pkg/ratelimit"
)

func main() {
    logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))

    upstreams := proxy.LoadUpstreamsFromEnv()
    rl := ratelimit.New(logger)

    handler := proxy.NewHandler(upstreams, rl, logger)
    server := &http.Server{
        Handler: handler,
    }

    listener, err := vsock.Listen(9999, nil)
    if err != nil {
        logger.Error("failed to listen on vsock", "err", err)
        os.Exit(1)
    }
    defer listener.Close()

    ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
    defer cancel()

    go func() {
        <-ctx.Done()
        server.Shutdown(context.Background())
    }()

    logger.Info("vsock-relayer listening on CID 2 port 9999")
    if err := server.Serve(listener); err != nil && err != http.ErrServerClosed {
        logger.Error("server failed", "err", err)
        os.Exit(1)
    }
}
```

### 10.7 DaemonSet manifest

```yaml
# charts/efe-agent-runner/templates/vsock-relayer-daemonset.yaml
{{- if .Values.vsockRelayer.enabled }}
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: {{ include "efe-agent-runner.fullname" . }}-vsock-relayer
  labels:
    {{- include "efe-agent-runner.labels" . | nindent 4 }}
    app.kubernetes.io/component: vsock-relayer
spec:
  selector:
    matchLabels:
      {{- include "efe-agent-runner.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: vsock-relayer
  template:
    metadata:
      labels:
        {{- include "efe-agent-runner.selectorLabels" . | nindent 8 }}
        app.kubernetes.io/component: vsock-relayer
    spec:
      hostPID: false
      hostNetwork: false
      serviceAccountName: {{ include "efe-agent-runner.fullname" . }}-vsock-relayer
      tolerations:
        - operator: Exists
      nodeSelector:
        {{- if .Values.kvm.required }}
        efe.ai/kvm-available: "true"
        {{- end }}
      containers:
        - name: relayer
          image: {{ .Values.vsockRelayer.image.repository }}:{{ .Values.vsockRelayer.image.tag }}
          securityContext:
            privileged: false
            allowPrivilegeEscalation: false
            capabilities:
              add: ["NET_BIND_SERVICE"]
              drop: ["ALL"]
            readOnlyRootFilesystem: true
          env:
            - name: EFE_RELAY_JWT_PUBLIC_KEY
              valueFrom:
                secretKeyRef:
                  name: efe-relay-jwt-key
                  key: pub.pem
            - name: EFE_LOG_FORMAT
              value: json
          volumeMounts:
            - name: vsock-dev
              mountPath: /dev/vsock
            - name: llm-client-tls
              mountPath: /var/run/tls/llm-client
              readOnly: true
            - name: mcp-client-tls
              mountPath: /var/run/tls/mcp-client
              readOnly: true
          resources:
            requests:
              cpu: 100m
              memory: 64Mi
            limits:
              cpu: 500m
              memory: 256Mi
          livenessProbe:
            httpGet:
              path: /healthz
              port: 8080
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /readyz
              port: 8080
            periodSeconds: 10
      volumes:
        - name: vsock-dev
          hostPath:
            path: /dev/vsock
            type: CharDevice
        - name: llm-client-tls
          secret:
            secretName: vsock-relayer-llm-client
        - name: mcp-client-tls
          secret:
            secretName: vsock-relayer-mcp-client
{{- end }}
```

### 10.8 Audit relay

Every cross-boundary call (LLM, tool) generates an audit event emitted to the controller's `/v1/audit/append` endpoint. The relayer ensures :
- The audit row has `actor_id = "system:vsock-relayer:<run_id>"` and `actor_role = "system"`.
- The HMAC chain stays intact (the controller's audit module handles chaining).

### 10.9 Failure modes

| Failure | Effect | Mitigation |
|---|---|---|
| Relayer pod crash | All in-flight microVM HTTP calls error out ; agent-core retries (tenacity) ; if relayer doesn't recover within 30 s, agent-core emits a `vsock_relay_failed` event and the controller marks run FAILED | DaemonSet ensures one relayer per node ; pod restart is automatic ; in worst case the K8s node is drained |
| Upstream service down | Relayer returns 503 to agent-core ; agent-core retries up to 3× ; on third failure, run is marked FAILED | Upstream services run with HA (≥ 2 replicas) |
| mTLS cert expired | Relayer fails to connect upstream ; readinessProbe fails ; pod restarts after cert-manager auto-renews | cert-manager auto-renews 30 d before expiry |

---

## 11. Next.js streaming routes (portal-admin)

### 11.1 Architecture

The browser-side `LiveTail` component currently polls `/api/agent/runs/${runId}/steps` (M25 stub never created). M26 implements two routes :

1. **`app/api/agent/runs/[id]/steps/route.ts`** — GET handler with SSE (Server-Sent Events). Long-lived connection ; pushes events as the agent emits steps.
2. **`app/api/agent/runs/[id]/tail/route.ts`** — GET handler with polling fallback (returns the last 50 steps + a continuation token). Used when SSE is blocked by intermediary proxies.

### 11.2 SSE handler

```typescript
// apps/portal-admin/app/api/agent/runs/[id]/steps/route.ts

import { NextRequest } from "next/server";
import { getRbacUser, getActiveTenant } from "@/lib/auth/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs"; // SSE needs Node runtime, not Edge

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";
const BACKEND_TOKEN = process.env.EFE_BACKEND_TOKEN ?? "";

export async function GET(
  req: NextRequest,
  { params }: { params: { id: string } },
) {
  const user = await getRbacUser();
  if (!user) {
    return new Response("unauthorized", { status: 401 });
  }
  const tenant = await getActiveTenant();
  if (!tenant) {
    return new Response("no active tenant", { status: 401 });
  }

  // Proxy the SSE upstream to efe-server.
  const upstream = await fetch(
    `${API_BASE}/v1/agent/runs/${params.id}/steps/stream`,
    {
      headers: {
        Authorization: `Bearer ${BACKEND_TOKEN}`,
        Accept: "text/event-stream",
        "X-EFE-Tenant": tenant,
        "X-EFE-User-Email": user.email ?? "",
      },
      signal: req.signal,
    },
  );

  if (!upstream.ok || !upstream.body) {
    return new Response(`upstream error: ${upstream.status}`, { status: upstream.status });
  }

  return new Response(upstream.body, {
    status: 200,
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      "Connection": "keep-alive",
      "X-Accel-Buffering": "no", // disable nginx buffering
    },
  });
}
```

### 11.3 Polling fallback

```typescript
// apps/portal-admin/app/api/agent/runs/[id]/tail/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getRbacUser, getActiveTenant } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

export async function GET(
  req: NextRequest,
  { params }: { params: { id: string } },
) {
  const user = await getRbacUser();
  if (!user) return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  const tenant = await getActiveTenant();
  if (!tenant) return NextResponse.json({ error: "no_tenant" }, { status: 401 });

  const since = req.nextUrl.searchParams.get("since") ?? "0";
  const r = await fetch(
    `${process.env.EFE_API_BASE}/v1/agent/runs/${params.id}/steps?since=${since}&limit=50`,
    {
      headers: {
        Authorization: `Bearer ${process.env.EFE_BACKEND_TOKEN}`,
        "X-EFE-Tenant": tenant,
      },
      cache: "no-store",
    },
  );
  if (!r.ok) return NextResponse.json({ error: "upstream" }, { status: r.status });
  return NextResponse.json(await r.json());
}
```

### 11.4 Updated LiveTail component

```typescript
// apps/portal-admin/components/agent/LiveTail.tsx (M26 replacement)

"use client";

import { useEffect, useState } from "react";
import type { AgentStep } from "@/lib/agent/api";

interface Props {
  runId: string;
  initialSteps: AgentStep[];
}

export default function LiveTail({ runId, initialSteps }: Props) {
  const [steps, setSteps] = useState<AgentStep[]>(initialSteps);
  const [connected, setConnected] = useState(false);
  const [terminal, setTerminal] = useState<string | null>(null);

  useEffect(() => {
    const es = new EventSource(`/api/agent/runs/${runId}/steps`);
    es.onopen = () => setConnected(true);
    es.addEventListener("agent_step", (ev) => {
      const step = JSON.parse((ev as MessageEvent).data) as AgentStep;
      setSteps((prev) => {
        if (prev.find((s) => s.id === step.id)) return prev;
        return [...prev, step];
      });
    });
    es.addEventListener("run_terminal", (ev) => {
      const data = JSON.parse((ev as MessageEvent).data);
      setTerminal(data.status);
      es.close();
    });
    es.onerror = () => {
      setConnected(false);
      es.close();
      // Fallback to polling.
      const poll = setInterval(async () => {
        const last = steps[steps.length - 1]?.step_index ?? -1;
        const r = await fetch(`/api/agent/runs/${runId}/tail?since=${last}`);
        if (r.ok) {
          const j = await r.json();
          setSteps((prev) => [...prev, ...j.steps]);
          if (j.terminal) {
            setTerminal(j.terminal_status);
            clearInterval(poll);
          }
        }
      }, 2000);
      return () => clearInterval(poll);
    };
    return () => es.close();
  }, [runId]);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-xs">
        <span className={connected ? "text-emerald-700" : "text-amber-700"}>
          {connected ? "● Live" : "● Reconnecting…"}
        </span>
        {terminal && (
          <span className="text-slate-700">→ Run reached {terminal}</span>
        )}
      </div>
      <ul className="font-mono text-xs space-y-1 max-h-[60vh] overflow-y-auto border rounded p-2">
        {steps.map((s) => (
          <li key={s.id} className="border-b py-1">
            <span className="text-slate-400">[{String(s.step_index).padStart(3, "0")}]</span>{" "}
            <span className="font-semibold">{s.kind}</span>{" "}
            <span className="text-slate-500">cost={s.cost_cents}c</span>{" "}
            <span className="text-slate-400">{new Date(s.created_at).toLocaleTimeString()}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
```

### 11.5 Browser observability

OpenTelemetry browser instrumentation captures :
- `agent_run_view_duration` — time from page load to first step rendered.
- `agent_sse_reconnect_count` — count of SSE drops per session.
- `agent_approval_action_latency` — time from button click to server response.

Spans are exported to the Grafana Tempo backend via the Next.js OTel exporter.

---

## 12. Orphan slot cleanup + reconcile resilience

### 12.1 The leak

In M25, `controller.rs::drive_pending_to_running` does (simplified) :

```rust
let slot = pool.lease().await?;          // step 3 — microVM running
let net_spec = NetworkPolicySpec::strict(tenant.clone(), run_id);
apply_network_policy(k8s, &net_spec).await?;  // step 5 — can fail
run::update_status(&mut tx, run_id, AgentRunStatus::Running).await?;  // step 6
```

If `apply_network_policy` fails, the leased slot is in `Running` state with no `agent_run` row pointing to it. The pool's pre-check (`total >= target_size + 1`) eventually rejects new leases ; the slot leaks until the warm_loop's `max_age` expiry (1 h default).

### 12.2 Fix : RAII guard

```rust
// crates/efe-agent-runner/src/sandbox/lease_guard.rs (NEW)

pub struct MicrovmLease {
    pool: Arc<MicrovmPool>,
    pub slot: MicrovmSlot,
    released: bool,
}

impl MicrovmLease {
    pub fn new(pool: Arc<MicrovmPool>, slot: MicrovmSlot) -> Self {
        Self { pool, slot, released: false }
    }

    /// Mark the lease as successfully bound to a run ; on drop, no release.
    pub fn commit(mut self) -> MicrovmSlot {
        self.released = true;
        std::mem::replace(&mut self.slot, MicrovmSlot::sentinel())
    }
}

impl Drop for MicrovmLease {
    fn drop(&mut self) {
        if !self.released {
            let pool = self.pool.clone();
            let vm_id = self.slot.handle.vm_id.clone();
            tokio::spawn(async move {
                if let Err(e) = pool.release(&vm_id).await {
                    tracing::warn!(vm_id = %vm_id, ?e, "failed to release leaked slot on drop");
                }
                crate::metrics::AGENT_MICROVM_LEAK_RECOVERED_TOTAL.inc();
            });
        }
    }
}
```

The controller becomes :

```rust
let lease = MicrovmLease::new(pool.clone(), pool.lease().await?);
// ... apply network policy, update status
let net_spec = NetworkPolicySpec::strict(tenant.clone(), run_id);
apply_network_policy(k8s, &net_spec).await?;  // if this fails, lease drops -> slot released
// ... only after everything succeeds :
let _slot = lease.commit();
```

### 12.3 Reaper goroutine

Even with the RAII guard, edge cases exist : controller pod crashes mid-flight, slot is in DB-tracked state but not in any in-memory pool. The reaper (`controller_reaper.rs`) runs every 5 minutes :

```rust
pub async fn reaper_loop(pg: PgPool, pools: Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>) {
    let mut tick = tokio::time::interval(Duration::from_secs(300));
    loop {
        tick.tick().await;
        if let Err(e) = reap_once(&pg, &pools).await {
            tracing::error!(?e, "reaper iteration failed");
        }
    }
}

async fn reap_once(pg: &PgPool, pools: &Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>) -> AgentRunnerResult<()> {
    // 1. List all currently-leased VMs (Running state, not Idle).
    // 2. For each VM, check : is there an active agent_run row referencing this vm_id?
    // 3. If NO and the slot has been Running for > 5 min, kill the VM and remove the slot.
    let pools = pools.lock().await;
    for (tenant, pool) in pools.iter() {
        let vms_in_use: Vec<String> = sqlx::query_scalar(
            r#"SELECT DISTINCT vm_id FROM agent_run
               WHERE tenant_id = $1
                 AND vm_id IS NOT NULL
                 AND status NOT IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')"#,
        )
        .bind(tenant)
        .fetch_all(pg)
        .await?;
        let in_use: HashSet<String> = vms_in_use.into_iter().collect();

        let stale = pool.list_stale(&in_use, Duration::from_secs(300)).await;
        for vm_id in stale {
            tracing::warn!(vm_id, %tenant, "reaping orphan microVM slot");
            let _ = pool.release(&vm_id).await;
            crate::metrics::AGENT_MICROVM_REAPED_TOTAL
                .with_label_values(&[tenant])
                .inc();
        }
    }
    Ok(())
}
```

### 12.4 Tests

- **happy path** : `MicrovmLease::commit()` keeps the slot bound ; no release on drop.
- **error path** : applying a NetworkPolicy that fails → `lease` drops without commit → `pool.release` called.
- **panic path** : an explicit `panic!` between lease and commit → the Drop runs → slot released (Tokio runtime catches panic in spawned task).
- **reaper happy path** : kill the controller pod after a successful lease but before status update ; restart the controller ; reaper detects the orphan after 5 min and reaps it.

---

## 13. Cosign signature pipeline finalization

### 13.1 The chain

For both `efe-agent-runner` controller image and `agent-core` microVM image :

```
GitHub Actions push to `main`
   │
   ├─► docker build → push to ghcr.io/efe-ai/<image>:<tag>
   │
   ├─► cosign sign with KMS-backed key
   │       │
   │       └─► Rekor entry created (transparency log)
   │
   └─► cosign verify in CI → confirms signature + Rekor entry valid

Production deployment :
   │
   ├─► Helm chart values.yaml pinned by sha256 digest
   │
   └─► At controller boot :
         ├─► Read EFE_M25_IMAGE_WHITELIST (digests)
         ├─► For each AgentRunTemplate, verify
         │     - the template's image digest matches whitelist
         │     - cosign signature matches the keyref
         │     - Rekor inclusion proof valid
         └─► Refuse to start if any check fails
```

### 13.2 Image whitelist enforcement at runtime

```rust
// crates/efe-agent-runner/src/sandbox/image.rs (M26 enhancement)

pub fn verify_image_whitelist_with_cosign(
    whitelist: &ImageWhitelist,
    image_ref: &str,
) -> AgentRunnerResult<()> {
    if !whitelist.allows(image_ref) {
        crate::metrics::AGENT_IMAGE_REJECTED_WHITELIST_TOTAL.inc();
        return Err(AgentRunnerError::ImageNotInWhitelist(image_ref.to_string()));
    }

    // M26 : also verify the cosign signature + Rekor inclusion.
    let entry = whitelist.entries.iter()
        .find(|e| image_ref.starts_with(&e.repository))
        .ok_or_else(|| AgentRunnerError::ImageNotInWhitelist(image_ref.to_string()))?;

    let key_ref = entry.cosign_key_ref.as_deref()
        .ok_or_else(|| AgentRunnerError::Internal("cosign key ref missing".into()))?;

    cosign_verify::verify(image_ref, key_ref, /* require Rekor */ true)
        .map_err(|e| AgentRunnerError::ImageNotInWhitelist(format!("{}: {}", image_ref, e)))?;

    Ok(())
}
```

### 13.3 GitHub Actions workflow

```yaml
# .github/workflows/release-agent-runner.yml (M26 addition)
name: Release agent-runner

on:
  push:
    tags: ["agent-runner-v*"]

jobs:
  build-and-sign:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build + push controller image
        id: build_controller
        uses: docker/build-push-action@v5
        with:
          context: ./crates/efe-agent-runner
          push: true
          tags: ghcr.io/efe-ai/efe-agent-runner:${{ github.ref_name }}
          provenance: mode=max

      - uses: sigstore/cosign-installer@v3
        with:
          cosign-release: v2.4.0

      - name: Sign controller image
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          cosign sign --yes --key gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign \
            ghcr.io/efe-ai/efe-agent-runner@${{ steps.build_controller.outputs.digest }}

      - name: Build + push agent-core image
        id: build_agent_core
        uses: docker/build-push-action@v5
        with:
          context: ./agent-core
          push: true
          tags: ghcr.io/efe-ai/agent-core:${{ github.ref_name }}
          provenance: mode=max

      - name: Sign agent-core image
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          cosign sign --yes --key gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign \
            ghcr.io/efe-ai/agent-core@${{ steps.build_agent_core.outputs.digest }}

      - name: Verify signatures
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          cosign verify --key gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign \
            ghcr.io/efe-ai/efe-agent-runner@${{ steps.build_controller.outputs.digest }}
          cosign verify --key gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign \
            ghcr.io/efe-ai/agent-core@${{ steps.build_agent_core.outputs.digest }}
```

### 13.4 Key rotation

Every 12 months : rotate the cosign signing key in the KMS key ring. Old digests remain verifiable (Rekor entries are immutable) ; new builds use the new key. The image whitelist references the key version explicitly :

```
EFE_M25_IMAGE_WHITELIST="ghcr.io/efe-ai/agent-core@sha256:abcd..."
EFE_M25_COSIGN_KEY_REF="gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign/cryptoKeyVersions/3"
```

A controller restart picks up the new env var ; in-flight runs keep their old verification state.

---

## 14. SLO framework + alerting

### 14.1 SLO catalogue (overview)

14 SLOs are defined ; full catalogue in [Annex D](#annex-d-slo-catalogue-full-table). Categories :

| Category | # SLOs |
|---|:--:|
| Run lifecycle (creation, provisioning, completion) | 4 |
| Approval latency | 2 |
| Sandbox security | 3 |
| Cost cap | 1 |
| API availability | 2 |
| Audit chain integrity | 1 |
| Data lifecycle (GDPR) | 1 |

### 14.2 Sample SLO : run provisioning latency

```yaml
# Helm template snippet
- name: agent_run_provisioning_p99_under_5s
  description: |
    99% of agent runs transition from PENDING to RUNNING within 5 seconds.
    A persistent breach indicates Firecracker spawn slowness or pool exhaustion.
  expression: |
    histogram_quantile(0.99,
      sum by (le) (rate(efe_agent_microvm_provision_duration_seconds_bucket[5m]))
    ) < 5
  burnRate:
    fast:  # 2% in 1h → page
      window: 1h
      threshold: 0.02
      severity: P1
    slow:  # 5% in 6h → ticket
      window: 6h
      threshold: 0.05
      severity: P3
```

### 14.3 Burn-rate alert routing

| Severity | Routing | Cooldown |
|---|---|---|
| P0 (sandbox escape, audit HMAC fail) | PagerDuty `efe-agent-oncall-prod` ; phone + SMS | 1 min |
| P1 (SLO fast burn) | PagerDuty `efe-agent-oncall-prod` ; push only | 5 min |
| P2 (SLO slow burn) | PagerDuty `efe-agent-oncall-business-hours` | 15 min |
| P3 (info / ticket) | Slack `#efe-agent-runtime-alerts` | n/a |

### 14.4 PrometheusRule + Alertmanager manifests

```yaml
# charts/efe-agent-runner/templates/slo-prometheusrule.yaml (excerpt)
{{- if .Values.slo.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {{ include "efe-agent-runner.fullname" . }}-slo
  labels:
    {{- include "efe-agent-runner.labels" . | nindent 4 }}
    prometheus: kube-prometheus
spec:
  groups:
    - name: efe-agent-runtime-slo
      interval: 30s
      rules:
        # SLO 1 : sandbox escape MUST be 0
        - alert: AgentSandboxEscapeAttempt
          expr: increase(efe_agent_sandbox_escape_attempts_total[1m]) > 0
          for: 0s
          labels:
            severity: P0
            team: efe-agent
          annotations:
            summary: "SECURITY P0 — sandbox escape attempt detected"
            description: |
              Tenant {{`{{ $labels.tenant }}`}} : kind={{`{{ $labels.kind }}`}}
              Runbook : https://efe.ai/docs/runbooks/M26#5-1-sandbox-escape

        # SLO 2 : audit chain HMAC verification rate must be 100%
        - alert: AgentAuditChainHmacFailure
          expr: |
            increase(efe_agent_audit_hmac_verify_failures_total[5m]) > 0
          for: 0s
          labels:
            severity: P0
          annotations:
            summary: "P0 — audit chain HMAC verify failed"

        # SLO 3 : provisioning P99 latency
        - alert: AgentProvisioningSlow
          expr: |
            histogram_quantile(0.99, sum by (le) (rate(efe_agent_microvm_provision_duration_seconds_bucket[5m]))) > 5
          for: 10m
          labels:
            severity: P1
          annotations:
            summary: "P1 — provisioning P99 > 5s"

        # SLO 4 : approval response latency
        - alert: AgentApprovalSlow
          expr: |
            histogram_quantile(0.95, sum by (le) (rate(efe_agent_approval_response_latency_seconds_bucket[1h]))) > 3600
          for: 1h
          labels:
            severity: P2
          annotations:
            summary: "P2 — 95th approval latency > 1h"
        # ... 10 more rules
{{- end }}
```

### 14.5 Grafana dashboards

Three dashboards ship :

1. **`M25 Agent Runtime — Operations`** — controller health, pool occupancy, provision rate, audit chain status.
2. **`M25 Agent Runtime — Tenant View`** — per-tenant cost, run counts, approval latency, failure rate (filtered by tenant slug).
3. **`M25 Agent Runtime — Security`** — sandbox escape counters (must always be 0), tool whitelist rejections, image whitelist rejections, HMAC failures.

---

## 15. Bench at production scale

### 15.1 Targets

| Bench | Scale | Duration | Pass criteria |
|---|---|---|---|
| `bench_50tenants_5concurrent` | 50 tenants × 5 concurrent runs = 250 simultaneous microVMs | 4 h | Provisioning P99 < 5 s, run failure rate < 1 %, 0 escape attempts |
| `bench_approval_storm` | 100 runs all needing approval at the same wall-clock minute | 30 min | All approvals processed within SLA, no token race wins |
| `bench_cost_cap_thrash` | 50 runs all with $0.01 cap, designed to hit cap fast | 10 min | All 50 transition to COST_CAPPED within 60s of cap exceeded ; no over-spend > 5 % |
| `bench_audit_chain_under_load` | 1000 events/s for 1 h | 1 h | Audit append P99 < 50 ms, verify chain succeeds end-to-end |
| `bench_pod_crash_recovery` | Kill controller pod every 30 s during 250-run bench | 30 min | Reaper recovers all orphaned slots within 5 min ; no DB inconsistency |

### 15.2 Test infrastructure

- **Cluster** : `efe-bench-m26` k3d cluster with 4 worker nodes, each with `/dev/kvm` (KVM via QEMU nested virt for CI-grade testing ; bare-metal for the canonical bench).
- **Postgres** : RDS db.r6g.xlarge with multi-AZ.
- **Concurrent driver** : Python script using `httpx` async to fire run-creation requests with realistic prompt distribution.
- **Observability** : full Prometheus + Grafana stack on the bench cluster ; results exported to `bench_baselines/v2.1.0/baseline.json`.

### 15.3 Bench result artifact

```json
{
  "milestone": "M26",
  "version": "2.1.0",
  "bench_run_id": "...",
  "started_at": "2027-Q4-15T...",
  "ended_at": "...",
  "scenarios": {
    "bench_50tenants_5concurrent": {
      "pass": true,
      "metrics": {
        "provisioning_p99_seconds": 3.2,
        "run_failure_rate_pct": 0.4,
        "sandbox_escape_attempts_total": 0,
        "max_concurrent_microvms": 250,
        "tenant_count": 50
      }
    },
    ...
  }
}
```

A regression ≥ 10 % from the M25 baseline (`bench_baselines/v2.0.0/baseline.json`) blocks the release.

---

## 16. Disaster recovery procedures

### 16.1 Disaster scenarios

| Scenario | Impact | Recovery time objective | Recovery point objective |
|---|---|---|---|
| **DB region failover** | All efe-server replicas lose primary | ≤ 5 min | ≤ 30 s (continuous WAL streaming to standby) |
| **Firecracker fleet loss** (all worker nodes go down) | All in-flight runs terminate | ≤ 15 min (auto-scale + cordon) | runs lose their state ; agent_state checkpoints survive |
| **KMS key compromise** | All encrypted blobs at risk | ≤ 1 h to rotate keys + re-encrypt active checkpoints | n/a (no data loss, but key rotation is invasive) |
| **cosign signing key compromise** | All future signatures suspect | ≤ 6 h (rotate key, re-sign current images, update whitelist) | n/a |
| **Audit chain HMAC re-key** | Old chain invalid against new key | ≤ 24 h to verify all chains + cut over | 0 (chain integrity preserved via dual-key window) |

### 16.2 Game day schedule

4 game days per year, one per quarter :

| Quarter | Scenario | Duration |
|---|---|---|
| Q1 | DB region failover (simulated during business hours) | 2 h |
| Q2 | Firecracker fleet loss (drain all KVM nodes) | 2 h |
| Q3 | KMS rotation drill | 4 h |
| Q4 | cosign key + audit HMAC rotation drill | 4 h |

Each game day is run by SRE on-call lead with eng + DPO observers. Outcomes are documented in `docs/runbooks/M26_DR_DRILL_LOG.md` with timestamps and lessons learned.

### 16.3 Backup + restore artifacts

| Artifact | Backup mechanism | Verification cadence |
|---|---|---|
| PostgreSQL (agent_run, agent_step, agent_audit, …) | PITR via WAL streaming + daily base backup to S3 | Quarterly restore drill (read replica from backup) |
| `agent_state` encrypted blobs | Same as Postgres (it's a column) ; dual-region replication of S3 backups | Quarterly checkpoint replay test |
| KMS keys | Cloud KMS automatic backups + disaster recovery key | Semi-annual rotation test |
| cosign signing key | Hardware HSM in 2 regions ; backup KMS key | Annual rotation test |
| Audit HMAC master key | Hashicorp Vault with sealed backup | Annual rotation test |

---

## 17. Penetration test integration

### 17.1 Scope

A 30-scenario pen test bundle ships in `bench/m26_pentest/` and runs against the `efe-bench-m26` cluster. Scenarios :

| # | Scenario | Threat |
|---|---|---|
| 1-5 | Sandbox escape vectors (ptrace, seccomp bypass, DNS exfil, image swap, fs breakout) | T1 — sandbox compromise |
| 6-10 | Tool whitelist bypass (hallucinated tool, layer 1/2/3 enforcement, race) | T2 — privilege escalation |
| 11-13 | Approval token replay, forgery, expiry bypass | T3 — Art. 14 oversight bypass |
| 14-16 | NetworkPolicy bypass (egress to non-whitelisted host, metadata endpoint, kube-dns DNS poisoning) | T4 — data exfiltration |
| 17-19 | Audit chain tamper detection (DB row delete, HMAC re-compute, chain reorder) | T5 — audit evasion |
| 20-22 | Cost cap bypass (delayed cost reporting, race, batch evasion) | T6 — resource abuse |
| 23-25 | RBAC + tenant cross-tenant attack (forge JWT tenant claim, role escalation, RLS bypass) | T7 — multi-tenant breach |
| 26-28 | DPIA + graduation bypass (set flags directly via SQL, bypass admission webhook) | T8 — compliance bypass |
| 29-30 | Cosign signature forgery, Rekor entry tampering | T9 — supply chain attack |

### 17.2 Findings categorization

| Sev | Definition | Resolution SLA |
|---|---|---|
| P0 | Sandbox escape achievable, audit chain forgery achievable, cross-tenant data leak | Block GA ; immediate fix required |
| P1 | Tool whitelist bypass (any layer), approval token replay, RBAC escalation | Fix before GA |
| P2 | Cost cap bypass, NetworkPolicy bypass to non-critical egress | Fix in next minor |
| P3 | Information disclosure (e.g. error message leaking internal state) | Fix when convenient |

**M26 GA criterion** : 0 P0/P1 findings open.

### 17.3 External pen test cadence

| Test | Cadence | Scope |
|---|---|---|
| Internal automated bundle (this 30-scenario set) | Per release ; on every PR to `main` | Full M25 + M26 surface |
| Internal manual pen test (ad-hoc) | Quarterly | Random surface area, depth focus |
| External pen test (third-party firm) | Annual | Full surface, including infrastructure |

External pen test results land as `pen_test_run` rows in the DB ; the `M25_INCIDENT_LOG.json` compliance pack export references these for auditor review.

---

## 18. Compliance pack integration (R-100)

### 18.1 Quarterly compliance pack additions

The R-100 compliance regen cron (existing project mechanism) gets 5 new outputs for M26 :

| Output | Source | Content |
|---|---|---|
| `M25_DPIA_LOG.json` | `tenant_feature.m25_dpia_*` columns | Per-tenant DPIA signed_at, expires_at, addendum_hash, last refresh, DPO email |
| `M25_AGENT_RUN_AUDIT.json` | `agent_run` + `agent_audit` last 90 days | Full run lifecycle for each tenant, with linked audit events |
| `M25_TOOL_REJECTION_LOG.json` | `efe_agent_tool_rejected_whitelist_total` + audit | Rejection events with reason (LLM hallucination vs. malicious attempt) |
| `M25_APPROVAL_LOG.json` | `agent_approval_request` last 90 days | Approve/reject decisions with rationales (pseudonymized) |
| `M25_INCIDENT_LOG.json` | `pen_test_run` + `dpo_review` | Pen test results, DPO reviews, AI Act Art. 26 reports |

### 18.2 AI Act Art. 26 post-market monitoring

A new endpoint `GET /v1/agent/post-market/quarterly` returns a JSON-LD document with :
- Total run counts by tenant + outcome.
- Aggregate cost by tenant.
- Sandbox escape attempts (always 0 expected).
- Tool rejection rate.
- Approval grant/reject ratio.
- DPIA expiration upcoming events.
- Provider distribution (% by provider).

This document is consumed by the auditor portal and cross-referenced against the DPIA addendum for compliance verification.

### 18.3 GDPR Art. 17 cascade enhancements

M25 already had cascade in scope. M26 adds :
- **Audit chain pseudonymization** : when a tenant exercises Art. 17, the audit chain rows are NOT deleted (regulatory retention requirement) but personal data fields are replaced with pseudonyms (`actor_id` → SHA-256 hash, `payload_json` → DLP-scrubbed). The HMAC chain remains valid (the hash is computed on the pseudonymized payload during the cascade transaction).
- **Cross-region cascade** : when multi-region is implemented (future), Art. 17 cascade triggers a fan-out ; M26 documents the contract (per-region listener subscribes to `agent_audit_cascade` topic).

---

## 19. Multi-tenant isolation hardening

### 19.1 Defense layers

| Layer | M25 | M26 enhancement |
|---|---|---|
| Postgres RLS | Per-tenant policies on all 7 tables | Add deny-default policies (no rows visible without tenant set) |
| Network | NetworkPolicy default-deny + per-run additive | Add per-tenant namespace ; deny cross-namespace traffic |
| K8s RBAC | Per-cluster role + binding | Per-tenant ServiceAccount tokens for AgentRun creation |
| Webhook | Validate tenant claim in JWT | Validate tenant claim matches CR `spec.tenantId` AND user has RBAC on that tenant |
| Audit chain | Per-tenant HMAC key | Add per-quarter sub-key derivation (rotates without losing prior chain integrity) |
| Image whitelist | Cosign signature verified | Per-tenant `m25_allowed_image_digests` (overrides global whitelist for tenants with private custom agent-cores) |

### 19.2 Per-tenant namespace

In M26, each tenant gets a dedicated K8s namespace : `efe-agent-runtime-<tenant_slug>`. AgentRun pods land in this namespace ; NetworkPolicies are scoped accordingly. Benefits :
- Easier audit (filter by namespace).
- Easier DR (drop a tenant by deleting the namespace).
- Easier isolation (NetworkPolicies compose per namespace).

The Helm chart's controller is in the shared `efe-system` namespace and has cross-namespace reconcile RBAC.

---

## 20. Data lifecycle management

### 20.1 Data classes

| Data class | Location | Retention | Erasure |
|---|---|---|---|
| Agent run records | `agent_run`, `agent_step` | 90 d default ; tenant-configurable up to 7 y | Cascade on Art. 17 |
| Approval requests | `agent_approval_request` | 365 d | Cascade |
| Audit chain | `agent_audit` | 7 y (regulatory) | Pseudonymize, never delete |
| Encrypted state checkpoints | `agent_state` | 30 d after run terminal | Cascade |
| Cross-run memory | `agent_memory` | tenant-configurable TTL ; max 365 d | Cascade |
| Tenant DPIA history | `tenant_feature.m25_dpia_*` | 7 y | Pseudonymize tenant_id |

### 20.2 Retention enforcement

A nightly cron (`agent_data_retention_cron`) :
1. Identifies records older than retention window.
2. For records in cascade-eligible tables : DELETE.
3. For audit records : pseudonymize (replace `actor_id`, `payload_json`).
4. Logs the action to `gdpr_data_retention_audit`.

### 20.3 Pseudonymization function

```sql
CREATE OR REPLACE FUNCTION fn_agent_audit_pseudonymize(target_tenant TEXT)
RETURNS BIGINT AS $$
DECLARE
    affected BIGINT := 0;
BEGIN
    -- Pseudonymize actor_id (replace with SHA-256 hash).
    UPDATE agent_audit
    SET
        actor_id = encode(digest(actor_id || tenant_id, 'sha256'), 'hex'),
        payload_json = jsonb_strip_personal_data(payload_json)
    WHERE tenant_id = target_tenant
      AND created_at < NOW() - INTERVAL '7 years';
    GET DIAGNOSTICS affected = ROW_COUNT;
    RETURN affected;
END;
$$ LANGUAGE plpgsql;
```

`jsonb_strip_personal_data` is a custom SQL function that removes/replaces fields like `email`, `phone`, `name` from arbitrary JSONB.

---

## 21. Cost model — production

### 21.1 Per-tenant cost components

| Component | Unit | M25 baseline | M26 production target |
|---|---|---|---|
| Controller pod | $0.05 / h | shared across tenants | shared, ~$36/mo |
| Per-microVM CPU + memory | $0.001 / min | $0.06/h | unchanged |
| Per-microVM disk | $0.0005 / min | $0.03/h | unchanged |
| LLM cost (variable) | per token | provider-specific | unchanged |
| mcp-server tool calls | $0 (in-cluster) | 0 | 0 |
| vsock-relayer | $0.02 / h / node | n/a | $14.40/mo/node × 3 nodes = $43.20/mo |
| Admission webhook | shared with controller | n/a | included |

### 21.2 Tenant pricing model

For STABLE tenants, EFE 2.1 launches with two tiers :

| Tier | Monthly fee | Per-run included | Overage |
|---|---|---|---|
| **Pro** | $299 | 100 runs | $0.50/run |
| **Enterprise** | $999 | 1000 runs | $0.20/run |

Tenants on EXPERIMENTAL pay nothing (design-partner pilot). The first 90 days post-graduation are free for early adopters as a thank-you.

### 21.3 Cost guardrails

- Per-run cap : enforced by SIGTERM (M25 mechanism, retained).
- Daily tenant budget : `m25_daily_budget_cents` ; exceeded → new runs rejected with HTTP 429 + Slack alert.
- Monthly tenant budget : `m25_monthly_budget_cents` ; same enforcement.
- Cost alarm : 80 % of monthly budget → email tenant operator.

---

## 22. Performance budget

| Operation | M25 measured | M26 target | Strict (alert if exceeded) |
|---|---|---|---|
| AgentRun creation (REST API) | < 200 ms P99 | < 150 ms P99 | < 300 ms P99 |
| MicroVM provisioning (cold) | < 200 ms P99 | < 150 ms P99 | < 500 ms P99 |
| MicroVM provisioning (warm pool hit) | < 50 ms P99 | < 30 ms P99 | < 100 ms P99 |
| LLM call (round-trip via relayer) | provider-dependent | < provider P99 + 50 ms | < provider P99 + 100 ms |
| Tool call (read) | < 50 ms P99 | < 30 ms P99 | < 100 ms P99 |
| Audit append | < 20 ms P99 | < 15 ms P99 | < 50 ms P99 |
| Approval grant | < 100 ms P99 | < 80 ms P99 | < 200 ms P99 |
| Audit chain verify (1 tenant, 90 days) | < 30 s P99 | < 20 s P99 | < 60 s P99 |

---

## 23. Migration plan : EXPERIMENTAL → STABLE

### 23.1 Tenant migration workflow

```bash
# 1. Verify graduation criteria (read-only check)
efe-admin agent graduate check --tenant acme

# OUTPUT:
# Operational :
#   ✓ 30 days in EXPERIMENTAL (62 days)
#   ✓ ≥ 100 successful runs (147)
#   ✓ 0 sandbox escape attempts (0)
#   ✓ Failure rate < 5% (1.2%)
#   ✓ DPIA valid (signed 2027-Q3, expires 2028-Q3)
#   ✗ Production graduation acknowledgement NOT signed
# Compliance :
#   ✓ Audit chain verified
#   ✓ R-100 compliance pack regen succeeded (last : 2027-Q4)
#   ✗ DPO has not reviewed previous quarter (deadline : tomorrow)
#
# Status : 2 missing items. Action required before graduation.

# 2. After all 6 + 4 criteria are met :
efe-admin agent graduate confirm --tenant acme \
  --acknowledgement-hash sha256:<hash-of-signed-pdf> \
  --confirm
# Sets m26_graduation_acknowledged_at = NOW()

# 3. Verify post-graduation
efe-admin agent graduate status --tenant acme
# OUTPUT :
# Tenant : acme
# Channel : STABLE
# Graduated at : 2027-12-15T...
# Acknowledgement hash : sha256:abcd...
# DPIA expires : 2028-Q3-...
```

### 23.2 Reversibility

If a tenant develops issues post-graduation (e.g. sustained P0/P1 incidents), they can be reverted :

```bash
efe-admin agent graduate revoke --tenant acme \
  --reason "3 P0 sandbox escape attempts in last 30 days, requires review" \
  --confirm
# Sets m26_graduation_revoked_at = NOW()
# Tenant returns to EXPERIMENTAL channel ; webhook applies relaxed validation
```

Revocation does NOT terminate in-flight runs ; it only affects new run creation.

### 23.3 Cluster-level graduation

The cluster graduation (sign-off from 6 stakeholders) is captured in `docs/release/M26_GRADUATION_SIGN_OFF.md` with 6 wet-ink signatures (or e-signatures via `efe-admin signature request`). The Helm chart values.yaml gets `release.channel: STABLE` once signed off.

---

## 24. Sunset clause review + reconfirmation

### 24.1 M25 EXPERIMENTAL sunset clause

> "Sunset clause if < 2 tenants at 12 months."

### 24.2 M26 STABLE channel sunset

After M26 graduation, the STABLE channel has its own sunset clause :

> "**STABLE sunset clause** : if < 3 tenants at 24 months OR aggregate run volume < 10,000/quarter at 18 months, the agent runtime is reviewed by the Caliper Steering Committee for a phased deprecation. Phase 1 : freeze new tenant onboarding. Phase 2 : 12-month migration window. Phase 3 : end-of-life."

### 24.3 Caliper Steering Committee (CSC)

| Member | Role |
|---|---|
| Eng lead | Implementation + technical debt steward |
| SRE on-call lead | Operational health steward |
| Security lead | Threat model + pen test steward |
| DPO | GDPR + AI Act steward |
| Product lead | Tenant feedback + GTM steward |
| Legal | Contract + compliance steward |

CSC meets monthly for the first 12 months post-graduation, then quarterly. Outputs : meeting notes published in `docs/csc/`.

---

## 25. AI Act + GDPR compliance updates

### 25.1 AI Act Art. 9 (high-risk AI risk management)

M25 implemented the framework. M26 :
- **Art. 9.2(a)** : risk management system continuously updated → linked to CSC meeting cadence.
- **Art. 9.2(c)** : adoption of suitable risk management measures → linked to pen test cadence + SLO burn-rate alerts.
- **Art. 9.2(g)** : examination of impact on fundamental rights → linked to DPO quarterly review.

### 25.2 AI Act Art. 14 (human oversight)

- **Art. 14.4(a)** : oversight by natural persons → operator approval workflow (M25 retained).
- **Art. 14.4(d)** : ability to stop → `terminate_agent_run` endpoint (M25 retained).
- **Art. 14.4(e)** : ability to interpret outputs → `final_output_json` always preserved with provenance metadata.

### 25.3 AI Act Art. 26 (post-market monitoring)

NEW for M26 : `GET /v1/agent/post-market/quarterly` returns the standardized JSON-LD report. The DPO + Eng lead jointly sign the quarterly report ; auditor portal consumes it.

### 25.4 GDPR Art. 22 (automated decision-making)

The agent never makes automated decisions affecting data subjects without prior operator approval (write tools require approval token). M26 reinforces this :
- The webhook validates that any template requesting a write tool also has `require_approval_for_writes: true`.
- A new dashboard panel : "Write tool approval coverage" shows 100 % of write tool calls had a granted token.

### 25.5 GDPR Art. 35 (DPIA)

M25 mandates DPIA. M26 adds :
- DPIA-addendum-versioning : the addendum hash is bumped quarterly to reflect any platform changes ; tenants are notified 90 days before expiry.
- Auditable DPIA review log per tenant.

---

## 26. Threat model update

### 26.1 New threats vs M25

| Threat | M25 status | M26 mitigation |
|---|---|---|
| **REST gateway forgery** (attacker forges OIDC JWT) | n/a (no gateway) | OIDC ES256 signature validation + audience binding + per-subject rate limit |
| **Webhook bypass** (attacker creates AgentRun via direct etcd) | n/a (no webhook) | Webhook is `failurePolicy: Fail` ; OPA Gatekeeper as defence-in-depth |
| **vsock-relayer exploitation** (attacker compromises relayer pod, extracts tenant tokens) | n/a (no relayer) | Per-microVM rate limit + JWT pinning ; relayer mTLS to upstream ; relayer pod security context strict |
| **Cosign signature replay** (attacker swaps image to old known-signed but vulnerable version) | partial (no Rekor verify) | M26 verifies Rekor inclusion + entry recency at boot ; image whitelist requires `min_signed_at` |
| **Per-tenant namespace escape** | n/a (no per-tenant ns) | NetworkPolicies enforce ns isolation ; Pod Security Standards = `restricted` |

### 26.2 Reconsidered M25 threats

| Threat | M25 mitigation | M26 strengthening |
|---|---|---|
| Sandbox escape | seccomp + ptrace deny + read-only rootfs | Add `mount` syscall deny ; kernel hardening profile (Annex H) |
| Tool whitelist bypass | 3-layer enforcement | Add 4th layer at vsock-relayer (only allow tool names from a known set) |
| Approval token replay | single-use HMAC | Add nonce + time-bound TTL with grace period for clock skew |
| Audit chain tampering | per-tenant HMAC chain | Add Rekor-style inclusion proof for selected events (sandbox escape, GDPR cascade) |

---

## 27. Activation flags + tenant enablement

### 27.1 Server flags

| Flag | M26 default | Purpose |
|---|---|---|
| `EFE_FEATURE_AGENT_RUNTIME_ENABLED` | false | Master switch (M25 retained) |
| `EFE_FEATURE_AGENT_RUNTIME_M26_GRADUATION` | false | Enables the graduation endpoint + DB column writes |
| `EFE_FEATURE_AGENT_RUNTIME_WEBHOOK_STRICT` | false | When true, webhook applies STABLE rules even for EXPERIMENTAL tenants (used for shadow testing) |

### 27.2 Tenant flags (DB)

Existing M25 columns are retained. New for M26 :
- `m26_graduation_acknowledged_at` (TIMESTAMPTZ).
- `m26_graduation_revoked_at` (TIMESTAMPTZ).
- `m26_graduation_acknowledgement_hash` (TEXT, sha256:...).

### 27.3 Per-template flags

Templates get a new field :
- `min_channel_required` : `EXPERIMENTAL` (default) or `STABLE`. STABLE-only templates can only be instantiated by graduated tenants.

---

## 28. Sprint plan D86–D92

### 28.1 Day-by-day

| Day | Focus | Deliverable |
|---|---|---|
| **D86** | Architecture review + this doc + tenant graduation workflow design | This doc reviewed + sign-off ; SQL migrations 0017-0020 drafted |
| **D87** | REST gateway implementation (efe-server v1::agent module) | 23 endpoints + integration tests passing |
| **D88** | Admission webhook server + cert-manager wiring | Webhook integration test + Helm chart change |
| **D89** | FirecrackerUdsDriver real implementation + chaos tests | Driver passes all 6 chaos scenarios |
| **D90** | vsock-relayer Go binary + DaemonSet manifest | E2E test : agent-core makes LLM call via relayer, audit row appears |
| **D91** | Next.js streaming routes + LiveTail update + reaper goroutine + RAII guard | Portal-admin smoke test ; bench scenario for orphan slot cleanup |
| **D92** | SLO PrometheusRule + alertmanager + 30-scenario pen test bundle + DR game day Q1 | All SLOs deployed ; pen test report ; DR drill log |

### 28.2 Critical path

```
D86 (arch + migrations)
  └─► D87 (REST gateway) ──┐
  └─► D88 (webhook) ───────┤── D91 (portal + reaper) ─► D92 (SLO + pen test + DR)
  └─► D89 (firecracker) ───┤
  └─► D90 (vsock-relayer) ─┘
```

D87, D88, D89, D90 can run in parallel (different teams). D91 needs D87 (for the REST endpoint). D92 needs everything.

### 28.3 Resource allocation

- **Eng** : 3 swimlanes (REST gateway / webhook+firecracker / vsock-relayer) for 4 days.
- **SRE** : SLO catalogue + alertmanager rules + DR drill (D92).
- **Security** : pen test scenarios + review (D92).
- **DPO** : DPIA addendum review + AI Act Art. 26 endpoint review (D87 → D92).
- **QA** : bench infrastructure + 5 bench scenarios (D89 → D92).

---

## 29. Architecture decision records (ADRs)

### ADR-M26-001 : REST gateway over tonic-web for portal/CLI surface

**Status** : Accepted.
**Context** : portal-admin (Next.js) and efe-admin (Rust reqwest) both speak HTTPS+JSON. A pure tonic gRPC surface would force tonic-web in browser + fatter client crate.
**Decision** : axum REST in efe-server is the public surface ; tonic gRPC reserved for internal mesh.
**Consequences** : extra layer (REST → AgentRuntimeService Rust call) ; gain : simpler client deps, easier auditor inspection (curl-able), less binary size.

### ADR-M26-002 : Admission webhook in controller binary, not separate

**Status** : Accepted.
**Context** : Webhook needs DB connection + tenant flags + profile registry — same as controller.
**Decision** : Webhook server runs as a sidecar HTTPS endpoint inside the controller pod.
**Consequences** : Tighter coupling ; one fewer pod ; cert-manager + replicas: 2 + PDB compensate for HA.

### ADR-M26-003 : vsock-relayer in Go (not Rust)

**Status** : Accepted.
**Context** : The relayer is a thin proxy ; Go's stdlib HTTP + the `mdlayher/vsock` library are mature and battle-tested.
**Decision** : Ship as a Go binary in a separate container.
**Consequences** : One more language in the stack ; smaller binary than equivalent Rust ; reuses Go's mature net/http + tls stack.

### ADR-M26-004 : SSE for live tail, not WebSocket

**Status** : Accepted.
**Context** : LiveTail needs server-pushed events to the browser. WebSocket is more flexible but heavier ; SSE is simpler and works with reverse proxies (after `X-Accel-Buffering: no`).
**Decision** : SSE for forward-flow events ; HTTP polling fallback.
**Consequences** : One-way push only ; if browser needs to send commands, separate HTTP POST.

### ADR-M26-005 : Per-tenant K8s namespace, not shared

**Status** : Accepted.
**Context** : M25 used a single namespace `efe-agent-runtime` ; M26 isolates per tenant.
**Decision** : `efe-agent-runtime-<tenant_slug>` per tenant.
**Consequences** : Easier audit, easier DR ; controller needs cross-namespace RBAC ; namespace creation requires elevated cluster role.

### ADR-M26-006 : Sunset clause for STABLE = 3 tenants @ 24 months

**Status** : Accepted.
**Context** : EXPERIMENTAL had 2 @ 12 months ; STABLE needs more aggressive thresholds.
**Decision** : 3 tenants AND 10K runs/quarter at 18 months.
**Consequences** : Forces continuous tenant acquisition + adoption ; CSC reviews quarterly.

### ADR-M26-007 : Cosign + Rekor inclusion proof at boot

**Status** : Accepted.
**Context** : M25 had image whitelist by digest but no Rekor verification.
**Decision** : Controller verifies Rekor inclusion proof at boot for every entry in `EFE_M25_IMAGE_WHITELIST`.
**Consequences** : Boot delay ~2 s per image ; supply chain attack surface reduced.

### ADR-M26-008 : OpenTelemetry as observability backbone

**Status** : Accepted.
**Context** : Prometheus + structured logs cover metrics + logs ; we lack traces.
**Decision** : OTel SDK in Rust + browser ; export to Tempo backend.
**Consequences** : Trace cardinality must be bounded (sample at 1 % for runs > 30 min) ; storage cost.

### ADR-M26-009 : RAII guard for microVM lease, not explicit release in handler

**Status** : Accepted.
**Context** : Forgetting to release on the error path is the root cause of the M25 leak.
**Decision** : `MicrovmLease` with `Drop` impl — release is automatic unless `commit()` is called.
**Consequences** : Idiomatic Rust ; harder to forget ; reaper provides defense-in-depth.

### ADR-M26-010 : Audit pseudonymization for retention beyond 7y, not deletion

**Status** : Accepted.
**Context** : GDPR Art. 17 conflicts with regulatory retention ; pseudonymization is the standard reconciliation.
**Decision** : Pseudonymize after retention window ; chain stays valid.
**Consequences** : Pseudonymization function complexity ; chain validity after pseudonymization requires careful HMAC re-compute.

---

## 30. Risk register

### 30.1 Risk matrix

| ID | Risk | Likelihood | Impact | Status | Mitigation owner |
|---|---|:--:|:--:|---|---|
| R-1 | Webhook crash → all AgentRun CR creation blocked | LOW | HIGH | Mitigated | SRE (replicas: 2 + PDB + readiness probe) |
| R-2 | Firecracker REST API change between versions | LOW | HIGH | Tracked | Eng (pin Firecracker version + CI test against +1 minor) |
| R-3 | vsock-relayer DoS by runaway agent | MEDIUM | MEDIUM | Mitigated | Per-microVM rate limit + circuit breaker |
| R-4 | cert-manager fails to renew webhook cert | LOW | HIGH | Mitigated | Renewal alarm + 30-day buffer |
| R-5 | KMS key rotation breaks active checkpoints | MEDIUM | HIGH | Tracked | Dual-key window during rotation ; DR drill quarterly |
| R-6 | Pen test finds P0 unfixable in time | LOW | HIGH | Tracked | GA criterion : 0 P0 ; if found, block GA |
| R-7 | OpenTelemetry export pipeline saturates | LOW | MEDIUM | Mitigated | 1 % sampling for long runs ; storage cost budget |
| R-8 | Per-tenant namespace creation fails on cluster | LOW | MEDIUM | Mitigated | Pre-flight check + automatic creation by controller |
| R-9 | Tenant graduation criteria too strict ; nobody graduates | MEDIUM | LOW | Tracked | Quarterly CSC review ; relax if needed (with ADR amendment) |
| R-10 | M25 → M26 wire format incompatibility (despite intent) | LOW | HIGH | Mitigated | Wire diff doc + integration tests |

### 30.2 Owner contacts

- Eng : eng-lead@efe.example.com
- SRE : sre-oncall@efe.example.com
- Security : security@efe.example.com
- DPO : dpo@efe.example.com
- Product : product-lead@efe.example.com
- Legal : legal@efe.example.com

---

## 31. Open questions

| Q# | Question | Owner | Decision deadline |
|---|---|---|---|
| Q-1 | Should the SSE endpoint use Postgres LISTEN/NOTIFY or polling for the upstream feed? | Eng | D87 |
| Q-2 | What's the exact SLO for approval response P95 in the Pro tier? Is 1h achievable? | Product + SRE | D86 |
| Q-3 | Should we use OPA Gatekeeper as a secondary admission layer, or trust our webhook alone? | Security | D88 |
| Q-4 | Can we mandate per-tenant namespace isolation, or do some tenants need shared namespaces (multi-team within one tenant)? | Product | D90 |
| Q-5 | What's the right cadence for cosign key rotation — 12 months too long? Too short? | Security + SRE | D91 |
| Q-6 | Should the audit pseudonymization function be customizable per tenant? | DPO + Legal | D92 |
| Q-7 | Pricing model : Pro at $299/mo with 100 runs — is this competitive? | Product + Sales | D86 |
| Q-8 | Should we launch GA at all 3 cloud providers (AWS/GCP/Azure) or AWS-only first? | Product | D86 |
| Q-9 | Do we need a public bug bounty program for the agent runtime? | Security | post-GA |
| Q-10 | What's the rollback strategy if a critical SLO is breached for > 24h post-GA? | SRE + Product | D92 |

---

## 32. Success metrics

### 32.1 30-day post-GA

| Metric | Target |
|---|---|
| At least 3 tenants graduated to STABLE | 3+ |
| Total run volume since GA | ≥ 1,000 |
| Sandbox escape attempts | 0 |
| Audit chain HMAC failures | 0 |
| P0 incidents | 0 |
| P1 incidents | ≤ 2 (any) |
| Tenant satisfaction (NPS) | ≥ 30 |

### 32.2 90-day post-GA

| Metric | Target |
|---|---|
| Tenants graduated | 5+ |
| Run volume | ≥ 10,000 |
| Failure rate (cumulative) | < 2 % |
| Mean approval response (P95) | < 1 h |
| MTTR for P0 | < 30 min |
| MTTR for P1 | < 4 h |
| Cost per tenant per run (median) | ≤ $0.05 |

### 32.3 1-year post-GA

| Metric | Target |
|---|---|
| Tenants on STABLE | 10+ |
| Run volume | ≥ 100,000 |
| Pen test findings (cumulative) | 0 P0/P1 unresolved |
| External pen test : major findings | ≤ 2 P2/P3 |
| AI Act Art. 26 quarterly reports submitted on time | 4/4 |
| DPIA renewals on time | 100 % |

---

## 33. Documentation deliverables

| Document | Path | Owner |
|---|---|---|
| Architecture (this doc) | `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` | Eng |
| Wire format diff | `docs/EFE_M26_WIRE_DIFF.md` | Eng |
| Production runbook | `docs/runbooks/M26_AGENT_RUNTIME_PRODUCTION_RUNBOOK.md` | SRE |
| DR playbook | `docs/runbooks/M26_DR_PLAYBOOK.md` | SRE |
| DPIA addendum | `docs/release/DPIA_M26_ADDENDUM.md` | DPO |
| Tenant graduation agreement | `docs/release/M26_TENANT_GRADUATION_AGREEMENT.md` | Legal |
| Cluster graduation sign-off | `docs/release/M26_GRADUATION_SIGN_OFF.md` | Eng + 5 stakeholders |
| Pen test scope | `bench/m26_pentest/README.md` | Security |
| CSC charter | `docs/csc/CHARTER.md` | Product |
| CSC meeting notes | `docs/csc/2027-Q4.md`, etc. | Product |
| CHANGELOG entry | `CHANGELOG.md` (M26 section) | Eng |
| Tenant migration guide | `docs/migration/M25_TO_M26.md` | Eng + DPO |
| Browser SDK guide (SSE) | `docs/sdk/PORTAL_SSE.md` | Eng |
| API reference (REST gateway) | `docs/api/REST_AGENT_V1.md` | Eng |
| Bench results | `bench_baselines/v2.1.0/baseline.json` + `bench_baselines/v2.1.0/REPORT.md` | QA |

---

## 34. Rollback strategy

### 34.1 Cluster-wide rollback

If the M26 release causes a cluster-wide regression :

```bash
# 1. Helm rollback
helm rollback efe-agent-runner <previous-revision> -n efe-system

# 2. Revert SQL migrations 0017..0020 (in reverse order)
psql -f crates/efe-server/migrations/0020_m26_post_market_report_rollback.sql
psql -f crates/efe-server/migrations/0019_m26_pen_test_run_rollback.sql
psql -f crates/efe-server/migrations/0018_m26_graduation_rollback.sql
psql -f crates/efe-server/migrations/0017_agent_run_rename_template_id_rollback.sql

# 3. Revert efe-server image to previous
kubectl set image deployment/efe-server efe-server=ghcr.io/efe-ai/efe-server:<prev-tag> -n efe-system

# 4. Notify tenants
efe-admin announce send-all "M26 rollback in effect ; service may be intermittent for 30 min"
```

### 34.2 Per-tenant rollback

If a single tenant needs to revert from STABLE to EXPERIMENTAL :

```bash
efe-admin agent graduate revoke --tenant acme \
  --reason "tenant requested revert during incident XYZ" \
  --confirm
```

This sets `m26_graduation_revoked_at = NOW()`. New runs use EXPERIMENTAL rules ; in-flight runs continue with the rules they started with.

### 34.3 Migration window

The M25 → M26 migration window is 12 months. Tenants on EXPERIMENTAL who don't graduate by the end of the window face the M25 sunset clause. The CSC may extend the window for extenuating circumstances.

---

## 35. Contract evolution + SDK changes

### 35.1 Wire compatibility matrix

| Surface | M25 (v2.0) | M26 (v2.1) | Compatibility |
|---|---|---|---|
| `/v1/agent/*` REST | n/a | NEW | additive |
| `agent.proto` gRPC | exists | unchanged | identical |
| `AgentRun` CRD | v1 | v1 (one new optional field : `spec.minChannelRequired`) | additive |
| `AgentRunTemplate` CRD | v1 | v1 (additive) | additive |
| `agent_run` SQL | exists | rename `template_id` → `template_slug` (with view) | view preserves SELECT compat ; INSERT must use new name |
| `agent_audit` SQL | exists | unchanged | identical |
| `tenant_feature` SQL | M25 cols | + `m26_graduation_*` cols | additive |

### 35.2 SDK changes

| SDK | M26 changes |
|---|---|
| `efe-admin` (Rust CLI) | New subcommands : `agent graduate {check, confirm, revoke, status}` ; new flag `--channel STABLE\|EXPERIMENTAL` for `agent template upsert` |
| `portal-admin` (Next.js) | New page `/agent/graduate` ; updated `LiveTail` for SSE ; new component `<GraduationBadge>` |
| Python `agent-core` | No external API changes ; internal : new `EFE_RELAY_JWT_AUDIENCE` env var |
| `efe-mcp-server` Python tools | No new tools (4 tools retained) ; internal validators relaxed for EXPERIMENTAL channel |

### 35.3 Deprecations

| Item | Deprecated in M26 | Removed in |
|---|---|---|
| `AgentRun.spec.image` field | YES (use template's image instead) | M27 |
| `efe-admin agent template upsert --legacy` flag | YES | M27 |
| Pre-cosign-Rekor signature verification | YES | M27 |

---

## Annex A — REST gateway endpoint catalogue

Full list of 23 endpoints under `/v1/agent/*` :

| # | Method | Path | Purpose | Body | Response |
|---|---|---|---|---|---|
| 1 | POST | `/v1/agent/runs` | Create new run | `{tenant_id, template_slug, initial_prompt, tool_whitelist?, timeout_seconds?, max_cost_usd?}` | `{run: AgentRun}` |
| 2 | GET | `/v1/agent/runs` | List runs by tenant | query : `tenant`, `include_completed`, `limit` | `[AgentRun]` |
| 3 | GET | `/v1/agent/runs/:id` | Get run detail | path : `id` | `{run, recent_steps, pending_approvals}` |
| 4 | POST | `/v1/agent/runs/:id/pause` | Pause run | `{reason?}` | `{success}` |
| 5 | POST | `/v1/agent/runs/:id/resume` | Resume paused run | `{}` | `{success}` |
| 6 | POST | `/v1/agent/runs/:id/terminate` | Force-stop run | `{reason}` | `{success}` |
| 7 | GET | `/v1/agent/runs/:id/steps` | List steps | query : `since`, `limit` | `[AgentStep]` |
| 8 | GET | `/v1/agent/runs/:id/steps/stream` | SSE live tail | n/a | `text/event-stream` |
| 9 | POST | `/v1/agent/approvals/approve` | Approve step | `{approval_id, reason}` | `{approval_token, expires_at}` |
| 10 | POST | `/v1/agent/approvals/reject` | Reject step | `{approval_id, reason}` | `{success}` |
| 11 | GET | `/v1/agent/approvals/pending` | List pending approvals | query : `tenant` | `[ApprovalRequest]` |
| 12 | POST | `/v1/agent/templates` | Upsert template | `AgentRunTemplate` | `AgentRunTemplate` |
| 13 | GET | `/v1/agent/templates` | List templates | query : `tenant` | `[AgentRunTemplate]` |
| 14 | GET | `/v1/agent/templates/:tenant/:slug` | Get one template | path : `tenant`, `slug` | `AgentRunTemplate` |
| 15 | DELETE | `/v1/agent/templates/:tenant/:slug` | Soft-delete (deprecate) | `{}` | `{success}` |
| 16 | GET | `/v1/agent/cost/daily` | Daily cost report | query : `tenant`, `days` | `[CostDailyRow]` |
| 17 | GET | `/v1/agent/cost/run/:id` | Per-run cost breakdown | path : `id` | `CostPerRun` |
| 18 | POST | `/v1/agent/dpia/sign` | Sign DPIA | `{tenant_id, addendum_hash, valid_days}` | `{success}` |
| 19 | GET | `/v1/agent/dpia/status` | DPIA status | query : `tenant` | `DpiaStatus` |
| 20 | GET | `/v1/agent/dpia/list` | List all tenants DPIA | n/a (admin only) | `[DpiaListRow]` |
| 21 | POST | `/v1/agent/dpia/refresh` | Refresh DPIA | `{tenant_id, valid_days}` | `{success}` |
| 22 | POST | `/v1/agent/graduate/check` | Check graduation criteria | `{tenant_id}` | `{operational: {...}, compliance: {...}, ready: bool}` |
| 23 | POST | `/v1/agent/graduate/confirm` | Confirm graduation | `{tenant_id, acknowledgement_hash}` | `{success, graduated_at}` |
| (24) | POST | `/v1/agent/graduate/revoke` | Revoke graduation | `{tenant_id, reason}` | `{success, revoked_at}` |
| (25) | GET | `/v1/agent/post-market/quarterly` | AI Act Art. 26 export | query : `quarter` | JSON-LD report |

(23 baseline + 2 graduation + 1 post-market = 25 total ; the section says "23" for the user-facing core ; admin endpoints don't count toward the SLO numerics).

---

## Annex B — Firecracker REST API call sequence

Annotated sequence diagram :

```
                                  Firecracker UDS
Controller                         (per-vm socket)
   │                                   │
   │ ──── PUT /boot-source ─────────►  │  ← step 1 (kernel + boot args)
   │ ◄────── 204 No Content ───────    │
   │                                   │
   │ ──── PUT /drives/rootfs ──────►   │  ← step 2 (rootfs ext4 ro)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ──── PUT /machine-config ────►    │  ← step 3 (vCPU + memory)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ── PUT /network-interfaces/eth0 ─►│  ← step 4 (TAP iface)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ──── PUT /vsock ──────────────►   │  ← step 5 (vsock CID)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ─── PUT /actions InstanceStart ─► │  ← step 6 (kick boot)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ··· (microVM boots, runs agent) ··│
   │                                   │
   │ ── PUT /actions SendCtrlAltDel ─► │  ← step 7 (graceful shutdown)
   │ ◄────── 204 ──────────────────    │
   │                                   │
   │ ── (10s grace period) ───────     │
   │                                   │
   │ ── PUT /actions InstanceHalt ──►  │  ← step 8 (kill if still alive)
   │ ◄────── 204 ──────────────────    │
   │                                   │
```

Errors at any step trigger the cleanup path (§ 9.3).

---

## Annex C — vsock-relayer wire protocol

```
agent-core (microVM)             vsock-relayer (host)
   │                                   │
   │  vsock CID 2 port 9999            │
   │ ────► HTTP/1.1 ────────────►      │
   │                                   │ JWT validation
   │                                   │ rate-limit check
   │                                   │ upstream lookup
   │                                   │
   │                                   │ ────► mTLS upstream call
   │                                   │       (LLM router or mcp-server)
   │                                   │ ◄────── upstream response
   │                                   │
   │ ◄────── HTTP response ─────       │
   │                                   │
   │                                   │ async : POST /v1/audit/append
   │                                   │       (audit chain entry)
   │                                   ▼
```

JWT claims required :
```json
{
  "iss": "efe-agent-runner-controller",
  "aud": "efe-agent-relay",
  "sub": "agent-run:<run_id>",
  "tenant": "<slug>",
  "run_id": "<uuid>",
  "iat": 1234567890,
  "exp": 1234571490,
  "tools_allowed": ["web_search", "fetch_url", "..."]
}
```

The relayer rejects requests where the requested tool is NOT in `tools_allowed`.

---

## Annex D — SLO catalogue (full table)

| # | SLO name | Indicator | Objective | Window | Burn alert |
|---|---|---|---|---|---|
| 1 | Sandbox escape MUST be 0 | `efe_agent_sandbox_escape_attempts_total` | = 0 | 1 h | P0 immediate |
| 2 | Audit chain HMAC verify failures MUST be 0 | `efe_agent_audit_hmac_verify_failures_total` | = 0 | 5 min | P0 immediate |
| 3 | Image whitelist rejections MUST be < 5 / day | `efe_agent_image_rejected_whitelist_total` | < 5 / d | 1 d | P2 (likely an attack or misconfiguration) |
| 4 | Provisioning P99 latency | `efe_agent_microvm_provision_duration_seconds` | < 5 s P99 | 5 min | P1 fast burn 10 min |
| 5 | Run failure rate | `efe_agent_runs_total{status="FAILED"}` / total | < 2 % | 1 h | P2 fast burn 1 h |
| 6 | Run completion rate | `efe_agent_runs_total{status="COMPLETED"}` / total | > 95 % | 1 d | P3 slow burn |
| 7 | Approval response P95 | `efe_agent_approval_response_latency_seconds` | < 1 h P95 | 1 h | P2 fast burn |
| 8 | Approval expired ratio | `efe_agent_approval_expired_total` / total | < 5 % | 1 d | P3 ticket |
| 9 | Cost cap response time | from cap-exceeded to terminate | < 60 s P99 | 1 d | P1 fast burn |
| 10 | API availability (REST gateway) | uptime | > 99.5 % | 30 d | P1 fast burn |
| 11 | API availability (admission webhook) | uptime | > 99.9 % | 30 d | P0 fast burn |
| 12 | DB connection pool saturation | active / max | < 80 % | 5 min | P2 fast burn |
| 13 | OpenTelemetry trace export rate | exported / total | > 95 % (sampled) | 1 h | P3 |
| 14 | GDPR Art. 17 cascade SLA | request → completion | < 30 d P99 | 30 d | P1 (regulatory) |

---

## Annex E — Pen test scope + threat scenarios

Full 30-scenario list :

```
01. ptrace another process from inside microVM
02. seccomp policy enumeration via prctl(PR_GET_SECCOMP)
03. DNS exfil via UDP punching to non-whitelisted port
04. image swap mid-run via OCI manifest tampering
05. filesystem breakout via /proc/self/root
06. mount syscall to gain access to host /var
07. tool whitelist bypass : LLM hallucinates "_efe_internal_*" prefix
08. tool whitelist bypass : agent-core layer 2 race (call before whitelist load)
09. tool whitelist bypass : mcp-server layer 3 race (call after layer 2 cache invalidation)
10. tool whitelist bypass : vsock-relayer JWT forgery
11. approval token replay : same token, two parallel calls
12. approval token forgery : known-key attack on HMAC
13. approval token expiry bypass : clock-skew exploitation
14. NetworkPolicy bypass : egress to non-whitelisted host via DNS canonical name resolution
15. NetworkPolicy bypass : metadata endpoint (169.254.169.254)
16. kube-dns DNS poisoning
17. audit chain row delete (DBA attack)
18. audit chain HMAC re-compute (insider attack)
19. audit chain reorder (ID forgery)
20. cost cap bypass : delayed cost reporting from agent-core
21. cost cap bypass : race between cap check and run continuation
22. cost cap bypass : batch evasion (single LLM call returning 10K tokens)
23. RBAC : forge JWT with cross-tenant claim
24. RBAC : role escalation via OIDC group manipulation
25. RBAC : Postgres RLS bypass via direct connection
26. DPIA bypass : SQL UPDATE m25_dpia_signed_at directly
27. DPIA bypass : graduation acknowledgement forgery
28. webhook bypass : direct K8s API CR creation (admission disabled)
29. cosign signature forgery : known-key attack
30. Rekor entry tampering : pre-hashed digest stuffing
```

---

## Annex F — DPIA addendum diff (M25 → M26)

Section-by-section diff highlighting changes :

- Section 1 (Identification) : add field `Channel` (EXPERIMENTAL or STABLE) + `Graduation acknowledged at`.
- Section 2 (Description) : add a sentence on AI Act Art. 26 quarterly post-market reporting.
- Section 3 (Risk assessment) : add row "REST gateway forgery" + "Webhook bypass" + "vsock-relayer exploitation" to the table.
- Section 4 (Lawful basis) : unchanged.
- Section 5 (Data subject rights) : add Art. 17 cascade pseudonymization mechanism description.
- Section 6 (DPO sign-off) : split into two : "DPO sign-off for EXPERIMENTAL" + "DPO sign-off for STABLE graduation".
- Section 7 (Attachments) : add "Tenant graduation acknowledgement signed PDF".
- Section 8 (Annual re-review) : add post-market report review checklist.

---

## Annex G — Operator readiness checklist

Per-tenant pre-graduation checklist :

- [ ] DPIA signed within last 11 months
- [ ] Operator team has completed M26 training (online module, ~2 h)
- [ ] At least 2 named operators with `agent_run.create` role
- [ ] At least 1 named operator with `agent_run.terminate` role + DPO contact info
- [ ] Approval response SLA agreed (1 h Pro / 30 min Enterprise)
- [ ] Backup operator on-call rotation defined
- [ ] Slack #efe-agent-runtime-alerts channel access for ops team
- [ ] Production graduation acknowledgement PDF signed by tenant security lead

---

## Annex H — Kernel hardening profile

The vmlinux image used for microVMs is built with :

```
CONFIG_SECCOMP=y
CONFIG_SECCOMP_FILTER=y
CONFIG_AUDIT=y
CONFIG_AUDITSYSCALL=y
CONFIG_SECURITY=y
CONFIG_SECURITY_NETWORK=y
CONFIG_SECURITY_PATH=y
CONFIG_HARDENED_USERCOPY=y
CONFIG_FORTIFY_SOURCE=y
CONFIG_STACKPROTECTOR_STRONG=y
CONFIG_RANDOM_TRUST_CPU=y
CONFIG_SLAB_FREELIST_HARDENED=y
CONFIG_SLAB_FREELIST_RANDOM=y
CONFIG_SHUFFLE_PAGE_ALLOCATOR=y
CONFIG_INIT_STACK_ALL_ZERO=y
CONFIG_INIT_ON_ALLOC_DEFAULT_ON=y
CONFIG_INIT_ON_FREE_DEFAULT_ON=y
CONFIG_PANIC_ON_OOPS=y
CONFIG_DEBUG_NOTIFIERS=y
# ... (full list ~80 options ; see kernel build config)
```

These options are enabled at compile time for the vmlinux image distributed in the agent-core OCI image.

---

## Annex I — Sigstore policy

The cluster's cosign verification policy :

```yaml
apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: efe-agent-runner-image-policy
spec:
  images:
    - glob: "ghcr.io/efe-ai/agent-core**"
    - glob: "ghcr.io/efe-ai/efe-agent-runner**"
  authorities:
    - name: efe-cosign-key
      key:
        kms: gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign
      ctlog:
        url: https://rekor.sigstore.dev
    # (Optional secondary key for old digests still in active whitelist)
    - name: efe-cosign-key-v2
      key:
        kms: gcpkms://projects/efe-ai/locations/global/keyRings/release/cryptoKeys/cosign-v2
      ctlog:
        url: https://rekor.sigstore.dev
```

---

## Annex J — Glossary

| Term | Definition |
|---|---|
| **CSC** | Caliper Steering Committee — monthly cross-functional review of M26 |
| **EXPERIMENTAL channel** | M25 launch channel ; relaxed validation, sunset clause active |
| **STABLE channel** | M26 graduation channel ; strict validation, full SLAs |
| **Graduation criteria** | The 6 operational + 4 compliance + 6 sign-off criteria for STABLE move |
| **vsock-relayer** | Go binary DaemonSet bridging microVM HTTP calls to in-cluster services |
| **Reaper** | Background goroutine that detects + cleans orphan microVM slots |
| **MicrovmLease** | RAII guard around `MicrovmSlot` ensuring release on error paths |
| **Rekor** | Sigstore's transparency log for cosign signatures |
| **AdmissionReview** | K8s API for validating webhooks (request + response wrapper) |
| **SLO burn rate** | Rate at which an error budget is consumed |
| **DPIA addendum** | Per-tenant signed document describing M25/M26 data processing |
| **Post-market monitoring** | AI Act Art. 26 obligation : quarterly AI system performance report |
| **GDPR Art. 17 cascade** | Right-to-erasure mechanism that removes data + pseudonymizes audit |
| **Pseudonymization** | Replacing personal data with reversible-by-controller identifiers |
| **Pen test bundle** | The 30-scenario adversarial test suite shipping with M26 |
| **DR drill** | Disaster recovery game day exercise |
| **HMAC chain** | Per-tenant audit log integrity mechanism (parent-linked, key-rotating) |

---

## Document metadata

| Field | Value |
|---|---|
| Version | 1.0 |
| Status | DRAFT for D86 review |
| Owner | Eng lead |
| Reviewers | SRE, Security, DPO, Product, Legal |
| Sign-off deadline | D86 EOD |
| Predecessor | EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md |
| Successor | (M27 if business case emerges for multi-region) |
| Related docs | EFE_M26_WIRE_DIFF, M26 runbook, M26 DR playbook, DPIA addendum, tenant graduation agreement |

---

## End of document

This architecture document is the canonical reference for the M26 production graduation effort. All implementation PRs must cite the relevant section number in their description ; deviations from the design must be justified via an ADR amendment with sign-off from at least 3 of 6 stakeholders.
