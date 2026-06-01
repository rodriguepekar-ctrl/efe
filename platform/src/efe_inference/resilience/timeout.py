"""TimeoutBudget — hierarchical deadlines (connect / first-byte / total)."""

from __future__ import annotations

from dataclasses import dataclass

from efe_inference.config import TimeoutSettings


@dataclass(frozen=True, slots=True)
class TimeoutBudget:
    connect_s: float
    first_byte_s: float
    inter_token_s: float
    total_s: float

    @classmethod
    def from_settings(cls, s: TimeoutSettings) -> TimeoutBudget:
        return cls(
            connect_s=s.connect_s,
            first_byte_s=s.first_byte_s,
            inter_token_s=s.inter_token_s,
            total_s=s.total_s,
        )

    def with_total(self, total_s: float) -> TimeoutBudget:
        return TimeoutBudget(
            connect_s=min(self.connect_s, total_s),
            first_byte_s=min(self.first_byte_s, total_s),
            inter_token_s=min(self.inter_token_s, total_s),
            total_s=total_s,
        )


__all__ = ["TimeoutBudget"]
