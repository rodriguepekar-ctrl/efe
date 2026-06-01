"""Baseline candidate — accumulation structure before promotion.

A :class:`BaselineCandidate` is the thing you populate during the
``PLANNED → ACCUMULATING → CANDIDATE`` phase of the FSM. It collects
samples over N days and can be "closed" into a
:class:`~efe_drift.records.BaselineSpec` that is then sanity-checked
and sealed.

Per feature kind :

* **numeric / binary** — stores raw samples + computes equifrequency
  edges on close (unless the caller pre-supplies custom edges).
* **categorical** — runs a :class:`CategoricalCounter` and on close
  produces the frozen top-K list.
* **embedding** — feeds an :class:`EmbeddingReservoir` and on close
  produces centroid + diagonal std.

The candidate tracks coverage — how many distinct days it has seen
— so :class:`BaselineValidator` can assert the minimum window.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from datetime import datetime, timezone

from efe_drift.aggregation.categorical import (
    OTHER_BUCKET,
    CategoricalCounter,
)
from efe_drift.aggregation.histogram import (
    custom_bins,
    equifrequency_bins,
    equiwidth_bins,
)
from efe_drift.aggregation.reservoir import EmbeddingReservoir
from efe_drift.errors import (
    EmptySampleSet,
    ExtractorTypeMismatch,
    ValidationError,
)
from efe_drift.records import (
    BaselineSpec,
    BinningStrategy,
    FeatureEntry,
    FeatureSample,
)


@dataclass(slots=True)
class BaselineCandidate:
    """Accumulation buffer + summariser for a single feature."""

    entry: FeatureEntry
    window_start: datetime
    window_end: datetime
    # numeric / binary
    numeric_samples: list[float] = field(default_factory=list)
    # categorical
    cat_counter: CategoricalCounter = field(default_factory=CategoricalCounter)
    # embedding
    reservoir: EmbeddingReservoir | None = None
    # coverage tracking
    days_seen: set[str] = field(default_factory=set)
    # Last wall-clock seen — used by the manager to detect stagnant
    # candidates.
    last_observed_at: datetime | None = None

    def __post_init__(self) -> None:
        if self.window_start.tzinfo is None:
            raise ValidationError("window_start must be tz-aware")
        if self.window_end.tzinfo is None:
            raise ValidationError("window_end must be tz-aware")
        if self.window_end <= self.window_start:
            raise ValidationError(
                f"window_end {self.window_end!r} must be > window_start "
                f"{self.window_start!r}",
            )
        if self.entry.kind == "embedding":
            self.reservoir = EmbeddingReservoir(size=10_000)

    @property
    def sample_count(self) -> int:
        if self.entry.kind in ("numeric", "binary"):
            return len(self.numeric_samples)
        if self.entry.kind == "categorical":
            return self.cat_counter.total
        return self.reservoir.seen if self.reservoir else 0

    @property
    def days_covered(self) -> int:
        return len(self.days_seen)

    # ---- ingest --------------------------------------------------------

    def ingest(self, sample: FeatureSample) -> None:
        if sample.feature_id != self.entry.feature_id:
            raise ValidationError(
                f"sample belongs to feature {sample.feature_id!r}, "
                f"candidate tracks {self.entry.feature_id!r}",
            )
        if sample.value.kind != self.entry.kind:
            raise ExtractorTypeMismatch(
                f"expected kind {self.entry.kind!r}, "
                f"got {sample.value.kind!r}",
            )
        observed = sample.observed_at.astimezone(timezone.utc)
        if observed < self.window_start or observed >= self.window_end:
            # Silently ignore samples outside the window rather than
            # raising — the rollup job can replay arbitrary ranges.
            return
        self.last_observed_at = observed
        self.days_seen.add(observed.date().isoformat())
        if self.entry.kind in ("numeric", "binary"):
            self.numeric_samples.append(sample.value.unwrap_numeric())
        elif self.entry.kind == "categorical":
            self.cat_counter.add(sample.value.unwrap_categorical())
        else:  # embedding
            assert self.reservoir is not None
            self.reservoir.add(sample.value.unwrap_embedding())

    # ---- close + summary ----------------------------------------------

    def close(
        self,
        *,
        n_bins: int = 10,
        top_k: int = 10,
        custom_edges: tuple[float, ...] | None = None,
        model_artifact_id: str | None = None,
        overlap_days: int = 3,
        notes: str | None = None,
    ) -> BaselineSpec:
        """Materialise the candidate into a :class:`BaselineSpec`.

        The returned spec is the canonical input to the seal/signature
        step in :class:`BaselineManager.promote`.
        """
        if self.sample_count == 0:
            raise EmptySampleSet(
                "candidate has no samples — cannot close",
            )
        if self.entry.kind in ("numeric", "binary"):
            bins = self._resolve_numeric_bins(
                strategy=self.entry.binning_strategy,
                n_bins=n_bins,
                custom_edges=custom_edges,
            )
            stats = self._numeric_stats()
            return BaselineSpec(
                tenant_id=self.entry.tenant_id,
                task_id=self.entry.task_id,
                feature_name=self.entry.name,
                feature_kind=self.entry.kind,
                window_start=self.window_start,
                window_end=self.window_end,
                sample_count=self.sample_count,
                bins=bins,
                stats=stats,
                model_artifact_id=model_artifact_id,
                overlap_days=overlap_days,
                notes=notes,
            )
        if self.entry.kind == "categorical":
            cats = self._resolve_categorical_top_k(top_k=top_k)
            return BaselineSpec(
                tenant_id=self.entry.tenant_id,
                task_id=self.entry.task_id,
                feature_name=self.entry.name,
                feature_kind=self.entry.kind,
                window_start=self.window_start,
                window_end=self.window_end,
                sample_count=self.sample_count,
                categorical_categories=cats,
                stats=self._categorical_stats(),
                model_artifact_id=model_artifact_id,
                overlap_days=overlap_days,
                notes=notes,
            )
        # embedding
        assert self.reservoir is not None
        centroid = self.reservoir.centroid()
        std = self.reservoir.std()
        return BaselineSpec(
            tenant_id=self.entry.tenant_id,
            task_id=self.entry.task_id,
            feature_name=self.entry.name,
            feature_kind=self.entry.kind,
            window_start=self.window_start,
            window_end=self.window_end,
            sample_count=self.sample_count,
            centroid=centroid,
            std=std,
            stats={
                "dim": float(self.reservoir.dim or 0),
                "seen": float(self.reservoir.seen),
            },
            model_artifact_id=model_artifact_id,
            overlap_days=overlap_days,
            notes=notes,
        )

    # ---- internals -----------------------------------------------------

    def _resolve_numeric_bins(
        self,
        *,
        strategy: BinningStrategy,
        n_bins: int,
        custom_edges: tuple[float, ...] | None,
    ) -> tuple[float, ...]:
        if strategy == "none":
            raise ValidationError(
                "numeric features cannot use binning_strategy='none' — "
                "choose equifrequency, equiwidth, or custom",
            )
        if strategy == "custom":
            if custom_edges is None:
                raise ValidationError(
                    "binning_strategy='custom' requires custom_edges",
                )
            return custom_bins(custom_edges)
        if strategy == "equifrequency":
            return equifrequency_bins(self.numeric_samples, n_bins=n_bins)
        if strategy == "equiwidth":
            return equiwidth_bins(self.numeric_samples, n_bins=n_bins)
        raise ValidationError(f"unknown binning strategy {strategy!r}")

    def _numeric_stats(self) -> dict[str, float]:
        n = len(self.numeric_samples)
        if n == 0:
            return {}
        mean = sum(self.numeric_samples) / n
        var = (
            sum((v - mean) ** 2 for v in self.numeric_samples) / (n - 1)
            if n > 1 else 0.0
        )
        s = sorted(self.numeric_samples)
        return {
            "n": float(n),
            "mean": mean,
            "std": math.sqrt(var),
            "min": s[0],
            "p50": _percentile(s, 0.50),
            "p95": _percentile(s, 0.95),
            "max": s[-1],
        }

    def _categorical_stats(self) -> dict[str, float]:
        total = self.cat_counter.total
        if total == 0:
            return {}
        return {
            "n": float(total),
            "cardinality": float(self.cat_counter.cardinality),
        }

    def _resolve_categorical_top_k(self, *, top_k: int) -> tuple[str, ...]:
        tk = self.cat_counter.top_k_with_other(k=top_k)
        # The frozen list excludes the OTHER_BUCKET sentinel — it is
        # synthesised at projection time. We return the names
        # alphabetically sorted so the seal hash stays deterministic
        # regardless of dict insertion order.
        return tuple(sorted(n for n in tk if n != OTHER_BUCKET))


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _percentile(sorted_values: list[float], q: float) -> float:
    if not sorted_values:
        return float("nan")
    n = len(sorted_values)
    if n == 1:
        return sorted_values[0]
    idx = q * (n - 1)
    lo = math.floor(idx)
    frac = idx - lo
    if lo + 1 >= n:
        return sorted_values[-1]
    return sorted_values[lo] * (1 - frac) + sorted_values[lo + 1] * frac


__all__ = ["BaselineCandidate"]
