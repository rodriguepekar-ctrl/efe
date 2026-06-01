"""Tests for the evaluation FSM."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import FakeClock
from efe_gate.db.models import EvalRun, GoldenSet
from efe_gate.errors import (
    InvalidTransition,
    NotFound,
    TenantMismatch,
    TerminalStateReached,
)
from efe_gate.state_machine import (
    EvalState,
    StateMachine,
    TERMINAL_STATES,
    VALID_TRANSITIONS,
    is_active,
    is_terminal,
    is_waiting,
)


async def _seed(
    session: AsyncSession, golden_set: GoldenSet,
    state: str = EvalState.QUEUED.value,
) -> EvalRun:
    run = EvalRun(
        id="11111111-1111-1111-1111-111111111111",
        tenant_id="acme", task_id="routing",
        candidate_run_id="22222222-2222-2222-2222-222222222222",
        candidate_uri="file:///cand",
        golden_set_id=golden_set.id,
        state=state, state_version=0,
        submitted_by="alice",
    )
    session.add(run)
    await session.commit()
    return run


def test_valid_transitions_closed() -> None:
    for src, dst in VALID_TRANSITIONS:
        assert isinstance(src, EvalState)
        assert isinstance(dst, EvalState)


def test_terminal_states_never_sources() -> None:
    # Except PROMOTED → ROLLED_BACK which we allow.
    for src, _dst in VALID_TRANSITIONS:
        if src in TERMINAL_STATES:
            pytest.fail(f"terminal {src} appears as source")


def test_predicates() -> None:
    assert is_terminal(EvalState.REJECTED)
    assert is_active(EvalState.RUNNING)
    assert is_waiting(EvalState.QUEUED)


async def test_happy_path(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set)
    sm = StateMachine(session, clock=fake_clock)
    for target in (
        EvalState.RUNNING, EvalState.MEASURED,
        EvalState.DECIDED, EvalState.PROMOTED,
    ):
        ctx = await sm.transition(
            run_id=run.id, to_state=target, actor="test",
        )
        assert ctx.to_state is target
    await session.refresh(run)
    assert run.state == EvalState.PROMOTED.value
    assert run.state_version == 4


async def test_illegal_transition(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set)
    sm = StateMachine(session, clock=fake_clock)
    with pytest.raises(InvalidTransition):
        await sm.transition(
            run_id=run.id, to_state=EvalState.PROMOTED, actor="x",
        )


async def test_rejected_is_terminal(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set, state=EvalState.REJECTED.value)
    sm = StateMachine(session, clock=fake_clock)
    with pytest.raises(TerminalStateReached):
        await sm.transition(
            run_id=run.id, to_state=EvalState.PROMOTED, actor="x",
        )


async def test_promoted_to_rolled_back_allowed(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set, state=EvalState.PROMOTED.value)
    sm = StateMachine(session, clock=fake_clock)
    ctx = await sm.transition(
        run_id=run.id, to_state=EvalState.ROLLED_BACK, actor="daemon",
    )
    assert ctx.to_state is EvalState.ROLLED_BACK


async def test_tenant_mismatch(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set)
    sm = StateMachine(session, clock=fake_clock)
    with pytest.raises(TenantMismatch):
        await sm.transition(
            run_id=run.id, to_state=EvalState.RUNNING,
            actor="x", tenant_id="evil",
        )


async def test_unknown_run(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    sm = StateMachine(session, clock=fake_clock)
    with pytest.raises(NotFound):
        await sm.transition(
            run_id="00000000-0000-0000-0000-000000000000",
            to_state=EvalState.RUNNING, actor="x",
        )


async def test_mutate_callback_applied(
    session: AsyncSession, golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    run = await _seed(session, golden_set)
    sm = StateMachine(session, clock=fake_clock)

    def _stamp(r: EvalRun) -> None:
        r.decision = "promote"

    await sm.transition(
        run_id=run.id, to_state=EvalState.RUNNING,
        actor="x", mutate=_stamp,
    )
    await session.refresh(run)
    assert run.decision == "promote"
