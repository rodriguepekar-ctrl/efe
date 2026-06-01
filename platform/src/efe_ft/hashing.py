"""Stable hashing helpers — used for dedup, content_hash, audit chain.

All hashes are SHA-256 by default. Inputs are normalised so that
equivalent logical content produces identical bytes: Unicode
normalised to NFKC, whitespace trimmed, newlines canonicalised.
"""

from __future__ import annotations

import hashlib
import json
import unicodedata
from collections.abc import Iterable, Mapping
from typing import Any


def normalise_text(text: str) -> str:
    """NFKC-normalise + strip + collapse whitespace (but not case)."""
    s = unicodedata.normalize("NFKC", text)
    s = s.replace("\r\n", "\n").replace("\r", "\n")
    return s.strip()


def sha256_text(text: str) -> bytes:
    """Return the SHA-256 of ``normalise_text(text)`` as 32 raw bytes."""
    return hashlib.sha256(normalise_text(text).encode("utf-8")).digest()


def sha256_bytes(blob: bytes) -> bytes:
    return hashlib.sha256(blob).digest()


def sha256_hex(blob: bytes | str) -> str:
    if isinstance(blob, str):
        return hashlib.sha256(blob.encode("utf-8")).hexdigest()
    return hashlib.sha256(blob).hexdigest()


def content_hash(payload: Mapping[str, Any]) -> bytes:
    """Deterministic hash of a dict: keys are sorted before serialisation.

    Unlike :func:`sha256_text`, this one hashes JSON-serialisable
    structures so the same logical dict always produces the same bytes,
    regardless of insertion order.
    """
    canonical = json.dumps(
        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
        default=_default_json,
    )
    return hashlib.sha256(canonical.encode("utf-8")).digest()


def content_hash_iter(ids: Iterable[str]) -> bytes:
    """SHA-256 of sorted UUID/ID list (used for snapshot content_hash)."""
    sorted_ids = sorted(ids)
    joined = "\n".join(sorted_ids).encode("utf-8")
    return hashlib.sha256(joined).digest()


def _default_json(o: Any) -> Any:
    if hasattr(o, "isoformat"):
        return o.isoformat()
    if hasattr(o, "hex"):
        return o.hex()
    return str(o)


__all__ = [
    "content_hash",
    "content_hash_iter",
    "normalise_text",
    "sha256_bytes",
    "sha256_hex",
    "sha256_text",
]
