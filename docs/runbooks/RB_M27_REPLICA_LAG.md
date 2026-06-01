# RB_M27_REPLICA_LAG — PG read replica lag

**Severity** : P1 (lag > 30s).
**Owner** : SRE on-call.
**Trigger** : `M27PgReplicaLag` (`efe_pg_replica_lag_seconds > 30 for 5min`).

## 1. Confirm

```bash
kubectl exec -ti <pg-replica-pod> -- psql -U efe -c \
  "SELECT now() - pg_last_xact_replay_timestamp() AS lag,
          pg_is_in_recovery() AS is_replica;"

# On primary :
kubectl exec -ti <pg-primary-pod> -- psql -U efe -c \
  "SELECT client_addr, state, sent_lsn, write_lsn, flush_lsn, replay_lsn,
          pg_wal_lsn_diff(sent_lsn, replay_lsn) AS bytes_lag
   FROM pg_stat_replication;"
```

## 2. Identify root cause

### 2.1 Long write transaction on primary
```sql
SELECT pid, now() - xact_start AS xact_dur, state, query
FROM pg_stat_activity
WHERE xact_start < now() - interval '30 seconds'
ORDER BY xact_dur DESC LIMIT 10;
```

If you spot a runaway `INSERT ... SELECT` on a huge table (M24 active
learning batch or audit chain reorganization) — that's it.

### 2.2 Replica CPU / IO bottleneck
```bash
kubectl top pod <pg-replica-pod>
# If CPU > 90% or IO wait high : replica can't keep up.
```

### 2.3 Network partition primary ↔ replica
```bash
kubectl exec -ti <pg-replica-pod> -- ping -c 5 <pg-primary-pod>
```

## 3. Mitigate

### 3.1 Activate RO → RW fallback (most common response)

```bash
# Tell every efe-server pod to route ro_pool() to rw pool :
kubectl -n efe-system set env deployment/efe-server EFE_FALLBACK_RO_TO_RW=true
```

`DbContext::set_fallback_to_rw(true)` picks this up within 30s.

**Impact** : every read goes to primary. Primary load rises ~30%.
Acceptable for < 30 min ; revert as soon as replica catches up.

### 3.2 Cancel runaway transaction
```sql
SELECT pg_cancel_backend(<pid>);
```

### 3.3 Block low-priority writes temporarily
```bash
# Pause M24 active learning batch jobs :
kubectl -n efe-system scale deployment efe-active-learning --replicas=0
# Confirm primary write rate dropped :
kubectl exec -ti <pg-primary> -- psql -c \
  "SELECT sum(tup_inserted + tup_updated) / 60 AS rate
   FROM pg_stat_database WHERE datname='efe';"
```

## 4. Revert

When `efe_pg_replica_lag_seconds < 5` for 5 min consecutively :

```bash
kubectl -n efe-system set env deployment/efe-server EFE_FALLBACK_RO_TO_RW=false
# Resume any paused background jobs.
kubectl -n efe-system scale deployment efe-active-learning --replicas=1
```

## 5. Long-term

- If lag recurring → add more replica replicas (read scale-out).
- If primary CPU is bottleneck → upgrade instance class.
- Move heavy analytical queries to a separate data warehouse with
  CDC ingestion (M28).
- Tune `wal_keep_size` if replica falls so far behind that it can't
  catch up.

## 6. Post-incident

- Identify the cause (runaway tx vs steady write load vs replica
  hardware).
- Adjust autoscaling / scheduling.
- Update `BENCHMARKS.md` if lag profile changes during typical
  production load.
