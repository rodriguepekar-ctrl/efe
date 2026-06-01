"""Tests for ``bench/soak/leak_detector.py`` — Sprint D9.4.

Pinned invariants:
  1. OLS slope on a known linear stream recovers the slope ±1 %.
  2. Slope on a flat stream is ≈ 0 (within noise).
  3. ``ols_slope_and_se`` raises on n < 3.
  4. Median-of-window smoothing returns N/window medians.
  5. ``decide`` returns FAIL only when both magnitude AND significance.
  6. ``decide`` returns INCONCLUSIVE when over threshold but |t| < 1.96.
  7. Warmup truncation drops the right rows.
  8. End-to-end: CSV with no leak → exit 0.
  9. End-to-end: CSV with a clear leak → exit 1.
 10. End-to-end: CSV with too few samples → exit 2.
 11. Bench-format emission has soak::rss_slope_bps key, jitter_ns = se×1e9.
 12. Threshold check respects amendment A3 sign convention (negative slope = PASS).
"""

from __future__ import annotations

import csv
import importlib.util
import json
import random
import sys
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[3]
_DETECTOR_PATH = _REPO / "bench" / "soak" / "leak_detector.py"
_spec = importlib.util.spec_from_file_location("leak_detector", _DETECTOR_PATH)
assert _spec and _spec.loader
leak_detector = importlib.util.module_from_spec(_spec)
sys.modules["leak_detector"] = leak_detector
_spec.loader.exec_module(leak_detector)


# ─────────────────────────────────────────────────────────────────────
# Pure math
# ─────────────────────────────────────────────────────────────────────

def test_ols_recovers_slope_on_linear_stream():
    # Synthesize y = 100 + 870 * t with no noise.
    t = [float(i * 60) for i in range(1, 1441)]   # 24 h × 1/min
    y = [100 + 870 * ti for ti in t]
    beta0, beta1, se, df = leak_detector.ols_slope_and_se(t, y)
    assert beta1 == pytest.approx(870.0, rel=1e-9)
    assert beta0 == pytest.approx(100.0, abs=1e-3)
    assert df == len(t) - 2


def test_ols_slope_zero_on_flat_stream_with_jitter():
    rnd = random.Random(42)
    t = [float(i * 60) for i in range(1, 1441)]
    # Fluctuate around 512 MiB ± 5 MB Gaussian-ish jitter.
    y = [512 * 1024 * 1024 + rnd.gauss(0, 5 * 1024 * 1024) for _ in t]
    _, beta1, se, df = leak_detector.ols_slope_and_se(t, y)
    # Slope should be much smaller than 870 B/s (no leak).
    assert abs(beta1) < 870


def test_ols_raises_on_n_below_3():
    with pytest.raises(ValueError, match="n ≥ 3"):
        leak_detector.ols_slope_and_se([0.0, 1.0], [10.0, 20.0])


def test_ols_raises_on_zero_denominator():
    # All timestamps identical → can't fit slope.
    with pytest.raises(ValueError, match="zero"):
        leak_detector.ols_slope_and_se(
            [5.0, 5.0, 5.0], [1.0, 2.0, 3.0]
        )


# ─────────────────────────────────────────────────────────────────────
# Median-of-window
# ─────────────────────────────────────────────────────────────────────

def test_median_window_size_1_is_identity():
    assert leak_detector.median_of_window([1.0, 2.0, 3.0], 1) == [1.0, 2.0, 3.0]


def test_median_window_returns_one_per_window():
    values = [10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
              20, 21, 22, 23, 24, 25, 26, 27, 28, 29]
    out = leak_detector.median_of_window(values, 10)
    # Two non-overlapping windows of 10 each.
    assert len(out) == 2
    assert out[0] == pytest.approx((14 + 15) / 2)   # median of 10..19
    assert out[1] == pytest.approx((24 + 25) / 2)   # median of 20..29


