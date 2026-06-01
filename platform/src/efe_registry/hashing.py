"""Hashing helpers — kept minimal, mirrors :mod:`efe_gate.hashing`.

Three public helpers suffice for M06:

* :func:`content_hash` — canonical SHA-256 of a mapping (used for
  artifact manifest hash and for audit payload hashing).
* :func:`canonical_json_bytes` — canonical bytes used as input to
  other hash computations (contract_hash, NATS payload HMAC).
* :func:`sha256_file` — streaming SHA-256 of a file on disk (used at
  L3 to re-verify the ``model.safetensors`` blob).
* :func:`hmac_sign` / :func:`hmac_verify` — NATS event authentication.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from pathlib import Path
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


def sha256_file(path: Path, *, chunk_size: int = 1 << 20) -> bytes:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            buf = fh.read(chunk_size)
            if not buf:
                break
            h.update(buf)
    return h.digest()


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
    "sha256_file",
]
