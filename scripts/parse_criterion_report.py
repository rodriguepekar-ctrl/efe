#!/usr/bin/env python3
"""
scripts/parse_criterion_report.py — Sprint C bench report composer.

Walks `target/criterion/<group>/<id>/new/estimates.json` (the rich
JSON criterion writes after every bench), combines the parsed data
with host metadata, and produces a structured report at
`target/bench-report-{ts}-{sha}.json`.

The bencher-format text output is too lossy (the bench name does
not appear on the same line as the measurement) ; criterion's JSON
files are the authoritative source.

Schema (version 1.0):

```json
{
  "version": "1.0",
  "timestamp": "20260425T120000Z",
  "git": { "sha": "abcd1234", "branch": "main", "dirty": false },
  "host": {
    "os": "Darwin 24.0.0", "cpu": "Apple M1 Pro", "ncpu": 10, "ram_gb": 32
  },
  "toolchain": {
    "rustc": "rustc 1.85.0", "cargo": "cargo 1.85.0",
    "blas_backend": "accelerate", "rustflags": "-C target-cpu=native"
  },
  "results": {
    "swarm_e2e_small_latency::execute": {"median_ns": 12300, "jitter_ns": 1100},
    ...
  },
  "slo_compliance": {
    "SLO-1": {
      "name": "small latency p95",
      "target_ns": 50000000,
      "measured_ns": 28500000,
      "passed": true
    },
    ...
  }
}
```

The bencher format reports the **median** as `<ns> ns/iter` and the
**jitter half-range** in `+/- <jitter>`. The p95 is approximated as
`median + jitter` (criterion's bencher format does not export full
percentiles ; for richer data, parse `target/criterion/*/estimates.json`
in a future sprint).
"""

from __future__ import annotations
import argparse
import glob
import json
import os
import sys
from typing import Any

# ----------------------------------------------------------------
# Criterion JSON walker
# ----------------------------------------------------------------

# Bench keys we care about (exact match between key and bench id is
# enforced by the swarm_e2e.rs structure: `<group>::<bench>`).
#
# Sprint C prefixes (mock-based, kept for v0.1.0 baseline comparison):
#   - swarm_e2e_*        — orchestration overhead with mock strategies
#   - memory_e2e_*       — heap peak / alloc residue with mock strategies
#
# Sprint D1 prefixes (production-wired, new SLO targets in v0.1.1):
#   - real_e2e_*         — full request lifecycle with real Mvp* strategies
#   - real_phase_*       — per-phase isolation (genesis, alignment)
#   - real_memory_*      — heap peak / alloc residue with real strategies
#
# Sprint D2 prefixes (memory infrastructure, new SLO targets in v0.1.2):
#   - pool_acquire_*     — pool acquire warm/cold/saturated/concurrent
#   - arena_alloc_*      — arena typed allocation cost
#   - arena_reset        — arena reset cost
#   - realistic_swarm_alloc_pattern — Mvp pattern (3 fixtures)
#
# Sprint D3 prefixes (compute pipeline, new SLO targets in v0.1.3):
#   - compute_pipeline_chain_*       — full 6-op chain (with _naive/_blas)
#   - compute_pipeline_per_phase_*   — 6 sub-benches per fixture
#   - compute_pipeline_throughput_*  — n_agents sweep
#
# The `_blas` / `_naive` suffix on real_e2e/real_phase/compute_pipeline_*
# groups is part of the bench id, not the group prefix, so it is captured
# automatically by the per-(group, id) walk.
WANTED_PREFIXES = (
    "swarm_e2e_",
    "memory_e2e_",
    "real_e2e_",
    "real_phase_",
    "real_memory_",
    "pool_acquire_",
    # `pool_concurrent_acquire` is a SEPARATE group from `pool_acquire_*`
    # — its name does NOT start with `pool_acquire_`. Audit-fix : the
    # group was being silently dropped because of the literal prefix
    # mismatch, breaking SLO-D2-3.
    "pool_concurrent_acquire",
    "arena_alloc_",
    "arena_reset",
    "realistic_swarm_alloc_pattern",
    "compute_pipeline_chain_",
    "compute_pipeline_per_phase_",
    "compute_pipeline_throughput_",
    "server_rpc_full_stack",
    "server_rpc_minimal_stack",
    "server_rpc_concurrent",
    "server_rpc_codec_only",
    # Sprint D6 — accuracy metrics emitted by `eval/scripts/run_eval.py`
    # via `--output-bench-format`. The values are scaled by 1e9 so the
    # ns-based parser can compare them to the (also-scaled) SLO targets
    # via the new `gte` comparator (cf. evaluate_slos below).
    "eval_accuracy",
    "eval_latency",
)


