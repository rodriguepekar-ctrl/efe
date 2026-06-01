"""SHA-256 helpers + chunked streaming hashers.

The worker needs two kinds of hashing:

1. **Whole-blob hash** — to verify S3 downloads against the manifest.
   :func:`sha256_bytes` handles the synchronous case.
2. **Streaming hash** — when a shard is large we want to hash as we
   write to disk; :class:`StreamHasher` wraps a file-like that yields
   the digest on ``.hexdigest()``.
"""

from __future__ import annotations

import hashlib
from typing import Any


def sha256_bytes(blob: bytes) -> bytes:
    return hashlib.sha256(blob).digest()


def sha256_hex(blob: bytes | str) -> str:
    if isinstance(blob, str):
        return hashlib.sha256(blob.encode("utf-8")).hexdigest()
    return hashlib.sha256(blob).hexdigest()


def sha256_file(path: str, *, chunk: int = 1 << 20) -> str:
    """Hex digest of a file's contents."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            buf = f.read(chunk)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()


class StreamHasher:
    """Accumulate bytes into a running SHA-256."""

    __slots__ = ("_h", "_n")

    def __init__(self) -> None:
        self._h = hashlib.sha256()
        self._n = 0

    def update(self, chunk: bytes) -> None:
        self._h.update(chunk)
        self._n += len(chunk)

    def hexdigest(self) -> str:
        return self._h.hexdigest()

    def digest(self) -> bytes:
        return self._h.digest()

    @property
    def bytes_seen(self) -> int:
        return self._n


def verify_hex(expected_hex: str, observed_hex: str) -> bool:
    """Constant-time comparison of two hex digests."""
    if len(expected_hex) != len(observed_hex):
        return False
    diff = 0
    for a, b in zip(expected_hex.lower(), observed_hex.lower(), strict=False):
        diff |= ord(a) ^ ord(b)
    return diff == 0


def coerce_hex(value: Any) -> str:
    """Normalise an expected sha256 value to lowercase hex.

    Accepts ``str`` (hex), ``bytes`` (raw 32-byte digest), or
    :class:`bytearray`. Raises :class:`ValueError` on anything else.
    """
    if isinstance(value, str):
        if len(value) != 64:
            raise ValueError(
                f"sha256 hex must be 64 chars, got {len(value)}",
            )
        return value.lower()
    if isinstance(value, (bytes, bytearray)):
        if len(value) != 32:
            raise ValueError(
                f"sha256 bytes must be 32 long, got {len(value)}",
            )
        return bytes(value).hex()
    raise ValueError(f"unsupported sha256 input type: {type(value).__name__}")


__all__ = [
    "StreamHasher",
    "coerce_hex",
    "sha256_bytes",
    "sha256_file",
    "sha256_hex",
    "verify_hex",
]
