# Runbook : d18_replicas_below_minimum

> **Severity** : P1 (degraded — HA at risk)
> **ETA to resolution** : 5 min
> **Sprint origin** : D18
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Alert : `D18ObsReplicasBelowMin`
- `kubectl get hpa -n efe-production` shows current replicas < min
- `kubectl get pdb` violation

## 2. Why this matters

- High Availability at risk
- Pod disruption budget violation → cluster autoscaler may block scale-down
- Compliance : SOC 2 A1.2 availability

## 3. Diagnostics

```bash
# Current replica count
kubectl get deployment efe-server -n efe-production -o jsonpath='{.status.readyReplicas}/{.spec.replicas}'

# HPA state
kubectl describe hpa efe-server -n efe-production

# Recent events
kubectl get events -n efe-production --sort-by='.lastTimestamp' | tail -20

# Resource pressure
kubectl top pods -n efe-production
kubectl top nodes
```

## 4. Mitigation

### Quick (< 5 min)
1. Manually bump replica count :
   ```bash
   kubectl scale deployment efe-server -n efe-production --replicas=5
   ```
2. Check if nodes have capacity (kubectl top nodes)
3. If not, trigger cluster autoscaler

### Longer
4. Review HPA target CPU/memory thresholds
5. Investigate pod scheduling failures

## 5. Resolution criteria

- Replicas back to ≥ min
- PDB satisfied
- HPA stable for 15 min

## 6. Post-incident actions

- [ ] Tune HPA thresholds if pattern
- [ ] Add capacity if recurring

## 7. Related runbooks

- `d9_hpa_stuck_at_max.md` (D9)
- `d14_pg_pool_exhausted.md`
