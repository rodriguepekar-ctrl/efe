"""Time bucketing utilities.

Every :class:`~efe_drift.records.FeatureSnapshot` is anchored on a
UTC bucket defined by ``bucket_kind`` + ``bucket_start``. The
bucketing functions here produce stable, tz-aware timestamps that
collapse observations into the right bucket.

Invariant I7 — bucketing is always done on ``observed_at`` (not
ingested time), so late-arriving events are rolled up into their
original hour / day.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

from efe_drift.errors import ValidationError
from efe_drift.records import BucketKind


def _ensure_utc(ts: datetime) -> datetime:
    if ts.tzinfo is None:
        raise ValidationError(
            f"timestamp {ts!r} is naive ; expected tz-aware UTC",
        )
    return ts.astimezone(timezone.utc)


def bucket_bounds(
    observed_at: datetime, kind: BucketKind,
) -> tuple[datetime, datetime]:
    """Return ``(bucket_start, bucket_end)`` for the given kind.

    * ``minute`` : truncate to 1-minute boundary.
    * ``hour``   : truncate to 1-hour boundary.
    * ``day``    : truncate to UTC midnight.
    * ``week``   : truncate to Monday 00:00 UTC (ISO week).
    """
    ts = _ensure_utc(observed_at)
    if kind == "minute":
        start = ts.replace(second=0, microsecond=0)
        return start, start + timedelta(minutes=1)
    if kind == "hour":
        start = ts.replace(minute=0, second=0, microsecond=0)
        return start, start + timedelta(hours=1)
    if kind == "day":
        start = ts.replace(hour=0, minute=0, second=0, microsecond=0)
        return start, start + timedelta(days=1)
    if kind == "week":
        day_start = ts.replace(hour=0, minute=0, second=0, microsecond=0)
        monday = day_start - timedelta(days=day_start.weekday())
        return monday, monday + timedelta(days=7)
    raise ValidationError(f"unknown bucket_kind {kind!r}")


def bucket_start(observed_at: datetime, kind: BucketKind) -> datetime:
    return bucket_bounds(observed_at, kind)[0]


def iter_buckets(
    *, start: datetime, end: datetime, kind: BucketKind,
) -> tuple[tuple[datetime, datetime], ...]:
    """Enumerate (inclusive-start, exclusive-end) buckets of ``kind``
    covering ``[start, end)``. Both endpoints must be aligned on a
    bucket boundary — otherwise we raise."""
    start_bounds = bucket_bounds(start, kind)
    if start_bounds[0] != _ensure_utc(start):
        raise ValidationError(
            f"start {start!r} is not aligned on a {kind} boundary",
        )
    buckets: list[tuple[datetime, datetime]] = []
    cur, cur_end = start_bounds
    end_utc = _ensure_utc(end)
    while cur_end <= end_utc:
        buckets.append((cur, cur_end))
        cur = cur_end
        cur_end = bucket_bounds(cur, kind)[1]
    return tuple(buckets)


__all__ = [
    "bucket_bounds",
    "bucket_start",
    "iter_buckets",
]
