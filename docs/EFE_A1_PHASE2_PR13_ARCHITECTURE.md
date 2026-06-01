# EFE-A1 Phase 2 — PR #13 : Helm sub-chart skeleton

| Field | Value |
|---|---|
| **PR** | #13 (Sprint A1 Phase 2 / P2.4) |
| **Branch** | `efe-a1/p2-pr13-helm-subchart` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× meta-swarm + 1× sre + 1× platform-deploy + 1× sec |
| **Depends on** | PR #5 (proto), PR #6 (scaffold + binaries), PR #8 (controller wire), PR #9 (participant wire), PR #10 (multiprocess), PR #11 (efe-server flag plumbing), PR #12 (SIGHUP + ReloadConfig) |
| **Blocks** | PR #14 (sidecar reload coordination), PR #16 (mTLS provisioning), PR #17 (multi-controller HA) |
| **L2 gate** | G10 (K8s deployment of meta-swarm controller + 3 participants with declarative configuration management) |
| **Threat IDs in scope** | T22 (secret leakage via values.yaml in git), T23 (NetworkPolicy bypass — east-west traffic), T24 (image-tag floating / supply chain) |
| **Roadmap reference** | EFE_A1_PHASE2_ROADMAP.md § C6 |
| **CVSS bandwidth** | All ≤ SEV-2 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals
  2. Pre-flight checks
  3. Branch + commit plan (6 commits)
  4. Architectural decisions (AD1–AD25)
  5. Topology + integration diagram
  6. Chart layout under `charts/efe-stack/charts/efe-meta-swarm/`
  7. `Chart.yaml` + parent dependency registration
  8. `values.yaml` schema (operator surface)
  9. `values.schema.json` (JSON Schema validation rules)
 10. `_helpers.tpl` (named templates)
 11. `configmap-manifest.yaml` (manifest YAML rendering)
 12. `controller-deployment.yaml`
 13. `controller-service.yaml` + headless DNS
 14. `participant-deployment.yaml` (`range` over participants)
 15. `participant-service.yaml`
 16. `serviceaccount.yaml` + RBAC (Phase 2 minimal surface)
 17. `pdb-controller.yaml` (PodDisruptionBudget)
 18. `servicemonitor.yaml` (Prometheus Operator, optional)
 19. `networkpolicy.yaml` (deny-by-default, optional)
 20. `templates/tests/connection-test.yaml` (`helm test` hook)
 21. `NOTES.txt` + `README.md`
 22. Image management — registry, tag pinning, pullSecrets
 23. SecurityContext / hardening
 24. Resource sizing + autoscaling deferral
 25. SIGHUP-on-`ConfigMap`-change strategy
 26. Golden snapshot test strategy (`tests/golden/`)
 27. CI integration (`helm lint` + `helm template` + `ct lint`)
 28. Multi-namespace + multi-tenant considerations
 29. TLS bootstrapping (cert-manager) — Phase-2 placeholders
 30. Metrics integration (Prometheus Operator)
 31. Logging / tracing wiring
 32. K8s DNS discovery — how the controller finds participants
 33. Concurrency / lifecycle invariants
 34. Threat-model coverage (T22 / T23 / T24)
 35. Tests strategy
 36. Acceptance criteria (AC1–AC30)
 37. Risks + mitigations
 38. Rollback strategy
 39. Sequence diagrams (ASCII)
 40. Reviewer checklist
 41. Operator runbook
 42. Open questions
 43. Appendix : full file inventory
 44. Appendix : example overrides (Kovant production / staging / dev)
 45. Glossary

---

## 0. TL;DR

PR #13 ships the **first deployable surface** for the Phase-2 meta-swarm
subsystem : a Helm **sub-chart** that lives under the existing
`charts/efe-stack/` umbrella and packages :

  * 1 × `controller` `Deployment` + `Service` + `ServiceAccount` + `PDB`
  * `N` × `participant` `Deployment` + `Service` + `ServiceAccount`
    (`N = 3` by default — the Kovant scenario : `supplier-scoring`,
    `contract-risk`, `spend-categorization`).
  * 1 × `ConfigMap` rendering the `manifest.yaml` from `values.yaml`.
  * Optional `ServiceMonitor` (Prometheus Operator) + `NetworkPolicy`
    (deny-by-default east-west traffic).
  * A `helm test` connection probe that runs `grpcurl` against
    `MetaSwarmService::Health`.

