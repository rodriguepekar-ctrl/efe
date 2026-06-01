"""KS, Chi-squared, Wasserstein, MMD, Classifier detectors."""

from __future__ import annotations

import math
from datetime import datetime, timezone

import pytest

from efe_drift.detection import (
    ChiSquaredDetector,
    ClassifierDriftDetector,
    DetectionContext,
    KsDetector,
    MmdDetector,
    WassersteinDetector,
)
from efe_drift.detection.chi_squared import (
    chi_squared_goodness_of_fit,
    g_test_statistic,
)
from efe_drift.detection.classifier import classifier_drift_score
from efe_drift.detection.mmd import (
    centroid_distance,
    mmd_permutation_p_value,
    mmd_squared_rbf,
)
from efe_drift.detection.wasserstein import wasserstein_1d
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


def test_chi_squared_worked_example():
    # Example from M09 §10.2 — fr/en/other 60/30/10 vs 40/40/20.
    observed = [40.0, 40.0, 20.0]
    expected = [60.0, 30.0, 10.0]
    chi_sq, df = chi_squared_goodness_of_fit(observed, expected)
    assert abs(chi_sq - 20.0) < 1e-6
    assert df == 2


def test_chi_squared_zero_on_perfect_fit():
    chi_sq, _ = chi_squared_goodness_of_fit(
        [100.0, 200.0], [100.0, 200.0],
    )
    assert chi_sq == 0.0


def test_chi_squared_drops_zero_expected_bins():
    chi_sq, df = chi_squared_goodness_of_fit(
        [100.0, 100.0, 100.0], [100.0, 100.0, 0.0],
    )
    # Last bin dropped → df = 2 - 1 = 1.
    assert df == 1


def test_g_test_converges_to_chi_squared_for_large_n():
    observed = [500.0, 480.0, 520.0]
    expected = [500.0, 500.0, 500.0]
    chi_sq, _ = chi_squared_goodness_of_fit(observed, expected)
    g, _ = g_test_statistic(observed, expected)
    assert abs(g - chi_sq) < 1.0


# ---------------------------------------------------------------------------
# Wasserstein
# ---------------------------------------------------------------------------


def test_wasserstein_identical_zero():
    assert wasserstein_1d([1, 2, 3], [1, 2, 3]) == 0.0


def test_wasserstein_uniform_shift():
    a = list(range(100))
    b = [x + 10 for x in a]
    d = wasserstein_1d(a, b)
    # Pure translation by 10 → W_1 = 10.
    assert abs(d - 10.0) < 0.5


def test_wasserstein_empty_nan():
    assert math.isnan(wasserstein_1d([], [1, 2, 3]))


# ---------------------------------------------------------------------------
# MMD²
# ---------------------------------------------------------------------------


def test_mmd_identical_distributions_near_zero():
    x = [(float(i), float(i + 1)) for i in range(15)]
    # Use the same points twice — MMD² must be ≤ 0 on unbiased estimator.
    mmd, _ = mmd_squared_rbf(x, x)
    assert mmd < 0.05


def test_mmd_detects_shifted_distributions():
    x = [(float(i), 0.0) for i in range(20)]
    y = [(float(i) + 10.0, 0.0) for i in range(20)]
    mmd, _ = mmd_squared_rbf(x, y)
    assert mmd > 0.3


def test_mmd_permutation_p_value_detects_shift():
    x = [(float(i), 0.0) for i in range(15)]
    y = [(float(i) + 10.0, 0.0) for i in range(15)]
    _, p = mmd_permutation_p_value(
        x, y, n_permutations=50, seed=42,
    )
    assert p < 0.1


def test_centroid_distance_zero_on_identical():
    assert centroid_distance((1.0, 2.0), (1.0, 2.0)) == 0.0


def test_centroid_distance_normalised_by_std():
    d = centroid_distance(
        (2.0, 2.0), (0.0, 0.0), baseline_std=(1.0, 1.0),
    )
    # Δ = (2, 2), normalised by (std + eps)=(1+1e-6, 1+1e-6) → ~√8.
    assert abs(d - math.sqrt(8.0)) < 1e-4


