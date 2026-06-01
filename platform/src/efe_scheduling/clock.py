"""Time abstraction — injectable :class:`Clock` Protocol.

Two implementations are provided :

* :class:`SystemClock` — production, reads ``datetime.now(tz=UTC)`` and
  ``time.monotonic()``.
* :class:`FakeClock` — deterministic test double. Unlike ``asyncio.sleep``,
  the clock's readings advance *only* when the test explicitly calls
  :meth:`FakeClock.advance` or :meth:`FakeClock.set`.

Both variants expose two readings :

* ``now()`` — wall-clock, tz-aware UTC datetime for durable timestamps.
* ``now_monotonic_s()`` — monotonic seconds for latency / budget
  instrumentation. Guaranteed strictly non-decreasing and unaffected by
  NTP steps.

Invariant **P4** (clocks injected) is enforced by forbidding any direct
call to ``datetime.now()`` outside of :mod:`efe_scheduling.clock`. A lint
rule in CI scans the codebase for violations.
"""

from __future__ import annotations

import time
from datetime import datetime, timedelta, timezone
from typing import Protocol, runtime_checkable

__all__ = ["Clock", "FakeClock", "SystemClock"]


@runtime_checkable
class Clock(Protocol):
    """Protocol for injectable clocks.

    Implementations must guarantee :

    * ``now()`` returns a tz-aware UTC datetime.
    * ``now_monotonic_s()`` returns a non-decreasing float — each call
      returns a value ``>= previous_call``.
    """

    def now(self) -> datetime:  # pragma: no cover
        """Return wall-clock time, tz-aware UTC."""
        ...

    def now_monotonic_s(self) -> float:  # pragma: no cover
        """Return monotonic seconds (non-decreasing, unaffected by NTP)."""
        ...


class SystemClock:
    """Production clock reading from the OS."""

    __slots__ = ()

    def now(self) -> datetime:
        return datetime.now(tz=timezone.utc)

    def now_monotonic_s(self) -> float:
        return time.monotonic()


class FakeClock:
    """Steppable deterministic clock for unit tests.

    Wall-clock and monotonic readings move in lockstep so tests reading
    either source see a consistent world.

    Usage::

        clk = FakeClock(epoch=datetime(2026, 4, 20, tzinfo=timezone.utc))
        clk.advance(60)  # +60 seconds
        assert clk.now_monotonic_s() == 60.0
    """

    __slots__ = ("_m", "_t")

    def __init__(self, epoch: datetime | None = None) -> None:
        base = epoch if epoch is not None else datetime(2026, 4, 21, tzinfo=timezone.utc)
        if base.tzinfo is None:
            raise ValueError("FakeClock epoch must be timezone-aware")
        self._t: datetime = base.astimezone(timezone.utc)
        self._m: float = 0.0

    def now(self) -> datetime:
        return self._t

    def now_monotonic_s(self) -> float:
        return self._m

    def advance(self, seconds: float) -> datetime:
        """Advance both readings by ``seconds``. Returns new ``now()``."""
        if seconds < 0:
            raise ValueError("FakeClock cannot move backwards")
        self._t = self._t + timedelta(seconds=seconds)
        self._m += float(seconds)
        return self._t

    def set(self, when: datetime) -> None:
        """Set wall-clock to ``when`` (must be >= current)."""
        if when.tzinfo is None:
            raise ValueError("FakeClock.set() requires a tz-aware datetime")
        target = when.astimezone(timezone.utc)
        delta = (target - self._t).total_seconds()
        if delta < 0:
            raise ValueError("FakeClock cannot move backwards via set()")
        self.advance(delta)
