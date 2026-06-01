# EFE Public SLOs

PR #22 — 4 frozen SLOs + multi-tier rollup compute pipeline +
signed quarterly evidence.

| | |
|---|---|
| Architecture | [`docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md`](../docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md) |
| Crate | `efe-slo-rollup` v0.1.0 (`slos/rollup/`) |
| Binary | `efe-slo-rollup` |
| Stability | STABILITY_CONTRACT v1.4 § 22 — frozen ≥ 6 chart-minor versions |
| Public surface | `https://efe.github.io/slos/` |

## The 4 SLOs

| ID | Target | Window |
|---|---|---|
| `L2_AVAILABILITY` | ≥ 99.5 % success rate | rolling 30 d |
| `L2_LATENCY_P95` | ≥ 99 % of 5-min windows ≤ 50 ms | rolling 30 d |
| `SWARM_RECOVERY_TIME` | ≥ 99 % of recoveries ≤ 60 s | rolling 30 d |
| `REPLICATION_LAG_P99` | ≥ 99 % of 1-min windows ≤ 100 | rolling 30 d |

## Quick start

```bash
# Compute the current hour's SLI for all 4 SLOs against a live Prom.
efe-slo-rollup compute \
    --chart-version 0.6.0 \
    --window 1h \
    --offset 1h \
    --prom-url http://prometheus.monitoring.svc:9090

# Build the monthly aggregate from the daily files.
efe-slo-rollup rollup --stage t2 --period 2026-05 --chart-version 0.6.0

# Build + sign the quarterly evidence PDF.
efe-slo-rollup rollup --stage t3 --period Q2-2026 \
    --chart-version 0.6.0 \
    --cosign-identity https://token.actions.githubusercontent.com

# Verify a signed PDF.
efe-slo-rollup evidence verify slos/v0.6.0/quarterly/Q2-2026.pdf

# Publish the public site (mdbook + JSON API + badges).
efe-slo-rollup publish --chart-version 0.6.0
```

## CI workflows

| Workflow | Trigger |
|---|---|
| [`slo-compute.yml`](../.github/workflows/slo-compute.yml) | cron `0 * * * *` (every hour, T1) |
| [`slo-monthly.yml`](../.github/workflows/slo-monthly.yml) | cron `0 3 1 * *` (1st of month, T2) |
| [`slo-quarterly.yml`](../.github/workflows/slo-quarterly.yml) | cron `0 4 1 1,4,7,10 *` (T3 + cosign) |
| [`slo-publish.yml`](../.github/workflows/slo-publish.yml) | on push:main + after the 3 above |

## Helm install

```bash
helm install efe-slos slos/rollup/helm \
    --namespace efe-slos --create-namespace \
    --set chartVersion=0.6.0 \
    --set cosign.identity=https://token.actions.githubusercontent.com
```

## Security

- **No PII** in SLI labels (T97 mitigation — `deny-pii` Cargo feature
  for compile-time guard).
- **No client-side JS** on the public page (T100).
- **Cosign keyless OIDC** signing chain via GitHub Actions (T101).
- **Namespace-scoped RBAC** (Role, not ClusterRole — T103/T104).
- **No `/JavaScript` in PDF** (T105).

## STABILITY_CONTRACT v1.4 § 22

Frozen for ≥ 6 chart-minor versions :
- 4 SLO IDs (closed taxonomy)
- 9 numerical thresholds
- 8-tier window taxonomy
- 3 storage file schemas
- 7 CLI subcommands
- Helm values surface
- Public page URLs
- Signing identity regex
