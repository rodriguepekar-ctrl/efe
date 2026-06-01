"""Retrain decision (pure function)."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime, timedelta

__all__ = [
    "RetrainDecision",
    "RetrainInputs",
    "RetrainOutcome",
    "decide_retrain",
]


class RetrainOutcome(str, enum.Enum):
    retrain = "retrain"
    hold = "hold"
    freeze = "freeze"


@dataclass(frozen=True, slots=True)
class RetrainInputs:
    now: datetime
    last_retrain_at: datetime | None
    cool_down_until: datetime | None
    max_drift_severity: str = "none"  # none|warning|major|critical
    critical_feature_count: int = 0
    overdue_threshold_d: int = 14
    accuracy_delta_last_7d: float = 0.0
    accuracy_delta_alarm: float = -0.02
    dataset_coverage_ok: bool = True
    min_critical_features: int = 3


@dataclass(frozen=True, slots=True)
class RetrainDecision:
    outcome: RetrainOutcome
    reason: str = ""
    hint: dict[str, str] = field(default_factory=dict)


def decide_retrain(inputs: RetrainInputs) -> RetrainDecision:
    if inputs.now.tzinfo is None:
        raise ValueError("now must be tz-aware")
    # 1) Freeze cool-down.
    if inputs.cool_down_until is not None and inputs.now < inputs.cool_down_until:
        return RetrainDecision(
            outcome=RetrainOutcome.freeze,
            reason="cool-down after rollback",
            hint={"cool_down_until": inputs.cool_down_until.isoformat()},
        )
    # 2) Severity + feature count.
    if (
        inputs.max_drift_severity == "critical"
        and inputs.critical_feature_count >= inputs.min_critical_features
    ):
        return RetrainDecision(
            outcome=RetrainOutcome.retrain,
            reason=(
                f"critical drift on {inputs.critical_feature_count} features "
                f">= {inputs.min_critical_features}"
            ),
        )
    # 3) Overdue.
    if (
        inputs.last_retrain_at is None
        or (inputs.now - inputs.last_retrain_at)
        > timedelta(days=inputs.overdue_threshold_d)
    ):
        return RetrainDecision(
            outcome=RetrainOutcome.retrain,
            reason=f"overdue by > {inputs.overdue_threshold_d}d",
        )
    # 4) Accuracy regression.
    if (
        inputs.accuracy_delta_last_7d < inputs.accuracy_delta_alarm
        and inputs.dataset_coverage_ok
    ):
        return RetrainDecision(
            outcome=RetrainOutcome.retrain,
            reason=(
                f"accuracy regression {inputs.accuracy_delta_last_7d:.3f} < "
                f"{inputs.accuracy_delta_alarm:.3f}"
            ),
        )
    return RetrainDecision(outcome=RetrainOutcome.hold, reason="no trigger")
