"""Value objects and dataclasses for M10.

All types are **immutable** (``frozen=True, slots=True``) so they can be
passed across threads and reused as dict keys when appropriate.

Discriminated unions (sum types) are implemented as tagged dataclasses :
the ``kind`` field carries the discriminant and each variant stores only
the fields it needs.
"""

from __future__ import annotations

import enum
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Literal

from efe_scheduling.hashing import (
    DOMAIN_TAG_JOB_SEAL,
    DOMAIN_TAG_SCHED_FIRE,
    canonical_json,
    hmac_seal,
)

__all__ = [
    "CompositeSchedule",
    "CronSchedule",
    "IntervalSchedule",
    "JobKind",
    "JobPriority",
    "JobRun",
    "JobRunStatus",
    "JobSpec",
    "JobState",
    "OneShotSchedule",
    "Origin",
    "RRuleSchedule",
    "RetryPolicyKind",
    "RetryPolicySpec",
    "Schedule",
    "ScheduleKind",
    "ScheduleSpec",
    "ScheduleState",
    "build_fire_key",
    "new_uuid",
    "utcnow",
]


# ---------------------------------------------------------------------------
# Enumerations
# ---------------------------------------------------------------------------


class ScheduleKind(str, enum.Enum):
    cron = "cron"
    interval = "interval"
    rrule = "rrule"
    one_shot = "one_shot"
    composite = "composite"


class ScheduleState(str, enum.Enum):
    disabled = "disabled"
    enabled = "enabled"
    completed = "completed"
    errored = "errored"


class JobKind(str, enum.Enum):
    """Stable registered job kinds."""

    baseline_refresh = "baseline_refresh"
    retrain_decision = "retrain_decision"
    data_curation = "data_curation"
    training_job = "training_job"
    validation_job = "validation_job"
    canary_evaluate = "canary_evaluate"
    promotion_decision = "promotion_decision"
    rollback = "rollback"
    noop = "noop"  # test harness


class JobPriority(int, enum.Enum):
    lowest = 0
    low = 10
    normal = 20
    high = 30
    critical = 40


class JobState(str, enum.Enum):
    queued = "queued"
    running = "running"
    succeeded = "succeeded"
    failed_permanent = "failed_permanent"
    cancelled = "cancelled"
    dead_letter = "dead_letter"


class JobRunStatus(str, enum.Enum):
    running = "running"
    succeeded = "succeeded"
    failed = "failed"
    cancelled = "cancelled"
    timeout = "timeout"


class RetryPolicyKind(str, enum.Enum):
    fixed = "fixed"
    linear = "linear"
    exponential = "exponential"
    decorrelated_jitter = "decorrelated_jitter"


class Origin(str, enum.Enum):
    schedule = "schedule"
    trigger = "trigger"
    manual = "manual"
    chained = "chained"


_TERMINAL_JOB_STATES: frozenset[JobState] = frozenset(
    {
        JobState.succeeded,
        JobState.failed_permanent,
        JobState.cancelled,
        JobState.dead_letter,
    }
)


# ---------------------------------------------------------------------------
# Utility helpers
# ---------------------------------------------------------------------------


def new_uuid() -> str:
    """Return a UUIDv4-looking string. Replaced in tests by a sequential gen."""
    return str(uuid.uuid4())


def utcnow() -> datetime:  # pragma: no cover - trivial
    return datetime.now(tz=timezone.utc)


def build_fire_key(schedule_id: str, fire_time: datetime) -> bytes:
    """Deterministic idempotency key for ``(schedule_id, fire_time)``.

    The key is ``HMAC(DOMAIN_TAG_SCHED_FIRE, {schedule_id, iso8601(fire_time)})``
    and is used as the ``UNIQUE`` column on the ``jobs`` table.
    """
    if fire_time.tzinfo is None:
        raise ValueError("fire_time must be tz-aware")
    payload = {
        "schedule_id": schedule_id,
        "fire_time": fire_time.astimezone(timezone.utc).isoformat(),
    }
    return hmac_seal(DOMAIN_TAG_SCHED_FIRE, payload)


# ---------------------------------------------------------------------------
# RetryPolicy
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class RetryPolicySpec:
    kind: RetryPolicyKind
    base_ms: int = 500
    max_ms: int = 60_000
    jitter_ratio: float = 0.2

    def __post_init__(self) -> None:
        if self.base_ms <= 0 or self.max_ms <= 0:
            raise ValueError("retry base/max ms must be positive")
        if self.max_ms < self.base_ms:
            raise ValueError("retry max_ms must be >= base_ms")
        if not 0.0 <= self.jitter_ratio <= 1.0:
            raise ValueError("jitter_ratio must be in [0, 1]")


