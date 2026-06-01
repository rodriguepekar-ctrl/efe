"""Tests for the heartbeat recorder + reaper."""

from __future__ import annotations

from datetime import timedelta

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import FakeClock
from efe_orch.config import HeartbeatSettings
from efe_orch.db.models import FinetuneRun, WorkerHeartbeat
from efe_orch.errors import (
    RunNotFound,
    StaleHeartbeat,
    WorkerProtocolError,
)
from efe_orch.heartbeat import HeartbeatReaper, HeartbeatRecorder
from efe_orch.heartbeat.recorder import HeartbeatInput
from efe_orch.state_machine import (
    InMemoryLockBackend,
    RunState,
    StateMachine,
)


async def _seed_run(
    session: AsyncSession, *, state: str = RunState.TRAINING.value,
    run_id: str = "11111111-1111-1111-1111-111111111111",
    worker_id: str = "worker-1",
) -> FinetuneRun:
    run = FinetuneRun(
        id=run_id,
        tenant_id="acme", task_id="routing",
        submitted_by="alice", snapshot_id="sn",
        state=state, priority="standard",
        pool="cpu-small",
        worker_id=worker_id,
        attempt=1, retry_count=0,
    )
    session.add(run)
    await session.commit()
    return run


async def test_record_heartbeat_persists(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session)
    rec = HeartbeatRecorder(session, clock=fake_clock)
    row = await rec.record(HeartbeatInput(
        run_id="11111111-1111-1111-1111-111111111111",
        worker_id="worker-1",
        beat_seq=1,
        progress_ratio=0.5,
        step=100,
    ))
    assert row.beat_seq == 1
    assert row.progress_ratio == 0.5
    await session.commit()


async def test_rejects_out_of_order_beats(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session)
    rec = HeartbeatRecorder(session, clock=fake_clock)
    await rec.record(HeartbeatInput(
        run_id="11111111-1111-1111-1111-111111111111",
        worker_id="worker-1",
        beat_seq=5,
    ))
    with pytest.raises(StaleHeartbeat):
        await rec.record(HeartbeatInput(
            run_id="11111111-1111-1111-1111-111111111111",
            worker_id="worker-1",
            beat_seq=5,
        ))
    with pytest.raises(StaleHeartbeat):
        await rec.record(HeartbeatInput(
            run_id="11111111-1111-1111-1111-111111111111",
            worker_id="worker-1",
            beat_seq=3,
        ))


async def test_unknown_run(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    rec = HeartbeatRecorder(session, clock=fake_clock)
    with pytest.raises(RunNotFound):
        await rec.record(HeartbeatInput(
            run_id="00000000-0000-0000-0000-000000000000",
            worker_id="w",
            beat_seq=1,
        ))


async def test_terminal_run_rejects_hb(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session, state=RunState.SUCCEEDED.value)
    rec = HeartbeatRecorder(session, clock=fake_clock)
    with pytest.raises(StaleHeartbeat):
        await rec.record(HeartbeatInput(
            run_id="11111111-1111-1111-1111-111111111111",
            worker_id="worker-1",
            beat_seq=1,
        ))


async def test_progress_ratio_bounded(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    rec = HeartbeatRecorder(session, clock=fake_clock)
    with pytest.raises(WorkerProtocolError):
        await rec.record(HeartbeatInput(
            run_id="x", worker_id="w", beat_seq=1, progress_ratio=1.5,
        ))


async def test_missing_run_id(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    rec = HeartbeatRecorder(session, clock=fake_clock)
    with pytest.raises(WorkerProtocolError):
        await rec.record(HeartbeatInput(
            run_id="", worker_id="w", beat_seq=1,
        ))


# --------------------------------------------------------------- reaper


async def test_reaper_transitions_stale_run_to_lost(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    # Seed a run in TRAINING with updated_at far in the past.
    run = await _seed_run(session)
    now = fake_clock.now()
    run.updated_at = now - timedelta(seconds=600)
    await session.commit()

    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    reaper = HeartbeatReaper(
        session,
        state_machine=sm,
        settings=HeartbeatSettings(interval_s=30, miss_threshold=3),
        clock=fake_clock,
    )
    report = await reaper.scan()
    assert report.scanned == 1
    assert report.reaped == (run.id,)
    await session.refresh(run)
    assert run.state == RunState.LOST.value


async def test_reaper_leaves_fresh_run(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session)
    # Fresh heartbeat now.
    rec = HeartbeatRecorder(session, clock=fake_clock)
    await rec.record(HeartbeatInput(
        run_id="11111111-1111-1111-1111-111111111111",
        worker_id="worker-1",
        beat_seq=1,
    ))
    await session.commit()
    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    reaper = HeartbeatReaper(
        session,
        state_machine=sm,
        settings=HeartbeatSettings(interval_s=30, miss_threshold=3),
        clock=fake_clock,
    )
    report = await reaper.scan()
    assert report.scanned == 0
    assert report.reaped == ()


async def test_reaper_ignores_terminal_runs(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    run = await _seed_run(
        session, state=RunState.SUCCEEDED.value,
        run_id="22222222-2222-2222-2222-222222222222",
    )
    now = fake_clock.now()
    run.updated_at = now - timedelta(seconds=600)
    await session.commit()

    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    reaper = HeartbeatReaper(
        session,
        state_machine=sm,
        settings=HeartbeatSettings(interval_s=30, miss_threshold=3),
        clock=fake_clock,
    )
    report = await reaper.scan()
    # Succeeded not in ACTIVE_STATES → not reaped.
    assert report.reaped == ()


async def test_recorder_stamps_worker_id_on_run(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session, worker_id="")  # cleared
    rec = HeartbeatRecorder(session, clock=fake_clock)
    await rec.record(HeartbeatInput(
        run_id="11111111-1111-1111-1111-111111111111",
        worker_id="worker-9",
        beat_seq=1,
    ))
    await session.commit()
    run = await session.get(
        FinetuneRun,
        "11111111-1111-1111-1111-111111111111",
    )
    assert run is not None
    assert run.worker_id == "worker-9"


async def test_recorder_creates_row_in_db(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    await _seed_run(session)
    rec = HeartbeatRecorder(session, clock=fake_clock)
    await rec.record(HeartbeatInput(
        run_id="11111111-1111-1111-1111-111111111111",
        worker_id="worker-1",
        beat_seq=1,
    ))
    await session.commit()
    rows = list(
        (await session.execute(select(WorkerHeartbeat))).scalars()
    )
    assert len(rows) == 1
