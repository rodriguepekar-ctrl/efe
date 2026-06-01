"""Tests for the rollback daemon."""

from __future__ import annotations

from datetime import timedelta

import numpy as np
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.registry import ChampionRegistry
from efe_gate.clock import FakeClock
from efe_gate.config import RollbackSettings
from efe_gate.db.models import (
    Champion,
    ChampionHistory,
    EvalRun,
    GoldenSet,
    RollbackMonitor,
)
from efe_gate.ids import new_uuid
from efe_gate.rollback import (
    RollbackDaemon,
    StaticLiveMetricsProvider,
)


async def _seed_champion_with_history(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet, fake_clock: FakeClock,
) -> tuple[EvalRun, EvalRun]:
    # First promote.
    e1 = EvalRun(
        id="11111111-1111-1111-1111-111111111111",
        tenant_id="acme", task_id="routing",
        candidate_run_id="aaaa1111-1111-1111-1111-111111111111",
        candidate_uri="s3://v1",
        golden_set_id=golden_set.id,
        state="decided", state_version=1, submitted_by="a",
    )
    session.add(e1)
    await session.commit()
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e1.candidate_run_id, artifact_uri="s3://v1",
        eval_run_id=e1.id,
        metrics_snapshot={
            "accuracy": {"role": "primary", "value": 0.9},
        },
        promoted_by="e",
    )
    fake_clock.advance(60)
    # Second promote.
    e2 = EvalRun(
        id="22222222-2222-2222-2222-222222222222",
        tenant_id="acme", task_id="routing",
        candidate_run_id="bbbb2222-2222-2222-2222-222222222222",
        candidate_uri="s3://v2",
        golden_set_id=golden_set.id,
        state="decided", state_version=1, submitted_by="a",
    )
    session.add(e2)
    await session.commit()
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e2.candidate_run_id, artifact_uri="s3://v2",
        eval_run_id=e2.id,
        metrics_snapshot={
            "accuracy": {"role": "primary", "value": 0.91},
        },
        promoted_by="e",
    )
    await session.commit()
    return e1, e2


async def _add_monitor(
    session: AsyncSession, e2: EvalRun, fake_clock: FakeClock,
    window_minutes: int = 5,
) -> RollbackMonitor:
    now = fake_clock.now()
    m = RollbackMonitor(
        id=new_uuid(),
        tenant_id=e2.tenant_id, task_id=e2.task_id,
        run_id=e2.candidate_run_id, eval_run_id=e2.id,
        started_at=now,
        deadline=now + timedelta(minutes=window_minutes),
        state="watching",
        observed_metrics={},
    )
    session.add(m)
    await session.commit()
    return m


async def test_tick_marks_expired_after_deadline(
    session: AsyncSession,
    registry: ChampionRegistry,
    golden_set: GoldenSet,
    fake_clock: FakeClock,
) -> None:
    _, e2 = await _seed_champion_with_history(
        session, registry, golden_set, fake_clock,
    )
    await _add_monitor(session, e2, fake_clock, window_minutes=1)
    fake_clock.advance(120)  # past deadline
    daemon = RollbackDaemon(
        session=session, registry=registry,
        live_metrics=StaticLiveMetricsProvider(),
        settings=RollbackSettings(
            window_minutes=1, tick_seconds=30,
            min_live_samples=1, auto_rollback_min_delta=0.1,
        ),
        clock=fake_clock,
    )
    report = await daemon.tick()
    await session.commit()
    assert len(report.expired) == 1
    assert report.rolled_back == ()


async def test_tick_rolls_back_on_live_regression(
    session: AsyncSession,
    registry: ChampionRegistry,
    golden_set: GoldenSet,
    fake_clock: FakeClock,
) -> None:
    _, e2 = await _seed_champion_with_history(
        session, registry, golden_set, fake_clock,
    )
    await _add_monitor(session, e2, fake_clock, window_minutes=60)
    # Simulate live accuracy of ~0.6 — way below baseline 0.91.
    live = np.array([0, 1, 0, 0, 0, 0, 1, 0, 0, 1] * 50, dtype=np.float64)
    lp = StaticLiveMetricsProvider(data={
        ("acme", "routing", "accuracy"): live,
    })
    daemon = RollbackDaemon(
        session=session, registry=registry, live_metrics=lp,
        settings=RollbackSettings(
            window_minutes=60, tick_seconds=30,
            min_live_samples=50, auto_rollback_min_delta=0.05,
            auto_rollback_threshold_p_value=0.9,
        ),
        clock=fake_clock,
    )
    report = await daemon.tick()
    await session.commit()
    assert len(report.rolled_back) == 1
    # Champion rolled back to v1.
    champ = await session.get(Champion, ("acme", "routing"))
    assert champ is not None
    assert champ.artifact_uri == "s3://v1"


async def test_tick_still_watching_when_healthy(
    session: AsyncSession,
    registry: ChampionRegistry,
    golden_set: GoldenSet,
    fake_clock: FakeClock,
) -> None:
    _, e2 = await _seed_champion_with_history(
        session, registry, golden_set, fake_clock,
    )
    await _add_monitor(session, e2, fake_clock, window_minutes=60)
    # Live roughly matches baseline 0.91.
    live = np.array([1] * 90 + [0] * 10, dtype=np.float64)
    lp = StaticLiveMetricsProvider(data={
        ("acme", "routing", "accuracy"): live,
    })
    daemon = RollbackDaemon(
        session=session, registry=registry, live_metrics=lp,
        settings=RollbackSettings(
            window_minutes=60, tick_seconds=30,
            min_live_samples=10, auto_rollback_min_delta=0.1,
        ),
        clock=fake_clock,
    )
    report = await daemon.tick()
    await session.commit()
    assert e2.candidate_run_id in report.still_watching


async def test_tick_skips_when_champion_changed(
    session: AsyncSession,
    registry: ChampionRegistry,
    golden_set: GoldenSet,
    fake_clock: FakeClock,
) -> None:
    _, e2 = await _seed_champion_with_history(
        session, registry, golden_set, fake_clock,
    )
    mon = await _add_monitor(session, e2, fake_clock, window_minutes=60)
    # Mutate champion externally to simulate another replace.
    champ = await session.get(Champion, ("acme", "routing"))
    assert champ is not None
    champ.run_id = "cccc-overridden-externally"
    await session.commit()
    daemon = RollbackDaemon(
        session=session, registry=registry,
        live_metrics=StaticLiveMetricsProvider(),
        settings=RollbackSettings(
            window_minutes=60, tick_seconds=30,
            min_live_samples=1,
        ),
        clock=fake_clock,
    )
    report = await daemon.tick()
    await session.commit()
    assert mon.run_id in report.expired


async def test_history_append_only(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    await _seed_champion_with_history(
        session, registry, golden_set, fake_clock,
    )
    stmt = select(ChampionHistory).order_by(ChampionHistory.at.asc())
    rows = list((await session.execute(stmt)).scalars())
    # Two promotes = 2 history rows.
    assert len(rows) == 2
    assert {r.action for r in rows} == {"promote"}