# ---------------------------------------------------------------------------
# Schedule variants (sum type)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class CronSchedule:
    kind: Literal[ScheduleKind.cron] = field(default=ScheduleKind.cron)
    expression: str = ""
    timezone_name: str = "UTC"
    jitter_s: int = 0

    def __post_init__(self) -> None:
        if not self.expression.strip():
            raise ValueError("cron expression must not be empty")
        if self.jitter_s < 0:
            raise ValueError("jitter_s must be >= 0")


@dataclass(frozen=True, slots=True)
class IntervalSchedule:
    kind: Literal[ScheduleKind.interval] = field(default=ScheduleKind.interval)
    period_s: int = 60
    phase_s: int = 0
    jitter_s: int = 0

    def __post_init__(self) -> None:
        if self.period_s < 60:
            raise ValueError("IntervalSchedule.period_s must be >= 60")
        if self.phase_s < 0:
            raise ValueError("phase_s must be >= 0")
        if self.jitter_s < 0:
            raise ValueError("jitter_s must be >= 0")


@dataclass(frozen=True, slots=True)
class RRuleSchedule:
    kind: Literal[ScheduleKind.rrule] = field(default=ScheduleKind.rrule)
    rrule_text: str = ""
    timezone_name: str = "UTC"
    dtstart: datetime | None = None

    def __post_init__(self) -> None:
        if not self.rrule_text.strip():
            raise ValueError("rrule_text must not be empty")
        if self.dtstart is not None and self.dtstart.tzinfo is None:
            raise ValueError("dtstart must be tz-aware")


@dataclass(frozen=True, slots=True)
class OneShotSchedule:
    kind: Literal[ScheduleKind.one_shot] = field(default=ScheduleKind.one_shot)
    at: datetime = field(default_factory=utcnow)

    def __post_init__(self) -> None:
        if self.at.tzinfo is None:
            raise ValueError("OneShotSchedule.at must be tz-aware")


@dataclass(frozen=True, slots=True)
class CompositeSchedule:
    kind: Literal[ScheduleKind.composite] = field(default=ScheduleKind.composite)
    op: Literal["union", "exclude"] = "union"
    left: ScheduleSpec | None = None
    right: ScheduleSpec | None = None

    def __post_init__(self) -> None:
        if self.left is None or self.right is None:
            raise ValueError("CompositeSchedule needs both left and right")
        if self.op not in ("union", "exclude"):
            raise ValueError("CompositeSchedule.op must be 'union' or 'exclude'")


# Union alias for the sum type.
ScheduleSpec = (
    CronSchedule | IntervalSchedule | RRuleSchedule | OneShotSchedule | CompositeSchedule
)


# ---------------------------------------------------------------------------
# Schedule aggregate
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Schedule:
    """Persisted schedule with its state and metadata."""

    schedule_id: str
    tenant_id: str
    task_id: str | None
    job_kind: JobKind
    spec: ScheduleSpec
    payload: dict[str, Any]
    enabled: bool
    state: ScheduleState
    state_version: int
    last_evaluated_at: datetime | None
    created_at: datetime
    updated_at: datetime

    def __post_init__(self) -> None:
        if not self.tenant_id:
            raise ValueError("Schedule.tenant_id is required")
        if self.state_version < 1:
            raise ValueError("state_version must be >= 1")
        if self.created_at.tzinfo is None or self.updated_at.tzinfo is None:
            raise ValueError("Schedule timestamps must be tz-aware")

    def with_state(self, state: ScheduleState, *, now: datetime) -> Schedule:
        return Schedule(
            schedule_id=self.schedule_id,
            tenant_id=self.tenant_id,
            task_id=self.task_id,
            job_kind=self.job_kind,
            spec=self.spec,
            payload=self.payload,
            enabled=(state == ScheduleState.enabled),
            state=state,
            state_version=self.state_version + 1,
            last_evaluated_at=self.last_evaluated_at,
            created_at=self.created_at,
            updated_at=now,
        )

    def with_evaluated(self, until: datetime) -> Schedule:
        return Schedule(
            schedule_id=self.schedule_id,
            tenant_id=self.tenant_id,
            task_id=self.task_id,
            job_kind=self.job_kind,
            spec=self.spec,
            payload=self.payload,
            enabled=self.enabled,
            state=self.state,
            state_version=self.state_version + 1,
            last_evaluated_at=until,
            created_at=self.created_at,
            updated_at=until,
        )


