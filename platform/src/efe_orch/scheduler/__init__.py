"""Resource scheduler — pool selection, quota enforcement.

Public surface:

    PoolSelector       — decide which pool a run should land in
    PoolCapacity       — live count of runs active in each pool
    QuotaGuard         — pre-admission quota check
    SchedulerService   — high-level façade wiring the three together
"""

from __future__ import annotations

from efe_orch.scheduler.policies import (
    PoolCapacity,
    PoolSelector,
    RunShape,
    SchedulingDecision,
)
from efe_orch.scheduler.pools import PoolRegistry
from efe_orch.scheduler.quotas import QuotaGuard, QuotaReservation
from efe_orch.scheduler.service import SchedulerService

__all__ = [
    "PoolCapacity",
    "PoolRegistry",
    "PoolSelector",
    "QuotaGuard",
    "QuotaReservation",
    "RunShape",
    "SchedulerService",
    "SchedulingDecision",
]
