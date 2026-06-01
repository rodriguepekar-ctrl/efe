"""Pseudonymisation helpers.

Two modes :

* :func:`pseudonymize` — HMAC with :const:`DOMAIN_TAG_USER_ID` ; keeps
  reversibility possible through a side-table protected by a dedicated
  scope (not implemented here, by design).
* :func:`pseudonymize_hashed` — one-way SHA-256 + tenant salt.
"""

from __future__ import annotations

import hashlib

from efe_security.crypto import (
    DOMAIN_TAG_PII_FIELD_HASH,
    DOMAIN_TAG_USER_ID,
    hmac_seal,
)

__all__ = ["pseudonymize", "pseudonymize_hashed"]


def pseudonymize(subject_id: str, *, tenant_id: str) -> str:
    """Deterministic, collision-resistant pseudonym keyed by
    ``(tenant, subject)`` with a dedicated domain tag."""
    mac = hmac_seal(
        DOMAIN_TAG_USER_ID,
        {"tenant_id": tenant_id, "subject_id": subject_id},
    )
    return mac.hex()


def pseudonymize_hashed(value: str, *, tenant_id: str, salt: bytes = b"") -> str:
    """One-way salted hash for field-level pseudonymisation (e.g. IP).

    Uses both a tenant-derived HMAC salt and an optional caller-provided
    salt for scope separation.
    """
    hmac_salt = hmac_seal(
        DOMAIN_TAG_PII_FIELD_HASH,
        {"tenant_id": tenant_id, "salt": salt.hex()},
    )
    return hashlib.sha256(hmac_salt + value.encode("utf-8")).hexdigest()
