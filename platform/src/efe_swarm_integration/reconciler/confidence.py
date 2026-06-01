"""Confidence computation.

The business confidence is a monotone decreasing function of
``alignment_delta / cone.max_projection_distance`` clamped to [0, 1].
"""

from __future__ import annotations

__all__ = ["compute_confidence", "confidence_from_alignment"]


def confidence_from_alignment(delta: float, radius: float) -> float:
    """Return ``max(0, 1 - delta/radius)`` clamped to [0, 1]."""
    if radius <= 0.0:
        return 0.0
    raw = 1.0 - (delta / radius)
    if raw < 0.0:
        return 0.0
    if raw > 1.0:
        return 1.0
    return raw


def compute_confidence(
    *,
    alignment_delta: float,
    cone_radius: float,
    server_reported_confidence: float | None = None,
) -> float:
    """Prefer the server-reported confidence when present (it has more
    context) ; otherwise fall back to the alignment-delta heuristic."""
    if server_reported_confidence is not None:
        # clamp
        return max(0.0, min(1.0, float(server_reported_confidence)))
    return confidence_from_alignment(alignment_delta, cone_radius)
