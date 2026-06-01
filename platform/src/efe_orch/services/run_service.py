"""The RunService — caller-facing run lifecycle.

This module implements every public verb that a user (or the
CLI / M02 UI) can ask the orchestrator to do on a run:

* :meth:`submit`       — DRAFT → QUEUED (+ pool pick + quota reserve)
* :meth:`dispatch`     — QUEUED → SCHEDULED (render + submit K8s Job)
* :meth:`cancel`       — any → CANCELLING / CANCELLED
* :meth:`get`          — read one run
* :meth:`list`         — paginated listing, tenant-scoped
* :meth:`retry`        — internal: RETRYING → QUEUED (called by reaper
  or worker-service when an attempt fails with a retryable class)

``submit`` and ``retry`` are the only two paths that can *create*
placement. Both go through :class:`SchedulerService` (pool pick +
quota reservation) and then through :class:`StateMachine.transition`
(atomic state write). If any step fails we roll back the quota
reservation so we don't "leak" slots.
"""

from __future__ import annotations

import random
from collections.abc import Sequence
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import Clock, SystemClock
from efe_orch.config import K8sSettings, RetrySettings
from efe_orch.db.models import FinetuneRun
from efe_orch.dispatcher import KubernetesClient, WorkerJobSpec, render_worker_job
from efe_orch.errors import (
    CannotCancel,
    RunNotFound,
    TenantMismatch,
    ValidationError,
)
from efe_orch.ids import new_run_id, new_worker_id
from efe_orch.observability.metrics import MetricsRegistry, noop_registry
from efe_orch.retry import ErrorClass, RetryPolicy, classify_error
from efe_orch.scheduler import RunShape, SchedulerService
from efe_orch.state_machine.states import (
    ACTIVE_STATES,
    RunState,
    is_terminal,
)
from efe_orch.state_machine.transitions import StateMachine

# ---------------------------------------------------------------------------
#  DTOs
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SubmitRunInput:
    tenant_id: str
    task_id: str
    submitted_by: str
    snapshot_id: str
    dataset_rows: int
    base_model: str = "sentence-transformers/all-MiniLM-L6-v2"
    recipe_version: str = "v1"
    priority: str = "standard"
    needs_gpu: bool = False
    requested_pool: str | None = None
    hyperparameters: dict[str, Any] = field(default_factory=dict)
    tags: dict[str, Any] = field(default_factory=dict)
    estimated_gpu_seconds: int = 0


@dataclass(frozen=True, slots=True)
class RunView:
    """Projection returned to API callers."""

    id: str
    tenant_id: str
    task_id: str
    submitted_by: str
    snapshot_id: str
    state: RunState
    state_version: int
    priority: str
    pool: str | None
    worker_id: str | None
    attempt: int
    retry_count: int
    last_error_code: str | None
    last_error_message: str | None
    created_at: datetime
    updated_at: datetime
    started_at: datetime | None
    finished_at: datetime | None
    result_artifact_url: str | None
    result_metrics: dict[str, Any]
    hyperparameters: dict[str, Any]
    tags: dict[str, Any]

    @classmethod
    def from_row(cls, r: FinetuneRun) -> RunView:
        return cls(
            id=r.id,
            tenant_id=r.tenant_id,
            task_id=r.task_id,
            submitted_by=r.submitted_by,
            snapshot_id=r.snapshot_id,
            state=RunState(r.state),
            state_version=r.state_version,
            priority=r.priority,
            pool=r.pool,
            worker_id=r.worker_id,
            attempt=r.attempt,
            retry_count=r.retry_count,
            last_error_code=r.last_error_code,
            last_error_message=r.last_error_message,
            created_at=r.created_at,
            updated_at=r.updated_at,
            started_at=r.started_at,
            finished_at=r.finished_at,
            result_artifact_url=r.result_artifact_url,
            result_metrics=dict(r.result_metrics or {}),
            hyperparameters=dict(r.hyperparameters or {}),
            tags=dict(r.tags or {}),
        )


