# RB_M29_PII_CIRCUIT_OPEN — Presidio sidecar outage

**Severity** : P2 (degraded ; strict-policy tenants impacted).
**Trigger** : `M29PiiCircuitOpen` or `M29PiiLatencyHigh`.

## 1. Check Presidio pods
```bash
kubectl -n efe-stack get pods -l app.kubernetes.io/name=efe-pii
kubectl -n efe-stack logs <presidio-pod> --tail=200
```

## 2. Common causes

### 2.1 Pod OOM
- Default 512Mi may be insufficient under load.
- Scale up :
  ```bash
  kubectl -n efe-stack patch deployment efe-stack-pii \
    --patch '{"spec":{"template":{"spec":{"containers":[{"name":"presidio","resources":{"limits":{"memory":"4Gi"}}}]}}}}'
  ```

### 2.2 Model load failure
- Init container missing en/fr language models.
- Re-pull image (some Microsoft tags ship without all models) :
  ```bash
  kubectl -n efe-stack delete pod -l app.kubernetes.io/name=efe-pii
  ```

### 2.3 Timeout cascade
- `EFE_PII_TIMEOUT_MS` too low for typical input length. Inspect
  p99 latency. Raise to 200ms if input length > 1000 chars typical.

## 3. Customer impact

### policy = off
- Zero impact.

### policy = detect | redact
- Fail-open : input passes through unredacted. Audit event
  `pii.unavailable` is logged. Customer should be notified.

### policy = strict
- Reject : request returns 422. **Tenant blocked from classify.**
- For high-criticality customers : the breaker auto-reset after 30s
  ; if it stays open, page SRE.

## 4. Emergency disable
If Presidio is completely down and breaking strict tenants :
```sql
-- Force every tenant to `detect` mode temporarily :
UPDATE tenant_feature SET pii_policy='detect' WHERE pii_policy='strict';
-- Restore from a backup script after Presidio recovers.
```

Avoid this : prefer scaling Presidio first.

## 5. Post-incident
- File P2 incident.
- If recurrent : evaluate redundancy (replicas → 3+) or alternative
  PII engines (cloud failover via tenant policy override).
