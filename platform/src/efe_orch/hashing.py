"""SHA-256 helpers — used by the transition audit and pool keys.

Kept as a standalone module (not re-exporting from ``efe_ft.hashing``)
to keep the two packages independent. The implementations are
byte-for-byte compatible.
"""

from __future__ import annotations

import hashlib
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
    """Deterministic SHA-256 of a dict (keys sorted, UTF-8 JSON)."""
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


__all__ = ["content_hash", "sha256_bytes", "sha256_hex"]
