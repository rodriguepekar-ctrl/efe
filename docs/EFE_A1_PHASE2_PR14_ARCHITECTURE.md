# EFE-A1 Phase 2 — PR #14 : Helm values + templates (production-grade)

| Field | Value |
|---|---|
| **PR** | #14 (Sprint A1 Phase 2 / P2.5) |
| **Branch** | `efe-a1/p2-pr14-helm-values-templates` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× meta-swarm + 1× sre + 1× platform-deploy + 1× sec + 1× observability |
| **Depends on** | PR #5 (proto), PR #6 (scaffold), PR #8 (controller wire), PR #9 (participant wire), PR #10 (multiprocess), PR #11 (efe-server flag plumbing), PR #12 (SIGHUP + ReloadConfig), PR #13 (Helm sub-chart skeleton) |
| **Blocks** | PR #15 (iterate dispatch metrics), PR #16 (mTLS provisioning), PR #17 (multi-controller HA / leader election), PR #18 (in-cluster `ConfigMap`-watcher) |
| **L2 gate** | G11 (operator-grade Helm chart with automated reload + observability + multi-env values + bundle weight-loader contract) |
| **Threat IDs in scope** | T22-T24 (carried from PR #13), T25 (Helm hook over-privileged RBAC), T26 (sidecar `shareProcessNamespace` /proc exposure), T27 (backup ConfigMap rotation tampering), T28 (bundle endpoint unauthenticated — deferred to PR #16) |
| **Roadmap reference** | EFE_A1_PHASE2_ROADMAP.md § C7 |
| **CVSS bandwidth** | All ≤ SEV-2 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals
  2. Pre-flight checks
  3. Branch + commit plan (8 commits)
  4. Architectural decisions (AD1–AD32)
  5. Topology + integration diagram (extends PR #13)
  6. Layout : files added + modified under `charts/efe-stack/charts/efe-meta-swarm/`
  7. Multi-environment values flavors (dev / staging / prod / canary)
  8. PriorityClass + HPA placeholder
  9. Per-tenant label propagation + multi-tenant slices
 10. Pre-upgrade Helm hook : backup `ConfigMap` snapshot (rotating ring of 5)
 11. Post-upgrade Helm hook : SIGHUP-all `Job` (automate the PR #12 runbook)
 12. RBAC additions for the hook `Job`s (least-privilege)
 13. Bundle weight-loader sidecar (structural placeholder ; full wire in PR #15)
 14. `PrometheusRule` alerts (reload failure rate, latency, availability)
 15. Grafana dashboard `ConfigMap` (sidecar import via annotation)
 16. TLS bootstrap stubs (cert-manager `Issuer` + `Certificate`)
 17. NetworkPolicy refinements (tenant slice + backup `Job` egress)
 18. Resource sizing matrix per environment
 19. Schema additions (`values.schema.json` PR #14 fields)
 20. Helm hook ordering + weights
 21. Operator-driven workflows : upgrade / rollback / hot reload
 22. Concurrency / lifecycle invariants (extends PR #13 § 33)
 23. Threat-model coverage (T22-T28)
 24. Tests strategy (extends PR #13 § 35)
 25. Acceptance criteria (AC1–AC38)
 26. Risks + mitigations
 27. Rollback strategy
 28. Sequence diagrams (helm upgrade with hooks ; bundle sidecar polling)
 29. Reviewer checklist
 30. Operator runbook (extended : 11 scenarios)
 31. Open questions
 32. Appendix : full file inventory
 33. Appendix : example overrides + canary slice
 34. Glossary

---

## 0. TL;DR

PR #14 evolves the PR #13 **skeleton** into a **production-grade
operator surface** by layering :

  1. **Multi-environment values flavors** (`values-dev.yaml`,
     `values-staging.yaml`, `values-prod.yaml`, `values-canary.yaml`)
     vendored inside the sub-chart under `examples/`.  Operators
     `helm upgrade … -f values-prod.yaml` ; the four flavors are
     curated to compose with the umbrella's `global:` block.
  2. **Helm pre-upgrade hook** : a `Job` that snapshots the
     manifest `ConfigMap` into a rotating ring of 5 `backup-N`
     `ConfigMap`s, indexed by `reload_gen`.  Operator-side audit
     trail without touching K8s `EventList`.
  3. **Helm post-upgrade hook** : a `Job` that does
     `kubectl exec deploy/<controller> -- kill -HUP 1` + same for
     every participant — automating the PR #12 § 25.1 runbook,
     removing the manual SIGHUP step.
  4. **Bundle weight-loader sidecar** : a structural placeholder
     `Container` next to every participant ; shares a `emptyDir`
     volume + `shareProcessNamespace: true` so the sidecar can
     SIGHUP the participant on weight bundle updates.  In Phase-2
     the sidecar is a `pause`-style no-op image ; PR #15 fills the
     real bundle-poll loop.  Architecturally locked in PR #14 so
     PR #15 has a stable contract.
  5. **`PrometheusRule` alerts** for the PR #12 reload metrics +
     per-binary availability (controller / each participant).
     Six alerts spanning warning + critical severities.
  6. **Grafana dashboard** as a `ConfigMap` with the
     `grafana_dashboard: "1"` annotation so the Grafana sidecar
     auto-imports.  Panels for reload_gen ramp, reload duration
     histogram, health probe success rate, RELOADING window
     duration.
  7. **`PriorityClass`** for the controller (`100000`) so it
     out-prioritises participants under resource pressure.
  8. **Per-tenant label propagation** : every resource gets
     `efe.io/tenant: <slug>` inherited from the umbrella's
     `global.tenant`.  Enables `kubectl get ... -l efe.io/tenant=foo`
     in multi-tenant clusters.
  9. **`HorizontalPodAutoscaler` placeholder** : `enabled: false`
     scaffold for PR #17's multi-controller HA work ; renders the
     full HPA spec when flipped but doesn't actually scale Phase-2's
     single-controller (the controller `Deployment` has
     `replicaCount: 1` hard-locked).
 10. **TLS bootstrap stubs** : cert-manager `Issuer` +
     `Certificate` template, gated on
     `controller.config.tls.certManager.enabled`.  PR #16 wires the
     binary side ; PR #14 ships the chart side.
 11. **NetworkPolicy refinements** : explicit `allow` rule for the
     `pre-upgrade-snapshot` and `post-upgrade-sighup` `Job`s' egress
     so they reach the API server / target pods under deny-by-default
     CNI.
 12. **Schema additions** : 11 new top-level keys in
     `values.schema.json` (priorityClass, hooks, sidecar, alerts,
     dashboard, tls, hpa, etc.) — every PR #14 surface is typed
     and validated at install time.

PR #14 closes Phase-2 graduation gate **G11 (operator-grade Helm
chart with automated reload + observability + multi-env values +
bundle weight-loader contract)** and unlocks PR #15 (iterate
dispatch + metrics surface), PR #16 (mTLS), PR #17 (multi-controller
HA).

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Evolve PR #13's deployable skeleton into a **production-grade**
> sub-chart by layering multi-environment values flavors, pre/post
> Helm hooks for automated manifest backup + SIGHUP-all, a
> structural bundle weight-loader sidecar (Phase-3 contract locked
> at PR #14 time), Prometheus alerts + Grafana dashboards, cert-manager
> TLS bootstrap stubs, a controller `PriorityClass`, and per-tenant
> label propagation — without breaking PR #13's `enabled: false`
> silent-by-default doctrine or PR #12's SIGHUP-driven reload
> semantics.

### 1.2 Non-goals (deferred to later PRs)

| Item | Why deferred | Target PR |
|---|---|---|
| Real bundle-poller binary inside the sidecar (today : `pause` no-op) | needs the bundle-endpoint contract + auth + cosign verify | PR #15 |
| Real `MetaSwarmClient::iterate` dispatch wiring on `efe-server` side | PR #11 only wired probe ; PR #15 wires dispatch | PR #15 |
| Real cert-manager `Issuer` + `Certificate` lifecycle (`Secret` rotation, mTLS at the gRPC layer) | needs the binary's tonic-tls integration | PR #16 |
| Multi-controller leader election + HPA scaling logic | needs leader-lock subsystem | PR #17 |
| In-cluster `ConfigMap`-watcher Pod (real-time SIGHUP) | PR #14's post-upgrade `Job` covers operator-driven path ; watcher is for `kubectl edit cm` cases | PR #18 |
| OTLP collector sidecar for distributed tracing | needs global OTEL operator wiring | PR #19 |
| Helm pre-delete hook for state quiesce | no persistent state in Phase 2 | PR #20 |
| Operator-driven `helm install --atomic` failure recovery | tooling responsibility | follow-up |
| Backup `ConfigMap` cross-cluster sync (DR) | infra concern | Phase 3 |
| Bundle endpoint authentication / cosign verification | needs PKI wiring | PR #16 |
| Per-tenant resource quotas | namespace-scope concern | Phase 3 |
| Per-tenant NetworkPolicy isolation | service-mesh concern | Phase 3 |
| Audit log streaming (every reload → SIEM) | observability platform concern | PR #19 |
| FluxCD / Argo CD reconciliation hooks | GitOps tooling, orthogonal to chart | follow-up |
| ClusterRole-based hooks (`kubectl exec` cluster-wide) | least-privilege says Role is enough | locked |
| Kustomize overlay equivalents | Helm-only is the official path | follow-up |
| HelmRelease (FluxCD CR) examples | tooling docs, separate PR | follow-up |
| Dual-stack IPv6 service support | infra concern | Phase 3 |

### 1.3 Scope summary

PR #14 ships :

  1. Four environment flavors under `examples/` (dev / staging / prod / canary).
  2. Pre-upgrade `Job` that snapshots `manifest` `ConfigMap` to a rotating ring of 5 backups.
  3. Post-upgrade `Job` that SIGHUPs the controller + every participant.
  4. RBAC `Role` + `RoleBinding` + dedicated `ServiceAccount` for the hooks (least-privilege).
  5. Bundle weight-loader sidecar template (structural placeholder, off by default).
  6. `PrometheusRule` template with 6 alerts (reload-failures, reload-latency, availability ×3, RELOADING-stuck).
  7. Grafana dashboard `ConfigMap` (auto-import via sidecar annotation).
  8. `PriorityClass` for the controller (`100000`).
  9. Per-tenant label propagation via `global.tenant`.
 10. `HorizontalPodAutoscaler` template (disabled by default, structural for PR #17).
 11. cert-manager `Issuer` + `Certificate` stubs (disabled by default).
 12. NetworkPolicy egress allow for the hook `Job`s.
 13. Schema additions (11 new keys + 28 sub-keys).
 14. ~ 18 new `helm-unittest` assertions covering PR #14 fields.
 15. 2 new scenario values files + 2 new golden snapshots (`kovant-prod.yaml`, `kovant-hooks.yaml`).
 16. Operator runbook extensions (11 scenarios).

PR #14 does **not** change :

  * The proto contract (PR #5).
  * The Phase-1 algorithm (`MetaSwarm::iterate`) — unchanged.
  * The Rust binaries (PR #6 / #8 / #9 / #12) — only the chart evolves.
  * PR #13's existing templates (additive only, except `controller-deployment.yaml`
    which gains the sidecar Container behind an `if` guard).

---

## 2. Pre-flight checks

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | PR #13 merged into `main` (sub-chart skeleton present) | git log | merge SHA on main |
| PF2 | `helm lint charts/efe-stack/charts/efe-meta-swarm/` clean on `main` | CI | exit 0 |
| PF3 | `helm template tst charts/efe-stack/charts/efe-meta-swarm -f tests/scenarios/kovant-3p.values.yaml` succeeds | CI | exit 0 |
| PF4 | All 23 PR #13 `helm-unittest` tests pass on `main` | CI | green |
| PF5 | `kubeconform -strict` clean on all 4 PR #13 golden snapshots | CI | 51/51 valid |
| PF6 | `Chart.lock` is in sync with `Chart.yaml` (PR #13 added efe-meta-swarm dep) | CI | digest matches |
| PF7 | Parent `efe-server` Deployment renders `EFE_META_SWARM_*` env vars when `global.metaSwarm.enabled = true` | CI | 5 env vars |
| PF8 | `monitoring.coreos.com/v1` CRDs documented as opt-in (operator pre-req) | manual | README clear |
| PF9 | `cert-manager.io/v1` CRDs documented as opt-in (operator pre-req) | manual | README clear |
| PF10 | `kubectl` version compat 1.28-1.30 | manual | kubeVersion matches |
| PF11 | Threat model (PR #7) v1.0 + T25-T28 sec sign-off | sec | green |
| PF12 | `helm-unittest` plugin v0.5.1 supports new assertion types we'll add (`hasDocuments`, `failedTemplate`, `contains`) | manual | docs verified |

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr14-helm-values-templates
```

Branched from `main` after PR #13 merge SHA.

### 3.2 Commit-by-commit plan (8 commits, each CI-green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| **C1** | `feat(charts): efe-meta-swarm multi-env values flavors (dev / staging / prod / canary)` | `+ examples/values-dev.yaml`, `+ examples/values-staging.yaml`, `+ examples/values-prod.yaml`, `+ examples/values-canary.yaml`, `+ examples/README.md`, `~ README.md` (link to examples) | +280 / -5 | `helm template … -f examples/values-prod.yaml` + golden diff |
| **C2** | `feat(charts): efe-meta-swarm PriorityClass + HPA placeholder + per-tenant labels` | `+ templates/priorityclass-controller.yaml`, `+ templates/hpa-controller.yaml`, `~ templates/_helpers.tpl` (+ tenant label propagation), `~ templates/controller-deployment.yaml` (+ priorityClassName ref) | +130 / -3 | `helm-unittest` (3 new) ; `kubeconform` |
| **C3** | `feat(charts): pre-upgrade manifest backup Job + rotating ConfigMap ring` | `+ templates/hooks/pre-upgrade-snapshot.yaml`, `+ templates/hooks/_hook-helpers.tpl` (shared between hooks) | +180 / 0 | `helm-unittest` (4 new) + golden diff with `hooks.enabled=true` |
| **C4** | `feat(charts): post-upgrade SIGHUP-all Job + dedicated SA / Role / RoleBinding` | `+ templates/hooks/post-upgrade-sighup.yaml`, `+ templates/hooks/hook-serviceaccount.yaml`, `+ templates/hooks/hook-role.yaml`, `+ templates/hooks/hook-rolebinding.yaml` | +220 / 0 | `helm-unittest` (5 new) + RBAC kubeconform |
| **C5** | `feat(charts): bundle weight-loader sidecar (structural placeholder)` | `~ templates/participant-deployment.yaml` (+ sidecar container behind `if` guard + emptyDir volume + shareProcessNamespace), `+ templates/_sidecar-helpers.tpl` | +150 / -2 | `helm-unittest` (3 new) ; rendered Pod kubeconform |
| **C6** | `feat(charts): PrometheusRule alerts (6 alerts) + Grafana dashboard ConfigMap` | `+ templates/prometheusrule.yaml`, `+ templates/grafana-dashboard.yaml`, `+ files/grafana-dashboard.json` (the embedded JSON) | +320 / 0 | `helm-unittest` (4 new) + golden diff with `alerts.enabled=true` + CRD-aware kubeconform |
| **C7** | `feat(charts): TLS bootstrap stubs (cert-manager Issuer + Certificate)` | `+ templates/tls-issuer.yaml`, `+ templates/tls-certificate.yaml`, `~ templates/controller-deployment.yaml` (+ TLS volume mounts + env vars under `if tls.enabled`), `~ templates/participant-deployment.yaml` (same) | +180 / 0 | `helm-unittest` (3 new) + CRD-aware kubeconform |
| **C8** | `feat(charts): updated schema + golden snapshots + CI integration + runbook` | `~ values.schema.json` (+ 11 keys, ~ 28 sub-keys), `~ values.yaml` (default values for new keys), `+ tests/scenarios/kovant-prod.values.yaml`, `+ tests/scenarios/kovant-hooks.values.yaml`, `+ tests/golden/kovant-prod.yaml`, `+ tests/golden/kovant-hooks.yaml`, `~ .github/workflows/helm-meta-swarm.yml` (+ new scenarios in CI), `~ README.md` (+ operator runbook extensions) | +540 / -20 | golden diff + kubeconform + Polaris + helm-unittest |

**Total : +2 000 / -30 = +1 970 LOC net.**

### 3.3 Drafting strategy

  * **C1** adds operator-facing flavors but doesn't change templates ; revertable in isolation.
  * **C2** adds two new templates (PriorityClass + HPA) + label refactor ; PriorityClass is referenced by controller-deployment so it must land first or be guarded.
  * **C3 / C4** are the hooks pair ; C4 depends on C3's `_hook-helpers.tpl`.
  * **C5** modifies the participant deployment in-place ; the sidecar `if`-guard makes the diff small.
  * **C6** is purely additive (PrometheusRule + Grafana ConfigMap) ; isolated revert.
  * **C7** is additive + 2 deployment template tweaks (TLS volume mounts) under `if`-guards.
  * **C8** is the closure : schema + golden + CI + runbook.

If C5 (sidecar) flakes — e.g., `shareProcessNamespace` is rejected by a cluster's PodSecurityPolicy — revert C5 alone ; the rest of PR #14 stays in trunk.

If C6 (PrometheusRule) flakes — e.g., CRDs missing on CI — gate on `alerts.enabled=false` by default ; CI uses `alerts.enabled=true` scenarios with CRD-aware `kubeconform`.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | Ship multi-env values flavors as files under `examples/` inside the sub-chart, NOT as parent-chart values overlays | Atomic with the chart ; versioned alongside the templates ; operators clone + override per cluster | Parent-chart `values-*.yaml` ; external GitOps repo overlays | Reversible — promotion to GitOps overlay is mechanical |
| **AD2** | Pre-upgrade `Job` snapshots `manifest` `ConfigMap` to `backup-{0..4}` rotating ring keyed by hook-time | Operator-side audit trail without depending on K8s `EventList` retention ; ring of 5 = ~5 last reloads ; rolls naturally | Persistent volume ; ConfigMap with chronological list ; ObjectBucket | Reversible — ring size is `values.yaml` configurable |
| **AD3** | Post-upgrade `Job` uses `kubectl exec ... -- kill -HUP 1` against each pod, NOT a direct K8s API call | Re-uses PR #12's SIGHUP-driven reload path ; the binary's signal handler is the canonical reload trigger ; no new gRPC surface needed | gRPC `MetaSwarmService::ReloadConfig` call ; force pod restart | Reversible — Phase 3 may add an in-cluster watcher that calls the RPC directly |
| **AD4** | Hooks run as a **dedicated** `ServiceAccount` with a minimal `Role` (only `configmaps:get,create,update` + `pods:exec` scoped to the chart's labels), NOT the default SA | Least-privilege ; T25 mitigation ; hooks SA is auditable in K8s `EventList` | Use chart's controller SA (over-privileged) ; use a `ClusterRole` (too broad) | Reversible — RBAC consolidation is one-line |
| **AD5** | Hooks `Job` `helm.sh/hook-delete-policy: before-hook-creation,hook-succeeded` — auto-clean | Standard Helm idiom ; keeps the namespace tidy ; failed Jobs stick around for debugging | `keep-on-success` ; `delete-on-success` | Reversible |
| **AD6** | Hooks are **non-blocking** : a failure does NOT roll back the chart upgrade | Operator preserves the manifest change even if the SIGHUP didn't land ; can manually re-trigger ; matches PR #12's transactional design (rejected reload leaves state intact) | Fail-fast ; auto-rollback via `--atomic` | Reversible — Phase 3 may add an `--atomic` integration with explicit hook failure modes |
| **AD7** | Bundle weight-loader sidecar uses `shareProcessNamespace: true` on the pod | Sidecar needs to signal the participant via `kill -HUP <pid>` ; without `shareProcessNamespace`, sidecar's `/proc` is isolated | gRPC reload RPC ; file-watcher in the participant (Phase 3) ; `kubectl exec` from the sidecar (RBAC) | Reversible — Phase 3 may replace with a file-watcher when participants gain `inotify` ; T26 trust boundary docs the implication |
| **AD8** | Sidecar image defaults to `gcr.io/google-containers/pause:3.9` (no-op) when `bundle.sidecar.enabled = true` and no override | Lets the operator opt-in to the sidecar layout (Pod gets the second container + shared volume) WITHOUT running real bundle-loader logic — useful for Phase-2 dry-runs ; PR #15 swaps `pause` for the real binary | Skip the sidecar entirely until PR #15 ; require explicit image override | Reversible — image is `values.yaml` configurable |
| **AD9** | Sidecar shares an `emptyDir` volume at `/var/lib/efe-meta-swarm/weights/` (Memory medium) | Volatile (pod-restart wipes) ; OK for Phase-2 since weights re-download on every boot ; `Memory` medium avoids disk wear | `Persistent volume` (overkill ; per-pod state) ; `hostPath` (security risk) | Reversible — `Persistent volume` migration is one PR |
| **AD10** | `PrometheusRule` ships **6 alerts** : ReloadFailureRate (warning), ReloadLatencyP99High (warning), ReloadingStuck (critical), ControllerDown (critical), ParticipantDown (warning, per-participant), ReloadGenNotAdvancing (info — operator forgot SIGHUP) | Covers PR #12's reload metric surface + per-binary availability ; severities aligned with the `runbook/` taxonomy | More alerts ; fewer alerts ; only `for: 5m` durations | Reversible — each alert is configurable per-environment |
| **AD11** | Grafana dashboard ships as a `ConfigMap` with annotation `grafana_dashboard: "1"` | Grafana sidecar auto-import standard ; works with `kube-prometheus-stack` defaults ; no `Dashboard` CR required | Direct API push ; `Dashboard` CR (Grafana Operator) | Reversible — Grafana Operator integration is a Phase-3 PR |
| **AD12** | Dashboard JSON is **embedded** in the chart's `files/grafana-dashboard.json`, loaded via `.Files.Get` | Versioned with the chart ; reproducible ; auditable | Fetched from a URL ; auto-generated from rules | Reversible |
| **AD13** | `PriorityClass` `value: 100000` for the controller | Higher than default user pods (1000) ; below system-critical (200000+) ; controller gets to evict participants if cluster runs OOM | `system-cluster-critical` (too high) ; no priority (defeats the point) | Reversible per-environment |
| **AD14** | `HorizontalPodAutoscaler` template ships but `enabled: false` by default ; structurally locked to controller (NOT participants — they're stateful per AD12 of PR #13) | PR #17 leader election will turn HPA on for the controller ; Phase 2 doesn't need it ; ship the template to lock the schema | Don't ship the template ; ship one for participants too | Reversible — flipping `enabled: true` exposes the surface |
| **AD15** | TLS via cert-manager `Issuer` (`type: SelfSigned` for Phase 2 ; PR #16 promotes to `CA`) + `Certificate` per role | Standard cluster pattern ; PR #16 wires the binary's tonic-tls side | Self-signed via Helm pre-install hook ; static `Secret`s | Reversible — `Issuer` type is `values.yaml` configurable |
| **AD16** | TLS material mounted at `/etc/efe-meta-swarm/tls/{tls.crt,tls.key,ca.crt}` | Matches PR #6's `EFE_META_SWARM_TLS_CERT/_KEY/_CA` env var paths | Different mount path | Reversible (env vars also configurable) |
| **AD17** | Per-tenant label `efe.io/tenant: <slug>` inherited from `global.tenant` (umbrella) on every resource | Multi-tenant kubectl queries ; matches sibling sub-charts | No tenant label ; only on pods | Reversible |
| **AD18** | The hook `Job`s' image defaults to `bitnami/kubectl:1.28` (pinned tag) | Most common `kubectl` image ; pinned to align with `kubeVersion` in `Chart.yaml`  | Custom-built image with `kubectl` + `yq` | Reversible — image is `values.yaml` configurable |
| **AD19** | Backup `ConfigMap` ring : 5 entries, rotated on each `pre-upgrade` Job run via `kubectl annotate` for the index | 5 = ~ 5 last reloads = ~ 1 week at 1 reload/day ; rolling FIFO ; index in annotation `efe.io/backup-index` | More entries (more etcd usage) ; fewer (less history) | Reversible — ring size is `values.yaml` configurable |
| **AD20** | Backup `ConfigMap`s carry the **same** labels as the live manifest (`efe.io/meta-swarm-role: shared`) + an extra `efe.io/manifest-state: backup` for filtering | Operator queries : `kubectl get cm -l efe.io/manifest-state=backup` ; live ConfigMap has `efe.io/manifest-state: live` | One label only ; separate namespace | Reversible |
| **AD21** | Post-upgrade SIGHUP `Job` runs serially (one pod at a time) | Operator can `kubectl logs job/post-upgrade-sighup` and see the per-pod sequence ; serial is simpler than parallel reaping ; SIGHUP itself is fast (< 100 ms) per pod | Parallel via `xargs -P` | Reversible — parallel is one-line if `values.yaml` flag flips |
| **AD22** | Post-upgrade `Job` ignores SIGHUP failures (e.g., pod not ready) via `\|\| true` and emits a warning to `kubectl logs` | Operator can re-run `kubectl exec ... kill -HUP 1` manually if a pod missed ; non-blocking matches AD6 | Fail-fast (rejected — AD6) | Reversible — fail-fast is one-line |
| **AD23** | The sidecar's `ServiceAccount` is the same as the participant's (default) | No extra K8s API access needed for Phase 2 (sidecar is a no-op `pause`) ; PR #15 may grant `configmaps:get` for the bundle metadata CM | Dedicated SA per sidecar | Reversible |
| **AD24** | `bundle.sidecar.resources` are **independent** from the participant's resources | Bundle-poller is small + steady-state ; participant is bursty ; K8s scheduling sums them but operators size each independently | Pool with the participant | Reversible |
| **AD25** | Helm hook ordering : `pre-upgrade` weight `-10` (early) ; `post-upgrade` weight `+10` (late) ; both `hook-delete-policy: before-hook-creation,hook-succeeded` | Pre-upgrade snapshot MUST run before the actual upgrade installs the new ConfigMap (negative weight → earlier) ; post-upgrade SIGHUP must run AFTER all resources are in place (positive weight → later) | No weights ; tighter weights | Reversible |
| **AD26** | Multi-env values flavors live under `examples/` (NOT `tests/`) | `examples/` is a Helm convention for operator-facing samples ; `tests/` is for CI assertions | `flavors/` ; `overlays/` | Reversible |
| **AD27** | `examples/values-canary.yaml` ships a **smaller-but-isolated** topology (1 controller + 1 participant for canary tenant slice) | Demonstrates the per-tenant pattern + smaller blast radius | No canary example ; full kovant + canary tenant overlay | Reversible |
| **AD28** | Hook `Job` failures emit K8s `Event`s with `reason: ReloadHookFailed` | Operators can `kubectl describe job/post-upgrade-sighup` to see why ; integrates with `kubectl get events --field-selector reason=ReloadHookFailed` | Only logs | Reversible |
| **AD29** | `PrometheusRule` alerts include `runbook_url` annotation pointing at the operator runbook section | Pagerduty / Alertmanager integration shows the runbook directly to the on-call | No URL ; hardcoded text | Reversible |
| **AD30** | Grafana dashboard auto-import uses **only** the `grafana_dashboard: "1"` annotation, NOT `Dashboard` CR | Compat with `kube-prometheus-stack`'s default Grafana sidecar ; works without Grafana Operator | Use both | Reversible |
| **AD31** | `HorizontalPodAutoscaler` is locked to `minReplicas: 1, maxReplicas: 1` in PR #14 (when `hpa.enabled=true` it's a no-op) | Phase 2 single-controller invariant ; lets PR #17 just flip the bounds | Skip the template ; allow > 1 replicas in PR #14 | Reversible — PR #17 changes the bounds |
| **AD32** | Operator runbook integrated in chart's `README.md` (NOT a separate `RUNBOOK.md`) | Single source of truth ; artifacthub.io renders the README | Separate file ; both | Reversible |

---

## 5. Topology + integration diagram

### 5.1 Resource graph after PR #14 (compared to PR #13)

```
                  ┌────────────────────────────────────────────────┐
                  │   Helm release : efe-stack                      │
                  │   Namespace    : efe-prod                       │
                  └─┬──────────────────────────────────────────────┘
                    │
        ┌───────────┼────────────────────────────────────────────────────┐
        │           │                                                    │
        ▼           ▼                                                    ▼
  ┌───────────┐ ┌────────────┐                                  ┌────────────────┐
  │ConfigMap  │ │ServiceAcct │                                  │ NetworkPolicy   │
  │ manifest  │ │ × 4 (PR #13)│                                  │ × 5 (PR #13)    │
  │ (live)    │ │  + hooks-sa │ ◄── NEW PR #14                  │  + hook-egress  │ ◄── NEW PR #14
  └────┬──────┘ └────┬───────┘                                  └────────────────┘
       │             │
   (mount)       (subject)
       │             │
       ▼             ▼
  ┌─────────────────────────────────────────────────────────────────┐
  │ Deployment : controller                                          │
  │   priorityClassName : <release>-efe-meta-swarm-controller-prio  │ ◄── NEW PR #14
  │   labels : efe.io/tenant: <slug>                                │ ◄── NEW PR #14
  │   ┌──────────────────────────────────────────────────────────┐  │
  │   │ container : controller                                    │  │
  │   │   image    : ghcr.io/efe-ai/efe-meta-swarm-server:0.2.0   │  │
  │   │   args     : ["controller"]                              │  │
  │   │   env      : ...                                          │  │
  │   │   volumeMounts:                                           │  │
  │   │     - manifest      (RO)                                  │  │
  │   │     - tls           (RO, IF tls.enabled)                  │ ◄── NEW PR #14
  │   └──────────────────────────────────────────────────────────┘  │
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ Deployment : participant-<id>                                    │
  │   shareProcessNamespace : true (IF bundle.sidecar.enabled)      │ ◄── NEW PR #14
  │   labels : efe.io/tenant: <slug>                                │ ◄── NEW PR #14
  │   ┌──────────────────────────────────────────────────────────┐  │
  │   │ container : participant                                   │  │
  │   │   ...                                                     │  │
  │   │   volumeMounts:                                           │  │
  │   │     - manifest      (RO)                                  │  │
  │   │     - weights       (RW, IF bundle.sidecar.enabled)       │ ◄── NEW PR #14
  │   │     - tls           (RO, IF tls.enabled)                  │ ◄── NEW PR #14
  │   ├──────────────────────────────────────────────────────────┤  │
  │   │ container : bundle-loader  (sidecar, IF enabled)          │ ◄── NEW PR #14
  │   │   image    : gcr.io/google-containers/pause:3.9 (Phase 2) │  │
  │   │   volumeMounts:                                           │  │
  │   │     - weights       (RW)                                  │  │
  │   └──────────────────────────────────────────────────────────┘  │
  │   volumes:                                                       │
  │     - emptyDir (medium: Memory) named `weights` (IF sidecar)     │ ◄── NEW PR #14
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ Helm hooks (Job kind, ServiceAccount: hooks-sa)                  │ ◄── NEW PR #14
  │                                                                  │
  │  helm.sh/hook: pre-upgrade  (weight -10)                         │
  │   Job : pre-upgrade-snapshot                                     │
  │     ─ Read live manifest ConfigMap                               │
  │     ─ Compute next backup index (annotation-driven)              │
  │     ─ Create/update <release>-efe-meta-swarm-manifest-backup-N   │
  │                                                                  │
  │  helm.sh/hook: post-upgrade (weight +10)                         │
  │   Job : post-upgrade-sighup                                      │
  │     ─ kubectl exec deploy/<controller> -- kill -HUP 1            │
  │     ─ for each participant : kubectl exec ... -- kill -HUP 1     │
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ Observability  (IF alerts.enabled / dashboard.enabled)           │ ◄── NEW PR #14
  │                                                                  │
  │  PrometheusRule  : 6 alerts                                      │
  │   - MetaSwarmReloadFailureRate           (warning)               │
  │   - MetaSwarmReloadLatencyP99High        (warning)               │
  │   - MetaSwarmReloadingStuck              (critical)              │
  │   - MetaSwarmControllerDown              (critical)              │
  │   - MetaSwarmParticipantDown             (warning, per-id)       │
  │   - MetaSwarmReloadGenNotAdvancing       (info)                  │
  │                                                                  │
  │  ConfigMap : grafana-dashboard                                   │
  │   annotations:                                                    │
  │     grafana_dashboard: "1"   (sidecar auto-import marker)        │
  │   data:                                                           │
  │     dashboard.json : <embedded from files/grafana-dashboard.json>│
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ TLS (IF tls.enabled)                                             │ ◄── NEW PR #14
  │  cert-manager Issuer  (SelfSigned for Phase 2)                   │
  │  cert-manager Certificate × 4 (controller + 3 participants)      │
  │   secretName : <release>-efe-meta-swarm-<role>-tls               │
  │   commonName : <release>-efe-meta-swarm-<role>.<ns>.svc          │
  └─────────────────────────────────────────────────────────────────┘
```

### 5.2 Helm upgrade lifecycle (with PR #14 hooks)

```
operator              helm                   K8s API           pods
   │                    │                      │                  │
   │ helm upgrade … -f values-prod.yaml -f examples/values-prod.yaml
   │───────────────────►│                      │                  │
   │                    │ template + lint      │                  │
   │                    │                      │                  │
   │                    │ HOOK PHASE 1 :       │                  │
   │                    │   pre-upgrade        │                  │
   │                    │     weight -10       │                  │
   │                    │     Job : pre-upgrade-snapshot          │
   │                    │     ─────────────────►                  │
   │                    │                      ▼                  │
   │                    │              [snapshot pod runs]        │
   │                    │              ▶ reads live manifest CM   │
   │                    │              ▶ computes next backup idx │
   │                    │              ▶ creates backup-N CM      │
   │                    │              ◀ exits 0                  │
   │                    │              Job deleted (hook-policy)  │
   │                    │                                          │
   │                    │ UPGRADE PHASE :                          │
   │                    │   apply manifests (Deployment etc.)      │
   │                    │     PATCH ConfigMap (new manifest)       │
   │                    │     PATCH Deployment (new image / env)   │
   │                    │     ... etc                              │
   │                    │                                          │
   │                    │ HOOK PHASE 2 :                           │
   │                    │   post-upgrade                           │
   │                    │     weight +10                           │
   │                    │     Job : post-upgrade-sighup            │
   │                    │     ─────────────────►                   │
   │                    │              [sighup pod runs]           │
   │                    │              ▶ exec controller -- kill -HUP 1
   │                    │              ────────────────────────────►│
   │                    │                                  SIGHUP → │
   │                    │                              apply_reload │
   │                    │                              gen ++       │
   │                    │              ◀ ok                         │
   │                    │              ▶ for p in participants:     │
   │                    │                  exec p -- kill -HUP 1   │
   │                    │                  ─────────────────────────►
   │                    │                                  SIGHUP → │
   │                    │                              apply_reload │
   │                    │                              gen ++       │
   │                    │              ◀ ok × 3                     │
   │                    │              ▶ exits 0                    │
   │                    │              Job deleted (hook-policy)    │
   │                    │                                            │
   │  Release : upgraded │                                            │
   │  NOTES.txt printed  │                                            │
   │◄────────────────────│                                            │
```

### 5.3 Bundle sidecar polling (Phase-3 contract, locked at PR #14)

```
participant pod
├── container : participant (PID 1, args=[participant])
│     reads /var/lib/efe-meta-swarm/weights/current.bin at boot
│     reloads on SIGHUP (re-reads from same path)
│
├── container : bundle-loader (PID 2, args=[poll])  ◄── PR #14 stub, PR #15 fills
│     loop {
│       sleep 60s
│       GET <bundle-endpoint>/metadata?id=<participant-id>
│       if sha256 != last_seen_sha256 {
│         GET <bundle-endpoint>/blob?id=<participant-id> > /tmp/new.bin
│         mv /tmp/new.bin /var/lib/efe-meta-swarm/weights/current.bin
│         kill -HUP 1   # ← signals the participant (shareProcessNamespace)
│         last_seen_sha256 = new_sha256
│       }
│     }
│
└── volumes:
      manifest  : ConfigMap mount (RO)         — controller-driven
      weights   : emptyDir (medium=Memory)     — sidecar-driven
      tls       : Secret mount (RO, optional)  — cert-manager-driven
```

---

## 6. Layout — files added + modified

### 6.1 New files (16)

```
charts/efe-stack/charts/efe-meta-swarm/
├── examples/                                          [NEW]
│   ├── README.md                                      [NEW : ~ 80 LOC]
│   ├── values-dev.yaml                                [NEW : ~ 60 LOC]
│   ├── values-staging.yaml                            [NEW : ~ 80 LOC]
│   ├── values-prod.yaml                               [NEW : ~ 110 LOC]
│   └── values-canary.yaml                             [NEW : ~ 90 LOC]
├── files/                                             [NEW]
│   └── grafana-dashboard.json                         [NEW : ~ 600 LOC JSON]
├── templates/
│   ├── _sidecar-helpers.tpl                           [NEW : ~ 50 LOC]
│   ├── priorityclass-controller.yaml                  [NEW : ~ 25 LOC]
│   ├── hpa-controller.yaml                            [NEW : ~ 45 LOC]
│   ├── prometheusrule.yaml                            [NEW : ~ 150 LOC]
│   ├── grafana-dashboard.yaml                         [NEW : ~ 30 LOC]
│   ├── tls-issuer.yaml                                [NEW : ~ 35 LOC]
│   ├── tls-certificate.yaml                           [NEW : ~ 90 LOC, ranges over controller + participants]
│   └── hooks/                                         [NEW]
│       ├── _hook-helpers.tpl                          [NEW : ~ 40 LOC]
│       ├── hook-serviceaccount.yaml                   [NEW : ~ 25 LOC]
│       ├── hook-role.yaml                             [NEW : ~ 50 LOC]
│       ├── hook-rolebinding.yaml                      [NEW : ~ 30 LOC]
│       ├── pre-upgrade-snapshot.yaml                  [NEW : ~ 110 LOC]
│       └── post-upgrade-sighup.yaml                   [NEW : ~ 100 LOC]
└── tests/
    ├── hooks_test.yaml                                [NEW : ~ 90 LOC, 5 helm-unittest assertions]
    ├── observability_test.yaml                        [NEW : ~ 70 LOC, 4 assertions]
    ├── sidecar_test.yaml                              [NEW : ~ 60 LOC, 3 assertions]
    ├── tls_test.yaml                                  [NEW : ~ 70 LOC, 3 assertions]
    └── tenant_labels_test.yaml                        [NEW : ~ 50 LOC, 3 assertions]
└── tests/scenarios/
    ├── kovant-prod.values.yaml                        [NEW : ~ 30 LOC]
    └── kovant-hooks.values.yaml                       [NEW : ~ 20 LOC]
└── tests/golden/
    ├── kovant-prod.yaml                               [NEW : ~ 1 200 LOC, helm template render]
    └── kovant-hooks.yaml                              [NEW : ~ 1 000 LOC]
```

### 6.2 Modified files (8)

```
charts/efe-stack/charts/efe-meta-swarm/
├── Chart.yaml                                         [MODIFIED : appVersion bump 0.2.0 → 0.3.0, chart version 0.1.0 → 0.2.0]
├── values.yaml                                        [MODIFIED : + 110 LOC for new top-level keys]
├── values.schema.json                                 [MODIFIED : + 230 LOC for 11 new top-level keys + 28 sub-keys]
├── README.md                                          [MODIFIED : + 200 LOC operator runbook extensions + examples links]
├── templates/
│   ├── _helpers.tpl                                   [MODIFIED : + 20 LOC for tenant labels]
│   ├── controller-deployment.yaml                     [MODIFIED : + 30 LOC for priorityClassName + TLS volume mounts (if-guarded)]
│   ├── participant-deployment.yaml                    [MODIFIED : + 50 LOC for sidecar container + shareProcessNamespace + TLS mounts]
│   └── networkpolicy.yaml                             [MODIFIED : + 40 LOC for hook Job egress allow + tenant slice]

.github/workflows/helm-meta-swarm.yml                  [MODIFIED : + 60 LOC for kovant-prod + kovant-hooks scenarios]
```

### 6.3 LOC tally

| Category | LOC |
|---|---|
| New examples + dashboard JSON | ~ 1 020 |
| New templates (priorityclass + hpa + prometheusrule + grafana CM + tls + hooks) | ~ 690 |
| New helm-unittest assertions | ~ 340 |
| New scenarios + golden snapshots | ~ 2 250 |
| Modified production templates | ~ 140 |
| Modified schema + values + README | ~ 540 |
| Modified CI workflow | ~ 60 |
| **Total new + modified** | **~ 5 040 LOC** (heavily test/snapshot weighted) |

Production-relevant net new : **~ 1 870 LOC** of YAML + JSON + JSON Schema.

---

## 7. Multi-environment values flavors

### 7.1 `examples/values-dev.yaml` — local development cluster

```yaml
# Local dev cluster — minikube / kind / Docker Desktop K8s.
# Optimised for fast iteration ; observability + hardening relaxed.
enabled: true

image:
  tag: "0.3.0-snapshot"
  pullPolicy: Always           # local rebuilds need pulls

manifest:
  meta:
    dim: 8                     # smaller for fast convergence
    safety:
      kind: box
      radius: 100.0
  iteration:
    max_iters: 50              # fast convergence in dev
    tolerance: 1.0e-3          # looser for speed
    alpha_default: 0.5
    coupling_gamma: 0.2
    max_wall_ms: 5000

controller:
  config:
    reflectionEnabled: true    # `grpcurl list` works
    maxInflight: 5
  resources:
    requests: {cpu: 100m, memory: 128Mi}
    limits:   {cpu: 500m, memory: 512Mi}

# Smaller participants for laptop-class clusters.
participants:
  - id: supplier-scoring
    dim: 4
    kappa: 0.6
    seed: 11
    service: {grpcPort: 50091, metricsPort: 9101}
    resources:
      requests: {cpu: 50m, memory: 64Mi}
      limits:   {cpu: 200m, memory: 256Mi}
  - id: contract-risk
    dim: 4
    kappa: 0.7
    seed: 22
    service: {grpcPort: 50092, metricsPort: 9102}
    resources:
      requests: {cpu: 50m, memory: 64Mi}
      limits:   {cpu: 200m, memory: 256Mi}
  - id: spend-categorization
    dim: 4
    kappa: 0.5
    seed: 33
    service: {grpcPort: 50093, metricsPort: 9103}
    resources:
      requests: {cpu: 50m, memory: 64Mi}
      limits:   {cpu: 200m, memory: 256Mi}

networkPolicy:
  enabled: false               # most dev clusters lack policy CNI

metrics:
  serviceMonitor:
    enabled: false             # likely no prometheus-operator

alerts:
  enabled: false
dashboard:
  enabled: false

hooks:
  enabled: false               # manual SIGHUP is the dev workflow

bundle:
  sidecar:
    enabled: false

tls:
  enabled: false
```

### 7.2 `examples/values-staging.yaml` — staging cluster (full surface, smaller scale)

```yaml
enabled: true

image:
  tag: "0.3.0"

manifest:
  meta:
    dim: 16
    safety:
      kind: box
      radius: 50.0
  iteration:
    max_iters: 100
    tolerance: 1.0e-4
    alpha_default: 0.5
    coupling_gamma: 0.2
    max_wall_ms: 15000

controller:
  resources:
    requests: {cpu: 500m, memory: 512Mi}
    limits:   {cpu: 2,    memory: 2Gi}

participants:
  - id: supplier-scoring
    dim: 16
    kappa: 0.6
    seed: 11
    service: {grpcPort: 50091, metricsPort: 9101}
    resources:
      requests: {cpu: 300m, memory: 384Mi}
      limits:   {cpu: 1,    memory: 1Gi}
  - id: contract-risk
    dim: 16
    kappa: 0.7
    seed: 22
    service: {grpcPort: 50092, metricsPort: 9102}
    resources:
      requests: {cpu: 300m, memory: 384Mi}
      limits:   {cpu: 1,    memory: 1Gi}
  - id: spend-categorization
    dim: 16
    kappa: 0.5
    seed: 33
    service: {grpcPort: 50093, metricsPort: 9103}
    resources:
      requests: {cpu: 300m, memory: 384Mi}
      limits:   {cpu: 1,    memory: 1Gi}

networkPolicy:
  enabled: true

metrics:
  serviceMonitor:
    enabled: true
    interval: 15s              # tighter scrape in staging

alerts:
  enabled: true
  severityOverride:
    ReloadFailureRate: warning   # not critical in staging

dashboard:
  enabled: true

hooks:
  enabled: true
  preUpgrade:
    snapshotEnabled: true
  postUpgrade:
    sighupEnabled: true

bundle:
  sidecar:
    enabled: false              # weights from PR #15+

tls:
  enabled: false                # PR #16 will flip
```

### 7.3 `examples/values-prod.yaml` — production cluster

```yaml
enabled: true

image:
  tag: "0.3.0"                  # pinned ; no :latest
  pullPolicy: IfNotPresent

manifest:
  meta:
    dim: 32                     # production dimension
    safety:
      kind: box
      radius: 100.0
  iteration:
    max_iters: 100
    tolerance: 1.0e-5           # tighter for prod
    alpha_default: 0.5
    coupling_gamma: 0.2
    max_wall_ms: 20000

controller:
  config:
    maxInflight: 20
  resources:
    requests: {cpu: 1,   memory: 1Gi}
    limits:   {cpu: 4,   memory: 4Gi}
  podDisruptionBudget:
    enabled: true
    maxUnavailable: 0
  nodeSelector:
    workload: meta-swarm
  tolerations:
    - key: workload
      operator: Equal
      value: meta-swarm
      effect: NoSchedule

priorityClass:
  enabled: true
  value: 100000

participants:
  - id: supplier-scoring
    dim: 32
    kappa: 0.6
    seed: 11
    service: {grpcPort: 50091, metricsPort: 9101}
    resources:
      requests: {cpu: 1,   memory: 1Gi}
      limits:   {cpu: 2,   memory: 2Gi}
    nodeSelector:
      workload: meta-swarm
    tolerations:
      - {key: workload, operator: Equal, value: meta-swarm, effect: NoSchedule}
  - id: contract-risk
    dim: 32
    kappa: 0.7
    seed: 22
    service: {grpcPort: 50092, metricsPort: 9102}
    resources:
      requests: {cpu: 1,   memory: 1Gi}
      limits:   {cpu: 2,   memory: 2Gi}
    nodeSelector:
      workload: meta-swarm
    tolerations:
      - {key: workload, operator: Equal, value: meta-swarm, effect: NoSchedule}
  - id: spend-categorization
    dim: 32
    kappa: 0.5
    seed: 33
    service: {grpcPort: 50093, metricsPort: 9103}
    resources:
      requests: {cpu: 1,   memory: 1Gi}
      limits:   {cpu: 2,   memory: 2Gi}
    nodeSelector:
      workload: meta-swarm
    tolerations:
      - {key: workload, operator: Equal, value: meta-swarm, effect: NoSchedule}

networkPolicy:
  enabled: true
  ingressNamespaceSelector:
    matchLabels:
      kubernetes.io/metadata.name: efe-prod
  prometheusNamespaceSelector:
    matchLabels:
      kubernetes.io/metadata.name: monitoring

metrics:
  serviceMonitor:
    enabled: true
    interval: 30s
    labels:
      prometheus: efe-prod

alerts:
  enabled: true
  severity:
    ReloadFailureRate: critical
    ReloadLatencyP99High: warning
    ReloadingStuck: critical
    ControllerDown: critical
    ParticipantDown: warning
    ReloadGenNotAdvancing: info
  runbookBaseUrl: "https://efe.example.com/runbook/meta-swarm"

dashboard:
  enabled: true

hooks:
  enabled: true
  preUpgrade:
    snapshotEnabled: true
    backupRingSize: 5
  postUpgrade:
    sighupEnabled: true
    serial: true                # one pod at a time in prod
    ignoreFailures: false       # prod: fail-fast on SIGHUP failure

bundle:
  sidecar:
    enabled: false              # PR #15+ will flip

tls:
  enabled: false                # PR #16 will flip ; certs via cert-manager
```

### 7.4 `examples/values-canary.yaml` — canary tenant slice

A SECOND release of the sub-chart with a DIFFERENT release name, deployed
alongside the main one to test new manifest / new image on a sliver of
production traffic.

```yaml
enabled: true

image:
  tag: "0.3.1-canary"           # newer image than prod

# Distinguishing labels so kubectl can target canary vs main.
global:
  tenant: kovant-canary

# Canary slice : 1 controller + 1 participant only (small blast radius).
manifest:
  meta:
    dim: 16
    safety:
      kind: box
      radius: 50.0
  iteration:
    max_iters: 100
    tolerance: 1.0e-4
    alpha_default: 0.55          # slightly different α — the canary's reason for being
    coupling_gamma: 0.2
    max_wall_ms: 15000

controller:
  resources:
    requests: {cpu: 250m, memory: 256Mi}
    limits:   {cpu: 1,    memory: 1Gi}

# Single participant for the canary slice.
participants:
  - id: canary-scoring
    dim: 16
    kappa: 0.6
    seed: 11
    service: {grpcPort: 50091, metricsPort: 9101}
    resources:
      requests: {cpu: 200m, memory: 256Mi}
      limits:   {cpu: 500m, memory: 512Mi}

networkPolicy:
  enabled: true
  # Different ingressNamespaceSelector — only efe-canary clients
  # can reach the canary controller.
  ingressNamespaceSelector:
    matchLabels:
      kubernetes.io/metadata.name: efe-canary

metrics:
  serviceMonitor:
    enabled: true
    labels:
      prometheus: efe-canary    # routed to canary's Prometheus

alerts:
  enabled: true

hooks:
  enabled: true                 # canary still wants auto-reload

bundle:
  sidecar:
    enabled: false

tls:
  enabled: false
```

**Operator deployment** :
```bash
# Main prod release.
$ helm install efe-stack ./charts/efe-stack -n efe-prod \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml

# Canary release alongside.
$ helm install efe-stack-canary ./charts/efe-stack -n efe-prod \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-canary.yaml
```

Two releases coexist in the same namespace, distinguished by release prefix
in resource names (`efe-stack-...` vs `efe-stack-canary-...`).

---

## 8. PriorityClass + HPA placeholder

### 8.1 `templates/priorityclass-controller.yaml`

```yaml
{{- if and .Values.enabled .Values.priorityClass.enabled }}
apiVersion: scheduling.k8s.io/v1
kind: PriorityClass
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}-priority
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
value: {{ .Values.priorityClass.value | default 100000 }}
globalDefault: false
description: |
  Priority class for the EFE meta-swarm controller pod.  Higher than
  default user workloads (1000) so the controller survives resource
  pressure ; lower than system-cluster-critical (2000000000) so it
  yields to kubelet / kube-proxy.
{{- end }}
```

### 8.2 Reference from `controller-deployment.yaml`

Inside the `Pod.spec` :
```yaml
{{- if .Values.priorityClass.enabled }}
priorityClassName: {{ include "efe-meta-swarm.controllerName" . }}-priority
{{- end }}
```

### 8.3 `templates/hpa-controller.yaml` (placeholder for PR #17)

```yaml
{{- if and .Values.enabled .Values.controller.hpa.enabled }}
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe-meta-swarm.controllerName" . }}
  # Phase-2 lock : min=max=1 (single controller).  PR #17 leader
  # election flips this.  Keeping the template in PR #14 means PR #17
  # only changes values, not adds a new template.
  minReplicas: 1
  maxReplicas: 1
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: {{ .Values.controller.hpa.targetCPUUtilization | default 70 }}
{{- end }}
```

Operators flipping `controller.hpa.enabled: true` in PR #14 get a no-op HPA.
PR #17 will bump `maxReplicas` to N.

---

## 9. Per-tenant label propagation

### 9.1 `_helpers.tpl` extension

```tpl
{{/*
  efe-meta-swarm.labels (PR #14 extension : + tenant label)
*/}}
{{- define "efe-meta-swarm.labels" -}}
helm.sh/chart: {{ include "efe-meta-swarm.chart" . }}
{{ include "efe-meta-swarm.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe
efe.io/tenant: {{ .Values.global.tenant | default "default" | quote }}
{{- with .Values.global.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end -}}
```

Every resource then carries `efe.io/tenant: <slug>` automatically.  Operators :

```bash
# All pods belonging to a tenant
$ kubectl get pods -l efe.io/tenant=kovant

# Only the canary slice
$ kubectl get pods -l efe.io/tenant=kovant-canary
```

---

## 10. Pre-upgrade Helm hook : backup `ConfigMap` snapshot

### 10.1 `templates/hooks/_hook-helpers.tpl`

```tpl
{{/*
  efe-meta-swarm.hookName — release-prefixed hook resource name.
*/}}
{{- define "efe-meta-swarm.hookName" -}}
{{- $root := index . 0 -}}
{{- $hook := index . 1 -}}
{{- printf "%s-hook-%s" (include "efe-meta-swarm.fullname" $root) $hook -}}
{{- end -}}

{{/*
  efe-meta-swarm.hookServiceAccountName
*/}}
{{- define "efe-meta-swarm.hookServiceAccountName" -}}
{{- printf "%s-hooks" (include "efe-meta-swarm.fullname" .) -}}
{{- end -}}
```

### 10.2 `templates/hooks/pre-upgrade-snapshot.yaml`

```yaml
{{- if and .Values.enabled .Values.hooks.enabled .Values.hooks.preUpgrade.snapshotEnabled }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "efe-meta-swarm.hookName" (list . "pre-upgrade-snapshot") }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
  annotations:
    "helm.sh/hook": pre-upgrade
    "helm.sh/hook-weight": "-10"
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  backoffLimit: 1
  ttlSecondsAfterFinished: 300
  template:
    metadata:
      labels:
        {{- include "efe-meta-swarm.labels" . | nindent 8 }}
        app.kubernetes.io/component: hook
    spec:
      restartPolicy: Never
      serviceAccountName: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
      automountServiceAccountToken: true     # hook needs API access
      enableServiceLinks: false
      securityContext:
        {{- include "efe-meta-swarm.commonPodSecurityContext" . | nindent 8 }}
      containers:
        - name: snapshot
          image: {{ .Values.hooks.image | quote }}
          imagePullPolicy: IfNotPresent
          securityContext:
            {{- include "efe-meta-swarm.commonContainerSecurityContext" . | nindent 12 }}
          resources:
            requests: {cpu: 50m, memory: 64Mi}
            limits:   {cpu: 200m, memory: 256Mi}
          command:
            - sh
            - -c
            - |
              set -eu
              NS="{{ .Release.Namespace }}"
              LIVE_CM="{{ include "efe-meta-swarm.fullname" . }}-manifest"
              RING_SIZE={{ .Values.hooks.preUpgrade.backupRingSize | default 5 }}

              # Compute next backup index : read live CM's annotation
              # `efe.io/backup-next-index` (default 0), increment modulo ring size.
              NEXT_IDX=$(kubectl -n "$NS" get cm "$LIVE_CM" \
                -o jsonpath='{.metadata.annotations.efe\.io/backup-next-index}' 2>/dev/null || echo "0")
              NEXT_IDX=${NEXT_IDX:-0}
              BACKUP_CM="${LIVE_CM}-backup-${NEXT_IDX}"

              echo "── Snapshot : live=$LIVE_CM → backup=$BACKUP_CM ──"

              # Read live ; create/update backup with same data + extra annotation.
              kubectl -n "$NS" get cm "$LIVE_CM" -o json \
                | jq --arg name "$BACKUP_CM" --arg idx "$NEXT_IDX" '
                    .metadata.name = $name
                    | .metadata.annotations["efe.io/manifest-state"] = "backup"
                    | .metadata.annotations["efe.io/backup-index"] = $idx
                    | .metadata.annotations["efe.io/backup-taken-at"] = (now | todate)
                    | del(.metadata.uid, .metadata.resourceVersion, .metadata.creationTimestamp, .metadata.managedFields)
                    | .metadata.labels["efe.io/manifest-state"] = "backup"
                  ' \
                | kubectl -n "$NS" apply -f -

              # Bump the index annotation on the live CM.
              NEW_NEXT_IDX=$(( (NEXT_IDX + 1) % RING_SIZE ))
              kubectl -n "$NS" annotate cm "$LIVE_CM" \
                "efe.io/backup-next-index=$NEW_NEXT_IDX" --overwrite

              echo "✓ Snapshot complete : index=$NEXT_IDX ; next=$NEW_NEXT_IDX"
{{- end }}
```

### 10.3 Why a ring of 5

  * 1 reload per day average → 5 reloads = 1 week history.
  * Each backup CM is ~ 2 KB (the manifest YAML).  5 × 2 KB = 10 KB
    of etcd budget per chart release.
  * Operator queries : `kubectl get cm -l efe.io/manifest-state=backup --sort-by=.metadata.annotations.efe\.io/backup-taken-at`

### 10.4 Restore procedure

```bash
# Find the desired backup.
$ kubectl -n efe get cm -l efe.io/manifest-state=backup \
    -o custom-columns='NAME:metadata.name,INDEX:metadata.annotations.efe\.io/backup-index,TAKEN:metadata.annotations.efe\.io/backup-taken-at'

# Promote backup to live (copy data, swap name).
$ kubectl -n efe get cm <release>-efe-meta-swarm-manifest-backup-2 -o json \
    | jq '.metadata.name = "<release>-efe-meta-swarm-manifest"
          | .metadata.annotations["efe.io/manifest-state"] = "live"' \
    | kubectl -n efe apply -f -

# Post-restore SIGHUP-all.
$ kubectl -n efe exec deploy/<release>-efe-meta-swarm-controller -- kill -HUP 1
$ for p in ... ; do kubectl -n efe exec deploy/<release>-efe-meta-swarm-participant-$p -- kill -HUP 1 ; done
```

---

## 11. Post-upgrade Helm hook : SIGHUP-all `Job`

### 11.1 `templates/hooks/post-upgrade-sighup.yaml`

```yaml
{{- if and .Values.enabled .Values.hooks.enabled .Values.hooks.postUpgrade.sighupEnabled }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "efe-meta-swarm.hookName" (list . "post-upgrade-sighup") }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
  annotations:
    "helm.sh/hook": post-upgrade,post-install
    "helm.sh/hook-weight": "10"
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  backoffLimit: 2
  ttlSecondsAfterFinished: 300
  template:
    metadata:
      labels:
        {{- include "efe-meta-swarm.labels" . | nindent 8 }}
        app.kubernetes.io/component: hook
    spec:
      restartPolicy: Never
      serviceAccountName: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
      automountServiceAccountToken: true
      enableServiceLinks: false
      securityContext:
        {{- include "efe-meta-swarm.commonPodSecurityContext" . | nindent 8 }}
      containers:
        - name: sighup
          image: {{ .Values.hooks.image | quote }}
          imagePullPolicy: IfNotPresent
          securityContext:
            {{- include "efe-meta-swarm.commonContainerSecurityContext" . | nindent 12 }}
          resources:
            requests: {cpu: 50m, memory: 64Mi}
            limits:   {cpu: 200m, memory: 256Mi}
          command:
            - sh
            - -c
            - |
              set -eu
              NS="{{ .Release.Namespace }}"
              IGNORE_FAIL={{ .Values.hooks.postUpgrade.ignoreFailures | default true }}

              echo "── SIGHUP-all : namespace=$NS ──"

              # Wait for pods to be Ready before SIGHUP-ing them.
              echo "Waiting for controller Deployment..."
              kubectl -n "$NS" rollout status \
                deploy/{{ include "efe-meta-swarm.controllerName" . }} \
                --timeout=120s

              # SIGHUP controller.
              echo "SIGHUP controller..."
              if ! kubectl -n "$NS" exec \
                   deploy/{{ include "efe-meta-swarm.controllerName" . }} \
                   -- kill -HUP 1 ; then
                if [ "$IGNORE_FAIL" = "true" ] ; then
                  echo "⚠ controller SIGHUP failed ; continuing (ignoreFailures=true)"
                else
                  echo "✗ controller SIGHUP failed"
                  exit 1
                fi
              fi

              # SIGHUP every participant.
              {{- range $p := .Values.participants }}
              echo "Waiting for participant {{ $p.id }}..."
              kubectl -n "$NS" rollout status \
                deploy/{{ include "efe-meta-swarm.participantName" (list $ $p.id) }} \
                --timeout=120s
              echo "SIGHUP participant {{ $p.id }}..."
              if ! kubectl -n "$NS" exec \
                   deploy/{{ include "efe-meta-swarm.participantName" (list $ $p.id) }} \
                   -- kill -HUP 1 ; then
                if [ "$IGNORE_FAIL" = "true" ] ; then
                  echo "⚠ participant {{ $p.id }} SIGHUP failed ; continuing"
                else
                  echo "✗ participant {{ $p.id }} SIGHUP failed"
                  exit 1
                fi
              fi
              {{- end }}

              echo "✓ SIGHUP-all complete"
{{- end }}
```

### 11.2 Why `post-install` too

The hook runs on `helm install` AND `helm upgrade`.  On install,
`SIGHUP` is a no-op (pods just booted ; manifest is freshly loaded).
But harmless.  Including `post-install` makes the runbook uniform.

### 11.3 `kubectl rollout status` before SIGHUP

If pods are mid-rollout (e.g., image upgrade), the SIGHUP would arrive
at the old pod just before termination.  Waiting for `rollout status`
ensures new pods are Ready first.  Timeout 120 s covers slow image
pulls.

---

## 12. RBAC for hooks

### 12.1 `templates/hooks/hook-serviceaccount.yaml`

```yaml
{{- if and .Values.enabled .Values.hooks.enabled }}
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
  annotations:
    "helm.sh/hook": pre-install,pre-upgrade
    "helm.sh/hook-weight": "-20"
    "helm.sh/hook-delete-policy": before-hook-creation
automountServiceAccountToken: true
{{- end }}
```

### 12.2 `templates/hooks/hook-role.yaml`

```yaml
{{- if and .Values.enabled .Values.hooks.enabled }}
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
  annotations:
    "helm.sh/hook": pre-install,pre-upgrade
    "helm.sh/hook-weight": "-19"
    "helm.sh/hook-delete-policy": before-hook-creation
rules:
  # pre-upgrade-snapshot Job needs : get + create + update on ConfigMaps.
  - apiGroups: [""]
    resources: ["configmaps"]
    verbs: ["get", "list", "create", "update", "patch"]
  # post-upgrade-sighup Job needs : get on Deployments + exec on Pods.
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["get", "list"]
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "list"]
  - apiGroups: [""]
    resources: ["pods/exec"]
    verbs: ["create"]
  # kubectl rollout status needs : get on Deployment and its ReplicaSets.
  - apiGroups: ["apps"]
    resources: ["replicasets"]
    verbs: ["get", "list", "watch"]
{{- end }}
```

### 12.3 `templates/hooks/hook-rolebinding.yaml`

```yaml
{{- if and .Values.enabled .Values.hooks.enabled }}
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
  annotations:
    "helm.sh/hook": pre-install,pre-upgrade
    "helm.sh/hook-weight": "-18"
    "helm.sh/hook-delete-policy": before-hook-creation
subjects:
  - kind: ServiceAccount
    name: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
    namespace: {{ .Release.Namespace }}
roleRef:
  kind: Role
  name: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
  apiGroup: rbac.authorization.k8s.io
{{- end }}
```

### 12.4 Threat model (T25)

The hook `Role` grants `pods/exec` — enough to run arbitrary commands
inside the chart's pods.  Mitigations :
  * Scope is `namespace`, not cluster.
  * Verbs are listed exhaustively (no `*`).
  * `automountServiceAccountToken: true` ONLY on the hook pods, not
    runtime pods.
  * Hook pod is short-lived (`ttlSecondsAfterFinished: 300`) ; SA
    token is rotated by K8s.

---

## 13. Bundle weight-loader sidecar (structural placeholder)

### 13.1 `templates/_sidecar-helpers.tpl`

```tpl
{{- define "efe-meta-swarm.sidecarContainer" -}}
{{- $root := index . 0 -}}
{{- $p := index . 1 -}}
- name: bundle-loader
  image: {{ $root.Values.bundle.sidecar.image | default "gcr.io/google-containers/pause:3.9" | quote }}
  imagePullPolicy: IfNotPresent
  securityContext:
    {{- include "efe-meta-swarm.commonContainerSecurityContext" $root | nindent 4 }}
  env:
    - name: EFE_BUNDLE_PARTICIPANT_ID
      value: {{ $p.id | quote }}
    - name: EFE_BUNDLE_ENDPOINT_URL
      value: {{ $root.Values.bundle.endpointUrl | quote }}
    - name: EFE_BUNDLE_POLL_INTERVAL_S
      value: {{ $root.Values.bundle.pollIntervalSeconds | default 60 | quote }}
    - name: EFE_BUNDLE_WEIGHTS_PATH
      value: /var/lib/efe-meta-swarm/weights/current.bin
  resources:
    {{- toYaml $root.Values.bundle.sidecar.resources | nindent 4 }}
  volumeMounts:
    - name: weights
      mountPath: /var/lib/efe-meta-swarm/weights
{{- end -}}
```

### 13.2 Patch to `participant-deployment.yaml`

Inside the `Pod.spec.containers` block, after the main `participant`
container :

```yaml
{{- if $.Values.bundle.sidecar.enabled }}
{{- include "efe-meta-swarm.sidecarContainer" (list $ $p) | nindent 8 }}
{{- end }}
```

And inside the `Pod.spec` :
```yaml
{{- if $.Values.bundle.sidecar.enabled }}
shareProcessNamespace: true
{{- end }}
```

And inside `Pod.spec.volumes` :
```yaml
{{- if $.Values.bundle.sidecar.enabled }}
- name: weights
  emptyDir:
    medium: Memory
    sizeLimit: {{ $.Values.bundle.sidecar.weightsSizeLimit | default "256Mi" }}
{{- end }}
```

And the participant container's `volumeMounts` :
```yaml
{{- if $.Values.bundle.sidecar.enabled }}
- name: weights
  mountPath: /var/lib/efe-meta-swarm/weights
  readOnly: true
{{- end }}
```

### 13.3 PR #15 wire (out of scope for PR #14)

PR #15 will :
  1. Build the bundle-loader binary (Rust) that polls
     `EFE_BUNDLE_ENDPOINT_URL`, downloads on SHA change, atomic-renames
     to `EFE_BUNDLE_WEIGHTS_PATH`, then `kill -HUP 1`.
  2. Replace the `pause:3.9` default with `ghcr.io/efe-ai/efe-bundle-loader:0.1.0`.
  3. Add `EFE_META_SWARM_WEIGHTS_PATH` env var to the participant
     binary (Rust side) so it reads from the shared volume.

### 13.4 T26 mitigation (`shareProcessNamespace` /proc exposure)

When `shareProcessNamespace: true`, every container in the pod sees the
others' `/proc` entries.  Mitigations :
  * Both containers run as `runAsNonRoot: true` + same UID 65532 — no
    privilege escalation possible.
  * `readOnlyRootFilesystem: true` on both containers — no `/proc`
    modifications.
  * Pod is a single trust domain ; the participant + sidecar are
    co-located by design.
  * Documented in README as a "trust boundary" decision.

---

## 14. `PrometheusRule` alerts

### 14.1 `templates/prometheusrule.yaml`

```yaml
{{- if and .Values.enabled .Values.alerts.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    prometheus: efe
  annotations:
    runbookBaseUrl: {{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" | quote }}
spec:
  groups:
    - name: efe-meta-swarm.reload
      interval: 30s
      rules:
        # Alert 1 : reload failures rate > 0.1/sec for 5 min.
        - alert: MetaSwarmReloadFailureRate
          expr: |
            sum(rate(efe_meta_swarm_reload_failures_total[5m]))
              by (namespace, role)
            > 0.1
          for: 5m
          labels:
            severity: {{ .Values.alerts.severity.ReloadFailureRate | default "warning" | quote }}
            component: meta-swarm
          annotations:
            summary: "Meta-swarm reload failure rate is elevated"
            description: |
              The reload (SIGHUP / ReloadConfig RPC) failure rate is > 0.1/sec
              for 5 minutes.  Operator action : `kubectl logs deploy/{{ include "efe-meta-swarm.controllerName" . }}`
              to inspect the rejection reason.
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#reload-failures"

        # Alert 2 : p99 reload latency > 1 sec.
        - alert: MetaSwarmReloadLatencyP99High
          expr: |
            histogram_quantile(0.99,
              sum(rate(efe_meta_swarm_reload_duration_microseconds_bucket[5m]))
              by (le, namespace, role)
            ) > 1000000
          for: 10m
          labels:
            severity: {{ .Values.alerts.severity.ReloadLatencyP99High | default "warning" | quote }}
            component: meta-swarm
          annotations:
            summary: "Meta-swarm reload p99 latency > 1 sec"
            description: |
              The 99th percentile reload duration exceeds 1 second.
              Expected : < 25 µs for healthy manifests.  Investigate :
              large manifests, slow validation, etc.
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#reload-latency"

        # Alert 3 : RELOADING status persists > 1 min (stuck swap).
        - alert: MetaSwarmReloadingStuck
          expr: |
            avg_over_time(efe_meta_swarm_health_status{status="RELOADING"}[1m]) > 0.5
          for: 1m
          labels:
            severity: {{ .Values.alerts.severity.ReloadingStuck | default "critical" | quote }}
            component: meta-swarm
          annotations:
            summary: "Controller stuck in RELOADING state"
            description: |
              The controller has been in RELOADING (status=3) for over 1 minute.
              Expected : < 25 µs.  Likely a crashed swap mid-flight.  Operator
              action : `kubectl rollout restart deploy/{{ include "efe-meta-swarm.controllerName" . }}`
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#reloading-stuck"

    - name: efe-meta-swarm.availability
      interval: 30s
      rules:
        # Alert 4 : controller unreachable > 5 min.
        - alert: MetaSwarmControllerDown
          expr: |
            up{job=~".*meta-swarm-controller.*"} == 0
          for: 5m
          labels:
            severity: {{ .Values.alerts.severity.ControllerDown | default "critical" | quote }}
            component: meta-swarm
          annotations:
            summary: "Meta-swarm controller is DOWN"
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#controller-down"

        # Alert 5 : participant unreachable > 5 min.
        - alert: MetaSwarmParticipantDown
          expr: |
            up{job=~".*meta-swarm-participant-.*"} == 0
          for: 5m
          labels:
            severity: {{ .Values.alerts.severity.ParticipantDown | default "warning" | quote }}
            component: meta-swarm
          annotations:
            summary: "Meta-swarm participant {{`{{ $labels.job }}`}} is DOWN"
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#participant-down"

    - name: efe-meta-swarm.reload-freshness
      interval: 60s
      rules:
        # Alert 6 : reload_gen hasn't moved in 24h despite ConfigMap churn.
        - alert: MetaSwarmReloadGenNotAdvancing
          expr: |
            increase(efe_meta_swarm_reload_gen[24h]) == 0
            and
            kube_configmap_info{configmap=~".*meta-swarm-manifest"} unless on (configmap)
              kube_configmap_info{configmap=~".*meta-swarm-manifest"} offset 24h
          for: 1h
          labels:
            severity: {{ .Values.alerts.severity.ReloadGenNotAdvancing | default "info" | quote }}
            component: meta-swarm
          annotations:
            summary: "ConfigMap changed but reload_gen didn't advance — operator forgot SIGHUP ?"
            description: |
              The manifest ConfigMap has been modified within the last 24h
              but `reload_gen` hasn't moved.  Likely : operator did
              `kubectl edit cm` or `helm upgrade` without the post-upgrade
              SIGHUP hook (or hook failed).
            runbook_url: "{{ .Values.alerts.runbookBaseUrl | default "https://efe.example.com/runbook/meta-swarm" }}#reload-gen-stale"
{{- end }}
```

### 14.2 Severity matrix

| Alert | Default | Why |
|---|---|---|
| ReloadFailureRate | warning | Rejected reloads are signal, not outage (PR #12 leaves state intact) |
| ReloadLatencyP99High | warning | Slow reload doesn't break Iterate (snapshot pattern) |
| ReloadingStuck | critical | Indicates a hung swap → in-flight Iterate may stall |
| ControllerDown | critical | No Iterate possible |
| ParticipantDown | warning | Iterate degrades but doesn't fully fail (other participants still up) |
| ReloadGenNotAdvancing | info | Operator hygiene issue, not an outage |

---

## 15. Grafana dashboard `ConfigMap`

### 15.1 `templates/grafana-dashboard.yaml`

```yaml
{{- if and .Values.enabled .Values.dashboard.enabled }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-dashboard
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    grafana_dashboard: "1"       # marker for Grafana sidecar auto-import
data:
  efe-meta-swarm.json: |-
    {{- .Files.Get "files/grafana-dashboard.json" | nindent 4 }}
{{- end }}
```

### 15.2 `files/grafana-dashboard.json` panels

(Schema : Grafana 9+ dashboard JSON.)

  1. **Reload Generation** (single-stat) : `max(efe_meta_swarm_reload_gen)` — current gen counter
  2. **Reload Attempts** (time series) : `rate(efe_meta_swarm_reload_attempts_total[5m])`
  3. **Reload Failures by Reason** (time series, stacked) : `rate(efe_meta_swarm_reload_failures_total{reason=~".*"}[5m]) by (reason)`
  4. **Reload Duration p50/p90/p99** (time series) : `histogram_quantile(...)`
  5. **Health Probes** (time series) : `rate(efe_meta_swarm_health_probes_total[5m])` + `rate(...errors_total)`
  6. **Health Probe Latency p99** (time series) : `histogram_quantile(0.99, ...)`
  7. **Pod Up Status** (per role) : `up{job=~".*meta-swarm-.*"}` heatmap

Operators import via Grafana sidecar OR `grafana-cli admin reset-admin-password
+ POST /api/dashboards/db`.

---

## 16. TLS bootstrap stubs

### 16.1 `templates/tls-issuer.yaml`

```yaml
{{- if and .Values.enabled .Values.tls.enabled .Values.tls.certManager.enabled }}
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-issuer
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
spec:
  {{- if eq .Values.tls.certManager.issuerType "selfSigned" }}
  selfSigned: {}
  {{- else if eq .Values.tls.certManager.issuerType "ca" }}
  ca:
    secretName: {{ .Values.tls.certManager.caSecretName }}
  {{- end }}
{{- end }}
```

### 16.2 `templates/tls-certificate.yaml`

Renders one `Certificate` per role (controller + each participant) :

```yaml
{{- if and .Values.enabled .Values.tls.enabled .Values.tls.certManager.enabled }}
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe-meta-swarm.controllerName" . }}-tls
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
spec:
  secretName: {{ include "efe-meta-swarm.controllerName" . }}-tls
  duration: 720h    # 30 days
  renewBefore: 168h # 7 days before expiry
  dnsNames:
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc.cluster.local
  issuerRef:
    name: {{ include "efe-meta-swarm.fullname" . }}-issuer
    kind: Issuer

{{- range $p := .Values.participants }}
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}-tls
  labels:
    {{- include "efe-meta-swarm.labels" $ | nindent 4 }}
    efe.io/participant-id: {{ $p.id | quote }}
spec:
  secretName: {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}-tls
  duration: 720h
  renewBefore: 168h
  dnsNames:
    - {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}.{{ $.Release.Namespace }}.svc
    - {{ include "efe-meta-swarm.participantName" (list $ $p.id) }}.{{ $.Release.Namespace }}.svc.cluster.local
  issuerRef:
    name: {{ include "efe-meta-swarm.fullname" $ }}-issuer
    kind: Issuer
{{- end }}
{{- end }}
```

### 16.3 Deployment template volume mounts (under `if tls.enabled`)

```yaml
{{- if .Values.tls.enabled }}
- name: tls
  mountPath: /etc/efe-meta-swarm/tls
  readOnly: true
{{- end }}
```

And volumes :
```yaml
{{- if .Values.tls.enabled }}
- name: tls
  secret:
    secretName: {{ include "efe-meta-swarm.controllerName" . }}-tls
{{- end }}
```

And env vars :
```yaml
{{- if .Values.tls.enabled }}
- name: EFE_META_SWARM_TLS_REQUIRED
  value: "true"
- name: EFE_META_SWARM_TLS_CERT
  value: /etc/efe-meta-swarm/tls/tls.crt
- name: EFE_META_SWARM_TLS_KEY
  value: /etc/efe-meta-swarm/tls/tls.key
- name: EFE_META_SWARM_TLS_CA
  value: /etc/efe-meta-swarm/tls/ca.crt
{{- end }}
```

PR #16 wires the binary's TLS-aware tonic startup ; PR #14 just stages
the K8s artefacts.

---

## 17. NetworkPolicy refinements

Two additions to PR #13's `networkpolicy.yaml` :

  1. **Hook `Job` egress** : the pre-upgrade-snapshot and
     post-upgrade-sighup pods need to reach the K8s API server.
     Default-deny would block them.

```yaml
{{- if and .Values.enabled .Values.networkPolicy.enabled .Values.hooks.enabled }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-hook-egress
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    app.kubernetes.io/component: hook
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: hook
  policyTypes: [Egress]
  egress:
    # K8s API server (kubernetes.default.svc usually 10.0.0.1:443).
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: default
          podSelector: {}
      ports:
        - protocol: TCP
          port: 443
    # DNS.
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
          podSelector:
            matchLabels:
              k8s-app: kube-dns
      ports:
        - {protocol: UDP, port: 53}
        - {protocol: TCP, port: 53}
{{- end }}
```

  2. **Bundle sidecar egress** : the sidecar (when enabled, in PR #15)
     polls an external bundle endpoint.

```yaml
{{- if and .Values.enabled .Values.networkPolicy.enabled .Values.bundle.sidecar.enabled }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-bundle-egress
spec:
  podSelector:
    matchLabels:
      {{- include "efe-meta-swarm.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: participant
  policyTypes: [Egress]
  egress:
    - to:
        - namespaceSelector:
            {{- toYaml .Values.bundle.endpointNamespaceSelector | nindent 12 }}
      ports:
        - protocol: TCP
          port: {{ .Values.bundle.endpointPort | default 443 }}
{{- end }}
```

---

## 18. Resource sizing matrix

| Environment | Controller CPU/Mem (req → lim) | Each Participant (req → lim) | Notes |
|---|---|---|---|
| dev | 100m / 128Mi → 500m / 512Mi | 50m / 64Mi → 200m / 256Mi | Fits on laptop |
| staging | 500m / 512Mi → 2 / 2Gi | 300m / 384Mi → 1 / 1Gi | Production-equivalent scaled down |
| prod | 1 / 1Gi → 4 / 4Gi | 1 / 1Gi → 2 / 2Gi | Production-grade |
| canary | 250m / 256Mi → 1 / 1Gi | 200m / 256Mi → 500m / 512Mi | Small canary slice |
| Hook Job | 50m / 64Mi → 200m / 256Mi | n/a | Short-lived |
| Bundle sidecar | 100m / 128Mi → 250m / 256Mi (PR #15) | n/a | Bundled with participant pod |

---

## 19. Schema additions (`values.schema.json`)

```json
{
  "properties": {
    "priorityClass": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled": { "type": "boolean" },
        "value":   { "type": "integer", "minimum": -2147483648, "maximum": 1000000000 }
      }
    },
    "controller": {
      "properties": {
        "hpa": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "enabled":                 { "type": "boolean" },
            "targetCPUUtilization":    { "type": "integer", "minimum": 10, "maximum": 100 }
          }
        }
      }
    },
    "hooks": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled":  { "type": "boolean" },
        "image":    { "type": "string", "minLength": 1 },
        "preUpgrade": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "snapshotEnabled": { "type": "boolean" },
            "backupRingSize":  { "type": "integer", "minimum": 1, "maximum": 20 }
          }
        },
        "postUpgrade": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "sighupEnabled":  { "type": "boolean" },
            "serial":         { "type": "boolean" },
            "ignoreFailures": { "type": "boolean" }
          }
        }
      }
    },
    "bundle": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "endpointUrl":              { "type": "string", "format": "uri" },
        "endpointPort":             { "type": "integer", "minimum": 1, "maximum": 65535 },
        "endpointNamespaceSelector":{ "type": "object" },
        "pollIntervalSeconds":      { "type": "integer", "minimum": 5, "maximum": 3600 },
        "sidecar": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "enabled":            { "type": "boolean" },
            "image":              { "type": "string", "minLength": 1 },
            "resources":          { "type": "object" },
            "weightsSizeLimit":   { "type": "string", "pattern": "^[0-9]+(Mi|Gi)$" }
          }
        }
      }
    },
    "alerts": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled":         { "type": "boolean" },
        "runbookBaseUrl":  { "type": "string", "format": "uri" },
        "severity": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "ReloadFailureRate":      { "enum": ["info","warning","critical"] },
            "ReloadLatencyP99High":   { "enum": ["info","warning","critical"] },
            "ReloadingStuck":         { "enum": ["info","warning","critical"] },
            "ControllerDown":         { "enum": ["info","warning","critical"] },
            "ParticipantDown":        { "enum": ["info","warning","critical"] },
            "ReloadGenNotAdvancing":  { "enum": ["info","warning","critical"] }
          }
        }
      }
    },
    "dashboard": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled": { "type": "boolean" }
      }
    },
    "tls": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled":      { "type": "boolean" },
        "certManager": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "enabled":        { "type": "boolean" },
            "issuerType":     { "enum": ["selfSigned", "ca"] },
            "caSecretName":   { "type": "string" }
          }
        }
      },
      "allOf": [
        {
          "if":   { "properties": { "enabled": { "const": true } }, "required": ["enabled"] },
          "then": { "required": ["certManager"] }
        }
      ]
    }
  }
}
```

---

## 20. Helm hook ordering + weights

```
Helm install/upgrade flow with PR #14 hooks:

  Phase 0 (pre-install / pre-upgrade)
    weight -20 : hook-serviceaccount.yaml   (SA created first)
    weight -19 : hook-role.yaml             (Role + RoleBinding)
    weight -18 : hook-rolebinding.yaml
    weight -10 : pre-upgrade-snapshot Job  (uses SA from above)

  Phase 1 (resources)
    All non-hook resources apply (ConfigMap, Services, Deployments, etc.)

  Phase 2 (post-install / post-upgrade)
    weight +10 : post-upgrade-sighup Job   (uses same SA)
