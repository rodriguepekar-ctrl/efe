"""Internal clock adapter — thin facade used by quota.py.

Prefer :mod:`efe_scheduling.clock` when available ; otherwise fall
back to ``time.monotonic()``.
"""

from __future__ import annotations

import time as _time

__all__ = ["_now_mono"]


def _now_mono() -> float:
    return _time.monotonic()
