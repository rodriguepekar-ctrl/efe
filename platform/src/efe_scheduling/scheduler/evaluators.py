"""Concrete schedule evaluators.

Each evaluator wraps one :class:`~efe_scheduling.types.ScheduleSpec`
variant and exposes :

* :meth:`iter_fires` — yield fire times in a half-open window
  ``(start, end]``, all in UTC.
* :meth:`next_fire_after` — convenience for the closest fire strictly
  after a given instant.

The :func:`build_evaluator` factory dispatches on ``spec.kind`` so the
rest of the engine can stay oblivious of the sum-type structure.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Iterator, Protocol, runtime_checkable

from efe_scheduling.errors import InvalidRRule, ValidationError
from efe_scheduling.scheduler.cron import CronExpression, parse_cron
from efe_scheduling.types import (
    CompositeSchedule,
    CronSchedule,
    IntervalSchedule,
    OneShotSchedule,
    RRuleSchedule,
    ScheduleKind,
    ScheduleSpec,
)

__all__ = [
    "CompositeEvaluator",
    "CronEvaluator",
    "IntervalEvaluator",
    "OneShotEvaluator",
    "RRuleEvaluator",
    "ScheduleEvaluator",
    "build_evaluator",
]


@runtime_checkable
class ScheduleEvaluator(Protocol):
    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]: ...

    def next_fire_after(self, t: datetime) -> datetime | None: ...


# ---------------------------------------------------------------------------
# Cron
# ---------------------------------------------------------------------------


class CronEvaluator:
    __slots__ = ("_expr",)

    def __init__(self, spec: CronSchedule) -> None:
        self._expr: CronExpression = parse_cron(
            spec.expression, timezone_name=spec.timezone_name
        )

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        yield from self._expr.iter_fires(start, end, limit=limit)

    def next_fire_after(self, t: datetime) -> datetime | None:
        return self._expr.next_fire_after(t)


# ---------------------------------------------------------------------------
# Interval
# ---------------------------------------------------------------------------


class IntervalEvaluator:
    """Periodic schedule.

    Fires at every ``t`` satisfying ``(t - epoch - phase) % period == 0``,
    where ``epoch`` is Unix 0.
    """

    __slots__ = ("_period", "_phase")

    def __init__(self, spec: IntervalSchedule) -> None:
        self._period: int = spec.period_s
        self._phase: int = spec.phase_s

    def _first_at_or_after(self, t: datetime) -> datetime:
        """Smallest fire instant ``>= t`` (sub-second precision preserved)."""
        import math

        if t.tzinfo is None:
            raise ValueError("IntervalEvaluator requires tz-aware datetimes")
        t_seconds: float = t.astimezone(timezone.utc).timestamp()
        k = math.ceil((t_seconds - self._phase) / self._period)
        aligned = self._phase + k * self._period
        return datetime.fromtimestamp(aligned, tz=timezone.utc)

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        if start.tzinfo is None or end.tzinfo is None:
            raise ValueError("iter_fires requires tz-aware datetimes")
        # Strictly exclusive start → shift by 1 microsecond.
        first = self._first_at_or_after(start + timedelta(microseconds=1))
        count = 0
        cur = first
        while cur <= end:
            yield cur
            count += 1
            if limit is not None and count >= limit:
                return
            cur += timedelta(seconds=self._period)

    def next_fire_after(self, t: datetime) -> datetime | None:
        return self._first_at_or_after(t + timedelta(microseconds=1))


# ---------------------------------------------------------------------------
# One-shot
# ---------------------------------------------------------------------------


class OneShotEvaluator:
    __slots__ = ("_at",)

    def __init__(self, spec: OneShotSchedule) -> None:
        self._at: datetime = spec.at.astimezone(timezone.utc)

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        del limit  # always at most one fire
        if start < self._at <= end:
            yield self._at

    def next_fire_after(self, t: datetime) -> datetime | None:
        return self._at if self._at > t else None


# ---------------------------------------------------------------------------
# RRULE (lightweight pure-python implementation covering our needs)
# ---------------------------------------------------------------------------


class RRuleEvaluator:
    """Minimal RRULE adapter — no external dependency.

    Supports these common patterns :

    * ``FREQ=MINUTELY;INTERVAL=N``
    * ``FREQ=HOURLY;INTERVAL=N``
    * ``FREQ=DAILY;INTERVAL=N``
    * ``FREQ=WEEKLY;INTERVAL=N``

    With optional ``COUNT=N`` or ``UNTIL=YYYYMMDDTHHMMSSZ``.

    Anything else raises :class:`InvalidRRule`.
    """

    __slots__ = ("_freq", "_interval", "_count", "_until", "_dtstart")

    _FREQ_TO_TD: dict[str, timedelta] = {
        "MINUTELY": timedelta(minutes=1),
        "HOURLY": timedelta(hours=1),
        "DAILY": timedelta(days=1),
        "WEEKLY": timedelta(weeks=1),
    }

    def __init__(self, spec: RRuleSchedule) -> None:
        parts: dict[str, str] = {}
        for piece in spec.rrule_text.split(";"):
            if "=" not in piece:
                raise InvalidRRule(f"bad RRULE token {piece!r}")
            k, v = piece.split("=", 1)
            parts[k.strip().upper()] = v.strip()
        freq = parts.get("FREQ", "")
        if freq not in self._FREQ_TO_TD:
            raise InvalidRRule(
                f"unsupported FREQ={freq!r}",
                detail={"supported": sorted(self._FREQ_TO_TD)},
            )
        self._freq: str = freq
        self._interval: int = int(parts.get("INTERVAL", "1"))
        if self._interval < 1:
            raise InvalidRRule("INTERVAL must be >= 1")

        count = parts.get("COUNT")
        self._count: int | None = int(count) if count is not None else None
        if self._count is not None and self._count < 1:
            raise InvalidRRule("COUNT must be >= 1")

        until_raw = parts.get("UNTIL")
        self._until: datetime | None = None
        if until_raw:
            # Support both YYYYMMDDTHHMMSSZ and ISO 8601.
            try:
                if until_raw.endswith("Z") and "T" in until_raw:
                    compact = until_raw
                    self._until = datetime.strptime(
                        compact, "%Y%m%dT%H%M%SZ"
                    ).replace(tzinfo=timezone.utc)
                else:
                    self._until = datetime.fromisoformat(
                        until_raw.replace("Z", "+00:00")
                    )
            except ValueError as exc:
                raise InvalidRRule(f"bad UNTIL={until_raw!r}") from exc

        self._dtstart: datetime = (
            spec.dtstart.astimezone(timezone.utc)
            if spec.dtstart is not None
            else datetime(2026, 1, 1, tzinfo=timezone.utc)
        )

    def _step(self) -> timedelta:
        return self._FREQ_TO_TD[self._freq] * self._interval

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        if start.tzinfo is None or end.tzinfo is None:
            raise ValueError("iter_fires requires tz-aware datetimes")
        step = self._step()
        # Find the first dtstart + k*step strictly greater than start.
        if start < self._dtstart:
            t = self._dtstart
            k = 0
        else:
            diff = start - self._dtstart
            # smallest k such that dtstart + k*step > start
            k = int(diff.total_seconds() // step.total_seconds()) + 1
            t = self._dtstart + step * k

        count_yielded = 0
        while t <= end:
            if self._until is not None and t > self._until:
                return
            if self._count is not None and k >= self._count:
                return
            yield t
            count_yielded += 1
            if limit is not None and count_yielded >= limit:
                return
            k += 1
            t = self._dtstart + step * k

    def next_fire_after(self, t: datetime) -> datetime | None:
        for fire in self.iter_fires(t, t + timedelta(days=366 * 10), limit=1):
            return fire
        return None


# ---------------------------------------------------------------------------
# Composite (union / exclude)
# ---------------------------------------------------------------------------


class CompositeEvaluator:
    __slots__ = ("_left", "_op", "_right")

    def __init__(self, spec: CompositeSchedule) -> None:
        assert spec.left is not None and spec.right is not None  # validated
        self._left: ScheduleEvaluator = build_evaluator(spec.left)
        self._right: ScheduleEvaluator = build_evaluator(spec.right)
        self._op: str = spec.op

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        left_fires = sorted(self._left.iter_fires(start, end))
        right_fires = set(self._right.iter_fires(start, end))
        if self._op == "union":
            merged = sorted({*left_fires, *right_fires})
        else:  # exclude
            merged = [t for t in left_fires if t not in right_fires]
        count = 0
        for t in merged:
            yield t
            count += 1
            if limit is not None and count >= limit:
                return

    def next_fire_after(self, t: datetime) -> datetime | None:
        horizon = t + timedelta(days=366 * 4)
        for fire in self.iter_fires(t, horizon, limit=1):
            return fire
        return None


# ---------------------------------------------------------------------------
# Factory
# ---------------------------------------------------------------------------


def build_evaluator(spec: ScheduleSpec) -> ScheduleEvaluator:
    """Dispatch on ``spec.kind`` to the appropriate evaluator."""
    match spec.kind:
        case ScheduleKind.cron:
            assert isinstance(spec, CronSchedule)
            return CronEvaluator(spec)
        case ScheduleKind.interval:
            assert isinstance(spec, IntervalSchedule)
            return IntervalEvaluator(spec)
        case ScheduleKind.rrule:
            assert isinstance(spec, RRuleSchedule)
            return RRuleEvaluator(spec)
        case ScheduleKind.one_shot:
            assert isinstance(spec, OneShotSchedule)
            return OneShotEvaluator(spec)
        case ScheduleKind.composite:
            assert isinstance(spec, CompositeSchedule)
            return CompositeEvaluator(spec)
        case _:
            raise ValidationError(f"unsupported ScheduleKind: {spec.kind}")
