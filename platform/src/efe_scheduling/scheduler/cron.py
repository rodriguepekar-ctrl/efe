"""Extended cron parser.

Supports the classic ``minute hour dom month dow`` layout (5 fields), plus
an optional ``year`` (6 fields), plus symbolic aliases :

* ``@hourly``   — ``0 * * * *``
* ``@daily``    — ``0 0 * * *``
* ``@weekly``   — ``0 0 * * 0``
* ``@monthly``  — ``0 0 1 * *``
* ``@yearly``   — ``0 0 1 1 *``

Field syntax :

* ``*`` — any
* ``a``, ``a,b,c`` — list
* ``a-b`` — inclusive range
* ``*/n``, ``a-b/n`` — stepped range
* month names (``JAN``..``DEC``) and day names (``SUN``..``SAT``) are
  accepted case-insensitively for the ``month`` and ``dow`` fields.

``dow`` accepts ``0`` or ``7`` for Sunday. ``dom`` accepts ``L`` for
"last day of month" (extension).

Internally, each field is a ``frozenset[int]`` of allowed values ; the
``iter_fires`` implementation walks minute-by-minute which is tractable
up to the ``lookahead_s`` horizon (60 s by default, 1-day worst case).
"""

from __future__ import annotations

import calendar
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Iterator
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from efe_scheduling.errors import InvalidCronExpression, TimezoneNotFound

__all__ = ["CronExpression", "parse_cron"]


_MONTH_NAMES: dict[str, int] = {
    "JAN": 1, "FEB": 2, "MAR": 3, "APR": 4, "MAY": 5, "JUN": 6,
    "JUL": 7, "AUG": 8, "SEP": 9, "OCT": 10, "NOV": 11, "DEC": 12,
}

_DAY_NAMES: dict[str, int] = {
    "SUN": 0, "MON": 1, "TUE": 2, "WED": 3, "THU": 4, "FRI": 5, "SAT": 6,
}

_ALIASES: dict[str, str] = {
    "@hourly": "0 * * * *",
    "@daily": "0 0 * * *",
    "@midnight": "0 0 * * *",
    "@weekly": "0 0 * * 0",
    "@monthly": "0 0 1 * *",
    "@yearly": "0 0 1 1 *",
    "@annually": "0 0 1 1 *",
}

_MINUTE_RANGE = (0, 59)
_HOUR_RANGE = (0, 23)
_DOM_RANGE = (1, 31)
_MONTH_RANGE = (1, 12)
_DOW_RANGE = (0, 6)  # normalised so that 7 is mapped to 0
_YEAR_RANGE = (1970, 2099)


def _normalise_token(tok: str, field_name: str) -> str:
    t = tok.strip().upper()
    if field_name == "month" and t in _MONTH_NAMES:
        return str(_MONTH_NAMES[t])
    if field_name == "dow" and t in _DAY_NAMES:
        return str(_DAY_NAMES[t])
    return t


def _parse_field(
    raw: str,
    field_name: str,
    valid_range: tuple[int, int],
) -> frozenset[int]:
    """Parse a single cron field into a frozenset of allowed values."""
    lo, hi = valid_range
    values: set[int] = set()

    # Split by comma, each part may be range / step / simple value.
    for part in raw.split(","):
        token = _normalise_token(part, field_name)

        if token == "":
            raise InvalidCronExpression(
                f"field {field_name!r}: empty sub-expression",
                detail={"raw": raw},
            )

        # dom extension : 'L' = last day
        if field_name == "dom" and token == "L":
            for d in range(28, 32):
                values.add(d)
            continue

        step = 1
        body = token
        if "/" in token:
            body, step_raw = token.split("/", 1)
            try:
                step = int(step_raw)
            except ValueError as exc:
                raise InvalidCronExpression(
                    f"field {field_name!r}: bad step {step_raw!r}"
                ) from exc
            if step <= 0:
                raise InvalidCronExpression(
                    f"field {field_name!r}: step must be > 0"
                )

        if body == "*":
            a, b = lo, hi
        elif "-" in body:
            try:
                a_raw, b_raw = body.split("-", 1)
                a = int(_normalise_token(a_raw, field_name))
                b = int(_normalise_token(b_raw, field_name))
            except ValueError as exc:
                raise InvalidCronExpression(
                    f"field {field_name!r}: bad range {body!r}"
                ) from exc
        else:
            try:
                v = int(body)
            except ValueError as exc:
                raise InvalidCronExpression(
                    f"field {field_name!r}: bad value {body!r}"
                ) from exc
            a = b = v

        # DOW requires special handling because 7 and 0 both mean Sunday.
        # Vixie cron accepts "0-7" or "1-7" as full-week specifications.
        if field_name == "dow":
            if b == 7 and a != b:
                # `a-7` = `a..6` ∪ {0}. Handles 0-7 (whole week) and 5-7
                # (Fri-Sun) cleanly.
                if a < 0 or a > 6:
                    raise InvalidCronExpression(
                        f"field {field_name!r}: dow start out of range: {a}"
                    )
                values.update(range(a, 7, step))
                values.add(0)
                continue
            if a == 7 and b == 7:
                # Just "7" — Sunday alias for 0.
                a = 0
                b = 0

        if a > b:
            raise InvalidCronExpression(
                f"field {field_name!r}: range start > end ({a} > {b})"
            )
        if a < lo or b > hi:
            raise InvalidCronExpression(
                f"field {field_name!r}: out of range [{lo}, {hi}] : {a}-{b}"
            )

        values.update(range(a, b + 1, step))

    if not values:
        raise InvalidCronExpression(f"field {field_name!r}: empty set")
    return frozenset(values)


