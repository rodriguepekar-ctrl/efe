"""Cache layer — verbatim + semantic, strict tenant partition."""

from __future__ import annotations

from efe_inference.cache.embed import (
    DeterministicEmbedder,
    Embedder,
    NullEmbedder,
)
from efe_inference.cache.key import CacheKey, CacheKeyBuilder
from efe_inference.cache.semantic import (
    InMemorySemanticCache,
    SemanticCache,
    SemanticHit,
)
from efe_inference.cache.verbatim import (
    InMemoryVerbatimCache,
    VerbatimCache,
)

__all__ = [
    "CacheKey",
    "CacheKeyBuilder",
    "DeterministicEmbedder",
    "Embedder",
    "InMemorySemanticCache",
    "InMemoryVerbatimCache",
    "NullEmbedder",
    "SemanticCache",
    "SemanticHit",
    "VerbatimCache",
]