```

Hook delete policies :
  * `before-hook-creation` : remove the previous run's Job before creating the new one.
  * `hook-succeeded` : delete on success.
  * Failed Jobs are kept (manual cleanup or `ttlSecondsAfterFinished: 300`).

---

## 21. Operator workflows

### 21.1 Production install

```bash
$ helm install efe-stack ./charts/efe-stack \
    --namespace efe-prod --create-namespace \
    -f charts/efe-stack/values-prod.yaml \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml \
    --set global.tenant=kovant \
    --set global.metaSwarm.enabled=true \
    --set efe-meta-swarm.enabled=true
```

### 21.2 Production upgrade (manifest change)

```bash
# 1. Edit values overlay.
$ vim values-prod.local.yaml      # add new manifest.alpha_default = 0.6

# 2. Helm upgrade.
$ helm upgrade efe-stack ./charts/efe-stack \
    --namespace efe-prod \
    -f charts/efe-stack/values-prod.yaml \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml \
    -f values-prod.local.yaml

# Hooks run automatically :
#   pre-upgrade : snapshot live ConfigMap → backup-N
#   apply : ConfigMap mutates
#   post-upgrade : SIGHUP controller + every participant

# 3. Verify.
$ kubectl -n efe-prod port-forward svc/efe-stack-efe-meta-swarm-controller 50080:50080 &
$ grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health \
  | jq '.reloadGen'

