"""Heartbeat ingestion.

A running worker POSTs one heartbeat every ``heartbeat.interval_s``
seconds. The recorder validates the payload, inserts a row into
``worker_heartbeats``, and updates soft progress counters on the
run's row (``updated_at``, optional progress metrics).

Validation rules:

* ``beat_seq`` must be strictly monotonic *per (run_id, worker_id)* —
  out-of-order beats are rejected as :class:`StaleHeartbeat` so a
  buggy worker replaying old beats doesn't skew the reaper.
* ``run_id`` must exist (else :class:`RunNotFound`).
* The run's state must be one that expects heartbeats (i.e. any of the
  ACTIVE states). Heartbeats from terminal runs are rejected with
  :class:`StaleHeartbeat` so the worker stops sending; this is a
  normal race condition (run cancelled while worker was still alive).
* ``progress_ratio`` must be in ``[0.0, 1.0]``.

We deliberately do **not** mutate ``run.state`` here — that's the job
of :class:`HeartbeatReaper` or the explicit ``ReportResult`` RPC.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import Clock, SystemClock
from efe_orch.db.models import FinetuneRun, WorkerHeartbeat
from efe_orch.errors import (
    RunNotFound,
    StaleHeartbeat,
    WorkerProtocolError,
)
from efe_orch.ids import new_hb_id
from efe_orch.state_machine.states import ACTIVE_STATES, RunState, is_terminal


@dataclass(frozen=True, slots=True)
class HeartbeatInput:
    """Parsed payload from a worker's ``Heartbeat`` RPC."""

    run_id: str
    worker_id: str
    beat_seq: int
    state_hint: str = "training"
    progress_ratio: float = 0.0
    step: int = 0
    extra: dict[str, Any] = field(default_factory=dict)


class HeartbeatRecorder:
    """Persist heartbeats + update soft counters on the run."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._clock = clock or SystemClock()

    async def record(self, hb: HeartbeatInput) -> WorkerHeartbeat:
        self._validate(hb)
        run = await self._session.get(FinetuneRun, hb.run_id)
        if run is None:
            raise RunNotFound(f"run {hb.run_id!r} is unknown")

        # Silently ignore beats from terminal runs.
        if is_terminal(run.state):
            raise StaleHeartbeat(
                f"run {hb.run_id!r} is {run.state}; heartbeat discarded",
                detail={"run_state": run.state},
            )

        # Defend against replay / out-of-order.
        last_seq = await self._last_seq(hb.run_id, hb.worker_id)
        if last_seq is not None and hb.beat_seq <= last_seq:
            raise StaleHeartbeat(
                f"beat_seq {hb.beat_seq} is not greater than last {last_seq}",
                detail={"beat_seq": hb.beat_seq, "last": last_seq},
            )

        now = self._clock.now()
        # Warn if run has a different worker_id (two workers think
        # they own the run). We accept the beat but surface a hint.
        extra = dict(hb.extra)
        if run.worker_id and run.worker_id != hb.worker_id:
            extra.setdefault("warning", "worker_id_mismatch")

        row = WorkerHeartbeat(
            id=new_hb_id(),
            run_id=hb.run_id,
            worker_id=hb.worker_id,
            beat_seq=hb.beat_seq,
            observed_at=now,
            state_hint=hb.state_hint,
            progress_ratio=max(0.0, min(1.0, float(hb.progress_ratio))),
            step=int(hb.step),
            extra=extra,
        )
        self._session.add(row)

        # Soft-update the run row so "last seen" queries don't need a
        # join to worker_heartbeats on the hot path.
        if RunState(run.state) in ACTIVE_STATES:
            run.updated_at = now
            if not run.worker_id:
                run.worker_id = hb.worker_id

        await self._session.flush()
        return row

    # ---------------------------------------------------------------- helpers

    def _validate(self, hb: HeartbeatInput) -> None:
        if not hb.run_id or not hb.worker_id:
            raise WorkerProtocolError("heartbeat missing run_id or worker_id")
        if hb.beat_seq < 0:
            raise WorkerProtocolError(
                f"beat_seq must be >= 0, got {hb.beat_seq}",
            )
        if not (0.0 <= float(hb.progress_ratio) <= 1.0):
            raise WorkerProtocolError(
                f"progress_ratio must be in [0,1], got {hb.progress_ratio}",
            )
        if hb.step < 0:
            raise WorkerProtocolError(
                f"step must be >= 0, got {hb.step}",
            )

    async def _last_seq(
        self, run_id: str, worker_id: str,
    ) -> int | None:
        stmt = (
            select(WorkerHeartbeat.beat_seq)
            .where(WorkerHeartbeat.run_id == run_id)
            .where(WorkerHeartbeat.worker_id == worker_id)
            .order_by(WorkerHeartbeat.beat_seq.desc())
            .limit(1)
        )
        res = await self._session.execute(stmt)
        return res.scalar_one_or_none()


__all__ = ["HeartbeatInput", "HeartbeatRecorder"]
