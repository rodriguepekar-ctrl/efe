# efe-meta-swarm Helm sub-chart

Phase-2 sub-chart that packages the meta-swarm controller + N
participants for K8s deployment.  Lives under
`charts/efe-stack/charts/efe-meta-swarm/` as a sibling of `efe-server`,
`efe-edge`, `efe-vector`, etc.

**Status** : PR #13 — initial skeleton.  Disabled by default.

See `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md` for the full design.

---

## What ships

When `enabled: true`, the chart provisions :

  * 1 × controller `Deployment` + `Service` + `ServiceAccount` + `PodDisruptionBudget`.
  * N × participant `Deployment` + `Service` + `ServiceAccount` (default N = 3 — the Kovant scenario).
  * 1 × `ConfigMap` rendering the Phase-2 manifest YAML.
  * Optional `ServiceMonitor` for Prometheus Operator stacks.
  * Optional `NetworkPolicy` for deny-by-default east-west traffic.
  * 1 × `helm test` Pod that probes `MetaSwarmService::Health` via grpcurl.

The sub-chart is **opt-in** : with `enabled: false` (the default),
zero resources are rendered.

## Quick start

```bash
# 1. Pin the image tag (values.schema.json rejects empty).
$ cat > values-dev.yaml <<EOF
efe-meta-swarm:
  enabled: true
  image:
    tag: "0.2.0"
EOF

# 2. Install.
$ helm install efe-stack ./charts/efe-stack \
    --namespace efe --create-namespace \
    -f values-dev.yaml

# 3. Verify pods.
$ kubectl -n efe get pods -l efe.io/meta-swarm-role

# 4. Run the connection probe.
$ helm test efe-stack -n efe
```

## Hot reload via SIGHUP

PR #12 ships SIGHUP-driven manifest reload.  The chart **deliberately**
does NOT use a `checksum/config` annotation on the Deployment pod
templates — editing the ConfigMap does **not** trigger a rolling
restart.  Instead, the operator triggers the reload explicitly :

```bash
# 1. Edit the manifest values.
$ vim values-prod.yaml

# 2. Helm upgrade — only the ConfigMap mutates ; pods stay up.
$ helm upgrade efe-stack ./charts/efe-stack -n efe -f values-prod.yaml

# 3. SIGHUP the controller.
$ kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-controller -- kill -HUP 1

# 4. SIGHUP every participant.
$ for id in supplier-scoring contract-risk spend-categorization ; do
    kubectl -n efe exec deploy/efe-stack-efe-meta-swarm-participant-$id -- kill -HUP 1
  done

# 5. Verify the reload landed (reload_gen should bump).
$ kubectl -n efe port-forward svc/efe-stack-efe-meta-swarm-controller 50080:50080 &
$ grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health \
  | jq '.reloadGen'
```

