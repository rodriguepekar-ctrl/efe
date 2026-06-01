"""Erasure service : soft-mark then hard-delete (cryptographic where possible)."""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.errors import ErasureBlocked

__all__ = ["ErasurePhase", "ErasureRecord", "ErasureService"]


class ErasurePhase(str, enum.Enum):
    requested = "requested"
    soft_marked = "soft_marked"
    hard_deleted = "hard_deleted"


@dataclass(frozen=True, slots=True)
class ErasureRecord:
    erasure_id: str
    tenant_id: str
    subject_id: str
    phase: ErasurePhase
    requested_at: datetime
    soft_marked_at: datetime | None = None
    hard_deleted_at: datetime | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "erasure_id": self.erasure_id,
            "tenant_id": self.tenant_id,
            "subject_id": self.subject_id,
            "phase": self.phase.value,
            "requested_at": self.requested_at.astimezone(timezone.utc).isoformat(),
            "soft_marked_at": (
                self.soft_marked_at.astimezone(timezone.utc).isoformat()
                if self.soft_marked_at
                else None
            ),
            "hard_deleted_at": (
                self.hard_deleted_at.astimezone(timezone.utc).isoformat()
                if self.hard_deleted_at
                else None
            ),
        }


@dataclass(slots=True)
class ErasureService:
    _records: dict[str, ErasureRecord] = field(default_factory=dict)
    _marked_subjects: set[tuple[str, str]] = field(default_factory=set)
    _locked_subjects: set[tuple[str, str]] = field(default_factory=set)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def request(
        self,
        *,
        erasure_id: str,
        tenant_id: str,
        subject_id: str,
        now: datetime | None = None,
    ) -> ErasureRecord:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            if (tenant_id, subject_id) in self._locked_subjects:
                raise ErasureBlocked(f"{tenant_id}:{subject_id}")
            rec = ErasureRecord(
                erasure_id=erasure_id,
                tenant_id=tenant_id,
                subject_id=subject_id,
                phase=ErasurePhase.requested,
                requested_at=now,
            )
            self._records[erasure_id] = rec
            return rec

    async def soft_mark(self, erasure_id: str, *, now: datetime | None = None) -> ErasureRecord:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            rec = self._records.get(erasure_id)
            if rec is None:
                raise ErasureBlocked(erasure_id)
            new = ErasureRecord(
                erasure_id=rec.erasure_id,
                tenant_id=rec.tenant_id,
                subject_id=rec.subject_id,
                phase=ErasurePhase.soft_marked,
                requested_at=rec.requested_at,
                soft_marked_at=now,
            )
            self._records[erasure_id] = new
            self._marked_subjects.add((rec.tenant_id, rec.subject_id))
            return new

    async def hard_delete(
        self, erasure_id: str, *, now: datetime | None = None
    ) -> ErasureRecord:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            rec = self._records.get(erasure_id)
            if rec is None:
                raise ErasureBlocked(erasure_id)
            new = ErasureRecord(
                erasure_id=rec.erasure_id,
                tenant_id=rec.tenant_id,
                subject_id=rec.subject_id,
                phase=ErasurePhase.hard_deleted,
                requested_at=rec.requested_at,
                soft_marked_at=rec.soft_marked_at,
                hard_deleted_at=now,
            )
            self._records[erasure_id] = new
            return new

    async def is_marked(self, *, tenant_id: str, subject_id: str) -> bool:
        async with self._lock:
            return (tenant_id, subject_id) in self._marked_subjects

    async def place_hold(self, *, tenant_id: str, subject_id: str) -> None:
        async with self._lock:
            self._locked_subjects.add((tenant_id, subject_id))

    async def release_hold(self, *, tenant_id: str, subject_id: str) -> None:
        async with self._lock:
            self._locked_subjects.discard((tenant_id, subject_id))

    async def list(self) -> list[ErasureRecord]:
        async with self._lock:
            return list(self._records.values())
