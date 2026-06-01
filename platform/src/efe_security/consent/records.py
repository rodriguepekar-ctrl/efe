"""Consent data model."""

from __future__ import annotations

import enum
import hashlib
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.crypto import DOMAIN_TAG_CONSENT_SEAL, canonical_json, hmac_seal

__all__ = [
    "ConsentAssertion",
    "ConsentRecord",
    "ConsentState",
    "LegalBasis",
]


class LegalBasis(str, enum.Enum):
    consent = "consent"
    contract = "contract"
    legal_obligation = "legal_obligation"
    vital_interests = "vital_interests"
    public_task = "public_task"
    legitimate_interests = "legitimate_interests"


class ConsentState(str, enum.Enum):
    active = "active"
    withdrawn = "withdrawn"
    expired = "expired"


@dataclass(frozen=True, slots=True)
class ConsentRecord:
    consent_id: str
    tenant_id: str
    subject_id: str
    purposes: frozenset[str]
    legal_basis: LegalBasis
    consent_text_digest: bytes
    locale: str
    collected_at: datetime
    collected_by: str
    expires_at: datetime | None
    withdrawn_at: datetime | None
    seal: bytes
    state: ConsentState = ConsentState.active

    def __post_init__(self) -> None:
        if self.collected_at.tzinfo is None:
            raise ValueError("collected_at must be tz-aware")
        if self.expires_at is not None and self.expires_at.tzinfo is None:
            raise ValueError("expires_at must be tz-aware")
        if self.withdrawn_at is not None and self.withdrawn_at.tzinfo is None:
            raise ValueError("withdrawn_at must be tz-aware")

    def is_valid(self, *, now: datetime) -> bool:
        if self.state != ConsentState.active:
            return False
        if self.expires_at is not None and now >= self.expires_at:
            return False
        return True

    def to_dict(self) -> dict[str, Any]:
        return {
            "consent_id": self.consent_id,
            "tenant_id": self.tenant_id,
            "subject_id": self.subject_id,
            "purposes": sorted(self.purposes),
            "legal_basis": self.legal_basis.value,
            "consent_text_digest": self.consent_text_digest.hex(),
            "locale": self.locale,
            "collected_at": self.collected_at.astimezone(timezone.utc).isoformat(),
            "collected_by": self.collected_by,
            "expires_at": (
                self.expires_at.astimezone(timezone.utc).isoformat()
                if self.expires_at
                else None
            ),
            "withdrawn_at": (
                self.withdrawn_at.astimezone(timezone.utc).isoformat()
                if self.withdrawn_at
                else None
            ),
            "state": self.state.value,
        }


def build_consent_digest(text: str) -> bytes:
    return hashlib.sha256(text.encode("utf-8")).digest()


def seal_consent(record_view: dict[str, Any]) -> bytes:
    return hmac_seal(DOMAIN_TAG_CONSENT_SEAL, record_view)


@dataclass(frozen=True, slots=True)
class ConsentAssertion:
    """Opaque proof that consent was checked for a given operation."""

    subject_id: str
    tenant_id: str
    purpose: str
    consent_id: str
    asserted_at: datetime
    legal_basis: LegalBasis

    def to_dict(self) -> dict[str, Any]:
        return {
            "subject_id": self.subject_id,
            "tenant_id": self.tenant_id,
            "purpose": self.purpose,
            "consent_id": self.consent_id,
            "asserted_at": self.asserted_at.astimezone(timezone.utc).isoformat(),
            "legal_basis": self.legal_basis.value,
        }


_ = canonical_json  # keep