# 4. Inspect backup.
$ kubectl -n efe-prod get cm -l efe.io/manifest-state=backup
```

### 21.3 Rollback to a previous manifest

```bash
# 1. List backups.
$ kubectl -n efe-prod get cm -l efe.io/manifest-state=backup \
    -o custom-columns='NAME:metadata.name,IDX:metadata.annotations.efe\.io/backup-index,TAKEN:metadata.annotations.efe\.io/backup-taken-at'

# 2. Promote backup-2 to live.
$ kubectl -n efe-prod get cm efe-stack-efe-meta-swarm-manifest-backup-2 -o json \
    | jq '.metadata.name = "efe-stack-efe-meta-swarm-manifest"
          | .metadata.annotations["efe.io/manifest-state"] = "live"
          | del(.metadata.uid, .metadata.resourceVersion)' \
    | kubectl -n efe-prod apply -f -

# 3. SIGHUP all to pick up the restored manifest.
$ kubectl -n efe-prod exec deploy/efe-stack-efe-meta-swarm-controller -- kill -HUP 1
$ for p in $(kubectl -n efe-prod get deploy -l efe.io/meta-swarm-role=participant -o name); do
    kubectl -n efe-prod exec $p -- kill -HUP 1
  done
```

### 21.4 Canary deployment

```bash
# Main release.
$ helm install efe-stack ./charts/efe-stack -n efe-prod \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml \
    --set global.tenant=kovant --set global.metaSwarm.enabled=true \
    --set efe-meta-swarm.enabled=true

