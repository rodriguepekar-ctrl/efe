# EFE — Chaos engineering runbook

> **Sprint D9 deliverable** — covers the 8 chaos scenarios from
> the deep-analysis supplement §10.
>
> 3 of 8 scenarios are GUARANTEED by the Helm chart's defaults
> (Pod kill, OOM, network policy). The remaining 5 require manual
> commands ; this document is the operator's cheat-sheet.

---

## Overview

| # | Scenario | Trigger | Recovery cible | Covered by D9 chart? |
|--:|----------|---------|---------------:|:---:|
| 1 | Pod kill | `kubectl delete pod` | < 30 s | ✅ |
| 2 | Node drain | `kubectl drain node` | < 60 s | partial |
| 3 | Control plane restart | n/a (cluster-level) | invisible | external |
| 4 | Network partition | NetworkPolicy egress=deny | pod isolated < 30 s | ✅ |
| 5 | DNS failure | `kubectl scale coredns --replicas=0` | mostly OK (cache) | external |
| 6 | OOM kill | inject memory burst | restart < 60 s | ✅ |
| 7 | Slow disk | `tc-netem` / cgroup IO throttle | latency degrades but no 5xx | partial |
| 8 | RPC corruption | `tc netem corrupt 0.1 %` | client retry → success | external |

Notes: "external" means a cluster-level concern not in EFE's
purview. "partial" means the chart sets reasonable defaults but
full validation requires a chaos harness (chaos-mesh / litmus).

---

## Scenario 1 — Pod kill

### Reproduce

```bash
# 1. Confirm baseline.
kubectl get pods -n efe -l app.kubernetes.io/name=efe
# Expect: at least 2 Ready Pods.

# 2. Inject.
target=$(kubectl get pod -n efe -l app.kubernetes.io/name=efe \
    -o jsonpath='{.items[0].metadata.name}')
kubectl delete pod -n efe "$target"

# 3. Watch recovery.
watch -n 1 kubectl get pods -n efe -l app.kubernetes.io/name=efe
```

### Expected behaviour

* `preStop` hook runs (`sleep 15`) → readiness probe fails → Service
  drops the Pod from the backend pool.
* In-flight requests finish within `terminationGracePeriodSeconds=30`.
* New Pod starts ; passes startup + readiness probes.
* Total user-visible disruption: 0 (assuming `replicaCount ≥ 2`).

### What to check

* `grpc_server_handled_total{grpc_code!="OK"}` should NOT spike.
* `up{job="efe"}` may briefly drop one Pod from the count, recovers
  within ~10 s.

---

## Scenario 2 — Node drain

### Reproduce

```bash
# Pick a node hosting EFE pods.
node=$(kubectl get pod -n efe -l app.kubernetes.io/name=efe \
    -o jsonpath='{.items[0].spec.nodeName}')
kubectl drain "$node" --ignore-daemonsets --delete-emptydir-data
```

### Expected behaviour

* `kubectl drain` cordons the node (no new Pods scheduled there).
* For each EFE Pod on the node: PDB `minAvailable: 2` may BLOCK
  the drain if exactly 2 Pods are Ready and 1 is on this node.
* Pods evict one-by-one ; replacement Pods come up on other nodes.

### Caveat

If your cluster has only 1 node (minikube), drain will fail —
expected. Use 3+ nodes to validate this scenario.

### Uncordon when done

```bash
kubectl uncordon "$node"
```

---

## Scenario 3 — Control plane restart

This is cluster operator territory ; EFE has no role.

### Confirm no impact on dataplane

```bash
# After your platform team triggers the control plane restart:
kubectl logs -n efe -l app.kubernetes.io/name=efe --tail=50
# Expect: no errors, EFE keeps serving.
```

EFE does not call the k8s API at runtime
(`automountServiceAccountToken: false`). Control plane outage is
transparent.

---

## Scenario 4 — Network partition (intra-cluster)

The default `NetworkPolicy` denies egress to anywhere not in the
allowed namespaces. Test by hardening it further:

```bash
# Patch to deny all egress.
kubectl patch -n efe networkpolicy efe --type=json \
    -p='[{"op":"replace","path":"/spec/egress","value":[]}]'

# Try a request (port-forward + python smoke).
# Expected: timeout — DNS to coredns blocked.

# Restore.
helm upgrade efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 -n efe --reuse-values
```

### What this proves

The NetworkPolicy correctly enforces egress restrictions. Combined
with a tightened ingress (only from labeled namespaces), this
limits blast radius if EFE is compromised.

---

## Scenario 5 — DNS failure

```bash
# Scale coredns down (BREAKS the cluster — staging only!)
kubectl scale -n kube-system deployment/coredns --replicas=0
```

### Expected behaviour

* gRPC clients with already-resolved DNS (in-cache) keep working
  thanks to keepalive on existing connections.
* New connections fail.
* When coredns scales back up, recovery is < 60 s.

### Restore

```bash
kubectl scale -n kube-system deployment/coredns --replicas=2
```

---

## Scenario 6 — OOM kill

### Reproduce (DEV / STAGING ONLY)

The simplest way is to lower the memory limit in Helm, then send
a stress burst :

```bash
# 1. Lower the limit so a stress burst will OOM.
helm upgrade efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 -n efe --reuse-values \
    --set resources.limits.memory=128Mi

# 2. Hammer the server.
bash bench/load/run_load_test.sh stress --endpoint <endpoint>

# 3. Observe.
kubectl get pods -n efe -l app.kubernetes.io/name=efe
# Expect: occasional CrashLoopBackOff or OOMKilled events.

# 4. Restore.
helm upgrade efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 -n efe --reuse-values
```

### What this proves

* Pods OOM ⇒ kubelet restarts them.
* Service routes traffic away from killed Pods (readiness fails).
* No 5xx escapes the cluster.

---

## Scenario 7 — Slow disk

EFE's I/O is minimal (mostly logs to stdout, no DB), so disk slowness
mostly affects audit-log writes. Reproduce with `tc-netem` on the
audit-log volume:

```bash
# Install tc on the node (privileged) — STAGING ONLY.
# This is platform-specific; consult chaos-mesh for k8s-native equivalents.
```

### Expected behaviour

* p99 latency degrades smoothly (200ms → 500ms+).
* No request is dropped.
* No SLO `D9-LOAD-1` breach unless degradation > 5 s p99.

---

## Scenario 8 — RPC corruption

```bash
# Inject 0.1 % packet corruption (STAGING ONLY).
# Kernel-level — out of scope for this runbook.
# See chaos-mesh "NetworkChaos" CRD for k8s-native injection.
```

### Expected behaviour

* gRPC client retries (HTTP/2 stream reset).
* Eventually success.
* Metric `grpc_server_handled_total{grpc_code="Internal"}` may
  show non-zero count ; alert threshold is 1 % (tolerable).

---

## Where to go for automated chaos

D9 ships only the **runbook**. For automated chaos engineering,
the Phase 3 plan integrates :

* [chaos-mesh](https://chaos-mesh.org) — k8s-native CRDs for
  pod-kill, network partition, DNS failure, IO throttle.
* [litmus](https://litmuschaos.io) — alternative k8s-native chaos.

Both work as drop-ins ; the chart already exposes the labels they
need (`app.kubernetes.io/name=efe`).

---

## See also

* [`troubleshooting.md`](troubleshooting.md) — manual debugging.
* Deep analysis §10 — design rationale for the 8 scenarios.
* `deploy/chaos/game-day.md` — quarterly game-day script.
