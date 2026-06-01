"""Heartbeat reaper — detect dead workers.

The reaper scans every ``reaper_scan_interval_s`` for runs whose last
observed heartbeat is older than
``interval_s * miss_threshold``. For each stale run:

1. The state machine transitions ``<ACTIVE> → LOST``.
2. A ``run_state_transitions`` row is written with reason
   ``heartbeat timeout after N seconds``.
3. The retry policy is consulted — if the run has retry budget left,
   the service submits it back to QUEUED with an upgraded pool if
   appropriate.

The reaper is deliberately **stateless** — each scan recomputes
everything. There's no in-memory cursor or cache to recover from a
crash.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_orch.clock import Clock, SystemClock
from efe_orch.config import HeartbeatSettings
from efe_orch.db.models import FinetuneRun, WorkerHeartbeat
from efe_orch.scheduler.service import SchedulerService
from efe_orch.state_machine.states import ACTIVE_STATES, RunState
from efe_orch.state_machine.transitions import StateMachine


def _as_utc(ts: datetime) -> datetime:
    """Coerce a SQLite-read naive datetime to UTC.

    SQLAlchemy with DateTime(timezone=True) on SQLite returns naive
    datetimes; we know they were written in UTC, so we graft the tz
    back on. On Postgres this is a no-op (ts is already UTC-aware).
    """
    if ts.tzinfo is None:
        return ts.replace(tzinfo=timezone.utc)
    return ts.astimezone(timezone.utc)


@dataclass(frozen=True, slots=True)
class ReaperReport:
    """Summary returned by :meth:`HeartbeatReaper.scan`."""

    scanned: int
    reaped: tuple[str, ...] = ()
    skipped: tuple[str, ...] = ()
    at: datetime | None = None


@dataclass
class _StaleRun:
    run_id: str
    tenant_id: str
    state: RunState
    last_seen: datetime | None
    worker_id: str | None
    age_s: float


class HeartbeatReaper:
    """Declare dead workers and transition their runs to LOST.

    Construction is cheap and stateless; you can run the reaper as a
    separate asyncio task in the main event loop or via a scheduled
    job. The reaper takes a state machine so the two LOST transitions
    and the audit row happen atomically per run.
    """

    def __init__(
        self,
        session: AsyncSession,
        *,
        state_machine: StateMachine,
        settings: HeartbeatSettings,
        scheduler: SchedulerService | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._sm = state_machine
        self._settings = settings
        # Optional: when supplied the reaper releases the tenant's
        # concurrency slot on every LOST transition so quota does not
        # drift upward over time. Strongly recommended in production;
        # tests can omit it when they don't care about quota.
        self._scheduler = scheduler
        self._clock = clock or SystemClock()

    async def scan(self, *, limit: int | None = None) -> ReaperReport:
        """Run one reaping cycle.

        Returns a :class:`ReaperReport` summarising the runs that were
        transitioned to LOST. Skipped runs are ones that had a fresh
        heartbeat between listing and locking (benign race).
        """
        now = self._clock.now()
        threshold_s = self._settings.interval_s * self._settings.miss_threshold
        cutoff = now - timedelta(seconds=threshold_s)
        batch_limit = limit or self._settings.reaper_batch_size

        stale = await self._list_stale(cutoff=cutoff, limit=batch_limit)
        reaped: list[str] = []
        skipped: list[str] = []

        for s in stale:
            # Re-check under lock-free conditions; the state machine
            # grabs the lock itself.
            try:
                await self._sm.transition(
                    run_id=s.run_id,
                    to_state=RunState.LOST,
                    actor="reaper",
                    reason=(
                        f"heartbeat timeout after {s.age_s:.0f}s "
                        f"(threshold={threshold_s}s)"
                    ),
                    payload={
                        "worker_id": s.worker_id,
                        "last_seen": s.last_seen.isoformat()
                                     if s.last_seen else None,
                        "threshold_s": threshold_s,
                    },
                )
            except Exception:
                skipped.append(s.run_id)
                continue

            # LOST is terminal → release the tenant's concurrency slot.
            # Without this, every reaped run silently leaks a slot and
            # the tenant eventually hits its quota cap with zombie
            # reservations. Best-effort: if quota release fails we
            # still count the run as reaped.
            if self._scheduler is not None:
                try:
                    await self._scheduler.release_concurrency(s.tenant_id)
                except Exception:
                    pass
            reaped.append(s.run_id)

        return ReaperReport(
            scanned=len(stale),
            reaped=tuple(reaped),
            skipped=tuple(skipped),
            at=now,
        )

    # ---------------------------------------------------------------- helpers

    async def _list_stale(
        self, *, cutoff: datetime, limit: int,
    ) -> list[_StaleRun]:
        """Find active runs whose last heartbeat (or updated_at) is older than cutoff."""
        # Active states we care about. RETRYING is *not* here because
        # a RETRYING run has no worker attached.
        active_values = [s.value for s in ACTIVE_STATES]

        # A single SQL would require a LATERAL/ROW_NUMBER which
        # SQLAlchemy + SQLite don't render cleanly. We do a two-step:
        # (a) pull every active run, (b) for each fetch the max
        # observed_at. The workload is small (hundreds of runs in
        # flight at most) so this is fine.
        stmt = (
            select(FinetuneRun)
            .where(FinetuneRun.state.in_(active_values))
            .order_by(FinetuneRun.updated_at.asc())
            .limit(limit)
        )
        rows = list((await self._session.execute(stmt)).scalars())

        now = _as_utc(self._clock.now())
        cutoff_utc = _as_utc(cutoff)
        out: list[_StaleRun] = []
        for r in rows:
            last_seen_raw = await self._latest_heartbeat(r.id)
            # Fall back to the run row's ``updated_at`` if no HB row
            # (e.g. the worker died before sending its first beat).
            effective = _as_utc(last_seen_raw or r.updated_at)
            age_s = (now - effective).total_seconds()
            if effective < cutoff_utc:
                out.append(_StaleRun(
                    run_id=r.id,
                    tenant_id=r.tenant_id,
                    state=RunState(r.state),
                    last_seen=effective,
                    worker_id=r.worker_id,
                    age_s=age_s,
                ))
        return out

    async def _latest_heartbeat(self, run_id: str) -> datetime | None:
        stmt = (
            select(WorkerHeartbeat.observed_at)
            .where(WorkerHeartbeat.run_id == run_id)
            .order_by(WorkerHeartbeat.observed_at.desc())
            .limit(1)
        )
        res = await self._session.execute(stmt)
        return res.scalar_one_or_none()


__all__ = ["HeartbeatReaper", "ReaperReport"]