## values.yaml reference

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Master switch — `false` renders zero resources |
| `image.repository` | string | `ghcr.io/efe-ai/efe-meta-swarm-server` | Container image |
| `image.tag` | string | `""` | **REQUIRED** — pin a real tag (no `:latest`) |
| `image.pullPolicy` | string | `IfNotPresent` | `Always` / `IfNotPresent` / `Never` |
| `manifest.version` | string | `v0.2.0` | Schema version |
| `manifest.meta.dim` | int | `16` | Meta-space dim |
| `manifest.meta.safety.kind` | string | `box` | `box` / `poly` / `hull` |
| `manifest.iteration.max_iters` | int | `100` | Convergence iter cap |
| `manifest.iteration.tolerance` | float | `1e-4` | Convergence threshold |
| `manifest.iteration.alpha_default` | float | `0.5` | Per-step convex weight (∈ (0, 1)) |
| `manifest.iteration.coupling_gamma` | float | `0.2` | Coupling weight (∈ (0, 1)) |
| `manifest.iteration.max_wall_ms` | int | `15000` | Per-Iterate deadline (cap 30000) |
| `controller.replicaCount` | int | `1` | **Locked** at 1 in Phase 2 |
| `controller.service.grpcPort` | int | `50080` | gRPC bind |
| `controller.service.metricsPort` | int | `9091` | Prometheus bind |
| `controller.config.maxInflight` | int | `10` | MAX_INFLIGHT (T7 mitigation) |
| `controller.config.discovery` | string | `static-localhost` | `static-localhost` / `k8s-dns` |
| `controller.resources` | object | `{cpu: 500m, mem: 512Mi}` / `{cpu: 2, mem: 2Gi}` | Standard K8s |
| `controller.podDisruptionBudget.enabled` | bool | `true` | PDB on by default |
| `controller.podDisruptionBudget.maxUnavailable` | int | `0` | Voluntary disruption locked |
| `participants[].id` | string | (varies) | DNS-compliant ; matches manifest entry |
| `participants[].dim` | int | (varies) | Participant tensor dim |
| `participants[].kappa` | float | (varies) | Contraction const ∈ (0, 1) |
| `participants[].seed` | int | (varies) | RNG seed |
| `participants[].replicaCount` | int | `1` | **Locked** at 1 in Phase 2 |
| `serviceAccount.create` | bool | `true` | One SA per role |
| `networkPolicy.enabled` | bool | `false` | Opt-in deny-by-default |
| `metrics.serviceMonitor.enabled` | bool | `false` | Opt-in (requires Prometheus Operator) |
| `tests.enabled` | bool | `true` | `helm test` probe Pod |

