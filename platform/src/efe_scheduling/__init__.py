"""M10 — Scheduling & Continuous Learning.

This package is the platform's orchestration brain. It turns three heterogenous
inputs — wall-clock time, metric thresholds, and drift results from M09 — into
a single coherent stream of *jobs* that drive the continuous-learning loop:
baseline refresh, retraining, canary evaluation, promotion, and rollback.

See :doc:`/M10_SCHEDULING_ARCHITECTURE` for the full specification.
"""

from __future__ import annotations

from efe_scheduling.clock import Clock, FakeClock, SystemClock
from efe_scheduling.errors import (
    ConfigError,
    DuplicateFireKey,
    HandlerCrashed,
    IdempotencyReplay,
    InvalidCronExpression,
    InvalidStateTransition,
    JobBudgetExhausted,
    JobError,
    JobNotFound,
    JobSealInvalid,
    JobTerminal,
    LeaseLost,
    NotFound,
    PayloadSchemaViolation,
    PolicyNotFound,
    ScheduleError,
    ScheduleNotFound,
    SchedulerError,
    SchedulingError,
    ScopeMissing,
    SecurityError,
    StateVersionConflict,
    StorageError,
    TenantMismatch,
    TriggerError,
    TriggerNotFound,
    UnknownJobKind,
    ValidationError,
    WorkerError,
)
from efe_scheduling.hashing import (
    DOMAIN_TAG_DATASET_SEAL,
    DOMAIN_TAG_DATASET_SEED,
    DOMAIN_TAG_IDEMPOTENCY_TOKEN,
    DOMAIN_TAG_JOB_SEAL,
    DOMAIN_TAG_PROMOTION_SEAL,
    DOMAIN_TAG_SCHED_EVENT_V1,
    DOMAIN_TAG_SCHED_FIRE,
    canonical_json,
    hmac_seal,
    verify_seal,
)
from efe_scheduling.randomness import FakeRandomness, Randomness, SystemRandomness
from efe_scheduling.types import (
    JobKind,
    JobPriority,
    JobRun,
    JobRunStatus,
    JobSpec,
    JobState,
    Origin,
    RetryPolicyKind,
    ScheduleKind,
    ScheduleState,
)

__all__ = [
    "Clock",
    "ConfigError",
    "DOMAIN_TAG_DATASET_SEAL",
    "DOMAIN_TAG_DATASET_SEED",
    "DOMAIN_TAG_IDEMPOTENCY_TOKEN",
    "DOMAIN_TAG_JOB_SEAL",
    "DOMAIN_TAG_PROMOTION_SEAL",
    "DOMAIN_TAG_SCHED_EVENT_V1",
    "DOMAIN_TAG_SCHED_FIRE",
    "DuplicateFireKey",
    "FakeClock",
    "FakeRandomness",
    "HandlerCrashed",
    "IdempotencyReplay",
    "InvalidCronExpression",
    "InvalidStateTransition",
    "JobBudgetExhausted",
    "JobError",
    "JobKind",
    "JobNotFound",
    "JobPriority",
    "JobRun",
    "JobRunStatus",
    "JobSealInvalid",
    "JobSpec",
    "JobState",
    "JobTerminal",
    "LeaseLost",
    "NotFound",
    "Origin",
    "PayloadSchemaViolation",
    "PolicyNotFound",
    "Randomness",
    "RetryPolicyKind",
    "ScheduleError",
    "ScheduleKind",
    "ScheduleNotFound",
    "ScheduleState",
    "SchedulerError",
    "SchedulingError",
    "ScopeMissing",
    "SecurityError",
    "StateVersionConflict",
    "StorageError",
    "SystemClock",
    "SystemRandomness",
    "TenantMismatch",
    "TriggerError",
    "TriggerNotFound",
    "UnknownJobKind",
    "ValidationError",
    "WorkerError",
    "canonical_json",
    "hmac_seal",
    "verify_seal",
]

__version__ = "1.0.0"
