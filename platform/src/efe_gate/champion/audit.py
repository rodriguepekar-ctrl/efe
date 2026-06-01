"""Signed audit chain — append-only, per-tenant, HMAC-signed.

Each :class:`AuditEvent` row carries:

* ``payload``   — canonical JSON of the decision envelope
* ``prev_hash`` — pointer to the previous tenant-local entry
* ``this_hash`` — ``sha256(prev_hash || canonical_json(envelope))``
* ``signature`` — ``HMAC-SHA256(this_hash)`` using the signing key

Chain integrity is verified by :func:`verify_chain` which replays
hashing from the first row and compares.
"""

from __future__ import annotations

import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from enum import Enum
from typing import Any, Final

from sqlalchemy import func, select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import Clock, SystemClock
from efe_gate.config import SignatureSettings
from efe_gate.db.models import AuditEvent
from efe_gate.errors import (
    ChainIntegrityBroken,
    SignatureError,
    ValidationError,
)
from efe_gate.hashing import content_hash, hmac_sign, hmac_verify


class AuditKind(str, Enum):
    DECISION_PROMOTE = "decision_promote"
    DECISION_REJECT = "decision_reject"
    DECISION_REVIEW = "decision_review"
    REVIEW_OVERRIDE_APPROVE = "review_override_approve"
    REVIEW_OVERRIDE_REJECT = "review_override_reject"
    ROLLBACK = "rollback"
    GATE_UPDATED = "gate_updated"
    GOLDEN_DEPRECATED = "golden_deprecated"


KNOWN_KINDS: Final[frozenset[str]] = frozenset(k.value for k in AuditKind)


@dataclass(frozen=True, slots=True)
class AuditEntry:
    id: str
    tenant_id: str
    seq: int
    kind: str
    actor: str
    subject_id: str | None
    payload: Mapping[str, Any]
    prev_hash: bytes | None
    this_hash: bytes
    signature: bytes | None
    signing_key_id: str | None
    created_at: datetime

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "tenant_id": self.tenant_id,
            "seq": self.seq,
            "kind": self.kind,
            "actor": self.actor,
            "subject_id": self.subject_id,
            "payload": dict(self.payload),
            "prev_hash": self.prev_hash.hex() if self.prev_hash else None,
            "this_hash": self.this_hash.hex(),
            "signature": self.signature.hex() if self.signature else None,
            "signing_key_id": self.signing_key_id,
            "created_at": self.created_at.isoformat(),
        }


#: Callable used by :func:`verify_chain` to resolve a signing key by id.
#: Returns the 32-byte key or ``None`` when the key is unknown.
KeyResolver = Callable[[str | None], bytes | None]


