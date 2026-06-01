"""Search layer for the audit trail (in-memory)."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import TYPE_CHECKING

from efe_security.audit.records import AuditRecord

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.audit.store import AuditStore

__all__ = ["AuditQuery", "InMemoryAuditSearch"]


@dataclass(frozen=True, slots=True)
class AuditQuery:
    tenant_id: str | None = None
    actor_id: str | None = None
    action_prefix: str | None = None
    start: datetime | None = None
    end: datetime | None = None
    limit: int = 100


@dataclass(slots=True)
class InMemoryAuditSearch:
    store: "AuditStore"

    async def query(self, q: AuditQuery) -> list[AuditRecord]:
        raw = await self.store.fetch_range(
            tenant_id=q.tenant_id, start=q.start, end=q.end, limit=q.limit * 10
        )
        filtered: list[AuditRecord] = []
        for r in raw:
            if q.actor_id is not None and r.actor_id != q.actor_id:
                continue
            if q.action_prefix is not None and not r.action.startswith(q.action_prefix):
                continue
            filtered.append(r)
            if len(filtered) >= q.limit:
                break
        return filtered
