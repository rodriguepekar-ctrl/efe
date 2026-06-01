# EFE A1 — Bench baseline tracking & master-gate hook

> **Status** : ACTIVE since Sprint A1 P1 PR #4 (2026-05).
> **Scope**  : `crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`.
> **Owner**  : meta-swarm-team.

This document describes the bench tracking and master-gate
integration for the **hierarchical multi-swarm coordination layer**
(`efe-meta-swarm`) shipped in Sprint A1 Phase 1.

It documents :

1. The **7 opposable SLOs** that gate the bench (§ 1).
2. How the bench produces them (§ 2).
3. How `run_suite.sh a1_meta_swarm` feeds the master-gate (§ 3).
4. The **freeze procedure** for release engineers (§ 4).
5. The **investigation procedure** when the gate fails (§ 5).
6. The **soft-launch** policy (3 main runs, `continue-on-error: true`)
   and how to flip it off when stable (§ 6).
7. FAQ (§ 7).

## 1. The 7 SLOs

The full definitions live in
[`bench_baselines/v0.10.0/meta_swarm_slos.yaml`][1].  Summary :

| SLO id                              | Type        | Comparator | Threshold | Source              |
|-------------------------------------|-------------|-----------:|----------:|---------------------|
| `SLO-A1-META-LATENCY-P99`           | latency     |       `<=` |    100 ms | Criterion p99 proxy |
| `SLO-A1-META-LATENCY-P50`           | latency     |       `<=` |     30 ms | Criterion median    |
| `SLO-A1-META-CONVERGENCE-ITERS`     | algorithmic |       `<=` |       60  | sidecar             |
| `SLO-A1-META-RESIDUAL-FINAL`        | algorithmic |       `<=` |    1e-3   | sidecar             |
| `SLO-A1-META-DETERMINISM`           | algorithmic |       `==` |      `1`  | sidecar (3 reruns)  |
| `SLO-A1-META-NON-REGRESSION`        | operational |       `<=` |   10 %    | sidecar (vs frozen) |
| `SLO-A1-META-MEMORY-PEAK-MB`        | operational |       `<=` |     64 MB | sidecar (Linux only)|

All thresholds are calibrated against the 3-swarm Kovant scenario :

- *supplier-scoring*       (384 d, κ=0.6, seed 11)
- *contract-risk*          (512 d, κ=0.7, seed 22)
- *spend-categorization*   (384 d, κ=0.5, seed 33)
- meta-space               (256 d, uniform meta-op, box safety ±10)
- α = 0.5, γ = 0.2, tol = 1e-4, max_iters = 100.

The scenario is **identical** to the PR #1 headline example
(`crates/efe-meta-swarm/examples/three_swarms_kovant.rs`) ;  drift is
pinned by `crates/efe-meta-swarm/tests/bench_smoke.rs`.

## 2. How the bench produces them

[`crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`][2]
exposes **5 Criterion benchmarks** :

| Bench function                                 | Purpose                                      |
|------------------------------------------------|----------------------------------------------|
| `three_swarms_kovant_latency_full_iterate`     | End-to-end `iterate()` (drives P50/P99 SLO). |
| `three_swarms_kovant_latency_step_once`        | Single `step_once()` (control).              |
| `three_swarms_kovant_latency_reset`            | `reset()` baseline.                          |
| `three_swarms_kovant_latency_build`            | `MetaSwarmBuilder::build()` baseline.        |
| `three_swarms_kovant_latency_consensus_norm`   | Tensor `norm()` baseline.                    |

Plus a non-Criterion **determinism check** that runs `iterate()` three
times and asserts bit-identical `(iterations, residual.to_bits())`
tuples.

### Two output streams

1. **Criterion native tree**
   `target/criterion/three_swarms_kovant_latency_*/new/estimates.json`
   — consumed by `scripts/parse_criterion_report.py` and
   `scripts/check_regression.py` for the latency SLOs.

2. **Sidecar SLO JSON**
   `target/criterion/three_swarms_kovant_latency/meta-swarm-extras.json`
   — emitted by a `FlushOnDrop` sentinel inside the bench `main()`.
   Carries the algorithmic + operational SLO values that Criterion
   does not measure natively.

Example sidecar :

```json
{
  "slo_compliance": {
    "SLO-A1-META-CONVERGENCE-ITERS": 47,
    "SLO-A1-META-RESIDUAL-FINAL":    8.6e-5,
    "SLO-A1-META-DETERMINISM":       1,
    "SLO-A1-META-MEMORY-PEAK-MB":    28.4,
    "SLO-A1-META-NON-REGRESSION":    0.034
  },
  "meta": {
    "bench_name":     "three_swarms_kovant_latency",
    "scenario":       "supplier-scoring(384) + contract-risk(512) + spend-categorization(384) → meta(256)",
    "writer":         "crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs",
    "writer_version": "PR4"
  }
}
```

When a SLO cannot be measured on the current host (e.g. memory peak
on macOS), the value is replaced with `{"skip_reason": "DT-A1-…"}` ;
`run_suite.sh a1_meta_swarm` strips these from the measures file so
the consolidator promotes them to `SKIP-DT` via the
`deferred_ticket` field on the baseline YAML.

