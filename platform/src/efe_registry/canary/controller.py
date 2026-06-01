"""CanaryController — pilots the ramp-up of a candidate artifact.

Each :meth:`tick` inspects one active canary and decides:

* abort if the SLO has breached N times in a row (``state=rolled_back``);
* advance to the next step if enough time has elapsed and SLO is green;
* stay put otherwise.

Leader election is outside the scope of this class; the calling
daemon is expected to hold a Redis lock per ``(tenant, task)``.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Protocol

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.canary.schedule import (
    find_step_index,
    schedule_for,
)
from efe_registry.canary.slo_gateway import SloBreach, SloGateway
from efe_registry.clock import Clock, SystemClock
from efe_registry.config import CanarySettings
from efe_registry.core.artifact import ArtifactRole
from efe_registry.db.models import CanaryRun, Endpoint
from efe_registry.errors import CanaryNotFound


class RollbackExecutorLike(Protocol):
    async def execute(
        self,
        *,
        tenant_id: str,
        task_id: str,
        actor: str,
        reason: str,
        trigger_kind: str,
        idempotency_key: bytes | None = None,
    ) -> object:  # pragma: no cover — concrete impl returns RollbackReceipt
        ...


class RoutingUpdaterLike(Protocol):
    async def set_canary_percent(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_endpoint_id: str,
        baseline_endpoint_id: str,
        percent: int,
        actor: str,
    ) -> int:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class CanaryTickResult:
    canary_id: str
    action: str      # "stay" | "advance" | "complete" | "paused" | "rolled_back"
    from_percent: int
    to_percent: int
    reason: str | None = None


class CanaryController:
    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        settings: CanarySettings,
        slo: SloGateway,
        rollback: RollbackExecutorLike,
        routing: RoutingUpdaterLike,
        clock: Clock | None = None,
    ) -> None:
        self._sf = session_factory
        self._cfg = settings
        self._slo = slo
        self._rollback = rollback
        self._routing = routing
        self._clock: Clock = clock or SystemClock()

    async def tick(self, canary_id: str) -> CanaryTickResult:
        async with self._sf() as session:
            # H7 — row-level lock on the canary for the full tick so two
            # concurrent daemon replicas cannot double-increment the
            # breach counter or double-advance the step.
            lock_stmt = (
                select(CanaryRun)
                .where(CanaryRun.canary_id == canary_id)
                .with_for_update()
            )
            await session.execute(lock_stmt)
            run = await session.get(CanaryRun, canary_id)
            if run is None:
                raise CanaryNotFound(
                    f"canary {canary_id!r} not found",
                )
            if run.state not in ("ramping", "paused"):
                return CanaryTickResult(
                    canary_id=canary_id,
                    action="stay",
                    from_percent=run.current_percent,
                    to_percent=run.current_percent,
                    reason=f"state={run.state}",
                )
            schedule = schedule_for(
                run.schedule_kind,
                step_duration_s=run.step_duration_s,
            )

            # 1) SLO check
            breach = await self._slo.evaluate(
                tenant_id=run.tenant_id,
                task_id=run.task_id,
                candidate_artifact=run.candidate_artifact,
                baseline_artifact=run.baseline_artifact,
                window_s=60,
            )
            if breach.breached:
                run.slo_breach_count = int(run.slo_breach_count) + 1
                if (
                    run.slo_breach_count
                    >= self._cfg.slo_breach_abort_threshold
                ):
                    return await self._abort(session, run, breach)
                await session.commit()
                return CanaryTickResult(
                    canary_id=canary_id,
                    action="stay",
                    from_percent=run.current_percent,
                    to_percent=run.current_percent,
                    reason=f"slo_breach_count={run.slo_breach_count}",
                )

            # 2) Time check
            now = self._clock.now()
            last = _aware(run.last_step_at or run.started_at)
            elapsed_s = (now - last).total_seconds()
            current_idx = find_step_index(schedule, run.current_percent)
            step = schedule[current_idx]
            if elapsed_s < step.min_duration_s:
                return CanaryTickResult(
                    canary_id=canary_id,
                    action="stay",
                    from_percent=run.current_percent,
                    to_percent=run.current_percent,
                    reason="dwell",
                )

            # 3) Advance
            if current_idx + 1 >= len(schedule):
                return await self._complete(session, run)
            next_step = schedule[current_idx + 1]
            from_percent = run.current_percent
            await self._apply_percent(
                session=session, run=run, percent=next_step.percent,
            )
            run.current_percent = next_step.percent
            run.last_step_at = now
            run.slo_breach_count = 0
            await session.commit()
            return CanaryTickResult(
                canary_id=canary_id,
                action="advance",
                from_percent=from_percent,
                to_percent=next_step.percent,
                reason=f"dwell_satisfied:{elapsed_s:.1f}s",
            )

    # ------------------------------------------------------------ helpers

    async def _apply_percent(
        self,
        *,
        session: AsyncSession,
        run: CanaryRun,
        percent: int,
    ) -> None:
        """Call routing updater with the endpoint IDs tied to this canary."""
        candidate_ep = await _find_endpoint(
            session,
            artifact_id=run.candidate_artifact,
            role=ArtifactRole.CANDIDATE,
        )
        baseline_ep = await _find_endpoint(
            session,
            artifact_id=run.baseline_artifact,
            role=ArtifactRole.CHAMPION,
        )
        if candidate_ep is None or baseline_ep is None:
            raise CanaryNotFound(
                f"missing endpoints for canary {run.canary_id!r}",
            )
        await self._routing.set_canary_percent(
            tenant_id=run.tenant_id,
            task_id=run.task_id,
            candidate_endpoint_id=candidate_ep.endpoint_id,
            baseline_endpoint_id=baseline_ep.endpoint_id,
            percent=int(percent),
            actor="canary-controller",
        )

    async def _complete(
        self, session: AsyncSession, run: CanaryRun,
    ) -> CanaryTickResult:
        run.state = "completed"
        run.ended_at = self._clock.now()
        await session.commit()
        return CanaryTickResult(
            canary_id=run.canary_id,
            action="complete",
            from_percent=run.current_percent,
            to_percent=run.current_percent,
        )

    async def _abort(
        self,
        session: AsyncSession,
        run: CanaryRun,
        breach: SloBreach,
    ) -> CanaryTickResult:
        reason = (
            f"slo_breach latency_ratio={breach.latency_p95_ratio:.2f} "
            f"err_ratio={breach.error_rate_ratio:.2f} "
            f"Δq={breach.quality_delta:+.3f}"
        )
        if run.auto_rollback:
            run.state = "rolled_back"
            run.ended_at = self._clock.now()
            await session.commit()
            await self._rollback.execute(
                tenant_id=run.tenant_id,
                task_id=run.task_id,
                actor="canary-controller",
                reason=reason,
                trigger_kind="auto_slo",
            )
            return CanaryTickResult(
                canary_id=run.canary_id,
                action="rolled_back",
                from_percent=run.current_percent,
                to_percent=0,
                reason=reason,
            )
        run.state = "paused"
        await session.commit()
        return CanaryTickResult(
            canary_id=run.canary_id,
            action="paused",
            from_percent=run.current_percent,
            to_percent=run.current_percent,
            reason=reason,
        )


def _aware(dt: datetime) -> datetime:
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


async def _find_endpoint(
    session: AsyncSession,
    *,
    artifact_id: str,
    role: ArtifactRole,
) -> Endpoint | None:
    stmt = (
        select(Endpoint)
        .where(
            Endpoint.artifact_id == artifact_id,
            Endpoint.role == role.value,
            Endpoint.state == "ready",
        )
        .limit(1)
    )
    return (await session.execute(stmt)).scalar_one_or_none()


__all__ = ["CanaryController", "CanaryTickResult"]
