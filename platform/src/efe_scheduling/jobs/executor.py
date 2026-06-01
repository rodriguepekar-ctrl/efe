"""JobExecutor — runs a handler and produces a JobResult.

The executor is the only component that actually calls the handler
function. It :

1. resolves the :class:`JobKind` in the registry,
2. validates the payload schema,
3. wraps the handler in ``asyncio.wait_for`` with the configured
   deadline,
4. collects any exception and classifies it into a :class:`JobResult`
   shape.

It does **not** persist anything — that is the worker's job.
"""

from __future__ import annotations

import asyncio
import logging
import time as _time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_scheduling.errors import HandlerCrashed
from efe_scheduling.jobs.registry import JobRegistry
from efe_scheduling.types import JobSpec

__all__ = ["JobExecutor", "JobResult"]

_LOG = logging.getLogger("efe_scheduling.executor")


@dataclass(frozen=True, slots=True)
class JobResult:
    """Outcome of a single handler invocation."""

    ok: bool
    output: dict[str, Any] = field(default_factory=dict)
    error: BaseException | None = None
    error_code: str | None = None
    error_message: str | None = None
    duration_s: float = 0.0
    started_at: datetime | None = None
    finished_at: datetime | None = None
    timed_out: bool = False
    telemetry: dict[str, float] = field(default_factory=dict)

    @classmethod
    def success(cls, output: dict[str, Any], *, telemetry: dict[str, float] | None = None) -> JobResult:
        return cls(ok=True, output=output, telemetry=dict(telemetry or {}))

    @classmethod
    def failure(
        cls,
        error: BaseException,
        *,
        code: str | None = None,
        telemetry: dict[str, float] | None = None,
    ) -> JobResult:
        return cls(
            ok=False,
            error=error,
            error_code=code or type(error).__name__,
            error_message=str(error),
            telemetry=dict(telemetry or {}),
        )


class JobExecutor:
    """Resolves a handler and runs it with a deadline."""

    __slots__ = ("_deps", "_registry")

    def __init__(self, registry: JobRegistry, deps: dict[str, Any]) -> None:
        self._registry = registry
        self._deps = dict(deps)

    async def execute(self, spec: JobSpec, *, default_timeout_s: float = 120.0) -> JobResult:
        defn = self._registry.get(spec.kind)
        # Validate payload at execution boundary (defensive: enqueue time
        # also validates).
        self._registry.validate_payload(spec.kind, spec.payload)

        deadline_s: float = default_timeout_s
        if spec.deadline is not None:
            now = datetime.now(tz=timezone.utc)
            remaining = (spec.deadline - now).total_seconds()
            if remaining <= 0:
                err = TimeoutError("deadline passed before execution started")
                # Populate timestamps so the caller can persist a well-formed
                # JobRun even on this short-circuit path.
                return JobResult(
                    ok=False,
                    error=err,
                    error_code="deadline_expired",
                    error_message=str(err),
                    duration_s=0.0,
                    started_at=now,
                    finished_at=now,
                    timed_out=True,
                )
            deadline_s = remaining

        started_at = datetime.now(tz=timezone.utc)
        t0 = _time.monotonic()
        try:
            output = await asyncio.wait_for(defn.handler(spec, self._deps), timeout=deadline_s)
            duration = _time.monotonic() - t0
            finished_at = datetime.now(tz=timezone.utc)
            if not isinstance(output, JobResult):
                # Accept a plain dict as "success" with that as output.
                if isinstance(output, dict):
                    return JobResult(
                        ok=True,
                        output=output,
                        duration_s=duration,
                        started_at=started_at,
                        finished_at=finished_at,
                    )
                err = HandlerCrashed(
                    f"handler for {spec.kind} returned unexpected type: {type(output).__name__}"
                )
                return JobResult.failure(err)
            return JobResult(
                ok=output.ok,
                output=output.output,
                error=output.error,
                error_code=output.error_code,
                error_message=output.error_message,
                duration_s=duration,
                started_at=started_at,
                finished_at=finished_at,
                timed_out=output.timed_out,
                telemetry=output.telemetry,
            )
        except asyncio.TimeoutError as exc:
            duration = _time.monotonic() - t0
            finished_at = datetime.now(tz=timezone.utc)
            _LOG.warning("executor.timeout job_id=%s kind=%s", spec.job_id, spec.kind)
            return JobResult(
                ok=False,
                error=exc,
                error_code="timeout",
                error_message=str(exc),
                duration_s=duration,
                started_at=started_at,
                finished_at=finished_at,
                timed_out=True,
            )
        except Exception as exc:  # noqa: BLE001 - executor captures everything
            duration = _time.monotonic() - t0
            finished_at = datetime.now(tz=timezone.utc)
            _LOG.exception(
                "executor.handler_crashed job_id=%s kind=%s", spec.job_id, spec.kind
            )
            return JobResult(
                ok=False,
                error=exc,
                error_code=type(exc).__name__,
                error_message=str(exc),
                duration_s=duration,
                started_at=started_at,
                finished_at=finished_at,
            )
