"""Canary stage and report models."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime

__all__ = ["CanaryReport", "CanaryStatus", "CanaryTrafficStage"]


class CanaryStatus(str, enum.Enum):
    observing = "observing"
    healthy = "healthy"
    unhealthy = "unhealthy"


@dataclass(frozen=True, slots=True)
class CanaryTrafficStage:
    """One stage of the canary ramp."""

    split_ppm: int
    hold_s: int

    def __post_init__(self) -> None:
        if not 0 < self.split_ppm <= 1_000_000:
            raise ValueError("split_ppm must be in (0, 1_000_000]")
        if self.hold_s < 0:
            raise ValueError("hold_s must be >= 0s")


DEFAULT_CANARY_RAMP: tuple[CanaryTrafficStage, ...] = (
    CanaryTrafficStage(10_000, 1800),
    CanaryTrafficStage(50_000, 3600),
    CanaryTrafficStage(250_000, 3600),
    CanaryTrafficStage(1_000_000, 0),
)


@dataclass(frozen=True, slots=True)
class CanaryReport:
    status: CanaryStatus
    started_at: datetime
    observed_at: datetime
    latency_p99_delta_ms: float = 0.0
    error_rate_delta: float = 0.0
    drift_score_delta: float = 0.0
    quality_metric_delta: float = 0.0
    warnings: tuple[str, ...] = field(default_factory=tuple)

    def __post_init__(self) -> None:
        if self.started_at.tzinfo is None or self.observed_at.tzinfo is None:
            raise ValueError("timestamps must be tz-aware")
