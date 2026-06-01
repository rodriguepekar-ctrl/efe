"""Per-class threshold calibration helpers."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

import numpy as np

from .stats import recommend_threshold_for_target


@dataclass(frozen=True)
class PerClassCalibration:
    class_name: str
    current_threshold: float
    recommended_threshold: float
    expected_precision: float
    expected_recall: float
    expected_f1: float
    samples_used: int


def calibrate_per_class(
    *,
    classes: list[dict],
    score_distributions: dict[str, dict],   # class_name -> {positives, negatives}
    target_metric: str = "f1",
    target_value: float = 0.9,
) -> list[PerClassCalibration]:
    """Compute recommended thresholds per class.

    `score_distributions` provides {positives: list[float], negatives: list[float]}
    for each class — typically built by sampling N items via `score_distribution`.
    """
    out: list[PerClassCalibration] = []
    for cls in classes:
        name = cls["name"]
        current = float(cls.get("confidence_threshold", 0.5))
        dist = score_distributions.get(name)
        if dist is None or not dist.get("positives"):
            out.append(
                PerClassCalibration(
                    class_name=name,
                    current_threshold=current,
                    recommended_threshold=current,
                    expected_precision=0.0,
                    expected_recall=0.0,
                    expected_f1=0.0,
                    samples_used=0,
                )
            )
            continue
        rec = recommend_threshold_for_target(
            positive_scores=dist["positives"],
            negative_scores=dist.get("negatives", []),
            target_metric=target_metric,
            target_value=target_value,
        )
        n_pos = len(dist["positives"])
        n_neg = len(dist.get("negatives", []))
        out.append(
            PerClassCalibration(
                class_name=name,
                current_threshold=current,
                recommended_threshold=rec.threshold,
                expected_precision=rec.expected_precision,
                expected_recall=rec.expected_recall,
                expected_f1=rec.f1,
                samples_used=n_pos + n_neg,
            )
        )
    return out
