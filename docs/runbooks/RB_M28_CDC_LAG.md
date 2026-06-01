# RB_M28_CDC_LAG — PG logical replication slot lag

**Severity** : P1 (> 1 GB lag) | P0 (lag will exhaust WAL disk).

## 1. Confirm
```sql
SELECT slot_name, active, restart_lsn, confirmed_flush_lsn,
       pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)) AS lag_bytes
  FROM pg_replication_slots WHERE slot_name = 'efe_cdc_publisher';
```

## 2. Identify cause
- `active = false` → publisher pod crashed. Check :
  ```bash
  kubectl -n efe-stack get pods -l app=efe-cdc-publisher
  ```
- `active = true` but lag growing : publisher can't keep up. Check
  CPU / NATS throughput :
  ```bash
  kubectl -n efe-stack top pod -l app=efe-cdc-publisher
  ```

## 3. Mitigate

### 3.1 Publisher crashed — restart
```bash
kubectl -n efe-stack rollout restart deployment efe-cdc-publisher
```

### 3.2 Slot abandoned (no consumer in > 1h)
```sql
-- DANGEROUS : drops the slot ; events between drop and recreate are lost.
SELECT pg_drop_replication_slot('efe_cdc_publisher');
SELECT pg_create_logical_replication_slot('efe_cdc_publisher', 'pgoutput');
```
Then restart the publisher. This is the **last resort** — only do
this if WAL is about to fill the disk (use `df -h` on the PG host).

### 3.3 Slow consumer — scale CDC publisher
```bash
# CDC publisher is single-leader by design ; scaling means upgrading
# its CPU / instance class :
kubectl -n efe-stack patch deployment efe-cdc-publisher \
  --patch '{"spec":{"template":{"spec":{"containers":[{"name":"cdc","resources":{"limits":{"cpu":"2"}}}]}}}}'
```

## 4. Post-incident
- Run a smoke test : insert a row in `agent_audit`, verify the event
  appears in NATS within 10s.
- If recurring, evaluate moving heavy classify_call_log writes to a
  separate replication slot.
