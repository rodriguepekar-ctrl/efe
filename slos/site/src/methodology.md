# Methodology

Each SLO is defined by a closed-form **Service Level Indicator (SLI)** —
a deterministic ratio computed from raw Prometheus counters or
histograms.  Compliance windows are aligned to UTC ; aggregation
follows the Google SRE *ratio-of-sums* (for SLO-1/3) or *mean-of-bits*
(for SLO-2/4) rule.

## SLI formulas (frozen v1.4 § 22.2)

### SLO-1 — L2 availability

```
N = Σ efe_l2_request_total{status="success"}
D = Σ efe_l2_request_total{status ∈ {success, server_error, transport_error}}
SLI = N / D    (D ≥ 100, else "indeterminate")
```

Client errors (4xx) are **deliberately excluded** from D — an attacker
spamming 4xx must not be able to game the SLI (T96 mitigation).

### SLO-2 — L2 latency p95 (compliance)

For each 5-min window W :

```
p95(W) = histogram_quantile(0.95, sum by le (rate(efe_l2_request_duration_seconds_bucket[5m])))
C(W)   = 1 if p95(W) ≤ 50 ms and coverage(W) ≥ 0.80, else 0 or NaN
SLI    = mean(C) over windows where C is defined
```

### SLO-3 — Swarm recovery time

For each chaos-induced recovery event $e$ :

```
C(e) = 1 if duration(e) ≤ 60 s, else 0
SLI  = Σ C(e) / |events|      (events ≥ 10, else "indeterminate")
```

### SLO-4 — Replication lag p99

For each 1-min window W :

```
L_p(W) = histogram_quantile(0.99, rate(efe_replication_lag_bucket{participant=p}[1m]))
L(W)   = min over participants p of L_p(W)              -- AD16, Raft-aware
C(W)   = 1 if L(W) ≤ 100, else 0
SLI    = mean(C) over windows where C is defined
```

## Window taxonomy (frozen v1.4 § 22.3)

8 UTC-aligned tiers : 1m / 5m / 30m / 1h / 6h / 24h / 7d / 30d.

## Aggregation rule

| SLO | Aggregation |
|---|---|
| SLO-1 (availability) | sum N / sum D |
| SLO-2 (latency) | mean of bits |
| SLO-3 (recovery) | sum within / sum total |
| SLO-4 (lag) | mean of bits |

## Rollup pipeline (PR #22 § 9)

- **T0** — Prom recording rules (5m, 30m, 1h, 6h) → live SLI series
- **T1** — hourly daily-Parquet builder (CronJob `0 * * * *`)
- **T2** — monthly JSON aggregator (CronJob `0 3 1 * *`)
- **T3** — quarterly signed PDF + JSON (CronJob `0 4 1 1,4,7,10 *`)

## Audit chain

- Daily Parquet : 90-day retention on PVC / S3.
- Monthly JSON : 7-year retention, committed to git via auto-PR.
- Quarterly PDF + JSON : **forever** retention, cosign-signed,
  Rekor-witnessed.
