"""Metrics harness — accuracy, F1 macro, latency percentiles, calibration.

Everything here is deterministic given a fixed prediction list.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
from statistics import mean
from typing import Protocol

import numpy as np

from benchmarks.dataset import CLASS_NAMES, Query


class Prediction(Protocol):
    qid: str
    predicted: str
    confidence: float
    latency_us: float


# ---------------------------------------------------------------------------
#  Result buckets
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ClassifierMetrics:
    """All the numbers the PDF needs for one classifier on one slice."""

    name: str
    slice_label: str  # "easy" | "hard" | "all"
    n_queries: int
    accuracy: float
    macro_f1: float
    per_class_f1: dict[str, float]
    #: Latency percentiles in microseconds.
    latency_p50_us: float
    latency_p95_us: float
    latency_p99_us: float
    latency_mean_us: float
    #: Expected Calibration Error (10-bin). Lower = better-calibrated
    #: confidence. 0 = perfect.
    ece: float
    #: Confusion matrix as nested dict[true][pred] = count.
    confusion: dict[str, dict[str, int]]
    #: Optional extra fields (swarm-only).
    extras: dict[str, float] = field(default_factory=dict)

    def to_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "slice_label": self.slice_label,
            "n_queries": self.n_queries,
            "accuracy": round(self.accuracy, 4),
            "macro_f1": round(self.macro_f1, 4),
            "per_class_f1": {c: round(v, 4) for c, v in self.per_class_f1.items()},
            "latency_us": {
                "p50": round(self.latency_p50_us, 2),
                "p95": round(self.latency_p95_us, 2),
                "p99": round(self.latency_p99_us, 2),
                "mean": round(self.latency_mean_us, 2),
            },
            "ece": round(self.ece, 4),
            "confusion": self.confusion,
            "extras": {k: round(v, 4) for k, v in self.extras.items()},
        }


# ---------------------------------------------------------------------------
#  Metric computation
# ---------------------------------------------------------------------------


def evaluate(
    *,
    name: str,
    slice_label: str,
    queries: Sequence[Query],
    predictions: Sequence[Prediction],
    extras: dict[str, float] | None = None,
) -> ClassifierMetrics:
    """Compute the full metric bundle for a (classifier, slice) pair."""
    if len(queries) != len(predictions):
        raise ValueError("queries and predictions must be aligned")

    pairs = list(zip(queries, predictions, strict=True))
    n = len(pairs)

    # Accuracy.
    correct = sum(1 for q, p in pairs if q.label == p.predicted)
    accuracy = correct / n if n else 0.0

    # Confusion matrix + per-class precision/recall/F1.
    confusion: dict[str, dict[str, int]] = {
        c: {c2: 0 for c2 in CLASS_NAMES} for c in CLASS_NAMES
    }
    for q, p in pairs:
        confusion[q.label][p.predicted] = confusion[q.label].get(p.predicted, 0) + 1

    per_class_f1: dict[str, float] = {}
    for c in CLASS_NAMES:
        tp = confusion[c][c]
        fn = sum(confusion[c][cc] for cc in CLASS_NAMES if cc != c)
        fp = sum(confusion[cc][c] for cc in CLASS_NAMES if cc != c)
        precision = tp / (tp + fp) if (tp + fp) else 0.0
        recall = tp / (tp + fn) if (tp + fn) else 0.0
        f1 = 2 * precision * recall / (precision + recall) if (precision + recall) else 0.0
        per_class_f1[c] = f1
    macro_f1 = mean(per_class_f1.values()) if per_class_f1 else 0.0

    # Latency.
    lat = np.array([p.latency_us for _, p in pairs])
    latency_p50 = float(np.percentile(lat, 50)) if len(lat) else 0.0
    latency_p95 = float(np.percentile(lat, 95)) if len(lat) else 0.0
    latency_p99 = float(np.percentile(lat, 99)) if len(lat) else 0.0
    latency_mean = float(lat.mean()) if len(lat) else 0.0

    # ECE (expected calibration error, 10 bins).
    ece = _expected_calibration_error(
        confidences=np.array([p.confidence for _, p in pairs]),
        correct=np.array([q.label == p.predicted for q, p in pairs], dtype=np.float64),
        n_bins=10,
    )

    return ClassifierMetrics(
        name=name,
        slice_label=slice_label,
        n_queries=n,
        accuracy=accuracy,
        macro_f1=macro_f1,
        per_class_f1=per_class_f1,
        latency_p50_us=latency_p50,
        latency_p95_us=latency_p95,
        latency_p99_us=latency_p99,
        latency_mean_us=latency_mean,
        ece=ece,
        confusion=confusion,
        extras=dict(extras or {}),
    )


def _expected_calibration_error(
    *, confidences: np.ndarray, correct: np.ndarray, n_bins: int = 10
) -> float:
    """Standard ECE: sum_b (|B_b|/N) * |acc(B_b) − conf(B_b)|."""
    if len(confidences) == 0:
        return 0.0
    bins = np.linspace(0.0, 1.0, n_bins + 1)
    ece = 0.0
    n = len(confidences)
    for i in range(n_bins):
        in_bin = (confidences > bins[i]) & (confidences <= bins[i + 1])
        if i == 0:
            in_bin |= confidences == 0.0
        count = int(in_bin.sum())
        if count == 0:
            continue
        bin_acc = float(correct[in_bin].mean())
        bin_conf = float(confidences[in_bin].mean())
        ece += (count / n) * abs(bin_acc - bin_conf)
    return ece


__all__ = ["ClassifierMetrics", "Prediction", "evaluate"]