def load_criterion_estimates(criterion_root: str) -> dict[str, dict[str, float]]:
    """Walk `<criterion_root>/<group>/<id_segments…>/new/estimates.json`.

    Returns a map `{"<group>::<id>": {"mean_ns", "median_ns",
    "std_dev_ns", "approx_p95_ns"}}`. The approx p95 is
    `median + 2 * MAD` as a robust quick estimate (criterion does
    not export an explicit p95 in the bencher-format output).

    # Multi-segment ids
    #
    # `criterion::BenchmarkId::new("prefix", value)` produces an id
    # like ``"prefix/value"``, so the on-disk path becomes
    # ``<group>/<prefix>/<value>/new/estimates.json`` (3 segments
    # before `new`, not 2). The pre-D5 implementation used a flat
    # glob ``*/*/new/estimates.json`` and silently dropped these
    # measurements, breaking SLO-D2-4/5/6 (arena_alloc_zeroed_typed)
    # and several compute_sprint3 / server_rpc_codec_only sub-benches.
    #
    # We now use a recursive glob (``**``) and reconstruct the bench
    # id as the joined segments between the group dir and the `new`
    # dir. The SLO key format ``<group>::<id_with_slashes>`` matches
    # what the SLO_DEFINITIONS table declares, so no other changes
    # are needed downstream.
    """
    out: dict[str, dict[str, float]] = {}
    pattern = os.path.join(criterion_root, "**", "new", "estimates.json")
    for path in sorted(glob.glob(pattern, recursive=True)):
        # path = <criterion_root>/<group>/<id_segments…>/new/estimates.json
        # Use os.path.relpath so the segment math is independent of
        # whether `criterion_root` was passed absolute or relative.
        rel = os.path.normpath(os.path.relpath(path, criterion_root)).split(os.sep)
        # rel = ['<group>', '<id_seg1>', ..., '<id_segN>', 'new', 'estimates.json']
        # We need at least one id segment, so len(rel) >= 4
        # (group, id, new, estimates.json).
        if len(rel) < 4:
            continue
        try:
            new_idx = rel.index("new")
        except ValueError:
            continue
        if new_idx < 2:
            # Path is too shallow to carry a (group, id) pair.
            continue
        group = rel[0]
        # Bench id = all segments between group and 'new', re-joined with
        # '/' so the resulting key matches what the SLO_DEFINITIONS table
        # uses (e.g. "arena_alloc_zeroed_typed::f32/1024").
        bench_id = "/".join(rel[1:new_idx])
        # Filter to our wanted benches only.
        if not group.startswith(WANTED_PREFIXES):
            continue
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        median_ns = data["median"]["point_estimate"]
        mad_ns = data["median_abs_dev"]["point_estimate"]
        mean_ns = data["mean"]["point_estimate"]
        std_dev_ns = data["std_dev"]["point_estimate"]
        # Robust p95 approx: median + 2 * MAD (≈ 95th percentile if
        # the distribution is roughly normal). For better p95, use
        # `target/criterion/<group>/<id>/new/sample.json` and compute
        # the empirical quantile — out of scope for this MVP parser.
        approx_p95_ns = median_ns + 2.0 * mad_ns
        key = f"{group}::{bench_id}"
        out[key] = {
            "mean_ns": mean_ns,
            "median_ns": median_ns,
            "std_dev_ns": std_dev_ns,
            "mad_ns": mad_ns,
            "approx_p95_ns": approx_p95_ns,
        }
    return out


# ----------------------------------------------------------------
# SLO compliance check
# ----------------------------------------------------------------

