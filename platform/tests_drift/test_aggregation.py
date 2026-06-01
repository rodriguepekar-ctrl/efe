"""Aggregation — bin edges, histogram, reservoir, bucketing, rollup."""

from __future__ import annotations

import math
from datetime import datetime, timedelta, timezone

import pytest

from efe_drift.aggregation import (
    CategoricalCounter,
    EmbeddingReservoir,
    FrozenBinSet,
    HistogramBuilder,
    OTHER_BUCKET,
    bucket_bounds,
    classify,
    custom_bins,
    equifrequency_bins,
    equiwidth_bins,
    project_into_categories,
    rollup_samples,
)
from efe_drift.catalog import FeatureCatalog, FeatureSpec
from efe_drift.errors import (
    EmptySampleSet,
    HistogramBinMismatch,
    InvalidBinningStrategy,
    ValidationError,
)
from efe_drift.extraction import build_default_registry
from efe_drift.records import FeatureSample, FeatureValue


# ---------------------------------------------------------------------------
# Bin edges
# ---------------------------------------------------------------------------


def test_equifrequency_bins_monotonic():
    samples = list(range(1000))
    edges = equifrequency_bins(samples, n_bins=10)
    assert len(edges) == 11
    for i in range(len(edges) - 1):
        assert edges[i + 1] > edges[i]


def test_equifrequency_bins_on_constant_input_jitters():
    samples = [5.0] * 100
    edges = equifrequency_bins(samples, n_bins=10)
    # Not strictly increasing by huge amounts, but monotone (jitter
    # bumps equal edges by 1e-12)
    for i in range(len(edges) - 1):
        assert edges[i + 1] > edges[i]


def test_equifrequency_bins_rejects_small_k():
    with pytest.raises(InvalidBinningStrategy):
        equifrequency_bins([1.0, 2.0, 3.0], n_bins=1)


def test_equifrequency_bins_rejects_empty():
    with pytest.raises(EmptySampleSet):
        equifrequency_bins([], n_bins=5)


def test_equiwidth_bins_linear_spacing():
    edges = equiwidth_bins([0.0, 10.0], n_bins=5)
    assert edges == (0.0, 2.0, 4.0, 6.0, 8.0, 10.0)


def test_custom_bins_validates_monotonicity():
    with pytest.raises(InvalidBinningStrategy):
        custom_bins([1.0, 2.0, 2.0, 3.0])
    with pytest.raises(InvalidBinningStrategy):
        custom_bins([1.0])
    with pytest.raises(InvalidBinningStrategy):
        custom_bins([1.0, float("inf"), 3.0])


def test_custom_bins_happy_path():
    assert custom_bins([0.0, 1.0, 2.0, 3.0]) == (0.0, 1.0, 2.0, 3.0)


# ---------------------------------------------------------------------------
# Classify
# ---------------------------------------------------------------------------


def test_classify_saturates_on_tails():
    edges = (0.0, 10.0, 20.0, 30.0)
    assert classify(-5.0, edges) == 0
    assert classify(15.0, edges) == 1
    assert classify(35.0, edges) == 2


def test_classify_bucket_boundaries_left_closed():
    edges = (0.0, 10.0, 20.0, 30.0)
    # Right-closed on the last bin, right-open elsewhere.
    assert classify(10.0, edges) == 1
    assert classify(20.0, edges) == 2
    assert classify(30.0, edges) == 2


def test_classify_rejects_tiny_edges():
    with pytest.raises(HistogramBinMismatch):
        classify(1.0, (1.0,))


# ---------------------------------------------------------------------------
# HistogramBuilder
# ---------------------------------------------------------------------------


def test_histogram_builder_counts_preserve_total():
    edges = (0.0, 10.0, 20.0, 30.0)
    hb = HistogramBuilder.from_edges(edges)
    for v in [5, 7, 15, 25, 25, 25]:
        hb.add(float(v))
    assert sum(hb.histogram()) == 6


