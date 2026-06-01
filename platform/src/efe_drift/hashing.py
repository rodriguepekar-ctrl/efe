"""Hashing primitives for :mod:`efe_drift`.

Three layers of hashing coexist :

1. :func:`content_hash` — canonical SHA-256 of a mapping. Used for
   the baseline ``spec_hash`` (invariant I4 / I2 integrity).

2. :func:`hmac_sign` / :func:`hmac_verify` — HMAC-SHA256 with a
   caller-supplied key **and** a mandatory ``domain_tag``. The domain
   tag isolates key reuse across sites so a signature on a NATS event
   cannot be replayed as a signature on a baseline seal (or vice
   versa).

3. :func:`hash_user_id` — salted SHA-256 of a user id, used for
   k-anonymising user identifiers before they hit histogram buckets.
   Zero PII guarantee (I6).

Domain tags :

* ``DOMAIN_TAG_BASELINE`` — for baseline seal signatures.
* ``DOMAIN_TAG_NATS`` — for NATS envelope signatures.
* ``DOMAIN_TAG_USER_ID`` — for user-id pseudonymisation.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from typing import Any

# ---------------------------------------------------------------------------
# Domain-separation tags
# ---------------------------------------------------------------------------

DOMAIN_TAG_BASELINE: bytes = b"efe-drift/baseline/v1\x00"
DOMAIN_TAG_NATS: bytes = b"efe-drift/nats/v1\x00"
DOMAIN_TAG_USER_ID: bytes = b"efe-drift/user-id/v1\x00"


# ---------------------------------------------------------------------------
# Canonical JSON + SHA-256
# ---------------------------------------------------------------------------


def canonical_json_bytes(payload: Any) -> bytes:
    """Deterministic JSON encoding — sorted keys, no whitespace, UTF-8.

    Values that aren't JSON-native are converted through
    :func:`_default_json` (datetimes become ISO-8601, bytes become hex,
    everything else becomes ``str(o)``).
    """
    return json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        default=_default_json,
    ).encode("utf-8")


def content_hash(payload: Mapping[str, Any]) -> bytes:
    """SHA-256 of the canonical JSON bytes of ``payload``."""
    return hashlib.sha256(canonical_json_bytes(payload)).digest()


# ---------------------------------------------------------------------------
# HMAC with explicit domain tag
# ---------------------------------------------------------------------------


def hmac_sign(
    message: bytes, key: bytes, *, domain_tag: bytes,
) -> bytes:
    """HMAC-SHA256 of ``domain_tag || message`` under ``key``."""
    if not isinstance(domain_tag, bytes) or not domain_tag:
        raise ValueError("domain_tag must be a non-empty byte string")
    mac = hmac.new(key, domain_tag + message, hashlib.sha256)
    return mac.digest()


def hmac_verify(
    message: bytes, key: bytes, signature: bytes, *, domain_tag: bytes,
) -> bool:
    """Constant-time verify of ``signature`` over ``message``.

    :func:`hmac.compare_digest` is used so an attacker cannot
    distinguish "wrong length" from "different byte by byte" via
    timing.
    """
    expected = hmac_sign(message, key, domain_tag=domain_tag)
    return hmac.compare_digest(expected, signature)


# ---------------------------------------------------------------------------
# User-id pseudonymisation
# ---------------------------------------------------------------------------


def hash_user_id(user_id: str, *, salt: bytes) -> str:
    """Return a hex-encoded SHA-256 of the salted user id.

    Used by the extraction plane to k-anonymise raw user identifiers
    **before** they reach any histogram or snapshot (invariant I6).
    Two separate tenants MUST use different salts to avoid
    cross-tenant correlation.
    """
    if not salt:
        raise ValueError("salt must be non-empty")
    h = hashlib.sha256(DOMAIN_TAG_USER_ID + salt + user_id.encode("utf-8"))
    return h.hexdigest()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _default_json(o: Any) -> Any:
    if hasattr(o, "isoformat"):
        return o.isoformat()
    if hasattr(o, "hex"):
        return o.hex()
    return str(o)


__all__ = [
    "DOMAIN_TAG_BASELINE",
    "DOMAIN_TAG_NATS",
    "DOMAIN_TAG_USER_ID",
    "canonical_json_bytes",
    "content_hash",
    "hash_user_id",
    "hmac_sign",
    "hmac_verify",
]