## 3. How the master-gate consumes them

The flow lives in two workflows :

### 3.1 `.github/workflows/bench.yml` — soft-launch PR gate

A new job `bench-meta-swarm-a1` (sibling of `bench-core-mocks`,
`bench-server`, etc.) runs the bench under the unified `bench_runner.sh`
driver and uploads the sidecar + estimates JSON as the partial
artefact `bench-partial-meta-swarm-a1-${sha}`.

**The sidecar is NOT forwarded to `parse_criterion_report.py` via
`--eval-bench-format`** — `SLO-A1-META-*` keys are not part of that
parser's SLO catalog, so the merge would be a no-op.  The
PR-time perf observation comes from the criterion estimates tree
(`new/estimates.json`) processed by `aggregate_bench_reports.py`,
which feeds the standard regression gate + PR-comment pipeline.

The SLO compliance check itself happens in workflow #2 below ;
bench.yml job is `continue-on-error: true` during the soft-launch
window (§ 6) only to mirror the gating behavior of accuracy benches.

### 3.2 `.github/workflows/ops-master-gate.yml` — hard gate

A new step "**A1 meta-swarm master gate**" calls

```bash
bash scripts/ops/run_suite.sh a1_meta_swarm
```

which :

1. Runs `cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency --quick`.
2. Walks `target/criterion/.../estimates.json` to extract median +
   p99-proxy latency.
3. Loads `meta-swarm-extras.json` for the 5 sidecar SLO values.
4. Merges them into `target/ops/measures.json`.

Then the existing **Consolidate** step picks up our YAML via the
`--baselines bench_baselines/v0.10.0/meta_swarm_slos.yaml` argument
and runs `master_gate_check.py`.