@dataclass(frozen=True, slots=True)
class ListRunsFilter:
    tenant_id: str
    task_id: str | None = None
    state: RunState | None = None
    submitted_by: str | None = None
    page_size: int = 50
    page_token: str | None = None


# ---------------------------------------------------------------------------
#  RunService
# ---------------------------------------------------------------------------


class RunService:
    """Orchestrator's caller-facing API surface (non-worker)."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        scheduler: SchedulerService,
        state_machine: StateMachine,
        k8s_client: KubernetesClient,
        k8s_settings: K8sSettings,
        retry_settings: RetrySettings,
        clock: Clock | None = None,
        metrics: MetricsRegistry | None = None,
    ) -> None:
        self._session = session
        self._scheduler = scheduler
        self._sm = state_machine
        self._k8s = k8s_client
        self._k8s_settings = k8s_settings
        self._retry_policy = RetryPolicy(retry_settings)
        self._clock = clock or SystemClock()
        self._metrics: MetricsRegistry = metrics or noop_registry()

    # ------------------------------------------------------------------ submit

    async def submit(self, inp: SubmitRunInput) -> RunView:
        """Accept a new run, pick a pool, reserve quota, enqueue it.

        Order of operations (each step can raise; we rollback previous
        side-effects on failure to avoid zombie rows or leaked quotas):

            1. Pre-flight validation (pure) — cheap reject.
            2. Quota reservation — increments the tenant counters.
               If this raises (QuotaExceeded), we never wrote to the DB.
            3. Pool decision — on failure, cancel the reservation so
               the counter goes back to zero.
            4. Persist DRAFT row + DRAFT→QUEUED transition under lock.
               On any DB/FSM failure, cancel the reservation.

        Note: the DRAFT row is created **only after** steps 1-3 succeed.
        A previous revision flushed the row before quota -> zombie rows
        accumulated on quota rejection. This ordering guarantees the
        DB never contains a DRAFT row whose owning request aborted.
        """
        self._validate_submit(inp)

        # 1) Quota reservation (raises on violation — no DB side-effect yet).
        reservation = await self._scheduler.reserve(
            tenant_id=inp.tenant_id,
            estimated_gpu_seconds=inp.estimated_gpu_seconds,
        )

        # 2) Pool pick.
        shape = RunShape(
            dataset_rows=inp.dataset_rows,
            priority=inp.priority,  # type: ignore[arg-type]
            needs_gpu=inp.needs_gpu,
            requested_pool=inp.requested_pool,
        )
        try:
            decision = await self._scheduler.plan_placement(
                tenant_id=inp.tenant_id, shape=shape,
            )
        except Exception:
            await self._scheduler.quota.cancel_reservation(reservation)
            raise

        # 3) Persist the DRAFT row. Only now do we touch the runs table.
        run = FinetuneRun(
            id=new_run_id(),
            tenant_id=inp.tenant_id,
            task_id=inp.task_id,
            submitted_by=inp.submitted_by,
            snapshot_id=inp.snapshot_id,
            base_model=inp.base_model,
            recipe_version=inp.recipe_version,
            state=RunState.DRAFT.value,
            state_version=0,
            priority=inp.priority,
            hyperparameters=dict(inp.hyperparameters),
            tags=dict(inp.tags),
            attempt=0,
            retry_count=0,
        )
        try:
            self._session.add(run)
            await self._session.flush()
        except Exception:
            await self._scheduler.quota.cancel_reservation(reservation)
            raise

        # 4) DRAFT → QUEUED under lock + audit.
        def _stamp(r: FinetuneRun) -> None:
            r.pool = decision.pool
            r.attempt = 1

        try:
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.QUEUED,
                actor=inp.submitted_by,
                tenant_id=inp.tenant_id,
                reason=decision.reason,
                payload={
                    "pool": decision.pool,
                    "dataset_rows": inp.dataset_rows,
                    "priority": inp.priority,
                    "considered": list(decision.considered),
                },
                mutate=_stamp,
            )
        except Exception:
            await self._scheduler.quota.cancel_reservation(reservation)
            raise

        self._metrics.runs_submitted.inc(
            1.0, tenant=inp.tenant_id, pool=decision.pool,
        )
        await self._session.refresh(run)
        return RunView.from_row(run)

    # ------------------------------------------------------------- dispatch

    async def dispatch(
        self,
        *,
        run_id: str,
        tenant_id: str | None = None,
        worker_id: str | None = None,
    ) -> RunView:
        """Submit the worker Job to K8s and move QUEUED → SCHEDULED."""
        run = await self._get_or_raise(run_id, tenant_id=tenant_id)
        if run.state != RunState.QUEUED.value:
            raise ValidationError(
                f"cannot dispatch run in state {run.state!r}",
                detail={"state": run.state},
            )
        if run.pool is None:
            raise ValidationError(
                "run has no pool — cannot dispatch",
                detail={"run_id": run_id},
            )
        pool = self._scheduler.registry.by_name(run.pool)

        wid = worker_id or new_worker_id()
        spec = WorkerJobSpec(
            run_id=run.id,
            tenant_id=run.tenant_id,
            task_id=run.task_id,
            snapshot_id=run.snapshot_id,
            worker_id=wid,
            pool=pool,
            base_model=run.base_model,
            recipe_version=run.recipe_version,
            hyperparameters=dict(run.hyperparameters or {}),
        )
        manifest = render_worker_job(spec, self._k8s_settings,
                                     attempt=run.attempt)
        handle = await self._k8s.submit(manifest)

        def _stamp(r: FinetuneRun) -> None:
            r.worker_id = wid
            r.k8s_job_name = handle.name

        # If the FSM write fails after we submitted the K8s Job, the
        # Job is orphaned — no DB row points at it, but it keeps
        # running in the cluster. We delete it (best-effort) and
        # re-raise so the caller sees the FSM error.
        try:
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.SCHEDULED,
                actor="orchestrator",
                tenant_id=run.tenant_id,
                reason=f"dispatched to pool {run.pool!r} as job {handle.name!r}",
                payload={
                    "job_name": handle.name,
                    "job_namespace": handle.namespace,
                    "worker_id": wid,
                    "pool": run.pool,
                    "created": handle.created,
                },
                mutate=_stamp,
            )
        except Exception:
            # Best-effort cleanup: delete the Job we just created so
            # it doesn't linger in K8s as an orphan.
            try:
                await self._k8s.delete(handle.name, handle.namespace)
            except Exception:
                pass
            raise

        await self._session.refresh(run)
        return RunView.from_row(run)

    # ---------------------------------------------------------------- cancel

    async def cancel(
        self,
        *,
        run_id: str,
        actor: str,
        tenant_id: str | None = None,
        reason: str = "",
    ) -> RunView:
        """Cancel a run. Path depends on the current state.

        * DRAFT / QUEUED       → CANCELLED immediately.
        * ACTIVE state         → CANCELLING (worker finishes current
          step, then the worker-service transitions to CANCELLED on the
          next heartbeat / report).
        * RETRYING             → CANCELLED.
        * Terminal             → :class:`CannotCancel`.
        """
        run = await self._get_or_raise(run_id, tenant_id=tenant_id)
        current = RunState(run.state)

        if is_terminal(current):
            raise CannotCancel(
                f"run is already terminal ({current.value})",
            )

        if current in (RunState.DRAFT, RunState.QUEUED):
            to = RunState.CANCELLED
            # Release quota immediately on short-path cancel.
            await self._scheduler.release_concurrency(run.tenant_id)
        elif current is RunState.RETRYING:
            to = RunState.CANCELLED
            await self._scheduler.release_concurrency(run.tenant_id)
        elif current in ACTIVE_STATES:
            to = RunState.CANCELLING
        else:  # pragma: no cover — exhaustive
            raise CannotCancel(
                f"no cancellation path from {current.value}",
            )

        # Optional K8s cleanup when we're stopping a running Job.
        if current in ACTIVE_STATES and run.k8s_job_name:
            try:
                await self._k8s.delete(
                    run.k8s_job_name, self._k8s_settings.namespace,
                )
            except Exception:  # best-effort; log via caller
                pass

        await self._sm.transition(
            run_id=run.id,
            to_state=to,
            actor=actor,
            tenant_id=run.tenant_id,
            reason=reason or "user cancellation",
            payload={"from": current.value, "to": to.value},
        )
        await self._session.refresh(run)
        return RunView.from_row(run)

    # ---------------------------------------------------------------- retry

    async def retry_or_fail(
        self,
        *,
        run_id: str,
        error_code: str | None,
        error_message: str | None,
        tenant_id: str | None = None,
        actor: str = "orchestrator",
    ) -> RunView:
        """Decide retry vs FAILED after an active-state failure.

        Called by :class:`WorkerService.report_error` and
        :class:`HeartbeatReaper` follow-ups. Moves the run through:

            <ACTIVE> → RETRYING (recorded by caller) → QUEUED | FAILED
        """
        run = await self._get_or_raise(run_id, tenant_id=tenant_id)
        if run.state != RunState.RETRYING.value:
            raise ValidationError(
                f"retry_or_fail requires state RETRYING, got {run.state}",
            )

        error_class = classify_error(error_code, error_message)
        decision = self._retry_policy.should_retry(
            error_class=error_class,
            attempt=run.attempt,
            current_pool=run.pool,
            rng=random.Random(run.id),  # deterministic by run
        )

        if not decision.should_retry:
            # Go to FAILED, release quota.
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.FAILED,
                actor=actor,
                tenant_id=run.tenant_id,
                reason=decision.reason,
                payload={
                    "error_class": error_class.value,
                    "attempt": run.attempt,
                },
                mutate=lambda r: _stamp_error(r, error_code, error_message),
            )
            await self._scheduler.release_concurrency(run.tenant_id)
            await self._session.refresh(run)
            return RunView.from_row(run)

        # Upgrade pool if the policy suggested one.
        new_pool = decision.next_pool_hint or run.pool

        def _bump(r: FinetuneRun) -> None:
            r.attempt = decision.attempt_next
            r.retry_count = r.retry_count + 1
            r.next_retry_at = None  # consumed
            r.pool = new_pool
            r.worker_id = None
            r.k8s_job_name = None
            r.last_error_code = error_code
            r.last_error_message = error_message

        await self._sm.transition(
            run_id=run.id,
            to_state=RunState.QUEUED,
            actor=actor,
            tenant_id=run.tenant_id,
            reason=decision.reason,
            payload={
                "error_class": error_class.value,
                "delay_s": decision.delay_s,
                "pool_hint": decision.next_pool_hint,
                "attempt": decision.attempt_next,
            },
            mutate=_bump,
        )
        await self._session.refresh(run)
        return RunView.from_row(run)

    async def mark_retrying(
        self,
        *,
        run_id: str,
        error_code: str | None,
        error_message: str | None,
        actor: str = "worker",
    ) -> RunView:
        """Move an ACTIVE run to RETRYING after a reported error.

        Typically followed by :meth:`retry_or_fail` on the next reconciler
        tick. We split the two so the audit trail shows the hop.
        """
        run = await self._get_or_raise(run_id)
        current = RunState(run.state)
        if current not in ACTIVE_STATES:
            raise ValidationError(
                f"mark_retrying requires an ACTIVE state, got {current.value}",
            )
        await self._sm.transition(
            run_id=run.id,
            to_state=RunState.RETRYING,
            actor=actor,
            reason="error reported by worker",
            payload={
                "error_code": error_code,
                "error_message": error_message,
            },
            mutate=lambda r: _stamp_error(r, error_code, error_message),
        )
        await self._session.refresh(run)
        return RunView.from_row(run)

    # --------------------------------------------------------------- read side

    async def get(
        self,
        *,
        run_id: str,
        tenant_id: str | None = None,
    ) -> RunView:
        run = await self._get_or_raise(run_id, tenant_id=tenant_id)
        return RunView.from_row(run)

    async def list(
        self, filt: ListRunsFilter,
    ) -> tuple[Sequence[RunView], str | None]:
        stmt = (
            select(FinetuneRun)
            .where(FinetuneRun.tenant_id == filt.tenant_id)
            .order_by(FinetuneRun.created_at.desc(), FinetuneRun.id.desc())
        )
        if filt.task_id:
            stmt = stmt.where(FinetuneRun.task_id == filt.task_id)
        if filt.state:
            stmt = stmt.where(FinetuneRun.state == filt.state.value)
        if filt.submitted_by:
            stmt = stmt.where(FinetuneRun.submitted_by == filt.submitted_by)

        # simple offset-based pagination (token = base-10 offset)
        offset = int(filt.page_token) if filt.page_token else 0
        stmt = stmt.limit(filt.page_size + 1).offset(offset)
        rows = list((await self._session.execute(stmt)).scalars())
        next_token: str | None = None
        if len(rows) > filt.page_size:
            next_token = str(offset + filt.page_size)
            rows = rows[: filt.page_size]
        return tuple(RunView.from_row(r) for r in rows), next_token

    # -------------------------------------------------------------- helpers

    async def _get_or_raise(
        self, run_id: str, *, tenant_id: str | None = None,
    ) -> FinetuneRun:
        run = await self._session.get(FinetuneRun, run_id)
        if run is None:
            raise RunNotFound(f"run {run_id!r} does not exist")
        if tenant_id is not None and run.tenant_id != tenant_id:
            raise TenantMismatch(
                f"run {run_id!r} does not belong to tenant {tenant_id!r}",
            )
        return run

    def _validate_submit(self, inp: SubmitRunInput) -> None:
        if not inp.tenant_id or not inp.task_id:
            raise ValidationError("tenant_id and task_id are required")
        if not inp.submitted_by:
            raise ValidationError("submitted_by is required")
        if not inp.snapshot_id:
            raise ValidationError("snapshot_id is required")
        if inp.dataset_rows < 0:
            raise ValidationError(
                f"dataset_rows must be >= 0, got {inp.dataset_rows}",
            )
        if inp.priority not in ("standard", "premium"):
            raise ValidationError(
                f"unknown priority {inp.priority!r}",
            )


# ---------------------------------------------------------------------------
#  Row mutators — kept at module scope for pickling / typing clarity
# ---------------------------------------------------------------------------


def _stamp_error(
    r: FinetuneRun, error_code: str | None, error_message: str | None,
) -> None:
    r.last_error_code = error_code
    r.last_error_message = error_message


# Used by tests + dashboards — classifies the *current* state into a
# rough lifecycle bucket ("queued", "running", "done", "failed").
def lifecycle_bucket(state: RunState) -> str:
    if state in (RunState.DRAFT, RunState.QUEUED, RunState.RETRYING):
        return "queued"
    if state in ACTIVE_STATES:
        return "running"
    if state is RunState.CANCELLING:
        return "cancelling"
    if state is RunState.SUCCEEDED:
        return "done"
    if state is RunState.CANCELLED:
        return "cancelled"
    return "failed"


__all__ = [
    "ListRunsFilter",
    "RunService",
    "RunView",
    "SubmitRunInput",
    "lifecycle_bucket",
]


# Re-export at module level so test that imports `ErrorClass` via the
# service sees it. Avoid circular import by late-binding.
__all_internal_for_tests__ = [ErrorClass]
