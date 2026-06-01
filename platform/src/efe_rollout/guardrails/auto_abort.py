"""Auto-abort executor — the end of the guardrail pipeline.

When a :class:`GuardrailWatcher` flags ``trips_abort=True`` the
experiment must move to ``ABORTED`` and, critically, M06 (the
:mod:`efe_registry` module) must perform the actual routing rollback
within the < 60 s SLO (invariant **I7**).

The executor is intentionally thin — it holds references to :

* A :class:`RolloutCallback` Protocol that the service layer (or tests)
  implements to publish NATS, persist the state transition, and signal
  M06.

* An :class:`RollbackClient` Protocol that abstracts the M06 rollback
  call so tests can replay it without spinning up the registry.

If either call fails, :class:`AutoAbortFailed` is raised so the caller
retries or pages. The watcher resets its counter on success and leaves
it untouched on failure (we want the next tick to retry the abort).
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Protocol

from efe_rollout.clock import Clock, SystemClock
from efe_rollout.errors import AutoAbortFailed
from efe_rollout.experiment import GuardrailSpec


@dataclass(frozen=True, slots=True)
class AbortContext:
    experiment_id: str
    tenant_id: str
    task_id: str
    layer: str
    guardrail: GuardrailSpec
    ratio: float | None
    breaches: int


class RolloutCallback(Protocol):
    async def transition_to_aborted(
        self, experiment_id: str, *, reason: str,
    ) -> None:  # pragma: no cover
        ...

    async def publish_aborted_event(
        self, ctx: AbortContext,
    ) -> None:  # pragma: no cover
        ...


class RollbackClient(Protocol):
    """Shim for :class:`efe_registry.RollbackExecutor.execute`."""

    async def execute(
        self,
        *,
        tenant_id: str,
        task_id: str,
        actor: str,
        reason: str,
    ) -> None:  # pragma: no cover
        ...


class AutoAbortExecutor:
    """Coordinates state + event + M06 rollback for one abort."""

    __slots__ = ("_callback", "_clock", "_deadline_s", "_lock", "_rollback")

    def __init__(
        self,
        *,
        callback: RolloutCallback,
        rollback: RollbackClient,
        deadline_s: float = 30.0,
        clock: Clock | None = None,
    ) -> None:
        self._callback = callback
        self._rollback = rollback
        self._clock = clock or SystemClock()
        self._deadline_s = float(deadline_s)
        self._lock = asyncio.Lock()

    async def abort(self, ctx: AbortContext) -> None:
        """Drive the full abort flow.

        Holds an :class:`asyncio.Lock` so two guardrails tripping at
        the same time can't clobber each other's state transitions.
        """
        async with self._lock:
            reason = (
                f"guardrail {ctx.guardrail.metric_name} breached "
                f"(ratio={ctx.ratio!r}, breaches={ctx.breaches})"
            )
            start = self._clock.now_monotonic_s()
            try:
                await self._callback.transition_to_aborted(
                    ctx.experiment_id, reason=reason,
                )
                await self._callback.publish_aborted_event(ctx)
                await self._rollback.execute(
                    tenant_id=ctx.tenant_id,
                    task_id=ctx.task_id,
                    actor="rollout-watcher",
                    reason=reason,
                )
            except Exception as exc:
                raise AutoAbortFailed(
                    f"auto-abort for {ctx.experiment_id!r} failed: {exc}",
                    detail={
                        "experiment_id": ctx.experiment_id,
                        "reason": reason,
                    },
                ) from exc
            elapsed = self._clock.now_monotonic_s() - start
            if elapsed > self._deadline_s:
                # Not an error but we log it via the callback — the
                # runbook has a 60 s SLO; we flag anything > 30 s.
                raise AutoAbortFailed(
                    f"auto-abort exceeded soft deadline "
                    f"({elapsed:.2f}s > {self._deadline_s}s)",
                    detail={"elapsed_s": elapsed},
                )


__all__ = [
    "AbortContext",
    "AutoAbortExecutor",
    "RollbackClient",
    "RolloutCallback",
]
