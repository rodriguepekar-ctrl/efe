"""PSI detector + JS divergence."""

from __future__ import annotations

import math
from datetime import datetime, timedelta, timezone

import pytest

from efe_drift.detection import (
    DetectionContext,
    JsDivergenceDetector,
    PsiDetector,
    compute_psi,
    js_divergence,
)
from efe_drift.errors import HistogramBinMismatch
from efe_drift.ids import new_uuid
from efe_drift.records import (
    Baseline,
    BaselineSpec,
    FeatureEntry,
    FeatureSnapshot,
)


# ---------------------------------------------------------------------------
# Pure math
# ---------------------------------------------------------------------------


def test_compute_psi_identical_histograms_zero():
    assert compute_psi([10] * 10, [10] * 10) == 0.0


def test_compute_psi_worked_example():
    """Reproduce the M09 §22.B worked example → PSI ≈ 0.328."""
    baseline = [3_000_000] * 10
    current = [1200, 2400, 3600, 4800, 5200, 6400, 6800, 5600, 3200, 800]
    psi = compute_psi(current, baseline)
    assert 0.32 < psi < 0.34


def test_compute_psi_bin_mismatch_raises():
    with pytest.raises(HistogramBinMismatch):
        compute_psi([1, 2, 3], [1, 2])


def test_compute_psi_empty_returns_nan():
    assert math.isnan(compute_psi([0, 0], [0, 0]))


def test_js_divergence_bounded():
    """JS ∈ [0, ln 2]."""
    js = js_divergence([100, 0], [0, 100])
    # Two distributions with disjoint support — JS → ln 2 ≈ 0.693.
    assert 0.65 < js <= math.log(2) + 1e-6


def test_js_divergence_identical_is_zero():
    assert abs(js_divergence([10] * 5, [10] * 5)) < 1e-12


# ---------------------------------------------------------------------------
# PsiDetector integration
# ---------------------------------------------------------------------------


def _entry(name: str = "prompt_len", kind: str = "numeric") -> FeatureEntry:
    return FeatureEntry(
        feature_id=new_uuid(),
        tenant_id="acme",
        task_id="summarize",
        name=name,
        version=1,
        kind=kind,  # type: ignore[arg-type]
        extractor="token_length",
        binning_strategy="equifrequency",
        retention_days=90,
    )


def _baseline(
    entry: FeatureEntry, *, bins: tuple[float, ...], n: int = 10_000,
) -> Baseline:
    spec = BaselineSpec(
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_name=entry.name,
        feature_kind=entry.kind,
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=n,
        bins=bins,
        stats={"mean": 100.0, "std": 50.0, "n": float(n)},
    )
    return Baseline(
        baseline_id=new_uuid(),
        spec=spec,
        state="active",
        state_version=1,
        promoted_at=datetime(2026, 4, 8, tzinfo=timezone.utc),
    )


def _snapshot(
    entry: FeatureEntry, *, bins: tuple[float, ...],
    histogram: tuple[int, ...],
    start: datetime,
) -> FeatureSnapshot:
    return FeatureSnapshot(
        snapshot_id=new_uuid(),
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        feature_kind=entry.kind,
        bucket_kind="minute",
        bucket_start=start,
        bucket_end=start + timedelta(minutes=1),
        sample_count=sum(histogram),
        histogram=histogram,
    )


def test_psi_detector_no_drift():
    entry = _entry()
    bins = (0.0, 10.0, 20.0, 30.0, 40.0, 50.0)
    baseline = _baseline(entry, bins=bins, n=5000)
    current = _snapshot(
        entry, bins=bins, histogram=(200, 200, 200, 200, 200),
        start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
    )
    ctx = DetectionContext(
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature=entry,
        baseline=baseline,
        window_start=current.bucket_start,
        window_end=current.bucket_end,
    )
    result = PsiDetector().detect_from_snapshots(ctx, (current,))
    assert result.method == "psi"
    assert result.score < 0.05
    assert result.severity == "none"


def test_psi_detector_detects_major_drift():
    entry = _entry()
    bins = (0.0, 10.0, 20.0, 30.0, 40.0, 50.0)
    baseline = _baseline(entry, bins=bins, n=10_000)
    # All mass shifted to last two bins
    current = _snapshot(
        entry, bins=bins, histogram=(0, 0, 0, 500, 500),
        start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
    )
    ctx = DetectionContext(
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature=entry,
        baseline=baseline,
        window_start=current.bucket_start,
        window_end=current.bucket_end,
    )
    result = PsiDetector().detect_from_snapshots(ctx, (current,))
    assert result.score > 0.4
    assert result.severity == "critical"
    assert result.is_breach()
    assert "per_bin" in result.details


def test_psi_detector_rejects_wrong_kind():
    entry = _entry(kind="categorical")
    bins = (0.0, 1.0, 2.0)
    baseline = _baseline(entry, bins=bins)
    ctx = DetectionContext(
        tenant_id="acme", task_id="t", feature=entry,
        baseline=baseline,
        window_start=datetime(2026, 4, 10, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 1, tzinfo=timezone.utc),
    )
    with pytest.raises(ValueError):
        PsiDetector().detect_from_snapshots(ctx, ())


def test_js_divergence_detector_on_same_snapshot_returns_zero():
    entry = _entry()
    bins = (0.0, 10.0, 20.0, 30.0, 40.0, 50.0)
    baseline = _baseline(entry, bins=bins, n=5000)
    # Uniform current ⟵ uniform baseline — JS ≈ 0
    current = _snapshot(
        entry, bins=bins, histogram=(200, 200, 200, 200, 200),
        start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
    )
    ctx = DetectionContext(
        tenant_id=entry.tenant_id, task_id=entry.task_id,
        feature=entry, baseline=baseline,
        window_start=current.bucket_start,
        window_end=current.bucket_end,
    )
    result = JsDivergenceDetector().detect_from_snapshots(ctx, (current,))
    assert result.method == "js_divergence"
    assert result.score < 1e-6
    assert result.severity == "none"
