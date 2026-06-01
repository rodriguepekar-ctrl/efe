"""Embedding client abstraction + deterministic hash-embedder for tests."""

from __future__ import annotations

import hashlib
import math
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Protocol


class Embedder(Protocol):
    dim: int
    model: str

    async def embed(
        self, texts: Sequence[str],
    ) -> list[list[float]]:  # pragma: no cover
        ...


@dataclass
class NullEmbedder:
    dim: int = 4
    model: str = "null"

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        return [[0.0] * self.dim for _ in texts]


class DeterministicEmbedder:
    """Test-only embedder.

    Maps a text to a float vector derived from SHA-256 so that :

    * same text → same vector (deterministic).
    * similar texts (same chars + trailing noise) → very close vectors
      (useful to exercise cosine threshold behaviour in tests).

    For production, inject an :class:`Embedder` that forwards to an
    embedding provider (OpenAI ``text-embedding-3-small``,
    ``sentence-transformers``, etc.).
    """

    def __init__(self, *, dim: int = 32) -> None:
        if dim < 2 or dim > 2048:
            raise ValueError("dim must be in [2, 2048]")
        self.dim = dim
        self.model = f"deterministic-{dim}"

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        return [self._one(t) for t in texts]

    def _one(self, text: str) -> list[float]:
        h = hashlib.sha256(text.encode("utf-8")).digest()
        # Expand deterministic bytes until we have enough floats.
        buf = b""
        seed = h
        while len(buf) < self.dim * 4:
            seed = hashlib.sha256(seed).digest()
            buf += seed
        # Interpret as int32 modulo → [-1, 1]
        raw = [
            int.from_bytes(buf[i * 4:(i + 1) * 4], "big", signed=True)
            for i in range(self.dim)
        ]
        max_abs = max(abs(x) for x in raw) or 1
        vec = [x / max_abs for x in raw]
        # L2 normalise for stable cosine.
        norm = math.sqrt(sum(v * v for v in vec)) or 1.0
        return [v / norm for v in vec]


__all__ = [
    "DeterministicEmbedder",
    "Embedder",
    "NullEmbedder",
]
