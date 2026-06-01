"""DSAR pipeline — access / rectify / erase / portability."""

from __future__ import annotations

import asyncio
import enum
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.crypto import DOMAIN_TAG_DSAR_RECEIPT, hmac_seal
from efe_security.errors import LegalHoldInForce, RequestAlreadyProcessed, SubjectNotFound
from efe_security.fsm import DSAR_ALLOWED, guard

__all__ = [
    "DsarKind",
    "DsarRequest",
    "DsarService",
    "DsarState",
]


class DsarKind(str, enum.Enum):
    access = "access"
    rectify = "rectify"
    erase = "erase"
    portability = "portability"


class DsarState(str, enum.Enum):
    received = "received"
    in_progress = "in_progress"
    completed = "completed"
    rejected = "rejected"


@dataclass(frozen=True, slots=True)
class DsarRequest:
    request_id: str
    tenant_id: str
    subject_id: str
    kind: DsarKind
    state: DsarState
    submitted_at: datetime
    completed_at: datetime | None
    receipt_seal: bytes | None
    notes: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "request_id": self.request_id,
            "tenant_id": self.tenant_id,
            "subject_id": self.subject_id,
            "kind": self.kind.value,
            "state": self.state.value,
            "submitted_at": self.submitted_at.astimezone(timezone.utc).isoformat(),
            "completed_at": (
                self.completed_at.astimezone(timezone.utc).isoformat()
                if self.completed_at
                else None
            ),
            "receipt_seal": self.receipt_seal.hex() if self.receipt_seal else None,
            "notes": self.notes,
        }


@dataclass(slots=True)
class DsarService:
    """In-memory DSAR pipeline. Production writes to Postgres."""

    _rows: dict[str, DsarRequest] = field(default_factory=dict)
    _legal_hold: set[tuple[str, str]] = field(default_factory=set)  # (tenant, subject)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def submit(
        self,
        *,
        tenant_id: str,
        subject_id: str,
        kind: DsarKind,
        now: datetime | None = None,
    ) -> DsarRequest:
        now = now or datetime.now(tz=timezone.utc)
        rid = f"dsar_{uuid.uuid4().hex}"
        req = DsarRequest(
            request_id=rid,
            tenant_id=tenant_id,
            subject_id=subject_id,
            kind=kind,
            state=DsarState.received,
            submitted_at=now,
            completed_at=None,
            receipt_seal=None,
        )
        async with self._lock:
            self._rows[rid] = req
        return req

    async def start(self, request_id: str, *, now: datetime | None = None) -> DsarRequest:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            req = self._rows.get(request_id)
            if req is None:
                raise SubjectNotFound(request_id)
            if req.state != DsarState.received:
                raise RequestAlreadyProcessed(request_id)
            if (req.tenant_id, req.subject_id) in self._legal_hold:
                raise LegalHoldInForce(f"{req.tenant_id}:{req.subject_id}")
            guard(req.state.value, DsarState.in_progress.value, DSAR_ALLOWED, aggregate="dsar")
            new = DsarRequest(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                subject_id=req.subject_id,
                kind=req.kind,
                state=DsarState.in_progress,
                submitted_at=req.submitted_at,
                completed_at=None,
                receipt_seal=None,
            )
            self._rows[request_id] = new
            return new

    async def complete(
        self,
        request_id: str,
        *,
        notes: str = "",
        now: datetime | None = None,
    ) -> DsarRequest:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            req = self._rows.get(request_id)
            if req is None:
                raise SubjectNotFound(request_id)
            guard(req.state.value, DsarState.completed.value, DSAR_ALLOWED, aggregate="dsar")
            view = {
                "request_id": req.request_id,
                "tenant_id": req.tenant_id,
                "subject_id": req.subject_id,
                "kind": req.kind.value,
                "completed_at": now.astimezone(timezone.utc).isoformat(),
                "notes": notes,
            }
            seal = hmac_seal(DOMAIN_TAG_DSAR_RECEIPT, view)
            new = DsarRequest(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                subject_id=req.subject_id,
                kind=req.kind,
                state=DsarState.completed,
                submitted_at=req.submitted_at,
                completed_at=now,
                receipt_seal=seal,
                notes=notes,
            )
            self._rows[request_id] = new
            return new

    async def reject(
        self,
        request_id: str,
        *,
        reason: str,
        now: datetime | None = None,
    ) -> DsarRequest:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            req = self._rows.get(request_id)
            if req is None:
                raise SubjectNotFound(request_id)
            guard(req.state.value, DsarState.rejected.value, DSAR_ALLOWED, aggregate="dsar")
            new = DsarRequest(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                subject_id=req.subject_id,
                kind=req.kind,
                state=DsarState.rejected,
                submitted_at=req.submitted_at,
                completed_at=now,
                receipt_seal=None,
                notes=reason,
            )
            self._rows[request_id] = new
            return new

    # ------------------------------------------------------------------
    # Legal hold
    # ------------------------------------------------------------------

    async def place_legal_hold(self, *, tenant_id: str, subject_id: str) -> None:
        async with self._lock:
            self._legal_hold.add((tenant_id, subject_id))

    async def release_legal_hold(self, *, tenant_id: str, subject_id: str) -> None:
        async with self._lock:
            self._legal_hold.discard((tenant_id, subject_id))

    async def list(
        self, *, tenant_id: str | None = None, state: DsarState | None = None
    ) -> list[DsarRequest]:
        async with self._lock:
            return [
                r
                for r in self._rows.values()
                if (tenant_id is None or r.tenant_id == tenant_id)
                and (state is None or r.state == state)
            ]
