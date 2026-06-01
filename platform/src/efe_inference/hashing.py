"""Hashing helpers — SHA-256, HMAC-SHA256, canonical JSON bytes.

Behaviour matches :mod:`efe_registry.hashing` so audit chains emitted
from M07 can be verified by the same :class:`efe_gate.champion.audit.AuditChain`
downstream.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from typing import Any


def canonical_json_bytes(payload: Any) -> bytes:
    return json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        default=_default_json,
    ).encode("utf-8")


def content_hash(payload: Mapping[str, Any]) -> bytes:
    return hashlib.sha256(canonical_json_bytes(payload)).digest()


def sha256_text(text: str) -> bytes:
    return hashlib.sha256(text.encode("utf-8")).digest()


def sha256_hex(text: str | bytes) -> str:
    if isinstance(text, str):
        return hashlib.sha256(text.encode("utf-8")).hexdigest()
    return hashlib.sha256(text).hexdigest()


def hmac_sign(message: bytes, key: bytes) -> bytes:
    return hmac.new(key, message, hashlib.sha256).digest()


def hmac_verify(message: bytes, key: bytes, signature: bytes) -> bool:
    expected = hmac_sign(message, key)
    return hmac.compare_digest(expected, signature)


def _default_json(o: Any) -> Any:
    if hasattr(o, "isoformat"):
        return o.isoformat()
    if hasattr(o, "hex"):
        return o.hex()
    return str(o)


__all__ = [
    "canonical_json_bytes",
    "content_hash",
    "hmac_sign",
    "hmac_verify",
    "sha256_hex",
    "sha256_text",
]
