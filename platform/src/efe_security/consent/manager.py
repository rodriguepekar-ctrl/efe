"""ConsentManager — CRUD + purpose binding."""

from __future__ import annotations

import asyncio
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import TYPE_CHECKING

from efe_security.consent.records import (
    ConsentAssertion,
    ConsentRecord,
    ConsentState,
    LegalBasis,
    build_consent_digest,
    seal_consent,
)
from efe_security.errors import (
    ConsentExpired,
    ConsentMissing,
    ConsentWithdrawn,
    PurposeNotDeclared,
)
from efe_security.fsm import CONSENT_ALLOWED, guard

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.consent.purpose import PurposeCatalog

__all__ = ["ConsentManager"]


@dataclass(slots=True)
class ConsentManager:
    purposes: "PurposeCatalog"
    _by_id: dict[str, ConsentRecord] = field(default_factory=dict)
    _by_subject: dict[tuple[str, str], list[str]] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def record(
        self,
        *,
        tenant_id: str,
        subject_id: str,
        purposes: frozenset[str],
        legal_basis: LegalBasis,
        consent_text: str,
        locale: str = "en-US",
        collected_by: str = "api",
        expires_at: datetime | None = None,
        now: datetime | None = None,
    ) -> ConsentRecord:
        for p in purposes:
            if not await self.purposes.exists(p):
                raise PurposeNotDeclared(p)
        now = now or datetime.now(tz=timezone.utc)
        consent_id = f"cnst_{uuid.uuid4().hex}"
        digest = build_consent_digest(consent_text)
        view = {
            "consent_id": consent_id,
            "tenant_id": tenant_id,
            "subject_id": subject_id,
            "purposes": sorted(purposes),
            "legal_basis": legal_basis.value,
            "consent_text_digest": digest.hex(),
            "locale": locale,
            "collected_at": now.astimezone(timezone.utc).isoformat(),
            "collected_by": collected_by,
            "expires_at": (
                expires_at.astimezone(timezone.utc).isoformat() if expires_at else None
            ),
        }
        seal = seal_consent(view)
        rec = ConsentRecord(
            consent_id=consent_id,
            tenant_id=tenant_id,
            subject_id=subject_id,
            purposes=purposes,
            legal_basis=legal_basis,
            consent_text_digest=digest,
            locale=locale,
            collected_at=now,
            collected_by=collected_by,
            expires_at=expires_at,
            withdrawn_at=None,
            seal=seal,
        )
        # FSM guard : nil → active
        guard("nil", ConsentState.active.value, CONSENT_ALLOWED, aggregate="consent")
        async with self._lock:
            self._by_id[consent_id] = rec
            self._by_subject.setdefault((tenant_id, subject_id), []).append(consent_id)
        return rec

    async def withdraw(
        self,
        *,
        consent_id: str,
        now: datetime | None = None,
    ) -> ConsentRecord:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            rec = self._by_id.get(consent_id)
            if rec is None:
                raise ConsentMissing(consent_id)
            guard(
                rec.state.value,
                ConsentState.withdrawn.value,
                CONSENT_ALLOWED,
                aggregate="consent",
            )
            new = ConsentRecord(
                consent_id=rec.consent_id,
                tenant_id=rec.tenant_id,
                subject_id=rec.subject_id,
                purposes=rec.purposes,
                legal_basis=rec.legal_basis,
                consent_text_digest=rec.consent_text_digest,
                locale=rec.locale,
                collected_at=rec.collected_at,
                collected_by=rec.collected_by,
                expires_at=rec.expires_at,
                withdrawn_at=now,
                seal=rec.seal,
                state=ConsentState.withdrawn,
            )
            self._by_id[consent_id] = new
            return new

    async def require(
        self,
        *,
        tenant_id: str,
        subject_id: str,
        purpose: str,
        now: datetime | None = None,
    ) -> ConsentAssertion:
        """Find an active consent that covers the ``purpose`` for the subject."""
        if not await self.purposes.exists(purpose):
            raise PurposeNotDeclared(purpose)
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            ids = list(self._by_subject.get((tenant_id, subject_id), []))
            candidates = [self._by_id[cid] for cid in ids]
        valid: ConsentRecord | None = None
        for rec in candidates:
            if purpose not in rec.purposes:
                continue
            if rec.state == ConsentState.withdrawn:
                raise ConsentWithdrawn(rec.consent_id)
            if rec.expires_at is not None and now >= rec.expires_at:
                continue
            valid = rec
            break
        if valid is None:
            raise ConsentMissing(f"{tenant_id}:{subject_id}:{purpose}")
        return ConsentAssertion(
            subject_id=subject_id,
            tenant_id=tenant_id,
            purpose=purpose,
            consent_id=valid.consent_id,
            asserted_at=now,
            legal_basis=valid.legal_basis,
        )

    async def get(self, consent_id: str) -> ConsentRecord:
        async with self._lock:
            rec = self._by_id.get(consent_id)
        if rec is None:
            raise ConsentMissing(consent_id)
        return rec

    async def list_for_subject(
        self, *, tenant_id: str, subject_id: str
    ) -> list[ConsentRecord]:
        async with self._lock:
            ids = list(self._by_subject.get((tenant_id, subject_id), []))
            return [self._by_id[cid] for cid in ids]
