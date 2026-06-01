"""ArtifactPromoter — consumes M05 decisions, drives canary creation.

Invoked either directly (in tests) or by the NATS subscriber listening
to ``efe.gate.decisions.promote``. It:

1. Checks idempotence via :class:`ArtifactPromotion` ledger.
2. Verifies signature (cached).
3. Advances the artifact FSM to SERVING.
4. Starts a canary at 1 % via the router updater.
5. Records a state transition audit event.

The promoter does **not** touch vLLM pod lifecycle — that is done by
the K8s operator reacting to the ``serving`` transition event.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Protocol

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.core.state_machine import ArtifactTransitionValidator
from efe_registry.db.models import (
    Artifact as ArtifactRow,
)
from efe_registry.db.models import (
    ArtifactPromotion,
    CanaryRun,
    Endpoint,
)
from efe_registry.errors import (
    ArtifactNotFound,
    ArtifactVerificationFailed,
    CanaryAlreadyActive,
    NoStandbyAvailable,
)


class SignatureVerifierLike(Protocol):
    async def verify_artifact(
        self, artifact_id: str,
    ) -> bool:  # pragma: no cover
        ...


class RoutingUpdaterLike(Protocol):
    async def start_canary(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_endpoint_id: str,
        baseline_endpoint_id: str,
        percent: int,
        actor: str,
    ) -> None:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class PromoteResult:
    decision_id: str
    artifact_id: str
    accepted: bool
    reason: str | None
    canary_id: str | None


class ArtifactPromoter:
    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        verifier: SignatureVerifierLike,
        router_updater: RoutingUpdaterLike,
        clock: Clock | None = None,
    ) -> None:
        self._sf = session_factory
        self._verifier = verifier
        self._router = router_updater
        self._clock: Clock = clock or SystemClock()

    async def promote(
        self,
        *,
        decision_id: str,
        artifact_id: str,
        actor: str,
        reason: str,
    ) -> PromoteResult:
        async with self._sf() as session:
            # --- idempotence ---
            existing = await session.get(ArtifactPromotion, decision_id)
            if existing is not None:
                return PromoteResult(
                    decision_id=decision_id,
                    artifact_id=existing.artifact_id,
                    accepted=existing.accepted,
                    reason=existing.rejection_reason,
                    canary_id=None,
                )

            artifact = await session.get(ArtifactRow, artifact_id)
            if artifact is None:
                raise ArtifactNotFound(
                    f"artifact {artifact_id!r} not found during promote",
                )

            # --- active canary exclusion (C6 app-level check) ---
            from sqlalchemy import select as _select
            active = await session.execute(
                _select(CanaryRun)
                .where(
                    CanaryRun.tenant_id == artifact.tenant_id,
                    CanaryRun.task_id == artifact.task_id,
                    CanaryRun.state.in_(("ramping", "paused")),
                )
                .limit(1),
            )
            if active.scalar_one_or_none() is not None:
                raise CanaryAlreadyActive(
                    f"a canary is already active for "
                    f"({artifact.tenant_id!r}, {artifact.task_id!r}); "
                    f"wait for completion or abort before promoting",
                )

            # --- verify signature (cached) ---
            ok = await self._verifier.verify_artifact(artifact_id)
            if not ok:
                session.add(ArtifactPromotion(
                    decision_id=decision_id,
                    artifact_id=artifact_id,
                    tenant_id=artifact.tenant_id,
                    task_id=artifact.task_id,
                    accepted=False,
                    rejection_reason="signature_invalid",
                ))
                await session.commit()
                raise ArtifactVerificationFailed(
                    f"signature check failed for {artifact_id!r}",
                )

            # --- pick endpoints (tenant-scoped, H2) ---
            candidate_endpoint = await _find_ready_endpoint(
                session, artifact_id=artifact_id,
                role=ArtifactRole.CANDIDATE,
                tenant_id=artifact.tenant_id,
                task_id=artifact.task_id,
            )
            if candidate_endpoint is None:
                raise NoStandbyAvailable(
                    f"no ready candidate endpoint for artifact "
                    f"{artifact_id!r}; promotion deferred",
                )
            baseline_endpoint = await _find_current_champion(
                session,
                tenant_id=artifact.tenant_id,
                task_id=artifact.task_id,
            )
            canary_id = str(uuid.uuid4())
            if baseline_endpoint is None:
                # First promotion: candidate becomes 100% champion. We
                # still journal a CanaryRun (state=completed, percent=100)
                # so H1/I5 auditability is preserved.
                session.add(CanaryRun(
                    canary_id=canary_id,
                    tenant_id=artifact.tenant_id,
                    task_id=artifact.task_id,
                    candidate_artifact=artifact_id,
                    baseline_artifact=artifact_id,
                    current_percent=100,
                    target_percent=100,
                    schedule_kind="manual",
                    state="completed",
                    started_at=self._clock.now(),
                    last_step_at=self._clock.now(),
                    ended_at=self._clock.now(),
                    auto_rollback=False,
                    auto_advance=False,
                ))
                await self._router.start_canary(
                    tenant_id=artifact.tenant_id,
                    task_id=artifact.task_id,
                    candidate_endpoint_id=candidate_endpoint.endpoint_id,
                    baseline_endpoint_id=candidate_endpoint.endpoint_id,
                    percent=100,
                    actor=actor,
                )
            else:
                # --- start canary at 1% ---
                session.add(CanaryRun(
                    canary_id=canary_id,
                    tenant_id=artifact.tenant_id,
                    task_id=artifact.task_id,
                    candidate_artifact=artifact_id,
                    baseline_artifact=baseline_endpoint.artifact_id or "",
                    current_percent=1,
                    target_percent=100,
                    schedule_kind="geometric",
                    state="ramping",
                    started_at=self._clock.now(),
                    last_step_at=self._clock.now(),
                    auto_rollback=True,
                    auto_advance=True,
                ))
                await self._router.start_canary(
                    tenant_id=artifact.tenant_id,
                    task_id=artifact.task_id,
                    candidate_endpoint_id=candidate_endpoint.endpoint_id,
                    baseline_endpoint_id=baseline_endpoint.endpoint_id,
                    percent=1,
                    actor=actor,
                )

            # --- FSM ADMITTED -> SERVING ---
            validator = ArtifactTransitionValidator(session)
            await validator.apply(
                artifact_id=artifact_id,
                expected_state=ArtifactState(artifact.state),
                new_state=ArtifactState.SERVING,
                expected_version=artifact.state_version,
                actor=actor,
                reason=reason,
            )
            # update served_at
            row = await session.get(ArtifactRow, artifact_id)
            assert row is not None
            row.served_at = self._clock.now()
            row.promoted_at = self._clock.now()

            # --- ledger ---
            session.add(ArtifactPromotion(
                decision_id=decision_id,
                artifact_id=artifact_id,
                tenant_id=artifact.tenant_id,
                task_id=artifact.task_id,
                accepted=True,
                rejection_reason=None,
            ))
            await session.commit()
            return PromoteResult(
                decision_id=decision_id,
                artifact_id=artifact_id,
                accepted=True,
                reason=None,
                canary_id=canary_id,
            )


async def _find_ready_endpoint(
    session: AsyncSession,
    *,
    artifact_id: str,
    role: ArtifactRole,
    tenant_id: str,
    task_id: str,
) -> Endpoint | None:
    """Find a ready endpoint for ``artifact_id`` in the given tenant/task.

    Tenant-scoped filter (H2 fix): previously the query ignored tenant
    boundaries, making cross-tenant route selection possible. We now
    filter on ``tenant_id`` and ``task_id`` columns added to the
    ``endpoints`` table specifically for this scoping.
    """
    from sqlalchemy import select

    stmt = (
        select(Endpoint)
        .where(
            Endpoint.artifact_id == artifact_id,
            Endpoint.tenant_id == tenant_id,
            Endpoint.task_id == task_id,
            Endpoint.role == role.value,
            Endpoint.state == "ready",
        )
        .limit(1)
    )
    return (await session.execute(stmt)).scalar_one_or_none()


async def _find_current_champion(
    session: AsyncSession,
    *,
    tenant_id: str,
    task_id: str,
) -> Endpoint | None:
    """Find the current champion endpoint for the given ``(tenant, task)``."""
    from sqlalchemy import select

    stmt = (
        select(Endpoint)
        .where(
            Endpoint.tenant_id == tenant_id,
            Endpoint.task_id == task_id,
            Endpoint.role == ArtifactRole.CHAMPION.value,
            Endpoint.state == "ready",
        )
        .limit(1)
    )
    return (await session.execute(stmt)).scalar_one_or_none()


__all__ = ["ArtifactPromoter", "PromoteResult"]
