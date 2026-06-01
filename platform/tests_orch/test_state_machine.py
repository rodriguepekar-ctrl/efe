"""Tests for the state machine + advisory locks."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import FakeClock
from efe_orch.db.models import FinetuneRun, RunStateTransition
from efe_orch.errors import (
    InvalidTransition,
    RunNotFound,
    RunTerminated,
    StateRaceDetected,
    TenantMismatch,
)
from efe_orch.state_machine import (
    ACTIVE_STATES,
    TERMINAL_STATES,
    VALID_TRANSITIONS,
    InMemoryLockBackend,
    RunState,
    StateMachine,
    is_terminal,
)
from efe_orch.state_machine.locks import lock_key_for


async def _seed(session: AsyncSession, **overrides: object) -> FinetuneRun:
    defaults: dict[str, object] = dict(
        id="11111111-1111-1111-1111-111111111111",
        tenant_id="acme",
        task_id="routing",
        submitted_by="alice@acme",
        snapshot_id="22222222-2222-2222-2222-222222222222",
        state=RunState.DRAFT.value,
        state_version=0,
        priority="standard",
        attempt=0,
        retry_count=0,
    )
    defaults.update(overrides)
    run = FinetuneRun(**defaults)  # type: ignore[arg-type]
    session.add(run)
    await session.commit()
    return run


def test_lock_key_stable() -> None:
    k1 = lock_key_for("abc-123")
    k2 = lock_key_for("abc-123")
    assert k1 == k2
    assert 0 <= k1 <= 0x7FFF_FFFF_FFFF_FFFF


def test_valid_transitions_are_closed() -> None:
    # Every target of a transition must itself be a known state.
    for src, dst in VALID_TRANSITIONS:
        assert isinstance(src, RunState)
        assert isinstance(dst, RunState)


def test_no_transition_leaves_terminal() -> None:
    for terminal in TERMINAL_STATES:
        for src, _ in VALID_TRANSITIONS:
            # Terminal never appears as a source.
            assert src is not terminal


def test_active_states_predicate() -> None:
    assert all(is_terminal(s) for s in TERMINAL_STATES)
    assert not any(is_terminal(s) for s in ACTIVE_STATES)


async def test_happy_path_transitions(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    run = await _seed(session)
    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    path = [
        RunState.QUEUED, RunState.SCHEDULED, RunState.TRAINING,
        RunState.EVALUATING, RunState.UPLOADING, RunState.SUCCEEDED,
    ]
    for step_idx, to in enumerate(path):
        ctx = await sm.transition(
            run_id=run.id, to_state=to, actor="system",
            reason=f"step {step_idx}",
        )
        assert ctx.to_state is to
        assert ctx.new_version == step_idx + 1
    await session.refresh(run)
    assert run.state == RunState.SUCCEEDED.value
    assert run.state_version == len(path)
    assert run.queued_at is not None
    assert run.started_at is not None
    assert run.finished_at is not None


async def test_illegal_transition_raises(
    session: AsyncSession,
) -> None:
    run = await _seed(session, state=RunState.DRAFT.value)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    with pytest.raises(InvalidTransition):
        await sm.transition(
            run_id=run.id, to_state=RunState.TRAINING, actor="system",
        )


async def test_cannot_transition_terminal(
    session: AsyncSession,
) -> None:
    run = await _seed(session, state=RunState.SUCCEEDED.value)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    with pytest.raises(RunTerminated):
        await sm.transition(
            run_id=run.id, to_state=RunState.QUEUED, actor="system",
        )


async def test_tenant_mismatch_rejected(session: AsyncSession) -> None:
    run = await _seed(session, tenant_id="acme")
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    with pytest.raises(TenantMismatch):
        await sm.transition(
            run_id=run.id, to_state=RunState.QUEUED, actor="system",
            tenant_id="evilcorp",
        )


async def test_unknown_run_raises(session: AsyncSession) -> None:
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    with pytest.raises(RunNotFound):
        await sm.transition(
            run_id="00000000-0000-0000-0000-000000000000",
            to_state=RunState.QUEUED, actor="x",
        )


async def test_expected_version_mismatch_raises(session: AsyncSession) -> None:
    run = await _seed(session)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    # Perform one transition to bump version to 1.
    await sm.transition(
        run_id=run.id, to_state=RunState.QUEUED, actor="system",
    )
    with pytest.raises(StateRaceDetected):
        await sm.transition(
            run_id=run.id, to_state=RunState.SCHEDULED, actor="system",
            expected_version=0,  # stale
        )


async def test_transition_writes_audit_row(session: AsyncSession) -> None:
    run = await _seed(session)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    await sm.transition(
        run_id=run.id, to_state=RunState.QUEUED, actor="alice",
        reason="submitted", payload={"source": "cli"},
    )
    await session.commit()
    hist = await sm.history(run.id)
    assert len(hist) == 1
    row: RunStateTransition = hist[0]
    assert row.from_state == RunState.DRAFT.value
    assert row.to_state == RunState.QUEUED.value
    assert row.actor == "alice"
    assert row.reason == "submitted"
    assert row.payload["source"] == "cli"


async def test_mutate_callback_runs(session: AsyncSession) -> None:
    run = await _seed(session)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())

    def _bump(r: FinetuneRun) -> None:
        r.pool = "cpu-large"

    await sm.transition(
        run_id=run.id, to_state=RunState.QUEUED, actor="system",
        mutate=_bump,
    )
    await session.refresh(run)
    assert run.pool == "cpu-large"


async def test_async_mutate_callback_runs(session: AsyncSession) -> None:
    run = await _seed(session)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())

    async def _bump(r: FinetuneRun) -> None:
        r.pool = "cpu-small"

    await sm.transition(
        run_id=run.id, to_state=RunState.QUEUED, actor="system",
        mutate=_bump,
    )
    await session.refresh(run)
    assert run.pool == "cpu-small"


async def test_cancel_before_schedule(session: AsyncSession) -> None:
    run = await _seed(session)
    sm = StateMachine(session, lock_backend=InMemoryLockBackend())
    await sm.transition(
        run_id=run.id, to_state=RunState.CANCELLED, actor="user",
    )
    await session.refresh(run)
    assert run.state == RunState.CANCELLED.value
    assert run.finished_at is not None


async def test_all_retry_edges_legal(session: AsyncSession) -> None:
    """From every ACTIVE state we must be able to enter RETRYING."""
    for active in ACTIVE_STATES:
        run = await _seed(
            session,
            id=f"aaaaaaaa-0000-0000-0000-{active.value[:12].ljust(12, '0')}",
            state=active.value,
        )
        sm = StateMachine(session, lock_backend=InMemoryLockBackend())
        ctx = await sm.transition(
            run_id=run.id, to_state=RunState.RETRYING, actor="x",
        )
        assert ctx.to_state is RunState.RETRYING
        await session.commit()
