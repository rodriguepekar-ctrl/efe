"""Tests for the job system : registry, retry, executor, worker."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.errors import DuplicateFireKey, PayloadSchemaViolation, UnknownJobKind
from efe_scheduling.jobs import (
    DeadLetterManager,
    JobExecutor,
    JobRegistry,
    JobResult,
    Worker,
    WorkerConfig,
    backoff_delay_ms,
    is_retryable,
)
from efe_scheduling.jobs.retry import RetryEngine
from efe_scheduling.randomness import FakeRandomness
from efe_scheduling.storage import (
    InMemoryDeadLetterStore,
    InMemoryJobStore,
    InMemoryRunStore,
)
from efe_scheduling.types import (
    JobKind,
    JobPriority,
    JobSpec,
    JobState,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    build_fire_key,
    new_uuid,
)


# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------


def test_registry_unknown_kind_raises() -> None:
    reg = JobRegistry()
    with pytest.raises(UnknownJobKind):
        reg.get(JobKind.training_job)


def test_registry_register_and_validate_payload() -> None:
    reg = JobRegistry()

    async def handler(spec, deps):
        return JobResult.success({"ok": True})

    reg.register(JobKind.noop, handler, idempotent=True)
    kinds = reg.list_kinds()
    assert JobKind.noop in kinds
    out = reg.validate_payload(JobKind.noop, {"a": 1})
    assert out == {"a": 1}


def test_registry_rejects_non_dict_payload() -> None:
    reg = JobRegistry()

    async def handler(spec, deps):
        return JobResult.success({})

    reg.register(JobKind.noop, handler)
    with pytest.raises(PayloadSchemaViolation):
        reg.validate_payload(JobKind.noop, "not a dict")  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# Retry
# ---------------------------------------------------------------------------


def test_backoff_exponential_grows_then_caps() -> None:
    pol = RetryPolicySpec(
        kind=RetryPolicyKind.exponential, base_ms=100, max_ms=2_000, jitter_ratio=0.0
    )
    assert backoff_delay_ms(pol, 1) == 100
    assert backoff_delay_ms(pol, 2) == 200
    assert backoff_delay_ms(pol, 3) == 400
    assert backoff_delay_ms(pol, 6) == 2_000  # capped


def test_backoff_decorrelated_in_bounds() -> None:
    rnd = FakeRandomness(seed=7)
    pol = RetryPolicySpec(
        kind=RetryPolicyKind.decorrelated_jitter,
        base_ms=500,
        max_ms=5_000,
        jitter_ratio=0.0,
    )
    prev = None
    for i in range(1, 10):
        d = backoff_delay_ms(pol, i, prev_delay_ms=prev, random_source=rnd)
        assert 500 <= d <= 5_000
        prev = d


def test_is_retryable_timeout_true() -> None:
    assert is_retryable(asyncio.TimeoutError())


def test_retry_engine_decides() -> None:
    pol = RetryPolicySpec(
        kind=RetryPolicyKind.fixed, base_ms=500, max_ms=500, jitter_ratio=0.0
    )
    eng = RetryEngine(pol, random_source=FakeRandomness(seed=1))
    d = eng.decide(
        error=asyncio.TimeoutError(),
        attempt_idx=1,
        attempts_left=2,
        wall_clock_budget_exhausted=False,
    )
    assert d.should_retry and d.delay_ms == 500


def test_retry_engine_respects_budget() -> None:
    pol = RetryPolicySpec(kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100)
    eng = RetryEngine(pol)
    d = eng.decide(
        error=asyncio.TimeoutError(),
        attempt_idx=1,
        attempts_left=0,
        wall_clock_budget_exhausted=False,
    )
    assert not d.should_retry


# ---------------------------------------------------------------------------
# Executor + Worker integration
# ---------------------------------------------------------------------------


def _make_job(kind: JobKind = JobKind.noop, payload: dict | None = None) -> JobSpec:
    now = datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc)
    job_id = new_uuid()
    return JobSpec(
        job_id=job_id,
        fire_key=build_fire_key(job_id, now),
        tenant_id="t1",
        task_id=None,
        kind=kind,
        payload=payload or {},
        priority=JobPriority.normal,
        priority_seq=job_id,
        run_at=now - timedelta(seconds=1),
        deadline=None,
        max_attempts=3,
        retry_policy=RetryPolicySpec(
            kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100, jitter_ratio=0.0
        ),
        origin=Origin.schedule,
        origin_ref="sched-1",
        created_at=now,
    ).sealed()


def test_enqueue_deduplicates_by_fire_key() -> None:
    async def scenario() -> None:
        js = InMemoryJobStore()
        job = _make_job()
        await js.enqueue(job)
        # Second insert with the same fire_key raises.
        dup = JobSpec(
            job_id=new_uuid(),
            fire_key=job.fire_key,
            tenant_id=job.tenant_id,
            task_id=job.task_id,
            kind=job.kind,
            payload=job.payload,
            priority=job.priority,
            priority_seq=new_uuid(),
            run_at=job.run_at,
            deadline=job.deadline,
            max_attempts=job.max_attempts,
            retry_policy=job.retry_policy,
            origin=job.origin,
            origin_ref=job.origin_ref,
            created_at=job.created_at,
        ).sealed()
        with pytest.raises(DuplicateFireKey):
            await js.enqueue(dup)

    asyncio.run(scenario())


def test_worker_succeeds_noop_job() -> None:
    async def scenario() -> None:
        reg = JobRegistry()

        async def handler(spec, deps):
            return JobResult.success({"hello": "world"})

        reg.register(JobKind.noop, handler)

        js = InMemoryJobStore()
        rs = InMemoryRunStore()
        dls = InMemoryDeadLetterStore()
        job = _make_job()
        await js.enqueue(job)
        executor = JobExecutor(reg, deps={})
        dl = DeadLetterManager(job_store=js, dead_letter_store=dls)
        worker = Worker(
            worker_id="w1",
            tenant_id="t1",
            job_store=js,
            run_store=rs,
            executor=executor,
            dead_letter=dl,
            clock=FakeClock(),
            config=WorkerConfig(lease_ttl_s=30),
        )
        processed = await worker.run_once()
        assert processed
        state = await js.current_state(job.job_id)
        assert state == JobState.succeeded
        runs = await rs.list_for_job(job.job_id)
        assert len(runs) == 1 and runs[0].status.value == "succeeded"

    asyncio.run(scenario())


def test_worker_retries_on_retryable_error() -> None:
    async def scenario() -> None:
        reg = JobRegistry()
        attempts = {"n": 0}

        async def handler(spec, deps):
            attempts["n"] += 1
            err = asyncio.TimeoutError("boom")
            return JobResult.failure(err, code="timeout")

        reg.register(JobKind.noop, handler)
        js = InMemoryJobStore()
        rs = InMemoryRunStore()
        dls = InMemoryDeadLetterStore()
        job = _make_job()
        await js.enqueue(job)
        executor = JobExecutor(reg, deps={})
        dl = DeadLetterManager(job_store=js, dead_letter_store=dls)
        worker = Worker(
            worker_id="w1",
            tenant_id="t1",
            job_store=js,
            run_store=rs,
            executor=executor,
            dead_letter=dl,
            clock=FakeClock(),
            config=WorkerConfig(lease_ttl_s=30),
        )
        # Process once → ends up back in queued.
        assert await worker.run_once()
        state = await js.current_state(job.job_id)
        assert state == JobState.queued

    asyncio.run(scenario())


def test_worker_dead_letters_on_exhaustion() -> None:
    async def scenario() -> None:
        reg = JobRegistry()

        async def handler(spec, deps):
            err = ValueError("non retryable")
            return JobResult.failure(err, code="bad_payload")

        reg.register(JobKind.noop, handler)
        js = InMemoryJobStore()
        rs = InMemoryRunStore()
        dls = InMemoryDeadLetterStore()
        now = datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc)
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
            max_attempts=1,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100, jitter_ratio=0.0
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=now,
        ).sealed()
        await js.enqueue(spec)
        executor = JobExecutor(reg, deps={})
        dl = DeadLetterManager(job_store=js, dead_letter_store=dls)
        worker = Worker(
            worker_id="w1",
            tenant_id="t1",
            job_store=js,
            run_store=rs,
            executor=executor,
            dead_letter=dl,
            clock=FakeClock(),
            config=WorkerConfig(lease_ttl_s=30),
        )
        assert await worker.run_once()
        dl_ids = await dls.list()
        assert spec.job_id in dl_ids
        state = await js.current_state(spec.job_id)
        assert state == JobState.dead_letter

    asyncio.run(scenario())
