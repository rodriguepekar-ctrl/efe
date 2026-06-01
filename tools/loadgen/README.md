# `tools/loadgen` — efe-meta-swarm-loadgen

PR #20 — production-grade L2 meta-swarm load generator.

## At a glance

| | |
|---|---|
| Binary | `efe-meta-swarm-loadgen` |
| Crate | `efe-meta-swarm-loadgen` v0.1.0 (workspace member at `tools/loadgen/`) |
| Helm chart | `charts/efe-meta-swarm-loadgen/` v0.1.0 (standalone, not a sub-chart) |
| Docker image | `ghcr.io/efe-ai/efe-meta-swarm-loadgen:0.1.0` (distroless multi-arch) |
| Architecture | [`docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md) |
| Stability | STABILITY_CONTRACT v1.2 § 10 — 5 STABLE Prometheus self-metrics + schema v1 |

## Why this exists

PR #11 → #19 shipped the meta-swarm controller + participant stack and
its operator surface (dashboards, alerts, SLOs).  What was missing : a
**deterministic, regression-safe, mTLS-capable, OTel-traceable load
generator** for the L2 gRPC surface.  PR #20 closes that gap.

Distinct from k6 (`deploy/load-tests/k6/`) which covers L1
SwarmService — PR #20 targets the L2 meta-swarm gRPC directly.

## Quick start

### Locally (against a dev cluster)

```bash
cargo run -p efe-meta-swarm-loadgen -- run \
    --target grpc://localhost:50051 \
    --qps 50 \
    --duration 1m \
    --mode dev \
    --output-format ndjson
```

### From a YAML profile

```bash
cargo run -p efe-meta-swarm-loadgen -- run \
    --profile tools/loadgen/examples/smoke-50qps-1min.yaml
```

### In-cluster via Helm

```bash
helm install loadgen-smoke charts/efe-meta-swarm-loadgen \
    --namespace efe \
    --set enabled=true \
    --set image.tag=0.1.0 \
    --set-file profile.inline=tools/loadgen/examples/smoke-50qps-1min.yaml
```

### Dry-run config validation

```bash
cargo run -p efe-meta-swarm-loadgen -- run \
    --target grpc://localhost:50051 \
    --qps 50 \
    --duration 1m \
    --dry-run
```

## Subcommands

| Subcommand | Purpose |
|---|---|
| `run` (default) | Execute a load test |
| `validate`      | Profile-YAML schema check, no traffic |
| `replay`        | Replay a recorded session against a new target |
| `aggregate`     | Aggregate multi-pod result files |
| `compare`       | Diff two result files |
| `version`       | Print version, git_sha, rustc_version, build_date |

## Capabilities

| Surface | Choices |
|---|---|
| Traffic patterns | constant · ramp · burst · diurnal · replay |
| Workload shapes  | iterate · step · reset · state · mixed |
| Rate controllers | open · closed · hybrid (default) |
| Termination      | duration · requests · until-slo · convergence · signal |
| Output formats   | terminal · json · csv · ndjson · parquet (stub) |
| Modes            | dev · staging · production (requires `--confirm-prod`) |

## Safety guards (T76-T85)

- `--qps` capped at 1 000 by default ; `--i-am-sure` unlocks up to
  100 000 ; hard cap regardless.
- `--mode=production` refuses to start without `--confirm-prod`.
- `--tls-insecure` refused in production.
- `--allow-mutation` refused in production.
- mTLS triplet `(cert, key, ca)` enforced atomically.
- Pod runs as nonroot, read-only rootfs, drop ALL capabilities.
- NetworkPolicy egress restricted to controllers + collector +
  participants.

## SLO regression usage

Validate the 4 STABILITY_CONTRACT v1.1 SLO targets in CI :

```bash
efe-meta-swarm-loadgen run \
    --profile tools/loadgen/examples/slo-validation-5min.yaml \
    --output-file /tmp/slo-result.json
# Exit code 0 = pass, 1 = SLO breach.
```

The nightly CI workflow `pr20-loadgen-slo-regression.yml` parses the
JSON output + posts a PR comment.

## Architecture references

- [`docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md)
  — full architecture (40 ADs, 32 ACs, 5 patterns, 4 workloads).
- [`docs/STABILITY_CONTRACT.md`](../../docs/STABILITY_CONTRACT.md) § 10
  — v1.2 commitments (loadgen self-metrics, profile schema, exit
  codes).
- [`docs/slo.md`](../../docs/slo.md) § 8 — 4 SLO targets loadgen
  validates.

## Operator runbook

For an SLO breach during a loadgen run, follow
[`RB_PR19_MSW1`](../../docs/runbooks/RB_PR19_MSW1.md) — loadgen is the
*producer* of the signal that wakes the operator ; the runbook covers
the *response*.

## Development

```bash
# Run all tests
cargo test -p efe-meta-swarm-loadgen --all-features

# Run lints
cargo clippy -p efe-meta-swarm-loadgen --all-targets -- -D warnings

# Regenerate CLI golden snapshot after intentional flag changes
cargo run -p efe-meta-swarm-loadgen -- --help > tools/loadgen/tests/data/golden-cli.txt

# Build the Docker image
docker buildx build \
    --platform linux/amd64,linux/arm64 \
    --file tools/loadgen/Dockerfile \
    --tag efe-meta-swarm-loadgen:dev \
    --load .
```

## Phase-3 follow-ups (architecture § 40 Appendix G)

- Adaptive ML-driven load curves (PR #25)
- Production-tap recording producer
- Soak-test mode (multi-day with cert rotation)
- Per-tenant load profiles (depends PR #25 multi-tenancy)
- Real Apache Parquet output via `arrow` crate
- ratatui live TUI (currently text-mode `terminal` formatter)
- Built-in chaos-mesh integration
- Cost attribution `$/request` panel
- SDK integration (TypeScript + Go)
- GraphQL control endpoint (start/pause/resume mid-run)
