"""Multi-scale (1h / 1d / 7d) window computation.

Invariant **I6** — each scale is evaluated independently. A short
burst of drift at 1h scale does not promote a 1d alert until its own
aggregate drift crosses the 1d threshold.

The :class:`MultiScaleWindow` is a pair ``(start, end)`` derived from
the evaluation ``now`` and the scale label (``"1h"``, ``"1d"``,
``"7d"``) — bounds are UTC-aligned on the scale boundary :

* ``1h`` : ``[now - 1h, now)`` aligned on the minute (so partial
  minutes do not leak between buckets)
* ``1d`` : ``[now - 24h, now)``
* ``7d`` : ``[now - 7d, now)``
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from efe_drift.detection.base import MultiScaleWindow


@dataclass(frozen=True, slots=True)
class ScaleWindow:
    label: MultiScaleWindow
    start: datetime
    end: datetime


def compute_scale_windows(
    now: datetime,
    *,
    enabled: tuple[MultiScaleWindow, ...] = ("1h", "1d", "7d"),
) -> tuple[ScaleWindow, ...]:
    """Return one :class:`ScaleWindow` per enabled scale label.

    The window ``end`` is truncated to minute precision so
    consecutive evaluator ticks within the same minute see the same
    bounds — keeps drift scores stable across restart / replay.
    """
    if now.tzinfo is None:
        raise ValueError("now must be tz-aware")
    now_utc = now.astimezone(timezone.utc)
    end = now_utc.replace(second=0, microsecond=0)
    out: list[ScaleWindow] = []
    for label in enabled:
        if label == "1h":
            start = end - timedelta(hours=1)
        elif label == "1d":
            start = end - timedelta(days=1)
        elif label == "7d":
            start = end - timedelta(days=7)
        else:
            raise ValueError(f"unknown scale label {label!r}")
        out.append(ScaleWindow(label=label, start=start, end=end))
    return tuple(out)


__all__ = ["ScaleWindow", "compute_scale_windows"]