The consolidator rolls the 7 surface A1 SLOs up into the new
master rollup `SLO-OPS-A1-META-SWARM` (added to
`bench_baselines/v0.9.0/ops_master_slos.yaml` in PR #4).  When ANY
surface SLO FAILs, the rollup FAILs, and `master_gate_check.py`
exits non-zero — blocking the release.

Soft-launch contract (§ 6) means :

  * The **`bench-meta-swarm-a1` job** in `bench.yml` is
    `continue-on-error: true` (PR-comment regression check
    observes ; does not gate).
  * The **"A1 meta-swarm master gate" step** in
    `ops-master-gate.yml` is also `continue-on-error: true` —
    a transient bench failure (e.g. CI runner OOM) does not block.
  * **But the Consolidate + Hard-gate steps remain blocking** :
    a real `SLO-A1-META-*` FAIL still rolls into
    `SLO-OPS-A1-META-SWARM` FAIL and blocks the gate.  This is
    intentional : regressions block, missing data doesn't (anti-Φ-D.A1).

## 4. Freezing a new baseline (release engineer)

When the perf characteristics have stabilised (e.g. after a major
refactor of `iteration/coupled.rs`), refresh the baseline as follows :

```bash
# 1. Clean build, checkout the release tag.
cargo clean
git checkout v0.11.0

# 2. Run the bench with full sampling.
cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency

# 3. Read the criterion p99 :
python3 - <<'PY'
import json, pathlib
est = pathlib.Path("target/criterion/three_swarms_kovant_latency_full_iterate/new/estimates.json")
d = json.loads(est.read_text())
median_ns = d["median"]["point_estimate"]
std_ns    = d["std_dev"]["point_estimate"]
print(f"median_ms = {median_ns/1e6:.3f}")
print(f"p99_proxy = {(median_ns + 2*std_ns)/1e6:.3f}")
PY

# 4. Edit `bench_baselines/v0.10.0/meta_swarm_slos.yaml` :
#    update `last_p99_observed_ms` to the value printed above,
#    update `host_fingerprint_baseline` to the freeze host descriptor,
#    bump the `version:` (e.g. v0.11.0).

# 5. If the YAML version changed, move the file to
#    bench_baselines/v0.11.0/meta_swarm_slos.yaml and update the
#    `--baselines` path in BOTH .github/workflows/ops-master-gate.yml
#    AND the test pin in tests/bench_smoke.rs::baseline_path().

# 6. Commit + open PR.
git add bench_baselines/ .github/workflows/ tests/bench_smoke.rs
git commit -m "chore(meta-swarm): freeze v0.11.0 bench baseline"
```

## 5. Investigating a master-gate failure

If `ops-master-gate` fails with a `SLO-A1-META-*` offender, follow this
ladder :

### Ladder rung 1 — Confirm the failure is real

The runbook is [`docs/runbooks/ops_master_gate_fail.md`][3].  Read it
first.  Then download the workflow artefact `ops-master-report` and
inspect `master_report.md` for the offender row :

```
| SLO-A1-META-LATENCY-P99 | FAIL | target=100.0 measure=128.4 | ... |
```

### Ladder rung 2 — Reproduce locally

```bash
cargo clean
cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency
cat target/criterion/three_swarms_kovant_latency/meta-swarm-extras.json
```

If your local p99 differs significantly from CI (~×2 is normal),
re-run on the same OS as CI (ubuntu-22.04, 4-vCPU).

### Ladder rung 3 — Diagnose

| Offender                              | Most-likely causes                                          |
|---------------------------------------|-------------------------------------------------------------|
| `SLO-A1-META-LATENCY-P99`             | `.clone()` added in iteration hot path ; new rayon overhead.|
| `SLO-A1-META-CONVERGENCE-ITERS`       | Coupling change in `MockParticipant::step_with_pull`.       |
| `SLO-A1-META-RESIDUAL-FINAL`          | Tolerance change in `ConvergenceCriterion::default`.        |
| `SLO-A1-META-DETERMINISM`             | Non-deterministic RNG seed in `Tensor::pseudo_random`.      |
| `SLO-A1-META-NON-REGRESSION`          | Cumulative perf drift > 10 % since last freeze.             |
| `SLO-A1-META-MEMORY-PEAK-MB`          | New allocation in hot path ; check `BoxSafety::project`.    |

### Ladder rung 4 — Mitigate

- If the regression is **expected** (e.g. PR #5 adds telemetry that
  measurably costs ~5 %) : update `last_p99_observed_ms` in the
  baseline YAML, document the change in the PR description.
- If the regression is **unexpected** : revert the offending change,
  open a follow-up to address the cause.

## 6. Soft-launch policy

The two workflow jobs are deliberately `continue-on-error: true`
during the first **3 consecutive `main` runs** post-merge.  This
follows the pattern adopted by `bench-accuracy` (Sprint D6) and
`bench-accuracy-hybrid` (Sprint D8) — see their comments in
`bench.yml` for details.

### Removing the soft-launch ticket

Once 3 main runs have PASS-ed unanimously :

1. Flip `continue-on-error: true` → `false` on :
   - the job `bench-meta-swarm-a1` in `.github/workflows/bench.yml`,
   - the step "A1 meta-swarm master gate" in `.github/workflows/ops-master-gate.yml`.
2. Remove every `deferred_ticket: DT-A1-META-SOFT-LAUNCH-3-RUNS`
   from `bench_baselines/v0.10.0/meta_swarm_slos.yaml`.
3. The smoke test `baseline_yaml_soft_launch_dt_present` MUST be
   removed in the same commit (it pins the soft-launch DT presence).
4. Open the PR with title
   `chore(meta-swarm): graduate A1 bench out of soft-launch`.

## 7. FAQ

> **My PR fails `SLO-A1-META-LATENCY-P99` ONLY in CI.**

Likely a CI runner variance.  Re-run the workflow once.  If still
red, look at the variance band reported by `bench-aggregate` ;  the
gate has a 10 % regression budget but no absolute variance budget.

> **The bench is unstable on my dev box.**

Set `MODE=full` (no `--quick`) and run on a quiet machine.  Compare
to the M3 Pro reference in `last_p99_observed_ms` divided by
`CI_COEFFICIENT` (1.5 in `bench.yml`).

> **Why a sidecar JSON instead of Criterion's built-in `--save-baseline`?**

Criterion's baseline-compare flag works only on temporal measures
(latency / throughput).  We need to gate on convergence iterations,
final residual, determinism, and memory — all non-temporal.  The
sidecar is the cheapest path that does not require forking Criterion.

> **Why are A1 SLOs `SKIP-DT` and not `PASS` when the bench works?**

The soft-launch contract (§ 6) sets `deferred_ticket: DT-A1-META-SOFT-LAUNCH-3-RUNS`
on every row.  The consolidator promotes successful measurements to
`PASS` regardless of the DT field, but the gate runs in
`--tolerate-skip-with-dt` mode so a transient `SKIP-DT` doesn't
break the build.  This is documented in `master_gate_check.py`
line 88 (anti-Φ-D.A1 doctrine).

> **What happens if `last_p99_observed_ms` is stale?**

The `NON-REGRESSION` SLO will tighten over time as upstream perf
improves, then *loosen* abruptly when a regression sneaks past.  The
freeze procedure (§ 4) is intended to be run **every release tag**
to keep the baseline fresh.  CI does not auto-update it.

## 8. References

- [`bench_baselines/v0.10.0/meta_swarm_slos.yaml`][1]
- [`crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`][2]
- [`docs/runbooks/ops_master_gate_fail.md`][3]
- [`docs/EFE_A1_PHASE1_ARCHITECTURE.md`][4] § 9 (bench targets)
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`][5] § 8 (master-gate doctrine)
- [`crates/efe-meta-swarm/PR4_DESCRIPTION.md`][6]

[1]: ../bench_baselines/v0.10.0/meta_swarm_slos.yaml
[2]: ../crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs
[3]: runbooks/ops_master_gate_fail.md
[4]: EFE_A1_PHASE1_ARCHITECTURE.md
[5]: EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md
[6]: ../crates/efe-meta-swarm/PR4_DESCRIPTION.md
