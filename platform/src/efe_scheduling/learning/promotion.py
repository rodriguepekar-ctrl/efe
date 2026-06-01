"""Promotion decision — promote / hold / rollback."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime, timedelta

from efe_scheduling.learning.canary import CanaryReport, CanaryStatus
from efe_scheduling.learning.validation import ValidationReport

__all__ = [
    "PromotionDecision",
    "PromotionInputs",
    "PromotionOutcome",
    "decide_promotion",
]


class PromotionOutcome(str, enum.Enum):
    promote = "promote"
    hold = "hold"
    rollback = "rollback"


@dataclass(frozen=True, slots=True)
class PromotionInputs:
    canary: CanaryReport
    validation: ValidationReport
    gate_major_failures: int = 0
    canary_started_at: datetime | None = None
    canary_observation_h: int = 2
    max_latency_p99_delta_ms: float = 30.0
    max_error_rate_delta: float = 0.01
    max_drift_score_delta: float = 0.10
    min_quality_metric_delta: float = -0.02
    min_coverage: float = 0.6
    now: datetime | None = None


@dataclass(frozen=True, slots=True)
class PromotionDecision:
    outcome: PromotionOutcome
    reason: str = ""
    rollback_to: str | None = None
    extras: dict[str, float] = field(default_factory=dict)


def decide_promotion(inputs: PromotionInputs) -> PromotionDecision:
    canary = inputs.canary
    validation = inputs.validation

    # 1) Rollback conditions dominate.
    if canary.status == CanaryStatus.unhealthy:
        return PromotionDecision(
            outcome=PromotionOutcome.rollback,
            reason="canary unhealthy",
        )
    if canary.latency_p99_delta_ms > inputs.max_latency_p99_delta_ms:
        return PromotionDecision(
            outcome=PromotionOutcome.rollback,
            reason=(
                f"latency_p99 delta {canary.latency_p99_delta_ms:.1f}ms > "
                f"{inputs.max_latency_p99_delta_ms:.1f}ms"
            ),
        )
    if canary.error_rate_delta > inputs.max_error_rate_delta:
        return PromotionDecision(
            outcome=PromotionOutcome.rollback,
            reason=(
                f"error_rate delta {canary.error_rate_delta:.4f} > "
                f"{inputs.max_error_rate_delta:.4f}"
            ),
        )
    if canary.drift_score_delta > inputs.max_drift_score_delta:
        return PromotionDecision(
            outcome=PromotionOutcome.rollback,
            reason=(
                f"drift delta {canary.drift_score_delta:.3f} > "
                f"{inputs.max_drift_score_delta:.3f}"
            ),
        )

    # 2) Hold conditions.
    if inputs.gate_major_failures > 0:
        return PromotionDecision(
            outcome=PromotionOutcome.hold,
            reason=f"{inputs.gate_major_failures} major gate failure(s)",
        )
    if not validation.ready:
        return PromotionDecision(
            outcome=PromotionOutcome.hold,
            reason="validation not ready",
        )
    if canary.quality_metric_delta < inputs.min_quality_metric_delta:
        return PromotionDecision(
            outcome=PromotionOutcome.hold,
            reason=(
                f"quality delta {canary.quality_metric_delta:.3f} < "
                f"{inputs.min_quality_metric_delta:.3f}"
            ),
        )
    # 3) Observation duration.
    if inputs.canary_started_at is not None and inputs.now is not None:
        elapsed = inputs.now - inputs.canary_started_at
        if elapsed < timedelta(hours=inputs.canary_observation_h):
            return PromotionDecision(
                outcome=PromotionOutcome.hold,
                reason=(
                    f"observation {elapsed.total_seconds() / 3600:.2f}h < "
                    f"{inputs.canary_observation_h}h"
                ),
            )

    return PromotionDecision(outcome=PromotionOutcome.promote, reason="all checks pass")