class AuditChain:
    """Append-only, signed audit writer for a single tenant.

    Identical hashing scheme to :mod:`efe_ft.audit.chain`; this
    version additionally stores an HMAC signature over ``this_hash``
    so downstream consumers can verify authenticity without a DB
    round-trip, and a ``signing_key_id`` for key rotation.
    """

    def __init__(
        self,
        session: AsyncSession,
        tenant_id: str,
        *,
        signing: SignatureSettings,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._tenant_id = tenant_id
        self._signing = signing
        try:
            self._key = bytes.fromhex(signing.key_hex)
        except ValueError as exc:
            raise SignatureError(
                f"invalid signing key: {exc}",
            ) from exc
        self._clock: Clock = clock or SystemClock()

    async def record(
        self,
        *,
        kind: AuditKind | str,
        actor: str,
        subject_id: str | None = None,
        payload: Mapping[str, Any] | None = None,
    ) -> AuditEntry:
        kind_value = kind.value if isinstance(kind, AuditKind) else str(kind)
        if kind_value not in KNOWN_KINDS:
            raise ValidationError(
                f"unknown audit kind {kind_value!r}; "
                f"extend AuditKind first",
            )
        if not actor:
            raise ValidationError("actor is required")
        # Compute the next per-tenant sequence number under the lock
        # implied by the enclosing DB transaction. The UNIQUE
        # constraint on (tenant_id, seq) is the last-line guard — two
        # concurrent writers that see the same ``max(seq)`` will
        # conflict on COMMIT rather than silently corrupt the chain.
        next_seq = await self._next_seq()
        prev = await self._last_hash()
        payload_map: dict[str, Any] = dict(payload or {})
        envelope: dict[str, Any] = {
            "tenant_id": self._tenant_id,
            "seq": next_seq,
            "kind": kind_value,
            "actor": actor,
            "subject_id": subject_id,
            "payload": payload_map,
            "prev_hash": prev.hex() if prev else None,
        }
        this_hash = content_hash(envelope)
        signature = hmac_sign(this_hash, self._key)
        row = AuditEvent(
            id=str(uuid.uuid4()),
            tenant_id=self._tenant_id,
            seq=next_seq,
            kind=kind_value,
            actor=actor,
            subject_id=subject_id,
            payload=payload_map,
            prev_hash=prev,
            this_hash=this_hash,
            signature=signature,
            signing_key_id=self._signing.key_id,
            created_at=self._clock.now(),
        )
        self._session.add(row)
        await self._session.flush()
        return AuditEntry(
            id=row.id, tenant_id=row.tenant_id, seq=row.seq,
            kind=row.kind, actor=row.actor, subject_id=row.subject_id,
            payload=row.payload, prev_hash=row.prev_hash,
            this_hash=row.this_hash, signature=row.signature,
            signing_key_id=row.signing_key_id,
            created_at=row.created_at,
        )

    async def _next_seq(self) -> int:
        stmt = (
            select(func.coalesce(func.max(AuditEvent.seq), 0))
            .where(AuditEvent.tenant_id == self._tenant_id)
        )
        current = (await self._session.execute(stmt)).scalar_one()
        return int(current) + 1

    async def _last_hash(self) -> bytes | None:
        # Ordering by ``seq DESC`` is deterministic (UNIQUE constraint)
        # and always reflects the real chain tail.
        stmt = (
            select(AuditEvent.this_hash)
            .where(AuditEvent.tenant_id == self._tenant_id)
            .order_by(AuditEvent.seq.desc())
            .limit(1)
        )
        return (await self._session.execute(stmt)).scalar_one_or_none()


async def verify_chain(
    session: AsyncSession, tenant_id: str,
    *,
    signing: SignatureSettings | None = None,
    key_resolver: KeyResolver | None = None,
) -> tuple[bool, str | None]:
    """Recompute hashes + optionally verify signatures.

    Parameters
    ----------
    signing
        Default key used to verify events whose ``signing_key_id``
        matches ``signing.key_id`` (or when no key resolver is given).
    key_resolver
        Optional callable ``signing_key_id → key_bytes | None`` used
        to support key rotation. When supplied, each row is verified
        with the key returned for its own ``signing_key_id``. A row
        whose key cannot be resolved is treated as a verification
        failure (:class:`ChainIntegrityBroken`).

    Ordering uses the dedicated ``seq`` column (``UNIQUE per tenant``)
    rather than ``created_at``, so two events recorded in the same
    microsecond still replay in the exact order they were written.
    """
    stmt = (
        select(AuditEvent)
        .where(AuditEvent.tenant_id == tenant_id)
        .order_by(AuditEvent.seq.asc())
    )
    rows = list((await session.execute(stmt)).scalars())
    prev: bytes | None = None

    def _key_for(row: AuditEvent) -> bytes | None:
        if key_resolver is not None:
            return key_resolver(row.signing_key_id)
        if signing is None:
            return None
        if (
            row.signing_key_id is None
            or row.signing_key_id == signing.key_id
        ):
            return bytes.fromhex(signing.key_hex)
        # Mismatch without a resolver — refuse to silently skip.
        return None

    for row in rows:
        envelope = {
            "tenant_id": row.tenant_id,
            "seq": row.seq,
            "kind": row.kind,
            "actor": row.actor,
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
        if signing is not None or key_resolver is not None:
            key = _key_for(row)
            if key is None:
                raise ChainIntegrityBroken(
                    f"no signing key for event {row.id!r} "
                    f"(key_id={row.signing_key_id!r})",
                )
            if (
                row.signature is not None
                and not hmac_verify(row.this_hash, key, row.signature)
            ):
                raise ChainIntegrityBroken(
                    f"signature mismatch at event {row.id!r}",
                )
        prev = row.this_hash
    return True, None


__all__ = [
    "KNOWN_KINDS",
    "AuditChain",
    "AuditEntry",
    "AuditKind",
    "KeyResolver",
    "verify_chain",
]
