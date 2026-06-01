"""Audit store — append-only.

No ``UPDATE`` or ``DELETE`` by design ; the Protocol only exposes
``append`` + ``fetch_range`` + ``last_seal``.
"""

from __future__ import annotations

import asyncio
import hashlib
from dataclasses import dataclass, field
from datetime import datetime
from typing import Iterable, Protocol

from efe_security.audit.chain import GENESIS, seal_record
from efe_security.audit.records import AuditDraft, AuditRecord
from efe_security.errors import ChainBroken

__all__ = ["AuditStore", "InMemoryAuditStore"]


class AuditStore(Protocol):
    async def append(self, draft: AuditDraft) -> AuditRecord: ...  # pragma: no cover
    async def fetch_range(
        self,
        *,
        tenant_id: str | None = None,
        start: datetime | None = None,
        end: datetime | None = None,
        limit: int = 1000,
    ) -> list[AuditRecord]: ...  # pragma: no cover
    async def last_seal(self, *, tenant_id: str | None = None) -> bytes: ...  # pragma: no cover
    async def size(self) -> int: ...  # pragma: no cover


@dataclass(slots=True)
class InMemoryAuditStore:
    """Append-only in-memory store that maintains the seal chain."""

    _rows: list[AuditRecord] = field(default_factory=list)
    _payloads: dict[str, dict] = field(default_factory=dict)  # for verifier
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)
    _last_seal: bytes = GENESIS
    _key_version: int = 1
    _id_counter: int = 0

    def _next_id(self) -> str:
        self._id_counter += 1
        return f"aud_{self._id_counter:010d}"

    async def append(self, draft: AuditDraft) -> AuditRecord:
        async with self._lock:
            rec_id = self._next_id()
            record = seal_record(
                audit_id=rec_id,
                draft=draft,
                prev_seal=self._last_seal,
                key_version=self._key_version,
            )
            self._rows.append(record)
            self._payloads[record.audit_id] = dict(draft.payload)
            self._last_seal = record.seal
            return record

    async def fetch_range(
        self,
        *,
        tenant_id: str | None = None,
        start: datetime | None = None,
        end: datetime | None = None,
        limit: int = 1000,
    ) -> list[AuditRecord]:
        async with self._lock:
            rows = list(self._rows)
        out: list[AuditRecord] = []
        for r in rows:
            if tenant_id is not None and r.tenant_id != tenant_id:
                continue
            if start is not None and r.occurred_at < start:
                continue
            if end is not None and r.occurred_at > end:
                continue
            out.append(r)
            if len(out) >= limit:
                break
        return out

    async def last_seal(self, *, tenant_id: str | None = None) -> bytes:
        async with self._lock:
            return self._last_seal

    async def size(self) -> int:
        async with self._lock:
            return len(self._rows)

    async def get_payload(self, audit_id: str) -> dict:
        async with self._lock:
            return dict(self._payloads.get(audit_id, {}))


_ = Iterable, hashlib, ChainBroken
