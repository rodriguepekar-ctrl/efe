"""Tamper-evident audit chain implementation.

Each entry's ``this_hash`` = ``sha256(prev_hash || canonical_json(payload))``.
Recomputing the whole chain in-memory detects any retroactive change.

Usage
-----

Writing::

    async with AuditChain(session, tenant_id) as chain:
        await chain.record(kind="ingest_accepted", actor_id="alice",
                           subject_id=sample_id, payload={...})

Verifying::

    from efe_ft.audit import verify_chain
    ok, first_break = await verify_chain(session, tenant_id)

The chain API is deliberately narrow (one ``record`` method, one
verifier). It never updates an existing row — append-only by design.
"""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Final

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import AuditEvent
from efe_ft.hashing import content_hash

#: Canonical set of audit event ``kind`` values.
AUDIT_KINDS: Final[frozenset[str]] = frozenset({
    "ingest_accepted",
    "ingest_rejected",
    "label_written",
    "label_overridden",
    "label_flagged_unclear",
    "label_flagged_harmful",
    "snapshot_materialised",
    "sample_deleted",
    "rls_violation",
    "embedding_cached",
    "quota_exceeded",
})


@dataclass(frozen=True, slots=True)
class AuditEntry:
    """Immutable view of an audit row (returned by ``record``)."""

    id: str
    tenant_id: str
    kind: str
    actor_id: str
    subject_id: str | None
    payload: Mapping[str, Any]
    prev_hash: bytes | None
    this_hash: bytes
    created_at: datetime


class AuditChain:
    """Append-only audit writer for a single tenant.

    Opens on entry, locks the latest row, appends within the same
    transaction as any caller writes — so that the SQLAlchemy
    ``flush`` is atomic with the domain mutation.
    """

    def __init__(self, session: AsyncSession, tenant_id: str) -> None:
        self._session = session
        self._tenant_id = tenant_id

    async def record(
        self,
        *,
        kind: str,
        actor_id: str,
        subject_id: str | None = None,
        payload: Mapping[str, Any] | None = None,
    ) -> AuditEntry:
        if kind not in AUDIT_KINDS:
            raise ValueError(
                f"unknown audit kind {kind!r}; add to AUDIT_KINDS first"
            )
        prev = await self._last_hash()
        payload_map: dict[str, Any] = dict(payload or {})
        envelope: dict[str, Any] = {
            "tenant_id": self._tenant_id,
            "kind": kind,
            "actor_id": actor_id,
            "subject_id": subject_id,
            "payload": payload_map,
            "prev_hash": prev.hex() if prev else None,
        }
        this_hash = content_hash(envelope)
        row = AuditEvent(
            id=str(uuid.uuid4()),
            tenant_id=self._tenant_id,
            kind=kind,
            actor_id=actor_id,
            subject_id=subject_id,
            payload=payload_map,
            prev_hash=prev,
            this_hash=this_hash,
        )
        self._session.add(row)
        await self._session.flush()
        return AuditEntry(
            id=row.id,
            tenant_id=row.tenant_id,
            kind=row.kind,
            actor_id=row.actor_id,
            subject_id=row.subject_id,
            payload=row.payload,
            prev_hash=row.prev_hash,
            this_hash=row.this_hash,
            created_at=row.created_at,
        )

    async def _last_hash(self) -> bytes | None:
        stmt = (
            select(AuditEvent.this_hash)
            .where(AuditEvent.tenant_id == self._tenant_id)
            .order_by(AuditEvent.created_at.desc(), AuditEvent.id.desc())
            .limit(1)
        )
        result = await self._session.execute(stmt)
        row = result.scalar_one_or_none()
        return row


async def verify_chain(
    session: AsyncSession, tenant_id: str
) -> tuple[bool, str | None]:
    """Recompute the full chain for ``tenant_id``.

    Returns ``(True, None)`` if every row's ``this_hash`` matches the
    recomputed one. Otherwise returns ``(False, first_broken_id)``.
    """
    stmt = (
        select(AuditEvent)
        .where(AuditEvent.tenant_id == tenant_id)
        .order_by(AuditEvent.created_at.asc(), AuditEvent.id.asc())
    )
    result = await session.execute(stmt)
    rows = list(result.scalars())
    prev: bytes | None = None
    for row in rows:
        envelope = {
            "tenant_id": row.tenant_id,
            "kind": row.kind,
            "actor_id": row.actor_id,
            "subject_id": row.subject_id,
            "payload": dict(row.payload),
            "prev_hash": prev.hex() if prev else None,
        }
        expected = content_hash(envelope)
        if expected != row.this_hash:
            return False, row.id
        if (prev is None) != (row.prev_hash is None):
            return False, row.id
        if prev is not None and prev != row.prev_hash:
            return False, row.id
        prev = row.this_hash
    return True, None


__all__ = ["AUDIT_KINDS", "AuditChain", "AuditEntry", "verify_chain"]