def test_histogram_builder_stats_welford():
    hb = HistogramBuilder.from_edges((0.0, 10.0))
    for v in range(1, 11):
        hb.add(float(v))
    stats = hb.stats_summary()
    assert stats["n"] == 10
    assert math.isclose(stats["mean"], 5.5, abs_tol=1e-9)
    # sample std of 1..10
    assert math.isclose(stats["std"], 3.02765035, rel_tol=1e-6)


def test_histogram_builder_ignores_nan():
    hb = HistogramBuilder.from_edges((0.0, 1.0))
    hb.add(float("nan"))
    assert hb.n == 0


# ---------------------------------------------------------------------------
# CategoricalCounter
# ---------------------------------------------------------------------------


def test_categorical_counter_top_k_with_other():
    c = CategoricalCounter()
    for v in ["a", "a", "a", "b", "b", "c", "d", "e"]:
        c.add(v)
    tk = c.top_k_with_other(k=2)
    assert tk["a"] == 3
    assert tk["b"] == 2
    assert tk[OTHER_BUCKET] == 3


def test_project_into_frozen_categories_puts_new_in_other():
    c = CategoricalCounter()
    for v in ["a", "a", "b", "c", "d"]:
        c.add(v)
    projected = project_into_categories(
        c, frozen_categories=("a", "b"),
    )
    assert projected["a"] == 2
    assert projected["b"] == 1
    assert projected[OTHER_BUCKET] == 2


def test_project_refuses_other_in_frozen_list():
    c = CategoricalCounter()
    c.add("a")
    with pytest.raises(ValidationError):
        project_into_categories(
            c, frozen_categories=(OTHER_BUCKET, "a"),
        )


# ---------------------------------------------------------------------------
# EmbeddingReservoir
# ---------------------------------------------------------------------------


def test_reservoir_returns_all_when_under_capacity():
    r = EmbeddingReservoir(size=5, seed=1)
    for i in range(3):
        r.add([float(i), float(i + 1)])
    assert len(r.sample()) == 3


def test_reservoir_caps_at_size():
    r = EmbeddingReservoir(size=5, seed=1)
    for i in range(100):
        r.add([float(i), float(i + 1)])
    assert len(r.sample()) == 5
    assert r.seen == 100


def test_reservoir_centroid_and_std():
    r = EmbeddingReservoir(size=100, seed=1)
    for i in range(10):
        r.add([float(i), float(-i)])
    c = r.centroid()
    s = r.std()
    assert math.isclose(c[0], 4.5, abs_tol=1e-9)
    assert math.isclose(c[1], -4.5, abs_tol=1e-9)
    assert all(v > 0 for v in s)


def test_reservoir_rejects_dim_change():
    r = EmbeddingReservoir(size=10)
    r.add([1.0, 2.0])
    with pytest.raises(ValueError):
        r.add([1.0, 2.0, 3.0])


# ---------------------------------------------------------------------------
# Bucketing
# ---------------------------------------------------------------------------


def test_bucket_bounds_minute():
    ts = datetime(2026, 4, 20, 14, 37, 12, tzinfo=timezone.utc)
    start, end = bucket_bounds(ts, "minute")
    assert start == datetime(2026, 4, 20, 14, 37, tzinfo=timezone.utc)
    assert end == datetime(2026, 4, 20, 14, 38, tzinfo=timezone.utc)


def test_bucket_bounds_hour():
    ts = datetime(2026, 4, 20, 14, 37, 12, tzinfo=timezone.utc)
    start, end = bucket_bounds(ts, "hour")
    assert start == datetime(2026, 4, 20, 14, 0, tzinfo=timezone.utc)
    assert end == datetime(2026, 4, 20, 15, 0, tzinfo=timezone.utc)


def test_bucket_bounds_day():
    ts = datetime(2026, 4, 20, 14, 37, 12, tzinfo=timezone.utc)
    start, end = bucket_bounds(ts, "day")
    assert start == datetime(2026, 4, 20, tzinfo=timezone.utc)
    assert end == datetime(2026, 4, 21, tzinfo=timezone.utc)


