"""Statistical helpers — histograms, percentiles, threshold optimisation."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

import numpy as np


@dataclass(frozen=True)
class Histogram:
    buckets: list[float]      # bin edges (n+1)
    positives: list[int]      # counts in bin (length n)
    negatives: list[int]      # may be all zeros


def histogram(
    *,
    positive_scores: Sequence[float],
    negative_scores: Sequence[float] | None = None,
    bin_count: int = 10,
) -> Histogram:
    """10-bucket histogram on [0.0, 1.0] for positive + optional negative scores."""
    edges = np.linspace(0.0, 1.0, bin_count + 1)
    pos_counts, _ = np.histogram(np.asarray(positive_scores, dtype=np.float32), bins=edges)
    if negative_scores is not None:
        neg_counts, _ = np.histogram(np.asarray(negative_scores, dtype=np.float32), bins=edges)
    else:
        neg_counts = np.zeros(bin_count, dtype=np.int64)
    return Histogram(
        buckets=[float(e) for e in edges],
        positives=[int(c) for c in pos_counts],
        negatives=[int(c) for c in neg_counts],
    )


def percentiles(
    scores: Sequence[float], qs: Sequence[float] = (0.1, 0.5, 0.9, 0.99)
) -> dict[str, float]:
    """Return {pNN: value} dict ; empty input returns 0 for all qs."""
    if len(scores) == 0:
        return {f"p{int(q*100)}": 0.0 for q in qs}
    arr = np.asarray(scores, dtype=np.float32)
    return {f"p{int(q*100)}": round(float(np.quantile(arr, q)), 4) for q in qs}


# ─────────────────────────────────────────────────────────────────────
# Threshold recommendation
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class ThresholdRecommendation:
    threshold: float
    expected_precision: float
    expected_recall: float
    f1: float


def _precision_recall_at(threshold: float, pos: np.ndarray, neg: np.ndarray) -> tuple[float, float]:
    tp = float(np.sum(pos >= threshold))
    fn = float(np.sum(pos < threshold))
    fp = float(np.sum(neg >= threshold))
    tn = float(np.sum(neg < threshold))
    precision = tp / (tp + fp) if (tp + fp) > 0 else 0.0
    recall = tp / (tp + fn) if (tp + fn) > 0 else 0.0
    return precision, recall


def recommend_threshold_f1(
    *,
    positive_scores: Sequence[float],
    negative_scores: Sequence[float],
    step: float = 0.01,
) -> ThresholdRecommendation:
    """Return the threshold (in [0,1]) that maximises F1.

    With no negatives we cannot compute precision ; the routine then
    returns the median positive score as a sane default with precision=1.
    """
    pos = np.asarray(positive_scores, dtype=np.float32)
    neg = np.asarray(negative_scores, dtype=np.float32)
    if len(pos) == 0:
        return ThresholdRecommendation(0.5, 0.0, 0.0, 0.0)
    if len(neg) == 0:
        threshold = float(np.median(pos))
        return ThresholdRecommendation(round(threshold, 3), 1.0, 1.0, 1.0)

    best = ThresholdRecommendation(0.5, 0.0, 0.0, -1.0)
    t = 0.0
    while t <= 1.0 + 1e-9:
        p, r = _precision_recall_at(t, pos, neg)
        f1 = (2 * p * r / (p + r)) if (p + r) > 0 else 0.0
        if f1 > best.f1:
            best = ThresholdRecommendation(round(t, 3), round(p, 4), round(r, 4), round(f1, 4))
        t += step
    return best


def recommend_threshold_for_target(
    *,
    positive_scores: Sequence[float],
    negative_scores: Sequence[float],
    target_metric: str,
    target_value: float,
    step: float = 0.01,
) -> ThresholdRecommendation:
    """Return the smallest threshold that meets `target_metric >= target_value`.

    `target_metric` ∈ {"precision", "recall", "f1"}. If no threshold
    satisfies, returns the threshold that gets closest to the target
    (with the best F1 amongst close-but-not-quite options).
    """
    pos = np.asarray(positive_scores, dtype=np.float32)
    neg = np.asarray(negative_scores, dtype=np.float32)
    if target_metric not in ("precision", "recall", "f1"):
        raise ValueError(f"unknown target_metric: {target_metric!r}")
    if len(pos) == 0:
        return ThresholdRecommendation(0.5, 0.0, 0.0, 0.0)

    best_meeting: ThresholdRecommendation | None = None
    best_close: ThresholdRecommendation = ThresholdRecommendation(0.5, 0.0, 0.0, 0.0)
    t = 0.0
    while t <= 1.0 + 1e-9:
        p, r = _precision_recall_at(t, pos, neg) if len(neg) > 0 else (1.0, 1.0)
        f1 = (2 * p * r / (p + r)) if (p + r) > 0 else 0.0
        rec = ThresholdRecommendation(round(t, 3), round(p, 4), round(r, 4), round(f1, 4))
        metric_value = {"precision": p, "recall": r, "f1": f1}[target_metric]
        if metric_value >= target_value:
            # Smallest threshold meeting target — track first one found.
            if best_meeting is None or rec.threshold < best_meeting.threshold:
                best_meeting = rec
        if rec.f1 > best_close.f1:
            best_close = rec
        t += step
    return best_meeting if best_meeting is not None else best_close
