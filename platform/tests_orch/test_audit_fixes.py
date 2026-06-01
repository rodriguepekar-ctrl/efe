"""Regression tests for the surgical audit fixes.

Each test here corresponds to a bug found in the M03 audit. The
bug numbers match the punch list in the post-audit delivery:

    #1 CRITICAL — reaper did not release tenant quota on LOST
    #2 CRITICAL — DRAFT row flushed before quota → zombie rows
    #3 HIGH     — worker-auth: ``run.worker_id`` None ⇒ any worker accepted
    #4 HIGH     — report_result crashed on UPLOADING→UPLOADING (double call)
    #5 HIGH     — dispatch: K8s Job orphaned on FSM failure
    #6 MEDIUM   — live_capacity counted QUEUED → false saturation
    #7 LOW      — pool gpu/gpu_count consistency not enforced
"""

from __future__ import annotations

from datetime import timedelta

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import FakeClock
from efe_orch.config import (
    HeartbeatSettings,
    OrchSettings,
    PoolSettings,
)
from efe_orch.db.models import FinetuneRun, TenantQuota
from efe_orch.dispatcher import FakeKubernetesClient
from efe_orch.errors import (
    ConfigError,
    QuotaExceeded,
    UnknownWorker,
    ValidationError,
)
from efe_orch.heartbeat.reaper import HeartbeatReaper
from efe_orch.heartbeat.recorder import HeartbeatInput
from efe_orch.scheduler import SchedulerService
from efe_orch.services import (
    ResultReport,
    RunService,
    SubmitRunInput,
    WorkerService,
)
from efe_orch.state_machine import (
    InMemoryLockBackend,
    RunState,
    StateMachine,
)


def _settings(**overrides: object) -> OrchSettings:
    base: dict[str, object] = {
        "environment": "test",
        "database": {"url": "sqlite+aiosqlite:///:memory:", "enable_rls": False},
        "k8s": {"dry_run": True},
    }
    base.update(overrides)
    return OrchSettings.from_env(overrides=base)


async def _build_services(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
    *,
    quota_overrides: dict[str, object] | None = None,
) -> tuple[RunService, WorkerService, SchedulerService, StateMachine]:
    overrides: dict[str, object] = {}
    if quota_overrides:
        overrides["quota"] = quota_overrides
    cfg = _settings(**overrides)
    sched = SchedulerService(
        session,
        scheduler_settings=cfg.scheduler,
        quota_settings=cfg.quota,
    )
    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    run_svc = RunService(
        session, scheduler=sched, state_machine=sm,
        k8s_client=fake_k8s,
        k8s_settings=cfg.k8s,
        retry_settings=cfg.retry,
        clock=fake_clock,
    )
    worker_svc = WorkerService(
        session, state_machine=sm, run_service=run_svc,
        scheduler=sched, clock=fake_clock,
    )
    return run_svc, worker_svc, sched, sm


def _sub(**overrides: object) -> SubmitRunInput:
    base: dict[str, object] = dict(
        tenant_id="acme",
        task_id="routing",
        submitted_by="alice@acme",
        snapshot_id="sn-1",
        dataset_rows=500,
    )
    base.update(overrides)
    return SubmitRunInput(**base)  # type: ignore[arg-type]


# --------------------------------------------------------------------- #1


