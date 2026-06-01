# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-D D31 (Bundle completion)** — pin tests pour les
2 scripts bench Python livrés audit-fix A5.

- `scripts/bench/criterion_to_slo.py`
- `scripts/bench/analyze_soak.py`
"""

from __future__ import annotations

import csv
import json
import sys
import tempfile
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "bench"))

import analyze_soak    # noqa: E402
import criterion_to_slo    # noqa: E402


# ──────────────────────────────────────────────────────────────────
# criterion_to_slo tests
# ──────────────────────────────────────────────────────────────────


def test_build_slo_compliance_maps_fusion_round() -> None:
    parsed = [
        {"bench_id": "fusion_round", "median_ns": 8_500_000},
        {"bench_id": "fusion_10rounds", "median_ns": 95_000_000},
    ]
    compliance = criterion_to_slo.build_slo_compliance(parsed)
    assert "SLO-CUDA-FUSION-ROUND" in compliance
    assert compliance["SLO-CUDA-FUSION-ROUND"]["measured_ns"] == 8_500_000
    assert compliance["SLO-CUDA-FUSION-TOTAL"]["measured_ns"] == 95_000_000


def test_build_slo_compliance_computes_speedup_ratio() -> None:
    parsed = [
        {"bench_id": "fusion_cpu", "median_ns": 500_000_000},
        {"bench_id": "fusion_gpu", "median_ns": 100_000_000},
    ]
    compliance = criterion_to_slo.build_slo_compliance(parsed)
    assert "SLO-CUDA-SPEEDUP-CPU" in compliance
    assert compliance["SLO-CUDA-SPEEDUP-CPU"]["measured_ratio"] == 5.0


def test_build_slo_compliance_unknown_bench_ignored() -> None:
    parsed = [{"bench_id": "irrelevant_bench", "median_ns": 1_000_000}]
    compliance = criterion_to_slo.build_slo_compliance(parsed)
    assert "irrelevant_bench" not in compliance
    assert len(compliance) == 0


def test_run_missing_criterion_dir_returns_2(tmp_path: Path) -> None:
    rc = criterion_to_slo.run(
        criterion_dir=tmp_path / "nonexistent",
        output=tmp_path / "out.json",
    )
    assert rc == 2


def test_run_empty_criterion_produces_empty_compliance(tmp_path: Path) -> None:
    criterion = tmp_path / "criterion"
    criterion.mkdir()
    output = tmp_path / "report.json"
    rc = criterion_to_slo.run(criterion_dir=criterion, output=output)
    assert rc == 0
    parsed = json.loads(output.read_text())
    assert parsed["slo_compliance"] == {}
    assert parsed["n_benches_parsed"] == 0


def test_parse_estimate_extracts_median(tmp_path: Path) -> None:
    # Convention Criterion : target/criterion/<bench_id>/new/estimates.json
    # → parent.parent.name = <bench_id>
    bench_dir = tmp_path / "fusion_round" / "new"
    bench_dir.mkdir(parents=True)
    p = bench_dir / "estimates.json"
    p.write_text(json.dumps({"median": {"point_estimate": 12345}}))
    result = criterion_to_slo.parse_estimate(p)
    assert result == {"bench_id": "fusion_round", "median_ns": 12345}


def test_parse_estimate_returns_none_on_missing_median(tmp_path: Path) -> None:
    bench_dir = tmp_path / "anything" / "new"
    bench_dir.mkdir(parents=True)
    p = bench_dir / "estimates.json"
    p.write_text(json.dumps({"mean": {"point_estimate": 10}}))   # missing median
    result = criterion_to_slo.parse_estimate(p)
    assert result is None


# ──────────────────────────────────────────────────────────────────
# analyze_soak tests
# ──────────────────────────────────────────────────────────────────


def test_ols_slope_constant_returns_zero() -> None:
    xs = [0.0, 1.0, 2.0, 3.0]
    ys = [5.0, 5.0, 5.0, 5.0]
    assert analyze_soak.ols_slope(xs, ys) == 0.0


def test_ols_slope_linear_perfect_fit() -> None:
    # y = 2x + 1
    xs = [0.0, 1.0, 2.0, 3.0, 4.0]
    ys = [1.0, 3.0, 5.0, 7.0, 9.0]
    slope = analyze_soak.ols_slope(xs, ys)
    assert abs(slope - 2.0) < 1e-9


def test_ols_slope_empty_returns_zero() -> None:
    assert analyze_soak.ols_slope([], []) == 0.0


def test_ols_slope_single_sample_returns_zero() -> None:
    assert analyze_soak.ols_slope([1.0], [5.0]) == 0.0


def test_analyze_empty_samples() -> None:
    r = analyze_soak.analyze([])
    assert r["n_samples"] == 0
    assert r["slo_compliance"]["SLO-CUDA-SOAK-24H"]["passed"] is None
    assert r["slo_compliance"]["SLO-CUDA-SOAK-24H"]["skipped"] == "no samples"


def test_analyze_stable_rss_passes_slo() -> None:
    # 24h soak, RSS oscille autour de 1 GiB sans drift
    base_ts = 1722499200
    samples = []
    for i in range(0, 1440):    # 24h × 60 = 1440 minutes
        rss = 1_073_741_824 + (i % 10) * 1024    # noise < 10 KiB
        samples.append((base_ts + i * 60, 4500, rss))
    r = analyze_soak.analyze(samples)
    assert r["slo_compliance"]["SLO-CUDA-SOAK-24H"]["passed"] is True
    # Drift négligeable
    assert abs(r["rss_drift_bytes_per_hour"]) < 100 * 1024    # < 100 KiB/h


def test_analyze_clear_leak_fails_slo() -> None:
    # 24h avec drift 20 MiB/h (= > seuil 10 MiB/h)
    base_ts = 1722499200
    bytes_per_minute = (20 * 1024 * 1024) / 60    # 20 MiB / h = 350 KiB / min
    samples = []
    for i in range(0, 1440):
        rss = 1_073_741_824 + int(i * bytes_per_minute)
        samples.append((base_ts + i * 60, 4500, rss))
    r = analyze_soak.analyze(samples)
    assert r["slo_compliance"]["SLO-CUDA-SOAK-24H"]["passed"] is False
    # Drift ≈ 20 MiB/h
    drift_mib_h = r["rss_drift_bytes_per_hour"] / 1024 / 1024
    assert 19.5 < drift_mib_h < 20.5


def test_parse_csv_skips_comments_and_invalid_lines(tmp_path: Path) -> None:
    p = tmp_path / "soak.csv"
    p.write_text(
        "# this is a comment\n"
        "1722499200,4500,1234567890\n"
        "invalid,line,here\n"
        "1722499260,4501,1234568000\n"
    )
    samples = analyze_soak.parse_csv(p)
    assert len(samples) == 2
    assert samples[0] == (1722499200, 4500, 1234567890)


def test_threshold_slo_aligned_with_yaml() -> None:
    # Pin contractuel : le seuil Python = celui du YAML baseline.
    # 10 MiB/h = 10 × 1024 × 1024 = 10 485 760 bytes/h.
    assert analyze_soak.SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR == 10 * 1024 * 1024
    assert analyze_soak.SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR == 10_485_760


def test_main_with_csv_returns_zero_on_pass(tmp_path: Path) -> None:
    p = tmp_path / "soak.csv"
    p.write_text("1722499200,4500,1234567890\n1722503600,4501,1234567920\n")
    rc = analyze_soak.main([str(p), "--quiet"])
    assert rc == 0
