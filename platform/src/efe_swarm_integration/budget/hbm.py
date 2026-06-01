"""HBM budget guard — raises before the wire call when budget exceeds."""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.budget.estimator import HbmEstimator
from efe_swarm_integration.config import MAX_HBM_BYTES
from efe_swarm_integration.errors import HbmBudgetExceeded
from efe_swarm_integration.types import SwarmSpec

__all__ = ["HbmBudgeter"]


@dataclass(slots=True)
class HbmBudgeter:
    estimator: HbmEstimator
    hard_cap: int = MAX_HBM_BYTES

    def verify(self, spec: SwarmSpec, *, anchors_count: int = 0) -> int:
        """Return the estimated HBM or raise :class:`HbmBudgetExceeded`."""
        est = self.estimator.estimate(spec, anchors_count=anchors_count)
        if est > self.hard_cap:
            raise HbmBudgetExceeded(
                f"hbm estimate {est} exceeds hard cap {self.hard_cap}",
                detail={"estimate": est, "cap": self.hard_cap},
            )
        return est
