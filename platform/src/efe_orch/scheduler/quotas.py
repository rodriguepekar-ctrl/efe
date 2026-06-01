"""Tenant quota enforcement.

The orchestrator has three hard ceilings per tenant, all read (and
mutated) on the ``tenant_quotas`` row:

1. **Concurrency** — ``current_concurrent_runs < max_concurrent_runs``.
   Incremented on QUEUED entry, decremented on terminal-state entry.
2. **Monthly run count** — ``current_monthly_runs < max_monthly_runs``.
   Reset at month boundaries by a scheduled job.
3. **Monthly GPU seconds** — ``current_monthly_gpu_seconds <
   max_monthly_gpu_seconds``. Added at SUCCEEDED entry.

The guard is consulted on ``DRAFT → QUEUED``. A reservation is
returned so the caller can roll it back on downstream failure.
"""

from __future__ import annotations

from dataclasses import dataclass

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.config import QuotaSettings
from efe_orch.db.models import TenantQuota
from efe_orch.errors import QuotaExceeded


@dataclass(frozen=True, slots=True)
class QuotaReservation:
    """Receipt returned by :meth:`QuotaGuard.reserve_run_slot`."""

    tenant_id: str
    reserved_concurrent: bool
    reserved_monthly: bool


class QuotaGuard:
    """Enforce per-tenant quotas at submission time.

    The guard auto-creates a ``tenant_quotas`` row on first sight of a
    tenant, seeded with the platform defaults.
    """

    def __init__(
        self, session: AsyncSession, defaults: QuotaSettings,
    ) -> None:
        self._session = session
        self._defaults = defaults

    # ------------------------------------------------------------------ API

    async def reserve_run_slot(
        self, *, tenant_id: str, estimated_gpu_seconds: int = 0,
    ) -> QuotaReservation:
        quota = await self._get_or_create(tenant_id)

        if quota.current_concurrent_runs >= quota.max_concurrent_runs:
            raise QuotaExceeded(
                f"tenant {tenant_id!r} has reached the concurrency cap "
                f"({quota.max_concurrent_runs} concurrent runs)",
                detail={"kind": "concurrent_runs",
                        "current": quota.current_concurrent_runs,
                        "limit": quota.max_concurrent_runs},
            )
        if quota.current_monthly_runs >= quota.max_monthly_runs:
            raise QuotaExceeded(
                f"tenant {tenant_id!r} has reached the monthly run cap "
                f"({quota.max_monthly_runs})",
                detail={"kind": "monthly_runs",
                        "current": quota.current_monthly_runs,
                        "limit": quota.max_monthly_runs},
            )
        if (
            estimated_gpu_seconds > 0
            and (
                quota.current_monthly_gpu_seconds + estimated_gpu_seconds
                > quota.max_monthly_gpu_seconds
            )
        ):
            raise QuotaExceeded(
                f"tenant {tenant_id!r} would exceed monthly GPU budget",
                detail={
                    "kind": "monthly_gpu_seconds",
                    "current": quota.current_monthly_gpu_seconds,
                    "estimate": estimated_gpu_seconds,
                    "limit": quota.max_monthly_gpu_seconds,
                },
            )

        quota.current_concurrent_runs += 1
        quota.current_monthly_runs += 1
        await self._session.flush()
        return QuotaReservation(
            tenant_id=tenant_id,
            reserved_concurrent=True,
            reserved_monthly=True,
        )

    async def release_concurrency(self, tenant_id: str) -> None:
        """Called on terminal-state entry."""
        quota = await self._get_or_create(tenant_id)
        if quota.current_concurrent_runs > 0:
            quota.current_concurrent_runs -= 1
        await self._session.flush()

    async def add_gpu_seconds(
        self, tenant_id: str, seconds: int,
    ) -> None:
        if seconds <= 0:
            return
        quota = await self._get_or_create(tenant_id)
        quota.current_monthly_gpu_seconds += int(seconds)
        await self._session.flush()

    async def cancel_reservation(self, reservation: QuotaReservation) -> None:
        """Roll back a reservation if the caller couldn't complete the queueing."""
        quota = await self._get_or_create(reservation.tenant_id)
        if reservation.reserved_concurrent and quota.current_concurrent_runs > 0:
            quota.current_concurrent_runs -= 1
        if reservation.reserved_monthly and quota.current_monthly_runs > 0:
            quota.current_monthly_runs -= 1
        await self._session.flush()

    async def allowed_pools(self, tenant_id: str) -> tuple[str, ...]:
        quota = await self._get_or_create(tenant_id)
        return tuple(quota.allowed_pools or [])

    # ---------------------------------------------------------------- helpers

    async def _get_or_create(self, tenant_id: str) -> TenantQuota:
        stmt = select(TenantQuota).where(TenantQuota.tenant_id == tenant_id)
        res = await self._session.execute(stmt)
        quota = res.scalar_one_or_none()
        if quota is None:
            quota = TenantQuota(
                tenant_id=tenant_id,
                max_concurrent_runs=self._defaults.default_max_concurrent_runs,
                max_monthly_runs=self._defaults.default_max_monthly_runs,
                max_monthly_gpu_seconds=(
                    self._defaults.default_max_monthly_gpu_seconds
                ),
                allowed_pools=[],
                current_concurrent_runs=0,
                current_monthly_runs=0,
                current_monthly_gpu_seconds=0,
            )
            self._session.add(quota)
            await self._session.flush()
        return quota


__all__ = ["QuotaGuard", "QuotaReservation"]
