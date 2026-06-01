# Runbook : d14_postgres_failover

> **Severity** : P0 (data backend offline)
> **ETA to resolution** : 30 min
> **Sprint origin** : D14 (Postgres backend)
> **Last updated** : 2026-04-30
> **Owner** : Data eng (backup : DevOps lead)

---

## 1. Symptom

- Alert : `D14ObsPgPrimaryDown`
- Customer report : "Cannot save catalog version"
- gRPC `Register` / `Evolve` calls return `unavailable` errors
- `kubectl get pods -n efe-pg` shows primary in `Error` or `CrashLoopBackOff`

## 2. Why this matters (impact)

- All Postgres-backed catalog operations failing
- Tenant deployments blocked
- Customer SLA P0 : 4h resolution target
- Audit chain (D14-bis) starts queueing if > 30 min
- Compliance : SOC 2 A1.2 (availability) breach if > 4h/month

## 3. Diagnostics

```bash
# Check primary pod state
kubectl get pods -n efe-pg -l app=efe-pg,role=primary
kubectl describe pod -n efe-pg <primary-pod>
kubectl logs -n efe-pg <primary-pod> --tail=200

# Check PG operator state
kubectl get clusters -n efe-pg
kubectl describe cluster efe-pg-cluster -n efe-pg

# Check replica state
kubectl get pods -n efe-pg -l app=efe-pg,role=replica
```

## 4. Mitigation steps

### Quick mitigations (< 5 min)

1. Verify replica is healthy + synchronized
2. Trigger PG operator failover :
   ```bash
   kubectl exec -n efe-pg efe-pg-cluster-1 -- /usr/bin/pg_ctl promote
   ```
3. Update efe-server to point to new primary :
   ```bash
   kubectl rollout restart deployment efe-server -n efe-production
   ```

### Longer mitigations (15-30 min)

4. If no synchronized replica :
   - Restore from last full backup (D17)
   - Re-apply WAL replay
5. If quorum loss in HA :
   - Manual recovery per PG operator runbook
6. Fallback to memory backend temporarily :
   ```bash
   helm upgrade efe charts/efe \
       --set config.catalog.backend=memory --reuse-values
   ```

## 5. Resolution criteria

- PG primary `Available` for 5 min
- efe-server connection rate > 95 %
- gRPC `Register` success rate > 99 %
- 0 customer reports for 30 min

## 6. Post-incident actions

- [ ] Update PG operator runbook
- [ ] Add anti-regression pin test if root cause is code
- [ ] Verify backup integrity post-failover
- [ ] Customer notification of resolution

## 7. Related runbooks

- `d14_pg_pool_exhausted.md`
- `d17_backup_failed.md`
- `general_incident_playbook.md`
