"""Non-regression tests for the audit round of M10 fixes.

Each test is tagged with the fix number from the audit report so the
origin of the test is traceable.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.hashing import is_using_dev_key, set_key, reset_key
from efe_scheduling.jobs import (
    DeadLetterManager,
    JobExecutor,
    JobRegistry,
    JobResult,
    Worker,
    WorkerConfig,
)
from efe_scheduling.learning import (
    ContinuousLearningPipeline,
    LearningConfig,
)
from efe_scheduling.randomness import FakeRandomness
from efe_scheduling.scheduler import parse_cron
from efe_scheduling.scheduler.engine import SchedulerEngine, SchedulerEngineConfig
from efe_scheduling.scheduler.leader import InMemoryLeaderElection
from efe_scheduling.storage import (
    InMemoryDeadLetterStore,
    InMemoryJobStore,
    InMemoryRunStore,
    InMemoryScheduleStore,
)
from efe_scheduling.telemetry.metrics import reset_registry, get_registry
from efe_scheduling.triggers import (
    DebouncerState,
    DriftEvent,
    DriftSeverity,
    DriftThresholdTrigger,
    DriftTriggerSpec,
    Fire,
    Hold,
    MetricSample,
    MetricTrigger,
    MetricTriggerSpec,
    TriggerContext,
)
from efe_scheduling.types import (
    CronSchedule,
    JobKind,
    JobPriority,
    JobSpec,
    JobState,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    Schedule,
    ScheduleState,
    build_fire_key,
    new_uuid,
)


# ---------------------------------------------------------------------------
# Fix 1 — Cron DOW range 0-7 / 1-7
# ---------------------------------------------------------------------------


def test_fix1_cron_dow_range_0_to_7_covers_whole_week() -> None:
    e = parse_cron("0 9 * * 0-7")
    assert e.dow == frozenset({0, 1, 2, 3, 4, 5, 6})


def test_fix1_cron_dow_range_1_to_7_covers_mon_through_sun() -> None:
    e = parse_cron("0 9 * * 1-7")
    # Mon..Sat = 1..6, plus Sun = 0
    assert e.dow == frozenset({1, 2, 3, 4, 5, 6, 0})


def test_fix1_cron_dow_5_to_7_covers_fri_sat_sun() -> None:
    e = parse_cron("0 9 * * 5-7")
    assert e.dow == frozenset({5, 6, 0})


def test_fix1_cron_dow_single_7_is_sunday() -> None:
    e = parse_cron("0 9 * * 7")
    assert e.dow == frozenset({0})


def test_fix1_cron_dow_standard_range_still_works() -> None:
    e = parse_cron("0 9 * * 1-5")
    assert e.dow == frozenset({1, 2, 3, 4, 5})


# ---------------------------------------------------------------------------
# Fix 2 — Trigger debouncer_state returned on Hold/Reset
# ---------------------------------------------------------------------------


def _drift_event(score: float = 0.5, severity: DriftSeverity = DriftSeverity.major) -> DriftEvent:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    return DriftEvent(
        result_id="r",
        tenant_id="t1",
        task_id=None,
        feature_name="latency_ms",
        method="psi",
        score=score,
        severity=severity,
        observed_at=now,
        window_start=now - timedelta(hours=1),
        window_end=now,
    )


def _ctx(signal, state=None):
    return TriggerContext(
        tenant_id="t1",
        now=datetime(2026, 4, 21, tzinfo=timezone.utc),
        clock=FakeClock(),
        random_source=FakeRandomness(),
        signal=signal,
        debouncer_state=state or {},
    )


def test_fix2_drift_hold_returns_new_debouncer_state() -> None:
    trg = DriftThresholdTrigger(
        trigger_id="d",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        spec=DriftTriggerSpec(
            threshold=0.1,
            min_severity=DriftSeverity.major,
            consecutive_breaches=3,
        ),
    )
    ev = _drift_event(score=0.5, severity=DriftSeverity.major)
    d = trg.should_fire(_ctx(ev, state={}))
    assert isinstance(d, Hold)
    # The Hold MUST carry the updated state, not an empty dict.
    assert d.debouncer_state.get("count") == 1
    assert d.debouncer_state.get("phase") == "counting"


def test_fix2_drift_state_propagation_across_calls() -> None:
    """Simulate what a caller would do : persist state between calls."""
    trg = DriftThresholdTrigger(
        trigger_id="d",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        spec=DriftTriggerSpec(
            threshold=0.1,
            min_severity=DriftSeverity.major,
            consecutive_breaches=3,
        ),
    )
    state: dict = {}
    for _ in range(2):
        d = trg.should_fire(_ctx(_drift_event(), state=state))
        state = d.debouncer_state
    # Third breach → Fire.
    d = trg.should_fire(_ctx(_drift_event(), state=state))
    assert isinstance(d, Fire)


def test_fix2_metric_hold_returns_state_even_on_tenant_mismatch() -> None:
    trg = MetricTrigger(
        trigger_id="m",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.rollback,
        spec=MetricTriggerSpec(
            metric="latency_p99_ms",
            threshold=100,
            op=">=",
            consecutive_breaches=1,
        ),
    )
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    sample = MetricSample(
        tenant_id="OTHER",
        task_id=None,
        metric="latency_p99_ms",
        value=1000,
        observed_at=now,
    )
    d = trg.should_fire(_ctx(sample, state={"count": 5, "phase": "counting"}))
    assert isinstance(d, Hold)
    # Existing state is preserved (not nuked).
    assert d.debouncer_state.get("count") == 5


# ---------------------------------------------------------------------------
# Fix 3 — Pipeline fire_key is deterministic (invariant I1)
# ---------------------------------------------------------------------------


def test_fix3_pipeline_deduplicates_replayed_drift() -> None:
    """Same drift event replayed twice → identical fire_key for each kind.

    We verify determinism at the JobSpec.fire_key level : two independent
    pipelines (isolated in-memory state) produce the same keys for the
    same input event. The JobStore UNIQUE constraint then suppresses the
    duplicate at enqueue time.
    """
    fc = FakeClock(datetime(2026, 4, 21, tzinfo=timezone.utc))
    pipe_a = ContinuousLearningPipeline(
        cfg=LearningConfig(retrain_min_critical_features=1),
        clock=fc,
    )
    pipe_b = ContinuousLearningPipeline(
        cfg=LearningConfig(retrain_min_critical_features=1),
        clock=fc,
    )
    ev = DriftEvent(
        result_id="r-123",
        tenant_id="t1",
        task_id=None,
        feature_name="latency_ms",
        method="psi",
        score=0.7,
        severity=DriftSeverity.critical,
        observed_at=fc.now(),
        window_start=fc.now() - timedelta(hours=1),
        window_end=fc.now(),
    )
    jobs_a = asyncio.run(pipe_a.on_drift(ev))
    jobs_b = asyncio.run(pipe_b.on_drift(ev))
    keys_a = {j.kind.value: j.fire_key for j in jobs_a}
    keys_b = {j.kind.value: j.fire_key for j in jobs_b}
    # Both pipelines emit the same kinds for the same event.
    assert set(keys_a) == set(keys_b) != set()
    # And the fire_keys match per kind.
    for kind in keys_a:
        assert keys_a[kind] == keys_b[kind], f"fire_key drift for kind={kind}"


def test_fix3_pipeline_fire_key_independent_of_job_id() -> None:
    """Same pipeline → same event → same fire_key, even across calls."""
    fc = FakeClock(datetime(2026, 4, 21, tzinfo=timezone.utc))
    pipe = ContinuousLearningPipeline(
        cfg=LearningConfig(retrain_min_critical_features=1),
        clock=fc,
    )
    ev = DriftEvent(
        result_id="r-xyz",
        tenant_id="t1",
        task_id=None,
        feature_name="f",
        method="psi",
        score=0.9,
        severity=DriftSeverity.critical,
        observed_at=fc.now(),
        window_start=fc.now() - timedelta(hours=1),
        window_end=fc.now(),
    )
    jobs_1 = asyncio.run(pipe.on_drift(ev))
    jobs_2 = asyncio.run(pipe.on_drift(ev))
    # retrain_decision is emitted in both (no suppression on drift → retrain path).
    rd1 = next(j for j in jobs_1 if j.kind.value == "retrain_decision")
    rd2 = next(j for j in jobs_2 if j.kind.value == "retrain_decision")
    assert rd1.fire_key == rd2.fire_key
    # But the job_ids DIFFER (each JobSpec is a fresh aggregate).
    assert rd1.job_id != rd2.job_id


# ---------------------------------------------------------------------------
# Fix 4 — JobStore Protocol declares attempts_left
# ---------------------------------------------------------------------------


def test_fix4_jobstore_protocol_declares_attempts_left() -> None:
    from efe_scheduling.storage.protocols import JobStore

    # Name exists in the Protocol namespace.
    assert hasattr(JobStore, "attempts_left")


def test_fix4_inmemory_jobstore_satisfies_protocol() -> None:
    js = InMemoryJobStore()
    assert hasattr(js, "attempts_left")


# ---------------------------------------------------------------------------
# Fix 5 — Engine uses renew() when already leader
# ---------------------------------------------------------------------------


def test_fix5_engine_renews_when_already_leader() -> None:
    """A second tick should not produce a new acquire — only a renew."""
    fc = FakeClock(datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc))
    sch = InMemoryScheduleStore()
    jobs = InMemoryJobStore()

    renew_calls: list[int] = []
    acquire_calls: list[int] = []

    class _InstrumentedLeader(InMemoryLeaderElection):
        async def acquire(self, *, ttl_s: int = 30) -> bool:  # type: ignore[override]
            acquire_calls.append(ttl_s)
            return await super().acquire(ttl_s=ttl_s)

        async def renew(self, *, ttl_s: int = 30) -> bool:  # type: ignore[override]
            renew_calls.append(ttl_s)
            return await super().renew(ttl_s=ttl_s)

    leader = _InstrumentedLeader(key="fix5", owner_id="w1")
    engine = SchedulerEngine(
        tenant_id="t1",
        schedule_store=sch,
        job_store=jobs,
        leader=leader,
        clock=fc,
    )

    async def scenario() -> None:
        await engine.tick()
        await engine.tick()
        await engine.tick()

    asyncio.run(scenario())
    # First tick acquires ; next two renew.
    assert len(acquire_calls) == 1
    assert len(renew_calls) == 2


# ---------------------------------------------------------------------------
# Fix 6 — Engine emits Prometheus counters
# ---------------------------------------------------------------------------


def test_fix6_engine_increments_fire_total() -> None:
    reset_registry()
    reg = get_registry()
    fire = reg.get("efe_sched_fire_total")
    # StubMetric exposes .samples
    baseline_samples = len(fire.samples) if hasattr(fire, "samples") else 0

    fc = FakeClock(datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc))
    sch = InMemoryScheduleStore()
    jobs = InMemoryJobStore()
    leader = InMemoryLeaderElection(key="fix6", owner_id="w1")
    engine = SchedulerEngine(
        tenant_id="t1",
        schedule_store=sch,
        job_store=jobs,
        leader=leader,
        clock=fc,
        config=SchedulerEngineConfig(catch_up_h=24),
    )

    async def scenario() -> None:
        now = fc.now()
        spec = CronSchedule(expression="*/1 * * * *")
        await sch.create(
            Schedule(
                schedule_id="s1",
                tenant_id="t1",
                task_id=None,
                job_kind=JobKind.noop,
                spec=spec,
                payload={},
                enabled=True,
                state=ScheduleState.enabled,
                state_version=1,
                last_evaluated_at=now,
                created_at=now,
                updated_at=now,
            )
        )
        fc.advance(180)
        await engine.tick()

    asyncio.run(scenario())
    if hasattr(fire, "samples"):
        assert len(fire.samples) > baseline_samples


# ---------------------------------------------------------------------------
# Fix 7 — Wall-clock budget respected
# ---------------------------------------------------------------------------


def test_fix7_wall_clock_budget_forces_failed_permanent() -> None:
    async def scenario() -> None:
        reg = JobRegistry()

        async def handler(spec, deps):
            import asyncio as _a
            return JobResult.failure(_a.TimeoutError("transient"), code="timeout")

        reg.register(JobKind.noop, handler)

        js = InMemoryJobStore()
        rs = InMemoryRunStore()
        dls = InMemoryDeadLetterStore()

        # Job created 10 days ago but has plenty of attempts left.
        past = datetime(2026, 4, 10, tzinfo=timezone.utc)
        job_id = new_uuid()
        spec = JobSpec(
            job_id=job_id,
            fire_key=build_fire_key(job_id, past),
            tenant_id="t1",
            task_id=None,
            kind=JobKind.noop,
            payload={},
            priority=JobPriority.normal,
            priority_seq=job_id,
            run_at=past,
            deadline=None,
            max_attempts=5,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100, jitter_ratio=0.0
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=past,
        ).sealed()
        await js.enqueue(spec)

        executor = JobExecutor(reg, deps={})
        dl = DeadLetterManager(job_store=js, dead_letter_store=dls)
        # Wall-clock budget is 1 day ; the job is 10 days old → budget
        # exhausted on the very first attempt.
        worker = Worker(
            worker_id="w1",
            tenant_id="t1",
            job_store=js,
            run_store=rs,
            executor=executor,
            dead_letter=dl,
            clock=FakeClock(datetime(2026, 4, 20, tzinfo=timezone.utc)),
            config=WorkerConfig(lease_ttl_s=30, retry_wall_budget_s=86_400),
        )
        assert await worker.run_once()
        # Failed permanent + moved to dead letter.
        dl_ids = await dls.list()
        assert spec.job_id in dl_ids

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Fix 8 — HMAC dev-default warning path
# ---------------------------------------------------------------------------


def test_fix8_is_using_dev_key_flag() -> None:
    reset_key()
    # Depending on the env var, we may or may not be on the dev key.
    # The invariant is: set_key flips the flag off.
    set_key(b"x" * 32)
    assert not is_using_dev_key()


# ---------------------------------------------------------------------------
# Fix 9 — Executor returns well-formed result on expired deadline
# ---------------------------------------------------------------------------


def test_fix9_executor_expired_deadline_has_timestamps() -> None:
    async def scenario() -> None:
        reg = JobRegistry()

        async def handler(spec, deps):
            return JobResult.success({})

        reg.register(JobKind.noop, handler)

        past = datetime(2026, 4, 1, tzinfo=timezone.utc)
        job_id = new_uuid()
        spec = JobSpec(
            job_id=job_id,
            fire_key=build_fire_key(job_id, past),
            tenant_id="t1",
            task_id=None,
            kind=JobKind.noop,
            payload={},
            priority=JobPriority.normal,
            priority_seq=job_id,
            run_at=past,
            deadline=past + timedelta(seconds=1),  # way in the past
            max_attempts=1,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100, jitter_ratio=0.0
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=past,
        ).sealed()
        executor = JobExecutor(reg, deps={})
        result = await executor.execute(spec)
        assert not result.ok
        assert result.error_code == "deadline_expired"
        assert result.started_at is not None
        assert result.finished_at is not None
        assert result.timed_out is True

    asyncio.run(scenario())
