"""RollbackExecutor — SLO <60s, idempotence, phases."""

from __future__ import annotations

from datetime import timedelta

import pytest

from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.errors import (
    NoPreviousChampion,
    NoStandbyAvailable,
)
from efe_registry.observability.audit import RecordingAuditChannel
from efe_registry.rollback.executor import (
    NullDrainAction,
    NullInflightMetricSource,
    RollbackExecutor,
)
from efe_registry.rollback.standby import (
    InMemoryStandbyProbe,
    PreWarmedStandbyManager,
)
from efe_registry.router.lua_runner import InMemoryLuaRunner
from efe_registry.router.updater import NullNatsPublisher, RoutingUpdater
from tests_registry.conftest import _seed_artifact, _seed_endpoint


async def _seed_full_scenario(session_factory, fake_clock):
    async with session_factory() as session:
        prev = await _seed_artifact(
            session, semver="1.0.0",
            state=ArtifactState.DEPRECATED,
            state_version=2,
            promoted_at=fake_clock.now() - timedelta(hours=2),
        )
        cur = await _seed_artifact(
            session, semver="2.0.0",
            state=ArtifactState.CHAMPION,
            state_version=1,
            promoted_at=fake_clock.now() - timedelta(minutes=20),
        )
        prev_ep = await _seed_endpoint(
            session, artifact_id=prev.artifact_id,
            role=ArtifactRole.STANDBY_WARM,
            state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        cur_ep = await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.CHAMPION,
            state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await session.commit()
        return prev, cur, prev_ep, cur_ep


@pytest.mark.asyncio
async def test_happy_path_under_slo(
    session_factory, settings, fake_clock,
):
    prev, cur, prev_ep, cur_ep = await _seed_full_scenario(
        session_factory, fake_clock,
    )
    probe = InMemoryStandbyProbe(ready_endpoints={prev_ep.endpoint_id})
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua,
        nats_publisher=NullNatsPublisher(),
        clock=fake_clock,
    )
    audit = RecordingAuditChannel()
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        drain=NullDrainAction(),
        metrics=NullInflightMetricSource(),
        audit=audit,
        clock=fake_clock,
    )
    receipt = await executor.execute(
        tenant_id=prev.tenant_id, task_id=prev.task_id,
        actor="tests", reason="unit",
        trigger_kind="manual",
    )
    assert receipt.final_state == "completed"
    assert receipt.latency_ms < settings.rollback.hard_deadline_s * 1000
    # phases recorded
    assert "lookup_s" in receipt.phases
    assert "swap_s" in receipt.phases
    assert "total_s" in receipt.phases
    # audit emitted
    assert audit.rollbacks
    assert audit.rollbacks[0]["trigger_kind"] == "manual"
    assert audit.rollbacks[0]["from_artifact_id"] == cur.artifact_id
    assert audit.rollbacks[0]["to_artifact_id"] == prev.artifact_id


@pytest.mark.asyncio
async def test_idempotent_second_call_returns_same_receipt(
    session_factory, settings, fake_clock,
):
    prev, cur, prev_ep, cur_ep = await _seed_full_scenario(
        session_factory, fake_clock,
    )
    probe = InMemoryStandbyProbe(ready_endpoints={prev_ep.endpoint_id})
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        clock=fake_clock,
    )
    idem = b"\xaa" * 32
    r1 = await executor.execute(
        tenant_id=prev.tenant_id, task_id=prev.task_id,
        actor="tests", reason="unit",
        trigger_kind="manual",
        idempotency_key=idem,
    )
    r2 = await executor.execute(
        tenant_id=prev.tenant_id, task_id=prev.task_id,
        actor="tests", reason="unit",
        trigger_kind="manual",
        idempotency_key=idem,
    )
    assert r1.rollback_id == r2.rollback_id
    assert r1.latency_ms == r2.latency_ms


@pytest.mark.asyncio
async def test_fails_when_no_previous_champion(
    session_factory, settings, fake_clock,
):
    # Only the current champion exists — nothing to roll back to.
    async with session_factory() as session:
        cur = await _seed_artifact(
            session, semver="2.0.0",
            state=ArtifactState.CHAMPION,
            state_version=1,
            promoted_at=fake_clock.now(),
        )
        await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.CHAMPION, state="ready",
        )
        await session.commit()

    probe = InMemoryStandbyProbe()
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        clock=fake_clock,
    )
    with pytest.raises(NoPreviousChampion):
        await executor.execute(
            tenant_id=cur.tenant_id, task_id=cur.task_id,
            actor="tests", reason="unit",
        )


@pytest.mark.asyncio
async def test_fails_when_standby_not_ready(
    session_factory, settings, fake_clock,
):
    prev, cur, prev_ep, cur_ep = await _seed_full_scenario(
        session_factory, fake_clock,
    )
    probe = InMemoryStandbyProbe(ready_endpoints=set())  # empty
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        clock=fake_clock,
    )
    with pytest.raises(NoStandbyAvailable):
        await executor.execute(
            tenant_id=prev.tenant_id, task_id=prev.task_id,
            actor="tests", reason="unit",
        )
