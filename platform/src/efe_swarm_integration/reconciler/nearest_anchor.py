"""Nearest-anchor decoder post-hook (M7.5 mirror)."""

from __future__ import annotations

import math
from typing import Sequence

__all__ = ["nearest_anchor_label"]


def nearest_anchor_label(
    centroid: Sequence[float],
    *,
    anchors: Sequence[Sequence[float]],
    labels: Sequence[str],
) -> tuple[str, int, float]:
    """Return ``(label, idx, distance)`` for the closest anchor."""
    if not anchors:
        raise ValueError("nearest_anchor_label requires at least one anchor")
    if len(anchors) != len(labels):
        raise ValueError("anchors and labels length mismatch")
    best_idx = 0
    best_d = math.inf
    for i, anchor in enumerate(anchors):
        if len(anchor) != len(centroid):
            raise ValueError(f"anchor {i} dim mismatch")
        d = math.sqrt(sum((a - c) ** 2 for a, c in zip(anchor, centroid)))
        if d < best_d:
            best_d = d
            best_idx = i
    return labels[best_idx], best_idx, best_d
