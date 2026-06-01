# Runbook — GPU OOM Recovery

> **Sévérité** : critical (PagerDuty P0).
> **Trigger** : alerte `GPU_OOM_BURST` (= OOM > 1/h) ou
> `GPU_SM_OCCUPANCY_LOW` post-OOM event.
> **Sprint d'origine** : Φ-D D31.
> **Référence architecture** : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md`
> § 7 + § 11.

---

## TL;DR

Le GPU est saturé en VRAM. Le circuit breaker `shrink batch + retry`
ne suffit pas. Les requests reçoivent `RESOURCE_EXHAUSTED` et P1
SLO violation imminente.

Trois causes typiques :
1. **Tenant noisy neighbor** : 1 tenant push trop de batch.
2. **VRAM leak** : arenas non-reset entre requêtes.
3. **Memory growth** : tile size mal calibré ou multi-tenant
   sharing trop agressif.

---

## 1. Triage (≤ 5 min)

### 1.1 Confirmer OOM réel

```bash
# OOM count sur 1h
$ curl -s 'http://prometheus:9090/api/v1/query?query=increase(efe_gpu_oom_total[1h])' | jq

# VRAM usage par device
$ nvidia-smi --query-gpu=memory.used,memory.total --format=csv

# Arena reset cadence (= devrait être ≈ RPS)
$ curl -s 'http://prometheus:9090/api/v1/query?query=efe:gpu_arena_reset_vs_rps_ratio' | jq
# Si < 0.8 : arenas pas reset → VRAM leak.
# Si > 1.2 : oubli reset = double-comptage (= bug différent).
```

### 1.2 Identifier le tenant noisy

```bash
# Top tenants sur les 5 dernières minutes
$ kubectl logs -n efe -l app=efe-server | jq '
    select(.fields.gpu_oom == true)
    | .fields.tenant_id
' | sort | uniq -c | sort -rn | head
```

---

## 2. Mitigations immédiates (≤ 15 min)

### 2.1 Rate-limit le tenant noisy

```bash
# Cap à 50 RPS sur ce tenant
$ kubectl exec -n efe efe-server-0 -- \
    curl -X PUT localhost:8081/admin/rate_limit \
    --data '{"tenant_id": "<id>", "rps_cap": 50}'
```

### 2.2 Réduire batch coalescer (= conservative mode)

```bash
$ kubectl set env deployment/efe-server -n efe \
    EFE_BATCH_COALESCE_MAX=8       # default 32 → 8
```

Effet : VRAM peak diminue ~ 4×. Trade-off : latence p99 ↑ légère.

### 2.3 Forcer arena reset agressif

```bash
$ kubectl set env deployment/efe-server -n efe \
    EFE_VRAM_ARENA_RESET_AFTER_REQUEST=true \
    EFE_VRAM_ARENA_FORCE_RESET_INTERVAL_SECS=60
```

---

## 3. Investigation root cause (≤ 1 h)

### 3.1 Memory leak detection

```bash
# RSS + VRAM trajectoire sur 6h
$ kubectl exec -n efe efe-server-0 -- /efe/bin/efe-server memory-report

# Si RSS croît linéairement → leak côté Rust (= cf. tokio_console)
# Si VRAM seul croît → leak côté cudarc / arena
```

### 3.2 Profile avec nsight-compute

```bash
# Spawn 1 replica en mode profiling (sur GPU dédié)
$ kubectl run efe-debug --image=registry.efe.ai/efe-server:debug-nsight \
    --env="EFE_DEBUG_NSIGHT_AUTO=true" \
    --env="EFE_GPU_DEVICE=1"   # = pas le GPU prod
```

Profile uploadé S3 + lien dans la trace OTel.

---

## 4. Mitigations long terme (≤ 24 h)

### 4.1 Re-calibrer tile sizes

```bash
$ ./target/release/efe-server tune --gpu-model t4 --benchmark-iterations 1000
# → produit `/etc/efe/gpu-tuned.env`
# Apply via Helm chart bump.
```

### 4.2 Augmenter VRAM quota per-tenant

Si le quota actuel (par défaut 200 MiB/tenant) est trop serré
pour les Enterprise clients :

```bash
$ kubectl exec -n efe efe-server-0 -- /efe/bin/efe-admin tenant set-quota \
    --tenant-id <id> --vram-mib 500
```

### 4.3 Provision plus de GPU

Si la cause est juste "plus de trafic que prévu" :

```bash
$ kubectl scale deployment/efe-server -n efe --replicas=8
```

ou bump GPU class : T4 → L4 (= 24 GiB VRAM).

---

## 5. Post-incident (≤ 5 j)

1. **RCA publique** sur `status.efe.ai`
2. **Post-mortem interne** (= template P2)
3. **Ajout chaos test** : injecter OOM artificiel dans `tests/p2_chaos_gpu_oom.rs`
4. **Update tune defaults** : si root cause = tile size, update
   `src/tune.rs` lookup table.

---

## 6. Liens utiles

- Architecture : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 7
- Dashboard : https://grafana.efe.ai/d/efe-gpu-kernels
- Runbook driver upgrade : `docs/runbook/gpu_driver_upgrade.md`
- Runbook multi-GPU failover : `docs/runbook/multi_gpu_failover.md`
