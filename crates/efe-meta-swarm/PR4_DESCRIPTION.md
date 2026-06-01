# [Sprint A1 P1 — PR #4] feat(efe-meta-swarm): bench baseline tracking + master-gate hook

## Summary

Wires the `efe-meta-swarm` bench `three_swarms_kovant_latency` into
the workspace bench infrastructure (`scripts/benches.list`,
`bench.yml`) AND the Sprint Φ-D-OPS master-gate
(`.github/workflows/ops-master-gate.yml`,
`scripts/ops/consolidate_all_slos.py`,
`scripts/ops/master_gate_check.py`).

Defines **7 opposable SLOs** for the hierarchical multi-swarm
coordination layer, frozen at the v0.10.0 baseline, soft-launched
(`continue-on-error: true`) for 3 main runs.

The PR closes line 93 of `PR_DESCRIPTION.md` :

> 3. `feat(efe-meta-swarm): bench baseline tracking + master-gate hook` (week 8).

## What this PR adds

| Area                  | Detail                                                                                |
|-----------------------|---------------------------------------------------------------------------------------|
| Bench refactor        | `crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs` (46 → 415 LOC)         |
| Sidecar JSON          | `meta-swarm-extras.json` written by the bench at `criterion_main!` exit               |
| Bench measures        | 5 Criterion benchmarks (full_iterate, step_once, reset, build, consensus_norm) +      |
|                       | 1 determinism check + 4 algorithmic/ops sidecar SLOs                                  |
| Baseline YAML         | `bench_baselines/v0.10.0/meta_swarm_slos.yaml` (7 SLOs, dual encoding)                |
| Catalog               | `scripts/benches.list` +1 entry, `scripts/check_benches_list.sh` `KNOWN_CRATES` +1    |
| Workflow `bench.yml`  | new job `bench-meta-swarm-a1` (sibling) + `bench-aggregate` discovers sidecar         |
| Master-gate workflow  | new step "A1 meta-swarm master gate" + `consolidate` `--baselines` += A1 file         |
| Suite runner          | `scripts/ops/run_suite.sh` += case `a1_meta_swarm` (bench + sidecar harvest)          |
| Tests                 | `crates/efe-meta-swarm/tests/bench_smoke.rs` (7 tests, 1 s)                           |
| Documentation         | `docs/EFE_A1_BENCH_TRACKING.md` (250 lines : SLO table, freeze, runbook, FAQ)         |
| PR description        | this file                                                                             |
| Cargo.toml            | `[[bench]] name = "…"` normalised (1 space, was alignment-padded) — for catalog lint  |

## The 7 SLOs

| SLO id                              | Type        | Operator | Threshold | Source              |
|-------------------------------------|-------------|---------:|----------:|---------------------|
| `SLO-A1-META-LATENCY-P99`           | latency     |     `<=` |    100 ms | Criterion           |
| `SLO-A1-META-LATENCY-P50`           | latency     |     `<=` |     30 ms | Criterion           |
| `SLO-A1-META-CONVERGENCE-ITERS`     | algorithmic |     `<=` |       60  | sidecar             |
| `SLO-A1-META-RESIDUAL-FINAL`        | algorithmic |     `<=` |    1e-3   | sidecar             |
| `SLO-A1-META-DETERMINISM`           | algorithmic |     `==` |      `1`  | sidecar (3 reruns)  |
| `SLO-A1-META-NON-REGRESSION`        | operational |     `<=` |   10 %    | sidecar (vs frozen) |
| `SLO-A1-META-MEMORY-PEAK-MB`        | operational |     `<=` |     64 MB | sidecar (Linux only)|

Calibration rationale documented inline in
[`bench_baselines/v0.10.0/meta_swarm_slos.yaml`][1] and in
[`docs/EFE_A1_BENCH_TRACKING.md`][2] § 1.

## Test plan

End-to-end pipeline verified locally :

- [x] `cargo test -p efe-meta-swarm --offline --test bench_smoke` :
      **7 / 7 pass** in 1.03 s.
- [x] `cargo test -p efe-meta-swarm --offline` : no regression vs PR #3
      (still 192 / 192 default, 202 / 202 with `attention-weighted`).
