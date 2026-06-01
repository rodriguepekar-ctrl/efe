"""Deep edge-case coverage added after the audit pass.

These tests specifically exercise the boundary conditions that crashed
or mis-behaved during the surgical audit.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.hashing import (
    DOMAIN_TAG_SCHED_FIRE,
    canonical_json,
    hmac_seal,
)
from efe_scheduling.jobs import JobExecutor, JobRegistry, JobResult
from efe_scheduling.scheduler import parse_cron
from efe_scheduling.scheduler.cron import _parse_field  # type: ignore[attr-defined]
from efe_scheduling.scheduler.evaluators import (
    CronEvaluator,
    IntervalEvaluator,
    RRuleEvaluator,
)
from efe_scheduling.storage import InMemoryJobStore
from efe_scheduling.types import (
    CronSchedule,
    IntervalSchedule,
    JobKind,
    JobPriority,
    JobSpec,
    Origin,
    RRuleSchedule,
    RetryPolicyKind,
    RetryPolicySpec,
    build_fire_key,
    new_uuid,
)


# ---------------------------------------------------------------------------
# Cron DOW — end-to-end iter_fires
# ---------------------------------------------------------------------------


def test_cron_dow_1_7_iterates_over_full_week() -> None:
    ev = CronEvaluator(CronSchedule(expression="0 0 * * 1-7"))
    start = datetime(2026, 4, 20, 0, 0, tzinfo=timezone.utc)  # Monday
    end = datetime(2026, 4, 27, 0, 0, tzinfo=timezone.utc)  # Monday + 7d
    fires = list(ev.iter_fires(start, end))
    # Every day at 00:00 — 7 fires including the Mon at end.
    assert len(fires) == 7


def test_cron_mon_to_fri_only() -> None:
    ev = CronEvaluator(CronSchedule(expression="0 0 * * 1-5"))
    start = datetime(2026, 4, 19, 23, 59, tzinfo=timezone.utc)  # Sunday
    end = datetime(2026, 4, 26, 0, 1, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end))
    # Mon Tue Wed Thu Fri (5 days).
    assert len(fires) == 5


# ---------------------------------------------------------------------------
# Cron parsing : edge cases
# ---------------------------------------------------------------------------


def test_cron_parses_step_expression() -> None:
    e = parse_cron("*/15 * * * *")
    assert e.minute == frozenset({0, 15, 30, 45})


def test_cron_parses_lists_and_ranges_together() -> None:
    e = parse_cron("0,15,30-45 * * * *")
    expected = {0, 15, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45}
    assert e.minute == expected


def test_cron_rejects_invalid_range() -> None:
    with pytest.raises(Exception):
        parse_cron("0-99 * * * *")


def test_cron_rejects_empty_expression() -> None:
    with pytest.raises(Exception):
        parse_cron("   ")


def test_cron_dom_L_expands_last_days() -> None:
    e = parse_cron("0 0 L * *")
    assert e.dom == frozenset({28, 29, 30, 31})


# ---------------------------------------------------------------------------
# Canonical JSON : unicode, NaN
# ---------------------------------------------------------------------------


def test_canonical_json_rejects_nan() -> None:
    with pytest.raises(ValueError):
        canonical_json({"x": float("nan")})


def test_canonical_json_preserves_unicode() -> None:
    out = canonical_json({"name": "café"})
    # `ensure_ascii=False` means café stays as UTF-8 bytes.
    assert "café".encode("utf-8") in out


# ---------------------------------------------------------------------------
# HMAC idempotency
# ---------------------------------------------------------------------------


def test_hmac_seal_same_input_same_output() -> None:
    a = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"k": [1, 2, 3]})
    b = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"k": [1, 2, 3]})
    assert a == b


def test_hmac_seal_key_order_insensitive() -> None:
    a = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"a": 1, "b": 2})
    b = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"b": 2, "a": 1})
    assert a == b


# ---------------------------------------------------------------------------
# Interval evaluator : large windows
# ---------------------------------------------------------------------------


def test_interval_evaluator_daily_24h_window() -> None:
    ev = IntervalEvaluator(IntervalSchedule(period_s=3600))  # hourly
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = start + timedelta(days=1)
    fires = list(ev.iter_fires(start, end))
    assert len(fires) == 24


def test_interval_evaluator_phase_shift() -> None:
    ev = IntervalEvaluator(IntervalSchedule(period_s=3600, phase_s=600))
    start = datetime(2026, 4, 21, 0, 0, 0, tzinfo=timezone.utc)
    end = datetime(2026, 4, 21, 2, 0, 0, tzinfo=timezone.utc)
    fires = list(ev.iter_fires(start, end))
    # Phase shifts by 10 minutes → fires at 00:10, 01:10.
    assert fires[0].minute == 10


# ---------------------------------------------------------------------------
# RRule : UNTIL
# ---------------------------------------------------------------------------


def test_rrule_until_stops_iteration() -> None:
    start = datetime(2026, 1, 1, tzinfo=timezone.utc)
    ev = RRuleEvaluator(
        RRuleSchedule(
            rrule_text="FREQ=DAILY;UNTIL=20260110T000000Z",
            dtstart=start,
        )
    )
    fires = list(ev.iter_fires(start - timedelta(seconds=1), start + timedelta(days=365)))
    # 10 days (Jan 1 through Jan 10 inclusive).
    assert len(fires) == 10
    assert fires[-1] == datetime(2026, 1, 10, tzinfo=timezone.utc)


# ---------------------------------------------------------------------------
# Executor : handler returning None
# ---------------------------------------------------------------------------


def test_executor_handler_returning_none_is_a_crash() -> None:
    async def scenario() -> None:
        reg = JobRegistry()

        async def handler(spec, deps):
            return None  # wrong return type

        reg.register(JobKind.noop, handler)

        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        job_id = new_uuid()
        spec = JobSpec(
            job_id=job_id,
            fire_key=build_fire_key(job_id, now),
            tenant_id="t1",
            task_id=None,
            kind=JobKind.noop,
            payload={},
            priority=JobPriority.normal,
            priority_seq=job_id,
            run_at=now,
            deadline=None,
            max_attempts=1,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=now,
        ).sealed()
        executor = JobExecutor(reg, deps={})
        result = await executor.execute(spec)
        assert not result.ok
        assert result.error_code == "HandlerCrashed"

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# JobStore : lease skips running jobs
# ---------------------------------------------------------------------------


def test_jobstore_lease_skips_in_flight() -> None:
    async def scenario() -> None:
        js = InMemoryJobStore()
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        job_id = new_uuid()
        spec = JobSpec(
            job_id=job_id,
            fire_key=build_fire_key(job_id, now),
            tenant_id="t1",
            task_id=None,
            kind=JobKind.noop,
            payload={},
            priority=JobPriority.normal,
            priority_seq=job_id,
            run_at=now - timedelta(seconds=1),
            deadline=None,
            max_attempts=3,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=now,
        ).sealed()
        await js.enqueue(spec)
        first = await js.lease_next(worker_id="w1", lease_ttl=timedelta(seconds=60))
        assert first is not None
        # Second call must not pick the same job up.
        second = await js.lease_next(worker_id="w2", lease_ttl=timedelta(seconds=60))
        assert second is None

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Priority : higher wins
# ---------------------------------------------------------------------------


def test_jobstore_respects_priority() -> None:
    async def scenario() -> None:
        js = InMemoryJobStore()
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)

        def _mk(prio: JobPriority, suffix: str) -> JobSpec:
            jid = f"{suffix}-{new_uuid()}"
            return JobSpec(
                job_id=jid,
                fire_key=build_fire_key(jid, now),
                tenant_id="t1",
                task_id=None,
                kind=JobKind.noop,
                payload={},
                priority=prio,
                priority_seq=jid,
                run_at=now - timedelta(seconds=1),
                deadline=None,
                max_attempts=1,
                retry_policy=RetryPolicySpec(
                    kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100
                ),
                origin=Origin.schedule,
                origin_ref="s",
                created_at=now,
            ).sealed()

        low = _mk(JobPriority.low, "low")
        crit = _mk(JobPriority.critical, "crit")
        await js.enqueue(low)
        await js.enqueue(crit)
        leased = await js.lease_next(worker_id="w1", lease_ttl=timedelta(seconds=60))
        assert leased is not None
        assert leased.priority == JobPriority.critical

    asyncio.run(scenario())