def test_bucket_bounds_week():
    # 2026-04-20 is a Monday → week should start on itself.
    ts = datetime(2026, 4, 22, 10, 0, tzinfo=timezone.utc)   # Wednesday
    start, end = bucket_bounds(ts, "week")
    assert start.weekday() == 0
    assert end == start + timedelta(days=7)


def test_bucket_bounds_rejects_naive():
    with pytest.raises(ValidationError):
        bucket_bounds(datetime(2026, 4, 20, 14, 37), "hour")


# ---------------------------------------------------------------------------
# Rollup
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_rollup_produces_one_snapshot_per_bucket():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    entry = await catalog.register(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric", extractor="token_length",
        binning_strategy="equifrequency",
    ))
    t0 = datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc)
    samples = tuple(
        FeatureSample(
            sample_id=f"s{i}",
            tenant_id="acme",
            task_id="summarize",
            feature_id=entry.feature_id,
            feature_name="prompt_length_tokens",
            observed_at=t0 + timedelta(seconds=i),
            value=FeatureValue(
                kind="numeric",
                feature_name="prompt_length_tokens",
                numeric_value=float(i),
            ),
        )
        for i in range(120)
    )
    # Pre-frozen bins equivalent to [0,20,40,60,80,100,120]
    frozen = FrozenBinSet(
        numeric_edges={
            entry.feature_id: (0.0, 20.0, 40.0, 60.0, 80.0, 100.0, 120.0),
        },
    )
    snapshots = rollup_samples(
        samples, catalog=catalog, bucket_kind="minute", frozen=frozen,
    )
    # 120 samples spaced 1 s apart starting on minute boundary → 2 minutes
    assert len(snapshots) == 2
    total = sum(s.sample_count for s in snapshots)
    assert total == 120


@pytest.mark.asyncio
async def test_rollup_categorical_emits_top_k():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    entry = await catalog.register(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="language",
        kind="categorical", extractor="lang_detect",
        binning_strategy="none",
    ))
    t0 = datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc)
    values = ["fr", "fr", "fr", "en", "en", "es", "de", "it", "pt"]
    samples = tuple(
        FeatureSample(
            sample_id=f"s{i}",
            tenant_id="acme",
            task_id="summarize",
            feature_id=entry.feature_id,
            feature_name="language",
            observed_at=t0,
            value=FeatureValue(
                kind="categorical",
                feature_name="language",
                categorical_value=v,
            ),
        )
        for i, v in enumerate(values)
    )
    snapshots = rollup_samples(
        samples, catalog=catalog, bucket_kind="minute",
        top_k_categorical=2,
    )
    assert len(snapshots) == 1
    counts = snapshots[0].categorical_counts
    assert counts["fr"] == 3
    assert counts["en"] == 2
    assert counts[OTHER_BUCKET] == 4


@pytest.mark.asyncio
async def test_rollup_embedding_computes_centroid():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    entry = await catalog.register(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="prompt_embedding",
        kind="embedding", extractor="deterministic_hash_embedding",
        binning_strategy="none",
    ))
    t0 = datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc)
    dim = 4
    vectors = [
        (1.0, 0.0, 0.0, 0.0),
        (0.0, 1.0, 0.0, 0.0),
        (0.0, 0.0, 1.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    ]
    samples = tuple(
        FeatureSample(
            sample_id=f"s{i}",
            tenant_id="acme",
            task_id="summarize",
            feature_id=entry.feature_id,
            feature_name="prompt_embedding",
            observed_at=t0,
            value=FeatureValue(
                kind="embedding",
                feature_name="prompt_embedding",
                embedding=vec,
            ),
        )
        for i, vec in enumerate(vectors)
    )
    snapshots = rollup_samples(
        samples, catalog=catalog, bucket_kind="minute",
    )
    assert len(snapshots) == 1
    centroid = snapshots[0].embedding_centroid
    assert centroid is not None
    for c in centroid:
        assert math.isclose(c, 0.25, abs_tol=1e-9)
