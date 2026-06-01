"""Jobs subsystem — registry, retry, executor, worker, dead-letter."""

from __future__ import annotations

from efe_scheduling.jobs.dead_letter import DeadLetterManager
from efe_scheduling.jobs.executor import JobExecutor, JobResult
from efe_scheduling.jobs.registry import (
    HandlerFn,
    JobDef,
    JobRegistry,
    default_registry,
    job_kind,
)
from efe_scheduling.jobs.retry import (
    RetryDecision,
    RetryEngine,
    backoff_delay_ms,
    is_retryable,
)
from efe_scheduling.jobs.worker import Worker, WorkerConfig

__all__ = [
    "DeadLetterManager",
    "HandlerFn",
    "JobDef",
    "JobExecutor",
    "JobRegistry",
    "JobResult",
    "RetryDecision",
    "RetryEngine",
    "Worker",
    "WorkerConfig",
    "backoff_delay_ms",
    "default_registry",
    "is_retryable",
    "job_kind",
]
