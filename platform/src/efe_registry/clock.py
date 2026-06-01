"""Time abstraction — parallels :mod:`efe_gate.clock`.

In addition to wall-clock ``now()`` we also expose a monotonic reading
which the :class:`RollbackExecutor` uses for phase instrumentation.
The monotonic source must be strictly increasing and unaffected by
NTP adjustments.
"""

from __future__ import annotations

import time
from datetime import datetime, timedelta, timezone
from typing import Protocol


class Clock(Protocol):
    def now(self) -> datetime:  # pragma: no cover
        ...

    def now_monotonic_s(self) -> float:  # pragma: no cover
        ...


class SystemClock:
    __slots__ = ()

    def now(self) -> datetime:
        return datetime.now(tz=timezone.utc)

    def now_monotonic_s(self) -> float:
        return time.monotonic()


class FakeClock:
    """Steppable clock; deterministic tests.

    Wall and monotonic clocks advance together to preserve the intuitive
    mapping in test assertions.
    """

    __slots__ = ("_m", "_t")

    def __init__(self, epoch: datetime | None = None) -> None:
        base = epoch or datetime(2026, 1, 1, tzinfo=timezone.utc)
        if base.tzinfo is None:
            raise ValueError("FakeClock epoch must be timezone-aware")
        self._t = base.astimezone(timezone.utc)
        self._m = 0.0

    def now(self) -> datetime:
        return self._t

    def now_monotonic_s(self) -> float:
        return self._m

    def advance(self, seconds: float) -> datetime:
        if seconds < 0:
            raise ValueError("FakeClock cannot move backwards")
        self._t = self._t + timedelta(seconds=seconds)
        self._m += float(seconds)
        return self._t

    def set(self, when: datetime) -> None:
        if when.tzinfo is None:
            raise ValueError("FakeClock.set() requires a tz-aware datetime")
        delta = (when.astimezone(timezone.utc) - self._t).total_seconds()
        if delta < 0:
            raise ValueError("FakeClock cannot move backwards via set()")
        self.advance(delta)


__all__ = ["Clock", "FakeClock", "SystemClock"]
