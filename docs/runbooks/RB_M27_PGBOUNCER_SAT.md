# RB_M27_PGBOUNCER_SAT — PgBouncer pool saturation

**Severity** : P1.
**Owner** : SRE on-call.
**Trigger** : `M27PgBouncerPoolSaturated` (pool > 90% utilized for 5min).

## 1. Confirm

```bash
kubectl -n efe-edge exec -ti $(kubectl -n efe-edge get pod \
  -l app.kubernetes.io/name=efe-pgbouncer -o name | head -1) -- \
  psql -h 127.0.0.1 -p 5432 -U pgbouncer_stats -d pgbouncer -c "SHOW POOLS;"
```

Look for : `cl_waiting` > 0, `sv_active` ≈ `pool_size`.

## 2. Identify root cause

### 2.1 Slow queries on PG primary

```bash
kubectl exec -ti $(kubectl get pod -l app=pg-primary -o name | head -1) -- \
  psql -U efe -c "SELECT pid, now() - query_start AS dur, state, query
                  FROM pg_stat_activity
                  WHERE state = 'active'
                    AND now() - query_start > interval '5s'
                  ORDER BY dur DESC LIMIT 10;"
```

If long-running query is the culprit :

```sql
-- Identify the query, then :
SELECT pg_cancel_backend(<pid>);
-- OR more aggressive :
SELECT pg_terminate_backend(<pid>);
```

### 2.2 Connection storm from app

```bash
kubectl -n efe-system get pods -l app=efe-server | wc -l
# If recently scaled up, every pod opens new conns up to its max.
```

### 2.3 PgBouncer config too tight

```bash
# Current values :
kubectl -n efe-edge get cm efe-edge-pgbouncer -o yaml | grep pool_size
```

## 3. Mitigate

### 3.1 Temporarily raise pool_size

```bash
# Edit umbrella values :
# charts/efe-stack/charts/efe-edge/values.yaml :
#   pgbouncer.config.poolSize: 100  (was 50)
helm upgrade efe-stack ./charts/efe-stack --reuse-values \
  --set efe-edge.pgbouncer.config.poolSize=100
kubectl -n efe-edge rollout restart deployment efe-edge-pgbouncer
```

⚠️ This consumes more server-side connections on PG. Watch
`pg_stat_activity` count.

### 3.2 Reduce client-side concurrency

```bash
# Scale efe-server down :
kubectl -n efe-system scale deployment efe-server --replicas=5
# Drop backpressure max_inflight :
kubectl -n efe-system set env deployment/efe-server \
  EFE_BACKPRESSURE_MAX_INFLIGHT=500
```

### 3.3 Activate RO fallback

If the saturation is on the RO pool (replica), and replicas are healthy,
no action needed. If RO pool saturation comes from portal-admin
analytical queries blowing up :

```bash
kubectl -n efe-system set env deployment/portal-admin \
  EFE_DATABASE_RO_URL=postgres://...:5432/efe_ro?application_name=portal-admin-prio
# Use a dedicated PgBouncer pool with higher pool_size.
```

## 4. Long-term

- Add an index for the slow query.
- Split heavy analytical queries to a read-only data warehouse (BigQuery
  / Snowflake) populated via CDC (M28).
- Optimize the `tenant_daily_usage` aggregator schedule (currently
  daily ; bump to hourly if PG load allows).

## 5. Post-incident

- Update `docs/runbooks/runbook.md` with the specific slow query
  pattern.
- File ADR if pool_size needs a permanent increase.
