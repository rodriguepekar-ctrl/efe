"""Wall-clock deadline computation."""

from __future__ import annotations

from datetime import datetime, timedelta

from efe_swarm_integration.config import NETWORK_MARGIN_MS
from efe_swarm_integration.types import SwarmSpec

__all__ = ["compute_deadline"]


def compute_deadline(
    spec: SwarmSpec,
    *,
    now: datetime,
    extra_margin_ms: int = 0,
) -> datetime:
    """Return a tz-aware UTC deadline that gives the server
    ``spec.budget.max_wall_clock_ms + NETWORK_MARGIN_MS + extra`` ms
    to respond."""
    if now.tzinfo is None:
        raise ValueError("now must be tz-aware")
    total_ms = spec.budget.max_wall_clock_ms + NETWORK_MARGIN_MS + extra_margin_ms
    return now + timedelta(milliseconds=total_ms)