The sub-chart is **disabled by default** (`efe-meta-swarm.enabled =
false` in the umbrella's `values.yaml`).  Setting it to `true`
renders all resources and wires the parent `efe-server` chart's
existing `EFE_META_SWARM_ENABLED=true` env var path (PR #11) at the
controller's `ClusterIP` DNS.

The chart deliberately **does not** use a `checksum/config` annotation
on the controller / participant `Deployment` pod template hash so that
a `ConfigMap` edit does **not** trigger a `RollingUpdate` (the whole
point of PR #12's SIGHUP-driven hot reload).  Operators trigger reload
via `kubectl exec ... -- kill -HUP 1` ; the runbook (§ 41) documents
the exact incantation.

`values.schema.json` enforces typed validation (kappa ∈ (0, 1),
image tag must be present, etc.) so typos surface at
`helm install --dry-run` time, not at pod-boot time.

A `tests/golden/` directory pins the rendered YAML for canonical
scenarios (kovant-3p / disabled / with-network-policy / with-service-monitor) ;
CI diffs the live `helm template` output against these snapshots,
catching unintentional template drift.

PR #13 closes Phase-2 graduation gate **G10 (K8s deployment of the
meta-swarm controller + 3 participants with declarative
configuration management)**.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Package the Phase-2 meta-swarm subsystem (1 controller + N participants
> + manifest `ConfigMap` + optional Prometheus / NetworkPolicy) as a
> Helm sub-chart under `charts/efe-stack/charts/efe-meta-swarm/`,
> disabled by default, with typed `values.schema.json` validation,
> golden-snapshot regression tests, and a `helm test` connection probe,
> while preserving PR #12's SIGHUP-driven hot reload semantics
> (no `checksum/config` annotation on the Deployment).

### 1.2 Non-goals (deferred to later PRs)

| Item | Why deferred | Target PR |
|---|---|---|
| Sidecar weight-loader + Init container | needs the on-disk weight contract | PR #14 |
| mTLS via cert-manager-issued certs | needs the CA bootstrap | PR #16 |
| Multi-controller HA / leader election | needs leader-lock subsystem | PR #17 |
| HorizontalPodAutoscaler for controller | autoscale + leader-election interplay is non-trivial | PR #17 |
| In-cluster `ConfigMap`-watcher → SIGHUP trigger Pod | one more moving part ; manual SIGHUP is sufficient for Phase 2 | PR #18 |
| OpenTelemetry collector sidecar | needs the global OTEL operator chart wiring | PR #19 |
| Helm `pre-upgrade` hook for state-migration | no state migration in Phase 2 | PR #20 |
| Vertical Pod Autoscaler integration | needs metrics-server prereq | PR #20 |
| OpenShift `SecurityContextConstraints` | minority deployment target ; runbook covers the override | follow-up |
| Helmfile / Argo CD integration | deployment-tool concern, orthogonal to chart correctness | follow-up |
| `Knative` / serverless flavor | not a Phase-2 target | follow-up |
| Cross-cluster federation | needs service-mesh contracts | Phase 3 |
| GPU node-pool support for participants | Phase-1 participants are CPU only | Phase 3 |
| `PodSecurityPolicy` (PSP) — deprecated in K8s 1.25 | we ship `SecurityContext` + `seccompProfile` + `runAsNonRoot` instead | n/a |
| Image-signing / cosign verification policy | needs OPA / Kyverno admission controller | Phase 3 |

### 1.3 Scope summary

PR #13 ships :

  1. Chart skeleton at `charts/efe-stack/charts/efe-meta-swarm/`.
  2. `values.yaml` with sensible Kovant defaults.
  3. `values.schema.json` with structural + range validation.
  4. `_helpers.tpl` with named templates (`<name>.fullname`,
     `.labels`, `.selectorLabels`, `.serviceAccountName`, `.image`,
     `.controllerEndpoint`, `.participantsEnv`).
  5. `ConfigMap` rendering the manifest YAML from `values`.
  6. `Deployment` + `Service` + `ServiceAccount` per role (controller +
     N participants).
  7. `PodDisruptionBudget` for the controller (`maxUnavailable: 0`).
  8. Optional `ServiceMonitor` for Prometheus Operator stacks.
  9. Optional `NetworkPolicy` for deny-by-default east-west traffic.
 10. `templates/tests/connection-test.yaml` for `helm test`.
 11. `NOTES.txt` (post-install operator hints) + `README.md`.
 12. `tests/golden/` snapshots for regression.
 13. Parent-chart wiring : `dependencies:` entry in
     `charts/efe-stack/Chart.yaml` with `condition:
     efe-meta-swarm.enabled`.
 14. Parent-chart values default : `efe-meta-swarm: { enabled: false }`.

PR #13 does **not** change :

  * The controller / participant binaries themselves (PR #6 / #8 / #9
    / #12 are the substance ; this PR just packages them for K8s).
  * The proto contract (PR #5).
  * The `efe-server` connection logic (PR #11) — we only template
    the connection-endpoint env var into the parent's Deployment when
    the sub-chart is enabled.

---

## 2. Pre-flight checks

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | `helm version` ≥ 3.13.0 (Helm 3 required ; Helm 2 EOL 2020) | CI | green |
| PF2 | `helm lint charts/efe-stack` is clean on `main` | CI | exit 0 |
| PF3 | `helm template charts/efe-stack` succeeds (parent dry-run) | CI | exit 0 |
| PF4 | `kubectl` version supported : 1.28 ≤ K8s ≤ 1.30 (matches `kubeVersion` in `charts/efe-stack/Chart.yaml`) | manual | matches |
| PF5 | `kubeconform` (or `kubeval`) binary available on CI | CI | found |
| PF6 | `chart-testing` (`ct`) binary available on CI | CI | found |
| PF7 | Container image `ghcr.io/efe-ai/efe-meta-swarm-server:<appVersion>` is **published** for the chart's `appVersion` (PR #10's CI builds it ; PR #13 references it) | CI | image manifest fetchable |
| PF8 | `prometheus-operator` CRDs (`monitoring.coreos.com/v1`) installed on target clusters that enable `serviceMonitor.enabled` (operator-side prerequisite) | manual | optional ; documented in runbook |
| PF9 | `networking.k8s.io/v1 NetworkPolicy` enforced by the cluster's CNI (calico / cilium) when `networkPolicy.enabled` | manual | optional ; documented |
| PF10 | Pre-existing `efe-stack` sub-chart conventions reviewed (`efe-server`, `efe-edge`, `efe-vector`) and our chart aligns | manual | conformance |
| PF11 | Threat model (PR #7) is at v1.0 frozen state | manual | sec sign-off |

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr13-helm-subchart
```

Branched from `main` after PR #12 merge SHA.

### 3.2 Commit-by-commit plan (6 commits, each `helm lint` + `helm template` green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| **C1** | `feat(charts): efe-meta-swarm scaffold (Chart.yaml + values.yaml + _helpers.tpl + NOTES.txt + README + values.schema.json)` | `+ charts/efe-stack/charts/efe-meta-swarm/Chart.yaml`, `+ values.yaml`, `+ values.schema.json`, `+ README.md`, `+ templates/_helpers.tpl`, `+ templates/NOTES.txt`, `~ charts/efe-stack/Chart.yaml`, `~ charts/efe-stack/values.yaml` (registers the sub-chart, disabled) | +320 / 0 | `helm lint` clean ; `helm template` produces zero meta-swarm resources by default |
| **C2** | `feat(charts): efe-meta-swarm ConfigMap with manifest YAML rendering` | `+ templates/configmap-manifest.yaml`, `+ tests/configmap_render.bats` (or shell script in `tests/`) | +110 / 0 | Golden test : rendered YAML round-trips through `manifest::load_and_validate` (PR #8) |
| **C3** | `feat(charts): efe-meta-swarm controller Deployment + Service + ServiceAccount + PDB` | `+ templates/controller-deployment.yaml`, `+ templates/controller-service.yaml`, `+ templates/controller-serviceaccount.yaml`, `+ templates/pdb-controller.yaml` | +280 / 0 | Golden test : kovant-3p scenario ; `kubeconform` validates |
| **C4** | `feat(charts): efe-meta-swarm participant Deployments + Services (range over participants)` | `+ templates/participant-deployment.yaml`, `+ templates/participant-service.yaml`, `+ templates/participant-serviceaccount.yaml` | +260 / 0 | Golden test : 3 participant Deployments + 3 Services rendered |
| **C5** | `feat(charts): efe-meta-swarm optional ServiceMonitor + NetworkPolicy` | `+ templates/servicemonitor.yaml`, `+ templates/networkpolicy.yaml` | +160 / 0 | Golden tests : both off → no resources ; both on → CRD-correct |
| **C6** | `feat(charts): efe-meta-swarm helm-test hook + golden snapshots + CI integration` | `+ templates/tests/connection-test.yaml`, `+ tests/golden/kovant-3p.yaml`, `+ tests/golden/kovant-disabled.yaml`, `+ tests/golden/kovant-network-policy.yaml`, `+ tests/golden/kovant-service-monitor.yaml`, `~ .github/workflows/helm.yml` (or equivalent) | +210 / 0 | CI runs `helm template ... > /tmp/rendered.yaml && diff tests/golden/*.yaml /tmp/...` ; `helm test` invocation against a kind cluster (Phase-3 stretch goal but plumbing here) |

**Total : +1 340 / 0 = +1 340 LOC net.**  No Rust code in PR #13 ;
the entire delta is YAML / Go templates / JSON schema / shell.

### 3.3 Drafting strategy

  * Each commit is **independently `helm lint`-green** so a partial
    revert is mechanical.
  * C1 ships the scaffold + parent registration with `enabled: false`
    by default — zero behavioral change.
  * C2-C5 add templates one resource family at a time, each guarded
    by `{{- if .Values.enabled }}` (so still no behavioral change
    without an operator opt-in).
  * C6 closes the loop with regression tests + CI wiring.

If C5 (NetworkPolicy / ServiceMonitor) flakes on a CI cluster that
lacks the CRDs, revert it alone — the production deployment stays on
the C1-C4 baseline.  Both are opt-in and behave as no-ops by default.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | Ship as a **sub-chart** under `charts/efe-stack/charts/efe-meta-swarm/`, not a standalone chart | Aligns with every other Phase-2 component (efe-server, efe-edge, efe-vector, efe-kg) ; one `helm install efe-stack` brings up everything coherent ; values inherited from umbrella `global:` block | Standalone chart with explicit dep ; Kustomize overlay ; raw manifests | Reversible — promotion to standalone is mechanical if cross-cluster federation needs it (Phase 3) |
| **AD2** | **Disabled by default** (`efe-meta-swarm.enabled = false` in umbrella values) | Mirrors PR #11's `EFE_META_SWARM_ENABLED=false` doctrine ; pre-Phase-2 deployments stay untouched | Enabled by default ; required dependency | Reversible — flipping to `true` in a new umbrella values flavor is one-line |
| **AD3** | **One ConfigMap holds the rendered manifest.yaml** ; both controller and participants mount it at the same path | Single source of truth ; PR #12's SIGHUP-driven reload picks up changes from this path ; matches Phase-1 `EFE_META_SWARM_MANIFEST` env var conventions | Two ConfigMaps (one per role) ; in-line env vars | Reversible — split is mechanical |
| **AD4** | **No `checksum/config` annotation** on Deployment pod templates | PR #12's SIGHUP-driven hot reload is the official live-reload mechanism ; the checksum annotation would trigger `RollingUpdate` on every ConfigMap edit, defeating the design ; operators trigger SIGHUP explicitly (§ 25) | Use the annotation (legacy, rolls pods on change) ; controller-side ConfigMap-watcher (Phase 3) | Reversible — adding the annotation is one line in `controller-deployment.yaml` ; we document the trade-off |
| **AD5** | **`Deployment` (not `StatefulSet`)** for controller + participants | Controller is stateless (manifest is the only durable input ; participants own their own tensor state in-process) ; `Deployment` gives faster `RollingUpdate`, simpler upgrade semantics ; Phase-2 deliberately does NOT distribute state across replicas | `StatefulSet` (would suit a Phase-3 stateful tensor-sharded participant model) ; `DaemonSet` (no, we want a fixed count) | Reversible — Phase 3 may migrate to `StatefulSet` once weights live on PVCs |
| **AD6** | **`ClusterIP` Service per binary**, no `Headless` Service for Phase 2 | Static discovery (PR #6's `DiscoveryMode::StaticLocalhost` extended to in-cluster DNS via the chart) ; `ClusterIP` gives a stable single-VIP for the controller-to-participant connection ; no need for endpoints enumeration in Phase 2 (each participant `Deployment` has `replicas: 1`) | `Headless` Service + pod-FQDN ; `LoadBalancer` (external — not Phase-2 scope) | Reversible — `Headless` swap is one-line for Phase 3 when participants replicate |
| **AD7** | **K8s DNS-based discovery** : the controller's `EFE_META_SWARM_PARTICIPANTS` env var is templated as `id1=svc1.ns.svc:port,id2=svc2.ns.svc:port,...` | Existing PR #6 `static-localhost` discovery mode honours `id=host:port` entries ; in-cluster DNS resolves the `<svc>.<ns>.svc` form deterministically ; no new code required on the controller side | Implement `DiscoveryMode::K8sDns` (the placeholder exists in PR #6) ; SRV records | Reversible — Phase-3 PR may add `K8sDns` discovery, the chart would set `EFE_META_SWARM_DISCOVERY=k8s-dns` and drop the static env var |
| **AD8** | **`runAsNonRoot: true`, `runAsUser: 65532`, `readOnlyRootFilesystem: true`** for controller + participants | Matches the existing `efe-server` sub-chart's hardening posture ; the binaries don't write to disk (manifest is read-only ; logs go to stdout) ; locks the attack surface | Run as `root` (rejected — least-privilege) ; `runAsUser: 1000` (no convention) | Reversible — relaxing requires a `values.yaml` override ; locked-down is the default |
| **AD9** | **`drop: ALL` capabilities + `allowPrivilegeEscalation: false`** in `securityContext` | Defence in depth ; the binaries don't need any Linux capability | Drop nothing ; selective drop | Reversible |
| **AD10** | **Resource requests + limits explicit** : controller `(500m, 512Mi) → (2, 2Gi)`, participant `(200m, 256Mi) → (1, 1Gi)` | Sized for Phase-1 benchmark numbers + 3 participants + MAX_INFLIGHT=10 ; gives K8s scheduler enough info to bin-pack ; prevents noisy-neighbour | Best-effort (no requests) ; guaranteed (req=limit) | Reversible per-environment via values override |
| **AD11** | **`PodDisruptionBudget` `maxUnavailable: 0` for the controller** | Phase-2 has a single controller ; voluntary disruption (node drain) MUST be operator-acked ; matches "no controller HA in Phase 2" non-goal | `minAvailable: 1` (equivalent for replicas=1) ; no PDB | Reversible — Phase 3 may relax to `maxUnavailable: 1` once HA is in play |
| **AD12** | **No PDB on participants** (Phase 2) | Each participant is a sharded coordinator ; losing one for ≤ 30 s during a node drain is acceptable (controller's Iterate fails over to retry path) | PDB per participant ; ditto for the controller | Reversible |
| **AD13** | **`Service.spec.type: ClusterIP`**, no external `LoadBalancer` / `Ingress` | Meta-swarm is an internal subsystem (consumed by `efe-server` only) ; never exposed externally | `LoadBalancer` ; `Ingress` ; `NodePort` | Reversible — Phase 3 may add `ServiceMesh` integration |
| **AD14** | **NetworkPolicy is opt-in (`networkPolicy.enabled = false` by default)** but the template ships | Many clusters lack a CNI that enforces NetworkPolicy ; on those clusters a NetworkPolicy is a no-op and would confuse operators ; opt-in keeps the chart compatible everywhere | Always-on ; never ship | Reversible per-environment |
| **AD15** | **`ServiceMonitor` is opt-in** (`serviceMonitor.enabled = false`) | Requires `prometheus-operator` CRDs ; ship the template, gate on the flag | Always-on ; never ship | Reversible |
| **AD16** | **`values.schema.json` enforces typed validation** : `kappa ∈ (0, 1)`, image tag required, `participants` array non-empty, `metaDim ≥ 1` | Catches operator typos at `helm install --dry-run` time, not at pod-boot time | No schema (rely on runtime rejection) ; OPA admission policy | Reversible — schema can be relaxed by removing constraints |
| **AD17** | **`image.repository` defaults to `ghcr.io/efe-ai/efe-meta-swarm-server`** ; `image.tag` defaults to `""` (operators MUST override, enforced by schema) | No `:latest` tag ; reproducible deployments ; supply-chain hygiene | Default to chart's `appVersion` ; default to `:latest` (rejected) | Reversible — Phase 3 may inject `appVersion` automatically with a digest pin |
| **AD18** | **Probes : TCP probe on the gRPC port for liveness + readiness** | Simpler than `grpc_health_probe` (third-party binary in the image) ; sufficient for Phase 2 (a TCP-accepting controller is serving) ; `grpc_health_probe` adds image weight + supply-chain surface | `grpc_health_probe` ; HTTP probe on `/metrics` ; exec probe with `grpcurl` | Reversible — Phase 3 may swap to `grpc_health_probe` when image already includes it for other reasons |
| **AD19** | **`Service.appProtocol: grpc`** + Kubernetes service mesh hints | Tells L4-aware proxies (Linkerd, Istio) that the protocol is HTTP/2 + gRPC ; enables proper load balancing | Omit ; `appProtocol: http` | Reversible |
| **AD20** | **One ServiceAccount per role** (controller, per-participant) | Future-proofs PR #16's mTLS work (SA → SPIFFE ID → cert) ; isolates audit trails | Single SA for the whole sub-chart | Reversible — Phase 3 may collapse if mTLS uses a different identity model |
| **AD21** | **Golden snapshots in `tests/golden/`** are checked into git ; CI does `helm template` + diff | Catches template drift ; a PR that accidentally changes the rendered output forces an explicit `make update-golden` step | Live `helm template` per CI run with no comparison ; `kubeconform` only (validates, doesn't catch drift) | Reversible — removing golden tests is mechanical |
| **AD22** | **`templates/tests/connection-test.yaml`** is a `Pod` annotated `"helm.sh/hook": test` that runs `grpcurl` against `MetaSwarmService::Health` | Operators verify the deployment with `helm test efe-stack -n efe` after install ; matches existing `efe-server` chart pattern | None (`helm test` is the standard mechanism) | Reversible |
| **AD23** | **Single-binary image** : both `controller` and `participant` ship in the same image (`efe-meta-swarm-server`), the binary is selected via the `args` field (`["controller"]` vs `["participant"]`) | Halves image storage / pull bandwidth ; matches `[[bin]]` setup in `efe-meta-swarm-server/Cargo.toml` (PR #6 ships both binaries from the same crate, AD1) | Two distinct images | Reversible — splitting later is mechanical |
| **AD24** | **`charts/efe-stack/charts/efe-meta-swarm/`** is a **vendored** sub-chart (not pulled from a Helm repo at install time) | Reproducible deployments ; no out-of-band repo dependency ; matches every other `efe-stack` sub-chart | OCI Helm registry pull ; `helm dependency update` | Reversible — promotion to OCI is mechanical |
| **AD25** | **Topology labels** (`app.kubernetes.io/component=controller` vs `=participant` ; `efe.io/meta-swarm-role={controller,participant}` ; `efe.io/participant-id=<id>` on each participant pod) | Standard `app.kubernetes.io/*` labels for tool interop ; custom `efe.io/*` labels for in-cluster queries (`kubectl get pods -l efe.io/meta-swarm-role=participant`) | Use only standard labels | Reversible — adding labels is non-breaking |

---

## 5. Topology + integration diagram

### 5.1 Resource graph rendered by the sub-chart (`enabled=true`)

```
                       ┌────────────────────────────────────┐
                       │  Helm release : efe-stack          │
                       │  Namespace    : efe                │
                       └────────┬───────────────────────────┘
                                │
              ┌─────────────────┼─────────────────────────────┐
              │                 │                             │
              ▼                 ▼                             ▼
   ┌─────────────────┐  ┌─────────────────┐         ┌──────────────────┐
   │ ConfigMap       │  │ ServiceAccount  │         │ NetworkPolicy    │
   │  manifest       │  │  meta-swarm-*   │         │  (opt-in)        │
   │  -manifest      │  │   (4 SAs)       │         │   deny-by-default│
   └─────────────────┘  └─────────────────┘         └──────────────────┘
              │                 │
              │                 │
         (mount)            (subject)
              │                 │
              ▼                 ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ Deployment : efe-meta-swarm-controller     (replicas=1)      │
   │  Pod                                                          │
   │   ┌────────────────────────────────────────────────────┐     │
   │   │ container : controller                              │     │
   │   │   image    : ghcr.io/efe-ai/efe-meta-swarm-server  │     │
   │   │   args     : ["controller"]                        │     │
   │   │   ports    : 50080 (grpc) , 9091 (metrics)         │     │
   │   │   envFrom  : ConfigMap envvars                      │     │
   │   │   volumes  : /etc/efe-meta-swarm/manifest.yaml      │     │
   │   └────────────────────────────────────────────────────┘     │
   └─────────┬────────────────────────────────────────────────────┘
             │
             │ (label selector)
             ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ Service : efe-meta-swarm-controller   (ClusterIP, gRPC, 50080)│
   │  DNS    : efe-meta-swarm-controller.efe.svc.cluster.local     │
   └──────────────────────────────────────────────────────────────┘
                │
                │  (consumed by efe-server pod when
                │   EFE_META_SWARM_ENABLED=true ; the parent
                │   chart's deployment.yaml injects the endpoint)
                ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ efe-server Deployment    (from sibling sub-chart, PR #11 hook) │
   └──────────────────────────────────────────────────────────────┘

   ┌──────────────────────────────────────────────────────────────┐
   │ Deployment : efe-meta-swarm-participant-supplier-scoring     │
   │   args : ["participant"]                                      │
   │   env  : EFE_META_SWARM_PARTICIPANT_ID = "supplier-scoring"   │
   │          EFE_META_SWARM_PARTICIPANT_ADDR = "0.0.0.0:50091"    │
   └─────────┬────────────────────────────────────────────────────┘
             ▼
   ┌──────────────────────────────────────────────────────────────┐
   │ Service : efe-meta-swarm-participant-supplier-scoring         │
   │  DNS    : efe-meta-swarm-participant-supplier-scoring.efe.svc│
   └──────────────────────────────────────────────────────────────┘
                ▲
                │ (controller's EFE_META_SWARM_PARTICIPANTS env var
                │   templated by the chart with this DNS)
                │
   (× 3 participants : supplier-scoring, contract-risk, spend-categorization)

   ┌──────────────────────────────────────────────────────────────┐
   │ PodDisruptionBudget : efe-meta-swarm-controller-pdb           │
   │   maxUnavailable: 0                                           │
   └──────────────────────────────────────────────────────────────┘

   (optional resources, gated by feature flags)
   ┌──────────────────────────────────────────────────────────────┐
   │ ServiceMonitor : controller + participants    (opt-in)        │
   │  (rendered iff metrics.serviceMonitor.enabled = true)         │
   └──────────────────────────────────────────────────────────────┘
```

### 5.2 efe-server → meta-swarm connection (PR #11 hook)

```
   efe-server pod
   ├── env  EFE_META_SWARM_ENABLED               = "true"
   ├── env  EFE_META_SWARM_CONTROLLER_ENDPOINT   = "http://efe-meta-swarm-controller.efe.svc:50080"
   ├── env  EFE_META_SWARM_CONNECT_TIMEOUT_MS    = "3500"
   ├── env  EFE_META_SWARM_CALL_TIMEOUT_MS       = "15000"
   └── env  EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP = "true"
                                              │
                                              ▼
                              MetaSwarmService::Health probe at boot
                              ↓ accepts SERVING (1) and RELOADING (3)
                              boot succeeds, server starts serving
```

The parent `efe-server` chart's `templates/deployment.yaml` already
ships these env vars (PR #11) ; PR #13's umbrella values override
templates them when the sub-chart is enabled — cf. § 7.4.

### 5.3 SIGHUP-driven reload from outside the cluster (operator workflow)

```
   operator             kubectl exec                      controller pod
   │                       │                                  │
   │  edit values.yaml ; helm upgrade efe-stack -f ...        │
   │     │                                                     │
   │     ▼                                                     │
   │  ConfigMap manifest-* mutated (one-shot)                  │
   │                                                           │
   │  (Pod template did NOT change because no `checksum/config`│
   │   annotation, AD4) — pods are NOT restarted by k8s.       │
   │                                                           │
   │  kubectl exec deploy/efe-meta-swarm-controller -- kill -HUP 1
   │  ─────────────────────────────────────────────────────────►│
   │                                                  SIGHUP → │
   │                                              (PR #12 path)│
   │                                              apply_reload │
   │                                          _from_disk landed│
   │                                                           │
   │  kubectl exec deploy/efe-meta-swarm-participant-... -- kill -HUP 1
   │  ─────────────────────────────────────────────────────────►│
   │                                              participant   │
   │                                              SIGHUP        │
   │                                                           │
   │  grpcurl ... Health → reload_gen incremented              │
   │  ✓ operator confirms the reload landed                    │
```

---

## 6. Chart layout under `charts/efe-stack/charts/efe-meta-swarm/`

```
charts/efe-stack/charts/efe-meta-swarm/
├── Chart.yaml                                   [NEW : ~ 35 LOC]
├── values.yaml                                  [NEW : ~ 220 LOC, fully commented]
├── values.schema.json                           [NEW : ~ 230 LOC]
├── README.md                                    [NEW : ~ 180 LOC, operator-facing]
├── templates/
│   ├── _helpers.tpl                             [NEW : ~ 130 LOC, ~ 12 named templates]
│   ├── NOTES.txt                                [NEW : ~ 60 LOC, post-install hints]
│   ├── configmap-manifest.yaml                  [NEW : ~ 80 LOC]
│   ├── controller-deployment.yaml               [NEW : ~ 180 LOC]
│   ├── controller-service.yaml                  [NEW : ~ 35 LOC]
│   ├── controller-serviceaccount.yaml           [NEW : ~ 20 LOC]
│   ├── pdb-controller.yaml                      [NEW : ~ 25 LOC]
│   ├── participant-deployment.yaml              [NEW : ~ 200 LOC, ranges over .Values.participants]
│   ├── participant-service.yaml                 [NEW : ~ 45 LOC, ranges]
│   ├── participant-serviceaccount.yaml          [NEW : ~ 30 LOC, ranges]
│   ├── servicemonitor.yaml                      [NEW : ~ 60 LOC, optional]
│   ├── networkpolicy.yaml                       [NEW : ~ 100 LOC, optional]
│   └── tests/
│       └── connection-test.yaml                 [NEW : ~ 60 LOC, helm test hook]
└── tests/
    └── golden/
        ├── kovant-3p.yaml                       [NEW : ~ 900 LOC : full helm template render]
        ├── kovant-disabled.yaml                 [NEW : ~ 0 LOC : empty render]
        ├── kovant-network-policy.yaml           [NEW : ~ 1 000 LOC]
        └── kovant-service-monitor.yaml          [NEW : ~ 1 050 LOC]
```

Parent-chart wiring (`charts/efe-stack/`) :

```
charts/efe-stack/
├── Chart.yaml                                   [MODIFIED : +6 LOC, registers sub-chart]
└── values.yaml                                  [MODIFIED : +25 LOC, defaults block]
```

And the CI integration :

```
.github/workflows/helm.yml                       [MODIFIED : +30 LOC, adds golden-diff step]
```

Total : **17 new files + 3 modified ; +1 340 LOC net** (production
templates + values + tests).

---

## 7. `Chart.yaml` + parent dependency registration

### 7.1 `charts/efe-stack/charts/efe-meta-swarm/Chart.yaml`

```yaml
apiVersion: v2
name: efe-meta-swarm
description: |
  EFE Phase-2 meta-swarm — controller + N participants for the
  joint contractive iteration workhorse (Phase-1 `MetaSwarm::iterate`
  packaged for production).  Disabled by default ; flip
  `enabled: true` in `efe-meta-swarm:` of the umbrella values.yaml
  to bring up 1 controller + 3 participants (Kovant scenario).
type: application
version: 0.1.0
appVersion: "0.2.0"      # matches efe-meta-swarm-server crate version
kubeVersion: ">=1.28.0-0"
home: https://efe.example.com/meta-swarm
sources:
  - https://github.com/efe-ai/efe/tree/main/crates/efe-meta-swarm-server
maintainers:
  - name: EFE Meta-Swarm Team
    email: meta-swarm@efe.example.com
keywords:
  - meta-swarm
  - contractive-iteration
  - phase-2
  - reload-config
icon: https://efe.example.com/static/meta-swarm.png
annotations:
  artifacthub.io/license: UNLICENSED
  artifacthub.io/changes: |
    - kind: added
      description: Phase 2 PR #13 — initial sub-chart skeleton
```

### 7.2 `charts/efe-stack/Chart.yaml` (modified)

Append to the existing `dependencies:` block :

```yaml
  # Phase-2 meta-swarm controller + N participants.  Cf.
  # `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md`.  Disabled by default
  # so pre-Phase-2 deployments stay untouched.
  - name: efe-meta-swarm
    version: "0.1.0"
    repository: ""
    condition: efe-meta-swarm.enabled
```

### 7.3 `charts/efe-stack/values.yaml` (modified)

Append a `efe-meta-swarm:` block that flips the sub-chart off by default
and supplies the cross-chart wiring values that the parent
`efe-server` Deployment needs to know about :

```yaml
# Phase-2 meta-swarm subsystem (PR #13).  Off by default ;
# enable in environment-specific values overrides.
efe-meta-swarm:
  enabled: false
  # When enabled, the parent's efe-server chart picks up these
  # connection-time env vars (PR #11 hook).  Override per env.
  efeServerWiring:
    controllerServiceName: efe-meta-swarm-controller
    controllerServicePort: 50080
    connectTimeoutMs: 3500
    callTimeoutMs: 15000
    healthProbeAtStartup: true
```

### 7.4 Parent `efe-server` Deployment template hook (existing chart)

The existing `charts/efe-stack/charts/efe-server/templates/deployment.yaml`
needs a small augmentation to consume the `efeServerWiring` values
when the meta-swarm sub-chart is enabled.  PR #11 already wired the
`EFE_META_SWARM_*` env vars in the binary's config layer ; the chart
just templates them :

```yaml
{{- if (index .Values "efe-meta-swarm" "enabled") }}
- name: EFE_META_SWARM_ENABLED
  value: "true"
- name: EFE_META_SWARM_CONTROLLER_ENDPOINT
  value: "http://{{ (index .Values "efe-meta-swarm" "efeServerWiring" "controllerServiceName") }}.{{ .Release.Namespace }}.svc:{{ (index .Values "efe-meta-swarm" "efeServerWiring" "controllerServicePort") }}"
- name: EFE_META_SWARM_CONNECT_TIMEOUT_MS
  value: "{{ (index .Values "efe-meta-swarm" "efeServerWiring" "connectTimeoutMs") }}"
- name: EFE_META_SWARM_CALL_TIMEOUT_MS
  value: "{{ (index .Values "efe-meta-swarm" "efeServerWiring" "callTimeoutMs") }}"
- name: EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP
  value: "{{ (index .Values "efe-meta-swarm" "efeServerWiring" "healthProbeAtStartup") }}"
{{- end }}
```

(Helm sub-charts can read their **sibling** sub-chart's values via
`index .Values "<sibling>" "key"` — this is the standard pattern.
The umbrella's `efe-server.deployment` template is in
`charts/efe-stack/charts/efe-server/templates/deployment.yaml` ; PR #13
adds the block above.)

---

## 8. `values.yaml` schema (operator surface)

Full reference, fully commented :

```yaml
# ─────────────────────────────────────────────────────────────────────
# efe-meta-swarm — Phase-2 sub-chart values.
#
# This file is merged into the umbrella `efe-stack` values under key
# `efe-meta-swarm:`.  Every field is **typed and validated** by
# `values.schema.json` — `helm install --dry-run` will reject
# malformed configs before any K8s API call.
#
# Cf. `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md`.
# ─────────────────────────────────────────────────────────────────────

# Master switch.  When `false` (default), this sub-chart renders zero
# resources.  When `true`, the chart provisions 1 controller + N
# participants + the manifest ConfigMap + optional NetworkPolicy /
# ServiceMonitor.
enabled: false

# ─────────────────────────────────────────────────────────────────────
# Image — both binaries (controller + participant) ship in one image.
# Operators MUST pin the tag explicitly (values.schema.json rejects
# the empty string).  No `:latest` policy.
# ─────────────────────────────────────────────────────────────────────
image:
  repository: ghcr.io/efe-ai/efe-meta-swarm-server
  tag: ""                      # REQUIRED — operator override per release
  pullPolicy: IfNotPresent
  # imagePullSecrets are inherited from the umbrella `global.imagePullSecrets`.

# ─────────────────────────────────────────────────────────────────────
# Manifest — the YAML that the controller + every participant reads
# at boot AND on SIGHUP (PR #12).  Rendered as a single ConfigMap.
# Operators edit values here ; running pods pick it up via
# `kubectl exec ... -- kill -HUP 1`.
# ─────────────────────────────────────────────────────────────────────
manifest:
  version: "v0.2.0"
  meta:
    # Meta-space dim (Phase-1 `MetaSpace::dim`).
    dim: 16
    safety:
      kind: box                # box | poly | hull
      radius: 50.0             # Box only — distance from origin
      # poly:                  # Poly only — dim_a × dim flat constraint matrix
      #   dim_a: 2
      #   a_flat: [...]
      #   b: [...]
      # hull:                  # Hull only — Phase-3 placeholder
      #   version: "v1"
      #   serialized_model_path: "/etc/.../hull.bin"
  iteration:
    max_iters: 100
    tolerance: 1.0e-4
    alpha_default: 0.5
    coupling_gamma: 0.2
    deterministic: true
    max_wall_ms: 15000

# ─────────────────────────────────────────────────────────────────────
# Controller — single Deployment, replicas=1.  Phase-2 does NOT
# replicate the controller (no leader election yet — PR #17 follow-up).
# ─────────────────────────────────────────────────────────────────────
controller:
  # replicas: 1 — hardcoded for Phase 2 ; multi-controller is PR #17.
  # We expose this for forward-compat but values.schema.json caps it
  # at 1 (override locked).
  replicaCount: 1

  # gRPC bind + Prometheus metrics ports.  Exposed via ClusterIP.
  service:
    type: ClusterIP
    grpcPort: 50080
    metricsPort: 9091
    appProtocol: grpc          # L4 hint for service meshes (AD19)

  # Knobs forwarded as env vars to the binary (PR #6's
  # `ControllerConfig::from_env`).
  config:
    maxInflight: 10            # MAX_INFLIGHT (T7 mitigation)
    defaultMaxWallMs: 15000
    workerThreads: 2
    drainTimeoutSeconds: 30
    discovery: static-localhost   # PR #6 modes : static-localhost | k8s-dns
    reflectionEnabled: false   # gRPC reflection — PR #18 wires
    # TLS — Phase 2 placeholder ; PR #16 wires cert-manager.
    tlsRequired: false

  # K8s resource requests + limits.  Tuned for kovant-scale ;
  # operators override per environment.
  resources:
    requests:
      cpu: 500m
      memory: 512Mi
    limits:
      cpu: 2
      memory: 2Gi

  # SecurityContext per-container ; the pod-level securityContext is
  # in _helpers.tpl as a shared template.
  securityContext:
    allowPrivilegeEscalation: false
    capabilities:
      drop: ["ALL"]
    readOnlyRootFilesystem: true
    runAsNonRoot: true
    runAsUser: 65532

  # PodDisruptionBudget — Phase-2 always-protected (AD11).
  podDisruptionBudget:
    enabled: true
    maxUnavailable: 0

  # Pod placement.  Default : no node selector / affinity ; operators
  # add per-env taint tolerations.
  nodeSelector: {}
  tolerations: []
  affinity: {}

  # Liveness + readiness probes — TCP for Phase 2 (AD18).
  livenessProbe:
    tcpSocket:
      port: 50080
    initialDelaySeconds: 10
    periodSeconds: 10
    timeoutSeconds: 3
    failureThreshold: 3
  readinessProbe:
    tcpSocket:
      port: 50080
    initialDelaySeconds: 5
    periodSeconds: 5
    timeoutSeconds: 3
    failureThreshold: 3

  # Extra env vars merged into the container's env block.  Useful for
  # debug toggles without forcing a values override schema update.
  extraEnv: []

# ─────────────────────────────────────────────────────────────────────
# Participants — list of distinct binaries, one Deployment + Service
# each.  Default : 3-participant Kovant scenario.  Each entry's
# `id` MUST match an entry in `manifest.participants[*].id` (the
# values.schema.json enforces this cross-reference).
# ─────────────────────────────────────────────────────────────────────
participants:
  - id: supplier-scoring
    # Echoed into the manifest YAML's participants list :
    dim: 8
    kappa: 0.6
    seed: 11
    blasBackend: none
    # Service / pod knobs :
    replicaCount: 1            # locked to 1 in Phase 2 (schema-enforced)
    service:
      grpcPort: 50091
      metricsPort: 9101
    resources:
      requests: {cpu: 200m, memory: 256Mi}
      limits:   {cpu: 1,    memory: 1Gi}
    nodeSelector: {}
    tolerations: []
    affinity: {}
    extraEnv: []

  - id: contract-risk
    dim: 8
    kappa: 0.7
    seed: 22
    blasBackend: none
    replicaCount: 1
    service: {grpcPort: 50092, metricsPort: 9102}
    resources:
      requests: {cpu: 200m, memory: 256Mi}
      limits:   {cpu: 1,    memory: 1Gi}

  - id: spend-categorization
    dim: 8
    kappa: 0.5
    seed: 33
    blasBackend: none
    replicaCount: 1
    service: {grpcPort: 50093, metricsPort: 9103}
    resources:
      requests: {cpu: 200m, memory: 256Mi}
      limits:   {cpu: 1,    memory: 1Gi}

# ─────────────────────────────────────────────────────────────────────
# ServiceAccount — one SA per role.  Phase-2 RBAC : zero permissions
# (no K8s API access from the binaries).  PR #16 may grant `secrets:get`
# for cert-manager-issued TLS material.
# ─────────────────────────────────────────────────────────────────────
serviceAccount:
  create: true
  # annotations: {}            # operators may add IAM role bindings here
  # automountServiceAccountToken: false (set in template — Phase 2 default)

# ─────────────────────────────────────────────────────────────────────
# NetworkPolicy — deny-by-default east-west traffic between
# meta-swarm pods and everything else.  Opt-in (AD14).
# ─────────────────────────────────────────────────────────────────────
networkPolicy:
  enabled: false
  # Egress :
  #   * meta-swarm controller → meta-swarm participants
  #   * meta-swarm controller → metrics scraper (Prometheus pod)
  # Ingress :
  #   * efe-server → meta-swarm controller (gRPC port)
  #   * Prometheus → meta-swarm metrics ports
  # Override the allow-list if your scraper / ingress namespace differs.
  ingressNamespaceSelector:
    matchLabels:
      kubernetes.io/metadata.name: efe
  prometheusNamespaceSelector:
    matchLabels:
      kubernetes.io/metadata.name: monitoring

# ─────────────────────────────────────────────────────────────────────
# Prometheus ServiceMonitor — Prometheus-Operator integration.  Opt-in
# (requires `monitoring.coreos.com/v1` CRDs).
# ─────────────────────────────────────────────────────────────────────
metrics:
  serviceMonitor:
    enabled: false
    interval: 30s
    scrapeTimeout: 10s
    labels: {}                 # extra labels for the operator selector

# ─────────────────────────────────────────────────────────────────────
# Pod-level shared annotations (e.g. linkerd.io/inject, kuma.io/sidecar).
# Empty by default ; operators inject per service-mesh deployment.
# ─────────────────────────────────────────────────────────────────────
podAnnotations: {}

# ─────────────────────────────────────────────────────────────────────
# Helm test connection probe (`helm test`).  Renders a Pod that runs
# grpcurl against MetaSwarmService::Health.
# ─────────────────────────────────────────────────────────────────────
tests:
  enabled: true
  grpcurlImage: "fullstorydev/grpcurl:v1.9.1"
  timeoutSeconds: 30
```

### 8.1 Why every field is in `values.yaml` (not in `_helpers.tpl`)

Helm idioms : the operator surface is `values.yaml`.  Templates read
values via `.Values.*` and never carry their own defaults beyond
trivial fallbacks (`default 30 .Values.foo`).  This makes the schema
auditable in one file and `helm install --dry-run -f override.yaml`
predictable.

---

## 9. `values.schema.json` (JSON Schema validation rules)

Per AD16, this file is shipped at chart root and consumed by Helm 3's
built-in schema validation (`helm install` rejects on violation
before any K8s API call).

Key constraints :

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://efe.example.com/schemas/efe-meta-swarm.json",
  "title": "efe-meta-swarm sub-chart values",
  "type": "object",
  "required": ["enabled"],
  "additionalProperties": false,
  "properties": {
    "enabled": { "type": "boolean" },
    "image": {
      "type": "object",
      "required": ["repository", "tag", "pullPolicy"],
      "additionalProperties": false,
      "properties": {
        "repository": {
          "type": "string",
          "minLength": 1,
          "pattern": "^[a-z0-9][a-z0-9._/-]*$"
        },
        "tag": {
          "type": "string",
          "minLength": 1,
          "description": "MUST be set (no :latest policy ; AD17)"
        },
        "pullPolicy": {
          "type": "string",
          "enum": ["Always", "IfNotPresent", "Never"]
        }
      }
    },
    "manifest": {
      "type": "object",
      "required": ["version", "meta", "iteration"],
      "additionalProperties": false,
      "properties": {
        "version": {
          "type": "string",
          "pattern": "^v[0-9]+\\.[0-9]+\\.[0-9]+(?:-[a-z0-9]+)?$"
        },
        "meta": {
          "type": "object",
          "required": ["dim", "safety"],
          "additionalProperties": false,
          "properties": {
            "dim": { "type": "integer", "minimum": 1, "maximum": 8192 },
            "safety": {
              "type": "object",
              "required": ["kind"],
              "properties": {
                "kind": {
                  "type": "string",
                  "enum": ["box", "poly", "hull"]
                },
                "radius":      { "type": "number", "exclusiveMinimum": 0 },
                "poly": {
                  "type": "object",
                  "required": ["dim_a", "a_flat", "b"],
                  "properties": {
                    "dim_a":  { "type": "integer", "minimum": 1 },
                    "a_flat": { "type": "array", "items": { "type": "number" } },
                    "b":      { "type": "array", "items": { "type": "number" } }
                  }
                },
                "hull": {
                  "type": "object",
                  "required": ["version", "serialized_model_path"],
                  "properties": {
                    "version":               { "type": "string" },
                    "serialized_model_path": { "type": "string" }
                  }
                }
              },
              "allOf": [
                {
                  "if":   { "properties": { "kind": { "const": "box" }}},
                  "then": { "required": ["radius"] }
                },
                {
                  "if":   { "properties": { "kind": { "const": "poly" }}},
                  "then": { "required": ["poly"] }
                },
                {
                  "if":   { "properties": { "kind": { "const": "hull" }}},
                  "then": { "required": ["hull"] }
                }
              ]
            }
          }
        },
        "iteration": {
          "type": "object",
          "required": [
            "max_iters", "tolerance", "alpha_default",
            "coupling_gamma", "deterministic", "max_wall_ms"
          ],
          "additionalProperties": false,
          "properties": {
            "max_iters":      { "type": "integer", "minimum": 1, "maximum": 200 },
            "tolerance":      { "type": "number",  "exclusiveMinimum": 0 },
            "alpha_default":  { "type": "number",  "minimum": 0, "exclusiveMaximum": 1 },
            "coupling_gamma": { "type": "number",  "minimum": 0, "exclusiveMaximum": 1 },
            "deterministic":  { "type": "boolean" },
            "max_wall_ms":    { "type": "integer", "minimum": 1, "maximum": 30000 }
          }
        }
      }
    },
    "controller": { "$ref": "#/$defs/controllerSpec" },
    "participants": {
      "type": "array",
      "minItems": 1,
      "maxItems": 64,
      "items":   { "$ref": "#/$defs/participantSpec" }
    },
    "serviceAccount":  { "$ref": "#/$defs/serviceAccountSpec" },
    "networkPolicy":   { "$ref": "#/$defs/networkPolicySpec" },
    "metrics":         { "$ref": "#/$defs/metricsSpec" },
    "podAnnotations":  { "type": "object" },
    "tests":           { "$ref": "#/$defs/testsSpec" }
  },
  "$defs": {
    "controllerSpec": {
      "type": "object",
      "properties": {
        "replicaCount": { "type": "integer", "const": 1, "description": "Phase-2 single-controller ; multi-controller HA in PR #17" },
        ".../"
      }
    },
    "participantSpec": {
      "type": "object",
      "required": ["id", "dim", "kappa", "seed"],
      "properties": {
        "id":           { "type": "string", "pattern": "^[a-z][a-z0-9-]{0,62}$" },
        "dim":          { "type": "integer", "minimum": 1, "maximum": 8192 },
        "kappa":        { "type": "number",  "exclusiveMinimum": 0, "exclusiveMaximum": 1 },
        "seed":         { "type": "integer", "minimum": 0 },
        "blasBackend":  { "type": "string", "enum": ["none", "openblas", "mkl", "accelerate"] },
        "replicaCount": { "type": "integer", "const": 1 }
      }
    }
  }
}
```

### 9.1 Cross-reference invariants enforced by `values.schema.json`

  * `image.tag` is non-empty (AD17).
  * Every participant id matches `^[a-z][a-z0-9-]{0,62}$` (DNS-compliant ; same constraint as Service names).
  * `kappa ∈ (0, 1)` — Phase-1's contraction constant range.
  * `controller.replicaCount == 1` (Phase-2 lock).
  * `participants[*].replicaCount == 1` (Phase-2 lock — sharded
    participants are Phase 3).
  * `iteration.max_iters ≤ 200` matches PR #8's `MAX_ITERS_HARD_CAP`.
  * `iteration.max_wall_ms ≤ 30000` matches PR #8's `MAX_WALL_MS_HARD_CAP`.
  * `safety.kind ∈ {box, poly, hull}` — Phase-1 enum.

What **cannot** be enforced by JSON schema (deferred to runtime
validation) :

  * Every participant id in `participants[*]` is also present in
    `manifest.participants[*].id`.  (JSON schema does not have
    cross-property uniqueness across two arrays.)
  * The boot-time `validate_manifest` (PR #8) re-runs the same checks
    on the rendered manifest — defence in depth.

---

## 10. `_helpers.tpl` (named templates)

Twelve helpers, each scoped to the sub-chart's naming :

```tpl
{{/*
  efe-meta-swarm.name — short chart name (max 63 chars, DNS-compliant).
*/}}
{{- define "efe-meta-swarm.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.fullname — release-prefixed, max 63 chars.
*/}}
{{- define "efe-meta-swarm.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{/*
  efe-meta-swarm.labels — full label set + chart version + tenant.
*/}}
{{- define "efe-meta-swarm.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "efe-meta-swarm.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe
efe.io/tenant: {{ .Values.global.tenant | default "default" | quote }}
{{- end -}}

{{/*
  efe-meta-swarm.selectorLabels — used in Deployment.spec.selector
  AND Service.spec.selector ; MUST be identical for pod targeting.
*/}}
{{- define "efe-meta-swarm.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-meta-swarm.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
  efe-meta-swarm.controllerName — Deployment / Service name for the
  controller.
*/}}
{{- define "efe-meta-swarm.controllerName" -}}
{{ include "efe-meta-swarm.fullname" . }}-controller
{{- end -}}

{{/*
  efe-meta-swarm.participantName — name of the Deployment / Service
  for a given participant id.  Caller is expected to set $participant
  in the surrounding scope (cf. participant-deployment.yaml `range`).
*/}}
{{- define "efe-meta-swarm.participantName" -}}
{{- $root := index . 0 -}}
{{- $id := index . 1 -}}
{{ include "efe-meta-swarm.fullname" $root }}-participant-{{ $id }}
{{- end -}}

{{/*
  efe-meta-swarm.serviceAccountName — operator-overridable, defaults
  to fullname-<role>.
*/}}
{{- define "efe-meta-swarm.controllerServiceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (printf "%s-controller" (include "efe-meta-swarm.fullname" .)) .Values.serviceAccount.controllerName -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.controllerName -}}
{{- end -}}
{{- end -}}

{{- define "efe-meta-swarm.participantServiceAccountName" -}}
{{- $root := index . 0 -}}
{{- $id := index . 1 -}}
{{- if $root.Values.serviceAccount.create -}}
{{ include "efe-meta-swarm.fullname" $root }}-participant-{{ $id }}
{{- else -}}
default
{{- end -}}
{{- end -}}

{{/*
  efe-meta-swarm.image — fully qualified image ref ; rejects empty tag
  (defence in depth ; the schema also enforces it).
*/}}
{{- define "efe-meta-swarm.image" -}}
{{- $tag := .Values.image.tag -}}
{{- if empty $tag -}}
{{- fail "efe-meta-swarm: .Values.image.tag MUST be set (no :latest policy ; AD17)" -}}
{{- end -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}

{{/*
  efe-meta-swarm.participantsEnv — comma-separated env value for the
  controller's `EFE_META_SWARM_PARTICIPANTS` ; renders
  `id=svc.ns.svc:port,id=svc.ns.svc:port,...` from .Values.participants.
*/}}
{{- define "efe-meta-swarm.participantsEnv" -}}
{{- $root := . -}}
{{- $entries := list -}}
{{- range .Values.participants -}}
{{- $svc := printf "%s-participant-%s" (include "efe-meta-swarm.fullname" $root) .id -}}
{{- $entry := printf "%s=%s.%s.svc:%d" .id $svc $root.Release.Namespace (int .service.grpcPort) -}}
{{- $entries = append $entries $entry -}}
{{- end -}}
{{- join "," $entries -}}
{{- end -}}

{{/*
  efe-meta-swarm.manifestYaml — the rendered Phase-2 manifest YAML body.
  Consumed both by configmap-manifest.yaml and (in tests) by the
  golden-snapshot tooling for round-tripping.
*/}}
{{- define "efe-meta-swarm.manifestYaml" -}}
version: {{ .Values.manifest.version | quote }}
meta:
  dim: {{ .Values.manifest.meta.dim }}
  safety:
    kind: {{ .Values.manifest.meta.safety.kind }}
{{- if eq .Values.manifest.meta.safety.kind "box" }}
    radius: {{ .Values.manifest.meta.safety.radius }}
{{- else if eq .Values.manifest.meta.safety.kind "poly" }}
    dim_a: {{ .Values.manifest.meta.safety.poly.dim_a }}
    a_flat: {{ .Values.manifest.meta.safety.poly.a_flat | toJson }}
    b:      {{ .Values.manifest.meta.safety.poly.b | toJson }}
{{- else if eq .Values.manifest.meta.safety.kind "hull" }}
    version: {{ .Values.manifest.meta.safety.hull.version | quote }}
    serialized_model_path: {{ .Values.manifest.meta.safety.hull.serialized_model_path | quote }}
{{- end }}
participants:
{{- range .Values.participants }}
  - id: {{ .id }}
    dim: {{ .dim }}
    kappa: {{ .kappa }}
    seed: {{ .seed }}
    blas_backend: {{ .blasBackend | quote }}
{{- end }}
iteration:
  max_iters:      {{ .Values.manifest.iteration.max_iters }}
  tolerance:      {{ .Values.manifest.iteration.tolerance }}
  alpha_default:  {{ .Values.manifest.iteration.alpha_default }}
  coupling_gamma: {{ .Values.manifest.iteration.coupling_gamma }}
  deterministic:  {{ .Values.manifest.iteration.deterministic }}
  max_wall_ms:    {{ .Values.manifest.iteration.max_wall_ms }}
{{- end -}}
```

### 10.1 Why `manifestYaml` is a named template (not inline in `configmap-manifest.yaml`)

  * **Reusable in tests** : the golden-snapshot tooling renders this
    template alone and runs the rendered string through Rust's
    `manifest::load_and_validate` (PR #8) to assert round-trip
    correctness.
  * **Single source of truth** : if the manifest schema changes (e.g.
    new safety variant), only one place to update.

---

## 11. `configmap-manifest.yaml` (manifest YAML rendering)

```yaml
{{- if .Values.enabled }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-manifest
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    efe.io/meta-swarm-role: shared
  annotations:
    efe.io/manifest-schema-version: {{ .Values.manifest.version | quote }}
data:
  # The whole Phase-2 manifest as a single YAML key.  Both the
  # controller and every participant mount this ConfigMap at
  # `/etc/efe-meta-swarm/manifest.yaml` and set
  # `EFE_META_SWARM_MANIFEST=/etc/efe-meta-swarm/manifest.yaml`.
  #
  # SIGHUP-driven reload (PR #12) re-reads from this path ; the
  # operator's edit-then-`helm upgrade`-then-`kill -HUP` workflow is
  # documented in the runbook (§ 41).
  manifest.yaml: |-
{{ include "efe-meta-swarm.manifestYaml" . | indent 4 }}
{{- end }}
```

### 11.1 Why one ConfigMap with one YAML key

  * Matches PR #6's `EFE_META_SWARM_MANIFEST=<path>` convention.
  * Atomic edits via `kubectl apply` or `helm upgrade` swap the whole
    file (no partial reads).
  * Operators can also `kubectl edit configmap ...` for emergency
    surgery, then SIGHUP the pods.

### 11.2 Why no `checksum/config` annotation propagation

AD4 : the operator owns the reload trigger.  `helm upgrade` swaps
the ConfigMap content ; the Pod templates do **not** carry a
content-hash annotation, so the `RollingUpdate` controller does not
restart pods on ConfigMap edit.  Reload is explicit via `kill -HUP`.

This is **opposite** to the parent `efe-server` chart's policy
(which DOES use `checksum/config`) because `efe-server` doesn't have
a SIGHUP path — it relies on rolling restart for config changes.

---

## 12. `controller-deployment.yaml`

```yaml
{{- if .Values.enabled }}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
    efe.io/meta-swarm-role: controller
spec:
  replicas: {{ .Values.controller.replicaCount | default 1 }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 0
      maxUnavailable: 1        # single-replica Deployment ; rolling restart drops the controller briefly (PR12-R7 concern — documented)
  selector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: controller
  template:
    metadata:
      labels:
        {{- include "efe-meta-swarm.selectorLabels" . | nindent 8 }}
        app.kubernetes.io/component: controller
        efe.io/meta-swarm-role: controller
      annotations:
        # PR #13 AD4 : NO `checksum/config` ; reload is SIGHUP-driven.
        prometheus.io/scrape: "true"
        prometheus.io/port: "{{ .Values.controller.service.metricsPort }}"
        prometheus.io/path: "/metrics"
        {{- with .Values.podAnnotations }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
    spec:
      serviceAccountName: {{ include "efe-meta-swarm.controllerServiceAccountName" . }}
      automountServiceAccountToken: false
      {{- with .Values.global.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        runAsGroup: 65532
        fsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: controller
          image: {{ include "efe-meta-swarm.image" . | quote }}
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          args: ["controller"]
          ports:
            - name: grpc
              containerPort: {{ .Values.controller.service.grpcPort }}
              protocol: TCP
            - name: metrics
              containerPort: {{ .Values.controller.service.metricsPort }}
              protocol: TCP
          env:
            - name: EFE_META_SWARM_CONTROLLER_ADDR
              value: "0.0.0.0:{{ .Values.controller.service.grpcPort }}"
            - name: EFE_META_SWARM_CONTROLLER_METRICS
              value: "0.0.0.0:{{ .Values.controller.service.metricsPort }}"
            - name: EFE_META_SWARM_MANIFEST
              value: /etc/efe-meta-swarm/manifest.yaml
            - name: EFE_META_SWARM_PARTICIPANTS
              value: {{ include "efe-meta-swarm.participantsEnv" . | quote }}
            - name: EFE_META_SWARM_DISCOVERY
              value: {{ .Values.controller.config.discovery | quote }}
            - name: EFE_META_SWARM_MAX_INFLIGHT
              value: {{ .Values.controller.config.maxInflight | quote }}
            - name: EFE_META_SWARM_DEFAULT_MAX_WALL_MS
              value: {{ .Values.controller.config.defaultMaxWallMs | quote }}
            - name: EFE_META_SWARM_WORKER_THREADS
              value: {{ .Values.controller.config.workerThreads | quote }}
            - name: EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S
              value: {{ .Values.controller.config.drainTimeoutSeconds | quote }}
            - name: EFE_META_SWARM_REFLECTION
              value: {{ .Values.controller.config.reflectionEnabled | quote }}
            - name: EFE_META_SWARM_TLS_REQUIRED
              value: {{ .Values.controller.config.tlsRequired | quote }}
            {{- with .Values.controller.extraEnv }}
            {{- toYaml . | nindent 12 }}
            {{- end }}
          resources:
            {{- toYaml .Values.controller.resources | nindent 12 }}
          securityContext:
            {{- toYaml .Values.controller.securityContext | nindent 12 }}
          livenessProbe:
            {{- toYaml .Values.controller.livenessProbe | nindent 12 }}
          readinessProbe:
            {{- toYaml .Values.controller.readinessProbe | nindent 12 }}
          volumeMounts:
            - name: manifest
              mountPath: /etc/efe-meta-swarm
              readOnly: true
      volumes:
        - name: manifest
          configMap:
            name: {{ include "efe-meta-swarm.fullname" . }}-manifest
      {{- with .Values.controller.nodeSelector }}
      nodeSelector:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.controller.tolerations }}
      tolerations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.controller.affinity }}
      affinity:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      terminationGracePeriodSeconds: {{ add (int .Values.controller.config.drainTimeoutSeconds) 5 }}
{{- end }}
```

### 12.1 Notes on the controller Deployment

  * `args: ["controller"]` — the binary's `Cargo.toml` declares two
    `[[bin]]` entries ; the image's ENTRYPOINT is either the package's
    default binary OR `["/usr/local/bin/entrypoint"]` that dispatches
    on argv (defined in the Dockerfile, NOT in PR #13 — this is a
    PR #6 / PR #10 image-build concern).
  * `terminationGracePeriodSeconds` = drain timeout + 5 s slack ;
    matches `EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S` + the binary's
    own SIGTERM drain path (PR #6).
  * No `lifecycle.preStop` hook — the binary's SIGTERM handler
    (`shutdown::await_termination`, PR #6) handles graceful drain
    natively.
  * `volumes.manifest` mounts the ConfigMap read-only ; the binary
    NEVER writes to the manifest path.

---

## 13. `controller-service.yaml` + headless DNS

```yaml
{{- if .Values.enabled }}
apiVersion: v1
kind: Service
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
spec:
  type: {{ .Values.controller.service.type }}
  ports:
    - name: grpc
      port: {{ .Values.controller.service.grpcPort }}
      targetPort: grpc
      protocol: TCP
      appProtocol: {{ .Values.controller.service.appProtocol | quote }}
    - name: metrics
      port: {{ .Values.controller.service.metricsPort }}
      targetPort: metrics
      protocol: TCP
  selector:
    {{- include "efe-meta-swarm.selectorLabels" . | nindent 4 }}
    app.kubernetes.io/component: controller
{{- end }}
```

### 13.1 DNS contract

The Service's FQDN is :
```
<release>-efe-meta-swarm-controller.<namespace>.svc.cluster.local
```

(or just `<svc>.<ns>.svc` for in-namespace clients).

The parent `efe-server` Deployment's `EFE_META_SWARM_CONTROLLER_ENDPOINT`
env var is templated to this FQDN (§ 7.4).

---

## 14. `participant-deployment.yaml` (`range` over participants)

```yaml
{{- if .Values.enabled }}
{{- range $p := .Values.participants }}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}
  labels:
    {{- include "efe-meta-swarm.labels" $ | nindent 4 }}
    app.kubernetes.io/component: participant
    efe.io/meta-swarm-role: participant
    efe.io/participant-id: {{ $p.id | quote }}
spec:
  replicas: {{ $p.replicaCount | default 1 }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 0
      maxUnavailable: 1
  selector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" $ | nindent 6 }}
      efe.io/participant-id: {{ $p.id | quote }}
  template:
    metadata:
      labels:
        {{- include "efe-meta-swarm.selectorLabels" $ | nindent 8 }}
        app.kubernetes.io/component: participant
        efe.io/meta-swarm-role: participant
        efe.io/participant-id: {{ $p.id | quote }}
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "{{ $p.service.metricsPort }}"
        prometheus.io/path: "/metrics"
        {{- with $.Values.podAnnotations }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
    spec:
      serviceAccountName: {{ include "efe-meta-swarm.participantServiceAccountName" (list $ $p.id) }}
      automountServiceAccountToken: false
      {{- with $.Values.global.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        runAsGroup: 65532
        fsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: participant
          image: {{ include "efe-meta-swarm.image" $ | quote }}
          imagePullPolicy: {{ $.Values.image.pullPolicy }}
          args: ["participant"]
          ports:
            - name: grpc
              containerPort: {{ $p.service.grpcPort }}
              protocol: TCP
            - name: metrics
              containerPort: {{ $p.service.metricsPort }}
              protocol: TCP
          env:
            - name: EFE_META_SWARM_PARTICIPANT_ID
              value: {{ $p.id | quote }}
            - name: EFE_META_SWARM_PARTICIPANT_ADDR
              value: "0.0.0.0:{{ $p.service.grpcPort }}"
            - name: EFE_META_SWARM_PARTICIPANT_METRICS
              value: "0.0.0.0:{{ $p.service.metricsPort }}"
            - name: EFE_META_SWARM_MANIFEST
              value: /etc/efe-meta-swarm/manifest.yaml
            - name: EFE_META_SWARM_WORKER_THREADS
              value: "1"
            {{- with $p.extraEnv }}
            {{- toYaml . | nindent 12 }}
            {{- end }}
          resources:
            {{- toYaml $p.resources | nindent 12 }}
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            readOnlyRootFilesystem: true
            runAsNonRoot: true
            runAsUser: 65532
          livenessProbe:
            tcpSocket:
              port: {{ $p.service.grpcPort }}
            initialDelaySeconds: 10
            periodSeconds: 10
            timeoutSeconds: 3
            failureThreshold: 3
          readinessProbe:
            tcpSocket:
              port: {{ $p.service.grpcPort }}
            initialDelaySeconds: 5
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 3
          volumeMounts:
            - name: manifest
              mountPath: /etc/efe-meta-swarm
              readOnly: true
      volumes:
        - name: manifest
          configMap:
            name: {{ include "efe-meta-swarm.fullname" $ }}-manifest
      {{- with $p.nodeSelector }}
      nodeSelector:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $p.tolerations }}
      tolerations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $p.affinity }}
      affinity:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      terminationGracePeriodSeconds: 30
{{- end }}
{{- end }}
```

### 14.1 Why per-participant Deployment + per-id label

  * Each participant is a *distinct* binary instance with its own
    `EFE_META_SWARM_PARTICIPANT_ID`.  A single Deployment with
    `replicas: 3` would have all pods share the same env var.
  * The `efe.io/participant-id` label lets operators
    `kubectl get pods -l efe.io/participant-id=supplier-scoring`.

---

## 15. `participant-service.yaml`

```yaml
{{- if .Values.enabled }}
{{- range $p := .Values.participants }}
---
apiVersion: v1
kind: Service
metadata:
  name: {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}
  labels:
    {{- include "efe-meta-swarm.labels" $ | nindent 4 }}
    app.kubernetes.io/component: participant
    efe.io/meta-swarm-role: participant
    efe.io/participant-id: {{ $p.id | quote }}
spec:
  type: ClusterIP
  ports:
    - name: grpc
      port: {{ $p.service.grpcPort }}
      targetPort: grpc
      protocol: TCP
      appProtocol: grpc
    - name: metrics
      port: {{ $p.service.metricsPort }}
      targetPort: metrics
      protocol: TCP
  selector:
    {{- include "efe-meta-swarm.selectorLabels" $ | nindent 4 }}
    efe.io/participant-id: {{ $p.id | quote }}
{{- end }}
{{- end }}
```

---

## 16. `serviceaccount.yaml` + RBAC (Phase-2 minimal surface)

```yaml
{{- if and .Values.enabled .Values.serviceAccount.create }}
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "efe-meta-swarm.controllerServiceAccountName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
    efe.io/meta-swarm-role: controller
  {{- with .Values.serviceAccount.annotations }}
  annotations:
    {{- toYaml . | nindent 4 }}
  {{- end }}
automountServiceAccountToken: false

{{- range $p := .Values.participants }}
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "efe-meta-swarm.participantServiceAccountName" (list $ $p.id) }}
  labels:
    {{- include "efe-meta-swarm.labels" $ | nindent 4 }}
    app.kubernetes.io/component: participant
    efe.io/meta-swarm-role: participant
    efe.io/participant-id: {{ $p.id | quote }}
automountServiceAccountToken: false
{{- end }}
{{- end }}
```

### 16.1 Why no `Role` / `RoleBinding`

Phase-2 binaries make zero K8s API calls.  `automountServiceAccountToken:
false` is the runtime-level guarantee ; no RBAC = no privileges.
PR #16 (mTLS) may grant `secrets:get` for cert-manager-issued certs ;
PR #18 (in-cluster watchers) may grant `configmaps:watch`.

---

## 17. `pdb-controller.yaml` (PodDisruptionBudget)

```yaml
{{- if and .Values.enabled .Values.controller.podDisruptionBudget.enabled }}
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}-pdb
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
spec:
  maxUnavailable: {{ .Values.controller.podDisruptionBudget.maxUnavailable }}
  selector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: controller
{{- end }}
```

---

## 18. `servicemonitor.yaml` (Prometheus Operator, optional)

```yaml
{{- if and .Values.enabled .Values.metrics.serviceMonitor.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    {{- with .Values.metrics.serviceMonitor.labels }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
spec:
  selector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
  namespaceSelector:
    matchNames:
      - {{ .Release.Namespace }}
  endpoints:
    - port: metrics
      path: /metrics
      interval: {{ .Values.metrics.serviceMonitor.interval }}
      scrapeTimeout: {{ .Values.metrics.serviceMonitor.scrapeTimeout }}
      honorLabels: true
      relabelings:
        - sourceLabels: [__meta_kubernetes_pod_label_efe_io_meta_swarm_role]
          targetLabel: role
        - sourceLabels: [__meta_kubernetes_pod_label_efe_io_participant_id]
          targetLabel: participant_id
{{- end }}
```

### 18.1 Scraped metrics surface (PR #11 / #12 / #17)

  * `efe_meta_swarm_health_probes_total` (PR #11)
  * `efe_meta_swarm_health_probe_errors_total` (PR #11)
  * `efe_meta_swarm_health_probe_duration_seconds` (PR #11)
  * `efe_meta_swarm_reload_attempts_total` (PR #12)
  * `efe_meta_swarm_reload_failures_total{reason}` (PR #12)
  * `efe_meta_swarm_reload_duration_microseconds` (PR #12)
  * `efe_meta_swarm_reload_gen` (PR #12)
  * (Phase 3) `efe_meta_swarm_iterate_*` (PR #17 wires)

---

## 19. `networkpolicy.yaml` (deny-by-default, optional)

```yaml
{{- if and .Values.enabled .Values.networkPolicy.enabled }}
---
# Default-deny ingress + egress on every meta-swarm pod.  Specific
# allow-lists below explicitly permit the legitimate traffic.
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-deny-all
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
  policyTypes: [Ingress, Egress]

---
# Allow efe-server → controller (gRPC) and Prometheus → metrics ports.
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}-allow-ingress
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: controller
  policyTypes: [Ingress]
  ingress:
    - from:
        - namespaceSelector:
            {{- toYaml .Values.networkPolicy.ingressNamespaceSelector | nindent 12 }}
          podSelector:
            matchLabels:
              app.kubernetes.io/name: efe-server
      ports:
        - protocol: TCP
          port: {{ .Values.controller.service.grpcPort }}
    - from:
        - namespaceSelector:
            {{- toYaml .Values.networkPolicy.prometheusNamespaceSelector | nindent 12 }}
      ports:
        - protocol: TCP
          port: {{ .Values.controller.service.metricsPort }}

---
# Allow controller → participant (gRPC fan-out).
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-controller-egress
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: controller
  policyTypes: [Egress]
  egress:
    - to:
        - podSelector:
            matchLabels:
              {{- include "efe-meta-swarm.selectorLabels" . | nindent 14 }}
              app.kubernetes.io/component: participant
      # Permit all participant gRPC ports (range).
      ports:
        {{- range $p := .Values.participants }}
        - protocol: TCP
          port: {{ $p.service.grpcPort }}
        {{- end }}
    # Allow DNS lookups (kube-dns is in kube-system by convention).
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
          podSelector:
            matchLabels:
              k8s-app: kube-dns
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53

---
# Allow participant ingress from controller + Prometheus.
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-participants-allow-ingress
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: participant
  policyTypes: [Ingress]
  ingress:
    - from:
        - podSelector:
            matchLabels:
              {{- include "efe-meta-swarm.selectorLabels" . | nindent 14 }}
              app.kubernetes.io/component: controller
      ports:
        {{- range $p := .Values.participants }}
        - protocol: TCP
          port: {{ $p.service.grpcPort }}
        {{- end }}
    - from:
        - namespaceSelector:
            {{- toYaml .Values.networkPolicy.prometheusNamespaceSelector | nindent 12 }}
      ports:
        {{- range $p := .Values.participants }}
        - protocol: TCP
          port: {{ $p.service.metricsPort }}
        {{- end }}
{{- end }}
```

### 19.1 Why opt-in

NetworkPolicy enforcement requires a CNI plug-in that supports it
(Calico, Cilium, Weave with policy).  On clusters with a non-policy
CNI (Flannel default), the resources render but have no effect ; the
operator may not realise.  Default-off makes the operator opt-in
consciously.

---

## 20. `templates/tests/connection-test.yaml` (`helm test` hook)

```yaml
{{- if and .Values.enabled .Values.tests.enabled }}
apiVersion: v1
kind: Pod
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-test-connection
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: test
  annotations:
    "helm.sh/hook": test
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  restartPolicy: Never
  activeDeadlineSeconds: {{ .Values.tests.timeoutSeconds }}
  serviceAccountName: default
  automountServiceAccountToken: false
  containers:
    - name: grpcurl
      image: {{ .Values.tests.grpcurlImage | quote }}
      imagePullPolicy: IfNotPresent
      command:
        - sh
        - -c
        - |
          set -eu
          ENDPOINT="{{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc:{{ .Values.controller.service.grpcPort }}"
          echo "Probing $ENDPOINT ..."
          # Use grpcurl reflection ; fallback to method-by-name if
          # reflection is disabled.
          grpcurl -plaintext -d '{}' -import-path /protos \
            -max-time {{ sub .Values.tests.timeoutSeconds 5 }} \
            "$ENDPOINT" \
            efe.v1.meta_swarm.MetaSwarmService/Health
{{- end }}
```

Operators run `helm test efe-stack -n efe` after install.

### 20.1 Why a Pod (not a Job)

  * `Job` semantics retry on failure ; `Pod` with `restartPolicy: Never`
    fails-fast — that's what we want for `helm test`.
  * `helm test` deletes the Pod automatically on success (via
    `hook-delete-policy`).

---

## 21. `NOTES.txt` + `README.md`

### 21.1 `NOTES.txt`

Rendered after `helm install` ; gives the operator the immediate
post-install steps.

```
{{- if .Values.enabled }}
═════════════════════════════════════════════════════════════════════
EFE-A1 Phase 2 meta-swarm — installed.
═════════════════════════════════════════════════════════════════════

Release   : {{ .Release.Name }}
Namespace : {{ .Release.Namespace }}
Topology  : 1× controller + {{ len .Values.participants }} participants

Controller endpoint (in-cluster) :
    http://{{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc:{{ .Values.controller.service.grpcPort }}

Participants :
{{- range $p := .Values.participants }}
    {{ $p.id }} → {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}.{{ $.Release.Namespace }}.svc:{{ $p.service.grpcPort }}
{{- end }}

Next steps :

  1. Verify pods came up :
       kubectl -n {{ .Release.Namespace }} get pods -l efe.io/meta-swarm-role

  2. Run the `helm test` connection probe :
       helm test {{ .Release.Name }} -n {{ .Release.Namespace }}

  3. SIGHUP-driven reload after a manifest edit
     (PR #12 hot reload — no pod restart) :
       helm upgrade {{ .Release.Name }} ./charts/efe-stack \
         -f values-prod.yaml
       kubectl -n {{ .Release.Namespace }} exec \
         deploy/{{ include "efe-meta-swarm.controllerName" . }} -- kill -HUP 1
{{- range $p := .Values.participants }}
       kubectl -n {{ $.Release.Namespace }} exec \
         deploy/{{ include "efe-meta-swarm.participantName" (list $ $p.id) }} -- kill -HUP 1
{{- end }}

  4. Operator runbook + threat model :
       docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md   (this PR)
       docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md   (SIGHUP semantics)
{{- end }}
```

### 21.2 `README.md`

Operator-facing.  ~ 180 LOC covering :

  * What the sub-chart provides
  * Default vs production overrides
  * `values.yaml` reference (top-level keys + types)
  * `helm install` / `helm upgrade` / `helm rollback` examples
  * SIGHUP reload workflow
  * Troubleshooting (Pod not Ready, gRPC connection refused, RELOADING stuck)
  * Cross-reference to threat model + PR #12 reload semantics

---

## 22. Image management — registry, tag pinning, pullSecrets

  * `image.repository` defaults to `ghcr.io/efe-ai/efe-meta-swarm-server`
    (placeholder — the production registry will be the EFE-private
    one ; the chart's default is configurable).
  * `image.tag` defaults to `""` — `values.schema.json` rejects, so
    operators MUST set it.  No `:latest` policy.
  * `imagePullSecrets` inherits from umbrella `global.imagePullSecrets`
    (already set up by sibling sub-charts).
  * `image.pullPolicy: IfNotPresent` matches existing conventions.

### 22.1 Image digest pinning (Phase 3 follow-up)

Once cosign / Sigstore is in the CI pipeline, the chart's helper
template can be enhanced to template `image: ghcr.io/.../efe-meta-swarm-server@sha256:...`
instead of `image: ...:tag` — matching the Phase-3 supply-chain story.

---

## 23. SecurityContext / hardening

Per-pod :
```yaml
runAsNonRoot:  true
runAsUser:     65532
runAsGroup:    65532
fsGroup:       65532
seccompProfile:
  type: RuntimeDefault
```

Per-container :
```yaml
allowPrivilegeEscalation: false
capabilities:
  drop: ["ALL"]
readOnlyRootFilesystem: true
runAsNonRoot: true
runAsUser:    65532
```

### 23.1 Why `readOnlyRootFilesystem: true`

The binaries don't write to disk — manifest is mounted RO, logs go to
stdout, metrics are served from memory.  Locking the rootfs prevents
post-exploitation persistence.  The only writable path is the
ConfigMap mount (RO at the mount level too).

If a future binary needs scratch space, add an `emptyDir` volume with
`medium: Memory` and a specific mount path.

---

## 24. Resource sizing + autoscaling deferral

| Role | Requests | Limits | Rationale |
|---|---|---|---|
| Controller | 500m / 512Mi | 2 / 2Gi | MAX_INFLIGHT=10 ; 3 participants ; worker threads = 2 ; headroom for SIGHUP reload spikes |
| Participant | 200m / 256Mi | 1 / 1Gi | dim ≤ 256 ; single tensor mutex ; SIGHUP reload is cheap |

### 24.1 Why no HPA in Phase 2

  * Controller : single-replica (AD11).  HPA on a single-replica
    Deployment is meaningless.
  * Participants : single-replica per id (AD12).  HPA per participant
    would replicate the state ; Phase-2 doesn't support sharded
    participant state.

Phase 3 (PR #17 multi-controller HA) may add HPA on the controller
with leader-election.

---

## 25. SIGHUP-on-`ConfigMap`-change strategy

PR #13 deliberately does **not** ship an automatic SIGHUP trigger.
Options considered :

  | Option | Pros | Cons | Phase-2 decision |
  |---|---|---|---|
  | **Manual `kubectl exec ... kill -HUP 1`** | Simplest ; operator-controlled audit trail | One command per pod | **Adopted for Phase 2** |
  | `checksum/config` annotation | Native K8s ; no extra moving parts | Rolling restart on every change (defeats SIGHUP) | Rejected — AD4 |
  | Reloader sidecar (third-party) | Automatic ; widely used | Extra component ; sidecar lifecycle | Rejected — out of scope |
  | In-cluster `ConfigMap`-watcher Pod that kubectl-exec-SIGHUPs targets | Native ; matches our design | Needs RBAC ; one more moving part | Deferred to PR #18 |

### 25.1 The operator's exact incantation (runbook lift)

```bash
# 1. Edit the manifest in values.yaml.
$ vim charts/efe-stack/values-prod.yaml

# 2. Upgrade the release ; only the ConfigMap mutates (pods stay up).
$ helm upgrade efe-stack ./charts/efe-stack \
    -f charts/efe-stack/values-prod.yaml \
    -n efe

# 3. SIGHUP the controller.
$ kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-controller -- kill -HUP 1

# 4. SIGHUP every participant.
$ for id in supplier-scoring contract-risk spend-categorization ; do
    kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-participant-$id -- kill -HUP 1
  done

# 5. Verify the reload landed.
$ kubectl -n efe port-forward svc/efe-stack-efe-meta-swarm-controller 50080:50080 &
$ grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health \
  | jq '.reloadGen'   # should have incremented by 1
```

### 25.2 Optional one-liner (Phase 2)

A `sighup_all.sh` helper script (NOT shipped in the chart ; lives in
`charts/efe-stack/scripts/` if useful) :

```bash
#!/usr/bin/env bash
set -euo pipefail
NS="${1:?usage: $0 <namespace> <release-name>}"
RELEASE="${2:?usage: $0 <namespace> <release-name>}"
kubectl -n "$NS" exec deploy/"${RELEASE}-efe-meta-swarm-controller" -- kill -HUP 1
for d in $(kubectl -n "$NS" get deploy -l app.kubernetes.io/component=participant \
              -o jsonpath='{.items[*].metadata.name}') ; do
  kubectl -n "$NS" exec deploy/"$d" -- kill -HUP 1
done
```

Documented in `README.md` but not auto-installed.

---

## 26. Golden snapshot test strategy

### 26.1 Mechanism

A `tests/golden/` directory inside the sub-chart holds the
**expected** `helm template` output for canonical scenarios :

  * `kovant-3p.yaml`           — default `values.yaml`
  * `kovant-disabled.yaml`     — `enabled: false` (must be empty)
  * `kovant-network-policy.yaml` — `networkPolicy.enabled: true`
  * `kovant-service-monitor.yaml` — `metrics.serviceMonitor.enabled: true`

CI runs :

```bash
# Render live.
helm template my-release ./charts/efe-stack/charts/efe-meta-swarm \
  --include-crds=false \
  --values charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-3p.values.yaml \
  > /tmp/rendered.yaml

# Diff against the golden.
diff -u charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-3p.yaml \
        /tmp/rendered.yaml
```

A change in any chart template that affects the rendered output
breaks the diff.  Updating the golden requires an explicit
`make update-golden` step (which regenerates the file in-place ; the
PR review then inspects the diff).

### 26.2 Scenario values overrides

`tests/scenarios/kovant-3p.values.yaml` :
```yaml
enabled: true
image:
  tag: "0.2.0-snapshot"
```

`tests/scenarios/kovant-disabled.values.yaml` :
```yaml
enabled: false
```

`tests/scenarios/kovant-network-policy.values.yaml` :
```yaml
enabled: true
image:
  tag: "0.2.0-snapshot"
networkPolicy:
  enabled: true
```

`tests/scenarios/kovant-service-monitor.values.yaml` :
```yaml
enabled: true
image:
  tag: "0.2.0-snapshot"
metrics:
  serviceMonitor:
    enabled: true
```

### 26.3 Update protocol

  * Local : `make update-golden` (a target in the root `Makefile`).
  * The target re-runs `helm template` for every scenario and
    overwrites the corresponding golden file.
  * The PR diff is the proof the change is intentional ; reviewers
    sign off.

---

## 27. CI integration

### 27.1 `.github/workflows/helm.yml` (or equivalent)

```yaml
name: helm-meta-swarm

on:
  pull_request:
    paths:
      - 'charts/efe-stack/charts/efe-meta-swarm/**'
      - 'charts/efe-stack/Chart.yaml'
      - 'charts/efe-stack/values.yaml'

jobs:
  lint-template-golden:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: install helm
        uses: azure/setup-helm@v4
        with:
          version: '3.13.3'

      - name: install kubeconform
        run: |
          curl -L https://github.com/yannh/kubeconform/releases/download/v0.6.4/kubeconform-linux-amd64.tar.gz \
            | tar xz
          sudo mv kubeconform /usr/local/bin/

      - name: helm dependency build
        run: |
          helm dependency build charts/efe-stack

      - name: helm lint
        run: |
          helm lint charts/efe-stack/charts/efe-meta-swarm \
            --values charts/efe-stack/charts/efe-meta-swarm/values.yaml

      - name: helm template + kubeconform
        run: |
          helm template tst charts/efe-stack/charts/efe-meta-swarm \
            -f charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-3p.values.yaml \
            | kubeconform -strict -summary -kubernetes-version 1.28.0 \
                          -schema-location default \
                          -schema-location 'https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{ .Group }}/{{ .ResourceKind }}_{{ .ResourceAPIVersion }}.json'

      - name: golden snapshot diff
        run: |
          for scenario in kovant-3p kovant-disabled kovant-network-policy kovant-service-monitor ; do
            helm template tst charts/efe-stack/charts/efe-meta-swarm \
              -f charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/${scenario}.values.yaml \
              > /tmp/${scenario}.yaml
            diff -u charts/efe-stack/charts/efe-meta-swarm/tests/golden/${scenario}.yaml \
                    /tmp/${scenario}.yaml
          done

      - name: round-trip manifest YAML through Rust validator
        run: |
          helm template tst charts/efe-stack/charts/efe-meta-swarm \
            -f charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-3p.values.yaml \
            | yq 'select(.kind == "ConfigMap") | .data["manifest.yaml"]' \
            > /tmp/manifest.yaml
          cd crates/efe-meta-swarm-server
          cargo test --test manifest_roundtrip -- --manifest-path /tmp/manifest.yaml
```

The last step round-trips the rendered manifest through PR #8's
`manifest::load_and_validate` to assert the chart and the Rust
validator agree on the schema.

### 27.2 `chart-testing` (`ct`) integration

For Phase-3 follow-up : `ct lint-and-install --target-branch main`
runs `helm install` + `helm test` against an in-CI `kind` cluster.

---

## 28. Multi-namespace + multi-tenant considerations

  * Each release lives in one namespace ; the chart never reaches
    across namespaces.
  * `global.tenant` label is propagated by `_helpers.tpl::labels` so
    multi-tenant clusters can `kubectl get pods -l efe.io/tenant=foo`.
  * Multiple parallel installs (e.g. one tenant per namespace) are
    supported : each release gets its own ConfigMap + Services +
    Deployments via `Release.Name` prefixing.

---

## 29. TLS bootstrapping (cert-manager) — Phase-2 placeholders

PR #16 will wire mTLS via cert-manager-issued `Issuer` + `Certificate`.
PR #13 leaves the env vars in place (PR #6) :

```yaml
controller:
  config:
    tlsRequired: false
    # When PR #16 lands :
    # tls:
    #   issuer:
    #     name: efe-meta-swarm-issuer
    #     kind: Issuer
    #   certSecretName: efe-meta-swarm-tls
```

For Phase 2 the binaries skip the TLS path (PR #6 has
`tls_required: false` default).

---

## 30. Metrics integration (Prometheus Operator)

### 30.1 Manual scraping (pre-Operator)

The Deployment pods are annotated with the `prometheus.io/*` set so
the legacy `kubernetes_sd_configs` based Prometheus discovery picks
them up automatically.  This is the lowest-friction path for clusters
without Prometheus Operator.

### 30.2 `ServiceMonitor` (Prometheus Operator)

When `metrics.serviceMonitor.enabled: true`, the chart renders a
`ServiceMonitor` (§ 18) that the Operator turns into the same scrape
config but with `kubernetes_sd_configs` driven by the `Service`
selector.  Cleaner ; relabelings expose `role` and `participant_id`
as Prometheus labels.

### 30.3 Per-binary metric series

  * Controller exposes the full `efe_meta_swarm_*` series (PR #11 + PR #12).
  * Participants expose the same `efe_meta_swarm_reload_*` series
    (PR #12) plus future `efe_meta_participant_*` once PR #17 / PR #18
    wire them.

---

## 31. Logging / tracing wiring

  * Pods write JSON logs to stdout (PR #6's `tracing_subscriber` json
    formatter).
  * The umbrella's `observability` sub-chart (Loki / Promtail) picks
    them up via the standard `kubernetes_sd_configs` path.
  * No extra wiring in PR #13 beyond ensuring `RUST_LOG` env var is
    accepted (it already is via PR #6).
  * For OpenTelemetry traces : PR #19 (out of scope here) will add
    `OTEL_EXPORTER_OTLP_ENDPOINT` env var ; the chart should template
    it conditionally when an `otel:` block is added to values.

---

## 32. K8s DNS discovery — how the controller finds participants

Per AD7 : the chart computes `EFE_META_SWARM_PARTICIPANTS` from
`values.participants[*]` at template time :

```
EFE_META_SWARM_PARTICIPANTS=
  supplier-scoring={release}-efe-meta-swarm-participant-supplier-scoring.{ns}.svc:50091,
  contract-risk={release}-efe-meta-swarm-participant-contract-risk.{ns}.svc:50092,
  spend-categorization={release}-efe-meta-swarm-participant-spend-categorization.{ns}.svc:50093
```

The controller's PR #6 `parse_participants` consumes this verbatim ;
its existing `DiscoveryMode::StaticLocalhost` happily resolves
`<svc>.<ns>.svc` via the in-pod resolv.conf.

When PR #18 ships a real `DiscoveryMode::K8sDns` (watches `Endpoints`
objects), the chart will template `EFE_META_SWARM_DISCOVERY=k8s-dns`
and drop the static env var.

---

## 33. Concurrency / lifecycle invariants

### 33.1 What a `helm install` does, ordered

  1. `ServiceAccount` (controller + N participants) — no dependencies.
  2. `ConfigMap` — name resolved at template time, no dependencies.
  3. `Service` (controller + N participants) — Service name resolved,
     no targets yet but ready to bind.
  4. `Deployment` (controller + N participants) — pods spawn, mount
     ConfigMap, bind to gRPC ports.
  5. `PodDisruptionBudget` (controller) — declarative ; activates as
     soon as pods exist.
  6. (Optional) `NetworkPolicy`, `ServiceMonitor`.
  7. `Pod` (helm test, hook-driven, not part of the install).

K8s API doesn't guarantee `ServiceAccount` is created before
`Deployment` references it ; the API server retries the Deployment's
pod creation until the SA resolves.  In practice this is a < 1 s
delay.

### 33.2 What a `helm upgrade` does

If only the `ConfigMap` mutates :
  * `kubectl get configmap` shows new content.
  * Pods are NOT restarted (AD4 — no `checksum/config`).
  * Operator MUST follow with `kill -HUP 1` per pod.

If the image tag changes :
  * `Deployment.spec.template.spec.containers[0].image` mutates.
  * K8s rolling-restarts pods.
  * Pods come up with the new image AND the current ConfigMap content
    (re-read at boot).

If a participant is added to `values.participants[]` :
  * New `Deployment` + `Service` + `ServiceAccount` resources are
    created.
  * `ConfigMap` is mutated to include the new row.
  * The controller's `EFE_META_SWARM_PARTICIPANTS` env var changes
    → triggers a `Deployment` rolling restart (which the operator
    chose, by editing the new entry).
  * Operator SIGHUPs everything to pick up the new manifest.

If a participant is removed :
  * `Deployment` + `Service` + `ServiceAccount` resources are
    garbage-collected by Helm.
  * `ConfigMap` mutates.
  * Controller's env var changes → rolling restart.
  * Pods of the removed participant are gracefully terminated.

### 33.3 What a `helm rollback` does

  * Reverts every resource to the previous revision.
  * Pods are restarted (image, env vars, ConfigMap all snap back).
  * Operator should re-issue any SIGHUPs if the rollback target's
    manifest differed semantically from current.

---

## 34. Threat-model coverage (T22 / T23 / T24)

| Threat | Severity | PR #13 mitigation |
|---|---|---|
| **T22** (Secret in `values.yaml` committed to git) | SEV-2 | `values.yaml` has NO secret fields ; TLS material (when PR #16 lands) sourced from `Secret`s, not values ; readme + values-prod.yaml example warn against secrets in values files ; consider Helm Secrets / sealed-secrets for future fields |
| **T23** (NetworkPolicy bypass — east-west traffic leaks) | SEV-2 | Default-deny NetworkPolicy ships (opt-in but recommended) ; allow-list is explicit ; the runbook documents the CNI compatibility requirement (Calico / Cilium) ; future PR may add a `policyEngine` admission webhook |
| **T24** (Floating image tag / supply-chain) | SEV-2 | `values.schema.json` enforces `image.tag` non-empty (AD17) ; runbook warns against `:latest` ; Phase-3 will add digest pinning via cosign |

### 34.1 What PR #13 does NOT defend against (deferred)

| Threat | Where | When |
|---|---|---|
| Malicious image push to the registry (image substitution before tag pin) | Registry admission | Phase 3 (cosign + Sigstore) |
| Compromised K8s API server | Out of scope | Cluster ops |
| Operator with `secrets/get` (e.g. `kubectl get secret efe-meta-swarm-tls`) | RBAC at the cluster level | Cluster ops |
| Side-channel info leak via `kubectl logs` (PII in tracing) | Application layer | Phase 3 (structured logging) |

---

## 35. Tests strategy

### 35.1 Helm lint

```
helm lint charts/efe-stack/charts/efe-meta-swarm \
  -f charts/efe-stack/charts/efe-meta-swarm/values.yaml
```

Must exit 0.

### 35.2 Helm template + kubeconform

For each scenario in `tests/scenarios/*.values.yaml` :

```
helm template t charts/efe-stack/charts/efe-meta-swarm \
  -f charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/<scen>.values.yaml \
  | kubeconform -strict -kubernetes-version 1.28.0
```

Must exit 0.

### 35.3 Golden diff

```
diff -u \
  charts/efe-stack/charts/efe-meta-swarm/tests/golden/<scen>.yaml \
  /tmp/<scen>.rendered.yaml
```

Must be empty.

### 35.4 Manifest YAML round-trip

```
helm template t charts/efe-stack/charts/efe-meta-swarm -f ... \
  | yq 'select(.kind == "ConfigMap") | .data["manifest.yaml"]' \
  > /tmp/manifest.yaml

cargo test -p efe-meta-swarm-server --test manifest_roundtrip -- \
  --manifest-path /tmp/manifest.yaml
```

The Rust test (`manifest_roundtrip.rs`, new in PR #13) calls
`efe_meta_swarm_server::manifest::load_and_validate(&path)` and
asserts `Ok(_)` for every scenario.

### 35.5 `helm test` connection probe (in `kind` cluster — Phase 3 stretch)

```
kind create cluster --name efe-pr13
helm install efe-stack ./charts/efe-stack -n efe --create-namespace \
  --set efe-meta-swarm.enabled=true \
  --set efe-meta-swarm.image.tag=0.2.0-test
helm test efe-stack -n efe
```

The probe Pod runs `grpcurl` against `MetaSwarmService::Health` ;
expected exit 0 + `status: SERVING` in the response.

### 35.6 Test matrix

| Scenario | Lint | Template | Kubeconform | Golden diff | Round-trip | Helm test |
|---|---|---|---|---|---|---|
| `kovant-3p` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (kind) |
| `kovant-disabled` | ✓ | ✓ | ✓ | ✓ | n/a (no ConfigMap) | n/a |
| `kovant-network-policy` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (kind) |
| `kovant-service-monitor` | ✓ | ✓ | ✓ (CRD-aware) | ✓ | ✓ | n/a |

---

## 36. Acceptance criteria (AC1–AC30)

PR #13 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `helm lint` clean on default values | CI |
| AC2 | `helm template` succeeds for all 4 scenarios | CI |
| AC3 | `kubeconform -strict` validates all rendered resources | CI |
| AC4 | Golden diff exits 0 for `kovant-3p` | CI |
| AC5 | Golden diff exits 0 for `kovant-disabled` (empty render) | CI |
| AC6 | Golden diff exits 0 for `kovant-network-policy` | CI |
| AC7 | Golden diff exits 0 for `kovant-service-monitor` | CI |
| AC8 | `values.schema.json` rejects missing `image.tag` | manual + CI |
| AC9 | `values.schema.json` rejects `kappa = 1.5` (out of (0, 1)) | manual + CI |
| AC10 | `values.schema.json` rejects empty `participants` array | manual + CI |
| AC11 | `values.schema.json` rejects `controller.replicaCount = 2` | manual + CI |
| AC12 | Rendered ConfigMap's `manifest.yaml` round-trips through `manifest::load_and_validate` | CI |
| AC13 | `enabled: false` produces 0 resources from the sub-chart | CI |
| AC14 | `enabled: true` produces controller Deployment + Service + SA + PDB + ConfigMap + N participant {Dep, Svc, SA} | CI |
| AC15 | Controller's `EFE_META_SWARM_PARTICIPANTS` env var is `id1=svc.ns.svc:port,...` (validates against `lookup_endpoints` in PR #8) | CI |
| AC16 | Both controller and participant Deployments mount the ConfigMap at `/etc/efe-meta-swarm/manifest.yaml` | CI |
| AC17 | `EFE_META_SWARM_MANIFEST=/etc/efe-meta-swarm/manifest.yaml` env var present on every meta-swarm pod | CI |
| AC18 | No `checksum/config` annotation on any Deployment pod template (AD4) | CI grep |
| AC19 | `seccompProfile: RuntimeDefault` on every pod | CI grep |
| AC20 | `runAsNonRoot: true` + `runAsUser: 65532` on every pod and container | CI grep |
| AC21 | `automountServiceAccountToken: false` on every pod | CI grep |
| AC22 | `PodDisruptionBudget` has `maxUnavailable: 0` (when enabled, the default) | CI |
| AC23 | `NetworkPolicy` deny-all + allow-controller-ingress + allow-controller-egress + allow-participants-ingress render correctly when enabled | CI |
| AC24 | `ServiceMonitor` references the controller AND participant Services when enabled | CI |
| AC25 | `templates/tests/connection-test.yaml` renders a `helm.sh/hook: test` Pod | CI |
| AC26 | Parent `charts/efe-stack/Chart.yaml` lists `efe-meta-swarm` as a dependency with `condition: efe-meta-swarm.enabled` | CI grep |
| AC27 | Parent `charts/efe-stack/values.yaml` has `efe-meta-swarm: { enabled: false }` default | CI grep |
| AC28 | `efe-server` Deployment template injects the `EFE_META_SWARM_*` env vars when `efe-meta-swarm.enabled = true` | CI |
| AC29 | Documentation : `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md` referenced from `README.md` + `NOTES.txt` | manual |
| AC30 | Threat-model coverage : T22 / T23 / T24 each have at least one mitigation in the chart or runbook | manual |

---

## 37. Risks + mitigations

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| **PR13-R1** | Helm 3 only — operators on Helm 2 break | L / H | `Chart.yaml::apiVersion: v2` (Helm 2 doesn't understand) ; documented in README |
| **PR13-R2** | CNI doesn't enforce `NetworkPolicy` — render is no-op | M / M | Opt-in (AD14) ; README documents the CNI compatibility list (Calico, Cilium, Weave-policy) |
| **PR13-R3** | `prometheus-operator` CRDs missing — `ServiceMonitor` install fails | M / M | Opt-in (AD15) ; the chart's `if` guard prevents render when off ; documented |
| **PR13-R4** | Operator forgets `kill -HUP 1` after `helm upgrade` and wonders why config didn't take | H / L | NOTES.txt prints the SIGHUP commands ; runbook lifts them ; future PR #18 may automate via in-cluster watcher |
| **PR13-R5** | `values.schema.json` rejects a valid config because of a too-strict pattern | L / M | Patterns reviewed against PR #6 / PR #8 validators ; schema unit-tested with edge cases |
| **PR13-R6** | Image tag pinning forces operator override — broken `helm install` for newcomers | M / L | README ships a working `tag` in the kovant-3p scenario ; defaults disabled so newcomers don't hit it |
| **PR13-R7** | Golden snapshots churn on chart-templating-engine version bump | M / M | Pinned Helm 3.13.3 in CI ; update protocol documented ; CI uses the same binary version as developers |
| **PR13-R8** | `range` over `participants` generates duplicate name on YAML serializer bugs | L / H | Templated names include `id` (guaranteed unique by schema's `pattern`) ; tested in golden |
| **PR13-R9** | ConfigMap exceeds 1 MB K8s limit on N=64 participants | LL / M | Schema caps `maxItems: 64` ; conservative manifest size (~ 200 B / participant) × 64 = 13 KB, comfortably under |
| **PR13-R10** | Operator on K8s 1.27 — `policy/v1 PodDisruptionBudget` not yet GA | LL / L | `kubeVersion: ">=1.28.0-0"` rejects older clusters ; runbook documents |
| **PR13-R11** | Sub-chart values key collision with sibling chart (e.g. `image.tag` overlap) | L / M | Sub-chart values are namespaced under `efe-meta-swarm:` in umbrella values ; no collision possible |
| **PR13-R12** | Cross-chart dependency (`efeServerWiring`) drifts from parent's expectation | M / M | Documented in § 7.3 ; CI runs an integration test that renders the umbrella + asserts `efe-server` Deployment has the right env vars |

---

## 38. Rollback strategy

If PR #13 lands and breaks production :

  1. **Operator level** : `helm rollback efe-stack <previous-rev>`.
     Reverts every resource to the pre-PR-#13 state in seconds.
  2. **Per-commit revert** : C6 → C5 → … → C1.  Each commit is
     independently `helm lint`-green.
  3. **Wholesale revert** : `git revert <merge-commit>` ; the sub-chart
     vanishes ; the umbrella's `Chart.yaml` dependency entry is gone ;
     the chart's `values.yaml` block is removed.
  4. **Hotfix** : if only golden tests flake, mark them `# golden-skip:
     true` ; production-facing templates are unaffected.

Total Time To Mitigate :
  * `helm rollback` : ~ 30 s.
  * Code revert + redeploy : ~ 5 min.

---

## 39. Sequence diagrams (ASCII)

### 39.1 `helm install` (sub-chart enabled)

```
operator                 helm                              K8s API
   |                       |                                 |
   | helm install efe-stack -f values-prod.yaml              |
   |──────────────────────►|                                 |
   |                       | template + lint + schema-validate
   |                       | render every resource           |
   |                       | POST ServiceAccount × 4 ───────►| → created
   |                       | POST ConfigMap   ──────────────►| → created
   |                       | POST Service × 4 ──────────────►| → created
   |                       | POST Deployment × 4 ───────────►| → created
   |                       |                                 |   ↓ pod scheduler binds to nodes
   |                       |                                 |   ↓ pod scheduler pulls image
   |                       |                                 |   ↓ kubelet starts containers
   |                       |                                 |   ↓ binaries call manifest::load
   |                       |                                 |   ↓ controller dials participants
   |                       |                                 |   ↓ all probes go green
   |  NOTES.txt printed    |                                 |
   |◄──────────────────────|                                 |
```

### 39.2 `helm upgrade` with manifest-only change (SIGHUP path)

```
operator               helm                    K8s API           pods
   |                     |                       |                |
   | helm upgrade ...    |                       |                |
   |────────────────────►|                       |                |
   |                     | diff resources        |                |
   |                     | ConfigMap mutated     |                |
   |                     | (no Deployment/Pod    |                |
   |                     |  template change)     |                |
   |                     | PATCH ConfigMap ─────►|                |
   |                     |                       | → committed    |
   |                     |                       |                | (pods unaware ; mounted RO copy is updated via inotify, but binaries don't watch)
   |                     |                       |                |
   | kubectl exec ... -- kill -HUP 1             |                |
   |──────────────────────────────────────────────────────────────►|
   |                     |                       |                | SIGHUP → apply_reload_from_disk
   |                     |                       |                | reload_gen++
   |                     |                       |                |
   | grpcurl Health      |                       |                |
   |──────────────────────────────────────────────────────────────►|
   | ◄────────────────────────────────────────── { reloadGen: 1 } |
```

### 39.3 `helm test`

```
operator               helm                    K8s API           pods
   |                     |                       |                |
   | helm test efe-stack |                       |                |
   |────────────────────►|                       |                |
   |                     | apply "hook: test" Pod                 |
   |                     | POST Pod ────────────►|                |
   |                     |                       | → grpcurl runs |
   |                     |                       |    ✓ Health    |
   |                     |                       | → Pod exits 0  |
   |                     |                       |                |
   | TEST SUITE: PASS    |                       |                |
   |◄────────────────────|                       |                |
```

---

## 40. Reviewer checklist

```
## Reviewer checklist — PR #13

### Chart correctness
- [ ] `helm lint` exit 0 with default values + every scenario override
- [ ] `helm template` produces zero meta-swarm resources when `enabled: false` (AD2)
- [ ] No `checksum/config` annotation on any Deployment (AD4)
- [ ] `seccompProfile: RuntimeDefault` everywhere (AD8)
- [ ] `runAsNonRoot: true` + drop ALL caps + readOnlyRootFilesystem (AD8 / AD9)
- [ ] `automountServiceAccountToken: false` on every pod (AD20)

### Networking
- [ ] Controller Service: ClusterIP, appProtocol=grpc, ports 50080 / 9091
- [ ] Participant Services rendered per `range` entry
- [ ] `EFE_META_SWARM_PARTICIPANTS` env var properly templated (AD7 / § 32)
- [ ] NetworkPolicy deny-all + targeted allow when enabled (AD14)

### Validation
- [ ] `values.schema.json` rejects empty `image.tag` (AC8)
- [ ] Schema rejects out-of-range `kappa` (AC9)
- [ ] Schema rejects multi-replica controller (AC11)
- [ ] Manifest YAML round-trips through `manifest::load_and_validate` (AC12)

### Reload
- [ ] Both controller and participants mount the ConfigMap at the same path (AC16)
- [ ] `EFE_META_SWARM_MANIFEST` env var present and correct (AC17)
- [ ] No `checksum/config` annotation (AC18)
- [ ] NOTES.txt prints the SIGHUP incantation
- [ ] Runbook lifts the SIGHUP-after-helm-upgrade workflow (§ 25.1)

### Observability
- [ ] Prometheus annotations present on every pod
- [ ] `ServiceMonitor` (when enabled) covers controller + participants (AC24)
- [ ] Relabelings expose `role` + `participant_id` labels

### Threat model
- [ ] T22 / T23 / T24 each have a mitigation in the chart or runbook (§ 34 / AC30)
- [ ] No secrets in `values.yaml`
- [ ] Image tag pinning enforced

### Cross-PR consistency
- [ ] Parent chart's `Chart.yaml` registers the dep with `condition: efe-meta-swarm.enabled`
- [ ] Parent chart's `values.yaml` defaults to `efe-meta-swarm: { enabled: false }`
- [ ] Parent `efe-server` deployment.yaml templates the `EFE_META_SWARM_*` env vars
      iff `efe-meta-swarm.enabled = true` (PR #11 hook)
- [ ] Golden snapshots check in to git (CI diff)
```

---

## 41. Operator runbook

### 41.1 Initial install (dev / staging)

```bash
# 1. Pin the image tag (no :latest).
$ cat > values-dev.yaml <<EOF
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0"
  controller:
    config:
      reflectionEnabled: true
  metrics:
    serviceMonitor:
      enabled: true
EOF

# 2. Install.
$ helm install efe-stack ./charts/efe-stack \
    --namespace efe --create-namespace \
    -f values-dev.yaml

# 3. Verify pods came up.
$ kubectl -n efe get pods -l efe.io/meta-swarm-role

# 4. Run the connection probe.
$ helm test efe-stack -n efe
```

### 41.2 Production install

```bash
# values-prod.yaml — same as dev but locked.
$ cat > values-prod.yaml <<EOF
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0"           # explicit, no :latest
    pullPolicy: IfNotPresent
  manifest:
    iteration:
      max_iters: 100
      tolerance: 0.0001
      alpha_default: 0.5
      coupling_gamma: 0.2
      max_wall_ms: 15000
  controller:
    resources:
      requests: {cpu: 1,   memory: 1Gi}
      limits:   {cpu: 4,   memory: 4Gi}
  participants:
    - id: supplier-scoring
      dim: 16
      kappa: 0.6
      seed: 11
      resources:
        requests: {cpu: 500m, memory: 512Mi}
        limits:   {cpu: 2,    memory: 2Gi}
    - id: contract-risk
      dim: 16
      kappa: 0.7
      seed: 22
      resources:
        requests: {cpu: 500m, memory: 512Mi}
        limits:   {cpu: 2,    memory: 2Gi}
    - id: spend-categorization
      dim: 16
      kappa: 0.5
      seed: 33
      resources:
        requests: {cpu: 500m, memory: 512Mi}
        limits:   {cpu: 2,    memory: 2Gi}
  networkPolicy:
    enabled: true
  metrics:
    serviceMonitor:
      enabled: true
EOF

$ helm install efe-stack ./charts/efe-stack \
    --namespace efe \
    -f values-prod.yaml
```

### 41.3 Hot reload (Phase 2 SIGHUP path)

```bash
# 1. Edit values-prod.yaml — e.g. bump alpha_default to 0.6.
$ vim values-prod.yaml

# 2. helm upgrade — mutates ConfigMap, leaves pods running.
$ helm upgrade efe-stack ./charts/efe-stack \
    --namespace efe \
    -f values-prod.yaml

# 3. SIGHUP every pod.
$ kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-controller -- kill -HUP 1
$ for id in supplier-scoring contract-risk spend-categorization ; do
    kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-participant-$id -- kill -HUP 1
  done

# 4. Verify.
$ kubectl -n efe port-forward svc/efe-stack-efe-meta-swarm-controller 50080:50080 &
$ grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health \
  | jq '.reloadGen'
```

### 41.4 Image upgrade (rolling restart path)

```bash
# 1. Bump image tag in values.
$ sed -i 's/tag: "0.2.0"/tag: "0.2.1"/' values-prod.yaml

# 2. helm upgrade — Deployment pod templates mutate, k8s rolls.
$ helm upgrade efe-stack ./charts/efe-stack -n efe -f values-prod.yaml

# 3. Watch the rolling restart.
$ kubectl -n efe rollout status deploy/efe-stack-efe-meta-swarm-controller
$ for id in supplier-scoring contract-risk spend-categorization ; do
    kubectl -n efe rollout status deploy/efe-stack-efe-meta-swarm-participant-$id
  done

# 4. helm test to confirm.
$ helm test efe-stack -n efe
```

### 41.5 Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `helm install` fails with `kappa must be in (0, 1)` | values.yaml typo | Edit values, re-install |
| Pod stuck in `ContainerCreating` | image pull failure | `kubectl describe pod ...` → check pullSecrets / registry creds |
| Pod CrashLoopBackOff | manifest validation failed at boot | `kubectl logs ...` — `validate_manifest` error message |
| `helm test` fails with `connection refused` | controller not ready | wait + retry ; check liveness probe |
| `Health.reloadGen` stays at 0 after edit | forgot `kill -HUP 1` | issue SIGHUP per pod (runbook § 41.3) |
| Pods crashing after SIGHUP | manifest reload rejected (drift) | `kubectl logs ...` — `validate_reload_compatibility` rejection reason |
| `RELOADING (3)` status sticks | reload swap never completed (controller panic mid-swap, unlikely with PR #12's safety) | `kubectl logs ...` ; if needed `helm rollback` to previous revision |
| ServiceMonitor not scraped | `prometheus-operator` not installed | install operator or set `metrics.serviceMonitor.enabled: false` |

---

## 42. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR13-1** | Should the chart ship a default `policyEngine: kyverno` admission policy for image-tag enforcement ? | Defer to PR #20 / supply chain story |
| **OQ-PR13-2** | Should participants use a `StatefulSet` for forward-compat with Phase-3 sharded state ? | NO for Phase 2 — switching is mechanical when state matters |
| **OQ-PR13-3** | Should the chart expose `vpa:` block for VPA integration ? | NO — VPA is out of Phase-2 scope ; can be added in a values overlay |
| **OQ-PR13-4** | Helm SemVer policy : major bump on incompatible values changes ? | YES — `version: x.y.z` follows SemVer, `Chart.yaml::version` bumps on every PR |
| **OQ-PR13-5** | Should we add a `topologySpreadConstraints` block to participants ? | YES in Phase 3 when participants replicate ; for Phase 2 (`replicas: 1`) it's a no-op |
| **OQ-PR13-6** | Should the umbrella chart's `values-prod.yaml` ship pinned image digests instead of tags ? | Phase 3 — needs cosign + Sigstore wiring |
| **OQ-PR13-7** | Should the `helm test` Pod use `grpcurl` from the same image as the controller (single image, no extra dep) ? | NO — `grpcurl` is not bundled in the EFE image ; adding it bloats the production image |
| **OQ-PR13-8** | Should we ship a `Job` for one-shot SIGHUP-all (a chart-internal `helm sighup` action) ? | NO for Phase 2 — runbook covers the manual workflow ; PR #18 may automate |

---

## 43. Appendix : full file inventory

### 43.1 Files added (17)

```
charts/efe-stack/charts/efe-meta-swarm/Chart.yaml                          NEW  ~ 35 LOC
charts/efe-stack/charts/efe-meta-swarm/values.yaml                         NEW  ~ 220 LOC
charts/efe-stack/charts/efe-meta-swarm/values.schema.json                  NEW  ~ 230 LOC
charts/efe-stack/charts/efe-meta-swarm/README.md                           NEW  ~ 180 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl              NEW  ~ 130 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/NOTES.txt                 NEW  ~ 60 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/configmap-manifest.yaml   NEW  ~ 80 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/controller-deployment.yaml NEW ~ 180 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/controller-service.yaml   NEW  ~ 35 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/controller-serviceaccount.yaml NEW ~ 20 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/pdb-controller.yaml       NEW  ~ 25 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/participant-deployment.yaml NEW ~ 200 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/participant-service.yaml  NEW  ~ 45 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/participant-serviceaccount.yaml NEW ~ 30 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml       NEW  ~ 60 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/networkpolicy.yaml        NEW  ~ 100 LOC
charts/efe-stack/charts/efe-meta-swarm/templates/tests/connection-test.yaml NEW ~ 60 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-3p.values.yaml NEW ~ 5 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-disabled.values.yaml NEW ~ 2 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-network-policy.values.yaml NEW ~ 7 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-service-monitor.values.yaml NEW ~ 7 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-3p.yaml         NEW  ~ 900 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-disabled.yaml   NEW  ~ 0 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-network-policy.yaml NEW ~ 1 000 LOC
charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-service-monitor.yaml NEW ~ 1 050 LOC

docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md                                    NEW  this file
crates/efe-meta-swarm-server/tests/manifest_roundtrip.rs                   NEW  ~ 60 LOC (round-trip test consumer for CI)
```

### 43.2 Files modified (4)

```
charts/efe-stack/Chart.yaml                                                MODIFIED  +6 LOC : sub-chart dependency
charts/efe-stack/values.yaml                                               MODIFIED  +25 LOC : `efe-meta-swarm:` defaults block
charts/efe-stack/charts/efe-server/templates/deployment.yaml               MODIFIED  +15 LOC : EFE_META_SWARM_* env vars when sub-chart enabled
.github/workflows/helm.yml (or chart-testing workflow)                     MODIFIED  +30 LOC : golden-diff + kubeconform + round-trip steps
```

### 43.3 LOC tally

| Category | LOC |
|---|---|
| New chart templates (production YAML) | ~ 1 270 |
| New values + schema | ~ 450 |
| New `_helpers.tpl` | ~ 130 |
| New README + NOTES.txt | ~ 240 |
| New scenario values | ~ 21 |
| New golden snapshots (auto-generated) | ~ 2 950 |
| New Rust round-trip test | ~ 60 |
| New CI workflow steps | ~ 30 |
| Modified parent chart wiring | ~ 46 |
| **Total net new** | **~ 5 200 LOC** (most of which is auto-generated golden YAML) |

Production-relevant net new (excluding generated goldens) :
**~ 2 250 LOC** of YAML / Go templates / JSON schema / shell.

---

## 44. Appendix : example overrides

### 44.1 Kovant dev cluster

```yaml
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0-rc.1"
    pullPolicy: Always
  controller:
    config:
      reflectionEnabled: true     # enable grpcurl reflection in dev
  metrics:
    serviceMonitor:
      enabled: false              # legacy Prometheus discovery instead
```

### 44.2 Kovant staging cluster

```yaml
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0"
  controller:
    resources:
      requests: {cpu: 1, memory: 1Gi}
      limits:   {cpu: 2, memory: 2Gi}
  metrics:
    serviceMonitor:
      enabled: true
      interval: 15s               # tighter scrape in staging for faster feedback
```

### 44.3 Kovant production cluster

```yaml
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0"                  # locked
    pullPolicy: IfNotPresent
  controller:
    resources:
      requests: {cpu: 1,   memory: 1Gi}
      limits:   {cpu: 4,   memory: 4Gi}
    podDisruptionBudget:
      enabled: true
      maxUnavailable: 0
  participants:
    - id: supplier-scoring
      dim: 32                     # production dim larger than dev
      kappa: 0.62
      seed: 11
      resources:
        requests: {cpu: 1, memory: 1Gi}
        limits:   {cpu: 2, memory: 2Gi}
    - id: contract-risk
      dim: 32
      kappa: 0.68
      seed: 22
      resources:
        requests: {cpu: 1, memory: 1Gi}
        limits:   {cpu: 2, memory: 2Gi}
    - id: spend-categorization
      dim: 32
      kappa: 0.55
      seed: 33
      resources:
        requests: {cpu: 1, memory: 1Gi}
        limits:   {cpu: 2, memory: 2Gi}
  networkPolicy:
    enabled: true                 # always-on in prod
    ingressNamespaceSelector:
      matchLabels:
        kubernetes.io/metadata.name: efe-prod
    prometheusNamespaceSelector:
      matchLabels:
        kubernetes.io/metadata.name: monitoring
  metrics:
    serviceMonitor:
      enabled: true
      labels:
        prometheus: efe-prod      # match the production Prometheus selector
```

### 44.4 Single-controller stress-test cluster (4 participants)

```yaml
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0-stress"
  manifest:
    meta:
      dim: 64
    iteration:
      max_iters: 200
      tolerance: 1.0e-6
      max_wall_ms: 30000
  controller:
    config:
      maxInflight: 20
    resources:
      requests: {cpu: 4,  memory: 4Gi}
      limits:   {cpu: 8,  memory: 8Gi}
  participants:
    - id: stress-a
      dim: 64
      kappa: 0.65
      seed: 1
      service: {grpcPort: 50091, metricsPort: 9101}
      resources:
        requests: {cpu: 2, memory: 2Gi}
        limits:   {cpu: 4, memory: 4Gi}
    - id: stress-b
      dim: 64
      kappa: 0.70
      seed: 2
      service: {grpcPort: 50092, metricsPort: 9102}
      resources:
        requests: {cpu: 2, memory: 2Gi}
        limits:   {cpu: 4, memory: 4Gi}
    - id: stress-c
      dim: 64
      kappa: 0.55
      seed: 3
      service: {grpcPort: 50093, metricsPort: 9103}
      resources:
        requests: {cpu: 2, memory: 2Gi}
        limits:   {cpu: 4, memory: 4Gi}
    - id: stress-d
      dim: 64
      kappa: 0.60
      seed: 4
      service: {grpcPort: 50094, metricsPort: 9104}
      resources:
        requests: {cpu: 2, memory: 2Gi}
        limits:   {cpu: 4, memory: 4Gi}
```

---

## 45. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **L2 gate G10** | Phase-2 graduation gate "K8s deployment with declarative configuration management" |
| **Sub-chart** | A Helm chart that lives under `charts/<parent>/charts/<child>/` and is referenced from the parent's `Chart.yaml::dependencies` |
| **Umbrella chart** | The parent chart (`charts/efe-stack/`) that bundles N sub-charts |
| **Vendored sub-chart** | A sub-chart whose source is committed into the parent's `charts/` directory (vs pulled from a Helm registry at install time, AD24) |
| **values.schema.json** | JSON Schema applied by Helm 3 to the merged `values.yaml` ; rejects malformed configs at install time |
| **Golden snapshot** | A reference `helm template`-rendered YAML file checked into git ; CI diffs the live render against it |
| **NOTES.txt** | Helm convention : a templated message rendered after `helm install` / `helm upgrade` ; printed to the operator's terminal |
| **`helm test`** | Helm convention : a `Pod` annotated `helm.sh/hook: test` runs probe logic after install ; `helm test <release>` executes it |
| **PodDisruptionBudget** | K8s policy/v1 : limits voluntary disruptions (node drain, scale-down) for a labelled pod set |
| **ServiceMonitor** | `prometheus-operator` CRD that tells the operator which Services to scrape for metrics |
| **NetworkPolicy** | K8s networking/v1 : deny / allow rules for pod-to-pod traffic, CNI-enforced |
| **`appProtocol: grpc`** | Service-port hint that the protocol is HTTP/2 + gRPC ; consumed by service-mesh proxies and L4 load balancers |
| **Helm hook** | A `helm.sh/hook` annotation that runs a resource at a specific lifecycle event (pre-install, post-upgrade, test, etc.) |
| **`fullnameOverride`** | Helm convention : allow operators to override the release-prefixed name (rare ; here for forward-compat) |

---

*End of architecture document. Total: ~ 2 200 lines.*

*Next deliverable : PR #13 code (6 commits, ~ +1 340 LOC of YAML / Go templates / JSON schema / shell + auto-generated goldens).*
