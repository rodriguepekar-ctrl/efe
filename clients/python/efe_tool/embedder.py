"""Embedder adapters — transform text to vector.

Only the ``Embedder`` protocol is exposed publicly; concrete
implementations are:

* :class:`LocalSTEmbedder` — wraps ``sentence-transformers``. Default
  MVP choice. 80 MiB model, 3-8 ms/text on CPU, free, self-contained.
* (future) ``OpenAIEmbedder`` — wraps the OpenAI embeddings API. Higher
  quality and multilingual but adds a network hop + cost.

An embedder is a **stable identity**: the same model (version, weights)
must always be used with the same catalog. Mismatches are detected via
the ``fingerprint()`` method that the catalog compares to its declared
fingerprint at load time.
"""

from __future__ import annotations

import hashlib
import os
from typing import Any, Protocol, Sequence, runtime_checkable

import numpy as np

from .errors import ConfigError, EmbedderError


@runtime_checkable
class Embedder(Protocol):
    """Protocol any embedder must satisfy."""

    def encode(self, text: str) -> np.ndarray:
        """Encode a single string into a dense f32 vector of shape ``(D,)``.

        Must return a unit-norm (L2=1) vector, or a vector that the
        caller can safely re-normalise without losing meaning.

        Raises:
            EmbedderError: if encoding fails (non-finite, API error).
        """
        ...

    def encode_batch(self, texts: Sequence[str]) -> np.ndarray:
        """Batched version — shape ``(len(texts), D)``."""
        ...

    def dim(self) -> int:
        """Return the output dimension ``D``."""
        ...

    def fingerprint(self) -> str:
        """A stable string uniquely identifying this embedder.

        Convention: ``family/model@dim``, e.g.
        ``sentence-transformers/all-MiniLM-L6-v2@384``. Catalogs
        declare this at compile time and the client refuses to load
        a catalog with a mismatching fingerprint.
        """
        ...


class LocalSTEmbedder:
    """Sentence-Transformers embedder loaded locally.

    Lazy-loads the model on first ``encode`` to keep import-time cheap.

    Example:
        >>> e = LocalSTEmbedder("sentence-transformers/all-MiniLM-L6-v2")
        >>> v = e.encode("hello world")
        >>> assert v.shape == (384,)
    """

    def __init__(self, model_name: str = "sentence-transformers/all-MiniLM-L6-v2") -> None:
        self._model_name = model_name
        self._model: Any | None = None  # SentenceTransformer — lazy
        self._dim: int | None = None

    @classmethod
    def from_env(cls, var: str = "EFE_EMBEDDER_MODEL") -> "LocalSTEmbedder":
        """Construct from an environment variable.

        Falls back to ``all-MiniLM-L6-v2`` when the var is unset.
        """
        model = os.environ.get(var) or "sentence-transformers/all-MiniLM-L6-v2"
        return cls(model)

    def _ensure_model(self) -> Any:
        if self._model is not None:
            return self._model
        try:
            from sentence_transformers import SentenceTransformer
        except ImportError as e:  # pragma: no cover — deps missing
            raise ConfigError(
                "sentence-transformers is required for LocalSTEmbedder; "
                "install with `pip install 'efe-tool[claude]'` or add sentence-transformers."
            ) from e
        self._model = SentenceTransformer(self._model_name)
        try:
            self._dim = int(self._model.get_sentence_embedding_dimension())
        except Exception as e:  # pragma: no cover — SBERT API surprise
            raise EmbedderError(f"cannot read embedder dim: {e}") from e
        return self._model

    def encode(self, text: str) -> np.ndarray:
        model = self._ensure_model()
        if not isinstance(text, str):
            raise EmbedderError(f"encode expected str, got {type(text).__name__}")
        try:
            vec = model.encode(
                text, normalize_embeddings=True, convert_to_numpy=True, show_progress_bar=False
            )
        except Exception as e:
            raise EmbedderError(f"local embedder failed: {e}") from e
        vec = np.asarray(vec, dtype=np.float32).reshape(-1)
        if not np.isfinite(vec).all():
            raise EmbedderError("embedder produced non-finite values")
        return vec

    def encode_batch(self, texts: Sequence[str]) -> np.ndarray:
        model = self._ensure_model()
        if not texts:
            return np.zeros((0, self.dim()), dtype=np.float32)
        try:
            vecs = model.encode(
                list(texts),
                normalize_embeddings=True,
                convert_to_numpy=True,
                show_progress_bar=False,
                batch_size=32,
            )
        except Exception as e:
            raise EmbedderError(f"local embedder batch failed: {e}") from e
        arr = np.asarray(vecs, dtype=np.float32)
        if not np.isfinite(arr).all():
            raise EmbedderError("embedder produced non-finite values in batch")
        return arr

    def dim(self) -> int:
        if self._dim is None:
            self._ensure_model()
        assert self._dim is not None
        return self._dim

    def fingerprint(self) -> str:
        # We include the dim in the fingerprint so catalogs that encode
        # the dim explicitly (e.g. 384) refuse to load if the model was
        # swapped for one with a different dim.
        #
        # Bug fix: when the caller passes a fully-qualified HuggingFace
        # name like ``"sentence-transformers/all-MiniLM-L6-v2"`` we used
        # to re-prefix it and end up with
        # ``"sentence-transformers/sentence-transformers/..."``, which
        # never matches the YAML catalog's declared fingerprint. Strip
        # the prefix first so both the HF-qualified and short forms
        # produce the same canonical fingerprint.
        name = self._model_name
        prefix = "sentence-transformers/"
        if name.startswith(prefix):
            name = name[len(prefix):]
        return f"sentence-transformers/{name}@{self.dim()}"


def fingerprint_of_text_hash(text: str) -> str:
    """SHA-256 hex of the text — used internally for cache keys and audit.

    Normalisation:
      * NFC Unicode form
      * strip
      * collapse internal whitespace to a single space
    """
    import unicodedata
    import re

    normed = unicodedata.normalize("NFC", text).strip()
    normed = re.sub(r"\s+", " ", normed)
    return hashlib.sha256(normed.encode("utf-8")).hexdigest()
