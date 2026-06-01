"""Phase instrumentation helpers.

Every rollback phase records its elapsed seconds into a
:class:`PhaseRecorder`. The recorder serialises into a JSONB blob
stored in ``rollback_events.phases_jsonb`` — post-mortems read it.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum

from efe_registry.clock import Clock, SystemClock


class PhaseName(str, Enum):
    LOOKUP = "lookup_s"
    STANDBY = "standby_s"
    SWAP = "swap_s"
    DRAIN = "drain_s"
    VERIFY = "verify_s"
    AUDIT = "audit_s"
    TOTAL = "total_s"


class PhaseTimeout(Exception):
    """Raised when a phase exceeds its hard deadline."""

    def __init__(self, phase: PhaseName, budget_s: float) -> None:
        super().__init__(
            f"phase {phase.value!r} exceeded budget of {budget_s:.2f}s",
        )
        self.phase = phase
        self.budget_s = budget_s


@dataclass
class PhaseRecorder:
    clock: Clock = field(default_factory=SystemClock)
    _started_at: float | None = None
    _phases: dict[str, float] = field(default_factory=dict)
    _last_tick: float | None = None

    def start(self) -> None:
        self._started_at = self.clock.now_monotonic_s()
        self._last_tick = self._started_at

    def mark(self, phase: PhaseName) -> float:
        """Record elapsed seconds since the previous :meth:`mark`."""
        if self._last_tick is None:
            self.start()
        now = self.clock.now_monotonic_s()
        assert self._last_tick is not None
        elapsed = max(0.0, now - self._last_tick)
        self._phases[phase.value] = elapsed
        self._last_tick = now
        return elapsed

    def total(self) -> float:
        if self._started_at is None:
            return 0.0
        t = self.clock.now_monotonic_s() - self._started_at
        self._phases[PhaseName.TOTAL.value] = t
        return t

    def to_dict(self) -> dict[str, float]:
        self.total()
        return dict(self._phases)


__all__ = ["PhaseName", "PhaseRecorder", "PhaseTimeout"]
