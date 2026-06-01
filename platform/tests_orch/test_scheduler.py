"""Tests for the scheduler: pool selection, quotas, live capacity."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.config import OrchSettings, PoolSettings
from efe_orch.db.models import FinetuneRun
from efe_orch.errors import NoSuitablePool, PoolSaturated, QuotaExceeded
from efe_orch.scheduler import (
    PoolCapacity,
    PoolRegistry,
    PoolSelector,
    RunShape,
    SchedulerService,
)
from efe_orch.scheduler.quotas import QuotaGuard
from efe_orch.state_machine.states import RunState


def _settings() -> OrchSettings:
    return OrchSettings.from_env(overrides={
        "environment": "test",
        "database": {"url": "sqlite+aiosqlite:///:memory:", "enable_rls": False},
    })


# --------------------------------------------------------------------- pure


def test_selector_picks_cheapest() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(dataset_rows=1000),
        PoolCapacity(current={}),
    )
    assert dec.pool == "cpu-small"
    assert dec.cost_score == 1


def test_selector_skips_saturated_pool() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(dataset_rows=1000),
        PoolCapacity(current={"cpu-small": 8}),  # at capacity
    )
    assert dec.pool == "cpu-large"
    # cpu-small must appear as rejected.
    assert any(r[0] == "cpu-small" for r in dec.rejected)


def test_selector_respects_dataset_size() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(dataset_rows=50_000),
        PoolCapacity(current={}),
    )
    assert dec.pool == "cpu-large"


def test_selector_respects_gpu_need() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(dataset_rows=500, needs_gpu=True),
        PoolCapacity(current={}),
    )
    assert dec.pool.startswith("gpu")


def test_selector_no_gpu_pool_raises() -> None:
    reg = PoolRegistry((
        PoolSettings(name="only-cpu", cpu_request=1.0, memory_gib=4.0),
    ))
    sel = PoolSelector(reg)
    with pytest.raises(NoSuitablePool):
        sel.pick(
            RunShape(dataset_rows=100, needs_gpu=True),
            PoolCapacity(current={}),
        )


def test_selector_all_saturated_raises() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    all_full = {
        "cpu-small": 8, "cpu-large": 4, "gpu-a10": 4, "gpu-a100": 2,
    }
    with pytest.raises(PoolSaturated):
        sel.pick(RunShape(dataset_rows=1000), PoolCapacity(current=all_full))


def test_selector_requested_pool_override() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(dataset_rows=100, requested_pool="gpu-a10"),
        PoolCapacity(current={}),
    )
    assert dec.pool == "gpu-a10"


def test_selector_requested_unknown_pool() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    with pytest.raises(NoSuitablePool):
        sel.pick(
            RunShape(dataset_rows=100, requested_pool="phantom"),
            PoolCapacity(current={}),
        )


def test_selector_priority_filtering() -> None:
    reg = PoolRegistry((
        PoolSettings(name="p1", priority_classes=("standard",),
                     cpu_request=1.0, memory_gib=1.0, cost_score=1),
        PoolSettings(name="p2", priority_classes=("premium",),
                     cpu_request=2.0, memory_gib=2.0, cost_score=2),
    ))
    sel = PoolSelector(reg)
    # standard run can only go to p1
    dec = sel.pick(
        RunShape(dataset_rows=10, priority="standard"),
        PoolCapacity(current={}),
    )
    assert dec.pool == "p1"
    # premium priority: p1 rejected, p2 accepted
    dec = sel.pick(
        RunShape(dataset_rows=10, priority="premium"),
        PoolCapacity(current={}),
    )
    assert dec.pool == "p2"


def test_selector_whitelist() -> None:
    reg = PoolRegistry(_settings().scheduler.pools)
    sel = PoolSelector(reg)
    dec = sel.pick(
        RunShape(
            dataset_rows=100,
            allowed_pools=("cpu-large",),
        ),
        PoolCapacity(current={}),
    )
    assert dec.pool == "cpu-large"


# --------------------------------------------------------------- with DB


async def test_quota_guard_reserve_then_release(
    session: AsyncSession,
) -> None:
    q = QuotaGuard(session, _settings().quota)
    res = await q.reserve_run_slot(tenant_id="acme")
    assert res.tenant_id == "acme"
    await q.release_concurrency("acme")
    # Row is persisted with concurrency back to 0.
    row = (await q._get_or_create("acme"))
    assert row.current_concurrent_runs == 0
    assert row.current_monthly_runs == 1  # monthly is NOT decremented


async def test_quota_concurrency_cap(session: AsyncSession) -> None:
    settings = _settings()
    # Patch to 2.
    q = QuotaGuard(
        session,
        settings.quota.model_copy(update={"default_max_concurrent_runs": 2}),
    )
    await q.reserve_run_slot(tenant_id="acme")
    await q.reserve_run_slot(tenant_id="acme")
    with pytest.raises(QuotaExceeded):
        await q.reserve_run_slot(tenant_id="acme")


async def test_scheduler_service_live_capacity(
    session: AsyncSession,
) -> None:
    cfg = _settings()
    # Seed one run in SCHEDULED on cpu-large
    run = FinetuneRun(
        id="aaaaaaaa-0000-0000-0000-000000000001",
        tenant_id="acme", task_id="routing",
        submitted_by="alice", snapshot_id="sn1",
        state=RunState.SCHEDULED.value, priority="standard",
        pool="cpu-large", attempt=1, retry_count=0,
    )
    session.add(run)
    await session.commit()

    svc = SchedulerService(
        session,
        scheduler_settings=cfg.scheduler,
        quota_settings=cfg.quota,
    )
    cap = await svc.live_capacity()
    assert cap.load("cpu-large") == 1
    assert cap.load("gpu-a10") == 0


async def test_scheduler_service_plan_placement_end_to_end(
    session: AsyncSession,
) -> None:
    cfg = _settings()
    svc = SchedulerService(
        session,
        scheduler_settings=cfg.scheduler,
        quota_settings=cfg.quota,
    )
    decision = await svc.plan_placement(
        tenant_id="acme",
        shape=RunShape(dataset_rows=50),
    )
    assert decision.pool == "cpu-small"