- [x] `cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency
      --offline -- --quick` :
      bench compiles + runs + emits `meta-swarm-extras.json` with
      correct schema.
- [x] `bash scripts/check_benches_list.sh` :
      catalog valid, 11 entries (10 + A1).
- [x] `bash scripts/ops/run_suite.sh a1_meta_swarm` :
      produces `target/ops/measures.a1_meta_swarm.json` with 6 SLO
      keys (memory skipped on macOS, ticket DT-A1-META-MEMORY-MACOS
      via the deferred_ticket mechanism).
- [x] `python3 scripts/ops/consolidate_all_slos.py --baselines …
      --measures … --master …` :
      surface SLOs found = 7, 6 PASS + 1 SKIP-DT, master_report.json
      well-formed.
- [x] `python3 scripts/ops/master_gate_check.py --report …
      --tolerate-skip-with-dt --fail-on-fail` : exit 0.
- [x] `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/bench.yml'))"` :
      parses.
- [x] `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ops-master-gate.yml'))"` :
      parses.

To-do post-merge (3 main runs observation, then second PR) :

- [ ] Confirm 3 consecutive `main` runs PASS the A1 SLO.
- [ ] Flip both `continue-on-error: true` → `false` (job
      `bench-meta-swarm-a1` + step "A1 meta-swarm master gate").
- [ ] Remove every `deferred_ticket: DT-A1-META-SOFT-LAUNCH-3-RUNS`
      from `bench_baselines/v0.10.0/meta_swarm_slos.yaml`.
- [ ] Remove the smoke test `baseline_yaml_soft_launch_dt_present`
      (it pins the soft-launch DT presence).

## Honest semantics

Three corner cases are documented in-source rather than worked around :

1. **p99 proxy via `median + 2 × std_dev`**.  Criterion does not
   directly report p99 ; we approximate from the median + sigma
   estimate, which is precise to ~5 % on near-normal latency
   distributions but underestimates for heavy-tailed runs.
   `parse_criterion_report.py` will be extended in Phase 2 to read
   the raw sample histograms and emit a real p99.
2. **Memory peak Linux-only**.  `/proc/self/status` is the cheapest
   portable way to read VmHWM ; macOS would require `mach_task_basic_info`
   and a `libc` dep we'd rather avoid for now.  On macOS the SLO is
   `SKIP-DT` with ticket `DT-A1-META-MEMORY-MACOS`.  CI ubuntu runs
   produce the real measurement, so the master-gate sees it.
3. **Dual YAML encoding** (`target`+`operator` AND
   `comparator`+`threshold_*`).  Two parsers read the baseline :
   `consolidate_all_slos.py` (Sprint Φ-D-OPS) reads the former,
   `check_regression.py` (Sprint D5) reads the latter.  PR #4
   carries both ; alignment is `DT-A2-SLO-PARSER-UNIFY`.

## Forward-compat

- **Phase 2** (gRPC) : a new bench `meta_swarm_grpc_latency` can be
  added by appending one row to `scripts/benches.list` + adding the
  corresponding SLOs to `bench_baselines/v0.11.0/meta_swarm_slos.yaml`.
  No infra change needed.
- **Phase 3** (per-tenant SLO budgets) : the sidecar schema accepts
  arbitrary `meta.*` fields ; a `tenant_id` key can be added without
  breaking `consolidate_all_slos.py`.
- **Phase 5** (SLA public) : the master rollup
  `SLO-OPS-A1-META-SWARM` (deferred, see § "Forward-compat" in
  `EFE_A1_BENCH_TRACKING.md`) graduates the 7 surface SLOs to a
  customer-facing SLA tracker.

## Master-gate impact

- Before PR #4 :  18 master rollups, 6 surface bundles (al, stream,
  comp, latact, mode2, mode3), no A1 surface SLO.
- After PR #4  :  **19** master rollups (+1 = `SLO-OPS-A1-META-SWARM`),
  7 surface bundles (+ a1_meta_swarm), 7 new surface A1 SLOs.

