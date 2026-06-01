"""Protocols for storage backends.

The engine, triggers and workers depend on these Protocols only. Concrete
implementations (in-memory, SQL) live in :mod:`efe_scheduling.storage`.

Every mutating verb requires ``expected_version`` and returns the new
version on success — or raises :class:`StateVersionConflict` (CAS).
"""

from __future__ import annotations

from datetime import datetime, timedelta
from typing import Any, Protocol

from efe_scheduling.types import (
    JobKind,
    JobRun,
    JobSpec,
    JobState,
    Schedule,
    ScheduleState,
)

__all__ = [
    "DeadLetterStore",
    "IdempotencyStore",
    "JobStore",
    "PolicyStore",
    "RunStore",
    "ScheduleStore",
    "TriggerRecord",
    "TriggerStore",
]


class ScheduleStore(Protocol):
    async def create(self, schedule: Schedule) -> Schedule: ...
    async def get(self, schedule_id: str) -> Schedule: ...
    async def list_enabled(
        self, *, tenant_id: str, evaluated_before: datetime
    ) -> list[Schedule]: ...
    async def mark_evaluated(
        self, *, schedule_id: str, until: datetime, expected_version: int
    ) -> int: ...
    async def transition(
        self,
        *,
        schedule_id: str,
        to_state: ScheduleState,
        expected_version: int,
        now: datetime,
    ) -> int: ...


class JobStore(Protocol):
    async def enqueue(self, spec: JobSpec) -> None:
        """Insert a new job. Raise :class:`DuplicateFireKey` on conflict."""

    async def get(self, job_id: str) -> JobSpec: ...
    async def lease_next(
        self, *, worker_id: str, tenant_id: str | None = None, lease_ttl: timedelta
    ) -> JobSpec | None: ...
    async def extend_lease(
        self, *, job_id: str, new_ttl: timedelta, expected_version: int
    ) -> int: ...
    async def transition(
        self,
        *,
        job_id: str,
        to_state: JobState,
        expected_version: int,
        fields: dict[str, Any],
    ) -> int: ...
    async def list_by_state(
        self, *, state: JobState, tenant_id: str | None = None, limit: int = 100
    ) -> list[JobSpec]: ...
    async def current_state(self, job_id: str) -> JobState: ...
    async def current_version(self, job_id: str) -> int: ...
    async def attempts_left(self, job_id: str) -> int:
        """Return the number of attempts remaining for ``job_id``.

        A freshly-enqueued job returns ``spec.max_attempts``.
        After each :meth:`lease_next`, the value decreases by exactly one.
        Reaches ``0`` when no further retry is permitted.
        """


class RunStore(Protocol):
    async def append(self, run: JobRun) -> None: ...
    async def list_for_job(self, job_id: str) -> list[JobRun]: ...


class DeadLetterStore(Protocol):
    async def move(
        self, *, job_id: str, last_run_id: str | None, reason: str, moved_at: datetime
    ) -> None: ...
    async def list(self, *, tenant_id: str | None = None) -> list[str]: ...


class PolicyStore(Protocol):
    async def get_active(
        self, *, tenant_id: str, task_id: str | None, kind: str
    ) -> dict[str, Any] | None: ...
    async def upsert(
        self,
        *,
        tenant_id: str,
        task_id: str | None,
        kind: str,
        cfg: dict[str, Any],
        expected_version: int | None,
    ) -> int: ...


class IdempotencyStore(Protocol):
    async def remember(
        self, *, key: str, result: dict[str, Any], ttl_s: int
    ) -> bool:
        """Return True if the key was new. False on replay."""

    async def recall(self, key: str) -> dict[str, Any] | None: ...


class TriggerRecord(Protocol):
    """Shape of a persisted trigger (minimal view for the store)."""

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


class TriggerStore(Protocol):
    async def create(self, record: TriggerRecord) -> TriggerRecord: ...
    async def get(self, trigger_id: str) -> TriggerRecord: ...
    async def list_enabled(self, *, tenant_id: str) -> list[TriggerRecord]: ...
    async def update_state(
        self,
        *,
        trigger_id: str,
        debouncer_state: dict[str, Any],
        last_fired_at: datetime | None,
        expected_version: int,
    ) -> int: ...
    async def set_enabled(
        self, *, trigger_id: str, enabled: bool, expected_version: int
    ) -> int: ...
