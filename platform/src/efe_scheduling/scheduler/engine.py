"""SchedulerEngine — the main orchestration loop.

Responsibilities :

1. Acquire and renew the leader lease.
2. Query enabled schedules from the store.
3. Compute fires in ``(schedule.last_evaluated_at, now]``.
4. Build and enqueue one :class:`JobSpec` per unique
   ``(schedule_id, fire_time)`` — conflict (duplicate fire_key) is
   silently counted.
5. Persist ``last_evaluated_at`` via CAS on ``state_version``.
6. Emit structured events and Prometheus metrics.

The engine is **cooperative async** : it expects to be driven by an
event loop and uses the injected :class:`Clock` / :class:`Randomness`
for determinism in tests.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING, Any

from efe_scheduling.clock import Clock, SystemClock
from efe_scheduling.errors import DuplicateFireKey, StateVersionConflict
from efe_scheduling.randomness import Randomness, SystemRandomness
from efe_scheduling.scheduler.evaluators import build_evaluator
from efe_scheduling.scheduler.fire_key import fire_key_for
from efe_scheduling.scheduler.leader import LeaderElection
from efe_scheduling.types import (
    JobPriority,
    JobSpec,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    Schedule,
    new_uuid,
)

if TYPE_CHECKING:
    from efe_scheduling.storage.protocols import JobStore, ScheduleStore

__all__ = ["SchedulerEngine", "SchedulerEngineConfig", "TickOutcome"]


_LOG = logging.getLogger("efe_scheduling.engine")


@dataclass(frozen=True, slots=True)
class SchedulerEngineConfig:
    tick_period_s: float = 1.0
    lookahead_s: int = 60
    max_batch: int = 500
    catch_up_h: int = 6
    leader_ttl_s: int = 30


@dataclass(frozen=True, slots=True)
class TickOutcome:
    """Summary of one tick — returned for tests and for metrics."""

    leader: bool
    schedules_evaluated: int
    fires_enqueued: int
    duplicates: int
    dropped_catch_up: int
    errors: int = 0
    at: datetime = field(default_factory=lambda: datetime(2026, 1, 1, tzinfo=timezone.utc))


class SchedulerEngine:
    """Driver of the scheduling loop.

    Call :meth:`tick` repeatedly (or use :meth:`run` to loop forever).
    """

    __slots__ = (
        "_cfg",
        "_clock",
        "_job_store",
        "_leader",
        "_random",
        "_schedule_store",
        "_tenant_id",
    )

    def __init__(
        self,
        *,
        tenant_id: str,
        schedule_store: "ScheduleStore",
        job_store: "JobStore",
        leader: LeaderElection,
        clock: Clock | None = None,
        random_source: Randomness | None = None,
        config: SchedulerEngineConfig | None = None,
    ) -> None:
        self._tenant_id = tenant_id
        self._schedule_store = schedule_store
        self._job_store = job_store
        self._leader = leader
        self._clock: Clock = clock if clock is not None else SystemClock()
        self._random: Randomness = random_source if random_source is not None else SystemRandomness()
        self._cfg = config if config is not None else SchedulerEngineConfig()

    @property
    def config(self) -> SchedulerEngineConfig:
        return self._cfg

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def tick(self) -> TickOutcome:
        now = self._clock.now()

        # Leader lifecycle : only acquire if we aren't already holder.
        # On a real pg_try_advisory_lock impl, calling acquire when we
        # already hold the lock may block or return false-negatives ;
        # renewing is the correct idiom.
        if self._leader.is_leader():
            await self._leader.renew(ttl_s=self._cfg.leader_ttl_s)
        else:
            acquired = await self._leader.acquire(ttl_s=self._cfg.leader_ttl_s)
            if not acquired:
                return TickOutcome(
                    leader=False,
                    schedules_evaluated=0,
                    fires_enqueued=0,
                    duplicates=0,
                    dropped_catch_up=0,
                    at=now,
                )

        enqueued = 0
        dup = 0
        dropped = 0
        errors = 0
        evaluated = 0
        try:
            schedules = await self._schedule_store.list_enabled(
                tenant_id=self._tenant_id, evaluated_before=now
            )
            for sched in schedules:
                try:
                    en, du, dr = await self._process_schedule(sched, now=now)
                    enqueued += en
                    dup += du
                    dropped += dr
                    evaluated += 1
                except Exception as exc:  # noqa: BLE001 - engine must not die
                    errors += 1
                    _LOG.exception(
                        "scheduler.engine.schedule_error schedule_id=%s err=%s",
                        sched.schedule_id,
                        exc,
                    )
            self._emit_tick_metrics(
                enqueued=enqueued, duplicates=dup, dropped=dropped
            )
        except Exception:  # pragma: no cover - defensive
            _LOG.exception("scheduler.engine.tick_error")
            errors += 1

        return TickOutcome(
            leader=True,
            schedules_evaluated=evaluated,
            fires_enqueued=enqueued,
            duplicates=dup,
            dropped_catch_up=dropped,
            errors=errors,
            at=now,
        )

    # ------------------------------------------------------------------
    # Metrics
    # ------------------------------------------------------------------

    def _emit_tick_metrics(
        self, *, enqueued: int, duplicates: int, dropped: int
    ) -> None:
        """Increment the fire / duplicate / dropped counters."""
        try:
            from efe_scheduling.telemetry.metrics import get_registry

            reg = get_registry()
            fire = reg.get("efe_sched_fire_total")
            dup = reg.get("efe_sched_fire_duplicate_total")
            drop = reg.get("efe_sched_fire_dropped_total")
            if fire is not None and enqueued:
                fire.inc(float(enqueued), tenant=self._tenant_id, kind="mixed")
            if dup is not None and duplicates:
                dup.inc(float(duplicates), tenant=self._tenant_id, kind="mixed")
            if drop is not None and dropped:
                drop.inc(float(dropped), tenant=self._tenant_id, reason="catch_up")
        except Exception:  # pragma: no cover - metrics must never break a tick
            _LOG.exception("scheduler.engine.metrics_error")

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    async def _process_schedule(
        self, sched: Schedule, *, now: datetime
    ) -> tuple[int, int, int]:
        """Compute and enqueue fires for a single schedule.

        Returns ``(enqueued, duplicates, dropped_catch_up)``.
        """
        evaluator = build_evaluator(sched.spec)
        # Catch-up truncation.
        min_start = now - timedelta(hours=self._cfg.catch_up_h)
        last_eval = sched.last_evaluated_at or min_start
        eff_start = max(last_eval, min_start)

        dropped = 0
        if last_eval < min_start:
            # Count the dropped window as a single metric hit; more precise
            # counting would require enumerating them, which defeats the
            # purpose of truncation.
            dropped = 1
            _LOG.warning(
                "scheduler.engine.catch_up_truncated schedule_id=%s dropped_until=%s",
                sched.schedule_id,
                min_start.isoformat(),
            )

        enqueued = 0
        duplicates = 0
        for fire_time in evaluator.iter_fires(
            eff_start, now, limit=self._cfg.max_batch
        ):
            key = fire_key_for(sched.schedule_id, fire_time)
            jitter_s = self._random.randint(0, max(0, self._jitter_budget(sched)))
            run_at = fire_time + timedelta(seconds=jitter_s)
            spec = JobSpec(
                job_id=new_uuid(),
                fire_key=key,
                tenant_id=sched.tenant_id,
                task_id=sched.task_id,
                kind=sched.job_kind,
                payload=dict(sched.payload),
                priority=JobPriority.normal,
                priority_seq=new_uuid(),
                run_at=run_at,
                deadline=None,
                max_attempts=3,
                retry_policy=RetryPolicySpec(
                    kind=RetryPolicyKind.exponential,
                    base_ms=500,
                    max_ms=60_000,
                    jitter_ratio=0.2,
                ),
                origin=Origin.schedule,
                origin_ref=sched.schedule_id,
                created_at=now,
            ).sealed()
            try:
                await self._job_store.enqueue(spec)
                enqueued += 1
            except DuplicateFireKey:
                duplicates += 1

        # Persist last_evaluated_at with CAS.
        try:
            await self._schedule_store.mark_evaluated(
                schedule_id=sched.schedule_id,
                until=now,
                expected_version=sched.state_version,
            )
        except StateVersionConflict:
            # Another instance raced us; that's fine in followers.
            _LOG.info(
                "scheduler.engine.cas_conflict schedule_id=%s", sched.schedule_id
            )

        return enqueued, duplicates, dropped

    @staticmethod
    def _jitter_budget(sched: Schedule) -> int:
        spec: Any = sched.spec
        return int(getattr(spec, "jitter_s", 0))
