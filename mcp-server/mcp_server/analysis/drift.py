"""Drift detection — compare two catalog versions."""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class ClassDrift:
    class_name: str
    centroid_shift_cosine: float    # 0 = no shift, 2 = total flip
    examples_added: int
    examples_removed: int
    size_a: int
    size_b: int
    drift_severity: str             # none | low | moderate | high | critical


def _severity(shift: float) -> str:
    if shift < 0.01:
        return "none"
    if shift < 0.05:
        return "low"
    if shift < 0.15:
        return "moderate"
    if shift < 0.30:
        return "high"
    return "critical"


def per_class_drift(
    *,
    classes_a: list[dict],
    classes_b: list[dict],
) -> tuple[list[ClassDrift], list[str], list[str]]:
    """Compute per-class drift between version A and B.

    Returns (drifts, new_class_names, removed_class_names).
    """
    by_name_a = {c["name"]: c for c in classes_a}
    by_name_b = {c["name"]: c for c in classes_b}

    drifts: list[ClassDrift] = []
    for name, cls_a in by_name_a.items():
        if name not in by_name_b:
            continue
        cls_b = by_name_b[name]
        ca = np.asarray(cls_a["centroid"], dtype=np.float32)
        cb = np.asarray(cls_b["centroid"], dtype=np.float32)
        # cosine similarity of two L2-normalized vectors lies in [-1, 1].
        # Shift = 1 - cos. (range [0, 2])
        na = float(np.linalg.norm(ca))
        nb = float(np.linalg.norm(cb))
        if na == 0 or nb == 0:
            shift = 1.0
        else:
            cos = float(np.dot(ca, cb) / (na * nb))
            shift = max(0.0, 1.0 - cos)

        ex_a = set(cls_a.get("examples", []))
        ex_b = set(cls_b.get("examples", []))
        added = len(ex_b - ex_a)
        removed = len(ex_a - ex_b)
        drifts.append(
            ClassDrift(
                class_name=name,
                centroid_shift_cosine=round(shift, 4),
                examples_added=added,
                examples_removed=removed,
                size_a=len(ex_a),
                size_b=len(ex_b),
                drift_severity=_severity(shift),
            )
        )

    new_classes = sorted(set(by_name_b.keys()) - set(by_name_a.keys()))
    removed_classes = sorted(set(by_name_a.keys()) - set(by_name_b.keys()))
    return drifts, new_classes, removed_classes


def overall_drift_score(drifts: list[ClassDrift]) -> float:
    """Aggregate drift score in [0, 1] = mean centroid shift across classes."""
    if not drifts:
        return 0.0
    return round(float(np.mean([d.centroid_shift_cosine for d in drifts])), 4)


def drift_recommendation(overall: float, n_new: int, n_removed: int) -> str:
    """Plain-English recommendation for the agent."""
    if n_removed > 0:
        return f"{n_removed} class(es) removed ; verify downstream consumers before rolling forward."
    if overall < 0.05:
        return "Drift is negligible ; rolling forward is safe."
    if overall < 0.15:
        return "Drift is low ; rolling forward should be safe ; consider re-calibrating thresholds."
    if overall < 0.30:
        return "Drift is moderate ; recommend running efe_calibrate_thresholds before promoting."
    return "Drift is high or critical ; recommend manual review + diagnose_catalog before promoting."
