"""Tests for the RunService — submit, dispatch, cancel, retry flows."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import FakeClock
from efe_orch.config import OrchSettings
from efe_orch.dispatcher import FakeKubernetesClient
from efe_orch.errors import (
    CannotCancel,
    QuotaExceeded,
    RunNotFound,
    ValidationError,
)
from efe_orch.heartbeat.recorder import HeartbeatInput
from efe_orch.scheduler import SchedulerService
from efe_orch.services import (
    CheckpointReport,
    ErrorReport,
    ListRunsFilter,
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
        session,
        scheduler=sched,
        state_machine=sm,
        k8s_client=fake_k8s,
        k8s_settings=cfg.k8s,
        retry_settings=cfg.retry,
        clock=fake_clock,
    )
    worker_svc = WorkerService(
        session,
        state_machine=sm,
        run_service=run_svc,
        scheduler=sched,
        clock=fake_clock,
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


# --------------------------------------------------------------- submit


async def test_submit_happy_path(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    view = await run_svc.submit(_sub())
    assert view.state is RunState.QUEUED
    assert view.pool == "cpu-small"
    assert view.attempt == 1
    await session.commit()


async def test_submit_validation_errors(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    with pytest.raises(ValidationError):
        await run_svc.submit(_sub(tenant_id=""))
    with pytest.raises(ValidationError):
        await run_svc.submit(_sub(priority="mystery"))
    with pytest.raises(ValidationError):
        await run_svc.submit(_sub(dataset_rows=-1))


async def test_submit_quota_limit_rolls_back(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
        quota_overrides={"default_max_concurrent_runs": 1},
    )
    await run_svc.submit(_sub())
    await session.commit()
    with pytest.raises(QuotaExceeded):
        await run_svc.submit(_sub())


# -------------------------------------------------------------- dispatch


async def test_dispatch_moves_to_scheduled(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    v = await run_svc.submit(_sub())
    view = await run_svc.dispatch(run_id=v.id)
    assert view.state is RunState.SCHEDULED
    assert view.worker_id is not None
    assert len(fake_k8s.submitted) == 1
    manifest = fake_k8s.submitted[0]
    assert manifest["metadata"]["labels"]["efe.io/tenant-id"] == "acme"


async def test_dispatch_from_wrong_state_raises(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    v = await run_svc.submit(_sub())
    await run_svc.dispatch(run_id=v.id)
    with pytest.raises(ValidationError):
        await run_svc.dispatch(run_id=v.id)  # already SCHEDULED


# --------------------------------------------------------------- cancel


async def test_cancel_draft_path_immediate(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    v = await run_svc.submit(_sub())
    view = await run_svc.cancel(run_id=v.id, actor="alice")
    assert view.state is RunState.CANCELLED
    assert view.finished_at is not None


async def test_cancel_running_goes_to_cancelling(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub())
    v = await run_svc.dispatch(run_id=v.id)
    # Take to TRAINING via first heartbeat.
    assert v.worker_id is not None
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    view = await run_svc.cancel(run_id=v.id, actor="alice")
    assert view.state is RunState.CANCELLING


async def test_cancel_terminal_raises(
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
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    # Walk to SUCCEEDED.
    await worker_svc.report_result(ResultReport(
        run_id=v.id, worker_id=v.worker_id,
        artifact_url="s3://x", metrics={"f1": 0.9},
    ))
    with pytest.raises(CannotCancel):
        await run_svc.cancel(run_id=v.id, actor="alice")


# -------------------------------------------------------------- list/get


async def test_get_and_list(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    v1 = await run_svc.submit(_sub())
    await run_svc.submit(_sub())
    got = await run_svc.get(run_id=v1.id)
    assert got.id == v1.id
    rows, next_tok = await run_svc.list(ListRunsFilter(tenant_id="acme"))
    assert len(rows) == 2
    assert next_tok is None


async def test_list_respects_state_filter(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    await run_svc.submit(_sub())
    rows, _ = await run_svc.list(ListRunsFilter(
        tenant_id="acme", state=RunState.TRAINING,
    ))
    assert rows == ()


async def test_get_unknown_raises(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, _, _, _ = await _build_services(session, fake_k8s, fake_clock)
    with pytest.raises(RunNotFound):
        await run_svc.get(run_id="00000000-0000-0000-0000-000000000000")


# ----------------------------------------------------------- full lifecycle


async def test_full_happy_lifecycle(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub(dataset_rows=100))
    v = await run_svc.dispatch(run_id=v.id)
    assert v.state is RunState.SCHEDULED
    assert v.worker_id is not None

    # 1st heartbeat → TRAINING
    v = await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    assert v.state is RunState.TRAINING

    # Checkpoint persists
    cid = await worker_svc.report_checkpoint(CheckpointReport(
        run_id=v.id, worker_id=v.worker_id, step=50,
        blob_uri="s3://bucket/ckpt-50", is_best=True,
    ))
    assert cid

    # Result: TRAINING→EVALUATING→UPLOADING→SUCCEEDED
    v = await worker_svc.report_result(ResultReport(
        run_id=v.id, worker_id=v.worker_id,
        artifact_url="s3://bucket/artifact",
        metrics={"f1": 0.91, "loss": 0.12},
    ))
    assert v.state is RunState.SUCCEEDED
    assert v.result_artifact_url == "s3://bucket/artifact"
    assert v.result_metrics["f1"] == 0.91


async def test_error_retry_path(
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
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    # Transient error → should retry back to QUEUED.
    v = await worker_svc.report_error(ErrorReport(
        run_id=v.id, worker_id=v.worker_id,
        error_code="pod_evicted",
        error_message="node drained",
    ))
    assert v.state is RunState.QUEUED
    assert v.attempt == 2
    assert v.retry_count == 1


async def test_oom_error_upgrades_pool(
    session: AsyncSession,
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
) -> None:
    run_svc, worker_svc, _, _ = await _build_services(
        session, fake_k8s, fake_clock,
    )
    v = await run_svc.submit(_sub(dataset_rows=100))
    assert v.pool == "cpu-small"
    v = await run_svc.dispatch(run_id=v.id)
    assert v.worker_id is not None
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    v = await worker_svc.report_error(ErrorReport(
        run_id=v.id, worker_id=v.worker_id,
        error_code="cuda_oom",
        error_message="CUDA OOM",
    ))
    assert v.pool == "cpu-large"
    assert v.attempt == 2


async def test_data_error_fails_immediately(
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
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    v = await worker_svc.report_error(ErrorReport(
        run_id=v.id, worker_id=v.worker_id,
        error_code="snapshot_missing",
        error_message="snapshot deleted",
    ))
    assert v.state is RunState.FAILED
    assert v.last_error_code == "snapshot_missing"


async def test_cancel_during_running_then_worker_completes(
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
    await worker_svc.heartbeat(HeartbeatInput(
        run_id=v.id, worker_id=v.worker_id, beat_seq=1,
    ))
    v = await run_svc.cancel(run_id=v.id, actor="alice")
    assert v.state is RunState.CANCELLING
    # Worker reports result after cancel requested → honour cancel.
    v = await worker_svc.report_result(ResultReport(
        run_id=v.id, worker_id=v.worker_id,
        artifact_url="s3://late",
        metrics={"f1": 0.8},
    ))
    assert v.state is RunState.CANCELLED
