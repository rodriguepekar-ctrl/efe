"""Time abstraction — same pattern as the rest of the platform."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Protocol


class Clock(Protocol):
    def now(self) -> datetime:  # pragma: no cover
        ...


class SystemClock:
    __slots__ = ()

    def now(self) -> datetime:
        return datetime.now(tz=timezone.utc)


class FakeClock:
    """Steppable clock; deterministic tests."""

    __slots__ = ("_t",)

    def __init__(self, epoch: datetime | None = None) -> None:
        base = epoch or datetime(2026, 1, 1, tzinfo=timezone.utc)
        if base.tzinfo is None:
            raise ValueError("FakeClock epoch must be timezone-aware")
        self._t = base.astimezone(timezone.utc)

    def now(self) -> datetime:
        return self._t

    def advance(self, seconds: float) -> datetime:
        if seconds < 0:
            raise ValueError("FakeClock cannot move backwards")
        self._t = self._t + timedelta(seconds=seconds)
        return self._t

    def set(self, when: datetime) -> None:
        if when.tzinfo is None:
            raise ValueError("FakeClock.set() requires a tz-aware datetime")
        self._t = when.astimezone(timezone.utc)


__all__ = ["Clock", "FakeClock", "SystemClock"]