# ---------------------------------------------------------------------------
# Classifier drift
# ---------------------------------------------------------------------------


def test_classifier_drift_zero_on_identical():
    # Two identical draws → classifier can't distinguish → score ≈ 0.
    base = [(float(i), 0.0) for i in range(20)]
    cur = [(float(i), 0.0) for i in range(20)]
    auc, score, n_train, n_test = classifier_drift_score(
        base, cur, seed=7,
    )
    assert n_test > 0
    assert abs(score) < 0.5    # well below threshold


def test_classifier_drift_detects_shifted():
    # Wide shift → perfect separation → AUC ≈ 1 → score ≈ 1.
    base = [(float(i), 0.0) for i in range(40)]
    cur = [(float(i) + 100.0, 0.0) for i in range(40)]
    _, score, _, _ = classifier_drift_score(base, cur, seed=42)
    assert score > 0.5


def test_classifier_drift_rejects_mismatched_dims():
    # ≥ 2 vectors per side so we pass the insufficiency guard and
    # reach the dim-mismatch check.
    with pytest.raises(ValueError):
        classifier_drift_score(
            [(1.0, 2.0), (3.0, 4.0)],
            [(1.0,), (2.0,)],
            seed=1,
        )


# ---------------------------------------------------------------------------
# Detectors wired against a Baseline
# ---------------------------------------------------------------------------


def _numeric_entry() -> FeatureEntry:
    return FeatureEntry(
        feature_id=new_uuid(),
        tenant_id="acme",
        task_id="summarize",
        name="latency_ms",
        version=1,
        kind="numeric",
        extractor="backend_latency",
        binning_strategy="equifrequency",
        retention_days=90,
    )


def _baseline_for(entry: FeatureEntry) -> Baseline:
    spec = BaselineSpec(
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_name=entry.name,
        feature_kind=entry.kind,
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=1000,
        bins=(0.0, 10.0, 20.0, 30.0, 40.0, 50.0),
        stats={"mean": 25.0, "std": 10.0, "n": 1000.0},
    )
    return Baseline(
        baseline_id=new_uuid(),
        spec=spec,
        state="active",
        state_version=1,
    )


def test_ks_detector_uniform_in_uniform_low_d():
    entry = _numeric_entry()
    baseline = _baseline_for(entry)
    # Current snapshot mirrors the baseline → KS D small.
    snapshot = FeatureSnapshot(
        snapshot_id=new_uuid(),
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        feature_kind=entry.kind,
        bucket_kind="minute",
        bucket_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        bucket_end=datetime(2026, 4, 10, 10, 1, tzinfo=timezone.utc),
        sample_count=500,
        histogram=(100, 100, 100, 100, 100),
    )
    ctx = DetectionContext(
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature=entry,
        baseline=baseline,
        window_start=snapshot.bucket_start,
        window_end=snapshot.bucket_end,
    )
    result = KsDetector().detect_from_snapshots(ctx, (snapshot,))
    assert result.method == "ks"
    assert result.score < 0.1


def test_ks_detector_raw_path_exact():
    entry = _numeric_entry()
    baseline = _baseline_for(entry)
    ctx = DetectionContext(
        tenant_id=entry.tenant_id, task_id=entry.task_id,
        feature=entry, baseline=baseline,
        window_start=datetime(2026, 4, 10, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 1, tzinfo=timezone.utc),
    )
    # Shifted distribution.
    cur = list(range(100, 200))
    base = list(range(100))
    result = KsDetector().detect_from_raw(ctx, cur, base)
    assert result.score == 1.0
    assert result.p_value is not None and result.p_value < 1e-10


def test_wasserstein_detector_raw_translation():
    entry = _numeric_entry()
    baseline = _baseline_for(entry)
    ctx = DetectionContext(
        tenant_id=entry.tenant_id, task_id=entry.task_id,
        feature=entry, baseline=baseline,
        window_start=datetime(2026, 4, 10, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 1, tzinfo=timezone.utc),
    )
    base = list(range(100))
    cur = [x + 20 for x in base]
    result = WassersteinDetector(normalise_by_std=False)\
        .detect_from_raw(ctx, cur, base)
    assert 19.0 < result.score < 21.0


