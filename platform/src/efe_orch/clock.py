"""Time abstraction for :mod:`efe_orch`.

Everything that needs ``now()`` depends on a :class:`Clock` instance
rather than on :func:`datetime.datetime.now` directly. This matters
because:

* The heartbeat reaper, backoff policy, and state machine timeouts are
  *all* time-driven; unit tests need a fake, steppable clock.
* We enforce UTC by construction — timezone-naive datetimes are
  rejected at the boundary.
* A single test fixture can freeze or fast-forward the whole service.

Two concrete implementations are shipped:

    :class:`SystemClock`    — delegates to :func:`datetime.now(tz=UTC)`.
    :class:`FakeClock`      — manual ``advance(seconds)`` in tests.

Both implement the narrow :class:`Clock` protocol.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Protocol


class Clock(Protocol):
    """Any object that can return the current UTC time."""

    def now(self) -> datetime:  # pragma: no cover — protocol
        ...


class SystemClock:
    """Real wall-clock time in UTC."""

    __slots__ = ()

    def now(self) -> datetime:
        return datetime.now(tz=timezone.utc)


class FakeClock:
    """Deterministic clock for tests.

    Starts at a configurable ``epoch`` (default ``2026-01-01T00:00:00Z``)
    and moves forward only when :meth:`advance` or :meth:`set` is
    called. Thread-safety is not required because asyncio tests run on
    a single event loop.
    """

    __slots__ = ("_t",)

    def __init__(
        self,
        epoch: datetime | None = None,
    ) -> None:
        base = epoch or datetime(2026, 1, 1, tzinfo=timezone.utc)
        if base.tzinfo is None:
            raise ValueError("FakeClock epoch must be timezone-aware (UTC)")
        self._t = base.astimezone(timezone.utc)

    def now(self) -> datetime:
        return self._t

    def advance(self, seconds: float) -> datetime:
        """Move forward by ``seconds`` and return the new time."""
        if seconds < 0:
            raise ValueError("FakeClock cannot move backwards")
        self._t = self._t + timedelta(seconds=seconds)
        return self._t

    def set(self, when: datetime) -> None:
        if when.tzinfo is None:
            raise ValueError("FakeClock.set() requires a timezone-aware datetime")
        self._t = when.astimezone(timezone.utc)


def ensure_utc(ts: datetime) -> datetime:
    """Return ``ts`` converted to UTC or raise on naive input."""
    if ts.tzinfo is None:
        raise ValueError("timezone-naive datetime not accepted in efe_orch")
    return ts.astimezone(timezone.utc)


__all__ = ["Clock", "FakeClock", "SystemClock", "ensure_utc"]
