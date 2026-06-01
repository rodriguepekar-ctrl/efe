"""Per-feature window aggregator.

:class:`WindowAggregator` maintains a **running aggregation** over a
single feature for a single ``(tenant, task, feature)`` scope. It is
the unit the minute-rollup job reads from and drains into the
``feature_snapshots`` table.

Two varieties :

* For **numeric / binary** features : a :class:`HistogramBuilder`
  (with frozen edges if a baseline is already active, otherwise an
  "accumulation" mode that just records raw samples until a baseline
  is promoted).

* For **categorical** features : a :class:`CategoricalCounter` +
  top-K bucketing applied on emit.

* For **embedding** features : an :class:`EmbeddingReservoir`.

The aggregator is stateful and **not** thread-safe on its own — a
single asyncio task owns it. Callers coordinate across features via
:class:`RollupRegistry`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Literal

from efe_drift.aggregation.bucketing import bucket_bounds
from efe_drift.aggregation.categorical import (
    OTHER_BUCKET,
    CategoricalCounter,
    project_into_categories,
)
from efe_drift.aggregation.histogram import HistogramBuilder
from efe_drift.aggregation.reservoir import EmbeddingReservoir
from efe_drift.errors import ExtractorTypeMismatch, ValidationError
from efe_drift.ids import new_uuid
from efe_drift.records import (
    BucketKind,
    FeatureKind,
    FeatureSample,
    FeatureSnapshot,
)


@dataclass(slots=True)
class NumericAggregator:
    """Numeric / binary window aggregator with frozen bin edges.

    ``edges`` must be supplied by the baseline manager before the
    aggregator can emit a snapshot. Until then, ``frozen`` is False
    and :meth:`ingest` just buffers raw values for the eventual
    baseline candidate.
    """

    edges: tuple[float, ...] | None = None
    raw_samples: list[float] = field(default_factory=list)
    builder: HistogramBuilder | None = None

    @property
    def frozen(self) -> bool:
        return self.edges is not None

    def freeze(self, edges: tuple[float, ...]) -> None:
        if len(edges) < 2:
            raise ValidationError(
                f"need ≥ 2 edges, got {len(edges)}",
            )
        self.edges = tuple(edges)
        self.builder = HistogramBuilder.from_edges(self.edges)
        for s in self.raw_samples:
            self.builder.add(s)
        self.raw_samples.clear()

    def ingest(self, value: float) -> None:
        if self.builder is not None:
            self.builder.add(value)
        else:
            self.raw_samples.append(float(value))

    @property
    def sample_count(self) -> int:
        if self.builder is not None:
            return self.builder.n
        return len(self.raw_samples)

    def histogram(self) -> tuple[int, ...] | None:
        return self.builder.histogram() if self.builder else None

    def stats(self) -> dict[str, float]:
        if self.builder is not None:
            return self.builder.stats_summary()
        if not self.raw_samples:
            return {
                "n": 0.0, "mean": float("nan"), "std": float("nan"),
                "min": float("nan"), "max": float("nan"),
            }
        n = len(self.raw_samples)
        mean = sum(self.raw_samples) / n
        var = (
            sum((v - mean) ** 2 for v in self.raw_samples) / (n - 1)
            if n > 1 else 0.0
        )
        return {
            "n": float(n),
            "mean": mean,
            "std": var ** 0.5,
            "min": min(self.raw_samples),
            "max": max(self.raw_samples),
        }


@dataclass(slots=True)
class CategoricalAggregator:
    top_k: int = 10
    counter: CategoricalCounter = field(default_factory=CategoricalCounter)
    frozen_categories: tuple[str, ...] | None = None

    def freeze(self, categories: tuple[str, ...]) -> None:
        if OTHER_BUCKET in categories:
            raise ValidationError(
                f"{OTHER_BUCKET!r} must not appear in frozen categories",
            )
        self.frozen_categories = tuple(categories)

    def ingest(self, value: str) -> None:
        self.counter.add(value)

    @property
    def sample_count(self) -> int:
        return self.counter.total

    def emit_counts(self) -> dict[str, int]:
        if self.frozen_categories is not None:
            return project_into_categories(
                self.counter,
                frozen_categories=self.frozen_categories,
            )
        return self.counter.top_k_with_other(k=self.top_k)


@dataclass(slots=True)
class EmbeddingAggregator:
    reservoir: EmbeddingReservoir = field(
        default_factory=lambda: EmbeddingReservoir(size=10_000),
    )
    expected_dim: int | None = None

    def ingest(self, vec: tuple[float, ...]) -> None:
        if self.expected_dim is None:
            self.expected_dim = len(vec)
        elif len(vec) != self.expected_dim:
            raise ValidationError(
                f"embedding dim mismatch : expected {self.expected_dim}, "
                f"got {len(vec)}",
            )
        self.reservoir.add(vec)

    @property
    def sample_count(self) -> int:
        return self.reservoir.seen


# ---------------------------------------------------------------------------
# WindowAggregator — the façade
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class WindowAggregator:
    """Aggregator for one (tenant, task, feature, bucket) slot.

    The aggregator has a fixed ``bucket_kind`` and a moving
    ``bucket_start`` — the caller rotates it (via :meth:`rotate`)
    each time the wall-clock leaves the current bucket.
    """

    tenant_id: str
    task_id: str
    feature_id: str
    feature_name: str
    feature_kind: FeatureKind
    bucket_kind: BucketKind
    bucket_start: datetime
    bucket_end: datetime
    numeric: NumericAggregator | None = None
    categorical: CategoricalAggregator | None = None
    embedding: EmbeddingAggregator | None = None

    @classmethod
    def numeric_from_bucket(
        cls,
        *,
        tenant_id: str,
        task_id: str,
        feature_id: str,
        feature_name: str,
        feature_kind: Literal["numeric", "binary"],
        observed_at: datetime,
        bucket_kind: BucketKind = "minute",
        edges: tuple[float, ...] | None = None,
    ) -> WindowAggregator:
        start, end = bucket_bounds(observed_at, bucket_kind)
        agg = cls(
            tenant_id=tenant_id,
            task_id=task_id,
            feature_id=feature_id,
            feature_name=feature_name,
            feature_kind=feature_kind,
            bucket_kind=bucket_kind,
            bucket_start=start,
            bucket_end=end,
            numeric=NumericAggregator(),
        )
        if edges is not None and agg.numeric is not None:
            agg.numeric.freeze(edges)
        return agg

    @classmethod
    def categorical_from_bucket(
        cls,
        *,
        tenant_id: str,
        task_id: str,
        feature_id: str,
        feature_name: str,
        observed_at: datetime,
        bucket_kind: BucketKind = "minute",
        top_k: int = 10,
        frozen_categories: tuple[str, ...] | None = None,
    ) -> WindowAggregator:
        start, end = bucket_bounds(observed_at, bucket_kind)
        agg = cls(
            tenant_id=tenant_id,
            task_id=task_id,
            feature_id=feature_id,
            feature_name=feature_name,
            feature_kind="categorical",
            bucket_kind=bucket_kind,
            bucket_start=start,
            bucket_end=end,
            categorical=CategoricalAggregator(top_k=top_k),
        )
        if frozen_categories is not None and agg.categorical is not None:
            agg.categorical.freeze(frozen_categories)
        return agg

    @classmethod
    def embedding_from_bucket(
        cls,
        *,
        tenant_id: str,
        task_id: str,
        feature_id: str,
        feature_name: str,
        observed_at: datetime,
        bucket_kind: BucketKind = "minute",
        reservoir_size: int = 10_000,
    ) -> WindowAggregator:
        start, end = bucket_bounds(observed_at, bucket_kind)
        return cls(
            tenant_id=tenant_id,
            task_id=task_id,
            feature_id=feature_id,
            feature_name=feature_name,
            feature_kind="embedding",
            bucket_kind=bucket_kind,
            bucket_start=start,
            bucket_end=end,
            embedding=EmbeddingAggregator(
                reservoir=EmbeddingReservoir(size=reservoir_size),
            ),
        )

    # ---- ingest --------------------------------------------------------

    def ingest(self, sample: FeatureSample) -> None:
        if sample.feature_id != self.feature_id:
            raise ValidationError(
                f"sample belongs to feature {sample.feature_id!r} "
                f"but aggregator tracks {self.feature_id!r}",
            )
        if self.feature_kind != sample.value.kind:
            raise ExtractorTypeMismatch(
                f"feature {self.feature_name!r} is {self.feature_kind!r} "
                f"but sample is {sample.value.kind!r}",
            )
        if self.numeric is not None:
            self.numeric.ingest(sample.value.unwrap_numeric())
        elif self.categorical is not None:
            self.categorical.ingest(sample.value.unwrap_categorical())
        elif self.embedding is not None:
            self.embedding.ingest(sample.value.unwrap_embedding())
        else:                                         # pragma: no cover
            raise RuntimeError(
                "aggregator has no inner accumulator — this is a bug",
            )

    # ---- emit ----------------------------------------------------------

    @property
    def sample_count(self) -> int:
        if self.numeric is not None:
            return self.numeric.sample_count
        if self.categorical is not None:
            return self.categorical.sample_count
        if self.embedding is not None:
            return self.embedding.sample_count
        return 0

    def emit(self) -> FeatureSnapshot:
        """Materialise the running state into a :class:`FeatureSnapshot`."""
        if self.numeric is not None:
            return FeatureSnapshot(
                snapshot_id=new_uuid(),
                tenant_id=self.tenant_id,
                task_id=self.task_id,
                feature_id=self.feature_id,
                feature_name=self.feature_name,
                feature_kind=self.feature_kind,
                bucket_kind=self.bucket_kind,
                bucket_start=self.bucket_start,
                bucket_end=self.bucket_end,
                sample_count=self.numeric.sample_count,
                histogram=self.numeric.histogram(),
                stats=self.numeric.stats(),
            )
        if self.categorical is not None:
            return FeatureSnapshot(
                snapshot_id=new_uuid(),
                tenant_id=self.tenant_id,
                task_id=self.task_id,
                feature_id=self.feature_id,
                feature_name=self.feature_name,
                feature_kind=self.feature_kind,
                bucket_kind=self.bucket_kind,
                bucket_start=self.bucket_start,
                bucket_end=self.bucket_end,
                sample_count=self.categorical.sample_count,
                categorical_counts=self.categorical.emit_counts(),
            )
        if self.embedding is not None:
            return FeatureSnapshot(
                snapshot_id=new_uuid(),
                tenant_id=self.tenant_id,
                task_id=self.task_id,
                feature_id=self.feature_id,
                feature_name=self.feature_name,
                feature_kind=self.feature_kind,
                bucket_kind=self.bucket_kind,
                bucket_start=self.bucket_start,
                bucket_end=self.bucket_end,
                sample_count=self.embedding.sample_count,
                embedding_centroid=self.embedding.reservoir.centroid(),
                embedding_std=self.embedding.reservoir.std(),
            )
        raise RuntimeError(                            # pragma: no cover
            "aggregator has no inner accumulator — this is a bug",
        )


__all__ = [
    "CategoricalAggregator",
    "EmbeddingAggregator",
    "NumericAggregator",
    "WindowAggregator",
]