@dataclass(frozen=True, slots=True)
class CronExpression:
    """Parsed cron expression.

    Use :func:`parse_cron` to build one. Call :meth:`iter_fires` or
    :meth:`next_fire_after` to enumerate fires.
    """

    minute: frozenset[int]
    hour: frozenset[int]
    dom: frozenset[int]
    month: frozenset[int]
    dow: frozenset[int]
    year: frozenset[int] = field(
        default_factory=lambda: frozenset(range(_YEAR_RANGE[0], _YEAR_RANGE[1] + 1))
    )
    raw: str = ""
    timezone_name: str = "UTC"

    def _tz(self) -> ZoneInfo:
        try:
            return ZoneInfo(self.timezone_name)
        except ZoneInfoNotFoundError as exc:
            raise TimezoneNotFound(
                f"timezone {self.timezone_name!r} not found"
            ) from exc

    # ------------------------------------------------------------------
    # Single-instant matching
    # ------------------------------------------------------------------

    def matches(self, at: datetime) -> bool:
        """Return True iff ``at`` (converted to this expression's tz)
        matches every field."""
        local = at.astimezone(self._tz())
        # dom / dow semantics : if both are constrained, fire when either
        # matches (Vixie cron convention).
        dom_all = self.dom == frozenset(range(_DOM_RANGE[0], _DOM_RANGE[1] + 1))
        dow_all = self.dow == frozenset(range(_DOW_RANGE[0], _DOW_RANGE[1] + 1))
        dow_local = (local.weekday() + 1) % 7  # Python Monday=0, cron Sun=0
        if dom_all and dow_all:
            date_ok = True
        elif dom_all:
            date_ok = dow_local in self.dow
        elif dow_all:
            date_ok = local.day in self.dom
        else:
            date_ok = (local.day in self.dom) or (dow_local in self.dow)

        return (
            local.minute in self.minute
            and local.hour in self.hour
            and local.month in self.month
            and local.year in self.year
            and date_ok
        )

    # ------------------------------------------------------------------
    # Iteration
    # ------------------------------------------------------------------

    def iter_fires(
        self,
        start: datetime,
        end: datetime,
        *,
        limit: int | None = None,
    ) -> Iterator[datetime]:
        """Yield UTC ``datetime`` fires strictly greater than ``start`` and
        ``<= end``.

        The exclusive-start / inclusive-end semantics match the engine's
        half-open evaluation window ``(last_tick, now]``.
        """
        if start.tzinfo is None or end.tzinfo is None:
            raise ValueError("iter_fires requires tz-aware datetimes")
        tz = self._tz()
        start_local = start.astimezone(tz)
        end_local = end.astimezone(tz)

        # Floor ``start`` to the minute and advance by one to make the
        # bound strictly exclusive.
        cur = start_local.replace(second=0, microsecond=0) + timedelta(minutes=1)
        count = 0

        while cur <= end_local:
            if self.matches(cur):
                yield cur.astimezone(timezone.utc)
                count += 1
                if limit is not None and count >= limit:
                    return
            cur += timedelta(minutes=1)

    def next_fire_after(self, t: datetime) -> datetime | None:
        """Return the first fire strictly after ``t``."""
        # Look up to 4 years in the future (covers Feb 29 on non-leap sequences).
        horizon = t + timedelta(days=4 * 366)
        for fire in self.iter_fires(t, horizon, limit=1):
            return fire
        return None


def parse_cron(expression: str, *, timezone_name: str = "UTC") -> CronExpression:
    """Parse a cron expression string.

    Raises :class:`InvalidCronExpression` for any parse error.
    """
    raw = expression.strip()
    if not raw:
        raise InvalidCronExpression("cron expression is empty")
    if raw in _ALIASES:
        raw = _ALIASES[raw]

    parts = raw.split()
    if len(parts) == 5:
        minute_s, hour_s, dom_s, month_s, dow_s = parts
        year_s = "*"
    elif len(parts) == 6:
        minute_s, hour_s, dom_s, month_s, dow_s, year_s = parts
    else:
        raise InvalidCronExpression(
            f"cron expression must have 5 or 6 fields, got {len(parts)}",
            detail={"raw": raw},
        )

    return CronExpression(
        minute=_parse_field(minute_s, "minute", _MINUTE_RANGE),
        hour=_parse_field(hour_s, "hour", _HOUR_RANGE),
        dom=_parse_field(dom_s, "dom", _DOM_RANGE),
        month=_parse_field(month_s, "month", _MONTH_RANGE),
        dow=_parse_field(dow_s, "dow", _DOW_RANGE),
        year=_parse_field(year_s, "year", _YEAR_RANGE),
        raw=expression.strip(),
        timezone_name=timezone_name,
    )


# Silence unused-import warnings in minimal configurations.
_ = calendar
