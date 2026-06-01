"""Rollup orchestration — drains raw samples into per-bucket aggregates.

:class:`MinuteRollup` is the core of L2's warm boucle. Given a
sequence of :class:`FeatureSample` rows and a catalog, it produces a
set of :class:`FeatureSnapshot` rows — one per
``(feature_id, bucket_start)`` pair.

The rollup is **pure** (no DB I/O), deterministic, and preserves
sample ordering across buckets. It is exposed as a free function
``rollup_samples(...)`` so tests can call it without constructing the
scheduling wrapper.

Higher-resolution rollups (hour, day) reuse the same function by
passing a different ``bucket_kind`` and pre-projected snapshots as
input.
"""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from typing import Literal

from efe_drift.aggregation.aggregator import WindowAggregator
from efe_drift.aggregation.bucketing import bucket_bounds
from efe_drift.catalog.registry import FeatureCatalog
from efe_drift.records import (
    BucketKind,
    FeatureEntry,
    FeatureSample,
    FeatureSnapshot,
)

# ---------------------------------------------------------------------------
# BaselineFrozenBins — lightweight lookup for pre-freezing aggregators
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class FrozenBinSet:
    """Container for frozen bin edges / categories passed to a rollup.

    Consumers pass this to :func:`rollup_samples` so every aggregator
    is pre-configured with the baseline's edges. When absent, the
    aggregator accumulates raw values (candidate mode).
    """

    numeric_edges: Mapping[str, tuple[float, ...]] = None  # type: ignore[assignment]
    categorical_categories: Mapping[str, tuple[str, ...]] = None  # type: ignore[assignment]

    def numeric_for(self, feature_id: str) -> tuple[float, ...] | None:
        if self.numeric_edges is None:
            return None
        return self.numeric_edges.get(feature_id)

    def categorical_for(self, feature_id: str) -> tuple[str, ...] | None:
        if self.categorical_categories is None:
            return None
        return self.categorical_categories.get(feature_id)


# ---------------------------------------------------------------------------
# Rollup function
# ---------------------------------------------------------------------------


def rollup_samples(
    samples: Iterable[FeatureSample],
    *,
    catalog: FeatureCatalog,
    bucket_kind: BucketKind = "minute",
    frozen: FrozenBinSet | None = None,
    top_k_categorical: int = 10,
    reservoir_size: int = 10_000,
) -> tuple[FeatureSnapshot, ...]:
    """Aggregate ``samples`` into a tuple of :class:`FeatureSnapshot`.

    The output is sorted by ``(feature_id, bucket_start)`` so that
    callers diffing successive rollup runs see a stable order.
    """
    frozen = frozen or FrozenBinSet()
    aggregators: dict[tuple[str, str], WindowAggregator] = {}

    for sample in samples:
        entry: FeatureEntry = catalog.get(sample.feature_id)
        b_start, _ = bucket_bounds(sample.observed_at, bucket_kind)
        key = (sample.feature_id, b_start.isoformat())
        agg = aggregators.get(key)
        if agg is None:
            agg = _make_aggregator(
                entry=entry,
                sample=sample,
                bucket_kind=bucket_kind,
                frozen=frozen,
                top_k=top_k_categorical,
                reservoir_size=reservoir_size,
            )
            aggregators[key] = agg
        agg.ingest(sample)

    snapshots = tuple(
        agg.emit()
        for _, agg in sorted(aggregators.items())
    )
    return snapshots


def _make_aggregator(
    *,
    entry: FeatureEntry,
    sample: FeatureSample,
    bucket_kind: BucketKind,
    frozen: FrozenBinSet,
    top_k: int,
    reservoir_size: int,
) -> WindowAggregator:
    if entry.kind == "numeric" or entry.kind == "binary":
        # Narrow the Literal for the downstream signature.
        narrow_kind: Literal["numeric", "binary"] = entry.kind
        return WindowAggregator.numeric_from_bucket(
            tenant_id=sample.tenant_id,
            task_id=sample.task_id,
            feature_id=sample.feature_id,
            feature_name=sample.feature_name,
            feature_kind=narrow_kind,
            observed_at=sample.observed_at,
            bucket_kind=bucket_kind,
            edges=frozen.numeric_for(sample.feature_id),
        )
    if entry.kind == "categorical":
        return WindowAggregator.categorical_from_bucket(
            tenant_id=sample.tenant_id,
            task_id=sample.task_id,
            feature_id=sample.feature_id,
            feature_name=sample.feature_name,
            observed_at=sample.observed_at,
            bucket_kind=bucket_kind,
            top_k=top_k,
            frozen_categories=frozen.categorical_for(sample.feature_id),
        )
    # embedding
    return WindowAggregator.embedding_from_bucket(
        tenant_id=sample.tenant_id,
        task_id=sample.task_id,
        feature_id=sample.feature_id,
        feature_name=sample.feature_name,
        observed_at=sample.observed_at,
        bucket_kind=bucket_kind,
        reservoir_size=reservoir_size,
    )


__all__ = ["FrozenBinSet", "rollup_samples"]
