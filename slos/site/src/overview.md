# EFE Public SLOs — Overview

The EFE meta-swarm Phase-2 gRPC L2 surface commits to **4 Service Level
Objectives (SLOs)** — frozen, numerical, auditable, signed quarterly.

| SLO | Target | Window | Badge |
|---|---|---|---|
| L2 availability | ≥ 99.5 % | rolling 30 d | ![L2 availability](./badges/slo-1-availability.svg) |
| L2 latency p95 | ≥ 99 % of 5-min windows ≤ 50 ms | rolling 30 d | ![L2 latency p95](./badges/slo-2-latency-p95.svg) |
| Swarm recovery time | ≥ 99 % of recoveries ≤ 60 s | rolling 30 d | ![Swarm recovery](./badges/slo-3-recovery-time.svg) |
| Replication lag p99 | ≥ 99 % of 1-min windows ≤ 100 | rolling 30 d | ![Replication lag](./badges/slo-4-replication-lag.svg) |

## Where these numbers come from

Live numbers : [current.json](./current.json).  Per-month archive :
[history/](./history/).  Quarterly signed evidence : [quarterly/](./quarterly/).

Every artefact under `slos/v{chart}/` is produced by the `efe-slo-rollup`
binary on a cron schedule.  Quarterly PDFs are signed via cosign keyless
OIDC ; a verifier ships in the same binary :

```bash
efe-slo-rollup evidence verify slos/quarterly/Q2-2026.pdf
```

## Stability contract

The 4 SLO IDs, thresholds, windows, file schemas, and signing identity
regex are committed in
[STABILITY_CONTRACT v1.4](./stability-contract-v1.4.md), frozen for **at
least 6 chart-minor versions**.  Any change is a major-version bump.

See [methodology](./methodology.md) for the closed-form SLI math.