async def test_reaper_releases_quota_on_lost(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """The reaper must decrement ``current_concurrent_runs`` on LOST."""
    from sqlalchemy import update

    from efe_orch.db.models import WorkerHeartbeat

    run_svc, worker_svc, sched, sm = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    v = await run_svc.dispatch(run_id=v.id)
    assert v.worker_id is not None
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    await session.commit()

    # At this point concurrency == 1 for acme.
    q_before = await sched.quota._get_or_create("acme")
    assert q_before.current_concurrent_runs == 1

    # Backdate BOTH ``updated_at`` on the run and ``observed_at`` on
    # every heartbeat (via raw UPDATE so ``onupdate=_utcnow`` does not
    # fire). The reaper uses the latest heartbeat if any exist.
    now = fake_clock.now()
    old = now - timedelta(seconds=600)
    await session.execute(
        update(FinetuneRun)
        .where(FinetuneRun.id == v.id)
        .values(updated_at=old),
    )
    await session.execute(
        update(WorkerHeartbeat)
        .where(WorkerHeartbeat.run_id == v.id)
        .values(observed_at=old),
    )
    await session.commit()
    session.expire_all()

    reaper = HeartbeatReaper(
        session,
        state_machine=sm,
        settings=HeartbeatSettings(interval_s=30, miss_threshold=3),
        scheduler=sched,
        clock=fake_clock,
    )
    report = await reaper.scan()
    assert report.reaped == (v.id,)

    q_after = await sched.quota._get_or_create("acme")
    assert q_after.current_concurrent_runs == 0  # released!


async def test_reaper_without_scheduler_does_not_release(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    """Reaper with scheduler=None still moves runs to LOST (no crash)."""
    run = FinetuneRun(
        id="aaaaaaaa-0000-0000-0000-000000000001",
        tenant_id="acme", task_id="routing",
        submitted_by="alice", snapshot_id="sn",
        state=RunState.TRAINING.value, priority="standard",
        pool="cpu-small", worker_id="w-1",
        attempt=1, retry_count=0,
    )
    run.updated_at = fake_clock.now() - timedelta(seconds=600)
    session.add(run)
    await session.commit()

    sm = StateMachine(
        session, lock_backend=InMemoryLockBackend(), clock=fake_clock,
    )
    reaper = HeartbeatReaper(
        session, state_machine=sm,
        settings=HeartbeatSettings(interval_s=30, miss_threshold=3),
        scheduler=None,
        clock=fake_clock,
    )
    report = await reaper.scan()
    assert report.reaped == (run.id,)


# --------------------------------------------------------------------- #2


async def test_quota_failure_does_not_create_draft_row(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """A quota rejection must leave zero rows in ``finetune_runs``."""
    run_svc, _, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
        quota_overrides={"default_max_concurrent_runs": 1},
    )
    v = await run_svc.submit(_sub())
    await session.commit()
    assert v.state is RunState.QUEUED

    # Second submit must be rejected AND must not leave a DRAFT row.
    with pytest.raises(QuotaExceeded):
        await run_svc.submit(_sub())
    await session.rollback()

    rows = list(
        (await session.execute(select(FinetuneRun))).scalars()
    )
    # Exactly one row (the successful first submit).
    assert len(rows) == 1
    assert rows[0].state == RunState.QUEUED.value


# --------------------------------------------------------------------- #3


async def test_worker_auth_rejects_when_run_has_no_worker_id(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """A worker call on a run whose ``worker_id`` is NULL must be refused."""
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    # Note: not dispatched → worker_id is None.
    assert v.worker_id is None
    with pytest.raises(UnknownWorker):
        await worker_svc.heartbeat(HeartbeatInput(
            run_id=v.id, worker_id="rogue", beat_seq=1,
        ))


async def test_worker_auth_rejects_mismatched_worker_id(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    v = await run_svc.dispatch(run_id=v.id)
    assert v.worker_id is not None
    with pytest.raises(UnknownWorker):
        await worker_svc.heartbeat(HeartbeatInput(
            run_id=v.id, worker_id="attacker", beat_seq=1,
        ))


# --------------------------------------------------------------------- #4


async def test_report_result_idempotent_walk_forward(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """Re-reporting a result must not raise ``InvalidTransition``.

    The previous implementation always tried TRAINING→EVALUATING→
    UPLOADING→SUCCEEDED and crashed on a second call when the run was
    already in UPLOADING state.
    """
    run_svc, worker_svc, _, sm = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    v = await run_svc.dispatch(run_id=v.id)
    assert v.worker_id is not None
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    # Manually push the run to UPLOADING, simulating a previous partial
    # report_result call that crashed between the EVALUATING→UPLOADING
    # step and the UPLOADING→SUCCEEDED step.
    await sm.transition(
        run_id=v.id, to_state=RunState.EVALUATING, actor="t",
    )
    await sm.transition(
        run_id=v.id, to_state=RunState.UPLOADING, actor="t",
    )
    await session.commit()

    # New report_result call should walk forward from UPLOADING to
    # SUCCEEDED without raising.
    v = await worker_svc.report_result(ResultReport(
        run_id=v.id, worker_id=v.worker_id,
        artifact_url="s3://ok", metrics={"f1": 0.9},
    ))
    assert v.state is RunState.SUCCEEDED


async def test_report_result_rejects_invalid_current_state(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """Reports from unrelated states (e.g. SCHEDULED, RETRYING) must fail cleanly."""
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    v = await run_svc.dispatch(run_id=v.id)
    assert v.state is RunState.SCHEDULED
    assert v.worker_id is not None
    with pytest.raises(ValidationError):
        await worker_svc.report_result(ResultReport(
            run_id=v.id, worker_id=v.worker_id,
            artifact_url="s3://ok", metrics={},
        ))


# --------------------------------------------------------------------- #5


async def test_dispatch_orphan_cleanup_on_fsm_failure(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """When the FSM transition fails after K8s submit, the Job is deleted.

    We inject a state machine whose ``transition`` coroutine raises on
    its first call. That way the dispatcher reaches the K8s submit
    (succeeds → ``submitted`` list grows), then the FSM write raises,
    triggering the cleanup branch that deletes the orphan Job.
    """
    from efe_orch.errors import InvalidTransition

    run_svc, _, _, real_sm = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    await session.commit()

    # Swap the state machine for one whose ``transition`` always raises.
    class _BrokenSM:
        async def transition(self, **_kw: object) -> object:
            raise InvalidTransition("synthetic FSM failure")

    run_svc._sm = _BrokenSM()  # type: ignore[assignment]

    n_submit_before = len(fake_k8s.submitted)
    n_delete_before = len(fake_k8s.deletions)

    with pytest.raises(InvalidTransition):
        await run_svc.dispatch(run_id=v.id)

    # The K8s submit DID happen (the dispatcher reached it before FSM).
    assert len(fake_k8s.submitted) == n_submit_before + 1
    # The orphan Job was deleted by the cleanup branch.
    assert len(fake_k8s.deletions) == n_delete_before + 1

    # Restore the real SM to not poison subsequent fixtures.
    run_svc._sm = real_sm  # type: ignore[assignment]


# --------------------------------------------------------------------- #6


async def test_live_capacity_excludes_queued(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """A QUEUED run must NOT count against pool capacity."""
    run_svc, _, sched, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    await run_svc.submit(_sub(dataset_rows=100))  # lands in cpu-small, QUEUED
    await session.commit()
    cap = await sched.live_capacity()
    # cpu-small has 0 ACTIVE runs — the QUEUED run does not count.
    assert cap.load("cpu-small") == 0


async def test_live_capacity_counts_active(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, worker_svc, sched, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub(dataset_rows=100))
    v = await run_svc.dispatch(run_id=v.id)
    assert v.worker_id is not None
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    await session.commit()
    cap = await sched.live_capacity()
    assert cap.load("cpu-small") == 1


# --------------------------------------------------------------------- #7


def test_pool_gpu_consistency_validator_rejects_gpu_with_zero_count() -> None:
    with pytest.raises(Exception):
        PoolSettings(
            name="bad", cpu_request=1.0, memory_gib=4.0,
            gpu="nvidia.com/gpu", gpu_count=0,
        )


def test_pool_gpu_consistency_validator_rejects_count_without_gpu() -> None:
    with pytest.raises(Exception):
        PoolSettings(
            name="bad", cpu_request=1.0, memory_gib=4.0,
            gpu=None, gpu_count=1,
        )


def test_pool_gpu_consistency_valid_cpu_pool() -> None:
    pool = PoolSettings(
        name="ok-cpu", cpu_request=1.0, memory_gib=4.0,
    )
    assert pool.gpu is None
    assert pool.gpu_count == 0


def test_pool_gpu_consistency_valid_gpu_pool() -> None:
    pool = PoolSettings(
        name="ok-gpu", cpu_request=4.0, memory_gib=32.0,
        gpu="nvidia.com/gpu", gpu_count=2,
    )
    assert pool.gpu == "nvidia.com/gpu"
    assert pool.gpu_count == 2


def test_config_pool_validator_via_from_env() -> None:
    """Ensure invalid pool config raises ConfigError from from_env."""
    with pytest.raises(ConfigError):
        OrchSettings.from_env(overrides={
            "scheduler": {
                "pools": [
                    {
                        "name": "bad",
                        "cpu_request": 1.0, "memory_gib": 4.0,
                        "gpu": "nvidia.com/gpu",
                        "gpu_count": 0,
                    },
                ],
            },
        })


# ---------------------------------------------------------- extra: run labels


def test_k8s_manifest_has_run_and_worker_labels() -> None:
    from efe_orch.config import K8sSettings
    from efe_orch.dispatcher import WorkerJobSpec, render_worker_job

    pool = PoolSettings(
        name="cpu-small", cpu_request=1.0, memory_gib=2.0,
        max_concurrent_runs=4,
    )
    spec = WorkerJobSpec(
        run_id="11111111-2222-3333-4444-555555555555",
        tenant_id="acme", task_id="routing",
        snapshot_id="sn-1",
        worker_id="worker-alpha",
        pool=pool,
        base_model="m", recipe_version="v1",
    )
    m = render_worker_job(spec, K8sSettings(namespace="efe-ft"))
    labels = m["metadata"]["labels"]
    assert "efe.io/run-id-short" in labels
    assert labels["efe.io/worker-id"] == "worker-alpha"


# ------------------------------------------------------ extra: quota released


async def test_tenant_quota_row_persists_concurrency(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    """Sanity: concurrency counter survives a commit + re-read."""
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    await run_svc.submit(_sub())
    await session.commit()
    q = (await session.execute(
        select(TenantQuota).where(TenantQuota.tenant_id == "acme"),
    )).scalar_one()
    assert q.current_concurrent_runs == 1
