# Runbook : d14_pg_pool_exhausted

> **Severity** : P1 (degraded — connection pool exhausted)
> **ETA to resolution** : 15 min
> **Sprint origin** : D14
> **Last updated** : 2026-04-30
> **Owner** : Data eng (backup : DevOps lead)

---

## 1. Symptom

- Alert : `D14ObsPgPoolExhausted`
- gRPC errors : `connection pool exhausted`
- Latency spike on Register / Evolve RPCs

## 2. Why this matters

Burst traffic > pool size queues operations ; if sustained, customer-visible degradation. P1 SLA 4h resolution.

## 3. Diagnostics

```bash
# Pool state via efe-server admin
kubectl exec -n efe-production efe-server-0 -- /efe-admin db-status

# Postgres-side connection count
kubectl exec -n efe-pg efe-pg-0 -- psql -c "SELECT count(*) FROM pg_stat_activity"

# Slow queries
kubectl exec -n efe-pg efe-pg-0 -- psql -c "SELECT * FROM pg_stat_statements ORDER BY mean_time DESC LIMIT 10"
```

## 4. Mitigation

### Quick (< 5 min)
1. Bump pool max temporarily :
   ```bash
   helm upgrade efe charts/efe \
       --set config.catalog.postgres.poolMaxConnections=16 --reuse-values
   ```
2. Restart slow queries (if any blocking)

### Longer (15-30 min)
3. Investigate slow queries via `pg_stat_statements`
4. Add Postgres replicas if needed
5. Optimize queries (add indexes if missing)

## 5. Resolution criteria

- `D14ObsPgPoolExhausted` cleared
- `pg_stat_activity` < 80 % of `max_connections`
- gRPC error rate < 1 %

## 6. Post-incident actions

- [ ] Long-term : tune pool size if pattern recurs
- [ ] Add PG metric to long-term retention
- [ ] Update D14 architecture doc if pattern

## 7. Related runbooks

- `d14_postgres_failover.md`
- `d18_replicas_below_minimum.md`
- `general_incident_playbook.md`