def test_median_window_drops_partial_trailing():
    values = list(range(15))   # 15 samples, window=10
    out = leak_detector.median_of_window(values, 10)
    assert len(out) == 1     # 15 // 10 = 1


# ─────────────────────────────────────────────────────────────────────
# Decision logic
# ─────────────────────────────────────────────────────────────────────

def test_decide_pass_when_slope_below_threshold():
    d = leak_detector.decide(
        slope_bps=100.0, se_bps=10.0, df=100,
        threshold_bps=870.0,
    )
    assert d["status"] == "PASS"


def test_decide_fail_when_above_threshold_and_significant():
    # slope/se = 100 / 10 = 10 ≫ 1.96
    d = leak_detector.decide(
        slope_bps=1000.0, se_bps=10.0, df=100,
        threshold_bps=870.0,
    )
    assert d["status"] == "FAIL"


def test_decide_inconclusive_when_above_threshold_but_not_significant():
    # slope/se = 1000/700 ≈ 1.43 < 1.96
    d = leak_detector.decide(
        slope_bps=1000.0, se_bps=700.0, df=100,
        threshold_bps=870.0,
    )
    assert d["status"] == "INCONCLUSIVE"


def test_decide_pass_when_negative_slope_even_if_large_magnitude():
    # Allocator releasing memory faster than acquiring → not a leak.
    d = leak_detector.decide(
        slope_bps=-2000.0, se_bps=10.0, df=100,
        threshold_bps=870.0,
    )
    assert d["status"] == "PASS"


def test_decide_inconclusive_on_df_zero():
    d = leak_detector.decide(
        slope_bps=10.0, se_bps=1.0, df=0, threshold_bps=870.0,
    )
    assert d["status"] == "INCONCLUSIVE"


# ─────────────────────────────────────────────────────────────────────
# Warmup truncation
# ─────────────────────────────────────────────────────────────────────

def test_apply_warmup_drops_samples_below_threshold():
    t = [60, 600, 1200, 1800, 2400, 3000]
    y = [100, 200, 300, 400, 500, 600]
    t2, y2 = leak_detector.apply_warmup(t, y, warmup_s=1800)
    assert t2 == [1800, 2400, 3000]
    assert y2 == [400, 500, 600]


def test_apply_warmup_zero_keeps_everything():
    t = [0, 60, 120]
    y = [1, 2, 3]
    t2, y2 = leak_detector.apply_warmup(t, y, warmup_s=0)
    assert t2 == t
    assert y2 == y


# ─────────────────────────────────────────────────────────────────────
# End-to-end CLI
# ─────────────────────────────────────────────────────────────────────

def _write_csv(path: Path, t_y_pairs: list[tuple[float, float]]) -> None:
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["t_seconds", "rss_bytes"])
        for t, y in t_y_pairs:
            w.writerow([t, y])


def _generate_no_leak_24h(rng: random.Random) -> list[tuple[float, float]]:
    """1 440 samples, baseline 512 MiB, jitter ±5 MB Gaussian."""
    out = []
    for i in range(1, 1441):
        t = i * 60.0
        y = 512 * 1024 * 1024 + rng.gauss(0, 5 * 1024 * 1024)
        out.append((t, y))
    return out


def _generate_leak_24h(slope_bps: float, rng: random.Random) -> list[tuple[float, float]]:
    """1 440 samples, baseline 512 MiB + slope*t + jitter."""
    out = []
    for i in range(1, 1441):
        t = i * 60.0
        y = 512 * 1024 * 1024 + slope_bps * t + rng.gauss(0, 1 * 1024 * 1024)
        out.append((t, y))
    return out


def test_e2e_no_leak_returns_zero(tmp_path: Path):
    rng = random.Random(123)
    csv_path = tmp_path / "no_leak.csv"
    _write_csv(csv_path, _generate_no_leak_24h(rng))
    rc = leak_detector.main([
        "--csv", str(csv_path),
        "--slope-threshold-bytes-per-sec", "870",
        "--warmup-seconds", "1800",
        "--smooth-window", "10",
        "--report-out", str(tmp_path / "report.md"),
        "--bench-out", str(tmp_path / "bench.json"),
    ])
    assert rc == 0
    report = (tmp_path / "report.md").read_text()
    assert "PASS" in report


