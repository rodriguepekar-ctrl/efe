"""Canonical in-memory value types used throughout :mod:`efe_drift`.

Every value type is an immutable ``frozen=True, slots=True``
dataclass — hashable, cheap, and impossible to mutate accidentally.

Taxonomy :

* :class:`InferenceEvent` — input to the extraction plane. Constructed
  by the NATS consumer or HTTP ingestion endpoint. Carries the raw
  fields relevant to feature extraction (prompt, response, tokens,
  latency, cost, request metadata).

* :class:`FeatureValue` — output of an extractor. Carries the
  ``kind`` (numeric / categorical / binary / embedding), the actual
  value, and the feature name.

* :class:`FeatureSample` — paired :class:`FeatureValue` + timing
  metadata, ready to be persisted in the hypertable.

* :class:`FeatureSnapshot` — aggregated histogram over a time bucket.

* :class:`Baseline` — runtime view of a baseline record.

* :class:`BaselineSpec` — canonical seal-input (hash-sealed).

Conventions :

* All timestamps are tz-aware UTC datetimes.
* All IDs are string UUIDs produced by :mod:`efe_drift.ids`.
* Feature kinds are :class:`~typing.Literal` strings.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Literal

FeatureKind = Literal["numeric", "categorical", "binary", "embedding"]
BinningStrategy = Literal["equifrequency", "equiwidth", "custom", "none"]
BucketKind = Literal["minute", "hour", "day", "week"]
BaselineState = Literal[
    "planned",
    "accumulating",
    "candidate",
    "active",
    "retiring",
    "retired",
    "rejected",
]


# ---------------------------------------------------------------------------
# Inference event (input to extractors)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class InferenceEvent:
    """Minimal subset of an ``efe.inference.completed`` event.

    Only the fields required by the extractors are present. Anything
    more would be a PII-leak risk (raw prompt / response bodies stay
    transient — we never persist them here).
    """

    event_id: str
    tenant_id: str
    task_id: str
    user_id: str                       # will be hashed by extractors
    observed_at: datetime
    request_id: str | None = None
    # Tokens + latency + cost are the default numeric features
    prompt_token_count: int | None = None
    response_token_count: int | None = None
    latency_ms: float | None = None
    cost_usd: float | None = None
    # Model + routing context
    model_id: str | None = None
    backend_kind: str | None = None
    # Request-time parameters
    temperature: float | None = None
    top_p: float | None = None
    # Raw texts (transient — never persisted)
    prompt_text: str | None = None
    response_text: str | None = None
    # Caller-supplied metadata for HTE slicing
    covariates: dict[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Feature value (output of one extractor)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class FeatureValue:
    """Pair of ``(kind, value)`` produced by an extractor.

    Matching rule :

    * ``kind == "numeric"``     → ``numeric_value`` set, others None
    * ``kind == "categorical"`` → ``categorical_value`` set
    * ``kind == "binary"``      → ``numeric_value`` ∈ {0.0, 1.0}
    * ``kind == "embedding"``   → ``embedding`` set (tuple[float, ...])

    Callers that construct :class:`FeatureValue` directly are
    responsible for the matching constraint. :mod:`efe_drift.extraction.base`
    provides typed factory helpers that enforce it.
    """

    kind: FeatureKind
    feature_name: str
    numeric_value: float | None = None
    categorical_value: str | None = None
    embedding: tuple[float, ...] | None = None

    def unwrap_numeric(self) -> float:
        if self.kind not in ("numeric", "binary"):
            raise ValueError(
                f"feature {self.feature_name!r} is kind={self.kind!r}, "
                "not numeric/binary",
            )
        if self.numeric_value is None:
            raise ValueError(
                f"feature {self.feature_name!r} has no numeric_value",
            )
        return self.numeric_value

    def unwrap_categorical(self) -> str:
        if self.kind != "categorical":
            raise ValueError(
                f"feature {self.feature_name!r} is kind={self.kind!r}, "
                "not categorical",
            )
        if self.categorical_value is None:
            raise ValueError(
                f"feature {self.feature_name!r} has no categorical_value",
            )
        return self.categorical_value

    def unwrap_embedding(self) -> tuple[float, ...]:
        if self.kind != "embedding":
            raise ValueError(
                f"feature {self.feature_name!r} is kind={self.kind!r}, "
                "not embedding",
            )
        if self.embedding is None:
            raise ValueError(
                f"feature {self.feature_name!r} has no embedding",
            )
        return self.embedding


# ---------------------------------------------------------------------------
# Feature sample (FeatureValue + timing + context)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class FeatureSample:
    """A :class:`FeatureValue` enriched with the observation context.

    This is the shape that the ingestion pipeline pushes into the
    hypertable ``raw_events``. The fields that never change across
    a single :class:`InferenceEvent` (tenant, task, observed_at) are
    copied here for simplicity at write time.
    """

    sample_id: str
    tenant_id: str
    task_id: str
    feature_id: str
    feature_name: str
    observed_at: datetime
    value: FeatureValue
    # Optional : hashed user_id for audit (k-anonymous)
    user_id_hash: str | None = None
    request_id: str | None = None


# ---------------------------------------------------------------------------
# Snapshot (aggregated histogram for a bucket)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class FeatureSnapshot:
    """Aggregated representation of a feature over a time bucket.

    The union of fields covers the three kinds of features :

    * **numeric / binary** : ``histogram`` (list of per-bin counts) +
      ``stats`` (mean, std, quantiles).
    * **categorical** : ``categorical_counts`` (dict of top-K + other).
    * **embedding** : ``embedding_centroid`` + ``embedding_std`` +
      optional ``embedding_ref`` (blob URL for reservoir samples).
    """

    snapshot_id: str
    tenant_id: str
    task_id: str
    feature_id: str
    feature_name: str
    feature_kind: FeatureKind
    bucket_kind: BucketKind
    bucket_start: datetime
    bucket_end: datetime
    sample_count: int
    histogram: tuple[int, ...] | None = None
    stats: dict[str, float] = field(default_factory=dict)
    categorical_counts: dict[str, int] = field(default_factory=dict)
    embedding_centroid: tuple[float, ...] | None = None
    embedding_std: tuple[float, ...] | None = None
    embedding_ref: str | None = None

    def __post_init__(self) -> None:
        if self.sample_count < 0:
            raise ValueError(
                f"sample_count must be non-negative, got {self.sample_count}",
            )
        if self.bucket_end <= self.bucket_start:
            raise ValueError(
                f"bucket_end {self.bucket_end!r} must be > bucket_start "
                f"{self.bucket_start!r}",
            )


# ---------------------------------------------------------------------------
# Baseline spec + runtime view
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class BaselineSpec:
    """Canonical, seal-input shape of a baseline.

    This is what :func:`efe_drift.hashing.content_hash` is applied to
    when producing the ``spec_hash``. Fields that change at runtime
    (state, timestamps, sample counts from later ingestion windows)
    are *not* part of the spec — only the frozen configuration is.
    """

    tenant_id: str
    task_id: str
    feature_name: str
    feature_kind: FeatureKind
    window_start: datetime
    window_end: datetime
    sample_count: int
    bins: tuple[float, ...] | None = None
    # ``baseline_histogram`` (numeric / binary) is aligned 1-to-1 with
    # ``bins`` (``len == len(bins) - 1``). When present, detectors use
    # these counts as the exact reference distribution. Falling back to
    # a uniform reconstruction (as we did before persisting this field)
    # is only correct for equifrequency baselines.
    baseline_histogram: tuple[int, ...] | None = None
    categorical_categories: tuple[str, ...] | None = None
    # ``baseline_categorical_counts`` keys are the categories in
    # ``categorical_categories`` plus an optional ``"__other__"`` entry
    # for mass observed outside the top-K. Mirrors the shape produced
    # by :meth:`CategoricalCounter.top_k_with_other`.
    baseline_categorical_counts: dict[str, int] | None = None
    centroid: tuple[float, ...] | None = None
    std: tuple[float, ...] | None = None
    stats: dict[str, float] = field(default_factory=dict)
    model_artifact_id: str | None = None
    overlap_days: int = 3
    notes: str | None = None

    def to_canonical_dict(self) -> dict[str, Any]:
        return {
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "feature_name": self.feature_name,
            "feature_kind": self.feature_kind,
            "window_start": self.window_start.isoformat(),
            "window_end": self.window_end.isoformat(),
            "sample_count": self.sample_count,
            "bins": list(self.bins) if self.bins else None,
            "baseline_histogram": (
                list(self.baseline_histogram)
                if self.baseline_histogram is not None
                else None
            ),
            "categorical_categories": (
                list(self.categorical_categories)
                if self.categorical_categories
                else None
            ),
            "baseline_categorical_counts": (
                dict(sorted(self.baseline_categorical_counts.items()))
                if self.baseline_categorical_counts is not None
                else None
            ),
            "centroid": list(self.centroid) if self.centroid else None,
            "std": list(self.std) if self.std else None,
            "stats": dict(sorted(self.stats.items())),
            "model_artifact_id": self.model_artifact_id,
            "overlap_days": self.overlap_days,
            "notes": self.notes,
        }


@dataclass(frozen=True, slots=True)
class Baseline:
    """Runtime view of a baseline row.

    The ``state`` and ``state_version`` fields track the FSM; every
    other field is derived from the sealed :class:`BaselineSpec`.
    """

    baseline_id: str
    spec: BaselineSpec
    state: BaselineState
    state_version: int
    spec_hash: bytes | None = None
    signing_key_id: str | None = None
    signature: bytes | None = None
    created_at: datetime | None = None
    promoted_at: datetime | None = None
    retired_at: datetime | None = None
    planned_end_at: datetime | None = None


# ---------------------------------------------------------------------------
# Feature catalog entry (in-memory)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class FeatureEntry:
    """In-memory record of a registered feature."""

    feature_id: str
    tenant_id: str
    task_id: str
    name: str
    version: int
    kind: FeatureKind
    extractor: str
    binning_strategy: BinningStrategy
    expected_cardinality: int | None = None
    retention_days: int = 90
    created_at: datetime | None = None
    created_by: str = "system"
    enabled: bool = True
    notes: str | None = None


__all__ = [
    "Baseline",
    "BaselineSpec",
    "BaselineState",
    "BinningStrategy",
    "BucketKind",
    "FeatureEntry",
    "FeatureKind",
    "FeatureSample",
    "FeatureSnapshot",
    "FeatureValue",
    "InferenceEvent",
]
