# RB_M30_REPLICATION_LAG — Postgres logical replication lag

**Severity** : P2 (failover preparedness degraded). P1 if lag > 5 min.
**Trigger** : `M30RegionHighReplicationLag` for ≥ 10 min.

## 1. Diagnose

On the **active** PG :
```sql
SELECT slot_name, active, pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)) AS lag_bytes,
       extract(epoch from now() - last_msg_send_time) AS send_lag_secs
  FROM pg_replication_slots
 WHERE slot_type='logical';
```

On the **passive** PG :
```sql
SELECT now() - pg_last_xact_replay_timestamp() AS lag,
       pg_is_in_recovery();
```

## 2. Common causes

### 2.1 Cross-region network saturation
- AWS publishes `NetworkOut` for the active instance — check the
  CloudWatch graph.
- Throttle non-critical traffic (S3 batch jobs, audit export
  sinks).

### 2.2 Long-running write transactions on active
- A bulk import or a missing index on the active makes WAL
  generation faster than replication can consume.
- Identify : `SELECT pid, state, query_start, query FROM pg_stat_activity
  WHERE state != 'idle' ORDER BY query_start LIMIT 10;`.

### 2.3 Replica disk full
- `df -h` on the replica node. Grow EBS via Terraform if < 10 % free.

### 2.4 Replication slot orphaned
- `pg_replication_slots.active = false` AND lag growing → slot is
  blocking WAL recycling without consuming. **Drop** the slot
  (this loses the replica permanently — re-bootstrap) :
  ```sql
  SELECT pg_drop_replication_slot('m30_main');
  ```

## 3. Customer impact

- Synchronous endpoints : **no impact** — the active still serves
  writes. Only the passive's catch-up is degraded.
- **Failover preparedness** : if a failover is required NOW, the
  passive will lose up to `lag_secs` of writes.

## 4. Mitigation
1. If lag < 5 min : let it catch up, observe.
2. If lag 5 - 30 min : page SRE, throttle batch traffic.
3. If lag > 30 min : page SRE + Compliance ; lag now violates the
   RPO SLA. Decide between :
   - Waiting (still safe — no failover required).
   - Re-bootstrapping the replica (cheap on a fresh AWS RDS read
     replica ; ~ 30 min for our DB size).

## 5. Post-incident
- Add Prometheus alert for the specific cause (e.g. long
  transactions ≥ 5 min).
- Audit any "slow" tenant queries hitting the active.
