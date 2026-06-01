# RB_M33_GRAPH_OOM — In-process graph exceeded pod memory

**Severity** : P2 ; P1 if multiple tenants affected.
**Trigger** : `M33GraphApproachingCap` or pod OOMKilled.

## 1. Diagnose
```bash
kubectl -n efe-system describe pod <efe-server>
kubectl -n efe-system top pod -l app.kubernetes.io/name=efe-server
```

## 2. Common causes

### 2.1 Tenant graph > 5 M nodes
- Single tenant past the soft cap. Hard cap 10 M.
- Mitigation : raise pod memory or split tenant to dedicated pod.

### 2.2 Embedding storage
- Storing 768-d embeddings on every entity → 3 KB/entity × 5 M =
  15 GB.
- Mitigation : drop embeddings from in-RAM graph (PG mirror keeps
  them). Use binary quantization on embeddings.

## 3. Customer impact
- All KG ops on the affected pod return 503.
- Inserts queue in NATS ; no data loss.

## 4. Mitigation
1. Vertical-scale memory immediately.
2. Long-term : split tenant via `efe-server` StatefulSet (M34).