# ---------------------------------------------------------------------------
# JobSpec & JobRun
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class JobSpec:
    """Immutable specification of a job to execute.

    The ``hmac_seal`` is computed at creation time against the canonical
    JSON of *all other* fields. Any post-creation tampering in storage
    is detected at load time by :func:`verify_seal`.
    """

    job_id: str
    fire_key: bytes
    tenant_id: str
    task_id: str | None
    kind: JobKind
    payload: dict[str, Any]
    priority: JobPriority
    priority_seq: str  # tie-breaker, typically the job_id
    run_at: datetime
    deadline: datetime | None
    max_attempts: int
    retry_policy: RetryPolicySpec
    origin: Origin
    origin_ref: str
    created_at: datetime
    hmac_seal: bytes = b""

    def __post_init__(self) -> None:
        if not self.tenant_id:
            raise ValueError("JobSpec.tenant_id is required")
        if self.run_at.tzinfo is None:
            raise ValueError("run_at must be tz-aware")
        if self.deadline is not None and self.deadline.tzinfo is None:
            raise ValueError("deadline must be tz-aware")
        if self.created_at.tzinfo is None:
            raise ValueError("created_at must be tz-aware")
        if not 1 <= self.max_attempts <= 10:
            raise ValueError("max_attempts must be in [1, 10]")
        if len(self.fire_key) != 32:
            raise ValueError("fire_key must be 32 bytes")

    def to_canonical_dict(self) -> dict[str, Any]:
        """Return the canonical mapping used to compute the HMAC seal."""
        return {
            "job_id": self.job_id,
            "fire_key": self.fire_key.hex(),
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "kind": self.kind.value,
            "payload": self.payload,
            "priority": int(self.priority),
            "priority_seq": self.priority_seq,
            "run_at": self.run_at.astimezone(timezone.utc).isoformat(),
            "deadline": (
                self.deadline.astimezone(timezone.utc).isoformat()
                if self.deadline is not None
                else None
            ),
            "max_attempts": self.max_attempts,
            "retry_policy": {
                "kind": self.retry_policy.kind.value,
                "base_ms": self.retry_policy.base_ms,
                "max_ms": self.retry_policy.max_ms,
                "jitter_ratio": self.retry_policy.jitter_ratio,
            },
            "origin": self.origin.value,
            "origin_ref": self.origin_ref,
            "created_at": self.created_at.astimezone(timezone.utc).isoformat(),
        }

    def sealed(self) -> JobSpec:
        """Return a copy with the HMAC seal filled in."""
        seal = hmac_seal(DOMAIN_TAG_JOB_SEAL, self.to_canonical_dict())
        return JobSpec(
            job_id=self.job_id,
            fire_key=self.fire_key,
            tenant_id=self.tenant_id,
            task_id=self.task_id,
            kind=self.kind,
            payload=self.payload,
            priority=self.priority,
            priority_seq=self.priority_seq,
            run_at=self.run_at,
            deadline=self.deadline,
            max_attempts=self.max_attempts,
            retry_policy=self.retry_policy,
            origin=self.origin,
            origin_ref=self.origin_ref,
            created_at=self.created_at,
            hmac_seal=seal,
        )

    def canonical_json(self) -> bytes:
        """Return the canonical JSON (without the seal) for wire / storage."""
        return canonical_json(self.to_canonical_dict())


@dataclass(frozen=True, slots=True)
class JobRun:
    """Single attempt to execute a job."""

    run_id: str
    job_id: str
    attempt_idx: int
    worker_id: str
    started_at: datetime
    finished_at: datetime | None
    status: JobRunStatus
    error_code: str | None = None
    error_message: str | None = None
    output_digest: bytes | None = None
    telemetry: dict[str, float] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.attempt_idx < 1:
            raise ValueError("attempt_idx must be >= 1")
        if self.started_at.tzinfo is None:
            raise ValueError("started_at must be tz-aware")
        if self.finished_at is not None and self.finished_at.tzinfo is None:
            raise ValueError("finished_at must be tz-aware")


def is_terminal(state: JobState) -> bool:
    return state in _TERMINAL_JOB_STATES