The new master rollup `SLO-OPS-A1-META-SWARM` inherits the 7
surface A1 SLOs.  Its `rolled_status` follows the standard rollup
rules :

  * PASS    iff every child PASS (or SKIP-DT).
  * SKIP-DT iff every child has `deferred_ticket` set and no FAIL.
  * **FAIL  iff any child FAIL.**  This case blocks
    `master_gate_check.py` (exit non-zero), thereby blocking the
    release.

Soft-launch contract : the 7 surface rows carry `deferred_ticket:
DT-A1-META-SOFT-LAUNCH-3-RUNS`, so a missing measurement
(e.g. `bench-meta-swarm-a1` job didn't run) is SKIP-DT, not
SKIP-SILENT.  A real FAIL still propagates upward — that is
intentional and matches the doctrine "regressions block, missing
data doesn't".  After 3 consecutive `main` runs PASS, a follow-up
PR removes the `deferred_ticket` rows to make the rollup
fail-closed on missing measurements as well.

## Non-regression statement (workspace master gate)

- ✅ All pre-existing tests pass : 192 default + 202 with feature
  flag.  The 7 new tests in `tests/bench_smoke.rs` are additive.
- ✅ `efe-server` SHA-256 byte-identical : PR #4 does **not** touch
  `crates/efe-server/` ; the bench remains in
  `crates/efe-meta-swarm/benches/`, which `efe-server/Cargo.toml`
  does not depend on.
- ✅ `efe-core` unchanged : `cargo check -p efe-core --offline`
  succeeds without diff.
- ✅ The 30 closed defects of Sprint Φ-D-OPS remain closed — PR #4
  does not touch their files.
- ✅ The 18 master rollups of `ops_master_slos.yaml` remain
  unchanged.
- ✅ Existing master-gate behaviour preserved : the new step is
  `continue-on-error: true` for the 3-run soft-launch window
  (pattern from `bench-accuracy` D6).
- ✅ Carve-out R5-B1 holds : `efe-meta-swarm` stays `UNLICENSED`.

## Reviewer guidance

Suggested review order :

1. [`docs/EFE_A1_BENCH_TRACKING.md`][2] — read this first ; it ties
   everything together and is the runbook for future on-call.
2. [`bench_baselines/v0.10.0/meta_swarm_slos.yaml`][1] — the 7 SLO
   contracts.  Pay attention to the thresholds (calibration in § 1
   of the doc).
3. [`crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`][3]
   — the bench logic, sidecar emission, FlushOnDrop sentinel,
   determinism check.
4. [`crates/efe-meta-swarm/tests/bench_smoke.rs`][4] — the 7 pin
   tests.  These are the regression net.
5. [`.github/workflows/bench.yml`][5] (new job, +30 lines) and
   [`.github/workflows/ops-master-gate.yml`][6] (new step, +10 lines).
6. [`scripts/ops/run_suite.sh`][7] (new case `a1_meta_swarm`,
   ~50 lines).
7. The tiny diffs (catalog, KNOWN_CRATES, Cargo.toml `[[bench]]`
   normalisation).

## References

- [`bench_baselines/v0.10.0/meta_swarm_slos.yaml`][1]
- [`docs/EFE_A1_BENCH_TRACKING.md`][2]
- [`crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`][3]
- [`crates/efe-meta-swarm/tests/bench_smoke.rs`][4]
- [`.github/workflows/bench.yml`][5]
- [`.github/workflows/ops-master-gate.yml`][6]
- [`scripts/ops/run_suite.sh`][7]
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`][8] § 8 (master-gate doctrine)
- [`PR_DESCRIPTION.md`][9] (PR #1) line 93 (this PR is the
  follow-up)

[1]: ../../bench_baselines/v0.10.0/meta_swarm_slos.yaml
[2]: ../../docs/EFE_A1_BENCH_TRACKING.md
[3]: benches/three_swarms_kovant_latency.rs
[4]: tests/bench_smoke.rs
[5]: ../../.github/workflows/bench.yml
[6]: ../../.github/workflows/ops-master-gate.yml
[7]: ../../scripts/ops/run_suite.sh
[8]: ../../docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md
[9]: PR_DESCRIPTION.md
