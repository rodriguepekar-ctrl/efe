"""Data classes for per-metric observations.

Three levels:

* :class:`SampleScores` — the raw per-sample scores for one metric
  (e.g. ``[1, 0, 1, 1, 0]`` for binary correctness on 5 samples).
  Stored as a numpy array to feed bootstrap / McNemar cheaply.
* :class:`MetricBundle` — one metric after aggregation: ``value``,
  ``ci_lower``, ``ci_upper``, ``sample_size``. This is what the
  comparator consumes.
* :class:`MetricsBundle` — the full envelope for one evaluation:
  dict of metric name → MetricBundle, plus per-group splits for
  fairness checks.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any

import numpy as np


class MetricRole(str, Enum):
    PRIMARY = "primary"
    SECONDARY = "secondary"
    BUDGET = "budget"
    FAIRNESS = "fairness"


# ---------------------------------------------------------------------------
# SampleScores — per-sample raw observations
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SampleScores:
    """Per-sample observations for one metric.

    ``values`` is a 1-D numpy array; each entry is the metric's
    contribution from one sample. Examples:

    * Accuracy — 0/1 per sample (``int`` array).
    * F1-like — we still store per-sample binary correctness on
      the *predicted* label and combine in the aggregator.
    * Recall@k — 0/1 per query.
    * Latency ms — float per sample.
    """

    metric_name: str
    values: np.ndarray
    # Optional per-sample group key (for fairness splits).
    groups: np.ndarray | None = None

    def __post_init__(self) -> None:
        if self.values.ndim != 1:
            raise ValueError(
                f"SampleScores.values must be 1-D, got shape {self.values.shape}",
            )
        if self.groups is not None and len(self.groups) != len(self.values):
            raise ValueError("groups length mismatch with values length")

    @property
    def n(self) -> int:
        return int(self.values.shape[0])

    def mean(self) -> float:
        if self.n == 0:
            return 0.0
        return float(self.values.mean())

    def with_group_mask(self, group: str) -> SampleScores:
        if self.groups is None:
            raise ValueError("no groups attached")
        mask = self.groups == group
        return SampleScores(
            metric_name=self.metric_name,
            values=self.values[mask],
            groups=None,
        )

    def group_keys(self) -> list[str]:
        if self.groups is None:
            return []
        return [str(g) for g in np.unique(self.groups)]


# ---------------------------------------------------------------------------
# MetricBundle — one aggregated metric
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class MetricBundle:
    name: str
    role: MetricRole
    value: float
    ci_lower: float | None = None
    ci_upper: float | None = None
    sample_size: int = 0
    higher_is_better: bool = True

    def as_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "role": self.role.value,
            "value": float(self.value),
            "ci_lower": (
                None if self.ci_lower is None else float(self.ci_lower)
            ),
            "ci_upper": (
                None if self.ci_upper is None else float(self.ci_upper)
            ),
            "sample_size": int(self.sample_size),
            "higher_is_better": self.higher_is_better,
        }


# ---------------------------------------------------------------------------
# MetricsBundle — full envelope for one evaluation
# ---------------------------------------------------------------------------


@dataclass
class MetricsBundle:
    """Output of the runner stage, input to the comparator."""

    metrics: dict[str, MetricBundle] = field(default_factory=dict)
    # Per-metric per-sample raw scores, keyed by metric_name.
    raw: dict[str, SampleScores] = field(default_factory=dict)
    # Per-group bundles (fairness). Key = group_name → { metric_name → MetricBundle }
    by_group: dict[str, dict[str, MetricBundle]] = field(default_factory=dict)

    def add(self, bundle: MetricBundle) -> None:
        self.metrics[bundle.name] = bundle

    def add_raw(self, scores: SampleScores) -> None:
        self.raw[scores.metric_name] = scores

    def add_group(
        self, group: str, bundle: MetricBundle,
    ) -> None:
        self.by_group.setdefault(group, {})[bundle.name] = bundle

    def get(self, name: str) -> MetricBundle:
        return self.metrics[name]

    def has(self, name: str) -> bool:
        return name in self.metrics

    def names(self) -> list[str]:
        return sorted(self.metrics)

    def role(self, name: str) -> MetricRole:
        return self.metrics[name].role

    def filter_role(self, role: MetricRole) -> list[MetricBundle]:
        return [m for m in self.metrics.values() if m.role is role]

    def as_dict(self) -> dict[str, Any]:
        return {
            "metrics": {k: v.as_dict() for k, v in self.metrics.items()},
            "by_group": {
                g: {k: v.as_dict() for k, v in grp.items()}
                for g, grp in self.by_group.items()
            },
        }


__all__ = [
    "MetricBundle",
    "MetricRole",
    "MetricsBundle",
    "SampleScores",
]
