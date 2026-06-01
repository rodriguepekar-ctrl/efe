"""Baseline refresh decision (pure function).

Given the last-refresh timestamp, whether the feature is still active
and the sanity check, decide whether to launch a new baseline snapshot.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime, timedelta

__all__ = [
    "BaselineRefreshDecision",
    "BaselineRefreshInputs",
    "BaselineRefreshOutcome",
    "decide_baseline_refresh",
]


class BaselineRefreshOutcome(str, enum.Enum):
    refresh = "refresh"
    skip_inactive = "skip_inactive"
    skip_recently_refreshed = "skip_recently_refreshed"
    skip_sanity_failed = "skip_sanity_failed"


@dataclass(frozen=True, slots=True)
class BaselineRefreshInputs:
    feature_id: str
    feature_active: bool
    last_refresh_at: datetime | None
    now: datetime
    sample_count_last_window: int
    min_samples: int
    variance_ratio: float
    max_variance_ratio: float

    def __post_init__(self) -> None:
        if self.now.tzinfo is None:
            raise ValueError("now must be tz-aware")
        if self.last_refresh_at is not None and self.last_refresh_at.tzinfo is None:
            raise ValueError("last_refresh_at must be tz-aware")
        if self.min_samples < 0:
            raise ValueError("min_samples >= 0")


@dataclass(frozen=True, slots=True)
class BaselineRefreshDecision:
    outcome: BaselineRefreshOutcome
    reason: str = ""
    next_check_in_s: int | None = None
    extra: dict[str, float] = field(default_factory=dict)


def decide_baseline_refresh(
    inputs: BaselineRefreshInputs,
    *,
    min_interval_h: int = 12,
) -> BaselineRefreshDecision:
    """Deterministic refresh decision."""
    if not inputs.feature_active:
        return BaselineRefreshDecision(
            outcome=BaselineRefreshOutcome.skip_inactive,
            reason="feature inactive",
        )
    if (
        inputs.last_refresh_at is not None
        and (inputs.now - inputs.last_refresh_at) < timedelta(hours=min_interval_h)
    ):
        return BaselineRefreshDecision(
            outcome=BaselineRefreshOutcome.skip_recently_refreshed,
            reason=f"refreshed < {min_interval_h}h ago",
            next_check_in_s=min_interval_h * 3600,
        )
    if inputs.sample_count_last_window < inputs.min_samples:
        return BaselineRefreshDecision(
            outcome=BaselineRefreshOutcome.skip_sanity_failed,
            reason=(
                f"not enough samples ({inputs.sample_count_last_window} < "
                f"{inputs.min_samples})"
            ),
            extra={"sample_count": float(inputs.sample_count_last_window)},
        )
    if inputs.variance_ratio > inputs.max_variance_ratio:
        return BaselineRefreshDecision(
            outcome=BaselineRefreshOutcome.skip_sanity_failed,
            reason=(
                f"variance_ratio {inputs.variance_ratio:.3f} > "
                f"{inputs.max_variance_ratio:.3f}"
            ),
            extra={"variance_ratio": inputs.variance_ratio},
        )
    return BaselineRefreshDecision(
        outcome=BaselineRefreshOutcome.refresh,
        reason="eligible",
    )
