"""Canary controller — advance, halt, rollback."""

from __future__ import annotations

import uuid
from dataclasses import dataclass

import pytest

from efe_registry.canary.controller import (
    CanaryController,
)
from efe_registry.canary.schedule import SCHEDULE_GEOMETRIC
from efe_registry.canary.slo_gateway import InMemorySloGateway
from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.db.models import CanaryRun
from tests_registry.conftest import _seed_artifact, _seed_endpoint


@dataclass
class _FakeRouting:
    calls: list = None

    def __post_init__(self):
        if self.calls is None:
            self.calls = []

    async def set_canary_percent(self, **kw):
        self.calls.append(kw)
        return 1


@dataclass
class _FakeRollback:
    called_with: list = None

    def __post_init__(self):
        if self.called_with is None:
            self.called_with = []

    async def execute(self, **kw):
        self.called_with.append(kw)

        class _R:
            latency_ms = 42
            final_state = "completed"

        return _R()


async def _seed_canary(session, fake_clock, *, schedule_kind="geometric"):
    cand = await _seed_artifact(
        session, semver="2.0.0",
        state=ArtifactState.SERVING,
    )
    base = await _seed_artifact(
        session, semver="1.0.0",
        state=ArtifactState.CHAMPION,
    )
    cand_ep = await _seed_endpoint(
        session, artifact_id=cand.artifact_id, role=ArtifactRole.CANDIDATE,
    )
    base_ep = await _seed_endpoint(
        session, artifact_id=base.artifact_id, role=ArtifactRole.CHAMPION,
    )
    canary = CanaryRun(
        canary_id=str(uuid.uuid4()),
        tenant_id=cand.tenant_id,
        task_id=cand.task_id,
        candidate_artifact=cand.artifact_id,
        baseline_artifact=base.artifact_id,
        current_percent=1,
        target_percent=100,
        schedule_kind=schedule_kind,
        step_duration_s=30,
        state="ramping",
        started_at=fake_clock.now(),
        last_step_at=fake_clock.now(),
    )
    session.add(canary)
    await session.commit()
    return canary, cand, base, cand_ep, base_ep


@pytest.mark.asyncio
async def test_stays_during_dwell(
    session_factory, settings, fake_clock,
):
    async with session_factory() as session:
        canary, *_ = await _seed_canary(session, fake_clock)

    slo = InMemorySloGateway()
    routing = _FakeRouting()
    ctrl = CanaryController(
        session_factory=session_factory,
        settings=settings.canary,
        slo=slo,
        rollback=_FakeRollback(),
        routing=routing,
        clock=fake_clock,
    )
    result = await ctrl.tick(canary.canary_id)
    assert result.action == "stay"
    assert result.reason == "dwell"


@pytest.mark.asyncio
async def test_advances_after_dwell(
    session_factory, settings, fake_clock,
):
    async with session_factory() as session:
        canary, *_ = await _seed_canary(session, fake_clock)

    slo = InMemorySloGateway()
    routing = _FakeRouting()
    ctrl = CanaryController(
        session_factory=session_factory,
        settings=settings.canary,
        slo=slo,
        rollback=_FakeRollback(),
        routing=routing,
        clock=fake_clock,
    )
    # Advance past dwell.
    fake_clock.advance(700.0)
    result = await ctrl.tick(canary.canary_id)
    assert result.action == "advance"
    assert result.to_percent == SCHEDULE_GEOMETRIC[1].percent  # 5
    assert routing.calls and routing.calls[0]["percent"] == 5


@pytest.mark.asyncio
async def test_rolls_back_after_3_breaches(
    session_factory, settings, fake_clock,
):
    async with session_factory() as session:
        canary, cand, base, *_ = await _seed_canary(session, fake_clock)

    slo = InMemorySloGateway()
    slo.set_breach(
        tenant_id=canary.tenant_id, task_id=canary.task_id,
    )
    rollback = _FakeRollback()
    ctrl = CanaryController(
        session_factory=session_factory,
        settings=settings.canary,
        slo=slo,
        rollback=rollback,
        routing=_FakeRouting(),
        clock=fake_clock,
    )
    # Breach #1
    res1 = await ctrl.tick(canary.canary_id)
    assert res1.action == "stay"
    # Breach #2
    res2 = await ctrl.tick(canary.canary_id)
    assert res2.action == "stay"
    # Breach #3 → abort
    res3 = await ctrl.tick(canary.canary_id)
    assert res3.action == "rolled_back"
    assert rollback.called_with
    assert rollback.called_with[0]["trigger_kind"] == "auto_slo"


@pytest.mark.asyncio
async def test_paused_if_auto_rollback_disabled(
    session_factory, settings, fake_clock,
):
    async with session_factory() as session:
        canary, *_ = await _seed_canary(session, fake_clock)
        canary.auto_rollback = False
        await session.commit()

    slo = InMemorySloGateway()
    slo.set_breach(
        tenant_id=canary.tenant_id, task_id=canary.task_id,
    )
    rollback = _FakeRollback()
    ctrl = CanaryController(
        session_factory=session_factory,
        settings=settings.canary,
        slo=slo,
        rollback=rollback,
        routing=_FakeRouting(),
        clock=fake_clock,
    )
    for _ in range(3):
        last = await ctrl.tick(canary.canary_id)
    assert last.action == "paused"
    assert not rollback.called_with
