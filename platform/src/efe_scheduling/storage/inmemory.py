"""In-memory implementations of every storage protocol.

These are used by tests and by single-node development. They faithfully
reproduce the CAS semantics of the SQL backend so tests catch version
conflicts the same way production would.

Thread/async-safe via a :class:`asyncio.Lock` per store. Not intended for
distributed use.

The :class:`InMemoryJobStore` honors invariant **P4** : it receives a
:class:`Clock` instance rather than calling ``datetime.now()`` directly,
so tests driven by a :class:`FakeClock` see reproducible lease windows.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any

from efe_scheduling.clock import Clock, SystemClock
from efe_scheduling.errors import (
    DuplicateFireKey,
    JobNotFound,
    ScheduleNotFound,
    StateVersionConflict,
    TriggerNotFound,
)
from efe_scheduling.fsm import JOB_ALLOWED, SCHEDULE_ALLOWED, guard
from efe_scheduling.types import (
    JobKind,
    JobRun,
    JobSpec,
    JobState,
    Schedule,
    ScheduleState,
)

__all__ = [
    "InMemoryDeadLetterStore",
    "InMemoryIdempotencyStore",
    "InMemoryJobStore",
    "InMemoryPolicyStore",
    "InMemoryRunStore",
    "InMemoryScheduleStore",
    "InMemoryTriggerRecord",
    "InMemoryTriggerStore",
]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _cas(expected: int, actual: int, aggregate: str) -> None:
    if expected != actual:
        raise StateVersionConflict(
            f"{aggregate}: version mismatch expected={expected} actual={actual}"
        )


# ---------------------------------------------------------------------------
# ScheduleStore
# ---------------------------------------------------------------------------


class InMemoryScheduleStore:
    """In-memory :class:`~efe_scheduling.storage.protocols.ScheduleStore`."""

    def __init__(self) -> None:
        self._rows: dict[str, Schedule] = {}
        self._lock = asyncio.Lock()

    async def create(self, schedule: Schedule) -> Schedule:
        async with self._lock:
            if schedule.schedule_id in self._rows:
                raise ValueError(f"schedule_id {schedule.schedule_id} already exists")
            self._rows[schedule.schedule_id] = schedule
            return schedule

    async def get(self, schedule_id: str) -> Schedule:
        async with self._lock:
            try:
                return self._rows[schedule_id]
            except KeyError as exc:
                raise ScheduleNotFound(f"schedule {schedule_id} not found") from exc

    async def list_enabled(
        self, *, tenant_id: str, evaluated_before: datetime
    ) -> list[Schedule]:
        async with self._lock:
            return [
                s
                for s in self._rows.values()
                if s.tenant_id == tenant_id
                and s.enabled
                and s.state == ScheduleState.enabled
                and (
                    s.last_evaluated_at is None
                    or s.last_evaluated_at < evaluated_before
                )
            ]

    async def mark_evaluated(
        self, *, schedule_id: str, until: datetime, expected_version: int
    ) -> int:
        async with self._lock:
            row = self._rows.get(schedule_id)
            if row is None:
                raise ScheduleNotFound(f"schedule {schedule_id} not found")
            _cas(expected_version, row.state_version, f"schedule {schedule_id}")
            updated = row.with_evaluated(until)
            self._rows[schedule_id] = updated
            return updated.state_version

    async def transition(
        self,
        *,
        schedule_id: str,
        to_state: ScheduleState,
        expected_version: int,
        now: datetime,
    ) -> int:
        async with self._lock:
            row = self._rows.get(schedule_id)
            if row is None:
                raise ScheduleNotFound(f"schedule {schedule_id} not found")
            _cas(expected_version, row.state_version, f"schedule {schedule_id}")
            guard(row.state.value, to_state.value, SCHEDULE_ALLOWED, aggregate="schedule")
            updated = row.with_state(to_state, now=now)
            self._rows[schedule_id] = updated
            return updated.state_version


# ---------------------------------------------------------------------------
# JobStore
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _JobRow:
    spec: JobSpec
    state: JobState
    state_version: int
    attempts_left: int
    lease_expires_at: datetime | None = None
    last_worker_id: str | None = None


class InMemoryJobStore:
    """In-memory :class:`~efe_scheduling.storage.protocols.JobStore`."""

    def __init__(self, *, clock: Clock | None = None) -> None:
        self._rows: dict[str, _JobRow] = {}
        self._by_fire_key: dict[bytes, str] = {}
        self._lock = asyncio.Lock()
        self._clock: Clock = clock if clock is not None else SystemClock()

    async def enqueue(self, spec: JobSpec) -> None:
        async with self._lock:
            if spec.fire_key in self._by_fire_key:
                raise DuplicateFireKey(
                    "duplicate fire_key", detail={"fire_key": spec.fire_key.hex()}
                )
            self._rows[spec.job_id] = _JobRow(
                spec=spec,
                state=JobState.queued,
                state_version=1,
                attempts_left=spec.max_attempts,
            )
            self._by_fire_key[spec.fire_key] = spec.job_id

    async def get(self, job_id: str) -> JobSpec:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            return row.spec

    async def current_state(self, job_id: str) -> JobState:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            return row.state

    async def current_version(self, job_id: str) -> int:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            return row.state_version

    async def lease_next(
        self,
        *,
        worker_id: str,
        tenant_id: str | None = None,
        lease_ttl: timedelta,
    ) -> JobSpec | None:
        async with self._lock:
            now = self._clock.now()
            candidates: list[_JobRow] = []
            for row in self._rows.values():
                if tenant_id is not None and row.spec.tenant_id != tenant_id:
                    continue
                if row.state != JobState.queued:
                    continue
                if row.spec.run_at > now:
                    continue
                if row.lease_expires_at is not None and row.lease_expires_at > now:
                    continue
                candidates.append(row)
            if not candidates:
                return None
            candidates.sort(key=lambda r: (-int(r.spec.priority), r.spec.run_at, r.spec.priority_seq))
            chosen = candidates[0]

            # transition to running, consume one attempt
            guard(chosen.state.value, JobState.running.value, JOB_ALLOWED, aggregate="job")
            chosen.state = JobState.running
            chosen.state_version += 1
            chosen.attempts_left -= 1
            chosen.lease_expires_at = now + lease_ttl
            chosen.last_worker_id = worker_id
            return chosen.spec

    async def extend_lease(
        self, *, job_id: str, new_ttl: timedelta, expected_version: int
    ) -> int:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            _cas(expected_version, row.state_version, f"job {job_id}")
            row.state_version += 1
            row.lease_expires_at = self._clock.now() + new_ttl
            return row.state_version

    async def transition(
        self,
        *,
        job_id: str,
        to_state: JobState,
        expected_version: int,
        fields: dict[str, Any],
    ) -> int:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            _cas(expected_version, row.state_version, f"job {job_id}")
            guard(row.state.value, to_state.value, JOB_ALLOWED, aggregate="job")
            row.state = to_state
            row.state_version += 1
            if "run_at" in fields:
                # Need to rebuild spec since it's frozen. Simulate by keeping
                # the run_at field in the _JobRow auxiliary attributes.
                new_run_at = fields["run_at"]
                row.spec = JobSpec(  # type: ignore[misc]
                    job_id=row.spec.job_id,
                    fire_key=row.spec.fire_key,
                    tenant_id=row.spec.tenant_id,
                    task_id=row.spec.task_id,
                    kind=row.spec.kind,
                    payload=row.spec.payload,
                    priority=row.spec.priority,
                    priority_seq=row.spec.priority_seq,
                    run_at=new_run_at,
                    deadline=row.spec.deadline,
                    max_attempts=row.spec.max_attempts,
                    retry_policy=row.spec.retry_policy,
                    origin=row.spec.origin,
                    origin_ref=row.spec.origin_ref,
                    created_at=row.spec.created_at,
                    hmac_seal=row.spec.hmac_seal,
                )
            if "release_lease" in fields and fields["release_lease"]:
                row.lease_expires_at = None
            return row.state_version

    async def list_by_state(
        self, *, state: JobState, tenant_id: str | None = None, limit: int = 100
    ) -> list[JobSpec]:
        async with self._lock:
            out: list[JobSpec] = []
            for row in self._rows.values():
                if row.state != state:
                    continue
                if tenant_id is not None and row.spec.tenant_id != tenant_id:
                    continue
                out.append(row.spec)
                if len(out) >= limit:
                    break
            return out

    async def attempts_left(self, job_id: str) -> int:
        async with self._lock:
            row = self._rows.get(job_id)
            if row is None:
                raise JobNotFound(f"job {job_id} not found")
            return row.attempts_left


# ---------------------------------------------------------------------------
# RunStore
# ---------------------------------------------------------------------------


class InMemoryRunStore:
    def __init__(self) -> None:
        self._rows: dict[str, list[JobRun]] = {}
        self._lock = asyncio.Lock()

    async def append(self, run: JobRun) -> None:
        async with self._lock:
            self._rows.setdefault(run.job_id, []).append(run)

    async def list_for_job(self, job_id: str) -> list[JobRun]:
        async with self._lock:
            return list(self._rows.get(job_id, []))


# ---------------------------------------------------------------------------
# DeadLetterStore
# ---------------------------------------------------------------------------


class InMemoryDeadLetterStore:
    def __init__(self) -> None:
        self._rows: dict[str, dict[str, Any]] = {}
        self._lock = asyncio.Lock()

    async def move(
        self,
        *,
        job_id: str,
        last_run_id: str | None,
        reason: str,
        moved_at: datetime,
    ) -> None:
        async with self._lock:
            self._rows[job_id] = {
                "job_id": job_id,
                "last_run_id": last_run_id,
                "reason": reason,
                "moved_at": moved_at,
            }

    async def list(self, *, tenant_id: str | None = None) -> list[str]:
        # tenant filter requires joining with jobs; omitted for in-memory
        del tenant_id
        async with self._lock:
            return list(self._rows.keys())


# ---------------------------------------------------------------------------
# PolicyStore
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _PolicyRow:
    cfg: dict[str, Any]
    state_version: int


class InMemoryPolicyStore:
    def __init__(self) -> None:
        self._rows: dict[tuple[str, str | None, str], _PolicyRow] = {}
        self._lock = asyncio.Lock()

    async def get_active(
        self, *, tenant_id: str, task_id: str | None, kind: str
    ) -> dict[str, Any] | None:
        async with self._lock:
            row = self._rows.get((tenant_id, task_id, kind))
            return dict(row.cfg) if row is not None else None

    async def upsert(
        self,
        *,
        tenant_id: str,
        task_id: str | None,
        kind: str,
        cfg: dict[str, Any],
        expected_version: int | None,
    ) -> int:
        async with self._lock:
            key = (tenant_id, task_id, kind)
            existing = self._rows.get(key)
            if existing is None:
                if expected_version not in (None, 0):
                    raise StateVersionConflict("no existing policy to update")
                self._rows[key] = _PolicyRow(cfg=dict(cfg), state_version=1)
                return 1
            if expected_version is None:
                raise StateVersionConflict("expected_version required for update")
            _cas(expected_version, existing.state_version, f"policy {kind}")
            existing.cfg = dict(cfg)
            existing.state_version += 1
            return existing.state_version


# ---------------------------------------------------------------------------
# IdempotencyStore
# ---------------------------------------------------------------------------


class InMemoryIdempotencyStore:
    def __init__(self) -> None:
        self._rows: dict[str, dict[str, Any]] = {}
        self._lock = asyncio.Lock()

    async def remember(self, *, key: str, result: dict[str, Any], ttl_s: int) -> bool:
        del ttl_s  # no GC in this toy impl
        async with self._lock:
            if key in self._rows:
                return False
            self._rows[key] = dict(result)
            return True

    async def recall(self, key: str) -> dict[str, Any] | None:
        async with self._lock:
            row = self._rows.get(key)
            return dict(row) if row is not None else None


# ---------------------------------------------------------------------------
# TriggerStore
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class InMemoryTriggerRecord:
    trigger_id: str
    tenant_id: str
    task_id: str | None
    job_kind: JobKind
    spec_json: dict[str, Any]
    enabled: bool
    state_version: int
    debouncer_state: dict[str, Any]
    last_fired_at: datetime | None
    created_at: datetime
    updated_at: datetime

    def clone(self) -> InMemoryTriggerRecord:
        return InMemoryTriggerRecord(
            trigger_id=self.trigger_id,
            tenant_id=self.tenant_id,
            task_id=self.task_id,
            job_kind=self.job_kind,
            spec_json=dict(self.spec_json),
            enabled=self.enabled,
            state_version=self.state_version,
            debouncer_state=dict(self.debouncer_state),
            last_fired_at=self.last_fired_at,
            created_at=self.created_at,
            updated_at=self.updated_at,
        )


class InMemoryTriggerStore:
    def __init__(self) -> None:
        self._rows: dict[str, InMemoryTriggerRecord] = {}
        self._lock = asyncio.Lock()

    async def create(self, record: InMemoryTriggerRecord) -> InMemoryTriggerRecord:
        async with self._lock:
            if record.trigger_id in self._rows:
                raise ValueError(f"trigger_id {record.trigger_id} already exists")
            self._rows[record.trigger_id] = record.clone()
            return record.clone()

    async def get(self, trigger_id: str) -> InMemoryTriggerRecord:
        async with self._lock:
            row = self._rows.get(trigger_id)
            if row is None:
                raise TriggerNotFound(f"trigger {trigger_id} not found")
            return row.clone()

    async def list_enabled(self, *, tenant_id: str) -> list[InMemoryTriggerRecord]:
        async with self._lock:
            return [
                row.clone()
                for row in self._rows.values()
                if row.tenant_id == tenant_id and row.enabled
            ]

    async def update_state(
        self,
        *,
        trigger_id: str,
        debouncer_state: dict[str, Any],
        last_fired_at: datetime | None,
        expected_version: int,
    ) -> int:
        async with self._lock:
            row = self._rows.get(trigger_id)
            if row is None:
                raise TriggerNotFound(f"trigger {trigger_id} not found")
            _cas(expected_version, row.state_version, f"trigger {trigger_id}")
            row.debouncer_state = dict(debouncer_state)
            row.last_fired_at = last_fired_at
            row.state_version += 1
            return row.state_version

    async def set_enabled(
        self, *, trigger_id: str, enabled: bool, expected_version: int
    ) -> int:
        async with self._lock:
            row = self._rows.get(trigger_id)
            if row is None:
                raise TriggerNotFound(f"trigger {trigger_id} not found")
            _cas(expected_version, row.state_version, f"trigger {trigger_id}")
            row.enabled = enabled
            row.state_version += 1
            return row.state_version
