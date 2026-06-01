# RB_M32_INDEX_OOM — `efe-server` pod hit memory limit during HNSW build

**Severity** : P2 (collection unavailable). P1 if multi-tenant.
**Trigger** : `M32CollectionApproachingCap` or pod OOMKilled.

## 1. Diagnose
```bash
kubectl -n efe-system describe pod <efe-server-pod> | head -100
kubectl -n efe-system logs <efe-server-pod> --previous | tail -200
```

## 2. Common causes

### 2.1 Large collection on small replica
- 5 M docs × dim 1536 × 4 bytes ≈ 30 GB. Pod limit 4 GB ⇒ OOM.
- Mitigation : raise pod memory limit OR enable binary quantization
  via collection patch :
  ```sql
  UPDATE vector_collection SET hnsw_m=8 WHERE name='big';
  ```

### 2.2 Multiple large collections
- Several 1M-doc collections co-located. Sum exceeds RAM.
- Mitigation : split tenants across pods via `efe-server`
  StatefulSet (deferred to M32.1).

### 2.3 Adversarial dim
- Tenant inserted vectors with dim 4096 → 4× memory vs dim 768.
- Verify `MAX_DIM = 4096` is enforced (it should reject upstream).

## 3. Customer impact
- All searches on the affected collection return 503 until pod
  recovers.
- Inserts queue in NATS ; no data loss.

## 4. Mitigation
1. Scale up `efe-server` memory limit immediately.
2. Switch the collection to `pgvector` backend (M32.1).
3. As a last resort, drop the collection (after operator + tenant
   alignment).

## 5. Post-incident
- File P2 incident.
- Add the offending tenant to the quota uplift workflow.
