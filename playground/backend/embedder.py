"""
EFE Playground — Embedder module.

Provides text → 384-dim embedding via 2 backends :

  1. **Real backend** (`backend=sentence-transformers`) :
     Uses `sentence-transformers` library + `all-MiniLM-L6-v2` model.
     Requires : `pip install sentence-transformers torch`.
     ~30 MiB model download on first use.
     ~5ms per encode on CPU.

  2. **Fallback simulator** (`backend=hash`) :
     Deterministic hash-based pseudo-embeddings. NOT semantically
     meaningful, but enables offline / no-deps testing of the full
     playground UI / API surface.
     ~0.1ms per encode.
     Embeddings are L2-normalized so cosine similarity is well-defined.

Auto-selects real backend if available, else falls back to hash.

Cf. EFE D16 §8 (loader Rust) for the production canonical implementation.
"""
from __future__ import annotations

import hashlib
import os
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import numpy as np

DIM = 384
MAX_SEQ_LEN = 256


@dataclass
class EmbedderInfo:
    backend: str
    model_name: str
    dim: int
    device: str
    notes: str = ""


class HashEmbedder:
    """Deterministic hash-based pseudo-embedder for offline use.

    Generates a 384-dim float32 L2-normalized vector from any input text
    via per-block SHA-256 + IEEE-754 float decoding. NOT semantically
    meaningful but stable + tested.
    """

    def __init__(self) -> None:
        self.dim = DIM
        self.info = EmbedderInfo(
            backend="hash",
            model_name="EFE-playground-hash-simulator-v1",
            dim=self.dim,
            device="cpu",
            notes=(
                "FALLBACK simulator — NOT semantically meaningful. "
                "Install sentence-transformers for real embeddings."
            ),
        )

    def encode(self, text: str) -> np.ndarray:
        # Normalize input
        text = (text or "").strip().lower()[:MAX_SEQ_LEN * 4]  # truncate
        # Generate enough hash bytes to fill 384 floats × 4 bytes = 1536 bytes
        # SHA-256 yields 32 bytes, so we need 1536 / 32 = 48 hash blocks.
        seed = text.encode("utf-8")
        chunks = []
        for i in range(48):
            block = hashlib.sha256(seed + struct.pack(">I", i)).digest()
            chunks.append(block)
        raw = b"".join(chunks)[:DIM * 4]
        # Decode as 384 float32 values in [-1, 1] approximately
        # Convert each 4-byte chunk to a float in [0, 1) via little-endian uint32
        ints = np.frombuffer(raw, dtype=np.uint32)
        floats = (ints.astype(np.float64) / (2**32)) * 2.0 - 1.0
        vec = floats.astype(np.float32)
        # L2-normalize
        norm = np.linalg.norm(vec)
        if norm > 0:
            vec = vec / norm
        return vec

    def encode_batch(self, texts: list[str]) -> np.ndarray:
        return np.stack([self.encode(t) for t in texts])


class SentenceTransformersEmbedder:
    """Real semantic embeddings via sentence-transformers library.

    Lazy-loads the model on first encode() call.
    """

    def __init__(self, model_name: str = "all-MiniLM-L6-v2") -> None:
        self.model_name = model_name
        self.dim = DIM
        self._model: Optional[object] = None
        self._device = "cpu"
        self.info = EmbedderInfo(
            backend="sentence-transformers",
            model_name=model_name,
            dim=self.dim,
            device=self._device,
            notes="Real semantic embedder via sentence-transformers library.",
        )

    def _load(self) -> None:
        if self._model is None:
            from sentence_transformers import SentenceTransformer
            cache_dir = os.environ.get(
                "EFE_PLAYGROUND_MODEL_CACHE",
                str(Path.home() / ".cache" / "efe-playground"),
            )
            Path(cache_dir).mkdir(parents=True, exist_ok=True)
            self._model = SentenceTransformer(
                self.model_name, cache_folder=cache_dir
            )

    def encode(self, text: str) -> np.ndarray:
        self._load()
        vec = self._model.encode(  # type: ignore[union-attr]
            text or "",
            convert_to_numpy=True,
            normalize_embeddings=True,
        )
        return vec.astype(np.float32)

    def encode_batch(self, texts: list[str]) -> np.ndarray:
        self._load()
        vecs = self._model.encode(  # type: ignore[union-attr]
            texts,
            convert_to_numpy=True,
            normalize_embeddings=True,
            batch_size=32,
        )
        return vecs.astype(np.float32)


def make_embedder(backend: Optional[str] = None) -> HashEmbedder | SentenceTransformersEmbedder:
    """Auto-select embedder backend.

    Order :
      1. Explicit `backend` argument (env `EFE_PLAYGROUND_BACKEND`)
      2. sentence-transformers if installed
      3. Hash simulator (fallback)
    """
    backend = backend or os.environ.get("EFE_PLAYGROUND_BACKEND", "auto")
    if backend == "hash":
        return HashEmbedder()
    if backend == "sentence-transformers":
        return SentenceTransformersEmbedder()
    # Auto : try real, fall back to hash
    try:
        import sentence_transformers  # noqa: F401
        return SentenceTransformersEmbedder()
    except ImportError:
        return HashEmbedder()


def cosine_sim(a: np.ndarray, b: np.ndarray) -> float:
    """Cosine similarity between two L2-normalized vectors.

    Both vectors must be pre-normalized for correctness ; both our
    embedders return normalized vectors.
    """
    return float(np.dot(a, b))
