"""Tests for the scheduler engine and evaluators."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.randomness import FakeRandomness
from efe_scheduling.scheduler import (
    SchedulerEngine,
    SchedulerEngineConfig,
    build_evaluator,
    parse_cron,
)
from efe_scheduling.scheduler.evaluators import (
    CompositeEvaluator,
    IntervalEvaluator,
    OneShotEvaluator,
    RRuleEvaluator,
)
from efe_scheduling.scheduler.leader import InMemoryLeaderElection
from efe_scheduling.storage import (
    InMemoryJobStore,
    InMemoryScheduleStore,
)
from efe_scheduling.types import (
    CompositeSchedule,
    CronSchedule,
    IntervalSchedule,
    JobKind,
    OneShotSchedule,
    RRuleSchedule,
    Schedule,
    ScheduleState,
)


# ---------------------------------------------------------------------------
# Cron parsing
# ---------------------------------------------------------------------------


def test_parse_cron_daily_alias() -> None:
    e = parse_cron("@daily")
    assert 0 in e.minute and 0 in e.hour


def test_parse_cron_invalid_field_raises() -> None:
    with pytest.raises(Exception):
        parse_cron("x x x x x")


def test_cron_matches_expected_times() -> None:
    # Every 5 minutes at any hour
    e = parse_cron("*/5 * * * *")
    t = datetime(2026, 4, 21, 12, 5, tzinfo=timezone.utc)
    assert e.matches(t)
    t2 = datetime(2026, 4, 21, 12, 6, tzinfo=timezone.utc)
    assert not e.matches(t2)


def test_cron_iter_fires_half_open() -> None:
    e = parse_cron("0 * * * *")  # every hour on :00
    start = datetime(2026, 4, 21, 11, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 13, 30, tzinfo=timezone.utc)
    fires = list(e.iter_fires(start, end))
    assert fires == [
        datetime(2026, 4, 21, 12, 0, tzinfo=timezone.utc),
        datetime(2026, 4, 21, 13, 0, tzinfo=timezone.utc),
    ]


# ---------------------------------------------------------------------------
# Interval evaluator
# ---------------------------------------------------------------------------


def test_interval_evaluator_fires_on_grid() -> None:
    ev = IntervalEvaluator(IntervalSchedule(period_s=300, phase_s=0))
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 0, 16, 0, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end))
    # Fires at 00:05, 00:10, 00:15
    assert fires == [
        datetime(2026, 4, 21, 0, 5, 0, tzinfo=timezone.utc),
        datetime(2026, 4, 21, 0, 10, 0, tzinfo=timezone.utc),
        datetime(2026, 4, 21, 0, 15, 0, tzinfo=timezone.utc),
    ]


def test_interval_evaluator_limit() -> None:
    ev = IntervalEvaluator(IntervalSchedule(period_s=60))
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 1, 0, 0, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end, limit=3))
    assert len(fires) == 3


# ---------------------------------------------------------------------------
# One-shot
# ---------------------------------------------------------------------------


def test_one_shot_fires_once_in_window() -> None:
    at = datetime(2026, 4, 21, 10, 0, 0, tzinfo=timezone.utc)
    ev = OneShotEvaluator(OneShotSchedule(at=at))
    fires = list(ev.iter_fires(at - timedelta(hours=1), at + timedelta(hours=1)))
    assert fires == [at]


def test_one_shot_not_in_window_yields_nothing() -> None:
    at = datetime(2026, 4, 21, 10, 0, 0, tzinfo=timezone.utc)
    ev = OneShotEvaluator(OneShotSchedule(at=at))
    future_only = list(
        ev.iter_fires(at + timedelta(hours=1), at + timedelta(hours=2))
    )
    assert future_only == []


# ---------------------------------------------------------------------------
# RRULE (minimal)
# ---------------------------------------------------------------------------


def test_rrule_daily_count() -> None:
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    ev = RRuleEvaluator(
        RRuleSchedule(rrule_text="FREQ=DAILY;COUNT=3", dtstart=start)
    )
    fires = list(
        ev.iter_fires(start - timedelta(seconds=1), start + timedelta(days=10))
    )
    assert len(fires) == 3


# ---------------------------------------------------------------------------
# Composite
# ---------------------------------------------------------------------------


def test_composite_union() -> None:
    a = CronSchedule(expression="0 9 * * *")  # 9am
    b = CronSchedule(expression="0 18 * * *")  # 6pm
    ev = CompositeEvaluator(CompositeSchedule(op="union", left=a, right=b))
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 23, 59, 0, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end))
    assert len(fires) == 2


def test_composite_exclude() -> None:
    a = CronSchedule(expression="0 * * * *")  # every hour
    b = CronSchedule(expression="0 */3 * * *")  # every 3 hours
    ev = CompositeEvaluator(CompositeSchedule(op="exclude", left=a, right=b))
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 6, 59, 0, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end))
    # Every hour from 1..6 minus 3,6 = {1,2,4,5}
    assert len(fires) == 4


# ---------------------------------------------------------------------------
# SchedulerEngine integration
# ---------------------------------------------------------------------------


def _mk_schedule(
    sid: str,
    expr: str,
    now: datetime,
    last_eval: datetime | None = None,
    tenant: str = "t1",
) -> Schedule:
    spec = CronSchedule(expression=expr)
    return Schedule(
        schedule_id=sid,
        tenant_id=tenant,
        task_id=None,
        job_kind=JobKind.noop,
        spec=spec,
        payload={"hello": "world"},
        enabled=True,
        state=ScheduleState.enabled,
        state_version=1,
        last_evaluated_at=last_eval,
        created_at=now,
        updated_at=now,
    )


def test_engine_enqueues_and_deduplicates() -> None:
    fc = FakeClock(datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc))
    rnd = FakeRandomness(seed=0)
    sch = InMemoryScheduleStore()
    jobs = InMemoryJobStore()
    leader = InMemoryLeaderElection(key="engine-test", owner_id="w1")
    engine = SchedulerEngine(
        tenant_id="t1",
        schedule_store=sch,
        job_store=jobs,
        leader=leader,
        clock=fc,
        random_source=rnd,
        config=SchedulerEngineConfig(catch_up_h=24),
    )

    async def scenario() -> None:
        now = fc.now()
        # Pre-seed last_evaluated_at so the engine only considers the new window.
        await sch.create(_mk_schedule("s1", "*/1 * * * *", now, last_eval=now))
        # Advance 5 minutes — expect 5 fires.
        fc.advance(5 * 60)
        out = await engine.tick()
        assert out.leader
        assert out.fires_enqueued == 5
        # Second tick without advancing produces no new fires.
        out2 = await engine.tick()
        assert out2.fires_enqueued == 0

    asyncio.run(scenario())


def test_engine_returns_non_leader_when_locked() -> None:
    fc = FakeClock()
    sch = InMemoryScheduleStore()
    jobs = InMemoryJobStore()
    leader_a = InMemoryLeaderElection(key="k-dup", owner_id="w1")
    leader_b = InMemoryLeaderElection(key="k-dup", owner_id="w2")

    async def scenario() -> None:
        assert await leader_a.acquire()
        engine = SchedulerEngine(
            tenant_id="t1",
            schedule_store=sch,
            job_store=jobs,
            leader=leader_b,
            clock=fc,
        )
        out = await engine.tick()
        assert not out.leader

    asyncio.run(scenario())


def test_engine_catchup_truncates() -> None:
    fc = FakeClock(datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc))
    sch = InMemoryScheduleStore()
    jobs = InMemoryJobStore()
    leader = InMemoryLeaderElection(key="catchup", owner_id="w1")
    engine = SchedulerEngine(
        tenant_id="t1",
        schedule_store=sch,
        job_store=jobs,
        leader=leader,
        clock=fc,
        config=SchedulerEngineConfig(catch_up_h=6),
    )

    async def scenario() -> None:
        # last_eval is 24h old — truncated to 6h.
        now = fc.now()
        last_eval = now - timedelta(hours=24)
        await sch.create(_mk_schedule("s1", "0 * * * *", now, last_eval=last_eval))
        out = await engine.tick()
        # Fires for the last 6h on the hour = at most 6, plus dropped_catch_up=1.
        assert out.fires_enqueued <= 6
        assert out.dropped_catch_up == 1

    asyncio.run(scenario())


def test_build_evaluator_dispatches_on_kind() -> None:
    ev = build_evaluator(IntervalSchedule(period_s=60))
    assert isinstance(ev, IntervalEvaluator)
    ev2 = build_evaluator(OneShotSchedule(at=datetime(2030, 1, 1, tzinfo=timezone.utc)))
    assert isinstance(ev2, OneShotEvaluator)
