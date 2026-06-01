"""DeadlineInterceptor — clamp to MAX_CLIENT_DEADLINE_S."""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.config import MAX_CLIENT_DEADLINE_S

__all__ = ["DeadlineInterceptor"]


@dataclass(slots=True)
class DeadlineInterceptor:
    max_deadline_s: int = MAX_CLIENT_DEADLINE_S

    def clamp(self, deadline_s: float | None) -> float:
        if deadline_s is None or deadline_s <= 0:
            return float(self.max_deadline_s)
        return min(float(deadline_s), float(self.max_deadline_s))