def test_e2e_clear_leak_returns_one(tmp_path: Path):
    rng = random.Random(456)
    # 5 000 B/s leak — well above 870 threshold.
    csv_path = tmp_path / "leak.csv"
    _write_csv(csv_path, _generate_leak_24h(5000.0, rng))
    rc = leak_detector.main([
        "--csv", str(csv_path),
        "--slope-threshold-bytes-per-sec", "870",
        "--warmup-seconds", "1800",
        "--smooth-window", "10",
        "--report-out", str(tmp_path / "report.md"),
        "--bench-out", str(tmp_path / "bench.json"),
    ])
    assert rc == 1
    report = (tmp_path / "report.md").read_text()
    assert "FAIL" in report


def test_e2e_too_few_samples_returns_two(tmp_path: Path):
    csv_path = tmp_path / "tiny.csv"
    _write_csv(csv_path, [(60.0 * i, 1e9) for i in range(1, 5)])  # 4 samples
    rc = leak_detector.main([
        "--csv", str(csv_path),
        "--slope-threshold-bytes-per-sec", "870",
    ])
    assert rc == 2


# ─────────────────────────────────────────────────────────────────────
# Bench-format invariants
# ─────────────────────────────────────────────────────────────────────

def test_bench_format_keys_and_units(tmp_path: Path):
    rng = random.Random(789)
    csv_path = tmp_path / "leak.csv"
    _write_csv(csv_path, _generate_leak_24h(2000.0, rng))
    leak_detector.main([
        "--csv", str(csv_path),
        "--bench-out", str(tmp_path / "bench.json"),
    ])
    bench = json.loads((tmp_path / "bench.json").read_text())
    # The bench file is FLAT (no `results` envelope) so the
    # aggregator's `results.update(json.load(f))` ingests it directly.
    assert "soak::rss_slope_bps" in bench
    assert "soak::sample_count" in bench
    assert "soak::leak_status" in bench
    # Slope is stored as B/s × 1e9 (ratio convention).
    measured_scaled = bench["soak::rss_slope_bps"]["median_ns"]
    assert abs(measured_scaled / 1e9 - 2000.0) < 100.0  # within noise


def test_bench_format_uses_se_as_jitter(tmp_path: Path):
    """The jitter_ns field carries SE(β̂₁) so a downstream consumer
    can compute confidence intervals without re-running OLS."""
    rng = random.Random(789)
    csv_path = tmp_path / "leak.csv"
    _write_csv(csv_path, _generate_leak_24h(2000.0, rng))
    leak_detector.main([
        "--csv", str(csv_path),
        "--bench-out", str(tmp_path / "bench.json"),
    ])
    bench = json.loads((tmp_path / "bench.json").read_text())
    jitter = bench["soak::rss_slope_bps"]["jitter_ns"]
    assert jitter > 0  # SE should never be exactly 0 with stochastic input


# ─────────────────────────────────────────────────────────────────────
# Disjoint key invariant
# ─────────────────────────────────────────────────────────────────────

def test_soak_keys_are_disjoint_from_load_and_accuracy(tmp_path: Path):
    """The aggregator merges bench-format files from load + soak +
    eval. None of the soak keys may collide with the others.
    """
    rng = random.Random(0)
    csv_path = tmp_path / "csv"
    _write_csv(csv_path, _generate_no_leak_24h(rng))
    leak_detector.main([
        "--csv", str(csv_path),
        "--bench-out", str(tmp_path / "bench.json"),
    ])
    bench = json.loads((tmp_path / "bench.json").read_text())
    for key in bench:
        assert key.startswith("soak::"), key
        assert not key.startswith("load_")
        assert not key.startswith("eval_accuracy::")
        assert not key.startswith("eval_latency::")
