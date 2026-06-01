# EFE Smoke — 9-Check Merge Gate

PR #23 — 9 closed-taxonomy smoke checks (S1–S9) that hard-block PR merge.

| | |
|---|---|
| Architecture | [`docs/EFE_A1_PHASE2_PR23_ARCHITECTURE.md`](../docs/EFE_A1_PHASE2_PR23_ARCHITECTURE.md) |
| Crate | `efe-smoke` v0.1.0 (`smoke/runner/`) |
| Binary | `efe-smoke` |
| Stability | STABILITY_CONTRACT v1.5 § 23 — frozen ≥ 6 chart-minor versions |
| Wall-clock | ≤ 4 min total (incl. ~ 60 s cluster boot) |
| Badge | `https://efe.github.io/smoke/badge.svg` (refreshed on push:main) |

## The 9 checks

| ID | Check | Budget | Validates |
|---|---|---|---|
| S1 | boot | 30 s | controller `/healthz` + 3 participants Ready |
| S2 | L1 SwarmService | 15 s | 5 canonical L1 RPCs, p99 < 500 ms |
| S3 | L2 meta-swarm | 20 s | controller↔participant chain, p99 < 1 s |
| S4 | mTLS rotation | 25 s | 0 dropped requests during cert switchover |
| S5 | SIGHUP reload | 20 s | manifest version increments, 0 in-flight drop |
| S6 | Prom metrics | 10 s | 14 STABLE families exposed (PR #17 contract) |
| S7 | OTel propagation | 15 s | trace chain has ≥ 2 spans, both expected services |
| S8 | restart resilience | 30 s | pod restart < 30 s, no quorum loss, error_rate < 5% |
| S9 | graceful shutdown | 15 s | SIGTERM exits 0 within 10 s, all in-flight gracefully completed |

## Quick start

```bash
# Local smoke (kind on laptop).
cargo build --release -p efe-smoke
./target/release/efe-smoke run \
    --chart-version 0.6.0 \
    --provisioner kind

# Mock path (no cluster, no network — used by unit tests).
./target/release/efe-smoke run --chart-version 0.6.0 --mock

# Restrict to a subset of checks.
./target/release/efe-smoke run --chart-version 0.6.0 --mock --check S1 --check S6

# Read a result, apply gate decision.
./target/release/efe-smoke gate --input smoke.json

# Validate a result against the v1 schema.
./target/release/efe-smoke validate smoke.json

# List the 9 frozen check IDs.
./target/release/efe-smoke list --format json
```

## CI workflows

| Workflow | Trigger | Gate |
|---|---|---|
| [`smoke-pr.yml`](../.github/workflows/smoke-pr.yml) | every PR + push:main | **hard merge gate** |
| [`smoke-nightly.yml`](../.github/workflows/smoke-nightly.yml) | cron `0 2 * * *` UTC | advisory (multi-node scale) |

## Security model

| Threat | Mitigation |
|---|---|
| T106 — flake-storm | AD4 (zero-tolerance) + AD17 (1 retry on early connection-refused) + runbook |
| T107 — credential leak | `--redact-logs` flag |
| T108 — production mis-target | `assert_not_production` checks `efe.io/environment` label |
| T109 — baseline drift | PR #22 SLO rollup catches via `source=synthetic_smoke` |
| T110 — `--allow-skip` abuse | AD33 — `--reason` required ; logged in GitHub audit |
| T111 — unbounded DoS | `concurrency.cancel-in-progress` + 8 min `timeout-minutes` |
| T112 — secrets in git | AD55 — generated in-cluster |
| T113 — prod network egress | AD12 — whitelisted to in-namespace + kind node only |
| T114 — shared PVC clobber | AD11 — `efe.io/auto-reap=true` namespace label |
| T115 — timing oracle | AD56 — jittered request times in S4 |

## STABILITY_CONTRACT v1.5 § 23

Frozen ≥ 6 chart-minor versions :
- 9 check IDs (closed taxonomy — AD1)
- 9 per-check wall-clock budgets
- 14 STABLE Prom metric families
- 5 CLI subcommands (`run | list | validate | gate | version`)
- Smoke result JSON schema v1
- `ClusterProvisioner` trait (shared with PR #21 bench)
- 6 ExitCode taxonomy

## Phase-3 follow-ups (Appendix G)

G.1 cross-region · G.2 Phase-1 binary compat · G.3 smoke-driven canary
· G.4 signed evidence · G.5 GPU smoke · G.6 chaos-aware smoke
· G.7 multi-arch · G.8 scale > 3 participants · G.9 IDE pre-commit
· G.10 `helm test` integration
