# RB_M31_REDIS_OUTAGE — Metering Redis degraded

**Severity** : P2 (metering degraded ; billing accuracy at risk).
**Trigger** : `M31MeterFlushFailing` or `M31MeterFlushSlow`.

## 1. Diagnose
```bash
kubectl -n efe-system get pod -l app.kubernetes.io/name=redis
kubectl -n efe-system logs <redis-pod> --tail=200
```

Check the Redis dashboard for memory pressure, command latency,
connection-pool saturation.

## 2. Common causes

### 2.1 Memory pressure
- TTL 48h on counters means a stuck rollup cron lets keys
  accumulate. `redis-cli INFO memory` shows `used_memory_peak`.
- Mitigation : run the manual rollup runner :
  ```bash
  scripts/m31/rollup_runner.sh hour
  ```

### 2.2 Pod kill (chaos drill)
- Cluster mode replicates ; the failover promotes a replica.
- Observed window : ~ 10-30 s of `cache_unavailable` errors.
- Calling code is fail-open ; user requests are unaffected.

### 2.3 Network partition
- Check `kubectl get networkpolicy` for recent changes.

## 3. Customer impact

| Window | Visible | Action |
|---|---|---|
| 0-2 min | Fail-open ; in-memory buffer retains events. | None ; standard. |
| 2-30 min | Buffer flush retries fail ; events buffered in NATS mirror. | Page SRE. |
| > 30 min | NATS subject lag grows. Replay possible from PG event log later. | Page SRE + Compliance ; alert tenants of potential billing delay. |

## 4. Mitigation
1. Scale up Redis replicas if memory-related.
2. Restart `efe-server` pods to flush stale connection pools.
3. If sustained outage, manually patch `tenant_feature.billing_enabled = FALSE`
   for all tenants — invoices will be deferred to the next billing
   cycle.

## 5. Recovery validation
- Restart the rollup cron manually after Redis is healthy.
- Compare `usage_rollup_hour` of the last 24 h to NATS subject lag
  to confirm no event loss.

## 6. Post-incident
- File P2 incident.
- If recurrent : evaluate Redis cluster sizing.
- Update this runbook if any step required improvisation.
