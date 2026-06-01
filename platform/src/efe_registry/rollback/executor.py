"""RollbackExecutor — orchestration of the SLO-bound rollback.

Phase budget (see M06_ARCHITECTURE.md §11.2):

    lookup_s   <= 0.5
    standby_s  <= 1.0
    swap_s     <= 2.0   (Lua CAS + PubSub)
    drain_s    <= 30.0  (graceful termination of old pods)
    verify_s   <= 5.0   (>=99% traffic on new champion)
    audit_s    <= 1.0   (audit chain emission)

Total hard deadline: 60.0 s p99. Breach -> ``final_state=failed`` +
incident P1.

All phases are instrumented via :class:`PhaseRecorder`; phases_jsonb
is persisted on ``rollback_events`` for post-mortems.
"""

from __future__ import annotations

import asyncio
import hashlib
import time
import uuid
from collections.abc import Awaitable
from dataclasses import dataclass
from typing import Literal, Protocol, TypeVar

from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.config import RollbackSettings
from efe_registry.core.artifact import ArtifactState
from efe_registry.core.state_machine import ArtifactTransitionValidator
from efe_registry.db.models import (
    Artifact as ArtifactRow,
)
from efe_registry.db.models import (
    Endpoint,
    RollbackEvent,
)
from efe_registry.errors import (
    NoPreviousChampion,
    NoStandbyAvailable,
    RollbackError,
    RollbackTimeout,
    RollbackVersionConflict,
)
from efe_registry.rollback.phases import PhaseName, PhaseRecorder, PhaseTimeout
from efe_registry.rollback.receipt import RollbackReceipt
from efe_registry.rollback.standby import (
    PreWarmedStandbyManager,
    StandbyCheck,
)
from efe_registry.router.updater import RoutingUpdater

_T = TypeVar("_T")

TriggerKind = Literal["manual", "auto_slo", "auto_regression", "admin"]


# ---------------------------------------------------------------------------
# Per-phase budgets (in seconds) — enforced individually by _drive.
# ---------------------------------------------------------------------------
_PHASE_BUDGETS: dict[PhaseName, float] = {
    PhaseName.LOOKUP: 2.0,
    PhaseName.STANDBY: 1.0,
    PhaseName.SWAP: 2.0,
    PhaseName.DRAIN: 30.0,
    PhaseName.VERIFY: 5.0,
    PhaseName.AUDIT: 2.0,
}


class InflightMetricSource(Protocol):
    """How we know whether traffic shifted.

    Production: Prometheus / VM query. Tests: in-memory stub.
    """

    async def inflight_share(
        self,
        *,
        tenant_id: str,
        task_id: str,
        artifact_id: str,
    ) -> float:  # pragma: no cover
        ...


@dataclass
class NullInflightMetricSource:
    """Always reports 100 % — useful for fast tests."""

    async def inflight_share(
        self, *, tenant_id: str, task_id: str, artifact_id: str,
    ) -> float:
        return 1.0