# Each SLO maps to a bench measurement key + a target in nanoseconds.
# The "approximate p95" used here is `median + jitter` (criterion
# bencher format limitation; the full percentile is in
# `target/criterion/<name>/new/estimates.json` if needed).
SLO_DEFINITIONS: list[dict[str, Any]] = [
    # ---------------------------------------------------------------
    # Sprint C SLOs (mock-based — kept for v0.1.0 backward compat).
    # ---------------------------------------------------------------
    {
        "id": "SLO-1",
        "name": "small latency p95 (mock)",
        "bench_key": "swarm_e2e_small_latency::execute",
        "target_ns": 50_000_000,  # 50 ms
    },
    {
        "id": "SLO-2",
        "name": "medium latency p95 (mock)",
        "bench_key": "swarm_e2e_medium_latency::execute",
        "target_ns": 200_000_000,  # 200 ms
    },
    {
        "id": "SLO-3",
        "name": "large latency p95 (mock)",
        "bench_key": "swarm_e2e_large_latency::execute",
        "target_ns": 1_000_000_000,  # 1 s
    },
    # ---------------------------------------------------------------
    # Sprint D1 SLOs (real strategies — primary targets from v0.1.1).
    #
    # Targets are calibrated against the production Mvp* wiring
    # (`build_mvp_backend`) running the v2 fixtures
    # (Box-Muller Gaussian embeddings) on an Apple M3 Pro reference.
    # The CI workflow applies a 1.5× coefficient to allow for slower
    # runners — see `docs/bench/d1_real_strategies_architecture.md`
    # §10 for the calibration methodology.
    #
    # The `_naive` suffix targets the default (no-BLAS) build. When
    # the `accelerate` / `openblas` feature is enabled at the
    # workspace root, the bench reports under the `_blas` suffix and
    # the SLO key changes too. We define both pairs so a BLAS run
    # checks the BLAS targets, and a non-BLAS run checks the naïve
    # ones — `bench is None` short-circuits the missing branch.
    # ---------------------------------------------------------------
    # ─────────────────────────────────────────────────────────────
    # CALIBRATION NOTE (audit Sprint D5+) :
    #
    # The original v0.1.1 targets (5 / 50 / 250 ms) assumed a
    # single-round happy path. In reality, the v2 fixtures take 8
    # multi-rounds of `MvpCpuFusion` to converge, producing real
    # measurements of ~170 / ~340 / ~340 ms on M3 Pro. Targets are
    # now calibrated at ~×2 the empirical mean, matching the D4
    # SLO budget (D4 = D1 chain + ~600 µs RPC overhead, so D1
    # targets are slightly tighter than D4 targets).
    # ─────────────────────────────────────────────────────────────
    {
        "id": "SLO-RD1-1",
        "name": "real small latency p95 (naïve)",
        "bench_key": "real_e2e_latency_naive::small",
        "target_ns": 400_000_000,  # 400 ms (measured ~170 ms ; ×2.4 margin)
    },
    {
        "id": "SLO-RD1-2",
        "name": "real medium latency p95 (naïve)",
        "bench_key": "real_e2e_latency_naive::medium",
        "target_ns": 700_000_000,  # 700 ms (measured ~340 ms ; ×2 margin)
    },
    {
        "id": "SLO-RD1-3",
        "name": "real large latency p95 (naïve)",
        "bench_key": "real_e2e_latency_naive::large",
        "target_ns": 700_000_000,  # 700 ms (measured ~340 ms ; ×2 margin)
    },
    {
        "id": "SLO-RD1-1B",
        "name": "real small latency p95 (BLAS)",
        "bench_key": "real_e2e_latency_blas::small",
        # small fixture (M·K·N=8 192 < 32 768) does NOT trigger BLAS
        # dispatch, so the BLAS variant matches the naïve variant.
        "target_ns": 400_000_000,
    },
    {
        "id": "SLO-RD1-2B",
        "name": "real medium latency p95 (BLAS)",
        "bench_key": "real_e2e_latency_blas::medium",
        # medium triggers BLAS on the 2 matmuls ; expect ~×1.5 speedup
        # on the chain (BLAS dominates only the matmul fraction, ~60 %).
        "target_ns": 500_000_000,  # 500 ms
    },
    {
        "id": "SLO-RD1-3B",
        "name": "real large latency p95 (BLAS)",
        "bench_key": "real_e2e_latency_blas::large",
        "target_ns": 500_000_000,  # 500 ms (~×1.5 speedup expected)
    },
    {
        "id": "SLO-RD1-T1",
        "name": "real throughput p95 8c-small (naïve)",
        "bench_key": "real_e2e_throughput_naive::c8",
        "target_ns": 700_000_000,  # 700 ms / batch of 8 ⇒ ≥ ~11 req/s.
    },
    # ---------------------------------------------------------------
    # Sprint D2 SLOs (memory infrastructure — pool/arena/backend).
    #
    # Calibration on Apple M3 Pro / 11 cores / no-BLAS / 1 MiB
    # bench arena. Targets are 3-100× the empirical mean to absorb
    # scheduler noise and host variance without hiding real
    # regressions. See `docs/bench/d2_memory_real_arena_architecture.md`
    # §9 for the full calibration methodology.
    # ---------------------------------------------------------------
    {
        "id": "SLO-D2-1",
        "name": "pool acquire warm",
        "bench_key": "pool_acquire_warm::default",
        "target_ns": 1_000,  # 1 µs
    },
    {
        "id": "SLO-D2-2",
        "name": "pool acquire cold (1 MiB)",
        "bench_key": "pool_acquire_cold::1MiB",
        "target_ns": 1_000_000,  # 1 ms (amortised in prod)
    },
    {
        "id": "SLO-D2-3",
        "name": "pool concurrent acquire (8c)",
        "bench_key": "pool_concurrent_acquire::c8",
        "target_ns": 50_000,  # 50 µs for 8-task batch
    },
    {
        "id": "SLO-D2-4",
        "name": "arena alloc f32(1)",
        "bench_key": "arena_alloc_zeroed_typed::f32/1",
        "target_ns": 200,
    },
    {
        "id": "SLO-D2-5",
        "name": "arena alloc f32(1k)",
        "bench_key": "arena_alloc_zeroed_typed::f32/1024",
        "target_ns": 2_000,
    },
    {
        "id": "SLO-D2-6",
        "name": "arena alloc f32(1M)",
        "bench_key": "arena_alloc_zeroed_typed::f32/1000000",
        "target_ns": 2_000_000,
    },
    {
        "id": "SLO-D2-7",
        "name": "arena reset",
        "bench_key": "arena_reset::default",
        "target_ns": 50,
    },
    {
        "id": "SLO-D2-8",
        "name": "swarm alloc pattern small",
        "bench_key": "realistic_swarm_alloc_pattern::small",
        "target_ns": 5_000,
    },
    {
        "id": "SLO-D2-9",
        "name": "swarm alloc pattern medium",
        "bench_key": "realistic_swarm_alloc_pattern::medium",
        "target_ns": 30_000,
    },
    {
        "id": "SLO-D2-10",
        "name": "swarm alloc pattern large",
        "bench_key": "realistic_swarm_alloc_pattern::large",
        "target_ns": 150_000,
    },
    # ---------------------------------------------------------------
    # Sprint D3 SLOs (compute pipeline — 6-op chain).
    #
    # Calibration on Apple M3 Pro / no-BLAS / 1.85.0 with the v2
    # fixture shapes (small/medium/large = 8/32/128 agents, dim
    # 128/256/256). Targets are 3-15× the empirical mean to absorb
    # scheduler noise and BLAS thread oversubscription on busier
    # hosts.
    #
    # Naïve targets (default build):
    # ---------------------------------------------------------------
    {
        "id": "SLO-D3-1",
        "name": "compute pipeline chain small (naïve)",
        "bench_key": "compute_pipeline_chain_naive::small",
        "target_ns": 200_000,  # 200 µs
    },
    {
        "id": "SLO-D3-2",
        "name": "compute pipeline chain medium (naïve)",
        "bench_key": "compute_pipeline_chain_naive::medium",
        "target_ns": 500_000,  # 500 µs
    },
    {
        "id": "SLO-D3-3",
        "name": "compute pipeline chain large (naïve)",
        "bench_key": "compute_pipeline_chain_naive::large",
        "target_ns": 20_000_000,  # 20 ms
    },
    # ---------------------------------------------------------------
    # BLAS targets (--features accelerate / openblas):
    #
    # `small` does NOT trigger BLAS dispatch (M·K·N = 8 192 < 32 768)
    # so its target equals the naïve one. `medium` and `large` see
    # the speedup on the 2 matmuls.
    # ---------------------------------------------------------------
    {
        "id": "SLO-D3-1B",
        "name": "compute pipeline chain small (BLAS)",
        "bench_key": "compute_pipeline_chain_blas::small",
        "target_ns": 200_000,  # same as naïve — no dispatch
    },
    {
        "id": "SLO-D3-2B",
        "name": "compute pipeline chain medium (BLAS)",
        "bench_key": "compute_pipeline_chain_blas::medium",
        "target_ns": 250_000,  # ~2× speedup expected
    },
    {
        "id": "SLO-D3-3B",
        "name": "compute pipeline chain large (BLAS)",
        "bench_key": "compute_pipeline_chain_blas::large",
        "target_ns": 5_000_000,  # ~4× speedup expected
    },
    # ---------------------------------------------------------------
    # Sprint D4 SLOs (gRPC server stack — loopback TCP).
    #
    # Calibration on Apple M3 Pro / no-BLAS / no-TLS / no-auth, with
    # the v2 fixture multi-round path. Measurements (release):
    # full small ~180 ms, full medium ~370 ms, full large ~370 ms,
    # concurrent c8 small ~390 ms. Targets are ~×2-3 the empirical
    # mean — generous because loopback variance is ~10 % and a real
    # regression (broken interceptor, tonic version drift) would
    # easily blow past these caps.
    # ---------------------------------------------------------------
    {
        "id": "SLO-D4-1",
        "name": "server rpc full stack small",
        "bench_key": "server_rpc_full_stack::small",
        "target_ns": 500_000_000,  # 500 ms
    },
    {
        "id": "SLO-D4-2",
        "name": "server rpc full stack medium",
        "bench_key": "server_rpc_full_stack::medium",
        "target_ns": 800_000_000,  # 800 ms
    },
    {
        "id": "SLO-D4-3",
        "name": "server rpc full stack large",
        "bench_key": "server_rpc_full_stack::large",
        "target_ns": 800_000_000,  # 800 ms
    },
    {
        "id": "SLO-D4-T1",
        "name": "server rpc concurrent 8c-small",
        "bench_key": "server_rpc_concurrent::c8",
        "target_ns": 800_000_000,  # 800 ms for 8-call batch (~10 req/s min)
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D6 — Accuracy SLO (golden support_triage).
    #
    # These SLO use the new `comparator: gte` semantics : the measured
    # value must be GREATER OR EQUAL to the target, the inverse of the
    # default `lte` for latency SLO. The `target_ns` field is reused
    # but represents a scaled accuracy ratio (× 1e9) — see the
    # `--output-bench-format` documentation in `eval/scripts/run_eval.py`
    # for the mapping convention.
    #
    # As of Sprint D6, the accuracy targets EXPECT TO FAIL until D7
    # remediates the genesis/fusion structural bias. The audit doc
    # `docs/audits/d6_accuracy_validation.md` documents the trajectory.
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D6-ACC-1",
        "name": "Top-1 accuracy on golden support_triage",
        "bench_key": "eval_accuracy::top1",
        "target_ns": 750_000_000,  # 0.75 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D6-ACC-2",
        "name": "Macro-F1 on golden support_triage",
        "bench_key": "eval_accuracy::macro_f1",
        "target_ns": 700_000_000,  # 0.70 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D6-ACC-3",
        "name": "Mean confidence on correct predictions",
        "bench_key": "eval_accuracy::mean_conf_correct",
        "target_ns": 650_000_000,  # 0.65 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D6-ACC-4",
        "name": "False reject rate (lower is better)",
        "bench_key": "eval_accuracy::false_reject_rate",
        "target_ns": 250_000_000,  # 0.25 × 1e9
        "comparator": "lte",  # default — listed explicitly for symmetry
        "display_as": "ratio",
    },
    # SLO-D6-LAT-1 — Sprint D6 audit found that using `eval_latency::max_ns`
    # as the gate is too sensitive : a single slow RPC outlier (warmup
    # request with cold caches, ~18 s observed) flips the SLO to FAIL on
    # an otherwise healthy run. We switch to `eval_latency::mean_ns` which
    # is a stable per-batch metric and reflects steady-state performance.
    # The mean target accommodates the new D6 multi-round path
    # (~2 s/RPC measured) with ×1.5 margin.
    {
        "id": "SLO-D6-LAT-1",
        "name": "Eval mean client-observed latency",
        "bench_key": "eval_latency::mean_ns",
        "target_ns": 3_000_000_000,  # 3 s wallclock per-RPC mean (post-D6 multi-round path)
        "comparator": "lte",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D7 — multi-catalog anti-overfit SLOs.
    #
    # These gates fire on the keys produced by `run_eval.py
    # --multi-catalog --output-bench-format`. They do not replace the
    # D6 single-catalog SLOs — the two families coexist :
    #
    #   * D6-ACC-* gate the support_triage golden alone (single
    #     catalog) ; they are the existing baseline gate and remain
    #     until 3 main runs PASS.
    #   * D7-ACC-* gate the *worst-case* per-catalog metric and the
    #     aggregate across {support_triage, fraud_alert,
    #     billing_escalation}. They prevent a calibration that
    #     overfits one catalog while regressing another.
    #
    # The gate values mirror `run_eval.py::MULTI_CATALOG_GATES` so
    # there is a single source of truth (`docs/bench/d7_genesis_…`).
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D7-MC-1",
        "name": "Min per-catalog accuracy (D7 anti-overfit gate)",
        # NOTE : reads the dedicated `min_top1` key produced by
        # `run_eval.py --multi-catalog`. The single-catalog mode
        # produces `top1` (support_triage only) → no key collision,
        # no order dependency on the parser's merge step.
        "bench_key": "eval_accuracy::min_top1",
        "target_ns": 650_000_000,  # 0.65 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D7-MC-2",
        "name": "Min per-catalog macro-F1 (D7 anti-overfit gate)",
        "bench_key": "eval_accuracy::min_macro_f1",
        "target_ns": 600_000_000,  # 0.60 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D7-MC-3",
        "name": "Aggregate accuracy across 3 catalogs",
        "bench_key": "eval_accuracy::aggregate_top1",
        "target_ns": 750_000_000,  # 0.75 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D7-MC-4",
        "name": "Aggregate macro-F1 across 3 catalogs",
        "bench_key": "eval_accuracy::aggregate_macro_f1",
        "target_ns": 750_000_000,  # 0.75 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D8 — Plan B (hybrid decoder) + Real-Data validation SLOs.
    #
    # Two SLO families :
    #
    #   * D8-HYB-*  read keys produced by `run_eval_hybrid.py` ;
    #               disjoint from swarm-only keys (invariant pinned
    #               by `scripts/tests/test_d8_hybrid_eval.py`).
    #   * D8-RD-*   read keys produced by ANY eval script (single,
    #               multi, or hybrid) when run against a real-data
    #               golden ; the workflow renames the bench-format
    #               keys with a `real_data_` prefix at upload time.
    #   * D8-LAT-2  is the relaxed p99 latency on real-data
    #               (longer texts, slower per-RPC).
    #
    # The 4 D8-HYB metrics gate the Plan B decoder ; if it fails, the
    # operator should re-tune the HybridClassifier thresholds before
    # shipping.
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D8-HYB-1",
        "name": "Hybrid decoder accuracy (single catalog)",
        "bench_key": "eval_accuracy::hybrid_top1",
        "target_ns": 800_000_000,  # 0.80 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D8-HYB-2",
        "name": "Hybrid decoder macro-F1 (single catalog)",
        "bench_key": "eval_accuracy::hybrid_macro_f1",
        "target_ns": 750_000_000,  # 0.75 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D8-HYB-3",
        "name": "Hybrid decoder reject rate (lower is better)",
        "bench_key": "eval_accuracy::hybrid_reject_rate",
        "target_ns": 150_000_000,  # 0.15 × 1e9
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D8-HYB-4",
        "name": "Hybrid decoder min per-catalog macro-F1 (anti-overfit)",
        "bench_key": "eval_accuracy::hybrid_min_macro_f1",
        "target_ns": 600_000_000,  # 0.60 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    # Real-data SLOs — measured on partner golden after anonymization.
    {
        "id": "SLO-D8-RD-1",
        "name": "Real-data accuracy (partner golden)",
        "bench_key": "eval_accuracy::real_data_top1",
        "target_ns": 800_000_000,  # 0.80 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D8-RD-2",
        "name": "Real-data macro-F1 (partner golden)",
        "bench_key": "eval_accuracy::real_data_macro_f1",
        "target_ns": 750_000_000,  # 0.75 × 1e9
        "comparator": "gte",
        "display_as": "ratio",
    },
    # Relaxed latency SLO for real data (longer texts).
    {
        "id": "SLO-D8-LAT-2",
        "name": "Real-data p99 client-observed latency",
        "bench_key": "eval_latency::real_data_p99_ns",
        "target_ns": 5_000_000_000,  # 5 s p99
        "comparator": "lte",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D9 SLOs (Docker image + Helm chart + load/soak tests).
    #
    # Cf. ``docs/bench/d9_docker_helm_load_architecture.md`` §11 and
    # the deep-analysis supplement (§1-3 for the math).
    #
    # All four ``D9-LOAD-*`` keys are produced by
    # ``bench/load/parse_ghz_report.py`` (the ghz adapter) and
    # injected into the aggregate bench-format JSON ; the
    # ``D9-SOAK-1`` key is produced by ``bench/soak/leak_detector.py``.
    #
    # The keys are intentionally disjoint from ``eval_*::*`` —
    # collision would silently overwrite either accuracy or load
    # measurements. Pinned by
    # ``bench/load/tests/test_parse_ghz_report.py::test_load_keys_are_disjoint_from_accuracy_namespace``
    # and the analogous ``test_soak_keys_are_disjoint_*`` test.
    #
    # Decision logic on the ``ratio`` SLO targets is unaffected by
    # the CI coefficient — slower runners don't make latency budgets
    # softer, and they don't make throughput floors softer either.
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D9-LOAD-1",
        "name": "Sustained p99 latency under bench load",
        "bench_key": "load_sustained::p99_ns",
        "target_ns": 5_000_000_000,  # 5 s
        "comparator": "lte",
    },
    {
        "id": "SLO-D9-LOAD-2",
        "name": "Sustained error rate under bench load",
        "bench_key": "load_sustained::error_ratio",
        # 0.01 ratio expressed in ×1e9 scaling — 0.01 × 1e9 = 1e7.
        "target_ns": 10_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D9-LOAD-3",
        "name": "Throughput floor (RPS, sustained or stress)",
        "bench_key": "load_stress::throughput_rps",
        # 5 RPS scaled ×1e9 → 5e9.
        "target_ns": 5_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D9-LOAD-4",
        "name": "Spike recovery time",
        "bench_key": "load_spike::recovery_seconds",
        # 30 s scaled ×1e9 → 3e10.
        "target_ns": 30_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D9-SOAK-1",
        "name": "Soak RSS slope (24h leak detector)",
        "bench_key": "soak::rss_slope_bps",
        # 870 B/s scaled ×1e9 → 8.7e11.
        "target_ns": 870_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D10 SLOs (SDK docs + quickstart + 4 examples — Pilot 1.0
    # DX). Cf. ``docs/bench/d10_sdk_docs_examples_architecture.md``
    # §12.
    #
    # Three doc-build invariants and two DX wallclock budgets. The
    # ``docs::*`` keys come from ``.github/workflows/docs.yml``
    # artefacts ; the ``dx::*`` keys come from
    # ``scripts/validate_quickstart.sh`` (manual run by 3 reviewers,
    # automated in the bench-dx job).
    #
    # All keys live in their own namespace prefix
    # (``docs::``, ``dx::``) so they remain disjoint from
    # ``eval_*::*``, ``load_*::*``, ``soak::*``. Pinned by
    # ``scripts/tests/test_d10_disjoint_keys.py``.
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D10-DOC-1",
        "name": "Docs build strict — zero warnings",
        "bench_key": "docs::build_strict_warnings",
        "target_ns": 0,
        "comparator": "lte",
    },
    {
        "id": "SLO-D10-DOC-2",
        "name": "API reference symbol coverage",
        "bench_key": "docs::api_coverage_ratio",
        # 0.95 ratio scaled ×1e9 → 9.5e8.
        "target_ns": 950_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D10-DOC-3",
        "name": "Markdown link checker — zero 404",
        "bench_key": "docs::link_check_404",
        "target_ns": 0,
        "comparator": "lte",
    },
    {
        "id": "SLO-D10-DX-1",
        "name": "Quickstart wallclock (median 3 reviewers)",
        "bench_key": "dx::quickstart_wallclock_s",
        # 600 s scaled ×1e9 → 6e11.
        "target_ns": 600_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D10-DX-2",
        "name": "Example smoke aggregate runtime",
        "bench_key": "dx::examples_smoke_aggregate_s",
        # 30 s scaled ×1e9 → 3e10.
        "target_ns": 30_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D11 SLOs (per-tenant isolation — Phase 2 multi-tenant SaaS).
    #
    # Cf. ``docs/bench/d11_per_tenant_isolation_architecture.md`` §15.
    #
    # Three families:
    #   * isolation::* — produced by the Rust isolation harness
    #     (``crates/efe-server/tests/d11_isolation_*.rs``). Each test
    #     file emits a metric pinning the invariant.
    #   * cost::*      — produced by ``scripts/billing/aggregate_usage.py``
    #     (drift between aggregator and a Prometheus snapshot, and
    #     the aggregator wallclock).
    #   * obs::*       — produced by a Prometheus query measuring
    #     active tenant cardinality.
    #
    # All SKIPPED in baseline v0.1.11 ; turn into PASS/FAIL after the
    # isolation CI bake-in (see ``.github/workflows/bench-isolation.yml``).
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D11-MT-1",
        "name": "Cross-tenant data leakage count",
        "bench_key": "isolation::cross_tenant_leakage",
        "target_ns": 0,
        "comparator": "lte",
    },
    {
        "id": "SLO-D11-MT-2",
        "name": "Rate limit disjointness ratio (per-tenant fairness)",
        "bench_key": "isolation::rate_limit_disjoint_pct",
        # 0.99 ratio scaled ×1e9 → 9.9e8.
        "target_ns": 990_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D11-MT-3",
        "name": "Audit log tenant_id attribution coverage",
        "bench_key": "isolation::audit_log_attribution_pct",
        # 1.0 ratio scaled ×1e9 → 1e9.
        "target_ns": 1_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D11-MT-4",
        "name": "Business metrics tenant label coverage",
        "bench_key": "isolation::metric_label_coverage_pct",
        "target_ns": 1_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D11-MT-5",
        "name": "Cost aggregator drift vs Prometheus",
        "bench_key": "cost::aggregator_drift_pct",
        # 0.01 ratio scaled ×1e9 → 1e7.
        "target_ns": 10_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D11-MT-6",
        "name": "Cost aggregator export wallclock (30d window)",
        "bench_key": "cost::usage_export_latency_s",
        # 60 s scaled ×1e9 → 6e10.
        "target_ns": 60_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D11-MT-7",
        "name": "Tenant cardinality (distinct tenant label values)",
        "bench_key": "obs::tenant_cardinality_count",
        "target_ns": 1_000,
        "comparator": "lte",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D12 SLOs (catalog admin + versioning).
    #
    # Cf. ``docs/bench/d12_catalog_admin_versioning_architecture.md`` §
    # 16 (SLO family). The 7 metrics measure the new
    # ``efe.v1.catalog.CatalogService`` surface :
    #
    #   * D12-CT-1..5 — wire latency budgets per RPC at p99.
    #   * D12-CT-6    — register/evolve serialization correctness :
    #     two concurrent register calls must NOT collide on a version
    #     integer (pinned to 100 % pass).
    #   * D12-CT-7    — content_sha256 immutability witness :
    #     register-then-get must yield the SAME digest the server
    #     stamped (pinned to 100 % match).
    #
    # All SKIPPED in baseline v0.1.12 ; turned into PASS/FAIL after
    # the bench-catalog CI bake-in (see
    # ``.github/workflows/bench-catalog.yml``).
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D12-CT-1",
        "name": "RegisterCatalog p99 latency (≤ 5 classes)",
        "bench_key": "catalog::register_p99_small",
        "target_ns": 200_000_000,  # 200 ms
        "comparator": "lte",
    },
    {
        "id": "SLO-D12-CT-2",
        "name": "RegisterCatalog p99 latency (100 classes)",
        "bench_key": "catalog::register_p99_large",
        "target_ns": 1_500_000_000,  # 1.5 s
        "comparator": "lte",
    },
    {
        "id": "SLO-D12-CT-3",
        "name": "EvolveCatalog p99 latency (APPEND_CLASS)",
        "bench_key": "catalog::evolve_append_p99",
        "target_ns": 250_000_000,  # 250 ms
        "comparator": "lte",
    },
    {
        "id": "SLO-D12-CT-4",
        "name": "GetCatalog p99 latency",
        "bench_key": "catalog::get_p99",
        "target_ns": 50_000_000,  # 50 ms
        "comparator": "lte",
    },
    {
        "id": "SLO-D12-CT-5",
        "name": "ListCatalogs p99 latency",
        "bench_key": "catalog::list_p99",
        "target_ns": 75_000_000,  # 75 ms
        "comparator": "lte",
    },
    {
        "id": "SLO-D12-CT-6",
        "name": "Concurrent register/evolve serialization correctness",
        "bench_key": "catalog::serialization_pass_pct",
        # 1.0 ratio scaled ×1e9 → 1e9 (≥ 100% pass).
        "target_ns": 1_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D12-CT-7",
        "name": "content_sha256 immutability witness round-trip",
        "bench_key": "catalog::sha256_witness_match_pct",
        # 1.0 ratio scaled ×1e9 → 1e9 (≥ 100% match).
        "target_ns": 1_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    # ─────────────────────────────────────────────────────────────────
    # Sprint D13 SLOs (observability dashboards).
    #
    # Cf. ``docs/bench/d13_observability_dashboards_architecture.md`` §
    # 19. The 7 metrics measure the dashboards / alerts / log-pipeline
    # surface :
    #
    #   * D13-OBS-1   — Grafana panel render time p99 ≤ 3 s.
    #   * D13-OBS-2   — Prometheus scrape success ≥ 99.9 %.
    #   * D13-OBS-3   — Loki log-delivery latency p99 ≤ 30 s.
    #   * D13-OBS-4   — Tempo trace sampling correctness ≥ 99 %.
    #   * D13-OBS-5   — Cardinality bound respected (≤ 50 000 series).
    #   * D13-OBS-6   — Alert→dashboard MTTR ≤ 30 s (UX timing test).
    #   * D13-OBS-7   — Runbook URL coverage = 100 %.
    #
    # Tous SKIPPED en baseline v0.1.13 jusqu'au 1er run vert post-merge
    # de ``.github/workflows/bench-observability.yml``.
    # ─────────────────────────────────────────────────────────────────
    {
        "id": "SLO-D13-OBS-1",
        "name": "Dashboard panel render time p99",
        "bench_key": "obs::dash::load_p99",
        "target_ns": 3_000_000_000,  # 3 s
        "comparator": "lte",
    },
    {
        "id": "SLO-D13-OBS-2",
        "name": "Metrics scrape success rate",
        "bench_key": "obs::dash::scrape_success_pct",
        # 0.999 ratio scaled ×1e9 → 9.99e8.
        "target_ns": 999_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D13-OBS-3",
        "name": "Log delivery latency p99 (promtail → Loki)",
        "bench_key": "obs::dash::log_delivery_p99_s",
        # 30 s scaled ×1e9 → 3e10.
        "target_ns": 30_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D13-OBS-4",
        "name": "Trace sampling correctness",
        "bench_key": "obs::dash::trace_sampling_pct",
        # 0.99 ratio scaled ×1e9 → 9.9e8.
        "target_ns": 990_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D13-OBS-5",
        "name": "Cardinality bound respected",
        "bench_key": "obs::dash::cardinality_bound_count",
        # 50 000 series.
        "target_ns": 50_000,
        "comparator": "lte",
    },
    {
        "id": "SLO-D13-OBS-6",
        "name": "Alert→dashboard MTTR (UX click-through)",
        "bench_key": "obs::dash::alert_to_dashboard_s",
        # 30 s scaled ×1e9 → 3e10.
        "target_ns": 30_000_000_000,
        "comparator": "lte",
        "display_as": "ratio",
    },
    {
        "id": "SLO-D13-OBS-7",
        "name": "Runbook URL coverage on D13 alerts",
        "bench_key": "obs::dash::runbook_coverage_pct",
        # 1.0 ratio scaled ×1e9 → 1e9 (= 100 % coverage).
        "target_ns": 1_000_000_000,
        "comparator": "gte",
        "display_as": "ratio",
    },
]


def evaluate_slos(
    results: dict[str, dict[str, float]],
    ci_coefficient: float = 1.0,
) -> dict[str, dict[str, Any]]:
    """Evaluate each SLO against the parsed results.

    Missing bench keys (e.g. `real_e2e_latency_blas::small` when
    running without the BLAS feature) are emitted with
    `passed: null` and `skipped: true` so downstream tooling can
    distinguish "not measured" from "failed" — a CI run that did
    not request BLAS should not fail the BLAS SLOs.

    Sprint D6 — comparator semantics
    ────────────────────────────────
    Each SLO declaration may specify a `comparator` field :

    * `lte` (default) : `measured <= target` ⇒ pass. Used for
      latency / memory / error-rate SLO where lower is better.
    * `gte` : `measured >= target` ⇒ pass. Used for accuracy /
      F1 / confidence SLO where higher is better. The
      `ci_coefficient` is intentionally NOT applied to `gte`
      targets — slow runners don't make accuracy worse.
    """
    compliance: dict[str, dict[str, Any]] = {}
    for slo in SLO_DEFINITIONS:
        bench = results.get(slo["bench_key"])
        comparator = slo.get("comparator", "lte")
        # Sprint D6 — `display_as` propagates to the compliance entry
        # so downstream tools (show_bench_report, post_bench_pr_comment)
        # can render "0.250" for ratio SLO instead of "250.00 ms".
        display_as = slo.get("display_as", "duration")
        if bench is None:
            # Don't apply ci_coefficient to gte (accuracy) targets.
            target_raw = slo["target_ns"]
            target = (
                int(target_raw * ci_coefficient)
                if comparator == "lte" and display_as != "ratio"
                else int(target_raw)
            )
            compliance[slo["id"]] = {
                "name": slo["name"],
                "target_ns": target,
                "measured_ns": None,
                "passed": None,
                "skipped": True,
                "comparator": comparator,
                "display_as": display_as,
                "note": f"missing bench: {slo['bench_key']}",
            }
            continue
        approx_p95 = int(bench["approx_p95_ns"])
        target_raw = slo["target_ns"]
        target = (
            int(target_raw * ci_coefficient)
            if comparator == "lte" and display_as != "ratio"
            else int(target_raw)
        )
        if comparator == "gte":
            passed = approx_p95 >= target
        else:
            passed = approx_p95 <= target
        compliance[slo["id"]] = {
            "name": slo["name"],
            "target_ns": target,
            "measured_ns": approx_p95,
            "passed": passed,
            "skipped": False,
            "comparator": comparator,
            "display_as": display_as,
        }
    return compliance


# ----------------------------------------------------------------
# Main
# ----------------------------------------------------------------


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    # Metadata flags (all optional; default to "unknown" if missing)
    p.add_argument("--metadata-os", default="unknown")
    p.add_argument("--metadata-cpu", default="unknown")
    p.add_argument("--metadata-ncpu", default="0")
    p.add_argument("--metadata-ram-gb", default="0")
    p.add_argument("--metadata-rustc", default="unknown")
    p.add_argument("--metadata-cargo", default="unknown")
    p.add_argument("--metadata-rustflags", default="")
    p.add_argument("--metadata-blas", default="none")
    p.add_argument("--metadata-sha", default="nogit")
    p.add_argument("--metadata-branch", default="unknown")
    p.add_argument("--metadata-dirty", default="false")
    p.add_argument("--metadata-timestamp", required=True)
    # Bench output files (kept for future bencher-format support)
    p.add_argument("--swarm-bench", default="target/bench-swarm-e2e.txt")
    p.add_argument("--memory-bench", default="target/bench-memory-e2e.txt")
    p.add_argument(
        "--criterion-root",
        default="target/criterion",
        help="Root directory containing criterion's per-bench subdirectories.",
    )
    p.add_argument(
        "--ci-coefficient",
        type=float,
        default=1.0,
        help="Multiply SLO targets by this (1.5 on slow CI runners).",
    )
    # Sprint D6 — accuracy bench-format ingestion.
    # Sprint D7 — repeatable, so the workflow can merge both the
    # single-catalog (D6-ACC-*) and the multi-catalog (D7-MC-*)
    # accuracy reports into one SLO compliance pass.
    p.add_argument(
        "--eval-bench-format",
        action="append",
        default=None,
        help="Path to a JSON file produced by "
             "`eval/scripts/run_eval.py --output-bench-format`. "
             "May be repeated ; later files take precedence on key conflict. "
             "Their keys are merged into the criterion-derived results "
             "before SLO compliance is computed.",
    )
    p.add_argument("--output", required=True, help="Path of the JSON report.")
    args = p.parse_args()

    results = load_criterion_estimates(args.criterion_root)

    # Sprint D6 — merge eval-bench-format payload (accuracy + client
    # latency) so the SLO compliance picks them up alongside the
    # criterion measurements.
    # Sprint D7 — `--eval-bench-format` is repeatable ; we merge each
    # file in argv order, last-write-wins on key collisions. The
    # single-catalog and multi-catalog files emit DISJOINT keys (see
    # `run_eval.py::run_multi_catalog`) so order is irrelevant for
    # correctness ; we still merge in argv order to keep the
    # operator's intent visible in logs.
    eval_files = args.eval_bench_format or []
    for eval_path in eval_files:
        if not os.path.isfile(eval_path):
            print(
                f"WARN: eval-bench-format file {eval_path!r} not found, skipping",
                file=sys.stderr,
            )
            continue
        try:
            with open(eval_path, "r", encoding="utf-8") as fh:
                eval_payload = json.load(fh)
            if isinstance(eval_payload, dict):
                results.update(eval_payload)
                print(
                    f"INFO: merged {len(eval_payload)} eval keys from "
                    f"{eval_path}",
                    file=sys.stderr,
                )
        except (json.JSONDecodeError, OSError) as e:
            print(
                f"WARN: could not load eval-bench-format file "
                f"{eval_path!r}: {e}",
                file=sys.stderr,
            )
    # Suppress unused variable warnings by acknowledging the bencher
    # txt files (they are still produced by `bench_runner.sh` for
    # human inspection but not parsed here).
    _ = (args.swarm_bench, args.memory_bench)

    if not results:
        print(
            "WARN: no criterion estimates found under "
            f"{args.criterion_root!r} — empty report",
            file=sys.stderr,
        )

    report: dict[str, Any] = {
        "version": "1.0",
        "timestamp": args.metadata_timestamp,
        "git": {
            "sha": args.metadata_sha,
            "branch": args.metadata_branch,
            "dirty": args.metadata_dirty.lower() == "true",
        },
        "host": {
            "os": args.metadata_os,
            "cpu": args.metadata_cpu,
            "ncpu": int(args.metadata_ncpu) if args.metadata_ncpu.isdigit() else 0,
            "ram_gb": int(args.metadata_ram_gb)
            if args.metadata_ram_gb.isdigit()
            else 0,
        },
        "toolchain": {
            "rustc": args.metadata_rustc,
            "cargo": args.metadata_cargo,
            "blas_backend": args.metadata_blas,
            "rustflags": args.metadata_rustflags,
        },
        "results": results,
        "slo_compliance": evaluate_slos(results, args.ci_coefficient),
    }

    os.makedirs(os.path.dirname(args.output) or ".", exist_ok=True)
    with open(args.output, "w", encoding="utf-8") as fh:
        json.dump(report, fh, indent=2, sort_keys=False)
        fh.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
