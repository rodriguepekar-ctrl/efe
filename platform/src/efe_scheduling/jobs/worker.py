"""Worker — pulls jobs, executes them, publishes runs, applies retry.

The worker loop is the only place that writes transitions on the
``jobs`` table. It uses :class:`JobExecutor` for the actual handler call
and :class:`RetryEngine` for the retry policy.

High-level sequence per pulled job :

1. ``lease_next()`` atomically reserves the job (consumes one attempt).
2. :class:`JobExecutor` runs the handler with the deadline.
3. Append a :class:`JobRun` to the run store.
4. Decide the next state :

   * ``ok`` → ``succeeded``
   * ``err`` retryable + budget left → ``queued`` with ``run_at`` = now + backoff
   * ``err`` non-retryable or budget exhausted → ``failed_permanent``

5. If ``failed_permanent``, hand to :class:`DeadLetterManager`.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING

from efe_scheduling.clock import Clock, SystemClock
from efe_scheduling.errors import JobError
from efe_scheduling.jobs.dead_letter import DeadLetterManager
from efe_scheduling.jobs.executor import JobExecutor, JobResult
from efe_scheduling.jobs.retry import RetryEngine
from efe_scheduling.randomness import Randomness, SystemRandomness
from efe_scheduling.types import (
    JobRun,
    JobRunStatus,
    JobSpec,
    JobState,
    new_uuid,
)

if TYPE_CHECKING:  # pragma: no cover
    from efe_scheduling.storage.protocols import JobStore, RunStore

__all__ = ["Worker", "WorkerConfig"]

_LOG = logging.getLogger("efe_scheduling.worker")


@dataclass(frozen=True, slots=True)
class WorkerConfig:
    lease_ttl_s: int = 60
    poll_interval_s: float = 0.5
    default_handler_timeout_s: float = 120.0
    retry_wall_budget_s: int = 86_400
    """Hard wall-clock budget from ``JobSpec.created_at`` across all retries.
    After this, the job transitions to ``failed_permanent`` regardless of
    ``attempts_left``."""


@dataclass(slots=True)
class Worker:
    """Single worker — typically many run in a pool."""

    worker_id: str
    tenant_id: str | None
    job_store: "JobStore"
    run_store: "RunStore"
    executor: JobExecutor
    dead_letter: DeadLetterManager
    clock: Clock = field(default_factory=SystemClock)
    random_source: Randomness = field(default_factory=SystemRandomness)
    config: WorkerConfig = field(default_factory=WorkerConfig)
    _stopping: bool = False

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def run_once(self) -> bool:
        """Process at most one job. Returns True if a job was processed."""
        lease_ttl = timedelta(seconds=self.config.lease_ttl_s)
        spec = await self.job_store.lease_next(
            worker_id=self.worker_id, tenant_id=self.tenant_id, lease_ttl=lease_ttl
        )
        if spec is None:
            return False
        await self._process(spec)
        return True

    async def run_forever(self) -> None:
        """Loop until :meth:`stop` is called."""
        while not self._stopping:
            processed = await self.run_once()
            if not processed:
                await asyncio.sleep(self.config.poll_interval_s)

    def stop(self) -> None:
        self._stopping = True

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    async def _process(self, spec: JobSpec) -> None:
        attempts_left = await self.job_store.attempts_left(spec.job_id)  # type: ignore[attr-defined]
        attempt_idx = spec.max_attempts - attempts_left
        _LOG.info(
            "worker.start worker_id=%s job_id=%s kind=%s attempt=%d",
            self.worker_id,
            spec.job_id,
            spec.kind,
            attempt_idx,
        )

        result = await self.executor.execute(
            spec, default_timeout_s=self.config.default_handler_timeout_s
        )

        started_at = result.started_at or self.clock.now()
        finished_at = result.finished_at or self.clock.now()

        # Persist the run.
        run = JobRun(
            run_id=new_uuid(),
            job_id=spec.job_id,
            attempt_idx=max(1, attempt_idx),
            worker_id=self.worker_id,
            started_at=started_at,
            finished_at=finished_at,
            status=self._run_status(result),
            error_code=result.error_code,
            error_message=result.error_message,
            output_digest=None,
            telemetry=dict(result.telemetry) or {"duration_s": result.duration_s},
        )
        await self.run_store.append(run)

        # Decide the next state.
        version = await self.job_store.current_version(spec.job_id)  # type: ignore[attr-defined]
        if result.ok:
            await self.job_store.transition(
                job_id=spec.job_id,
                to_state=JobState.succeeded,
                expected_version=version,
                fields={"release_lease": True},
            )
            _LOG.info("worker.succeeded job_id=%s", spec.job_id)
            return

        retry_engine = RetryEngine(spec.retry_policy, random_source=self.random_source)
        wall_elapsed = (self.clock.now() - spec.created_at).total_seconds()
        wall_exhausted = wall_elapsed > self.config.retry_wall_budget_s
        decision = retry_engine.decide(
            error=result.error,
            attempt_idx=max(1, attempt_idx),
            attempts_left=attempts_left,
            wall_clock_budget_exhausted=wall_exhausted,
        )
        if decision.should_retry:
            next_run_at = self.clock.now() + timedelta(milliseconds=decision.delay_ms)
            await self.job_store.transition(
                job_id=spec.job_id,
                to_state=JobState.queued,
                expected_version=version,
                fields={"release_lease": True, "run_at": next_run_at},
            )
            _LOG.info(
                "worker.retry job_id=%s delay_ms=%d reason=%s",
                spec.job_id,
                decision.delay_ms,
                decision.reason,
            )
            return

        # Permanent failure.
        await self.job_store.transition(
            job_id=spec.job_id,
            to_state=JobState.failed_permanent,
            expected_version=version,
            fields={"release_lease": True},
        )
        await self.dead_letter.send(
            job_id=spec.job_id,
            last_run_id=run.run_id,
            reason=(
                result.error_code
                or (type(result.error).__name__ if result.error else "unknown")
            ),
            now=self.clock.now(),
        )
        _LOG.error(
            "worker.failed_permanent job_id=%s code=%s",
            spec.job_id,
            result.error_code,
        )

    @staticmethod
    def _run_status(result: JobResult) -> JobRunStatus:
        if result.timed_out:
            return JobRunStatus.timeout
        if result.ok:
            return JobRunStatus.succeeded
        if isinstance(result.error, JobError):
            return JobRunStatus.failed
        return JobRunStatus.failed