# Canary release (different release name).
$ helm install efe-stack-canary ./charts/efe-stack -n efe-prod \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-canary.yaml \
    --set global.tenant=kovant-canary --set global.metaSwarm.enabled=true \
    --set efe-meta-swarm.enabled=true

# Operators target canary by label.
$ kubectl get pods -l efe.io/tenant=kovant-canary
```

---

## 22. Concurrency / lifecycle invariants

PR #14 extends PR #13 § 33 with hook-specific invariants :

  * **Hook idempotency** : the pre-upgrade-snapshot Job is safe to
    re-run (creates/updates the backup CM atomically).  The
    post-upgrade-sighup Job's SIGHUPs are idempotent (PR #12's reload
    is transactional).
  * **Hook order** : pre-upgrade runs before resource apply ;
    post-upgrade runs after.  Within a phase, weight orders.
  * **Hook failure mode** : `ignoreFailures: true` (dev/staging
    default) treats SIGHUP failures as warnings.  `ignoreFailures:
    false` (prod default) fails the upgrade if any SIGHUP fails.
  * **PriorityClass** : controller's priority 100000 wins evictions
    over participants (1000) and default user pods (0).
  * **Sidecar lifecycle** : sidecar starts WITH the participant
    container ; on participant restart, sidecar restarts too (same
    Pod).  `shareProcessNamespace` is set at pod-spec level so both
    containers see each other's PIDs from boot.

---

## 23. Threat-model coverage

PR #14 ships 4 new threats (T25–T28).  All ≤ SEV-2.

| Threat | Severity | PR #14 mitigation |
|---|---|---|
| **T25** (Helm hook over-privileged RBAC — `pods/exec` allows arbitrary commands inside pods) | SEV-2 | Dedicated `ServiceAccount` per hook ; `Role` scoped to namespace ; verbs listed exhaustively (`get,list,create,update,patch` on ConfigMap ; `get,list` on Deployment ; `create` on pods/exec only) ; SA token mounted ONLY on hook pods, not runtime pods ; pods short-lived (TTL 300 s) ; documented in README |
| **T26** (Sidecar `shareProcessNamespace: true` exposes /proc to participant container — privilege escalation if sidecar is compromised) | SEV-2 | Both containers run as `runAsNonRoot: true` + same UID 65532 ; `readOnlyRootFilesystem: true` ; same trust boundary (single Pod) ; sidecar image in Phase 2 is `pause:3.9` (no logic) ; in PR #15, sidecar image will be a tightly-scoped Rust binary with no shell ; documented in README as a "trust boundary" |
| **T27** (Backup ConfigMap rotation tampering — operator with write access could plant a fake "last good" manifest) | SEV-3 | Backup CM labels (`efe.io/manifest-state: backup`) are visible in audit logs ; K8s `Event`s emit on each backup creation ; ring index is annotation-driven (auditable) ; restoration is a deliberate `kubectl apply` (logged) ; future PR may sign manifests with cosign |
| **T28** (Bundle endpoint unauthenticated — sidecar could fetch tampered weights) | SEV-2 | Phase-2 sidecar is `pause:3.9` (doesn't fetch anything) ; PR #16 will add mTLS auth between sidecar and bundle endpoint ; cosign verification deferred to Phase 3 ; documented as a known limitation in README's "Threat model" section |

Carried from PR #13 :
  * T22 (secret in values.yaml) — README warns ; values examples don't include secrets
  * T23 (NetworkPolicy bypass) — refined NetworkPolicy for hook + bundle egress
  * T24 (image-tag floating) — schema continues to enforce non-empty tag (AD17 of PR #13)

---

## 24. Tests strategy

### 24.1 Unit tests (helm-unittest)

| File | Tests added | Coverage focus |
|---|---|---|
| `tests/hooks_test.yaml` | ≥ 5 | hook Job rendering, hook annotations, RBAC presence, hook image, hook env |
| `tests/observability_test.yaml` | ≥ 4 | PrometheusRule render (6 alerts), Grafana ConfigMap with grafana_dashboard label, alerts severity overrides |
| `tests/sidecar_test.yaml` | ≥ 3 | sidecar container presence behind `enabled` guard, shareProcessNamespace, emptyDir volume |
| `tests/tls_test.yaml` | ≥ 3 | Issuer + Certificate × 4 rendering, volume mounts under tls.enabled guard |
| `tests/tenant_labels_test.yaml` | ≥ 3 | `efe.io/tenant: <slug>` propagation across all resources |
| **Total new** | **≥ 18** | (on top of PR #13's 23) |

### 24.2 Integration tests

  * `kovant-prod.yaml` golden snapshot : full prod-grade rendering with
    hooks + alerts + dashboard + priority class.
  * `kovant-hooks.yaml` golden snapshot : hooks-only scenario.
  * Existing `kovant-3p`, `kovant-disabled`, `kovant-network-policy`,
    `kovant-service-monitor` are **regression-checked** to ensure
    PR #14 doesn't break them (no new resources unless flags set).

### 24.3 CI workflow extensions

  * New scenarios in `.github/workflows/helm-meta-swarm.yml` :
    - `helm template … -f kovant-prod.values.yaml` → expect 25+ resources
    - `helm template … -f kovant-hooks.values.yaml` → expect 8+ resources
  * Polaris audit re-run on the new scenarios (severity=danger gate).
  * `kubectl --dry-run=server` on a kind cluster (Phase 3 stretch) to
    validate the RBAC + hook integration.

### 24.4 Test count target

| Layer | Baseline (PR #13) | After PR #14 |
|---|---|---|
| `helm-unittest` | 23 | **≥ 41** (+18) |
| Golden snapshots | 4 | **6** (+2) |
| `kubeconform` | 51 | **80+** (+25 from new scenarios) |
| Rust round-trip | 1 | **1** (unchanged) |
| **Total chart tests** | **79** | **130+** |

---

## 25. Acceptance criteria

PR #14 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `helm lint` clean on sub-chart with all 4 example flavors | CI |
| AC2 | `helm template` for each of `kovant-3p`, `kovant-disabled`, `kovant-network-policy`, `kovant-service-monitor`, `kovant-prod`, `kovant-hooks` succeeds | CI |
| AC3 | All 6 scenarios pass `kubeconform -strict` | CI |
| AC4 | All 6 golden snapshots match `helm template` output | CI |
| AC5 | All 4 example flavors (dev/staging/prod/canary) render cleanly | CI |
| AC6 | `helm-unittest` runs ≥ 41 tests, all pass | CI |
| AC7 | `values.schema.json` rejects invalid alert severity | manual + schema test |
| AC8 | Schema rejects `hooks.preUpgrade.backupRingSize: 25` (max 20) | schema test |
| AC9 | Schema rejects `bundle.pollIntervalSeconds: 1` (min 5) | schema test |
| AC10 | Schema requires `tls.certManager` when `tls.enabled: true` | schema test |
| AC11 | Pre-upgrade Job has `helm.sh/hook: pre-upgrade` + weight `-10` | golden + CI grep |
| AC12 | Post-upgrade Job has `helm.sh/hook: post-upgrade,post-install` + weight `+10` | golden + CI grep |
| AC13 | Hook SA + Role + RoleBinding created with hook annotations | helm-unittest |
| AC14 | Role grants ONLY `configmaps` + `pods/exec` + `deployments/replicasets:get,list` | helm-unittest + CI grep |
| AC15 | PrometheusRule has 6 alerts when `alerts.enabled: true` | helm-unittest |
| AC16 | All 6 alerts have `runbook_url` annotation | CI grep |
| AC17 | Grafana ConfigMap has `grafana_dashboard: "1"` label | helm-unittest |
| AC18 | Embedded Grafana JSON is valid (parseable) | CI : `jq . files/grafana-dashboard.json` |
| AC19 | PriorityClass rendered with value 100000 when enabled | helm-unittest |
| AC20 | Controller Deployment references the PriorityClass when enabled | helm-unittest |
| AC21 | HPA template renders with min=max=1 when enabled (Phase-2 lock) | helm-unittest |
| AC22 | Sidecar container present in participant Pod when enabled | helm-unittest |
| AC23 | `shareProcessNamespace: true` set when sidecar enabled | helm-unittest |
| AC24 | `emptyDir` weights volume present when sidecar enabled | helm-unittest |
| AC25 | TLS Issuer rendered when `tls.certManager.enabled` | helm-unittest |
| AC26 | TLS Certificate × 4 (controller + 3 participants) rendered | helm-unittest |
| AC27 | TLS volume mounts injected into Deployments when `tls.enabled` | helm-unittest |
| AC28 | `efe.io/tenant: <slug>` label on every resource | helm-unittest + CI grep |
| AC29 | NetworkPolicy hook-egress rendered when hooks + networkPolicy both enabled | helm-unittest |
| AC30 | Polaris severity=danger gate clean on all 6 scenarios | CI |
| AC31 | Backward compat : disabling PR #14 features (defaults) reproduces PR #13's exact rendering for kovant-3p | golden diff |
| AC32 | Parent chart hook (`global.metaSwarm.enabled`) still works (PR #13 § 28 hook unchanged) | CI |
| AC33 | Manifest round-trip through Rust validator still passes for prod-flavor manifest | CI |
| AC34 | Chart.yaml version bumped to 0.2.0 (PR #14 is a feature release) ; appVersion to 0.3.0 | manual |
| AC35 | All 4 example flavors documented in `examples/README.md` | manual |
| AC36 | PR #14 architecture doc referenced from `README.md` + `NOTES.txt` | manual |
| AC37 | Threat IDs T25/T26/T27/T28 documented in README's threat model section | manual |
| AC38 | Operator runbook covers 11 scenarios (install / upgrade / rollback / canary / hooks failure / etc.) | manual |

---

## 26. Risks + mitigations

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| **PR14-R1** | `shareProcessNamespace: true` rejected by PodSecurityPolicy / OPA Gatekeeper on strict clusters | M / M | Documented prerequisite ; sidecar opt-in (default `false`) ; runbook covers admission policy override |
| **PR14-R2** | Hook Job uses `bitnami/kubectl:1.28` which may drift from cluster K8s version | L / L | Pinned tag matches `kubeVersion` in Chart.yaml ; operator override via `hooks.image` |
| **PR14-R3** | Backup ConfigMap ring corruption (race between two concurrent upgrades) | LL / M | Helm releases serialise via Tiller-less locking ; manual mitigation via `kubectl delete cm -l efe.io/manifest-state=backup` and let next upgrade re-snapshot |
| **PR14-R4** | Operator runs `helm upgrade --no-hooks` accidentally → no SIGHUP, manifest desync | M / L | Documented in runbook ; `kubectl logs` post-upgrade hook still emits the hostnames ; PR12-R3 watch task may auto-recover |
| **PR14-R5** | `cert-manager` CRDs missing — `Issuer` / `Certificate` install fails | M / M | Opt-in via `tls.certManager.enabled` ; documented prerequisite ; chart's `if` guard prevents render when off |
| **PR14-R6** | `prometheus-operator` CRDs missing — `PrometheusRule` install fails | M / M | Opt-in via `alerts.enabled` ; documented |
| **PR14-R7** | `grafana_dashboard: "1"` label not recognized by some Grafana installs | L / L | Documented ; alternative : import JSON manually via Grafana UI |
| **PR14-R8** | PriorityClass `value: 100000` evicts participants under pressure but participants then can't restart due to nodeSelector taints | M / M | Operator configures participant `tolerations` to match controller's nodeSelector ; runbook covers |
| **PR14-R9** | Post-upgrade SIGHUP Job runs before pods are Ready (race after Deployment image change) | M / L | `kubectl rollout status` wait before each SIGHUP ; 120 s timeout |
| **PR14-R10** | Pre-upgrade snapshot Job fails (e.g., quota exceeded on ConfigMaps) → helm upgrade rolls back | L / M | `backoffLimit: 1` ; manual mitigation : disable `preUpgrade.snapshotEnabled: false` for the upgrade ; runbook covers |
| **PR14-R11** | Operator forgets to set `global.metaSwarm.enabled=true` alongside `efe-meta-swarm.enabled=true` (PR #13 lesson) | M / H | README + values.yaml comments explicit ; runbook §21.1 shows both flags |
| **PR14-R12** | `priorityClassName` references a PriorityClass that hasn't been created yet (race in helm apply order) | L / L | PriorityClass renders WITHOUT hook annotations → applies in normal phase ; PriorityClass + Deployment apply order is dependency-managed by K8s API server |
| **PR14-R13** | Backup ConfigMap rotation makes `kubectl get cm` noisier | LL / L | `efe.io/manifest-state=backup` label allows filtering ; backups have short names |
| **PR14-R14** | Bundle sidecar resources (CPU/mem) double the Pod's resource footprint | M / M | Documented in resource sizing matrix § 18 ; pause:3.9 is < 5 Mi RAM ; PR #15's real loader < 50 Mi |
| **PR14-R15** | Grafana dashboard JSON drifts from Prometheus metric names | M / M | CI step that grep's dashboard.json for the metric names ; rebuild dashboard against PR #11/#12 metrics |
| **PR14-R16** | Helm 3.13.3 vs 3.15.2 dashboard rendering differs (string-encoding of embedded JSON) | LL / L | CI uses 3.13.3 for golden generation ; cross-tested with 3.15.2 |

---

## 27. Rollback strategy

If PR #14 lands and breaks production :

  1. **Operator-level** : `helm rollback efe-stack <previous-rev>` —
     restores PR #13 state in seconds.
  2. **Per-commit revert** : C8 → C7 → C6 → C5 → C4 → C3 → C2 → C1.
     Each is independently green.
  3. **Disable specific features** : flip values flags to false for
     hooks / alerts / dashboard / sidecar / tls without reverting code.
  4. **Wholesale revert** : `git revert <merge-commit>` ; chart goes
     back to PR #13's skeleton.

Total TTM :
  * `helm rollback` : ~ 30 s.
  * Feature-flag disable : ~ 1 min (`helm upgrade -f <values>`).
  * Code revert + redeploy : ~ 10 min.

---

## 28. Sequence diagrams

(Already shown in § 5.2 + § 5.3.  Below are 2 additional flows.)

### 28.1 Helm rollback with hooks

```
operator        helm                K8s API
   │             │                    │
   │ helm rollback efe-stack 3        │
   │────────────►│                    │
   │             │ Helm computes :    │
   │             │   target revision  │
   │             │   = rev 3 manifest │
   │             │                    │
   │             │ HOOK PHASE 1 :     │
   │             │   pre-rollback hooks (none in PR #14 design — pre-upgrade only)
   │             │                    │
   │             │ APPLY PHASE :      │
   │             │   restore resources to rev 3 state
   │             │   PATCH ConfigMap ──►│ (manifest reverts to old)
   │             │                    │
   │             │ HOOK PHASE 2 :     │
   │             │   post-rollback (the post-upgrade hook fires on rollback too in Helm 3)
   │             │   Job : post-upgrade-sighup
   │             │   ─────────────────►│
   │             │              ▶ SIGHUP all (pods reload old manifest)
   │             │                    │
   │  Done       │                    │
   │◄────────────│                    │
```

### 28.2 Failed post-upgrade hook

```
operator        helm                  K8s API           pods
   │             │                       │                │
   │ helm upgrade efe-stack -f new.yaml  │                │
   │────────────►│                       │                │
   │             │ apply manifests        │                │
   │             │   ConfigMap updated   │                │
   │             │   Deployment patched  │                │
   │             │                       │                │
   │             │ HOOK PHASE 2 :        │                │
   │             │   post-upgrade-sighup │                │
   │             │   ────────────────────►                │
   │             │                       │ pod stuck in   │
   │             │                       │ ContainerCreating
   │             │                       │ (image pull slow)│
   │             │                       │                │
   │             │              kubectl rollout status timeout 120s
   │             │                       │                │
   │             │              exit 1 if !ignoreFailures │
   │             │              (prod default)            │
   │             │              ◀ exit 1                  │
   │             │                       │                │
   │  Hook failed │ — release marked "deployed" but with  │
   │              │   Job evidence in `kubectl describe`  │
   │◄────────────│                                         │
   │                                                       │
   │ Operator inspects + manually fixes :                 │
   │ $ kubectl get jobs -l app.kubernetes.io/component=hook
   │ $ kubectl logs job/...-post-upgrade-sighup
   │ $ <fix root cause, e.g., image pull issue>           │
   │ $ kubectl exec deploy/... -- kill -HUP 1  (manual)   │
```

---

## 29. Reviewer checklist

```
## Reviewer checklist — PR #14

### Helm correctness
- [ ] All 6 scenarios `helm lint` clean (kovant-3p, disabled, networkpolicy, servicemonitor, prod, hooks)
- [ ] All 6 scenarios `helm template` succeed
- [ ] All 6 golden snapshots committed + match live rendering
- [ ] All 41+ `helm-unittest` tests pass
- [ ] `kubeconform -strict` clean (+ CRD-aware for ServiceMonitor / PrometheusRule / Certificate)

### Schema
- [ ] All new values typed in `values.schema.json` with `additionalProperties: false`
- [ ] Cross-field validation : `tls.enabled=true` requires `tls.certManager`
- [ ] All severity enums constrained to `info/warning/critical`
- [ ] Image tags pinned (no `:latest`)

### Hooks
- [ ] pre-upgrade and post-upgrade Jobs render with correct weights
- [ ] Hook ServiceAccount has minimal RBAC (no `*` verbs)
- [ ] Hook image pinned (`bitnami/kubectl:1.28`)
- [ ] Hook delete policies set (`before-hook-creation,hook-succeeded`)

### Sidecar (T26)
- [ ] `shareProcessNamespace: true` only when `bundle.sidecar.enabled`
- [ ] Sidecar container shares `weights` emptyDir volume with participant
- [ ] Sidecar uses `pause:3.9` (Phase-2 placeholder)
- [ ] Sidecar's `securityContext` matches participant's (nonroot, drop caps)

### Observability
- [ ] PrometheusRule has 6 alerts
- [ ] All alerts have `runbook_url` annotation
- [ ] Severity overridable via `values.alerts.severity.*`
- [ ] Grafana ConfigMap has `grafana_dashboard: "1"` label
- [ ] Embedded JSON is parseable + references PR #11/#12 metric names

### TLS (PR #16 contract)
- [ ] Issuer + Certificate × 4 render when `tls.certManager.enabled`
- [ ] Deployment volume mounts and env vars guarded by `tls.enabled`
- [ ] No actual cert lifecycle logic in PR #14 (PR #16 wires the binary)

### Multi-env values flavors
- [ ] All 4 example flavors render cleanly
- [ ] `values-canary.yaml` deploys alongside `values-prod.yaml` without conflict
- [ ] `examples/README.md` documents each flavor

### Threat model
- [ ] T25 / T26 / T27 / T28 mitigations documented in README
- [ ] Hook SA token NOT mounted on runtime pods
- [ ] Bundle endpoint auth deferred to PR #16 (documented)

### Tests + CI
- [ ] CI workflow updated with new scenarios (prod, hooks)
- [ ] Polaris severity=danger gate clean on new scenarios
- [ ] Helm 3.13.3 ≡ 3.15.2 cross-version compat verified

### Code quality
- [ ] `cargo fmt` and `cargo clippy` still clean (PR #14 doesn't touch Rust)
- [ ] Architecture doc cross-referenced from `README.md` + `NOTES.txt`
- [ ] Operator runbook covers 11 scenarios
```

---

## 30. Operator runbook (11 scenarios)

### 30.1 Scenario 1 : First-time production install

(Covered in § 21.1.)

### 30.2 Scenario 2 : Live manifest change (hot reload via hooks)

(Covered in § 21.2.)

### 30.3 Scenario 3 : Rollback to a previous manifest

(Covered in § 21.3.)

### 30.4 Scenario 4 : Canary deployment

(Covered in § 21.4.)

### 30.5 Scenario 5 : Post-upgrade hook failed (pod not Ready)

```bash
# 1. Inspect the failed Job.
$ kubectl get jobs -l app.kubernetes.io/component=hook
$ kubectl logs job/<release>-hook-post-upgrade-sighup

# 2. Fix root cause (e.g., image pull, missing Secret).
$ kubectl describe pod -l app.kubernetes.io/component=controller

# 3. Once pods Ready, SIGHUP manually.
$ kubectl exec deploy/<release>-efe-meta-swarm-controller -- kill -HUP 1
```

### 30.6 Scenario 6 : Pre-upgrade snapshot failed

```bash
# 1. Inspect.
$ kubectl logs job/<release>-hook-pre-upgrade-snapshot

# 2. If quota exceeded :
$ kubectl get resourcequota -n efe-prod
$ kubectl delete cm -l efe.io/manifest-state=backup  # frees space, loses history

# 3. Re-run helm upgrade with snapshot disabled :
$ helm upgrade ... --set hooks.preUpgrade.snapshotEnabled=false
```

### 30.7 Scenario 7 : Alert fired — ReloadFailureRate

  1. Check `kubectl logs deploy/<release>-efe-meta-swarm-controller | grep "reload rejected"`
  2. Inspect the rejection reason (drift, validation, IO).
  3. Fix the manifest (edit values + helm upgrade) OR rollback.

### 30.8 Scenario 8 : Alert fired — ReloadingStuck

  Likely : controller panic mid-swap (very rare with PR #12's transactional design).
  Action : `kubectl rollout restart deploy/<release>-efe-meta-swarm-controller`.

### 30.9 Scenario 9 : Alert fired — ControllerDown / ParticipantDown

  Standard K8s pod-down diagnosis : `kubectl describe pod`, `kubectl logs`, check resources, etc.

### 30.10 Scenario 10 : Enable TLS (PR #16 wire)

```bash
# 1. Ensure cert-manager is installed.
$ kubectl get crds | grep cert-manager

# 2. Upgrade with TLS values.
$ helm upgrade efe-stack ./charts/efe-stack \
    --set efe-meta-swarm.tls.enabled=true \
    --set efe-meta-swarm.tls.certManager.enabled=true \
    --set efe-meta-swarm.tls.certManager.issuerType=selfSigned

# 3. Verify certificates issued.
$ kubectl get certificates -l app.kubernetes.io/name=efe-meta-swarm

# 4. SIGHUP all to pick up TLS env vars.
$ # (hook handles this automatically)
```

### 30.11 Scenario 11 : Enable bundle sidecar (PR #15 wire)

```bash
# 1. Bundle endpoint must be reachable.
# 2. Upgrade with sidecar enabled.
$ helm upgrade efe-stack ./charts/efe-stack \
    --set efe-meta-swarm.bundle.sidecar.enabled=true \
    --set efe-meta-swarm.bundle.sidecar.image=ghcr.io/efe-ai/efe-bundle-loader:0.1.0 \
    --set efe-meta-swarm.bundle.endpointUrl=https://bundles.efe.example.com

# 3. Watch the sidecar logs.
$ kubectl logs -l efe.io/meta-swarm-role=participant -c bundle-loader -f
```

---

## 31. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR14-1** | Should the post-upgrade hook also restart the `efe-server` Deployment when manifest changes meta.dim ? | NO — manifest reloads happen entirely in-process (PR #12) ; efe-server doesn't read the manifest |
| **OQ-PR14-2** | Should we ship a `ServiceMonitor` for the hook Jobs themselves ? | NO — Jobs are short-lived ; not Prometheus-scrape-worthy |
| **OQ-PR14-3** | Should we use Helm's `--atomic` flag in the install runbook ? | YES — recommended in the runbook ; auto-rolls back on hook failure |
| **OQ-PR14-4** | Should the backup ConfigMap ring be a single CM with appended history vs N separate CMs ? | NO — separate CMs are simpler to `kubectl apply` (re-create vs patch) ; ring of 5 keeps it bounded |
| **OQ-PR14-5** | Should the sidecar's `weights` volume be PV-backed (instead of `emptyDir`) for crash-survival ? | NO for Phase 2 — re-download on boot is fine ; Phase 3 may revisit when bundles are GB-scale |
| **OQ-PR14-6** | Should we expose a Helm flag to skip the post-upgrade SIGHUP for image-only upgrades (where pods restart anyway) ? | YES — `hooks.postUpgrade.sighupEnabled: false` covers this ; operators flip per upgrade |
| **OQ-PR14-7** | Should PR #14 ship the `Polaris` config file in the chart ? | NO — Polaris is upstream-configured ; chart doesn't carry tooling configs |
| **OQ-PR14-8** | Should we bundle a `Job` template for "force-reload-all" outside of helm upgrade ? | NO for PR #14 — operator runs `kubectl exec -- kill -HUP 1` manually ; PR #18 may add a CronJob |
| **OQ-PR14-9** | Should we add an `examples/values-disaster-recovery.yaml` flavor ? | NO for PR #14 — DR is a Phase-3 concern with PVC + snapshot integration |
| **OQ-PR14-10** | Should the Grafana dashboard ConfigMap also have `grafana_folder: efe-meta-swarm` annotation for organization ? | YES — added for AC18 ; default folder is "General" otherwise |

---

## 32. Appendix : full file inventory

(See § 6.)

---

## 33. Appendix : example overrides

### 33.1 Production multi-cluster (5 regions)

```yaml
# values-prod-eu.yaml — Europe region.
efe-meta-swarm:
  enabled: true
  image: {tag: "0.3.0"}
  manifest:
    meta: {dim: 32}
  controller:
    nodeSelector: {topology.kubernetes.io/region: eu-west-1}
  participants:
    - {id: supplier-scoring, dim: 32, kappa: 0.6, seed: 11, service: {grpcPort: 50091, metricsPort: 9101}}
    - ...
```

### 33.2 Per-tenant slice (one chart release per tenant)

```bash
$ for tenant in tenant-a tenant-b tenant-c ; do
    helm install efe-stack-$tenant ./charts/efe-stack \
      -n efe-prod \
      -f examples/values-prod.yaml \
      --set global.tenant=$tenant \
      --set efe-meta-swarm.enabled=true \
      --set global.metaSwarm.enabled=true
  done

# Operator queries per-tenant :
$ kubectl get pods -l efe.io/tenant=tenant-a
```

### 33.3 Air-gapped install (no external image pulls)

```yaml
image:
  repository: registry.internal/efe/efe-meta-swarm-server
  tag: "0.3.0"
  pullPolicy: IfNotPresent
hooks:
  image: "registry.internal/efe/kubectl:1.28"   # mirrored locally
bundle:
  sidecar:
    image: "registry.internal/efe/pause:3.9"
tests:
  grpcurlImage: "registry.internal/efe/grpcurl:v1.9.1"
```

---

## 34. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **L2 gate G11** | Phase-2 graduation gate "operator-grade Helm chart with automated reload + observability + multi-env values + bundle weight-loader contract" |
| **Helm hook** | A Helm-managed K8s resource that runs at a specific release lifecycle event (pre-install, pre-upgrade, post-upgrade, etc.) ; annotated with `helm.sh/hook` |
| **`backup-N` ConfigMap** | Snapshot of the manifest ConfigMap taken by the pre-upgrade hook ; indexed 0..4 in a rotating ring |
| **`shareProcessNamespace`** | K8s Pod spec field that makes containers share `/proc` ; required for cross-container `kill -HUP <pid>` |
| **Bundle weight-loader** | Sidecar container that polls a bundle endpoint and downloads model weights to a shared volume ; signals the participant via SIGHUP ; PR #14 stubs, PR #15 fills |
| **`grafana_dashboard: "1"`** | Label that Grafana sidecar (in `kube-prometheus-stack`) recognises to auto-import a dashboard JSON from a ConfigMap |
| **`PriorityClass`** | K8s scheduling.k8s.io/v1 resource that ranks pod priorities ; controller value 100000 wins evictions over participants |
| **HPA placeholder** | `HorizontalPodAutoscaler` template that ships but is locked to `min=max=1` ; PR #17 leader election will unlock it |
| **Issuer (cert-manager)** | A cert-manager CR that signs Certificates ; SelfSigned for Phase 2, CA for Phase 3 |
| **`ttlSecondsAfterFinished`** | K8s Job field that auto-deletes completed pods after N seconds ; hook Jobs use 300 (5 min) |
| **Canary slice** | A separate chart release with smaller scale, different tenant slug, deployed alongside production ; tests new manifest/image on a sliver of traffic |
| **Runbook URL** | Annotation on a PrometheusRule alert pointing at the operator's troubleshooting doc ; Alertmanager / PagerDuty link to it |

---

*End of architecture document. Total: ~ 2 500 lines.*

*Next deliverable : PR #14 code (8 commits, ~ +2 000 LOC of YAML + JSON + JSON Schema + helm-unittest).*
