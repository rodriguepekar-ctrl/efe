"""Aggregation plane : histograms, reservoirs, bucketing, rollups."""

from __future__ import annotations

from efe_drift.aggregation.aggregator import (
    CategoricalAggregator,
    EmbeddingAggregator,
    NumericAggregator,
    WindowAggregator,
)
from efe_drift.aggregation.bucketing import (
    bucket_bounds,
    bucket_start,
    iter_buckets,
)
from efe_drift.aggregation.categorical import (
    OTHER_BUCKET,
    CategoricalCounter,
    project_into_categories,
)
from efe_drift.aggregation.histogram import (
    HistogramBuilder,
    classify,
    custom_bins,
    equifrequency_bins,
    equiwidth_bins,
    validate_bin_match,
)
from efe_drift.aggregation.reservoir import EmbeddingReservoir
from efe_drift.aggregation.rollup import FrozenBinSet, rollup_samples

__all__ = [
    "OTHER_BUCKET",
    "CategoricalAggregator",
    "CategoricalCounter",
    "EmbeddingAggregator",
    "EmbeddingReservoir",
    "FrozenBinSet",
    "HistogramBuilder",
    "NumericAggregator",
    "WindowAggregator",
    "bucket_bounds",
    "bucket_start",
    "classify",
    "custom_bins",
    "equifrequency_bins",
    "equiwidth_bins",
    "iter_buckets",
    "project_into_categories",
    "rollup_samples",
    "validate_bin_match",
]
