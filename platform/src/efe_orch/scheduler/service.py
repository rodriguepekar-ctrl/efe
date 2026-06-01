"""High-level scheduler façade.

Stitches together:

* :class:`PoolRegistry` — static pool catalogue,
* :class:`PoolSelector` — pure decision function,
* :class:`QuotaGuard`   — per-tenant quota gate,
* live pool capacity query from the DB.

Exposes a single ``plan_placement(run_shape, tenant_id)`` call that
returns a :class:`SchedulingDecision` (or raises).
"""

from __future__ import annotations

from sqlalchemy import func, select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.config import QuotaSettings, SchedulerSettings
from efe_orch.db.models import FinetuneRun
from efe_orch.scheduler.policies import (
    PoolCapacity,
    PoolSelector,
    RunShape,
    SchedulingDecision,
)
from efe_orch.scheduler.pools import PoolRegistry
from efe_orch.scheduler.quotas import QuotaGuard, QuotaReservation
from efe_orch.state_machine.states import ACTIVE_STATES


class SchedulerService:
    """Everything the run-service needs to place a run."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        scheduler_settings: SchedulerSettings,
        quota_settings: QuotaSettings,
    ) -> None:
        self._session = session
        self._registry = PoolRegistry(scheduler_settings.pools)
        self._selector = PoolSelector(self._registry)
        self._quota = QuotaGuard(session, quota_settings)

    @property
    def registry(self) -> PoolRegistry:
        return self._registry

    @property
    def quota(self) -> QuotaGuard:
        return self._quota

    # ------------------------------------------------------------------ API

    async def plan_placement(
        self,
        *,
        tenant_id: str,
        shape: RunShape,
    ) -> SchedulingDecision:
        """Consult pool registry + live capacity, return a decision.

        The tenant whitelist is merged into ``shape.allowed_pools`` so
        the selector honours it without the caller having to pass it
        explicitly.
        """
        whitelist = tuple(await self._quota.allowed_pools(tenant_id))
        effective_shape = RunShape(
            dataset_rows=shape.dataset_rows,
            priority=shape.priority,
            needs_gpu=shape.needs_gpu,
            requested_pool=shape.requested_pool,
            allowed_pools=whitelist or shape.allowed_pools,
        )
        capacity = await self.live_capacity()
        return self._selector.pick(effective_shape, capacity)

    async def live_capacity(self) -> PoolCapacity:
        """Count *actually placed* runs grouped by pool.

        Only ACTIVE states (SCHEDULED/TRAINING/EVALUATING/UPLOADING) are
        counted. An earlier revision included QUEUED runs in the load,
        which double-booked capacity: a QUEUED run has no worker yet,
        so it does not consume a pool slot. Counting it produced
        false-positive ``PoolSaturated`` errors on submission when the
        pool was in fact idle.
        """
        active_states = [s.value for s in ACTIVE_STATES]
        stmt = (
            select(FinetuneRun.pool, func.count(FinetuneRun.id))
            .where(FinetuneRun.state.in_(active_states))
            .where(FinetuneRun.pool.is_not(None))
            .group_by(FinetuneRun.pool)
        )
        res = await self._session.execute(stmt)
        loads: dict[str, int] = {}
        for pool, count in res.all():
            if pool is not None:
                loads[str(pool)] = int(count)
        return PoolCapacity(current=loads)

    # ---------------------------- quota delegates (thin wrappers) ----

    async def reserve(
        self, *, tenant_id: str, estimated_gpu_seconds: int = 0,
    ) -> QuotaReservation:
        return await self._quota.reserve_run_slot(
            tenant_id=tenant_id,
            estimated_gpu_seconds=estimated_gpu_seconds,
        )

    async def release_concurrency(self, tenant_id: str) -> None:
        await self._quota.release_concurrency(tenant_id)


__all__ = ["SchedulerService"]
