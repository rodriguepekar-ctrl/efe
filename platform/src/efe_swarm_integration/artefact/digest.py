"""SHA-256 digest helper."""

from __future__ import annotations

import hashlib
from typing import Iterable

__all__ = ["compute_sha256", "compute_sha256_stream"]


def compute_sha256(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def compute_sha256_stream(chunks: Iterable[bytes]) -> bytes:
    h = hashlib.sha256()
    for c in chunks:
        h.update(c)
    return h.digest()