def test_chi_squared_detector_basic():
    entry = FeatureEntry(
        feature_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        name="language", version=1, kind="categorical",
        extractor="lang_detect",
        binning_strategy="none", retention_days=90,
    )
    spec = BaselineSpec(
        tenant_id="acme", task_id="summarize",
        feature_name="language", feature_kind="categorical",
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=300,
        categorical_categories=("fr", "en", "es"),
    )
    baseline = Baseline(
        baseline_id=new_uuid(), spec=spec, state="active", state_version=1,
    )
    snapshot = FeatureSnapshot(
        snapshot_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        feature_id=entry.feature_id, feature_name="language",
        feature_kind="categorical",
        bucket_kind="minute",
        bucket_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        bucket_end=datetime(2026, 4, 10, 10, 1, tzinfo=timezone.utc),
        sample_count=100,
        categorical_counts={"fr": 60, "en": 40},
    )
    ctx = DetectionContext(
        tenant_id="acme", task_id="summarize",
        feature=entry, baseline=baseline,
        window_start=snapshot.bucket_start,
        window_end=snapshot.bucket_end,
    )
    result = ChiSquaredDetector().detect_from_snapshots(ctx, (snapshot,))
    assert result.method == "chi_squared"
    assert result.p_value is not None
    assert result.details["df"] >= 1


def test_mmd_detector_centroid_path():
    entry = FeatureEntry(
        feature_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        name="prompt_emb", version=1, kind="embedding",
        extractor="deterministic_hash_embedding",
        binning_strategy="none", retention_days=30,
    )
    spec = BaselineSpec(
        tenant_id="acme", task_id="summarize",
        feature_name="prompt_emb", feature_kind="embedding",
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=100,
        centroid=(0.0, 0.0, 0.0),
        std=(1.0, 1.0, 1.0),
    )
    baseline = Baseline(
        baseline_id=new_uuid(), spec=spec, state="active", state_version=1,
    )
    snapshot = FeatureSnapshot(
        snapshot_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        feature_id=entry.feature_id, feature_name="prompt_emb",
        feature_kind="embedding",
        bucket_kind="minute",
        bucket_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        bucket_end=datetime(2026, 4, 10, 10, 1, tzinfo=timezone.utc),
        sample_count=50,
        embedding_centroid=(2.0, 2.0, 2.0),
        embedding_std=(1.0, 1.0, 1.0),
    )
    ctx = DetectionContext(
        tenant_id="acme", task_id="summarize",
        feature=entry, baseline=baseline,
        window_start=snapshot.bucket_start,
        window_end=snapshot.bucket_end,
    )
    result = MmdDetector().detect_from_snapshots(ctx, (snapshot,))
    assert result.method == "mmd"
    assert result.score > 0.0
    assert result.details["mode"] == "centroid"


def test_classifier_detector_on_raw_vectors():
    entry = FeatureEntry(
        feature_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        name="prompt_emb", version=1, kind="embedding",
        extractor="deterministic_hash_embedding",
        binning_strategy="none", retention_days=30,
    )
    spec = BaselineSpec(
        tenant_id="acme", task_id="summarize",
        feature_name="prompt_emb", feature_kind="embedding",
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=40,
    )
    baseline = Baseline(
        baseline_id=new_uuid(), spec=spec, state="active", state_version=1,
    )
    ctx = DetectionContext(
        tenant_id="acme", task_id="summarize",
        feature=entry, baseline=baseline,
        window_start=datetime(2026, 4, 10, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 1, tzinfo=timezone.utc),
    )
    base = [(float(i), 0.0) for i in range(40)]
    cur = [(float(i) + 100.0, 0.0) for i in range(40)]
    result = ClassifierDriftDetector().detect_from_raw(ctx, cur, base)
    assert result.method == "classifier_based"
    assert result.details["auc"] > 0.9
    assert result.severity == "critical"
