"""Tests for ``bench/load/parse_ghz_report.py`` — Sprint D9.3.

Pinned invariants:
  1. Bench-format keys for load tests are disjoint from accuracy.
  2. Scenario labelling falls back gracefully when metadata absent.
  3. Latency percentile extraction tolerates missing pXX entries.
  4. Throughput formula = count / duration_s (validated against
     hand-rolled fixtures).
  5. Error ratio formula = 1 - OK_count / total.
  6. Spike-recovery is None when ``details`` array missing
     (graceful SKIP rather than hard fail).
  7. SLO evaluation respects the ``gte`` vs ``lte`` comparator flip.
  8. CLI emits both bench-format and markdown summary, exit code
     is 0 iff every SLO PASS.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest


# Load the script as a module — no PYTHONPATH manipulation needed.
_REPO = Path(__file__).resolve().parents[3]
_PARSER_PATH = _REPO / "bench" / "load" / "parse_ghz_report.py"
_spec = importlib.util.spec_from_file_location("parse_ghz_report", _PARSER_PATH)
assert _spec and _spec.loader
parse_ghz_report = importlib.util.module_from_spec(_spec)
sys.modules["parse_ghz_report"] = parse_ghz_report
_spec.loader.exec_module(parse_ghz_report)


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────

def _make_ghz_report(
    *,
    scenario: str = "sustained",
    count: int = 1000,
    duration_ns: int = 5_000_000_000,  # 5 s
    p50_ns: int = 200_000_000,
    p95_ns: int = 2_500_000_000,
    p99_ns: int = 4_900_000_000,
    ok_count: int | None = None,
    extra: dict | None = None,
) -> dict:
    """Synthesise a ghz JSON report shaped like ghz ≥ 0.117."""
    if ok_count is None:
        ok_count = count
    report = {
        "count": count,
        "total": duration_ns,
        "options": {
            "concurrency": 4 if scenario == "sustained" else 16,
            "metadata": {"x-load-test-scenario": scenario},
        },
        "latencyDistribution": [
            {"percentage": 50, "latency": p50_ns},
            {"percentage": 95, "latency": p95_ns},
            {"percentage": 99, "latency": p99_ns},
        ],
        "statusCodeDistribution": {"OK": ok_count},
    }
    if scenario == "spike":
        report["options"]["loadSchedule"] = "step"
    if extra:
        report.update(extra)
    return report


# ─────────────────────────────────────────────────────────────────────
# 1. Disjoint-key invariant
# ─────────────────────────────────────────────────────────────────────

def test_load_keys_are_disjoint_from_accuracy_namespace():
    bench = parse_ghz_report._to_bench_format(
        {"sustained": {"p99_ns": 4_000_000_000, "throughput_rps": 9.5,
                       "error_ratio": 0.0, "count": 100}},
        git_sha="abc", spike_recovery_s=None,
    )
    # No key should sneak into eval_accuracy:: or eval_latency::.
    # The bench output is now a FLAT mapping (no 'results' envelope).
    parse_ghz_report.assert_disjoint_keys(bench)
    for key in bench:
        assert not key.startswith("eval_accuracy::")
        assert not key.startswith("eval_latency::")
        assert key.startswith("load_")


def test_disjoint_keys_raises_when_namespace_collides():
    bench = {"eval_accuracy::top1": {"median_ns": 0.9}}
    with pytest.raises(ValueError, match="accuracy-namespace"):
        parse_ghz_report.assert_disjoint_keys(bench)


# ─────────────────────────────────────────────────────────────────────
# 2. Scenario labelling
# ─────────────────────────────────────────────────────────────────────

def test_scenario_label_from_metadata_takes_precedence():
    report = _make_ghz_report(scenario="stress")
    # Even with concurrency=16, the explicit metadata wins.
    assert parse_ghz_report._ghz_scenario_label(report) == "stress"


def test_scenario_label_fallback_uses_loadschedule_for_spike():
    report = {
        "options": {"concurrency": 16, "loadSchedule": "step"},
    }
    assert parse_ghz_report._ghz_scenario_label(report) == "spike"


def test_scenario_label_fallback_uses_concurrency_for_stress():
    report = {"options": {"concurrency": 16}}
    assert parse_ghz_report._ghz_scenario_label(report) == "stress"


def test_scenario_label_fallback_smoke_when_concurrency_low():
    report = {"options": {"concurrency": 1}}
    assert parse_ghz_report._ghz_scenario_label(report) == "smoke"


# ─────────────────────────────────────────────────────────────────────
# 3. Percentile extraction robustness
# ─────────────────────────────────────────────────────────────────────

def test_p99_extraction_returns_none_if_distribution_missing():
    report = {"count": 100}
    assert parse_ghz_report._ghz_p_ns(report, 99) is None


def test_p99_extraction_returns_none_if_target_pct_missing():
    report = {"latencyDistribution": [{"percentage": 50, "latency": 100}]}
    assert parse_ghz_report._ghz_p_ns(report, 99) is None


def test_p99_extraction_returns_value_when_present():
    report = _make_ghz_report(p99_ns=3_141_592_653)
    assert parse_ghz_report._ghz_p_ns(report, 99) == 3_141_592_653.0


# ─────────────────────────────────────────────────────────────────────
# 4. Throughput formula
# ─────────────────────────────────────────────────────────────────────

def test_throughput_count_over_duration_seconds():
    report = _make_ghz_report(count=1000, duration_ns=10_000_000_000)
    # 1000 requests over 10 s = 100 RPS
    assert parse_ghz_report._ghz_throughput_rps(report) == pytest.approx(100.0)


def test_throughput_zero_when_count_zero():
    report = _make_ghz_report(count=0, duration_ns=5_000_000_000)
    assert parse_ghz_report._ghz_throughput_rps(report) == 0.0


def test_throughput_zero_when_duration_zero():
    report = _make_ghz_report(count=10, duration_ns=0)
    assert parse_ghz_report._ghz_throughput_rps(report) == 0.0


# ─────────────────────────────────────────────────────────────────────
# 5. Error ratio formula
# ─────────────────────────────────────────────────────────────────────

def test_error_ratio_zero_on_all_ok():
    report = _make_ghz_report(count=1000, ok_count=1000)
    assert parse_ghz_report._ghz_error_ratio(report) == 0.0


def test_error_ratio_one_on_all_failed():
    report = _make_ghz_report(count=1000, ok_count=0)
    assert parse_ghz_report._ghz_error_ratio(report) == 1.0


def test_error_ratio_partial():
    report = _make_ghz_report(count=1000, ok_count=990)
    # 1 - 990/1000 = 0.01
    assert parse_ghz_report._ghz_error_ratio(report) == pytest.approx(0.01)


def test_error_ratio_one_when_count_zero():
    # Defensive — ghz can emit count=0 for a failed config; treat as 100% error.
    report = {"count": 0, "statusCodeDistribution": {}}
    assert parse_ghz_report._ghz_error_ratio(report) == 1.0


# ─────────────────────────────────────────────────────────────────────
# 6. Spike recovery
# ─────────────────────────────────────────────────────────────────────

def test_spike_recovery_returns_none_when_details_absent():
    report = _make_ghz_report(scenario="spike")
    # No `details` key → cannot compute recovery.
    assert parse_ghz_report._ghz_spike_recovery_seconds(
        report, p99_budget_ns=5e9
    ) is None


def _iso(t):
    return t.isoformat().replace("+00:00", "Z")


def test_spike_recovery_finds_window_under_budget():
    """Synthesise a sample stream where p99 is high during 30-60 s
    (spike) then drops to 1 s for the rest. The detector with the
    new 10 s default window should report a recovery time slightly
    above 30 s (the first 10 s window with all-recovered samples
    starts at t=60 → recovery = 60-30 = 30 s)."""
    from datetime import datetime, timedelta, timezone

    t0 = datetime(2026, 1, 1, tzinfo=timezone.utc)

    samples = []
    # 0-30 s: warm baseline, p99 ≈ 1 s.
    for i in range(0, 30):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 1_000_000_000, "status": "OK"})
    # 30-60 s: spike, p99 ≈ 8 s (over budget).
    for i in range(30, 60):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 8_000_000_000, "status": "OK"})
    # 60-180 s: fully recovered, p99 ≈ 1 s.
    for i in range(60, 180):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 1_000_000_000, "status": "OK"})

    report = {
        "options": {"loadSchedule": "step", "concurrency": 16,
                    "metadata": {"x-load-test-scenario": "spike"}},
        "details": samples,
    }
    rec = parse_ghz_report._ghz_spike_recovery_seconds(
        report, p99_budget_ns=5_000_000_000,
    )
    # The first 10 s window starting strictly after t=30 that
    # contains only post-spike samples is at t=60 (window 60-70).
    # Recovery time = 60 - 30 = 30 s.
    assert rec is not None
    assert 25.0 <= rec <= 35.0, f"expected ~30s recovery, got {rec}"


def test_spike_recovery_does_NOT_return_zero_at_spike_onset():
    """Audit fix L21 — the parser previously returned 0 at t=30
    because the 60 s window starting at t=30 reached far enough
    that p99 dropped under budget. With the new 10 s default and
    earliest-window-start = spike_start_s + 1, this is impossible."""
    from datetime import datetime, timedelta, timezone

    t0 = datetime(2026, 1, 1, tzinfo=timezone.utc)
    samples = []
    # 0-30 s warm
    for i in range(0, 30):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 1_000_000_000, "status": "OK"})
    # 30-90 s spike (60 s of high latency).
    for i in range(30, 90):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 8_000_000_000, "status": "OK"})
    # 90-150 s recovery
    for i in range(90, 150):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 1_000_000_000, "status": "OK"})

    report = {
        "options": {"loadSchedule": "step", "concurrency": 16,
                    "metadata": {"x-load-test-scenario": "spike"}},
        "details": samples,
    }
    rec = parse_ghz_report._ghz_spike_recovery_seconds(
        report, p99_budget_ns=5_000_000_000,
    )
    # Recovery is at t=90 (first 10 s window after spike ends).
    # rec = 90 - 30 = 60 s. Must NOT be 0 (which was the L21 bug).
    assert rec is not None
    assert rec > 30.0, f"recovery {rec} suggests L21 regression — must be > 30s"


def test_spike_recovery_returns_full_window_if_never_recovered():
    from datetime import datetime, timedelta, timezone

    t0 = datetime(2026, 1, 1, tzinfo=timezone.utc)
    samples = []
    # All samples over budget — system never recovers.
    for i in range(0, 120):
        samples.append({"timestamp": _iso(t0 + timedelta(seconds=i)),
                        "latency": 8_000_000_000, "status": "OK"})
    report = {
        "options": {"loadSchedule": "step", "concurrency": 16,
                    "metadata": {"x-load-test-scenario": "spike"}},
        "details": samples,
    }
    rec = parse_ghz_report._ghz_spike_recovery_seconds(
        report, p99_budget_ns=5_000_000_000,
    )
    # Failure mode: returns the duration past spike_start so the
    # SLO comparison clearly fails.
    assert rec is not None
    assert rec > 30.0


# ─────────────────────────────────────────────────────────────────────
# 7. SLO evaluation comparator
# ─────────────────────────────────────────────────────────────────────

def test_slo_d9_load_1_pass_when_p99_under_budget():
    # Both flat and wrapped shapes are accepted by _eval_slos.
    bench = {"load_sustained::p99_ns": {"median_ns": 4_500_000_000}}
    overall, rows = parse_ghz_report._eval_slos(
        bench, p99_budget_ms=5000, error_budget_ratio=0.01,
        throughput_min_rps=5, recovery_budget_s=30,
    )
    row = next(r for r in rows if r["id"] == "D9-LOAD-1")
    assert row["status"] == "PASS"
    # Other SLOs SKIP because their bench keys are absent.


def test_slo_d9_load_1_fail_when_p99_over_budget():
    bench = {"load_sustained::p99_ns": {"median_ns": 6_000_000_000}}
    overall, rows = parse_ghz_report._eval_slos(
        bench, p99_budget_ms=5000, error_budget_ratio=0.01,
        throughput_min_rps=5, recovery_budget_s=30,
    )
    row = next(r for r in rows if r["id"] == "D9-LOAD-1")
    assert row["status"] == "FAIL"
    assert overall is False


def test_slo_d9_load_3_pass_picks_max_of_sustained_and_stress():
    """Throughput SLO should use whichever scenario achieved more
    RPS — sustained validates the steady-state, stress validates the
    saturation ceiling."""
    bench = {
        "load_sustained::throughput_rps": {"median_ns": 1.6 * 1e9},  # below
        "load_stress::throughput_rps": {"median_ns": 8.0 * 1e9},     # above
    }
    overall, rows = parse_ghz_report._eval_slos(
        bench, p99_budget_ms=5000, error_budget_ratio=0.01,
        throughput_min_rps=5, recovery_budget_s=30,
    )
    row = next(r for r in rows if r["id"] == "D9-LOAD-3")
    assert row["status"] == "PASS", row


def test_slo_skip_when_metric_missing():
    bench: dict = {}
    _, rows = parse_ghz_report._eval_slos(
        bench, p99_budget_ms=5000, error_budget_ratio=0.01,
        throughput_min_rps=5, recovery_budget_s=30,
    )
    assert all(r["status"] == "SKIP" for r in rows), rows


def test_eval_slos_accepts_wrapped_results_envelope():
    """Backward compat: aggregator can pass the report-format too."""
    bench = {"results": {"load_sustained::p99_ns": {"median_ns": 4_500_000_000}}}
    _, rows = parse_ghz_report._eval_slos(
        bench, p99_budget_ms=5000, error_budget_ratio=0.01,
        throughput_min_rps=5, recovery_budget_s=30,
    )
    row = next(r for r in rows if r["id"] == "D9-LOAD-1")
    assert row["status"] == "PASS"


# ─────────────────────────────────────────────────────────────────────
# Audit fix L10/X3 — _format_value unit tags
# ─────────────────────────────────────────────────────────────────────

def test_format_value_ns_renders_milliseconds():
    out = parse_ghz_report._format_value(2_500_000_000.0, "ns")
    assert out == "2500.0 ms"


def test_format_value_ratio_renders_percentage():
    # 0.01 ratio stored as 1e7 (×1e9 scaling) → "1.000 %"
    out = parse_ghz_report._format_value(1e7, "ratio")
    assert out == "1.000 %"


def test_format_value_rps_renders_rps_unit_NOT_percent():
    """Audit fix L10 — throughput must NOT be rendered as
    `500.000 %` (the bug pre-fix). Must read as `5.00 RPS`."""
    out = parse_ghz_report._format_value(5e9, "rps")
    assert "RPS" in out
    assert "%" not in out
    assert out == "5.00 RPS"


def test_format_value_seconds_renders_seconds():
    out = parse_ghz_report._format_value(30.0 * 1e9, "s")
    assert out == "30.0 s"


def test_format_value_none_returns_em_dash():
    assert parse_ghz_report._format_value(None, "ns") == "—"


# ─────────────────────────────────────────────────────────────────────
# 8. End-to-end CLI
# ─────────────────────────────────────────────────────────────────────

def test_cli_writes_bench_format_and_summary(tmp_path: Path):
    report = _make_ghz_report(
        scenario="sustained",
        count=3000, duration_ns=300_000_000_000,  # 300 s
        p99_ns=4_500_000_000, ok_count=3000,
    )
    report_path = tmp_path / "ghz_sustained.json"
    report_path.write_text(json.dumps(report))

    bench_out = tmp_path / "load_test_bench.json"
    summary_out = tmp_path / "summary.md"
    summary_out.write_text("# header\n")  # parser appends.

    rc = parse_ghz_report.main([
        "--reports", str(report_path),
        "--bench-out", str(bench_out),
        "--summary-out", str(summary_out),
        "--p99-budget-ms", "5000",
        "--error-budget-ratio", "0.01",
        "--throughput-min-rps", "5.0",
        "--recovery-budget-s", "30",
    ])
    assert rc == 0, "expected SLOs to pass on this fixture"

    bench = json.loads(bench_out.read_text())
    # Output is a FLAT eval-bench-format mapping (no `results` envelope).
    assert "load_sustained::p99_ns" in bench
    assert "load_sustained::throughput_rps" in bench
    # Each entry has the canonical bench fields.
    entry = bench["load_sustained::p99_ns"]
    for fld in ("mean_ns", "median_ns", "std_dev_ns", "mad_ns", "approx_p95_ns"):
        assert fld in entry, fld

    summary = summary_out.read_text()
    assert "# header" in summary  # original header preserved
    assert "SLO compliance — D9-LOAD-*" in summary
    assert "Sustained p99 latency" in summary


def test_cli_returns_1_when_slo_fails(tmp_path: Path):
    # Force a p99 over budget.
    report = _make_ghz_report(
        scenario="sustained",
        p99_ns=10_000_000_000,  # 10 s ≫ 5 s budget
    )
    report_path = tmp_path / "ghz_sustained.json"
    report_path.write_text(json.dumps(report))

    bench_out = tmp_path / "bench.json"
    summary_out = tmp_path / "summary.md"
    summary_out.write_text("")

    rc = parse_ghz_report.main([
        "--reports", str(report_path),
        "--bench-out", str(bench_out),
        "--summary-out", str(summary_out),
        "--p99-budget-ms", "5000",
        "--error-budget-ratio", "0.01",
        "--throughput-min-rps", "5.0",
        "--recovery-budget-s", "30",
    ])
    assert rc == 1


def test_cli_returns_2_when_no_usable_reports(tmp_path: Path):
    empty = tmp_path / "empty.json"
    empty.touch()
    bench_out = tmp_path / "bench.json"
    summary_out = tmp_path / "summary.md"
    summary_out.write_text("")
    rc = parse_ghz_report.main([
        "--reports", str(empty),
        "--bench-out", str(bench_out),
        "--summary-out", str(summary_out),
    ])
    assert rc == 2
