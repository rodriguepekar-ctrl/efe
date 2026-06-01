"""In-process cost tracker. The authoritative counter lives in the DB
(`agent_run.current_cost_cents`) ; this is a fast local check so we
can SIGTERM ourselves *before* asking the LLM router for another call.
"""
from __future__ import annotations


class CostTracker:
    def __init__(self, cap_cents: int) -> None:
        self.cap_cents = cap_cents
        self.spent_cents = 0

    def add(self, cents: int) -> None:
        if cents < 0:
            raise ValueError("cost delta must be >= 0")
        self.spent_cents += cents

    def exceeded(self) -> bool:
        return self.spent_cents >= self.cap_cents

    def remaining(self) -> int:
        return max(0, self.cap_cents - self.spent_cents)

    def pct_consumed(self) -> float:
        if self.cap_cents <= 0:
            return 100.0
        return (self.spent_cents / self.cap_cents) * 100.0