Full schema enforcement in `values.schema.json`.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `helm install` rejects `kappa` | Out of `(0, 1)` range | Edit values |
| `helm install` rejects empty `image.tag` | No tag pinned | Set `image.tag` explicitly |
| Pod stuck in `ContainerCreating` | Image pull failure | `kubectl describe pod` ; check pullSecrets |
| Pod `CrashLoopBackOff` | manifest validation failed at boot | `kubectl logs` — `validate_manifest` error message |
| `helm test` fails `connection refused` | Controller not Ready | Wait + retry ; check liveness probe |
| `reload_gen` stays at 0 after edit | Forgot `kill -HUP 1` | Issue SIGHUP per pod |
| Pods crashing after SIGHUP | Reload rejected (drift) | `kubectl logs` — `validate_reload_compatibility` reason |
| `RELOADING (3)` status sticks | Reload swap never completed (controller panic mid-swap, unlikely with PR #12 safety) | `kubectl logs` ; if needed `helm rollback` |
| `ServiceMonitor` not scraped | `prometheus-operator` CRDs missing | Install operator or set `metrics.serviceMonitor.enabled: false` |
| `NetworkPolicy` no effect | CNI doesn't enforce policy (e.g. Flannel) | Use Calico / Cilium / Weave-policy |

## CNI compatibility (NetworkPolicy)

The `NetworkPolicy` resources need a CNI plug-in that **enforces** them.
Compatible CNIs :

  * Calico
  * Cilium
  * Weave (with `--enable-network-policies`)
  * Antrea
  * AWS VPC CNI (with `enableNetworkPolicy: true`)

Incompatible (renders but no-ops) :

  * Flannel (default for many k3s / kubeadm installs)
  * AWS VPC CNI without policy add-on

## References

  * Phase-2 architecture : `docs/EFE_A1_PHASE2_ARCHITECTURE.md`
  * PR #11 (efe-server wiring) : `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md`
  * PR #12 (SIGHUP + ReloadConfig) : `docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md`
  * PR #13 (this chart skeleton) : `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md`
  * PR #14 (this chart — production-grade) : `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md`
  * PR #16 (mTLS + cert-manager + rustls) : `docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md`
  * PR #17 (Prometheus metrics + STABILITY_CONTRACT v1.0) : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md`
  * PR #18 (OTel spans + trace context) : `docs/EFE_A1_PHASE2_PR18_ARCHITECTURE.md`
  * **PR #19** (9 dashboards + 10 alerts + 4 SLOs) : `docs/EFE_A1_PHASE2_PR19_ARCHITECTURE.md`
  * STABILITY_CONTRACT (metrics + dashboards + alerts + SLOs) : `docs/STABILITY_CONTRACT.md` v1.1
  * SLO contract (4 meta-swarm SLOs since PR #19) : `docs/slo.md`

---

# PR #14 additions

PR #14 adds five operator-facing capabilities on top of PR #13's
skeleton.  Each is OFF by default ; flip the corresponding values
flag to opt in.

| Feature | values flag | Default | Purpose |
|---|---|---|---|
| Multi-env values flavors | (example files) | n/a | Operator-friendly per-env overlays |
| PriorityClass for controller | `priorityClass.enabled` | `false` | Survives resource pressure |
| Helm pre/post-upgrade hooks | `hooks.enabled` | `false` | Auto SIGHUP + manifest backup |
| Bundle weight-loader sidecar | `bundle.sidecar.enabled` | `false` | Phase-3 bundle reload coordination |
| PrometheusRule alerts | `alerts.enabled` | `false` | 6 alerts on PR #12 metrics |
| Grafana dashboard | `dashboard.enabled` | `false` | Auto-imported by Grafana sidecar |
| TLS bootstrap stubs | `tls.enabled` | `false` | cert-manager Issuer + Certificate × 4 |

## Multi-env values flavors

Four operator-curated overlays in `examples/` :

  * `examples/values-dev.yaml` — local laptop K8s
  * `examples/values-staging.yaml` — pre-prod cluster
  * `examples/values-prod.yaml` — production
  * `examples/values-canary.yaml` — canary slice alongside production

See `examples/README.md` for usage.

## Pre/Post-upgrade hooks

When `hooks.enabled: true`, two Helm Jobs run automatically :

  * **`pre-upgrade-snapshot`** (weight `-10`) : snapshots the live
    manifest `ConfigMap` to a rotating ring of 5 backups indexed by
    `efe.io/backup-index`.  Operator can restore via :

    ```bash
    kubectl get cm -l efe.io/manifest-state=backup
    kubectl get cm <release>-efe-meta-swarm-manifest-backup-2 -o json \
      | jq '.metadata.name = "<release>-efe-meta-swarm-manifest"
            | .metadata.annotations["efe.io/manifest-state"] = "live"
            | del(.metadata.uid, .metadata.resourceVersion)' \
      | kubectl apply -f -
    ```

  * **`post-upgrade-sighup`** (weight `+10`) : SIGHUPs the controller
    + every participant after the upgrade applies — automates PR #12's
    runbook step.  `hooks.postUpgrade.ignoreFailures: true` (default
    dev/staging) treats SIGHUP failures as warnings ; in prod set
    `ignoreFailures: false` to fail-fast.

A dedicated `ServiceAccount` + `Role` + `RoleBinding` gives the hook
Jobs minimal permissions : `configmaps` CRUD + `pods/exec` + `deployments`
read (T25 mitigation).

## Bundle weight-loader sidecar

When `bundle.sidecar.enabled: true`, every participant Pod gets a
second container with `shareProcessNamespace: true` and a shared
`emptyDir` (Memory) volume at `/var/lib/efe-meta-swarm/weights/`.

Phase 2 ships a `pause:3.9` placeholder (no-op).  PR #15 will swap for
the real polling binary that :

  1. Polls `GET ${EFE_BUNDLE_ENDPOINT_URL}/metadata?id=<id>`
  2. On SHA change, downloads the blob
  3. Atomic rename → `/var/lib/efe-meta-swarm/weights/current.bin`
  4. `kill -HUP 1` (signals the participant via the shared PID namespace)

**T26 trust boundary** : `shareProcessNamespace: true` exposes `/proc`
between containers.  Both containers run as `runAsNonRoot: true` + same
UID `65532` + `readOnlyRootFilesystem: true` to keep the trust domain
tight.  Documented as a known trade-off.

## Prometheus alerts

When `alerts.enabled: true`, the chart renders a `PrometheusRule`
with 6 alerts spanning :

| Alert | Default severity | Trigger |
|---|---|---|
| `MetaSwarmReloadFailureRate` | warning | Reload failure rate > 0.1/sec for 5 min |
| `MetaSwarmReloadLatencyP99High` | warning | p99 reload duration > 1 sec for 10 min |
| `MetaSwarmReloadingStuck` | critical | RELOADING status held > 1 min |
| `MetaSwarmControllerDown` | critical | Controller pod down > 5 min |
| `MetaSwarmParticipantDown` | warning | Any participant down > 5 min |
| `MetaSwarmReloadGenNotAdvancing` | info | `reload_gen` flat for 24h |

Each alert carries a `runbook_url` annotation pointing at
`alerts.runbookBaseUrl#<alert-slug>`.

## Grafana dashboard

When `dashboard.enabled: true`, a `ConfigMap` carries the dashboard
JSON with the `grafana_dashboard: "1"` label — Grafana sidecar
(in `kube-prometheus-stack`) auto-imports it.

Panels :
  * Reload Generation (stat)
  * Reload Attempts (time series)
  * Reload Failures by Reason (stacked)
  * Reload Duration p50/p90/p99
  * Health Probes (success + error rates)
  * Health Probe Latency p99
  * Pod Up Status (per role)

## TLS bootstrap stubs

When `tls.enabled: true` AND `tls.certManager.enabled: true`, the
chart provisions a cert-manager `Issuer` + one `Certificate` per role
(controller + each participant).  Deployments mount the issued
`Secret` at `/etc/efe-meta-swarm/tls/` and set the
`EFE_META_SWARM_TLS_{CERT,KEY,CA}` env vars to match PR #6's binary
contract.

PR #16 will wire the binary's tonic-tls integration ; PR #14 just
stages the K8s artefacts.

## Operator runbook — extended scenarios

### Scenario 5 : Hook Job failure recovery

```bash
# 1. Inspect.
kubectl get jobs -l app.kubernetes.io/component=hook
kubectl logs job/<release>-hook-post-upgrade-sighup

# 2. Fix the root cause (e.g., image pull, pod CrashLoop).

# 3. Once pods Ready, SIGHUP manually.
kubectl exec deploy/<release>-efe-meta-swarm-controller -- kill -HUP 1
```

### Scenario 6 : Rollback to a previous manifest via backup ring

```bash
# 1. List backups.
kubectl get cm -l efe.io/manifest-state=backup \
  -o custom-columns='NAME:metadata.name,IDX:metadata.annotations.efe\.io/backup-index,TAKEN:metadata.annotations.efe\.io/backup-taken-at'

# 2. Promote backup-N to live + SIGHUP all.
```

### Scenario 7-11

(See `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md` § 30 for the full
runbook covering 11 operator scenarios.)

## Threat model — additions for PR #14

  * **T25** (Helm hook over-privileged RBAC) → dedicated SA, Role
    scoped to namespace, verbs listed exhaustively (no `*`).
  * **T26** (sidecar `shareProcessNamespace` exposes `/proc`) →
    both containers run as nonroot + readOnlyRootFilesystem + same
    UID 65532 ; trust boundary documented.
  * **T27** (backup ConfigMap rotation tampering) → labels +
    annotations are auditable ; K8s `Event`s emit on each backup ;
    restoration is a deliberate `kubectl apply` (logged).
  * **T28** (unauthenticated bundle endpoint) → Phase-2 sidecar is
    `pause` (no traffic) ; PR #16 wires mTLS for the bundle endpoint.
