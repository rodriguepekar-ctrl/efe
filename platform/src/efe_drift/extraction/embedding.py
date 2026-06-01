"""Embedding extractors.

Two flavours :

* :class:`SbertEmbeddingExtractor` — wraps a
  ``sentence-transformers`` model (``all-MiniLM-L6-v2`` by default,
  384-dim). The model is lazy-loaded the first time ``extract`` is
  called so module import stays cheap.

* :class:`DeterministicHashEmbeddingExtractor` — the deterministic
  pseudo-embedding used by unit tests. Produces a low-dim vector
  from a BLAKE2 hash of the input text. No ML dependency needed,
  perfectly reproducible.

Both implement the :class:`~efe_drift.extraction.base.FeatureExtractor`
Protocol.
"""

from __future__ import annotations

import hashlib
import math
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from efe_drift.extraction.base import (
    embedding,
)
from efe_drift.records import FeatureKind, FeatureValue, InferenceEvent

# ---------------------------------------------------------------------------
# Deterministic pseudo-embedding — test fixture
# ---------------------------------------------------------------------------


def _blake2_vector(text: str, *, dim: int) -> tuple[float, ...]:
    """Map ``text`` to a deterministic L2-normalised vector of ``dim``.

    Uses BLAKE2b with a large digest size then unpacks bytes into
    floats in ``[-1, 1]``. The result is L2-normalised so it lives on
    the unit sphere — the same topology as a real normalised SBERT
    embedding, which keeps MMD² / cosine tests meaningful.
    """
    if dim < 2 or dim > 4096:
        raise ValueError(f"dim must be in [2, 4096], got {dim}")
    # BLAKE2b digest = 64 bytes max; for larger dims we iterate with
    # a counter suffix so each block is still deterministic.
    byts = bytearray()
    iteration = 0
    needed = dim * 2  # 2 bytes per component
    while len(byts) < needed:
        h = hashlib.blake2b(
            text.encode("utf-8") + iteration.to_bytes(4, "big"),
            digest_size=64,
        )
        byts.extend(h.digest())
        iteration += 1
    comps: list[float] = []
    for i in range(dim):
        pair = byts[2 * i: 2 * i + 2]
        unsigned = int.from_bytes(pair, "big")
        # Map to [-1, 1]
        normed = (unsigned / 0xFFFF) * 2.0 - 1.0
        comps.append(normed)
    # L2 normalise
    norm = math.sqrt(sum(c * c for c in comps)) or 1.0
    return tuple(c / norm for c in comps)


@dataclass(frozen=True, slots=True)
class DeterministicHashEmbeddingExtractor:
    """Pseudo-embedder used by unit tests.

    Runs in microseconds, produces reproducible vectors, and needs no
    heavy dependency. Its only job is to let the rest of the pipeline
    be exercised end-to-end.
    """

    dim: int = 32
    name: str = "deterministic_hash_embedding"
    kind: FeatureKind = "embedding"
    feature_name: str = "prompt_embedding"
    source: str = "prompt"               # "prompt" or "response"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        text = (
            event.prompt_text if self.source == "prompt"
            else event.response_text
        )
        if not text:
            return None
        return embedding(
            self.feature_name, _blake2_vector(text, dim=self.dim),
        )


# ---------------------------------------------------------------------------
# Sentence-transformers embedder — lazy loaded
# ---------------------------------------------------------------------------


class SbertEmbeddingExtractor:
    """Wraps ``sentence-transformers`` with lazy model load.

    The embedder is a heavy runtime dependency (~80 MB for MiniLM +
    PyTorch CPU). We keep ``import sentence_transformers`` inside
    :meth:`_ensure_model` so the module is importable without it —
    essential for unit tests that don't exercise this extractor.
    """

    __slots__ = (
        "_device",
        "_feature_name",
        "_kind",
        "_loader",
        "_model",
        "_model_name",
        "_name",
        "_source",
    )

    # ---- Protocol attributes --------------------------------------------

    def __init__(
        self,
        *,
        model_name: str = "all-MiniLM-L6-v2",
        device: str = "cpu",
        source: str = "prompt",
        name: str = "sbert_minilm",
        feature_name: str = "prompt_embedding",
        loader: Callable[[str, str], Any] | None = None,
    ) -> None:
        if source not in ("prompt", "response"):
            raise ValueError(
                f"source must be 'prompt' or 'response', got {source!r}",
            )
        self._model_name = model_name
        self._device = device
        self._source = source
        self._name = name
        self._feature_name = feature_name
        self._kind: FeatureKind = "embedding"
        self._model: Any = None
        self._loader = loader or self._default_loader

    @property
    def name(self) -> str:
        return self._name

    @property
    def kind(self) -> FeatureKind:
        return self._kind

    # ---- extract --------------------------------------------------------

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        text = (
            event.prompt_text if self._source == "prompt"
            else event.response_text
        )
        if not text:
            return None
        model = self._ensure_model()
        vec = model.encode(text, normalize_embeddings=True)
        return embedding(self._feature_name, list(vec))

    # ---- internals ------------------------------------------------------

    def _ensure_model(self) -> Any:
        if self._model is None:
            self._model = self._loader(self._model_name, self._device)
        return self._model

    @staticmethod
    def _default_loader(model_name: str, device: str) -> Any:
        # Lazy import so the module is importable without the optional
        # dependency.
        from sentence_transformers import SentenceTransformer
        return SentenceTransformer(model_name, device=device)


__all__ = [
    "DeterministicHashEmbeddingExtractor",
    "SbertEmbeddingExtractor",
]
