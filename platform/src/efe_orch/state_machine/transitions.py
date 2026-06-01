"""Atomic, advisory-locked state-machine transitions.

Every mutation of ``finetune_runs.state`` MUST flow through
:meth:`StateMachine.transition`. The method guarantees:

1. **Exclusive access** to the row — we acquire an advisory lock
   keyed by ``run_id`` before reading the row. Anyone else trying
   to transition the same run will wait (or, on SQLite, block on the
   in-process mutex).
2. **Legal transition check** — the ``(from, to)`` pair must be in
   :data:`VALID_TRANSITIONS`; otherwise
   :class:`InvalidTransition` is raised.
3. **Optimistic concurrency** — the row is re-read under the lock,
   and if the observed ``state_version`` doesn't match the caller's
   ``expected_version`` (when provided) we raise
   :class:`StateRaceDetected`.
4. **Side-effect atomicity** — the state mutation, the
   ``run_state_transitions`` insert, and any caller-supplied
   ``mutate`` callback run inside the same unit of work (flushed in
   one commit by the caller).

The transition function is the *only* place where the service
writes to ``finetune_runs.state`` — any direct assignment is
considered a bug and caught by a simple grep in CI (see
``tools/forbid_direct_state_write.sh`` in the devops repo).
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import Clock, SystemClock
from efe_orch.db.models import FinetuneRun, RunStateTransition
from efe_orch.errors import (
    InvalidTransition,
    RunNotFound,
    RunTerminated,
    StateRaceDetected,
    TenantMismatch,
)
from efe_orch.ids import new_uuid
from efe_orch.observability.metrics import MetricsRegistry, noop_registry
from efe_orch.state_machine.locks import InMemoryLockBackend, LockBackend
from efe_orch.state_machine.states import (
    TERMINAL_STATES,
    VALID_TRANSITIONS,
    RunState,
)


@dataclass(frozen=True, slots=True)
class TransitionCtx:
    """Immutable summary of one transition — returned by ``transition``."""

    run_id: str
    from_state: RunState | None
    to_state: RunState
    actor: str
    reason: str
    new_version: int
    at: datetime
    transition_id: str


#: Signature of the optional mutator callback passed to ``transition``.
Mutator = Callable[[FinetuneRun], Awaitable[None]] | Callable[[FinetuneRun], None]


class StateMachine:
    """Thin service that owns transition logic.

    One instance per request/unit-of-work. It's *not* a singleton —
    callers wire it with their session, lock backend, and clock.
    """

    def __init__(
        self,
        session: AsyncSession,
        *,
        lock_backend: LockBackend | None = None,
        clock: Clock | None = None,
        metrics: MetricsRegistry | None = None,
    ) -> None:
        self._session = session
        self._lock_backend: LockBackend = lock_backend or InMemoryLockBackend()
        self._clock: Clock = clock or SystemClock()
        self._metrics: MetricsRegistry = metrics or noop_registry()

    # ------------------------------------------------------------------ API

    async def transition(
        self,
        *,
        run_id: str,
        to_state: RunState,
        actor: str,
        tenant_id: str | None = None,
        reason: str = "",
        payload: Mapping[str, Any] | None = None,
        expected_version: int | None = None,
        mutate: Mutator | None = None,
    ) -> TransitionCtx:
        """Perform one state transition atomically.

        Parameters
        ----------
        run_id:
            ID of the run to mutate. Must exist in ``finetune_runs``.
        to_state:
            Target :class:`RunState`. Must form a legal edge with the
            current state (see :data:`VALID_TRANSITIONS`).
        actor:
            Who initiated the transition (``system``, ``worker/<id>``,
            or a user email). Stored verbatim in the audit row.
        tenant_id:
            If supplied, asserts ``run.tenant_id == tenant_id``. This
            is the last line of defence against cross-tenant writes.
        reason:
            Free-text explanation ("heartbeat timeout", "user cancel",
            "eval loss NaN", …). Stored in the audit row.
        payload:
            Arbitrary dict merged into the audit row's ``payload``.
        expected_version:
            If supplied, the row's current ``state_version`` must
            match. Used by callers to guard against stale reads.
        mutate:
            Optional callback executed *after* the state mutation,
            *before* flush. Receives the row. Intended for small,
            transition-local field updates (e.g. ``pool = 'cpu-small'``
            on ``DRAFT→QUEUED``).

        Returns
        -------
        TransitionCtx
            Immutable summary of what just happened.
        """
        async with self._lock_backend.acquire(self._session, run_id) as _lock:
            run = await self._session.get(FinetuneRun, run_id)
            if run is None:
                raise RunNotFound(f"run {run_id!r} does not exist")

            if tenant_id is not None and run.tenant_id != tenant_id:
                raise TenantMismatch(
                    f"run {run_id!r} belongs to a different tenant"
                )

            current = RunState(run.state)

            if current in TERMINAL_STATES:
                raise RunTerminated(
                    f"run {run_id!r} is already {current.value}; "
                    f"no further transitions allowed",
                )

            if (current, to_state) not in VALID_TRANSITIONS:
                raise InvalidTransition(
                    f"illegal transition {current.value} → {to_state.value}",
                    detail={"run_id": run_id, "from": current.value,
                            "to": to_state.value},
                )

            if (
                expected_version is not None
                and run.state_version != expected_version
            ):
                raise StateRaceDetected(
                    f"run {run_id!r} version {run.state_version} != "
                    f"expected {expected_version}",
                    detail={"observed": run.state_version,
                            "expected": expected_version},
                )

            # --- write the new state ---
            now = self._clock.now()
            run.state = to_state.value
            run.state_version = run.state_version + 1
            run.updated_at = now

            # Timestamp semantics: stamp on *enter*.
            if to_state is RunState.QUEUED and run.queued_at is None:
                run.queued_at = now
            if to_state is RunState.TRAINING and run.started_at is None:
                run.started_at = now
            if to_state in TERMINAL_STATES and run.finished_at is None:
                run.finished_at = now

            # --- optional mutate hook ---
            if mutate is not None:
                r = mutate(run)
                if r is not None:  # awaitable
                    await r

            # --- audit row ---
            trans_id = new_uuid()
            trans = RunStateTransition(
                id=trans_id,
                run_id=run_id,
                from_state=current.value,
                to_state=to_state.value,
                actor=actor,
                reason=reason,
                payload=dict(payload or {}),
                created_at=now,
            )
            self._session.add(trans)
            await self._session.flush()

            # Emit transition metric (no-op if metrics registry absent).
            self._metrics.runs_transitioned.inc(
                1.0,
                from_state=current.value,
                to_state=to_state.value,
            )

            return TransitionCtx(
                run_id=run_id,
                from_state=current,
                to_state=to_state,
                actor=actor,
                reason=reason,
                new_version=run.state_version,
                at=now,
                transition_id=trans_id,
            )

    # ---------------------------------------------------------------- helpers

    async def history(self, run_id: str) -> list[RunStateTransition]:
        """Return the full transition history for ``run_id`` (asc)."""
        stmt = (
            select(RunStateTransition)
            .where(RunStateTransition.run_id == run_id)
            .order_by(
                RunStateTransition.created_at.asc(),
                RunStateTransition.id.asc(),
            )
        )
        res = await self._session.execute(stmt)
        return list(res.scalars())


__all__ = ["Mutator", "StateMachine", "TransitionCtx"]
