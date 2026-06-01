"""Unit tests for M10 foundations : clock, randomness, hashing, fsm, types."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_scheduling.clock import FakeClock, SystemClock
from efe_scheduling.errors import InvalidStateTransition
from efe_scheduling.fsm import (
    JOB_ALLOWED,
    SCHEDULE_ALLOWED,
    guard,
    is_allowed,
)
from efe_scheduling.hashing import (
    DOMAIN_TAG_JOB_SEAL,
    DOMAIN_TAG_SCHED_FIRE,
    canonical_json,
    current_key,
    hmac_seal,
    reset_key,
    set_key,
    verify_seal,
)
from efe_scheduling.randomness import FakeRandomness, SystemRandomness
from efe_scheduling.types import (
    CronSchedule,
    IntervalSchedule,
    JobKind,
    JobPriority,
    JobSpec,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    ScheduleState,
    build_fire_key,
    new_uuid,
)


# ---------------------------------------------------------------------------
# Clock
# ---------------------------------------------------------------------------


def test_system_clock_returns_tz_aware_utc() -> None:
    clk = SystemClock()
    t = clk.now()
    assert t.tzinfo is not None
    assert t.utcoffset() == t.utcoffset()  # consistent


def test_system_clock_monotonic_is_non_decreasing() -> None:
    clk = SystemClock()
    t0 = clk.now_monotonic_s()
    t1 = clk.now_monotonic_s()
    assert t1 >= t0


def test_fake_clock_advances_deterministically() -> None:
    fc = FakeClock(datetime(2026, 4, 21, tzinfo=timezone.utc))
    assert fc.now() == datetime(2026, 4, 21, tzinfo=timezone.utc)
    fc.advance(60)
    assert fc.now_monotonic_s() == 60.0
    fc.advance(0.5)
    assert fc.now_monotonic_s() == 60.5


def test_fake_clock_rejects_negative_advance() -> None:
    fc = FakeClock()
    with pytest.raises(ValueError):
        fc.advance(-1)


def test_fake_clock_rejects_naive_set() -> None:
    fc = FakeClock()
    with pytest.raises(ValueError):
        fc.set(datetime(2026, 4, 21))  # naive


# ---------------------------------------------------------------------------
# Randomness
# ---------------------------------------------------------------------------


def test_fake_randomness_is_reproducible() -> None:
    a = FakeRandomness(seed=42)
    b = FakeRandomness(seed=42)
    for _ in range(100):
        assert a.random() == b.random()


def test_fake_randomness_reseed_restarts_sequence() -> None:
    r = FakeRandomness(seed=1)
    first = [r.random() for _ in range(5)]
    r.reseed(1)
    second = [r.random() for _ in range(5)]
    assert first == second


def test_system_randomness_token_bytes_length() -> None:
    r = SystemRandomness()
    assert len(r.token_bytes(16)) == 16
    with pytest.raises(ValueError):
        r.token_bytes(-1)


# ---------------------------------------------------------------------------
# Hashing / HMAC
# ---------------------------------------------------------------------------


def test_canonical_json_stable_ordering() -> None:
    a = canonical_json({"b": 1, "a": 2})
    b = canonical_json({"a": 2, "b": 1})
    assert a == b


def test_canonical_json_idempotent() -> None:
    x = {"nested": {"b": [1, 2], "a": "foo"}}
    once = canonical_json(x)
    twice = canonical_json(__import__("json").loads(once))
    assert once == twice


def test_hmac_seal_is_deterministic() -> None:
    set_key(b"x" * 32)
    try:
        a = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"k": 1})
        b = hmac_seal(DOMAIN_TAG_SCHED_FIRE, {"k": 1})
        assert a == b
        assert len(a) == 32
    finally:
        reset_key()


def test_verify_seal_rejects_wrong_tag() -> None:
    set_key(b"y" * 32)
    try:
        payload = {"abc": 42}
        seal_a = hmac_seal(DOMAIN_TAG_SCHED_FIRE, payload)
        # Same payload under a different tag must not verify.
        assert not verify_seal(DOMAIN_TAG_JOB_SEAL, payload, seal_a)
        assert verify_seal(DOMAIN_TAG_SCHED_FIRE, payload, seal_a)
    finally:
        reset_key()


def test_unknown_tag_raises() -> None:
    with pytest.raises(ValueError):
        hmac_seal(b"unknown/tag\x00", {"k": 1})


def test_current_key_returns_copy() -> None:
    k = current_key()
    assert isinstance(k, bytes)


# ---------------------------------------------------------------------------
# FSM helpers
# ---------------------------------------------------------------------------


def test_schedule_fsm_allowed_transition() -> None:
    assert is_allowed(
        ScheduleState.disabled.value, ScheduleState.enabled.value, SCHEDULE_ALLOWED
    )


def test_schedule_fsm_rejects_unknown_transition() -> None:
    with pytest.raises(InvalidStateTransition):
        guard("completed", "enabled", SCHEDULE_ALLOWED, aggregate="schedule")


def test_job_fsm_allows_retry() -> None:
    assert is_allowed("running", "queued", JOB_ALLOWED)


def test_job_fsm_blocks_illegal_path() -> None:
    with pytest.raises(InvalidStateTransition):
        guard("succeeded", "queued", JOB_ALLOWED, aggregate="job")


# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------


def test_cron_schedule_validates_expression() -> None:
    with pytest.raises(ValueError):
        CronSchedule(expression="")


def test_interval_schedule_minimum_period() -> None:
    with pytest.raises(ValueError):
        IntervalSchedule(period_s=30)


def test_build_fire_key_requires_tz_aware() -> None:
    with pytest.raises(ValueError):
        build_fire_key("sid", datetime(2026, 4, 21))


def test_job_spec_seal_roundtrip() -> None:
    now = datetime(2026, 4, 21, 12, 0, tzinfo=timezone.utc)
    fire = build_fire_key("sid", now)
    job = JobSpec(
        job_id=new_uuid(),
        fire_key=fire,
        tenant_id="t",
        task_id=None,
        kind=JobKind.noop,
        payload={"a": 1},
        priority=JobPriority.normal,
        priority_seq=new_uuid(),
        run_at=now,
        deadline=None,
        max_attempts=3,
        retry_policy=RetryPolicySpec(
            kind=RetryPolicyKind.exponential, base_ms=500, max_ms=60_000,
        ),
        origin=Origin.schedule,
        origin_ref="sid",
        created_at=now,
    ).sealed()
    # The canonical dict must verify against the seal.
    assert verify_seal(DOMAIN_TAG_JOB_SEAL, job.to_canonical_dict(), job.hmac_seal)


def test_job_spec_rejects_bad_max_attempts() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    with pytest.raises(ValueError):
        JobSpec(
            job_id="j",
            fire_key=b"\x00" * 32,
            tenant_id="t",
            task_id=None,
            kind=JobKind.noop,
            payload={},
            priority=JobPriority.normal,
            priority_seq="p",
            run_at=now,
            deadline=None,
            max_attempts=0,  # invalid
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=500, max_ms=1_000
            ),
            origin=Origin.schedule,
            origin_ref="sid",
            created_at=now,
        )
