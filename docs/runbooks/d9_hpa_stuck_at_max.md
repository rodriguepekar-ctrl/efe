# Runbook : d9_hpa_stuck_at_max

> **Severity** : P1 (capacity exhausted)
> **ETA to resolution** : 30 min
> **Sprint origin** : D9 (HPA scaling)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Alert : `D9ObsHpaStuckAtMax`
- HPA shows current replicas = max replicas
- CPU/memory usage > 80 % sustained
- Customer-reported latency degradation

## 2. Why this matters

- Capacity exhausted ; further load causes degradation
- SLO M11-RPC-3 (P99 < 200 ms) at risk
- Customer SLA P1 : 4h resolution

## 3. Diagnostics

```bash
# HPA state
kubectl describe hpa efe-server -n efe-production

# Current scale
kubectl get deployment efe-server -n efe-production -o jsonpath='{.status.readyReplicas}'

# Top resource users
kubectl top pods -n efe-production --sort-by=cpu

# Check if root cause is malicious load
kubectl logs -n efe-production -l app=efe-server --since 30m | grep -E "rate_limit|throttle"
```

## 4. Mitigation

### Quick (< 5 min)
1. Bump HPA max temporarily :
   ```bash
   kubectl patch hpa efe-server -n efe-production \
       -p '{"spec":{"maxReplicas":50}}'
   ```
2. Verify cluster has capacity (kubectl top nodes)
3. If capacity available, replicas should scale up

### Longer (15-30 min)
4. If actual load increase : tune HPA target thresholds
5. If malicious load : tighten rate limits (M11)
6. Add cluster nodes if needed

## 5. Resolution criteria

- HPA scaled up to handle load
- CPU/memory < 70 %
- P99 < 200 ms

## 6. Post-incident actions

- [ ] If recurring : update HPA permanent max
- [ ] Capacity planning review

## 7. Related runbooks

- `d18_replicas_below_minimum.md`
- `d14_pg_pool_exhausted.md`
