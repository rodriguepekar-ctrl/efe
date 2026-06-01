"""Idempotency key for a scheduled fire.

``fire_key_for(schedule_id, fire_time)`` is a 32-byte HMAC that uniquely
identifies one fire of one schedule. The ``jobs`` table carries a
``UNIQUE`` constraint on this column, which backs invariant **I1**.

Separating the helper into a dedicated module keeps the dependency graph
acyclic : :mod:`efe_scheduling.scheduler.engine` depends on this module
but not on :mod:`efe_scheduling.types`.
"""

from __future__ import annotations

from datetime import datetime

from efe_scheduling.types import build_fire_key

__all__ = ["fire_key_for"]


def fire_key_for(schedule_id: str, fire_time: datetime) -> bytes:
    return build_fire_key(schedule_id, fire_time)
