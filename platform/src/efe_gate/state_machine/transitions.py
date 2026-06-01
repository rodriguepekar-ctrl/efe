"""Atomic state transitions for :class:`EvalRun`.

Mirrors the pattern used in ``efe_orch.state_machine.transitions``:
the :class:`StateMachine` reads the row, checks the transition is
in :data:`VALID_TRANSITIONS`, writes the new state, bumps the
version, and runs an optional :class:`Mutator` callback — all inside
the same unit of work so the caller can commit atomically.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import Clock, SystemClock
from efe_gate.db.models import EvalRun
from efe_gate.errors import (
    InvalidTransition,
    NotFound,
    TenantMismatch,
    TerminalStateReached,
)
from efe_gate.state_machine.states import (
    TERMINAL_STATES,
    VALID_TRANSITIONS,
    EvalState,
)


@dataclass(frozen=True, slots=True)
class TransitionCtx:
    """Returned by :meth:`StateMachine.transition`."""

    run_id: str
    from_state: EvalState | None
    to_state: EvalState
    actor: str
    reason: str
    new_version: int
    at: datetime


Mutator = Callable[[EvalRun], Awaitable[None]] | Callable[[EvalRun], None]


class StateMachine:
    """Thin transition service bound to a session."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._clock: Clock = clock or SystemClock()

    async def transition(
        self,
        *,
        run_id: str,
        to_state: EvalState,
        actor: str,
        tenant_id: str | None = None,
        reason: str = "",
        payload: Mapping[str, Any] | None = None,
        mutate: Mutator | None = None,
    ) -> TransitionCtx:
        run = await self._session.get(EvalRun, run_id)
        if run is None:
            raise NotFound(f"eval run {run_id!r} does not exist")
        if tenant_id is not None and run.tenant_id != tenant_id:
            raise TenantMismatch(
                f"eval run {run_id!r} is in a different tenant",
            )
        current = EvalState(run.state)

        # PROMOTED is near-terminal: only ROLLED_BACK is allowed after.
        # The ROLLED_BACK transition is in VALID_TRANSITIONS so the
        # generic check covers it. Other terminal states block
        # everything.
        if (
            current in TERMINAL_STATES
            and to_state is not EvalState.ROLLED_BACK
        ):
            raise TerminalStateReached(
                f"eval run {run_id!r} is {current.value}",
            )

        if (current, to_state) not in VALID_TRANSITIONS:
            raise InvalidTransition(
                f"illegal transition {current.value} → {to_state.value}",
                detail={
                    "run_id": run_id,
                    "from": current.value,
                    "to": to_state.value,
                },
            )

        now = self._clock.now()
        run.state = to_state.value
        run.state_version = run.state_version + 1
        run.updated_at = now
        if to_state is EvalState.RUNNING and run.started_at is None:
            run.started_at = now
        if (
            to_state in TERMINAL_STATES
            or to_state in (EvalState.PROMOTED, EvalState.REJECTED)
        ) and run.finished_at is None:
            run.finished_at = now

        if mutate is not None:
            r = mutate(run)
            if r is not None:  # awaitable
                await r

        _ = payload  # payload currently unused on the run row; audit
                     # chain consumes it via a separate call.

        await self._session.flush()
        return TransitionCtx(
            run_id=run_id,
            from_state=current,
            to_state=to_state,
            actor=actor,
            reason=reason,
            new_version=run.state_version,
            at=now,
        )


__all__ = ["Mutator", "StateMachine", "TransitionCtx"]
