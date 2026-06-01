"""Rollback decision — live runtime safety net."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field

__all__ = [
    "RollbackDecision",
    "RollbackInputs",
    "RollbackOutcome",
    "decide_rollback",
]


class RollbackOutcome(str, enum.Enum):
    rollback = "rollback"
    hold = "hold"


@dataclass(frozen=True, slots=True)
class RollbackInputs:
    tenant_id: str
    task_id: str | None
    error_rate_current: float
    error_rate_baseline: float
    latency_p99_current_ms: float
    latency_p99_baseline_ms: float
    drift_score_current: float
    drift_score_baseline: float
    saturation_ratio: float = 0.0  # 0..1

    @property
    def error_rate_delta(self) -> float:
        return self.error_rate_current - self.error_rate_baseline

    @property
    def latency_delta(self) -> float:
        return self.latency_p99_current_ms - self.latency_p99_baseline_ms

    @property
    def drift_delta(self) -> float:
        return self.drift_score_current - self.drift_score_baseline


@dataclass(frozen=True, slots=True)
class RollbackDecision:
    outcome: RollbackOutcome
    reason: str = ""
    cool_down_h: int = 24
    extras: dict[str, float] = field(default_factory=dict)


def decide_rollback(
    inputs: RollbackInputs,
    *,
    max_error_rate_delta: float = 0.02,
    max_latency_delta_ms: float = 100.0,
    max_drift_delta: float = 0.2,
    max_saturation: float = 0.85,
) -> RollbackDecision:
    if inputs.error_rate_delta > max_error_rate_delta:
        return RollbackDecision(
            outcome=RollbackOutcome.rollback,
            reason=f"error_rate delta {inputs.error_rate_delta:.4f} > {max_error_rate_delta:.4f}",
        )
    if inputs.latency_delta > max_latency_delta_ms:
        return RollbackDecision(
            outcome=RollbackOutcome.rollback,
            reason=f"latency delta {inputs.latency_delta:.1f}ms > {max_latency_delta_ms:.1f}ms",
        )
    if inputs.drift_delta > max_drift_delta:
        return RollbackDecision(
            outcome=RollbackOutcome.rollback,
            reason=f"drift delta {inputs.drift_delta:.3f} > {max_drift_delta:.3f}",
        )
    if inputs.saturation_ratio > max_saturation:
        return RollbackDecision(
            outcome=RollbackOutcome.rollback,
            reason=f"saturation {inputs.saturation_ratio:.2f} > {max_saturation:.2f}",
        )
    return RollbackDecision(outcome=RollbackOutcome.hold, reason="healthy")
