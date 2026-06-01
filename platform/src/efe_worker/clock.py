"""Time abstraction — mirror of :mod:`efe_orch.clock`.

The worker's heartbeat cadence, deadline checks, and backoff timers
all depend on a :class:`Clock` protocol rather than
:func:`datetime.datetime.now` directly so tests can fast-forward time.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Protocol


class Clock(Protocol):
    def now(self) -> datetime:  # pragma: no cover
        ...

    def monotonic(self) -> float:  # pragma: no cover
        ...


class SystemClock:
    """Wall-clock + real monotonic."""

    __slots__ = ()

    def now(self) -> datetime:
        return datetime.now(tz=timezone.utc)

    def monotonic(self) -> float:
        import time
        return time.monotonic()


class FakeClock:
    """Steppable clock. Useful for deterministic tests of deadlines + HB."""

    __slots__ = ("_mono", "_now")

    def __init__(
        self,
        epoch: datetime | None = None,
        monotonic: float = 0.0,
    ) -> None:
        base = epoch or datetime(2026, 1, 1, tzinfo=timezone.utc)
        if base.tzinfo is None:
            raise ValueError("FakeClock requires a timezone-aware epoch")
        self._now = base.astimezone(timezone.utc)
        self._mono = float(monotonic)

    def now(self) -> datetime:
        return self._now

    def monotonic(self) -> float:
        return self._mono

    def advance(self, seconds: float) -> datetime:
        if seconds < 0:
            raise ValueError("FakeClock cannot move backwards")
        self._now = self._now + timedelta(seconds=seconds)
        self._mono += float(seconds)
        return self._now


__all__ = ["Clock", "FakeClock", "SystemClock"]
