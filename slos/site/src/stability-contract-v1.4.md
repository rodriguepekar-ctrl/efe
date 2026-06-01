# STABILITY_CONTRACT v1.4 — SLO surface (PR #22)

Frozen for **at least 6 chart-minor versions**.  Any change requires a
new architecture PR + major-version bump + migration plan.

## 22.1 — SLO IDs (closed taxonomy)

```
L2_AVAILABILITY
L2_LATENCY_P95
SWARM_RECOVERY_TIME
REPLICATION_LAG_P99
```

## 22.2 — Numerical thresholds

| Constant | Value |
|---|---|
| `L2_AVAILABILITY_TARGET` | 0.995 |
| `L2_LATENCY_P95_TARGET_S` | 0.050 |
| `L2_LATENCY_P95_COMPLIANCE` | 0.99 |
| `SWARM_RECOVERY_TIME_TARGET_S` | 60.0 |
| `SWARM_RECOVERY_TIME_COMPLIANCE` | 0.99 |
| `REPLICATION_LAG_TARGET` | 100 |
| `REPLICATION_LAG_COMPLIANCE` | 0.99 |
| `BURN_RATE_FAST` | 14.4 |
| `BURN_RATE_SLOW` | 6.0 |

## 22.3 — Window taxonomy (8 tiers)

`1m / 5m / 30m / 1h / 6h / 24h / 7d / 30d` (UTC-aligned).

## 22.4 — Storage schemas

- `slos/schemas/daily-parquet-v1.json`
- `slos/schemas/monthly-json-v1.json`
- `slos/schemas/quarterly-evidence-v1.json`

## 22.5 — CLI subcommand surface

`compute | rollup | publish | evidence | list | validate | version`

## 22.6 — Helm values keys

`chartVersion`, `region`, `storage.*`, `cronjobs.*`, `cosign.identity`,
`prometheus.url`

## 22.7 — Public page URLs

```
/current.json
/history/{yyyy-mm}.json
/quarterly/{Q1..4}-{yyyy}.{json,pdf,pdf.bundle}
/badges/slo-{1..4}-{slug}.svg
```

## 22.8 — Signing identity regex

```
https://github\\.com/efe-ai/efe/\\.github/workflows/slo-quarterly\\.yml@refs/.+
```

(used by the cosign verifier — frozen).
