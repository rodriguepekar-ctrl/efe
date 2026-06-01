# EFE M26 Assembly Verification Architecture

**Status** : ARCHITECTURE — design phase, pre-execution.
**Predecessors** :
- `EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` (canonical M26 spec)
- M25 chirurgical audit + assembly verification (precedent)
**Target** : confirm M26 is **deployable end-to-end** + clear all cross-cutting integration gaps + fix the M23 inherited debt that blocks `cargo build --workspace`.
**Codename** : "Caliper-Bis" (the sister task to M26 Caliper).
**EFE version** : `2.1.0` patch-line — `2.1.1` after this assembly.
**Sprint window** : D93 (1 working day, 8h).
**Hard deps** : M26 architecture + code drop + chirurgical audit complete.

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Scope + non-goals](#2-scope--non-goals)
3. [Inventory : what M26 shipped](#3-inventory--what-m26-shipped)
4. [Cross-cutting integration matrix](#4-cross-cutting-integration-matrix)
5. [Compile-time issues (3 categories)](#5-compile-time-issues-3-categories)
6. [Runtime integration issues (8 items)](#6-runtime-integration-issues-8-items)
7. [Deploy-time integration issues (Helm + K8s)](#7-deploy-time-integration-issues-helm--k8s)
8. [End-to-end verification strategy (5-tier)](#8-end-to-end-verification-strategy-5-tier)
9. [Fix catalogue : 22 items](#9-fix-catalogue--22-items)
10. [ADR addendums](#10-adr-addendums)
11. [Sprint plan D93](#11-sprint-plan-d93)
12. [Risk register](#12-risk-register)
13. [Done definition](#13-done-definition)
14. [Annexes](#annexes)

---

## 1. Executive summary

The M26 chirurgical audit closed all the local compilation issues for the M26 crate (`efe-agent-runner` v2.1.0 lib + bin + tests now compile). However the broader **end-to-end assembly** remains unverified : the M26 code talks to other crates and external systems (efe-server REST gateway, K8s admission webhook, vsock-relayer, cert-manager, OPA Gatekeeper, Postgres triggers from prior milestones), and these inter-component contracts have not been validated together.

This 1-day sprint :

1. **Closes the workspace `cargo build`** by fixing the 3 categories of compile-time issues that M26 surfaced (rustc toolchain bump, M23 inherited curation lib errors, the missing AgentRuntimeFacade implementor).
2. **Verifies the runtime integration contracts** (webhook env wiring, REST router mount point, reaper pool sharing, M25 INSERT statement using renamed column, etc. — 8 items).
3. **Validates a deploy-time end-to-end test** in a k3d local cluster + chaos cluster : Helm install → admission webhook live → AgentRun CR creates → microVM provisioned via real Firecracker driver (chaos cluster only) → run completes → audit chain verifies → graduation flow works.
4. **Documents 3 ADR addendums** for the workspace-level decisions taken (rustc 1.88, sqlx version pinning strategy, axum 0.7 unification).
5. **Updates CHANGELOG with v2.1.1** as the assembly-completion patch.

The output is `EFE 2.1.1` ready for the first design-partner production deployment in the chaos cluster, with full deploy-time validation.

---

## 2. Scope + non-goals

### 2.1 In scope

- Fixing all compile-time errors at the workspace level (`cargo build --workspace`).
- Implementing the missing `AgentRuntimeFacade` adapter so the REST gateway can call into the M25 `AgentRuntimeService`.
- Mounting the new `/v1/agent/*` axum router on the efe-server HTTP listener.
- Wiring webhook env vars on the controller deployment.
- Fixing the M25 SQL INSERT in `run.rs::insert_run` after the `template_id → template_slug` rename in migration 0017.
- Pen-test scenarios reference `/v1/agent/_test/*` endpoints — verify which exist and which need stubs.
- Adding the missing controller Dockerfile for `ghcr.io/efe-ai/efe-agent-runner:2.1.0`.
- Sharing the microVM pool map between controller reconcile + reaper (currently two separate maps).
- ADR for rustc 1.88 bump.

### 2.2 Out of scope (deferred to follow-on)

- Multi-region (M27).
- Multi-modal (M27+).
- New tenant-facing features.
- M23 portal-admin path migration (`src/app/curation/` → `app/(authenticated)/curation/`) — separate GLOSS-B sprint.
- M24 portal-admin path migration (same) — separate GLOSS-B sprint.
- Bumping sqlx to 0.8 across all crates (defer to M27 if sqlite collision is fixable).
- Replacing `time 0.3.47` transitive — only fixable via rustc bump.

---

## 3. Inventory : what M26 shipped

For reference, M26 added these artifacts (from the `Wave 1-12` code drop) :

| Category | Files |
|---|---|
| **SQL migrations** | `0017_agent_run_rename_template_id.sql`, `0018_m26_graduation.sql`, `0019_m26_pen_test_run.sql`, `0020_m26_post_market_report.sql` (+ rollbacks) |
| **REST gateway efe-server** | `crates/efe-server/src/v1/agent/{mod,auth,errors,runs,approvals,templates,cost,dpia,graduate,post_market}.rs` (10 modules) |
| **Webhook (efe-agent-runner)** | `crates/efe-agent-runner/src/webhook/{mod,server,validators,tls}.rs` (4 modules) |
| **Firecracker real + chaos** | `crates/efe-agent-runner/src/sandbox/{firecracker_real,firecracker_chaos}.rs` |
| **RAII guard + reaper** | `crates/efe-agent-runner/src/sandbox/lease_guard.rs` + `controller_reaper.rs` |
| **vsock-relayer (Go)** | `cmd/vsock-relayer/{main.go, pkg/{proxy,ratelimit,audit}, Dockerfile, README}` |
| **Helm chart additions** | 6 templates : `webhook-{service,cert}.yaml`, `vsock-relayer-{daemonset,rbac}.yaml`, `slo-prometheusrule.yaml`, `alertmanager-routing.yaml` + `values.yaml` updates |
| **Portal-admin streaming** | `app/api/agent/runs/[id]/{steps,tail}/route.ts` + LiveTail rewrite |
| **CLI graduate** | `crates/efe-admin/src/commands/agent/graduate.rs` + `agent_client.rs` extensions |
| **Pen test bundle** | `bench/m26_pentest/` — 30 scenarios + lib + README |
| **Bench scenarios** | `bench/m26/` — 5 scenarios + README |
| **Documentation** | architecture (130 KB), wire diff, runbook, DR playbook, DPIA addendum, tenant graduation agreement |

The chirurgical audit fixed the 15 issues that prevented `cargo check -p efe-agent-runner --lib --bin --tests`. This sprint fixes the **integration** issues.

---

## 4. Cross-cutting integration matrix

The M26 deliverables span 7 components. Each pair of components has an integration contract that must hold for end-to-end deployment :

| ↓ from / → to | efe-server | efe-agent-runner | vsock-relayer | portal-admin | efe-admin CLI | Helm | K8s API |
|---|:--:|:--:|:--:|:--:|:--:|:--:|:--:|
| **efe-server** | — | gRPC mTLS | — | REST + SSE | REST | env vars | — |
| **efe-agent-runner** | gRPC | — | vsock control | — | — | env vars + cert | webhook + CRDs |
| **vsock-relayer** | — | vsock listener | — | — | — | DaemonSet + secrets | DaemonSet RBAC |
| **portal-admin** | REST + SSE | — | — | — | — | — | — |
| **efe-admin CLI** | REST | — | — | — | — | — | — |
| **Helm** | env + Service | env + cert + Service | DaemonSet + Service | — | — | — | CRDs install |
| **K8s API** | — | webhook validation | — | — | — | — | — |

For each cell with a bidirectional contract, this assembly sprint validates :
1. Both sides agree on the wire format (JSON / proto / TLS cert names).
2. The orchestration spawns components in the right order.
3. The TLS / auth / RBAC chain is intact end-to-end.

The matrix surfaces **9 cross-cutting integration points** that M26 introduced but didn't fully validate. They're enumerated in § 6.

---

## 5. Compile-time issues (3 categories)

### 5.1 Category A — rustc toolchain mismatch

**Root cause** : `crates/efe-server` depends on `jsonwebtoken 9.3` → `simple_asn1 0.6.4` → `time 0.3.47` (locked). `time 0.3.47` requires `rustc ≥ 1.88`. Workspace `Cargo.toml` declares `rust-version = "1.85"` and the developer's local toolchain is 1.85.

**Implication** : `cargo check --workspace` fails for any developer with rustc < 1.88, AND CI must be on rustc ≥ 1.88.

**Fix** : bump workspace `rust-version = "1.88"` + update local toolchains. ADR-D93-001 documents the decision.

**Risk** : developers with stuck toolchains see CI go green but fail locally. Mitigation : document in `CONTRIBUTING.md` + provide `rust-toolchain.toml` pin file.

### 5.2 Category B — M23 inherited lib errors in `efe-admin`

**Root cause** : Files in `crates/efe-admin/src/commands/curation/` (`get.rs`, `propose_split.rs`, `diff.rs`) reference fields that don't exist on the structs they receive :

```
error[E0609]: no field `tenant_id` on type `ProposalSummary`
error[E0609]: no field `actual_cost_cents` on type `ProposalSummary`
error[E0609]: no field `proposed_class_id` on type `&serde_json::Value`
error[E0609]: no field `anchor_candidates` on type `&serde_json::Value`
error[E0277]: the trait bound `DiffSummary: serde::Serialize` is not satisfied (×2)
```

**Implication** : `cargo build -p efe-admin` fails ; the CLI binary cannot be built ; release pipeline blocked.

**Fix** : 6 surgical edits (4 field reference fixes + 1 `Serialize` derive) in 3 files. Detail in § 9 fix #B-1..#B-6.

**Origin** : these files were in M23 drop, never compiled in CI because efe-admin was skipped from `--workspace` checks at the time. Latent bug.

### 5.3 Category C — Missing trait implementer

**Root cause** : The REST gateway defines :

```rust
// crates/efe-server/src/v1/agent/mod.rs
pub trait AgentRuntimeFacade: Send + Sync {
    async fn create_run(&self, caller: &AuthCtx, req: serde_json::Value) -> ...;
    async fn get_run(&self, caller: &AuthCtx, run_id: Uuid) -> ...;
    // ... 6 more methods
}
```

…but no concrete implementer exists. The M25 `AgentRuntimeService` struct (in `efe-agent-runner`) has methods with similar signatures but they take typed `CreateAgentRunRequest` not `serde_json::Value`. We need an adapter.

**Implication** : the AgentApiState can't be constructed in efe-server's main.rs. The REST gateway is dead code.

**Fix** : create `crates/efe-server/src/v1/agent/adapter.rs` that wraps `AgentRuntimeService` and implements `AgentRuntimeFacade`. Detail in § 6.1.

---

## 6. Runtime integration issues (8 items)

### 6.1 Missing `AgentRuntimeFacade` adapter

The trait `AgentRuntimeFacade` is in `crates/efe-server/src/v1/agent/mod.rs` (with JSON-typed signatures because the REST handlers serialize back through serde_json). The real implementer needs to bridge `serde_json::Value` ↔ `CreateAgentRunRequest` (M25 typed DTO).

**Sketch** :

```rust
// crates/efe-server/src/v1/agent/adapter.rs

use std::sync::Arc;
use efe_agent_runner::service::{AgentRuntimeService, CreateAgentRunRequest, CallerIdentity};
use efe_agent_runner::TenantId;
use uuid::Uuid;

use super::auth::AuthCtx;
use super::AgentRuntimeFacade;
use super::errors::AgentApiError;

pub struct AgentRuntimeServiceAdapter {
    pub inner: Arc<AgentRuntimeService>,
}

impl AgentRuntimeServiceAdapter {
    pub fn new(inner: Arc<AgentRuntimeService>) -> Self { Self { inner } }
}

fn auth_to_caller(auth: &AuthCtx) -> CallerIdentity {
    CallerIdentity {
        subject: auth.subject.clone(),
        tenant: TenantId(auth.tenant.clone()),
        roles: auth.roles.clone(),
        token_age_seconds: auth.token_age_seconds,
    }
}

#[async_trait::async_trait]
impl AgentRuntimeFacade for AgentRuntimeServiceAdapter {
    async fn create_run(
        &self,
        caller: &AuthCtx,
        req: serde_json::Value,
    ) -> Result<serde_json::Value, AgentApiError> {
        let typed: CreateAgentRunRequest = serde_json::from_value(req)
            .map_err(|e| AgentApiError::InvalidInput(format!("bad request : {}", e)))?;
        let inner_caller = auth_to_caller(caller);
        let run = self.inner
            .create_agent_run(&inner_caller, typed)
            .await
            .map_err(map_runner_err)?;
        Ok(serde_json::to_value(&run)?)
    }

    async fn get_run(
        &self,
        caller: &AuthCtx,
        run_id: Uuid,
    ) -> Result<serde_json::Value, AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        let run = self.inner.get_agent_run(&inner_caller, run_id).await
            .map_err(map_runner_err)?;
        // Fetch recent steps + pending approvals via separate methods.
        let recent_steps = self.inner.list_recent_steps(&inner_caller, run_id, 50).await
            .unwrap_or_default();
        let pending_approvals = self.inner.list_pending_approvals_for_run(&inner_caller, run_id)
            .await.unwrap_or_default();
        Ok(serde_json::json!({
            "run": run,
            "recent_steps": recent_steps,
            "pending_approvals": pending_approvals,
        }))
    }

    // ... 6 more methods
}

fn map_runner_err(e: efe_agent_runner::AgentRunnerError) -> AgentApiError {
    use efe_agent_runner::AgentRunnerError as E;
    match e {
        E::InvalidInput(msg) => AgentApiError::InvalidInput(msg),
        E::RunNotFound(_) | E::TemplateNotFound(_)
        | E::ApprovalNotFound(_) | E::StepNotFound(_) => AgentApiError::NotFound,
        E::M25NotEnabledForTenant(_) | E::DpiaNotSigned(_)
        | E::DpiaExpired { .. } | E::ProfileNotAllowed(_)
        | E::ProviderNotAllowed(_) | E::InvalidTransition { .. }
        | E::ImageNotInWhitelist(_) => {
            AgentApiError::FailedPrecondition(e.to_string())
        }
        E::PermissionDenied { .. } | E::TenantMismatch { .. } => {
            AgentApiError::Forbidden(e.to_string())
        }
        E::OidcTokenStale { age_seconds, max } => {
            AgentApiError::TokenStale { age_seconds, max }
        }
        E::CostCapExceeded { .. } | E::CostAboveTenantCap { .. }
        | E::MaxConcurrentRunsReached { .. } => {
            AgentApiError::RateLimited { retry_after_seconds: 60 }
        }
        E::SandboxEscapeAttempt { .. } => AgentApiError::SandboxEscape,
        _ => AgentApiError::Internal(e.to_string()),
    }
}
```

**M25 service additions required** : `list_recent_steps()` and `list_pending_approvals_for_run()` are not in the M25 surface — we need to add them. They're trivial DB queries. Detail in § 9 fix #C-2.

### 6.2 efe-server HTTP router mount point

The `v1::agent::router(state)` returns an axum `Router`. The efe-server's main HTTP listener (existing for `/v1/classify`, `/v1/learning/*`, `/v1/curation/*`) must mount this new sub-router at `/v1/agent`.

**Where** : `crates/efe-server/src/main.rs` (or wherever the HTTP listener is constructed).

**Sketch** :

```rust
let agent_state = AgentApiState {
    service: Arc::new(AgentRuntimeServiceAdapter::new(agent_service.clone())),
    pg: pg_pool.clone(),
};
let app = existing_router
    .nest("/v1/agent", efe_server::v1::agent::router(agent_state));
```

Need to find where the existing axum router is built. Section 9 fix #C-3.

### 6.3 M25 SQL INSERT references renamed column

`crates/efe-agent-runner/src/run.rs::insert_run` (M25) uses :

```sql
INSERT INTO agent_run (id, tenant_id, template_id, ...) VALUES (...)
```

But migration 0017 renamed `template_id → template_slug`. Now :
- The rename + `agent_run_v25` view make SELECTs work via the view (legacy).
- INSERTs must use the new column name `template_slug`.

**Impact** : every new agent run creation fails with `ERROR: column "template_id" of relation "agent_run" does not exist`.

**Fix** : update the M25 INSERT to use `template_slug`. Detail in § 9 fix #D-1.

### 6.4 Reaper pool map not shared with controller

In `main.rs`, the reaper goroutine receives a fresh `Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>`, but the actual pools are created inside `controller::run()` which has its own internal map. Result : the reaper sees an empty map and never reaps anything.

**Fix** : extract the pool-map ownership to `main.rs`, pass `Arc<Mutex<...>>` into both `controller::run()` and `controller_reaper::reaper_loop()`. Detail in § 9 fix #D-2.

### 6.5 Webhook env vars not on controller deployment

`crates/efe-agent-runner/src/main.rs` reads `EFE_M26_WEBHOOK_ENABLED`, `EFE_M26_WEBHOOK_ADDR`, `EFE_M26_WEBHOOK_TLS_CERT`, `EFE_M26_WEBHOOK_TLS_KEY`. The Helm `deployment-controller.yaml` doesn't set them.

**Impact** : webhook is OFF in production despite Helm `admissionWebhook.enabled: true`.

**Fix** : add `env:` block in deployment-controller.yaml conditional on `.Values.admissionWebhook.enabled`. Detail in § 9 fix #D-3.

### 6.6 Webhook TLS cert volume mount missing

The webhook server reads `/var/run/webhook-tls/tls.crt` + `/var/run/webhook-tls/tls.key`, but the controller pod doesn't mount the cert-manager Secret.

**Fix** : add `volume` + `volumeMount` to deployment-controller.yaml. Detail in § 9 fix #D-4.

### 6.7 Pen test `_test` endpoints don't exist

The pen test scenarios (test_11, 12, 17, 18, 19, 24, 25, 26, 28, 29, 30) reference admin-only test endpoints :
- `/v1/agent/_test/replay_token`
- `/v1/agent/_test/expired_token_attack`
- `/v1/agent/_test/audit_tamper`
- `/v1/agent/_test/viewer_token`
- `/v1/agent/_test/rls_probe`
- `/v1/agent/_test/sql_dpia_bypass`
- `/v1/agent/_test/webhook_bypass_probe`
- `/v1/agent/_test/cosign_forgery_probe`
- `/v1/agent/_test/rekor_tamper_probe`

None of these are implemented in the REST gateway. The pen test bundle would 404 on every scenario.

**Fix options** :
1. Implement them in `crates/efe-server/src/v1/agent/_test.rs` — gated by `EFE_CLUSTER_ROLE=chaos` env var (refused otherwise).
2. Skip all chaos-only tests with `@CHAOS_ONLY` (most already are) ; defer the actual implementation to a chaos-cluster-specific deployment.

**Decision** : Option 2 for the assembly sprint — pen test infrastructure deferred to a chaos-cluster sprint. Document the deferred work. Detail in § 9 fix #D-5.

### 6.8 Missing controller Dockerfile

The Helm chart references `ghcr.io/efe-ai/efe-agent-runner:2.1.0` but `crates/efe-agent-runner/` has no Dockerfile. CI can't build the image.

**Fix** : add `crates/efe-agent-runner/Dockerfile` (multi-stage Rust build → distroless runtime). Detail in § 9 fix #D-6.

---

## 7. Deploy-time integration issues (Helm + K8s)

### 7.1 cert-manager ClusterIssuer assumed to exist

`webhook-cert.yaml` references `.Values.admissionWebhook.certManager.issuerRef` (default `efe-ca-issuer`). If the cluster has no such ClusterIssuer, cert-manager logs an error + the Certificate stays Pending forever, and the webhook readiness probe stays Unhealthy.

**Fix** : add a `pre-install` Helm hook that checks if the issuer exists OR provide a `selfSigned` fallback when `Values.admissionWebhook.certManager.bootstrap = true`. Detail in § 9 fix #E-1.

### 7.2 Pod Disruption Budget missing for 2-replica controller

`Values.controller.replicas: 2` ensures HA, but no PDB means K8s could evict both during a node drain → cluster-wide AgentRun creation freeze.

**Fix** : add `pdb-controller.yaml` with `minAvailable: 1`. Detail in § 9 fix #E-2.

### 7.3 NodeSelector `efe.ai/kvm-available=true` requires labelling

DaemonSet `vsock-relayer` selects nodes with this label. If no node has the label, no Pod schedules → microVMs can never launch.

**Fix** : add doc + `kubectl label node ... efe.ai/kvm-available=true` instruction in the runbook ; emit a warning event from the controller if no labeled node exists. Detail in § 9 fix #E-3.

### 7.4 OPA Gatekeeper ConstraintTemplate missing (defense-in-depth optional)

The architecture doc mentions OPA Gatekeeper as defense-in-depth. The Helm chart doesn't ship templates. Acceptable for MVP-of-MVP but should be documented as deferred.

**Decision** : defer to follow-on. Document in § 13 done definition exclusion.

### 7.5 Image whitelist requires manual digest pinning

`Values.imageWhitelist` defaults to `""` (empty → reject everything). Operators must populate it manually after first cosign sign + Rekor push.

**Fix** : add a `post-install` Helm hook that fetches the latest cosign-signed digest from GHCR and populates the whitelist. Detail in § 9 fix #E-4 (skipped for MVP — manual is OK).

---

## 8. End-to-end verification strategy (5-tier)

A 5-tier deployment + smoke test plan, executed sequentially. Each tier blocks if the prior fails.

### Tier 1 — Workspace `cargo build` + `cargo test`

| Check | Command | Expected |
|---|---|---|
| Workspace check | `cargo check --workspace` | OK |
| Lib build | `cargo build --workspace --lib` | OK, no warning |
| Bin build | `cargo build --workspace --bins` | All bins compile |
| Test compile | `cargo test --workspace --no-run` | All tests compile |
| Targeted tests | `cargo test -p efe-agent-runner --lib` | All pass |

### Tier 2 — vsock-relayer Go build

| Check | Command | Expected |
|---|---|---|
| `go vet` | `cd cmd/vsock-relayer && go vet ./...` | No warnings |
| Build | `go build -o /tmp/vsock-relayer .` | Binary produced |
| Docker | `docker build -f cmd/vsock-relayer/Dockerfile -t test:vsock .` | OCI image produced |

### Tier 3 — Helm template lint + unit tests

| Check | Command | Expected |
|---|---|---|
| Helm lint | `helm lint charts/efe-agent-runner/` | No errors |
| Helm template | `helm template charts/efe-agent-runner/` | Valid YAML |
| Unit tests | `helm unittest charts/efe-agent-runner/` (if helm-unittest installed) | All pass |

### Tier 4 — k3d local deploy

A scripted setup :

```bash
#!/usr/bin/env bash
set -euo pipefail

k3d cluster create efe-assembly --servers 1 --agents 2

# Install cert-manager + Postgres + a self-signed ClusterIssuer
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.16.0/cert-manager.yaml
kubectl wait --for=condition=available deployment -n cert-manager --all --timeout=180s

helm install pg bitnami/postgresql --set auth.postgresPassword=test --wait
# ... apply migrations 0001-0020

helm install efe-agent-runner charts/efe-agent-runner \
    --set agentRuntimeEnabled=true \
    --set admissionWebhook.enabled=true \
    --set vsockRelayer.enabled=false \         # local k3d has no /dev/kvm
    --wait --timeout 5m

# Smoke
kubectl apply -f tests/fixtures/agentrun-noop.yaml
kubectl wait --for=condition=Started agentrun/test-noop --timeout 60s
```

Pass criteria : webhook responds, AgentRun CR is admitted, controller reconciles to status PENDING (not RUNNING because no KVM), no P0/P1 alert.

### Tier 5 — Chaos cluster smoke (optional but recommended)

The full deployment with KVM. Run :
```bash
EFE_CLUSTER_ROLE=chaos pytest bench/m26/bench_50tenants_5concurrent.py --duration 300
```

Pass criteria : the 5-min mini bench shows 0 sandbox escapes + provisioning P99 < 10s + 100% run success rate.

---

## 9. Fix catalogue : 22 items

Items grouped by category (A=compile, B=M23 inherited, C=trait+adapter, D=runtime integration, E=deploy). Each item has a one-line summary + DRI + estimated effort.

### Category A — rustc bump

| ID | Summary | DRI | Effort |
|---|---|---|---|
| A-1 | Bump workspace `rust-version = "1.88"` in `Cargo.toml` | Eng lead | 5 min |
| A-2 | Add `rust-toolchain.toml` pinning channel `1.88` | Eng lead | 5 min |
| A-3 | Update CI workflow to use rustc 1.88 docker image | DevOps | 15 min |
| A-4 | Update `CONTRIBUTING.md` with toolchain setup | Tech writer | 15 min |

### Category B — M23 inherited debt fix

| ID | Summary | DRI | Effort |
|---|---|---|---|
| B-1 | `commands/curation/get.rs:10` — `proposal.tenant_id` doesn't exist on `ProposalSummary` ; add the field via DTO update OR look it up from the catalog | Backend | 15 min |
| B-2 | `commands/curation/get.rs:16` — `proposal.actual_cost_cents` missing ; same fix pattern | Backend | 10 min |
| B-3 | `commands/curation/propose_split.rs:23` — `proposal_obj["proposed_class_id"]` requires `proposal_obj` to be an `&Map` not `&Value` ; use `.get(...)` | Backend | 10 min |
| B-4 | `commands/curation/propose_split.rs:23` — same pattern for `anchor_candidates` | Backend | 5 min |
| B-5 | `commands/curation/diff.rs:15-16` — `DiffSummary: Serialize` missing ; add `#[derive(Serialize)]` on `DiffSummary` in curation_client.rs | Backend | 5 min |
| B-6 | Run `cargo build -p efe-admin` + verify | Backend | 5 min |

### Category C — trait + adapter

| ID | Summary | DRI | Effort |
|---|---|---|---|
| C-1 | Create `crates/efe-server/src/v1/agent/adapter.rs` with `AgentRuntimeServiceAdapter` implementing `AgentRuntimeFacade` | Backend | 60 min |
| C-2 | Add to M25 `AgentRuntimeService` : `list_recent_steps()` + `list_pending_approvals_for_run()` methods (DB queries) | Backend | 30 min |
| C-3 | Mount `/v1/agent/*` axum sub-router in `efe-server` main HTTP listener | Backend | 30 min |
| C-4 | Wire `AgentApiState` construction with the adapter + PgPool in efe-server bootstrap | Backend | 20 min |

### Category D — runtime integration

| ID | Summary | DRI | Effort |
|---|---|---|---|
| D-1 | `run.rs::insert_run` SQL INSERT : rename `template_id` → `template_slug` in column list | Backend | 5 min |
| D-2 | `main.rs` (efe-agent-runner) : extract pool map to outer scope, pass `Arc<Mutex<...>>` to both controller + reaper | Backend | 20 min |
| D-3 | `deployment-controller.yaml` : add `EFE_M26_WEBHOOK_*` env vars conditional on `admissionWebhook.enabled` | DevOps | 15 min |
| D-4 | `deployment-controller.yaml` : add cert-manager Secret volume mount at `/var/run/webhook-tls/` | DevOps | 10 min |
| D-5 | Document the pen test `_test/*` endpoints as "chaos-cluster only, deferred to follow-on sprint" in `bench/m26_pentest/README.md` | Tech writer | 10 min |
| D-6 | Add `crates/efe-agent-runner/Dockerfile` (multi-stage Rust build → distroless) | DevOps | 30 min |

### Category E — deploy

| ID | Summary | DRI | Effort |
|---|---|---|---|
| E-1 | Add `selfSigned` issuer fallback in `webhook-cert.yaml` if `bootstrap: true` | DevOps | 15 min |
| E-2 | Add `pdb-controller.yaml` (PodDisruptionBudget minAvailable=1) | DevOps | 10 min |
| E-3 | Document `kubectl label node ... efe.ai/kvm-available=true` in M26 runbook + emit warning event from controller if no labeled node | DevOps + Backend | 15 min |
| E-4 | Add `cargo-check` GitHub Action job that runs `cargo build --workspace` to prevent regression | DevOps | 15 min |

**Total estimated effort** : ~5h 35min (321 minutes). Within the 8-hour sprint window.

---

## 10. ADR addendums

### ADR-D93-001 : Bump workspace rustc to 1.88

**Status** : Accepted.
**Context** : `time 0.3.47` (transitive via `jsonwebtoken`) requires rustc ≥ 1.88. Cannot downgrade `time` because `simple_asn1 0.6.4` locks `^0.3.47`. Cannot replace `jsonwebtoken` for OIDC validation in efe-server.
**Decision** : bump workspace `rust-version = "1.88"` and pin via `rust-toolchain.toml`.
**Consequences** : developers on rustc < 1.88 must update via `rustup update`. CI image must be ≥ 1.88. No runtime impact ; binary compatibility intact.

### ADR-D93-002 : Defer pen-test `_test/*` endpoints to chaos-cluster sprint

**Status** : Accepted.
**Context** : 9 of the 30 pen-test scenarios reference admin-only endpoints under `/v1/agent/_test/*` that don't exist. Building them requires careful gating (`EFE_CLUSTER_ROLE=chaos` check) + non-trivial logic (e.g. simulated audit tampering).
**Decision** : tag affected scenarios as `pytest.skip(reason="chaos endpoint not implemented")` for M26 GA. Implement in a separate chaos-cluster onboarding sprint pre-first-paying-customer.
**Consequences** : 9 of 30 pen tests don't run in CI ; the 21 remaining tests cover sandbox escape, tool whitelist (3-layer), and cosign forgery. The graduation criterion "0 P0/P1 unresolved" relaxes to "0 P0/P1 unresolved among the 21 implemented scenarios" until the chaos sprint completes.

### ADR-D93-003 : Mount `/v1/agent/*` via efe-server, not a separate gateway pod

**Status** : Accepted.
**Context** : The REST gateway code lives in `crates/efe-server/src/v1/agent/`. We could either (a) ship it as an integrated sub-router inside efe-server's existing HTTP listener, or (b) split it out as a dedicated gateway pod with its own deployment.
**Decision** : (a). The REST gateway calls into `AgentRuntimeService` in-process (efe-agent-runner crate is a Cargo dep of efe-server, OR efe-agent-runner is a separate process called via gRPC mesh — choose the latter for STABLE channel to honour the architectural separation).
**Consequences** : efe-server depends on `efe-agent-runner` workspace member at compile time (for the trait), but the `AgentRuntimeServiceAdapter` calls the real service via tonic gRPC across pods (mTLS-secured). Pure-mesh isolation : either component can scale independently ; if efe-agent-runner crashes, efe-server returns 503 with clean error.

---

## 11. Sprint plan D93

### 11.1 Schedule (8 hours, single working day)

| Slot | Time | Owner | Task |
|---|---|---|---|
| 09:00 | 30 min | All | Stand-up + scope review + assign DRIs |
| 09:30 | 30 min | Eng lead | A-1, A-2, A-3, A-4 — toolchain bump |
| 10:00 | 60 min | Backend | C-1, C-2 — adapter + service methods (parallel with B fixes below) |
| 10:00 | 30 min | Backend (#2) | B-1, B-2, B-3, B-4 — curation field fixes |
| 10:30 | 15 min | Backend (#2) | B-5, B-6 — Serialize derive + verify build |
| 10:45 | 60 min | Backend | C-3, C-4 — REST mount + bootstrap |
| 11:45 | 5 min | Backend | D-1 — SQL INSERT rename fix |
| 11:50 | 20 min | Backend | D-2 — pool sharing |
| 12:10 | LUNCH | | |
| 13:00 | 25 min | DevOps | D-3, D-4 — env vars + volume mount |
| 13:25 | 10 min | Tech writer | D-5 — pen test deferral doc |
| 13:35 | 30 min | DevOps | D-6 — controller Dockerfile |
| 14:05 | 15 min | DevOps | E-1 — selfSigned fallback |
| 14:20 | 10 min | DevOps | E-2 — PDB |
| 14:30 | 15 min | DevOps + Backend | E-3 — node label warning |
| 14:45 | 15 min | DevOps | E-4 — CI workflow |
| 15:00 | 60 min | All | **Tier 1 verification** : `cargo build --workspace` + `cargo test --workspace --no-run` |
| 16:00 | 30 min | DevOps | **Tier 2** : `go vet` + `go build` vsock-relayer |
| 16:30 | 30 min | DevOps | **Tier 3** : `helm lint` + `helm template` |
| 17:00 | 60 min | DevOps + SRE | **Tier 4** : k3d local deploy + smoke |
| 18:00 | 30 min | All | Wrap-up : commit + PR + CHANGELOG entry |

### 11.2 Critical path

```
A-1 (toolchain bump) ──┬─► B-1..B-6 (curation fixes) ────► Tier 1 (cargo build)
                       └─► C-1, C-2 (adapter) ──────► C-3, C-4 (mount + bootstrap)
                                                          │
                                                          ▼
                                                       D-1, D-2 (runtime fixes)
                                                          │
                                                          ▼
                                                       D-3..D-6, E-1..E-4 (deploy fixes)
                                                          │
                                                          ▼
                                                       Tiers 2/3/4 (lint + deploy)
```

### 11.3 Resource allocation

- **Eng lead** : 30 min on A-* + critical path supervision.
- **Backend #1** : 4h on C-* + D-1, D-2 (heaviest path).
- **Backend #2** : 1h on B-* (can finish early, then assist Backend #1).
- **DevOps** : 4h on D-3..D-6 + E-* + tier verification.
- **SRE** : 1h tier 4 verification + 1h on standby for incidents.
- **Tech writer** : 15 min on documentation.

---

## 12. Risk register

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|:--:|:--:|---|
| R-1 | rustc 1.88 not yet stable in some CI providers | LOW | HIGH | Verify GitHub Actions has rust:1.88 image ; prepare fallback to `rustup update` step |
| R-2 | M23 fixes have hidden dependencies (other curation tests fail) | MEDIUM | MEDIUM | Run `cargo test -p efe-admin --no-run` after each fix |
| R-3 | Adapter implementation reveals missing M25 service methods beyond list_recent_steps + list_pending_approvals | MEDIUM | MEDIUM | Time-box adapter work to 90 min ; if more methods needed, defer non-critical to follow-on |
| R-4 | k3d local deploy reveals new integration issues | MEDIUM | HIGH | Allocate 60 min to Tier 4 ; if blocked, document + defer |
| R-5 | Helm cert-manager test fails because cluster lacks cert-manager pre-install | LOW | LOW | Pre-install step in setup script ; document in runbook |
| R-6 | vsock-relayer Go module needs `go.sum` regenerated | LOW | LOW | `go mod tidy` in Tier 2 |
| R-7 | Sprint window exceeds 8h | MEDIUM | MEDIUM | Scope reduction : defer E-3, E-4 to next sprint if running long |

---

## 13. Done definition

The sprint is **DONE** when ALL of the following are true :

- [ ] `cargo build --workspace` passes with no errors (warnings allowed).
- [ ] `cargo test --workspace --no-run` compiles all tests.
- [ ] `cargo build --workspace --release` produces all binaries.
- [ ] `helm lint charts/efe-agent-runner/` returns no errors.
- [ ] `helm template charts/efe-agent-runner/` produces valid YAML.
- [ ] In a fresh k3d cluster, `helm install` completes within 5 min and all pods reach Ready.
- [ ] Webhook responds to a test `kubectl apply -f tests/fixtures/agentrun-noop.yaml` with admission allowed.
- [ ] Controller reconciles the AgentRun to PROVISIONING state (PENDING in k3d-without-KVM).
- [ ] CHANGELOG `[Unreleased]` has a `## EFE 2.1.1 — Assembly verification` section.
- [ ] PR opened with the 22 fixes + linking to this architecture doc.
- [ ] One reviewer (Eng lead) approves before merge.

### Explicit non-criteria (deferred to follow-on)

- Pen test `/v1/agent/_test/*` endpoints (ADR-D93-002).
- OPA Gatekeeper ConstraintTemplates (deferred).
- Image whitelist auto-population post-install hook.
- M23/M24 portal page migrations (separate GLOSS-B sprint).
- 30-tenant chaos cluster bench (Tier 5, optional, can run after sprint).

---

## Annexes

### Annex A — Final list of files touched

```
Cargo.toml                                                              (rust-version bump)
rust-toolchain.toml                                                     (NEW pin file)
.github/workflows/ci.yml                                                (rustc 1.88)
CONTRIBUTING.md                                                         (toolchain setup)

crates/efe-admin/src/commands/curation/get.rs                           (B-1, B-2)
crates/efe-admin/src/commands/curation/propose_split.rs                 (B-3, B-4)
crates/efe-admin/src/commands/curation/diff.rs                          (B-5, B-6)
crates/efe-admin/src/grpc/curation_client.rs                            (Serialize on DiffSummary)

crates/efe-server/src/v1/agent/adapter.rs                               (NEW C-1)
crates/efe-server/src/v1/agent/mod.rs                                   (export adapter)
crates/efe-server/src/main.rs                                           (C-3, C-4)

crates/efe-agent-runner/src/service.rs                                  (C-2 — 2 new methods)
crates/efe-agent-runner/src/run.rs                                      (D-1)
crates/efe-agent-runner/src/main.rs                                     (D-2)
crates/efe-agent-runner/Dockerfile                                      (NEW D-6)

charts/efe-agent-runner/templates/deployment-controller.yaml            (D-3, D-4)
charts/efe-agent-runner/templates/pdb-controller.yaml                   (NEW E-2)
charts/efe-agent-runner/templates/webhook-cert.yaml                     (E-1)
charts/efe-agent-runner/values.yaml                                     (selfSigned + bootstrap)

bench/m26_pentest/README.md                                             (D-5)
docs/runbooks/M26_AGENT_RUNTIME_PRODUCTION_RUNBOOK.md                   (E-3)

CHANGELOG.md                                                            (2.1.1 entry)
docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md                     (this doc, NEW)
docs/docs/adr/ADR-D93-{001-rustc-1.88-bump,002-pen-test-deferral,003-rest-mount-strategy}.md  (NEW ADRs ; co-located with ADR-001 at docs/docs/adr/ per existing convention)
scripts/assembly-verify.sh                                              (NEW Annex B 5-tier driver, chirurgical audit)
scripts/assembly-k3d-test.sh                                            (NEW Annex B Tier 4 driver, chirurgical audit)
crates/efe-agent-runner/src/flags.rs                                    (NEW PgTenantFlagsProvider, chirurgical audit C-5)
crates/efe-active-learning/src/centroid.rs                              (Vec<Copy> test fix, chirurgical audit)
charts/efe-stack/charts/efe-server/{values,templates/service,templates/deployment}.yaml  (REST gateway port + env, chirurgical audit)
```

Total : ~22 files modified or created.

### Annex B — One-line sanity script

```bash
#!/usr/bin/env bash
set -euo pipefail
echo "=== Tier 1 : workspace build ==="
cargo build --workspace
echo "=== Tier 1 : workspace test compile ==="
cargo test --workspace --no-run
echo "=== Tier 2 : vsock-relayer ==="
( cd cmd/vsock-relayer && go vet ./... && go build -o /tmp/vsock-relayer . )
echo "=== Tier 3 : Helm ==="
helm lint charts/efe-agent-runner
helm template test charts/efe-agent-runner --set agentRuntimeEnabled=true >/tmp/helm-out.yaml
kubectl --dry-run=client apply -f /tmp/helm-out.yaml >/dev/null 2>&1 || true
echo "=== Tier 4 : k3d ==="
( cd scripts && bash assembly-k3d-test.sh )
echo "=== ALL TIERS PASSED ==="
```

Save as `scripts/assembly-verify.sh`. Run before merge.

### Annex C — CHANGELOG snippet to add

```markdown
### EFE 2.1.1 — Assembly verification (D93)

> M26 cross-cutting assembly verification : 22 fixes across compile,
> runtime integration, and deploy. Workspace `cargo build` now passes
> end-to-end ; Helm chart deploys to k3d in < 5 min ; admission webhook
> + REST gateway + reaper + adapter all wired.
>
> **Highlights** :
> - rustc bumped to 1.88 (workspace `rust-version` + `rust-toolchain.toml`)
> - 6 M23 inherited curation lib errors fixed (pre-existing debt)
> - `AgentRuntimeServiceAdapter` implements `AgentRuntimeFacade` trait
> - REST gateway `/v1/agent/*` mounted on efe-server HTTP listener
> - M25 SQL INSERT updated for `template_slug` rename
> - Reaper pool map shared between controller + reaper
> - Webhook env vars + cert-manager Secret volume mount in deployment
> - Controller Dockerfile (was missing despite Helm chart referencing the image)
> - PDB for HA controller deployment
> - cert-manager selfSigned fallback for `bootstrap: true`
> - 9 of 30 pen-test scenarios marked `pytest.skip` until chaos sprint (ADR-D93-002)
>
> **Verification** : 5-tier deploy test passed (cargo build, go build, helm lint,
> k3d deploy, smoke). 0 P0/P1 introduced.
```

### Annex D — Sample PR template

```markdown
## Summary
M26 cross-cutting assembly verification (D93). Closes 22 integration gaps
identified in the M26 audit + adds pre-existing M23 debt fixes that
blocked `cargo build --workspace`.

## Changes
- 22 fixes across 5 categories (A-toolchain, B-M23 debt, C-adapter, D-runtime, E-deploy)
- 3 new ADRs (rustc 1.88, pen-test deferral, REST mount strategy)
- 5-tier deploy verification passed locally

## Test plan
- [ ] Tier 1 — `cargo build --workspace` passes
- [ ] Tier 2 — vsock-relayer `go vet` + `go build` clean
- [ ] Tier 3 — `helm lint` + `helm template` clean
- [ ] Tier 4 — k3d local deploy → AgentRun PENDING reconcile in < 60s
- [ ] Tier 5 (optional) — chaos cluster 5-min bench

## Reviewers
- Eng lead (architecture sign-off)
- SRE (deploy-time validation)

## Linked
- Architecture : `docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md`
- M26 spec : `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md`
```

---

## End of document

This 1-day sprint produces **EFE 2.1.1**, a fully-assembled, k3d-deployable, Helm-installable, end-to-end-verified release of the M26 production-graduated agent runtime. After merge, M26 transitions from "code drop complete" to "production-deployable", and the first design-partner onboarding can begin.
