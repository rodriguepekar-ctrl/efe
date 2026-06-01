"""Shadow deployment — tap on a candidate, score offline."""

from __future__ import annotations

from efe_registry.shadow.scorer import (
    ShadowScore,
    bleu4_on_tokens,
    cosine_similarity,
    latency_delta_ms,
)
from efe_registry.shadow.tapper import ShadowTapper

__all__ = [
    "ShadowScore",
    "ShadowTapper",
    "bleu4_on_tokens",
    "cosine_similarity",
    "latency_delta_ms",
]
