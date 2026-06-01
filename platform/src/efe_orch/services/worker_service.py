"""Worker-facing service — ingest heartbeats, checkpoints, results, errors.

This service is the inverse of :class:`RunService`: it accepts calls
from the worker fleet and drives the FSM based on what each worker
reports. The four verbs mirror the ``WorkerService`` gRPC proto:

    Heartbeat         → :meth:`heartbeat`
    ReportCheckpoint  → :meth:`report_checkpoint`
    ReportResult      → :meth:`report_result` (success OR cancel ack)
    ReportError       → :meth:`report_error`

Each call is tenant-scoped via the run's ``tenant_id``; we never
trust ``tenant_id`` passed by the worker. Workers authenticate via a
short-lived bearer token whose claims include ``worker_id``; the
service asserts the bound ``worker_id`` matches the one on the run
(defence against a rogue worker posting on another run's ID).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import Clock, SystemClock
from efe_orch.db.models import FinetuneRun, RunCheckpoint
from efe_orch.errors import (
    RunNotFound,
    UnknownWorker,
    ValidationError,
    WorkerProtocolError,
)
from efe_orch.heartbeat.recorder import HeartbeatInput, HeartbeatRecorder
from efe_orch.ids import new_uuid
from efe_orch.scheduler import SchedulerService
from efe_orch.services.run_service import RunService, RunView
from efe_orch.state_machine.states import (
    ACTIVE_STATES,
    RunState,
    is_terminal,
)
from efe_orch.state_machine.transitions import StateMachine


@dataclass(frozen=True, slots=True)
class CheckpointReport:
    run_id: str
    worker_id: str
    step: int
    blob_uri: str
    metrics: dict[str, Any] = field(default_factory=dict)
    size_bytes: int = 0
    is_best: bool = False


@dataclass(frozen=True, slots=True)
class ResultReport:
    run_id: str
    worker_id: str
    artifact_url: str
    metrics: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ErrorReport:
    run_id: str
    worker_id: str
    error_code: str
    error_message: str


class WorkerService:
    def __init__(
        self,
        session: AsyncSession,
        *,
        state_machine: StateMachine,
        run_service: RunService,
        scheduler: SchedulerService,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._sm = state_machine
        self._run_service = run_service
        self._scheduler = scheduler
        self._clock = clock or SystemClock()
        self._hb_recorder = HeartbeatRecorder(session, clock=self._clock)

    # ------------------------------------------------------------------ HB

    async def heartbeat(self, hb: HeartbeatInput) -> RunView:
        """Record a heartbeat.

        Transitions SCHEDULED → TRAINING on first heartbeat (the
        worker is up and doing something). Any later beat is just
        persisted.
        """
        run = await self._get_run(hb.run_id, hb.worker_id)
        await self._hb_recorder.record(hb)

        if run.state == RunState.SCHEDULED.value:
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.TRAINING,
                actor=f"worker/{hb.worker_id}",
                reason="first heartbeat received",
                payload={
                    "beat_seq": hb.beat_seq,
                    "state_hint": hb.state_hint,
                },
            )
        await self._session.refresh(run)
        return RunView.from_row(run)

    # --------------------------------------------------------------- CKPT

    async def report_checkpoint(self, rep: CheckpointReport) -> str:
        """Persist a checkpoint pointer."""
        if rep.step < 0:
            raise WorkerProtocolError(
                f"checkpoint step must be >= 0, got {rep.step}",
            )
        if not rep.blob_uri:
            raise WorkerProtocolError("checkpoint blob_uri required")
        run = await self._get_run(rep.run_id, rep.worker_id)
        if run.state not in (RunState.TRAINING.value, RunState.EVALUATING.value):
            raise ValidationError(
                f"cannot accept checkpoint in state {run.state!r}",
            )
        ckpt = RunCheckpoint(
            id=new_uuid(),
            run_id=rep.run_id,
            step=rep.step,
            blob_uri=rep.blob_uri,
            metrics=dict(rep.metrics),
            size_bytes=max(0, int(rep.size_bytes)),
            is_best=bool(rep.is_best),
        )
        self._session.add(ckpt)
        await self._session.flush()
        return ckpt.id

    # ------------------------------------------------------------- RESULT

    async def report_result(self, rep: ResultReport) -> RunView:
        """Worker declares success. Drives TRAINING → EVALUATING → UPLOADING → SUCCEEDED.

        Implementation is *walk-forward*: whatever progression-compatible
        state the run is in (TRAINING, EVALUATING, or UPLOADING), we
        step it forward one transition at a time until it reaches
        SUCCEEDED. This makes the call idempotent under worker retries:
        a worker that successfully delivered a result but failed to
        observe the ack can re-send — we just pick up from wherever the
        previous call left off instead of crashing with
        ``InvalidTransition`` (the previous implementation, buggy, tried
        ``UPLOADING → UPLOADING`` and raised).

        Cancellation races are handled explicitly: if the run reached
        CANCELLING in the meantime (user cancelled), we honour the
        cancel and never transition to SUCCEEDED.
        """
        run = await self._get_run(rep.run_id, rep.worker_id)
        if is_terminal(run.state):
            raise ValidationError(
                f"run already terminal ({run.state!r})",
            )
        if run.state == RunState.CANCELLING.value:
            # Worker finished but user cancelled meanwhile → honour cancel.
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.CANCELLED,
                actor=f"worker/{rep.worker_id}",
                reason="worker completed while cancellation was requested",
            )
            await self._scheduler.release_concurrency(run.tenant_id)
            await self._session.refresh(run)
            return RunView.from_row(run)

        current = RunState(run.state)
        if current not in (
            RunState.TRAINING, RunState.EVALUATING, RunState.UPLOADING,
        ):
            raise ValidationError(
                f"cannot report result in state {current.value!r}",
                detail={"state": current.value},
            )

        # Walk forward from wherever we are. Every branch is a legal
        # edge in the FSM (see state_machine.states.VALID_TRANSITIONS).
        if current is RunState.TRAINING:
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.EVALUATING,
                actor=f"worker/{rep.worker_id}",
                reason="training complete, starting eval summary",
            )
            current = RunState.EVALUATING

        if current is RunState.EVALUATING:
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.UPLOADING,
                actor=f"worker/{rep.worker_id}",
                reason="eval complete, uploading artifact",
                payload={"metrics_keys": sorted(rep.metrics.keys())},
            )
            current = RunState.UPLOADING

        # current must be UPLOADING here.
        def _stamp(r: FinetuneRun) -> None:
            r.result_artifact_url = rep.artifact_url
            r.result_metrics = dict(rep.metrics)

        await self._sm.transition(
            run_id=run.id,
            to_state=RunState.SUCCEEDED,
            actor=f"worker/{rep.worker_id}",
            reason="artifact uploaded",
            payload={"artifact_url": rep.artifact_url},
            mutate=_stamp,
        )
        await self._scheduler.release_concurrency(run.tenant_id)
        await self._session.refresh(run)
        return RunView.from_row(run)

    # --------------------------------------------------------------- ERROR

    async def report_error(self, rep: ErrorReport) -> RunView:
        """Worker reports a failure. Moves to RETRYING, then retry or fail."""
        run = await self._get_run(rep.run_id, rep.worker_id)
        current = RunState(run.state)

        if is_terminal(current):
            # Already failed/cancelled — just log.
            await self._session.refresh(run)
            return RunView.from_row(run)

        if current is RunState.CANCELLING:
            # User had requested cancel; the worker error confirms shutdown.
            await self._sm.transition(
                run_id=run.id,
                to_state=RunState.CANCELLED,
                actor=f"worker/{rep.worker_id}",
                reason=f"cancelled during worker error: {rep.error_code}",
            )
            await self._scheduler.release_concurrency(run.tenant_id)
            await self._session.refresh(run)
            return RunView.from_row(run)

        if current not in ACTIVE_STATES:
            raise ValidationError(
                f"report_error requires ACTIVE state, got {current.value}",
            )

        # First hop: ACTIVE → RETRYING (records the error on the row).
        await self._run_service.mark_retrying(
            run_id=run.id,
            error_code=rep.error_code,
            error_message=rep.error_message,
            actor=f"worker/{rep.worker_id}",
        )
        # Second hop: RETRYING → QUEUED | FAILED (consulting retry policy).
        return await self._run_service.retry_or_fail(
            run_id=run.id,
            error_code=rep.error_code,
            error_message=rep.error_message,
            actor=f"worker/{rep.worker_id}",
        )

    # -------------------------------------------------------------- helpers

    async def _get_run(self, run_id: str, worker_id: str) -> FinetuneRun:
        """Fetch a run and assert the caller is its owning worker.

        We require ``worker_id`` to match ``run.worker_id`` exactly.
        Previously the check used ``if run.worker_id and ...`` which
        accidentally let *any* worker call heartbeat/result/error on a
        run whose ``worker_id`` was still NULL (e.g., the narrow window
        between K8s submit and FSM write in ``dispatch``). That was an
        authorisation hole; it is now closed by requiring the field to
        be both non-null AND equal.

        The only legitimate caller before the dispatcher has stamped
        ``worker_id`` would be the worker itself, but the worker does
        not send its first RPC until it has its assigned ``worker_id``
        (supplied via env var in the Job manifest). So refusing calls
        with ``run.worker_id is None`` is safe.
        """
        run = await self._session.get(FinetuneRun, run_id)
        if run is None:
            raise RunNotFound(f"run {run_id!r} does not exist")
        if run.worker_id is None:
            raise UnknownWorker(
                f"run {run_id!r} has no assigned worker yet; "
                f"rejecting RPC from {worker_id!r}",
            )
        if run.worker_id != worker_id:
            raise UnknownWorker(
                f"worker {worker_id!r} is not the owner of run {run_id!r}",
            )
        return run


__all__ = [
    "CheckpointReport",
    "ErrorReport",
    "ResultReport",
    "WorkerService",
]
