# Runbook — Multi-GPU NCCL Failover

> **Sévérité** : critical (PagerDuty P0) si > 50 % du fleet down ;
> warning sinon.
> **Trigger** : alerte `GPU_NCCL_ALLREDUCE_SLOW` ou
> détection 1 GPU lost dans un cluster 4× T4.
> **Sprint d'origine** : Φ-D D31.
> **Référence architecture** : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md`
> § 9.5.

---

## TL;DR

Un cluster multi-GPU (= typiquement 4× T4 sur un node g4dn.12xlarge)
a perdu 1+ GPUs ou son NCCL communicateur est en hang. Le multi-GPU
speedup ≥ 3× attendu (SLO-CUDA-MULTI-GPU-SPEEDUP) est perdu ;
le serveur dégrade automatiquement vers single-GPU.

3 scénarios possibles :
1. **1 GPU lost** (= drain k8s, ECC error) → comm reset
2. **NCCL hang** (= deadlock topology) → process restart
3. **PCIe degraded** (= bandwidth ↓ 10×) → alert ops, manuel

---

## 1. Triage (≤ 5 min)

### 1.1 Identifier le scénario

```bash
# GPU count visible
$ kubectl exec -n efe efe-server-cluster-0 -- nvidia-smi --list-gpus | wc -l
# Si < 4 → scénario 1 (GPU lost)

# NCCL allreduce p99 sur 5 min
$ curl -s 'http://prometheus:9090/api/v1/query?query=efe:gpu_nccl_allreduce_p99_seconds*1000' | jq
# > 100 ms → scénario 2 (hang) ou 3 (PCIe degraded)

# PCIe bandwidth probe (= rare, sur demande)
$ kubectl exec -n efe efe-server-cluster-0 -- /efe/bin/efe-server pcie-probe
# Si < 10 GB/s expected → scénario 3
```

### 1.2 Vérifier impact P1

```bash
# Le serveur dégrade-t-il en mode single-GPU ?
$ kubectl logs -n efe efe-server-cluster-0 | grep MULTI_GPU_DEGRADED
```

---

## 2. Scénario 1 — 1 GPU Lost (= drain ou ECC error)

### 2.1 Détection

```
[ERROR] NCCL_FAIL : ncclAllReduce returned NCCL_UNHANDLED_CUDA_ERROR
[WARN]  MULTI_GPU_DEGRADED : falling back to 3-GPU comm (rank 2 down)
```

### 2.2 Action automatique (= déjà câblé Φ-D D30)

Le code Rust déjà gère :
1. NCCL `ncclAllReduce` returns NCCL_FAIL.
2. Try `ncclCommReset` (= reconfigure comm avec rangs survivants).
3. Si reset échoue → kill process → k8s replace pod.

### 2.3 Validation post-failover

```bash
# Vérifier que le pod a redémarré
$ kubectl get pod -n efe -l app=efe-server-cluster -o jsonpath='{.items[*].status.containerStatuses[*].restartCount}'

# Vérifier nouvelle config NCCL
$ kubectl logs -n efe efe-server-cluster-0 | grep "NCCL.*world_size"
```

### 2.4 Si le GPU est physiquement HS

Ouvrir un ticket cloud provider (AWS support / GCP support) pour
remplacer le node. K8s replace le pod sur un autre node sain.

---

## 3. Scénario 2 — NCCL Hang (= deadlock)

### 3.1 Détection

```
[ERROR] NCCL operation timed out after 10s : all-reduce N=1024 D=384
```

### 3.2 Mitigations

#### a) Kill + restart

```bash
$ kubectl delete pod -n efe efe-server-cluster-0
# K8s replace ; restart NCCL clean.
```

#### b) Topology mismatch ?

```bash
# Vérifier que tous les GPUs voient les autres via PCIe
$ kubectl exec -n efe efe-server-cluster-0 -- nvidia-smi topo -m
```

Si pas d'NVLink mais cluster déclaré comme tel → mismatch. Fix :

```bash
$ kubectl set env deployment/efe-server-cluster -n efe \
    EFE_NCCL_TOPOLOGY=pcie    # forcer PCIe tree
```

---

## 4. Scénario 3 — PCIe Degraded

### 4.1 Détection

```bash
# NCCL allreduce p99 ↑ 5-10× sans GPU lost
$ kubectl exec -n efe efe-server-cluster-0 -- /efe/bin/efe-server pcie-probe
PCIe Gen 3 link width x16 ; bandwidth 12 GB/s (expected) → 1.2 GB/s (measured)
                                                            ⚠️ DEGRADED 10×
```

### 4.2 Action

PCIe degraded ne se "fix" pas runtime — c'est typiquement un
hardware issue (= card seating, slot dégradé). Action ops :

1. **Cordon le node** (= isoler de la charge production)
2. **Ticket cloud provider** pour replace le node
3. **K8s replace pod** sur un autre node

---

## 5. Reset complet du cluster (= nuclear option)

Si tout fail :

```bash
$ kubectl scale deployment/efe-server-cluster -n efe --replicas=0
$ sleep 30
$ kubectl scale deployment/efe-server-cluster -n efe --replicas=1
```

Effet : tout le NCCL communicateur est réinitialisé. Trafic
single-GPU pendant le reset (~ 60 s).

---

## 6. Post-incident

1. Métriques `efe_gpu_nccl_allreduce_seconds` revenues sous 10 ms p99
2. `SLO-CUDA-MULTI-GPU-SPEEDUP` PASS sur prochain bench
3. Ouvrir Jira ticket si root cause = hardware (= cloud provider claim)
4. Post-mortem si downtime > 30 min

---

## 7. Liens utiles

- Architecture : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 9
- NCCL troubleshooting : https://docs.nvidia.com/deeplearning/nccl/user-guide/docs/troubleshooting.html
- Runbook OOM : `docs/runbook/gpu_oom_recovery.md`
- Runbook driver upgrade : `docs/runbook/gpu_driver_upgrade.md`
