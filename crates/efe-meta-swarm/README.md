# efe-meta-swarm

Hierarchical multi-swarm coordination for EFE — Phase 1 scaffold.

## Status

This is the **Phase 1 in-process scaffold** of the meta-swarm extension (cf.
`EFE_A1_PHASE1_ARCHITECTURE.md`). The crate compiles, runs examples, and
passes its tests in isolation. It does **not** yet link into `efe-server`
production binaries — that is Phase 2.

## Quick start

```bash
# Run all tests
cargo test -p efe-meta-swarm

# Run the simplest example
cargo run --example two_swarms_uniform -p efe-meta-swarm

# Run the headline Kovant demo
cargo run --release --example three_swarms_kovant -p efe-meta-swarm

# Bench it
cargo bench --bench three_swarms_kovant_latency -p efe-meta-swarm
```

## User-facing activation model

Phase 1 activation is purely opt-in via `cargo` invocations.  No
`efe-server` runtime is affected by the presence of this crate ;  the
production binary remains byte-identical to the pre-A1 release.

Future phases progressively expose the meta-swarm :

| Phase | Activation surface                  | Audience    |
|-------|-------------------------------------|-------------|
| 1     | `cargo run --example …`             | EFE team    |
| 2     | `EFE_META_SWARM_ENABLED` env var    | EFE team    |
| 3     | `AdminService.SetTenantFeature(…)`  | Kovant alpha|
| 4     | SDK opt-in module + portal toggle   | Public beta |
| 5     | Tarification + GA                   | All clients |

## Bench baseline tracking (PR #4)

Since Sprint A1 P1 PR #4, the bench `three_swarms_kovant_latency` is
wired into the workspace master gate via 7 opposable SLOs
(`SLO-A1-META-*`) frozen at
[`bench_baselines/v0.10.0/meta_swarm_slos.yaml`](../../bench_baselines/v0.10.0/meta_swarm_slos.yaml).

Each run of `bash scripts/ops/run_suite.sh a1_meta_swarm` produces a
sidecar JSON (`target/criterion/three_swarms_kovant_latency/meta-swarm-extras.json`)
that the master-gate consolidator
(`scripts/ops/consolidate_all_slos.py`) ingests to classify the 7
SLOs as PASS / FAIL / SKIP-DT.

See [`docs/EFE_A1_BENCH_TRACKING.md`](../../docs/EFE_A1_BENCH_TRACKING.md)
for the full SLO table, freeze procedure, and investigation guide.

## License

See workspace root LICENSE.  This crate falls under the EFE Apache-2.0
clients carve-out once published.
