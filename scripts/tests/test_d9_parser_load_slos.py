"""Sprint D9 anti-regression tests for the D9-LOAD-* and D9-SOAK-1
SLO definitions in ``scripts/parse_criterion_report.py``.

Pinned invariants:
  1. Five new SLO IDs exist (D9-LOAD-1..4 + D9-SOAK-1).
  2. Each new SLO has a non-empty bench_key, target_ns > 0, and a
     valid comparator (``lte`` or ``gte``).
  3. All bench_keys are disjoint from the D4/D6/D7/D8 namespaces.
  4. End-to-end parser merge: feed a synthetic load-test bench-format
     file → SLO_compliance contains the 4 LOAD entries with PASS.
  5. End-to-end parser merge: feed a synthetic soak bench-format file
     → SLO_compliance contains the SOAK entry with PASS.
  6. SKIPPED is correctly returned when the bench_key is missing
     (avoids the "silent gate gap" regression class).

Run ::

    pytest scripts/tests/test_d9_parser_load_slos.py
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest


_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parents[1]
_PARSER = _ROOT / "scripts" / "parse_criterion_report.py"

_spec = importlib.util.spec_from_file_location(
    "parse_criterion_report", _PARSER
)
assert _spec and _spec.loader
parse_criterion_report = importlib.util.module_from_spec(_spec)
sys.modules["parse_criterion_report"] = parse_criterion_report
_spec.loader.exec_module(parse_criterion_report)


# ─────────────────────────────────────────────────────────────────────
# 1-3. SLO_DEFINITIONS structural invariants
# ─────────────────────────────────────────────────────────────────────

def test_d9_slos_present():
    ids = {slo["id"] for slo in parse_criterion_report.SLO_DEFINITIONS}
    expected = {
        "SLO-D9-LOAD-1", "SLO-D9-LOAD-2",
        "SLO-D9-LOAD-3", "SLO-D9-LOAD-4",
        "SLO-D9-SOAK-1",
    }
    missing = expected - ids
    assert not missing, f"missing D9 SLOs: {missing}"


def test_d9_slos_have_well_formed_fields():
    d9 = [s for s in parse_criterion_report.SLO_DEFINITIONS
          if s["id"].startswith("SLO-D9-")]
    assert len(d9) == 5, f"expected 5 D9 SLOs, got {len(d9)}: {[s['id'] for s in d9]}"
    for slo in d9:
        assert slo["bench_key"], slo
        assert slo["target_ns"] > 0, slo
        assert slo["comparator"] in ("lte", "gte"), slo


def test_d9_load_keys_disjoint_from_eval_namespaces():
    d9_keys = {
        s["bench_key"] for s in parse_criterion_report.SLO_DEFINITIONS
        if s["id"].startswith("SLO-D9-")
    }
    for key in d9_keys:
        assert not key.startswith("eval_accuracy::"), key
        assert not key.startswith("eval_latency::"), key
        # Load keys live in load_<scenario>::, soak in soak::
        assert key.startswith("load_") or key.startswith("soak::"), key


def test_d9_load_keys_disjoint_from_d4_d6_d7_d8():
    """Anti-collision invariant — same as D8 added when introducing
    hybrid_top1 vs top1 — applied to D9."""
    d9_keys = {
        s["bench_key"] for s in parse_criterion_report.SLO_DEFINITIONS
        if s["id"].startswith("SLO-D9-")
    }
    other_keys = {
        s["bench_key"] for s in parse_criterion_report.SLO_DEFINITIONS
        if not s["id"].startswith("SLO-D9-")
    }
    overlap = d9_keys & other_keys
    assert not overlap, f"D9 bench keys collide with earlier sprints: {overlap}"


# ─────────────────────────────────────────────────────────────────────
# 4-5. End-to-end parser ingestion
# ─────────────────────────────────────────────────────────────────────

def _make_dummy_criterion_root(root: Path) -> None:
    grp = root / "dummy" / "id" / "new"
    grp.mkdir(parents=True, exist_ok=True)
    (grp / "estimates.json").write_text(json.dumps({
        "mean":    {"point_estimate": 1_000_000.0},
        "median":  {"point_estimate": 1_000_000.0},
        "std_dev": {"point_estimate": 0.0},
        "median_abs_dev": {"point_estimate": 0.0},
    }))


def _bench_entry(value_ns: float, jitter_ns: float = 0.0) -> dict[str, float]:
    """Mirrors what `scripts/parse_criterion_report.py` expects from a
    flat eval-bench-format file.
    """
    v = float(value_ns)
    return {
        "mean_ns": v,
        "median_ns": v,
        "std_dev_ns": float(jitter_ns),
        "mad_ns": float(jitter_ns),
        "approx_p95_ns": v + float(jitter_ns),
        "jitter_ns": float(jitter_ns),
    }


def _make_load_bench(path: Path, *, p99_ms: float = 4500,
                     err_ratio: float = 0.005,
                     stress_rps: float = 8.0,
                     recovery_s: float = 20.0) -> None:
    """Emit a **flat** eval-bench-format payload (no `results`
    envelope). The aggregator does ``results.update(json.load(f))``
    directly onto the file.
    """
    bench = {
        "load_sustained::p99_ns":         _bench_entry(p99_ms * 1e6),
        "load_sustained::error_ratio":    _bench_entry(err_ratio * 1e9),
        "load_stress::throughput_rps":    _bench_entry(stress_rps * 1e9),
        "load_spike::recovery_seconds":   _bench_entry(recovery_s * 1e9),
    }
    path.write_text(json.dumps(bench))


def _make_soak_bench(path: Path, *, slope_bps: float = 100.0) -> None:
    bench = {
        "soak::rss_slope_bps": _bench_entry(slope_bps * 1e9, 50 * 1e9),
        "soak::sample_count":  _bench_entry(1440.0),
        "soak::leak_status":   _bench_entry(0.0),
    }
    path.write_text(json.dumps(bench))


def _run_parser(criterion_root: Path, eval_files: list[Path],
                output: Path) -> dict:
    cmd: list[str] = [
        sys.executable, str(_PARSER),
        "--criterion-root", str(criterion_root),
        "--metadata-timestamp", "20260427T000000Z",
        "--output", str(output),
    ]
    for ev in eval_files:
        cmd.extend(["--eval-bench-format", str(ev)])
    res = subprocess.run(cmd, capture_output=True, text=True)
    assert res.returncode == 0, (
        f"parser failed (rc={res.returncode}) "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )
    return json.loads(output.read_text())


def test_e2e_load_bench_passes_load_slos(tmp_path: Path):
    crit = tmp_path / "criterion"
    _make_dummy_criterion_root(crit)
    load_bench = tmp_path / "load_test_bench.json"
    _make_load_bench(load_bench, p99_ms=4500, err_ratio=0.005,
                     stress_rps=8.0, recovery_s=20.0)
    out = tmp_path / "report.json"
    report = _run_parser(crit, [load_bench], out)

    compl = report["slo_compliance"]
    for slo_id in ("SLO-D9-LOAD-1", "SLO-D9-LOAD-2",
                   "SLO-D9-LOAD-3", "SLO-D9-LOAD-4"):
        assert slo_id in compl, f"missing {slo_id} in compliance map"
        assert compl[slo_id]["passed"] is True, (slo_id, compl[slo_id])
        assert compl[slo_id]["skipped"] is False


def test_e2e_load_bench_fails_on_p99_breach(tmp_path: Path):
    crit = tmp_path / "criterion"
    _make_dummy_criterion_root(crit)
    load_bench = tmp_path / "load_test_bench.json"
    _make_load_bench(load_bench, p99_ms=10000)  # 10 s ≫ 5 s budget
    out = tmp_path / "report.json"
    report = _run_parser(crit, [load_bench], out)
    assert report["slo_compliance"]["SLO-D9-LOAD-1"]["passed"] is False


def test_e2e_soak_bench_passes_when_slope_low(tmp_path: Path):
    crit = tmp_path / "criterion"
    _make_dummy_criterion_root(crit)
    soak_bench = tmp_path / "soak_bench.json"
    _make_soak_bench(soak_bench, slope_bps=100.0)
    out = tmp_path / "report.json"
    report = _run_parser(crit, [soak_bench], out)
    assert report["slo_compliance"]["SLO-D9-SOAK-1"]["passed"] is True


def test_e2e_soak_bench_fails_when_slope_above_threshold(tmp_path: Path):
    crit = tmp_path / "criterion"
    _make_dummy_criterion_root(crit)
    soak_bench = tmp_path / "soak_bench.json"
    _make_soak_bench(soak_bench, slope_bps=2000.0)  # ≫ 870 B/s
    out = tmp_path / "report.json"
    report = _run_parser(crit, [soak_bench], out)
    assert report["slo_compliance"]["SLO-D9-SOAK-1"]["passed"] is False


# ─────────────────────────────────────────────────────────────────────
# 6. SKIPPED behavior
# ─────────────────────────────────────────────────────────────────────

def test_d9_slos_skipped_when_load_bench_absent(tmp_path: Path):
    """No --eval-bench-format → all D9 SLOs SKIPPED, not FAILED."""
    crit = tmp_path / "criterion"
    _make_dummy_criterion_root(crit)
    out = tmp_path / "report.json"
    report = _run_parser(crit, [], out)
    compl = report["slo_compliance"]
    for slo_id in ("SLO-D9-LOAD-1", "SLO-D9-LOAD-2",
                   "SLO-D9-LOAD-3", "SLO-D9-LOAD-4",
                   "SLO-D9-SOAK-1"):
        # SKIP semantics: passed=null, skipped=true (the same shape
        # used by D8-RD-* when partner data isn't yet attached).
        assert compl[slo_id]["skipped"] is True, (slo_id, compl[slo_id])


# ─────────────────────────────────────────────────────────────────────
# Order-independence (recovery from a permutation regression)
# ─────────────────────────────────────────────────────────────────────

def test_d9_slos_order_independent():
    """If `SLO_DEFINITIONS` is reshuffled, the IDs and bench_keys
    must stay coherent. This test is cheap insurance against
    accidental edits that (re)introduce duplicate IDs."""
    ids = [s["id"] for s in parse_criterion_report.SLO_DEFINITIONS
           if s["id"].startswith("SLO-D9-")]
    assert len(ids) == len(set(ids)), f"duplicate D9 SLO IDs: {ids}"

    bench_keys = [s["bench_key"] for s in parse_criterion_report.SLO_DEFINITIONS
                  if s["id"].startswith("SLO-D9-")]
    assert len(bench_keys) == len(set(bench_keys)), (
        f"duplicate D9 bench_keys: {bench_keys}"
    )
