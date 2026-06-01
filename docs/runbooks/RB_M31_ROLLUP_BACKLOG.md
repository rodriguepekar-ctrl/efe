# RB_M31_ROLLUP_BACKLOG — Hourly rollup CronJob behind

**Severity** : P3 if < 6 h ; P2 if > 6 h ; P1 if > 24 h.
**Trigger** : `M31RollupLagHigh` for ≥ 15 min.

## 1. Diagnose
```bash
kubectl -n efe-system get cronjob/efe-stack-stripe-rollup-hour
kubectl -n efe-system get jobs -l component=rollup-hour --sort-by=.metadata.creationTimestamp | tail
kubectl -n efe-system logs job/<last-job> --tail=200
```

## 2. Common causes

### 2.1 PG primary slow / OOM
- The rollup performs bulk `INSERT INTO usage_rollup_hour ... ON CONFLICT`. If PG
  is overloaded, the cron times out at the K8s backoff limit.
- Check `pg_stat_activity` for long-running queries.

### 2.2 Redis SCAN starving
- Too many `m31:counter:*` keys (tens of thousands) → SCAN is slow.
- This is **expected** at scale and is bounded — the cron walks the
  keyspace in chunks. If the entire walk takes > 30 min, scale up
  the Redis cluster.

### 2.3 NATS lag
- If `meter.bumped` events are queued, the durable replay path
  is delayed. Normally invisible to the rollup itself (which
  reads Redis, not NATS).

## 3. Customer impact

- `usage/now` keeps working (reads Redis live counters).
- `usage/daily` is delayed by the lag amount.
- Monthly invoices DEPEND on hourly rollups landing in PG ; if the
  lag is > 24 h close to month-end, invoice generation will be
  late. Page Compliance.

## 4. Manual catch-up
```bash
# Run hourly rollup synchronously
scripts/m31/rollup_runner.sh hour

# Then day rollup
scripts/m31/rollup_runner.sh day
```

## 5. Permanent mitigation
- Vertical-scale PG IOPS if writes are the bottleneck.
- Increase Redis replicas to spread the SCAN load.
- Partition `usage_rollup_hour` by `period_start` (deferred to
  M31.1).

## 6. Post-incident
- File P3 (or higher) ticket.
- Compare rollup-cron duration p99 over the last 30 days ; if it
  has been climbing, prepare for capacity-planning.
