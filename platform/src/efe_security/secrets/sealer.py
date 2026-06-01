"""Sealer — stable, domain-separated integrity signatures.

Abstracts the *which algorithm* / *which domain tag* decision for the
caller. For short-lived integrity checks, HMAC is cheapest ; for
publishable signatures, Ed25519 is used (when the crypto lib is
available).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal

from efe_security.crypto import (
    ALG_ED25519_V1,
    ALG_HMAC_SHA256_V1,
    DOMAIN_TAG_AUDIT_CHAIN,
    DOMAIN_TAG_CONSENT_SEAL,
    DOMAIN_TAG_DSAR_RECEIPT,
    DOMAIN_TAG_INCIDENT_SEAL,
    canonical_json,
    hmac_seal,
    verify_seal,
)
from efe_security.errors import CryptoError

__all__ = ["SealedPayload", "Sealer"]


SealPurpose = Literal["audit", "consent", "dsar", "incident"]


_TAG_FOR_PURPOSE: dict[str, bytes] = {
    "audit": DOMAIN_TAG_AUDIT_CHAIN,
    "consent": DOMAIN_TAG_CONSENT_SEAL,
    "dsar": DOMAIN_TAG_DSAR_RECEIPT,
    "incident": DOMAIN_TAG_INCIDENT_SEAL,
}


@dataclass(frozen=True, slots=True)
class SealedPayload:
    purpose: str
    alg_id: str
    payload_bytes: bytes
    signature: bytes

    def to_dict(self) -> dict[str, Any]:
        return {
            "purpose": self.purpose,
            "alg_id": self.alg_id,
            "payload": self.payload_bytes.hex(),
            "signature": self.signature.hex(),
        }


@dataclass(slots=True)
class Sealer:
    """Default sealer — uses HMAC for fast in-cluster integrity."""

    alg_id: str = ALG_HMAC_SHA256_V1

    def seal(self, payload: Any, *, purpose: SealPurpose) -> SealedPayload:
        tag = _TAG_FOR_PURPOSE.get(purpose)
        if tag is None:
            raise CryptoError(f"unknown seal purpose: {purpose}")
        raw = canonical_json(payload)
        if self.alg_id == ALG_HMAC_SHA256_V1:
            sig = hmac_seal(tag, payload)
        elif self.alg_id == ALG_ED25519_V1:
            # Ed25519 support is wired through DefaultCryptoProvider when
            # a signing key is loaded ; here we leave a clear hook.
            raise CryptoError(
                "Ed25519 sealing requires a DefaultCryptoProvider bound to a key"
            )
        else:
            raise CryptoError(f"unsupported seal alg_id: {self.alg_id}")
        return SealedPayload(
            purpose=purpose, alg_id=self.alg_id, payload_bytes=raw, signature=sig
        )

    def verify(self, sealed: SealedPayload, *, payload: Any) -> bool:
        tag = _TAG_FOR_PURPOSE.get(sealed.purpose)
        if tag is None:
            return False
        if sealed.alg_id != ALG_HMAC_SHA256_V1:
            return False  # for now
        return verify_seal(tag, payload, sealed.signature)
