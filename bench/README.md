# EFE Public Bench

PR #21 — reproducible cross-version benchmark + chaos suite for the
EFE Phase-2 meta-swarm gRPC L2 surface.

| | |
|---|---|
| Architecture | [`docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md`](../docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md) |
| Crate | `efe-bench-harness` v0.1.0 (`bench/harness/`) |
| Binary | `efe-bench-harness` |
| Stability | STABILITY_CONTRACT v1.3 § 9 — scenarios + tolerances + result schema frozen for ≥ 6 chart-minor versions |
| Baselines | [`bench_baselines/v0.6.0/L2/`](../bench_baselines/v0.6.0/L2/) (first commit) |

## What this is

This is the **publicly-auditable benchmark methodology** for the
EFE meta-swarm.  Anyone — operators, reviewers, partners, contributors,
auditors — can :

1. **Reproduce** our published numbers on their own kind / k3s
   / managed cluster.
2. **Compare** their numbers to ours using a published per-metric
   tolerance contract.
3. **Audit** the methodology + scenarios + chaos faults openly.
4. **Extend** the suite with new scenarios via PRs (the schema is
   versioned + stable).

This is the EFE equivalent of TPC for databases, MLPerf for ML, SPEC
for CPU.

## Quick start

```bash
# Build the harness.
cargo build --release -p efe-bench-harness

# Run the smoke scenario against a kind cluster.
./target/release/efe-bench-harness run \
    --scenario S01_baseline_100qps_5min \
    --provisioner kind

# Run the full suite.
./target/release/efe-bench-harness run --all --provisioner kind

# Validate scenarios YAML schema-wise.
./target/release/efe-bench-harness validate

# Compare a fresh result vs baseline.
./target/release/efe-bench-harness compare \
    --result bench/results/{run_id}/{scenario}/summary.json \
    --baseline bench_baselines/v0.6.0/L2/{scenario}.json
```

## Scenario catalogue (10)

| ID | Description | Duration | Tier |
|---|---|---|---|
| [`S01_baseline_100qps_5min`](scenarios/S01_baseline_100qps_5min.yaml) | 100 QPS sanity smoke | 5 min | quick |
| [`S02_canary_1000qps_15min`](scenarios/S02_canary_1000qps_15min.yaml) | Production-like canary | 15 min | perf |
| [`S03_ramp_to_breakdown`](scenarios/S03_ramp_to_breakdown.yaml) | 1→5000 QPS capacity ceiling | 20 min | capacity |
| [`S04_chaos_participant_loss`](scenarios/S04_chaos_participant_loss.yaml) | Kill participant @ 3 min | 10 min | chaos |
| [`S05_chaos_network_partition`](scenarios/S05_chaos_network_partition.yaml) | NetEm partition @ 5 min | 15 min | chaos |
| [`S06_chaos_cert_expiry`](scenarios/S06_chaos_cert_expiry.yaml) | TLS cert rotation mid-run | 10 min | chaos |
| [`S07_chaos_reload_storm`](scenarios/S07_chaos_reload_storm.yaml) | SIGHUP every 30 s for 15 min | 15 min | chaos |
| [`S08_multi_region_replay`](scenarios/S08_multi_region_replay.yaml) | 3-region weighted | 20 min | geo |
| [`S09_soak_2h_stability`](scenarios/S09_soak_2h_stability.yaml) | 2-hour soak | 2 h | soak |
| [`S10_slo_validation_30min`](scenarios/S10_slo_validation_30min.yaml) | All 4 STABILITY v1.1 SLOs | 30 min | compliance |

## Chaos faults (6)

Each fault is a [chaos-mesh](https://chaos-mesh.org/) CRD applied via
`kubectl apply -f` (AD20).  Live under
[`deploy/chaos/pr21_meta_swarm/`](../deploy/chaos/pr21_meta_swarm/) :

  - `participant-loss.yaml` — PodChaos pod-kill
  - `network-partition.yaml` — NetworkChaos partition
  - `cert-expiry.yaml` — cert-manager rotation (placeholder, Phase-3)
  - `reload-storm.yaml` — SIGHUP loop (placeholder, Phase-3)
  - `cpu-throttle.yaml` — StressChaos cpu
  - `memory-pressure.yaml` — StressChaos memory

## Comparison metrics (closed taxonomy — STABILITY_CONTRACT v1.3 § 21.3)

| Metric | Direction | Default fail-pct |
|---|---|---|
| `p50_ms` | lower-is-better | +15 % |
| `p95_ms` | lower-is-better | +25 % |
| `p99_ms` | lower-is-better | +35 % |
| `error_rate` | lower-is-better | +1 pp |
| `throughput_qps` | higher-is-better | -15 % |
| `recovery_time_s` | lower-is-better | +15 s |

Each scenario can override per-metric tolerance in its YAML `tolerance:` block.

## CI workflows

| Workflow | Trigger | Scope | Cap |
|---|---|---|---|
| [`bench-quick.yml`](../.github/workflows/bench-quick.yml) | PR label `bench` | S01 + S04 + S10 | 1 h |
| [`bench-full.yml`](../.github/workflows/bench-full.yml) | Nightly cron 03:00 UTC | All 10 | 6 h |
| [`bench-release.yml`](../.github/workflows/bench-release.yml) | Tag push `v*.*.*` | All 10 + auto-regen baseline | 7 h |
| [`bench-compare.yml`](../.github/workflows/bench-compare.yml) | PR comment `/bench-compare <sha>` | Diff | 30 min |

## Result format

Each scenario produces three artefacts in
`bench/results/{run_id}/{scenario_id}/` :

  - `summary.json` — canonical metrics ([schema](schemas/baseline-v1.json))
  - `result.parquet` (or ndjson stub) — per-request raw data
  - `prom-snapshot.tar.gz` (optional)

## Public Pages site

Published at `https://efe.github.io/bench/` :

  - 10 SVG badges (one per scenario, refreshed nightly)
  - Historical trend graphs per chart minor
  - Quarterly compliance evidence files (S10)
  - Per-scenario methodology pages

## Reproducing our numbers

1. **kind on Linux** : exactly the same kind image, kubectl version.
2. **Same `efe-meta-swarm` chart version** (e.g. `v0.6.0`).
3. **Same `efe-meta-swarm-loadgen` build** (consumed as a library — pinned by `Cargo.lock`).
4. **Same seed** (set in scenario YAML, e.g. `run.seed: 42`).

Differences in node class (CPU model, NIC speed) will produce
different absolute numbers but relative regression detection
remains valid.  Cross-architecture reproducibility is a Phase-3
follow-up (Appendix G.6).

## Security

Bench runs **NEVER** hit production :
  - `loadgen.run.mode=production` is hard-rejected (AD11, T86).
  - Chaos faults scoped to `efe-bench-*` namespaces only (AD21, T93).
  - kubeconfig never logged (AD31, T90).
  - Scenario YAMLs reject remote `!Include` / URL fetch (AD32, T91).

## License + governance

UNLICENSED — internal use only at this stage.  Phase-3 :
contributor agreement + public benchmark-as-a-service offering
(Appendix G.7).

## Phase-3 follow-ups

Appendix G of the architecture doc lists 10 follow-ups :
EKS / GKE auto-provision (G.1), ML anomaly detection (G.2),
cross-version baseline interpolation (G.3), bench-as-a-service (G.7),
cost attribution (G.8), customer-facing landing (G.9), soak Prom
rolling retention (G.10).
