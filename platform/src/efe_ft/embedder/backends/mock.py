"""Deterministic mock embedder — pseudo-random seeded by sha256(text).

Good enough for tests, demos, and the on-device benchmark harness.
Zero external dependency.
"""

from __future__ import annotations

import hashlib

import numpy as np


class MockEmbedder:
    """Fast, deterministic pseudo-embedder.

    For each input ``text`` we seed a PRNG from ``sha256(text)`` and
    draw a vector from N(0, 1), L2-normalised. Distinct texts produce
    nearly-orthogonal embeddings, distinct renderings of the same text
    (after NFKC normalisation) produce identical embeddings.
    """

    def __init__(self, dim: int = 128, version: str = "mock-ff01a2") -> None:
        if dim < 8:
            raise ValueError("dim must be >= 8")
        self._dim = dim
        self._version = version

    @property
    def dim(self) -> int:
        return self._dim

    @property
    def version(self) -> str:
        return self._version

    def embed(self, text: str) -> np.ndarray:
        h = hashlib.sha256(text.encode("utf-8")).digest()
        seed = int.from_bytes(h[:8], "little")
        rng = np.random.default_rng(seed)
        v = rng.standard_normal(self._dim).astype(np.float32)
        n = np.linalg.norm(v)
        return v if n == 0 else v / n

    def embed_batch(self, texts: list[str]) -> np.ndarray:
        return np.stack([self.embed(t) for t in texts], axis=0)


__all__ = ["MockEmbedder"]
