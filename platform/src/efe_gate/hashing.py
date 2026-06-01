"""Hashing + content-hash helpers — same semantics as the rest of the platform.

Three functions dominate this module:

* :func:`content_hash` — deterministic SHA-256 of a dict (sorted keys,
  canonical JSON). Used for the audit chain and golden-set locking.
* :func:`sha256_bytes` — raw bytes → digest. Used for shard checks.
* :func:`hmac_sign` — lightweight signing for decisions when no KMS
  is available. Uses an HMAC-SHA256 with a server-side secret. In
  production the signing key is injected via K8s Secret; for tests
  we use a fixed key.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from typing import Any


def sha256_bytes(blob: bytes) -> bytes:
    return hashlib.sha256(blob).digest()


def sha256_hex(blob: bytes | str) -> str:
    if isinstance(blob, str):
        return hashlib.sha256(blob.encode("utf-8")).hexdigest()
    return hashlib.sha256(blob).hexdigest()


def content_hash(payload: Mapping[str, Any]) -> bytes:
    """Deterministic SHA-256 of a JSON-serialisable dict."""
    canonical = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        default=_default_json,
    )
    return hashlib.sha256(canonical.encode("utf-8")).digest()


def _default_json(o: Any) -> Any:
    if hasattr(o, "isoformat"):
        return o.isoformat()
    if hasattr(o, "hex"):
        return o.hex()
    return str(o)


def hmac_sign(message: bytes, key: bytes) -> bytes:
    """Compute HMAC-SHA256 of ``message`` under ``key``.

    Returned digest is 32 raw bytes (hex-encode before storing as
    text). Constant-time compare via :func:`hmac.compare_digest` is
    provided as :func:`hmac_verify`.
    """
    return hmac.new(key, message, hashlib.sha256).digest()


def hmac_verify(message: bytes, key: bytes, signature: bytes) -> bool:
    expected = hmac_sign(message, key)
    return hmac.compare_digest(expected, signature)


__all__ = [
    "content_hash",
    "hmac_sign",
    "hmac_verify",
    "sha256_bytes",
    "sha256_hex",
]
