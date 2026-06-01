"""Time abstraction — wall + monotonic clocks.

Pattern identical to :mod:`efe_registry.clock`: production uses a
:class:`SystemClock`; tests inject :class:`FakeClock`. Async waits
that should remain time-bounded in tests use :func:`time.monotonic`
directly to avoid looping forever when the fake clock does not
advance (see M06 audit §C4 fix).
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
    """Deterministic clock. Wall + monotonic advance together."""

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


__all__ = ["Clock", "FakeClock", "SystemClock"]
