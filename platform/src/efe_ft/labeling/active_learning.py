"""Three active-learning samplers + hybrid combinator.

* :func:`uncertainty_sampler` — pick samples with max softmax entropy.
* :func:`diversity_sampler` — farthest-point selection in cosine space.
* :func:`hybrid_select` — over-pick by uncertainty, prune by diversity.

All samplers are **pure** — no I/O, no mutation. The caller is
responsible for loading the pool probabilities and embeddings.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

import numpy as np

from efe_ft.errors import ActiveLearningError


@dataclass(frozen=True, slots=True)
class Candidate:
    """One sample surfaced by an AL sampler."""

    sample_id: str
    uncertainty: float
    diversity_score: float = 0.0
    payload: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "sample_id": self.sample_id,
            "uncertainty": round(self.uncertainty, 4),
            "diversity_score": round(self.diversity_score, 4),
            **({"payload": dict(self.payload)} if self.payload else {}),
        }


@dataclass(frozen=True, slots=True)
class ALBatch:
    """A fixed-size batch of candidates returned to a labeler session."""

    candidates: tuple[Candidate, ...]
    alpha: float
    batch_size: int

    def to_dict(self) -> dict[str, Any]:
        return {
            "candidates": [c.to_dict() for c in self.candidates],
            "alpha": self.alpha,
            "batch_size": self.batch_size,
        }


# ---------------------------------------------------------------------------
#  Samplers
# ---------------------------------------------------------------------------


def entropy(p: np.ndarray) -> float:
    """Shannon entropy of one probability distribution (base e)."""
    p = np.clip(p, 1e-12, 1.0)
    return float(-(p * np.log(p)).sum())


def uncertainty_sampler(
    pool_probs: np.ndarray, top_k: int
) -> np.ndarray:
    """Pool indices sorted by predictive entropy, descending."""
    if pool_probs.ndim != 2:
        raise ActiveLearningError(
            f"pool_probs must be 2-D, got shape {pool_probs.shape}",
        )
    if top_k <= 0 or pool_probs.shape[0] == 0:
        return np.array([], dtype=np.int64)
    h = np.apply_along_axis(entropy, 1, pool_probs)
    return np.argsort(-h)[: min(top_k, len(h))]


def diversity_sampler(
    pool_embeddings: np.ndarray,
    top_k: int,
    rng: np.random.Generator | None = None,
) -> np.ndarray:
    """Farthest-point greedy selection in cosine space.

    Assumes ``pool_embeddings`` are L2-normalised. The first point is
    sampled uniformly at random (via ``rng``); each subsequent point
    maximises the min-distance to the already-selected set.
    """
    n = pool_embeddings.shape[0]
    if top_k <= 0:
        return np.array([], dtype=np.int64)
    if n == 0:
        return np.array([], dtype=np.int64)
    rng = rng or np.random.default_rng(0)
    first = int(rng.integers(0, n))
    chosen = [first]
    # min_sim[i] = max cosine(i, chosen) — we pick the index with the
    # LOWEST value to maximise distance from the chosen set.
    min_sim = pool_embeddings @ pool_embeddings[first]
    for _ in range(min(top_k, n) - 1):
        idx = int(np.argmin(min_sim))
        chosen.append(idx)
        new_sim = pool_embeddings @ pool_embeddings[idx]
        min_sim = np.maximum(min_sim, new_sim)
        # Avoid re-picking same index (set that dim very high)
        min_sim[idx] = np.inf
    return np.array(chosen, dtype=np.int64)


def hybrid_select(
    pool_probs: np.ndarray,
    pool_embeddings: np.ndarray,
    batch_size: int,
    *,
    alpha: float = 0.7,
    rng: np.random.Generator | None = None,
) -> np.ndarray:
    """Hybrid: pick ``batch_size / alpha`` by uncertainty, prune by diversity.

    ``alpha=1.0`` ≡ pure uncertainty ; ``alpha=0.0`` ≡ pure diversity ;
    intermediate values balance exploration vs exploitation.
    """
    if not 0.0 <= alpha <= 1.0:
        raise ActiveLearningError(f"alpha must be in [0, 1], got {alpha}")
    if pool_probs.shape[0] != pool_embeddings.shape[0]:
        raise ActiveLearningError(
            "pool_probs and pool_embeddings must have matching N",
        )
    if batch_size <= 0 or pool_probs.shape[0] == 0:
        return np.array([], dtype=np.int64)
    rng = rng or np.random.default_rng(0)
    if alpha <= 1e-6:
        return diversity_sampler(pool_embeddings, batch_size, rng)
    k_unc = int(np.ceil(batch_size / alpha))
    k_unc = min(k_unc, pool_probs.shape[0])
    unc_idx = uncertainty_sampler(pool_probs, k_unc)
    if alpha >= 1.0 - 1e-6 or len(unc_idx) == batch_size:
        return np.asarray(unc_idx[:batch_size], dtype=np.int64)
    sub_emb = pool_embeddings[unc_idx]
    div_local = diversity_sampler(sub_emb, batch_size, rng)
    return np.asarray(unc_idx[div_local], dtype=np.int64)


__all__ = [
    "ALBatch",
    "Candidate",
    "diversity_sampler",
    "entropy",
    "hybrid_select",
    "uncertainty_sampler",
]