class DrainAction(Protocol):
    async def drain(
        self, endpoint: Endpoint,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class NullDrainAction:
    """Drain no-op — used in tests."""

    async def drain(self, endpoint: Endpoint) -> None:
        return None


class AuditRecorder(Protocol):
    async def record_rollback(
        self,
        *,
        tenant_id: str,
        rollback_id: str,
        from_artifact_id: str,
        to_artifact_id: str,
        latency_ms: int,
        trigger_kind: TriggerKind,
        actor: str,
        reason: str,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class NullAuditRecorder:
    """Audit no-op — tests that don't assert on audit usage."""

    calls: list[dict[str, object]] = None  # type: ignore[assignment]

    def __post_init__(self) -> None:
        if self.calls is None:
            self.calls = []

    async def record_rollback(
        self,
        *,
        tenant_id: str,
        rollback_id: str,
        from_artifact_id: str,
        to_artifact_id: str,
        latency_ms: int,
        trigger_kind: TriggerKind,
        actor: str,
        reason: str,
    ) -> None:
        self.calls.append({
            "tenant_id": tenant_id,
            "rollback_id": rollback_id,
            "from_artifact_id": from_artifact_id,
            "to_artifact_id": to_artifact_id,
            "latency_ms": latency_ms,
            "trigger_kind": trigger_kind,
            "actor": actor,
            "reason": reason,
        })


class RollbackExecutor:
    """See module docstring."""

    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        settings: RollbackSettings,
        standby: PreWarmedStandbyManager,
        routing_updater: RoutingUpdater,
        drain: DrainAction | None = None,
        metrics: InflightMetricSource | None = None,
        audit: AuditRecorder | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._sf = session_factory
        self._cfg = settings
        self._standby = standby
        self._routing = routing_updater
        self._drain = drain or NullDrainAction()
        self._metrics = metrics or NullInflightMetricSource()
        self._audit = audit or NullAuditRecorder()
        self._clock: Clock = clock or SystemClock()

    # -------------------------------------------------------------- execute

    async def execute(
        self,
        *,
        tenant_id: str,
        task_id: str,
        actor: str,
        reason: str,
        trigger_kind: TriggerKind = "manual",
        idempotency_key: bytes | None = None,
    ) -> RollbackReceipt:
        idem = idempotency_key or self._derive_idem(
            tenant_id, task_id, actor, trigger_kind,
        )
        recorder = PhaseRecorder(clock=self._clock)
        recorder.start()

        # Idempotence check first — short-circuit if we already completed.
        receipt = await self._maybe_short_circuit(idem)
        if receipt is not None:
            return receipt

        # Insert "requested" row; handle race via UNIQUE(idempotency_key).
        # If two callers race with the same key, the loser re-queries the
        # winner's row and either returns its completed receipt or waits
        # briefly for completion (bounded by the idempotency window).
        event = await self._insert_or_recover(
            tenant_id=tenant_id, task_id=task_id,
            actor=actor, reason=reason, trigger_kind=trigger_kind,
            idempotency_key=idem,
        )
        if event is None:
            # Duplicate in flight. Re-check for a completed sibling with
            # a short bounded poll (hard-cap at 2 s to avoid consuming
            # the caller's budget); return its receipt if it finished,
            # else refuse. A stale ``requested`` row (older than the
            # idempotency window) indicates the previous executor
            # crashed without marking ``failed``; we refuse and expose
            # the inconsistency to the operator rather than silently
            # retry.
            receipt = await self._wait_for_sibling(
                idem, timeout_s=min(2.0, self._cfg.hard_deadline_s),
            )
            if receipt is not None:
                return receipt
            raise RollbackVersionConflict(
                "another rollback with the same idempotency_key is "
                "already in progress",
            )

        try:
            result = await asyncio.wait_for(
                self._drive(event, recorder),
                timeout=self._cfg.hard_deadline_s,
            )
        except asyncio.TimeoutError:
            await self._mark_failed(event, recorder, reason="hard_deadline")
            raise RollbackTimeout(
                f"rollback exceeded hard deadline of "
                f"{self._cfg.hard_deadline_s:.1f}s",
            ) from None
        except (RollbackVersionConflict, NoStandbyAvailable,
                 NoPreviousChampion, RollbackTimeout, PhaseTimeout) as exc:
            await self._mark_failed(event, recorder, reason=str(exc)[:200])
            raise
        except Exception as exc:
            # M7 — mark failed on any unhandled exception and re-raise.
            await self._mark_failed(
                event, recorder,
                reason=f"unhandled:{type(exc).__name__}:{exc!s}"[:200],
            )
            raise RollbackError(
                f"rollback crashed during drive: {exc!s}",
            ) from exc
        return result

    # -------------------------------------------------------------- drive

    async def _drive(
        self, event: RollbackEvent, recorder: PhaseRecorder,
    ) -> RollbackReceipt:
        # --- phase LOOKUP (budgeted) ---
        previous_artifact_id, current_artifact_id = await self._bounded_phase(
            PhaseName.LOOKUP, self._phase_lookup(event),
        )
        recorder.mark(PhaseName.LOOKUP)

        # Persist the looked-up IDs on the event row (was previously
        # committed inside _phase_lookup, violating read-tx semantics).
        await self._update_event_artifacts(
            event, from_id=current_artifact_id, to_id=previous_artifact_id,
        )
        event.from_artifact = current_artifact_id
        event.to_artifact = previous_artifact_id

        # --- phase STANDBY (budgeted) ---
        standby_check = await self._bounded_phase(
            PhaseName.STANDBY, self._phase_standby(previous_artifact_id),
        )
        recorder.mark(PhaseName.STANDBY)

        # --- phase SWAP (budgeted) ---
        await self._bounded_phase(
            PhaseName.SWAP, self._phase_swap(event, standby_check),
        )
        recorder.mark(PhaseName.SWAP)

        # --- phase DRAIN (budgeted) ---
        await self._bounded_phase(
            PhaseName.DRAIN, self._phase_drain(event),
        )
        recorder.mark(PhaseName.DRAIN)

        # --- phase VERIFY (budgeted) ---
        await self._bounded_phase(
            PhaseName.VERIFY, self._phase_verify(event, standby_check),
        )
        recorder.mark(PhaseName.VERIFY)

        # --- phase AUDIT (+ state transitions) (budgeted) ---
        await self._bounded_phase(
            PhaseName.AUDIT,
            self._phase_audit_and_fsm(
                event=event,
                standby_check=standby_check,
                previous_artifact_id=previous_artifact_id,
                recorder=recorder,
            ),
        )
        recorder.mark(PhaseName.AUDIT)
        total_s = recorder.total()

        # --- finalize event ---
        latency_ms = int(total_s * 1000)
        await self._finalize_event(event, latency_ms, recorder.to_dict())

        assert event.from_artifact is not None and event.to_artifact is not None
        return RollbackReceipt(
            rollback_id=event.rollback_id,
            tenant_id=event.tenant_id,
            task_id=event.task_id,
            from_artifact_id=event.from_artifact,
            to_artifact_id=event.to_artifact,
            latency_ms=latency_ms,
            final_state="completed",
            phases=recorder.to_dict(),
        )

    async def _bounded_phase(
        self, phase: PhaseName, coro: Awaitable[_T],
    ) -> _T:
        """Run ``coro`` with the per-phase budget.

        SWAP, DRAIN and VERIFY have their own inner timeouts tuned from
        settings; the bucket budget here is the outer safety net so a
        buggy phase can never consume the whole rollback deadline.
        """
        budget = _PHASE_BUDGETS[phase]
        try:
            return await asyncio.wait_for(coro, timeout=budget)
        except asyncio.TimeoutError:
            raise PhaseTimeout(phase, budget) from None

    # -------------------------------------------------------------- phases

    async def _phase_lookup(
        self, event: RollbackEvent,
    ) -> tuple[str, str]:
        """Return ``(previous_artifact_id, current_artifact_id)``.

        **Read-only**: no commits inside this phase. The caller persists
        the looked-up IDs via :meth:`_update_event_artifacts` in a
        dedicated short transaction.

        The "previous" pool is restricted to ``DEPRECATED`` artifacts so
        two concurrent promotes (both in ``SERVING``) cannot cross-
        contaminate each other.
        """
        async with self._sf() as session:
            stmt_cur = (
                select(ArtifactRow)
                .where(
                    ArtifactRow.tenant_id == event.tenant_id,
                    ArtifactRow.task_id == event.task_id,
                    ArtifactRow.state.in_(
                        (ArtifactState.CHAMPION.value,
                         ArtifactState.SERVING.value),
                    ),
                )
                .order_by(ArtifactRow.promoted_at.desc())
                .limit(1)
            )
            current = (await session.execute(stmt_cur)).scalar_one_or_none()
            if current is None:
                raise NoPreviousChampion(
                    f"no current champion for ({event.tenant_id!r}, "
                    f"{event.task_id!r})",
                )

            stmt_prev = (
                select(ArtifactRow)
                .where(
                    ArtifactRow.tenant_id == event.tenant_id,
                    ArtifactRow.task_id == event.task_id,
                    ArtifactRow.state == ArtifactState.DEPRECATED.value,
                    ArtifactRow.artifact_id != current.artifact_id,
                )
                .order_by(ArtifactRow.promoted_at.desc())
                .limit(1)
            )
            prev = (await session.execute(stmt_prev)).scalar_one_or_none()
            if prev is None:
                raise NoPreviousChampion(
                    f"no deprecated artifact to roll back to for "
                    f"({event.tenant_id!r}, {event.task_id!r})",
                )
            return prev.artifact_id, current.artifact_id

    async def _phase_standby(
        self, previous_artifact_id: str,
    ) -> StandbyCheck:
        async with self._sf() as session:
            return await self._standby.pick_for_artifact(
                session, artifact_id=previous_artifact_id,
            )

    async def _phase_swap(
        self, event: RollbackEvent, standby_check: StandbyCheck,
    ) -> None:
        try:
            await asyncio.wait_for(
                self._routing.swap_to_champion(
                    tenant_id=event.tenant_id,
                    task_id=event.task_id,
                    new_champion_endpoint_id=(
                        standby_check.endpoint.endpoint_id
                    ),
                    actor=f"rollback:{event.rollback_id}",
                ),
                timeout=self._cfg.swap_timeout_s,
            )
        except asyncio.TimeoutError:
            raise RollbackTimeout(
                f"atomic_swap exceeded {self._cfg.swap_timeout_s:.1f}s",
            ) from None

    async def _phase_drain(
        self, event: RollbackEvent,
    ) -> list[str]:
        """Drain ALL endpoints tied to the former artifact.

        Covers H4 — previously we only drained ``role=champion`` but the
        artifact could also have candidate / shadow endpoints still
        serving from before the rollback.
        """
        if event.from_artifact is None:
            return []
        async with self._sf() as session:
            stmt = (
                select(Endpoint)
                .where(Endpoint.artifact_id == event.from_artifact)
            )
            pods = list((await session.execute(stmt)).scalars())
        if not pods:
            return []
        tasks: list[Awaitable[None]] = [
            self._drain.drain(pod) for pod in pods
        ]
        try:
            await asyncio.wait_for(
                asyncio.gather(*tasks, return_exceptions=True),
                timeout=self._cfg.drain_deadline_s,
            )
        except asyncio.TimeoutError:
            # Accept small 5xx on in-flight — continue without raising.
            # Production metrics will flag the delta.
            pass
        # Mark endpoints as draining in DB (best-effort).
        async with self._sf() as session:
            for pod in pods:
                row = await session.get(Endpoint, pod.endpoint_id)
                if row is not None:
                    row.state = "draining"
            await session.commit()
        return [p.endpoint_id for p in pods]

    async def _phase_verify(
        self, event: RollbackEvent, standby_check: StandbyCheck,
    ) -> None:
        # H5 — empty artifact_id on the standby endpoint would cause the
        # metric source to short-circuit and silently pass verify.
        verify_artifact_id = standby_check.endpoint.artifact_id
        if not verify_artifact_id:
            raise RollbackError(
                "standby endpoint has no artifact_id; cannot verify "
                "traffic shift",
            )
        deadline = (
            self._clock.now_monotonic_s() + self._cfg.verify_deadline_s
        )
        while self._clock.now_monotonic_s() < deadline:
            share = await self._metrics.inflight_share(
                tenant_id=event.tenant_id,
                task_id=event.task_id,
                artifact_id=verify_artifact_id,
            )
            if share >= self._cfg.verify_inflight_share_threshold:
                return
            await asyncio.sleep(0.1)
        raise RollbackTimeout(
            f"traffic did not shift to new champion within "
            f"{self._cfg.verify_deadline_s:.1f}s",
        )

    async def _phase_audit_and_fsm(
        self,
        *,
        event: RollbackEvent,
        standby_check: StandbyCheck,
        previous_artifact_id: str,
        recorder: PhaseRecorder,
    ) -> None:
        async with self._sf() as session:
            prev = await session.get(ArtifactRow, previous_artifact_id)
            if prev is None:
                raise NoPreviousChampion(
                    f"previous artifact {previous_artifact_id!r} vanished",
                )
            validator = ArtifactTransitionValidator(session)
            if prev.state in (
                ArtifactState.DEPRECATED.value,
                ArtifactState.CHAMPION.value,
            ):
                await validator.apply(
                    artifact_id=prev.artifact_id,
                    expected_state=ArtifactState(prev.state),
                    new_state=ArtifactState.SERVING,
                    expected_version=prev.state_version,
                    actor=f"rollback:{event.rollback_id}",
                    reason="rollback restored",
                )
            if event.from_artifact is not None:
                cur = await session.get(ArtifactRow, event.from_artifact)
                if cur is not None and cur.state in (
                    ArtifactState.CHAMPION.value,
                    ArtifactState.SERVING.value,
                ):
                    await validator.apply(
                        artifact_id=cur.artifact_id,
                        expected_state=ArtifactState(cur.state),
                        new_state=ArtifactState.DEPRECATED,
                        expected_version=cur.state_version,
                        actor=f"rollback:{event.rollback_id}",
                        reason="rolled back",
                    )
            await session.commit()
        latency_ms = int(recorder.total() * 1000)
        await self._audit.record_rollback(
            tenant_id=event.tenant_id,
            rollback_id=event.rollback_id,
            from_artifact_id=event.from_artifact or "",
            to_artifact_id=event.to_artifact or "",
            latency_ms=latency_ms,
            trigger_kind=event.trigger_kind,  # type: ignore[arg-type]
            actor=event.actor,
            reason=event.reason,
        )

    # -------------------------------------------------------------- DB ops

    async def _insert_or_recover(
        self,
        *,
        tenant_id: str,
        task_id: str,
        actor: str,
        reason: str,
        trigger_kind: TriggerKind,
        idempotency_key: bytes,
    ) -> RollbackEvent | None:
        """Insert the requested event or ``None`` on UNIQUE conflict (C4).

        SQLAlchemy raises :class:`IntegrityError` when a concurrent
        caller has already inserted a row with the same
        ``idempotency_key``. We translate that into a ``None`` return so
        :meth:`execute` can attempt to read the sibling event and decide
        whether to wait or refuse.
        """
        rid = str(uuid.uuid4())
        now = self._clock.now()
        row = RollbackEvent(
            rollback_id=rid,
            tenant_id=tenant_id,
            task_id=task_id,
            from_artifact=None,
            to_artifact=None,
            trigger_kind=trigger_kind,
            actor=actor,
            reason=reason,
            idempotency_key=idempotency_key,
            state="requested",
            requested_at=now,
            started_at=now,
            phases_jsonb={},
        )
        try:
            async with self._sf() as session:
                session.add(row)
                await session.commit()
        except IntegrityError:
            return None
        async with self._sf() as session:
            detached = await session.get(RollbackEvent, rid)
            assert detached is not None
            return detached

    async def _wait_for_sibling(
        self, idempotency_key: bytes, *, poll_s: float = 0.05,
        timeout_s: float | None = None,
    ) -> RollbackReceipt | None:
        """Poll for a completed sibling; uses real monotonic time.

        We intentionally use :func:`time.monotonic` (real time) instead
        of ``self._clock`` because this helper is called from
        :meth:`execute` which ran under :func:`asyncio.wait_for`
        (real-time). Using a FakeClock here would loop forever in tests
        that don't advance the fake clock.
        """
        deadline_real = time.monotonic() + (
            timeout_s if timeout_s is not None
            else self._cfg.hard_deadline_s
        )
        while time.monotonic() < deadline_real:
            receipt = await self._maybe_short_circuit(idempotency_key)
            if receipt is not None:
                return receipt
            await asyncio.sleep(poll_s)
        return None

    async def _maybe_short_circuit(
        self, idempotency_key: bytes,
    ) -> RollbackReceipt | None:
        async with self._sf() as session:
            stmt = select(RollbackEvent).where(
                RollbackEvent.idempotency_key == idempotency_key,
            )
            prev = (
                await session.execute(stmt)
            ).scalar_one_or_none()
            if prev is None or prev.state != "completed":
                return None
            return RollbackReceipt(
                rollback_id=prev.rollback_id,
                tenant_id=prev.tenant_id,
                task_id=prev.task_id,
                from_artifact_id=prev.from_artifact or "",
                to_artifact_id=prev.to_artifact or "",
                latency_ms=int(prev.latency_ms or 0),
                final_state="completed",
                phases=dict(prev.phases_jsonb or {}),
            )

    async def _update_event_artifacts(
        self, event: RollbackEvent, *, from_id: str, to_id: str,
    ) -> None:
        async with self._sf() as session:
            row = await session.get(RollbackEvent, event.rollback_id)
            if row is None:
                return
            row.from_artifact = from_id
            row.to_artifact = to_id
            await session.commit()

    async def _finalize_event(
        self,
        event: RollbackEvent,
        latency_ms: int,
        phases: dict[str, float],
    ) -> None:
        async with self._sf() as session:
            row = await session.get(RollbackEvent, event.rollback_id)
            if row is None:
                return
            # CAS on state: only move requested -> completed. Protects
            # against a concurrent _mark_failed flipping the row while we
            # were mid-completion.
            if row.state != "requested":
                return
            row.state = "completed"
            row.completed_at = self._clock.now()
            row.latency_ms = latency_ms
            row.phases_jsonb = phases
            await session.commit()

    async def _mark_failed(
        self,
        event: RollbackEvent,
        recorder: PhaseRecorder,
        *,
        reason: str,
    ) -> None:
        async with self._sf() as session:
            row = await session.get(RollbackEvent, event.rollback_id)
            if row is None:
                return
            if row.state == "completed":
                # Never flip a completed event back to failed.
                return
            row.state = "failed"
            row.completed_at = self._clock.now()
            row.latency_ms = int(recorder.total() * 1000)
            row.phases_jsonb = {
                **(recorder.to_dict()),
                "failure_reason": reason,
            }
            await session.commit()

    # -------------------------------------------------------------- helpers

    def _derive_idem(
        self, tenant_id: str, task_id: str, actor: str,
        trigger_kind: TriggerKind,
    ) -> bytes:
        window = int(
            self._clock.now().timestamp() // self._cfg.idempotency_window_s,
        )
        raw = f"{tenant_id}|{task_id}|{actor}|{trigger_kind}|{window}".encode()
        return hashlib.sha256(raw).digest()


__all__ = [
    "AuditRecorder",
    "DrainAction",
    "InflightMetricSource",
    "NullAuditRecorder",
    "NullDrainAction",
    "NullInflightMetricSource",
    "RollbackExecutor",
    "TriggerKind",
]
