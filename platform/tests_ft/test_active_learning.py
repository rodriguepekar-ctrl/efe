"""Tests for active-learning samplers."""

from __future__ import annotations

import numpy as np
import pytest

from efe_ft.errors import ActiveLearningError
from efe_ft.labeling.active_learning import (
    diversity_sampler,
    entropy,
    hybrid_select,
    uncertainty_sampler,
)


def test_entropy_bounds() -> None:
    # Certain = 0
    p_certain = np.array([1.0, 0.0, 0.0])
    assert entropy(p_certain) < 1e-9
    # Uniform over K classes = log(K)
    p_uniform = np.array([1 / 3, 1 / 3, 1 / 3])
    assert abs(entropy(p_uniform) - np.log(3)) < 1e-9


def test_uncertainty_ranks_correctly() -> None:
    probs = np.array([
        [0.99, 0.005, 0.005],   # certain
        [0.40, 0.35, 0.25],     # uncertain
        [0.33, 0.33, 0.34],     # very uncertain
    ])
    idx = uncertainty_sampler(probs, top_k=3)
    assert idx[0] == 2      # most uncertain first
    assert idx[-1] == 0     # most certain last


def test_uncertainty_empty() -> None:
    idx = uncertainty_sampler(np.zeros((0, 3)), top_k=5)
    assert idx.size == 0


def test_uncertainty_bad_shape() -> None:
    with pytest.raises(ActiveLearningError):
        uncertainty_sampler(np.array([0.5, 0.5]), top_k=1)


def test_diversity_sampler_picks_distinct() -> None:
    rng = np.random.default_rng(42)
    # 10 points in R^8, L2-normalised
    embs = rng.standard_normal((10, 8)).astype(np.float32)
    embs /= np.linalg.norm(embs, axis=1, keepdims=True)
    idx = diversity_sampler(embs, top_k=5, rng=rng)
    assert len(idx) == 5
    assert len(set(idx.tolist())) == 5


def test_diversity_sampler_empty_pool() -> None:
    rng = np.random.default_rng(0)
    idx = diversity_sampler(np.zeros((0, 8)), top_k=3, rng=rng)
    assert idx.size == 0


def test_hybrid_basic_shape() -> None:
    rng = np.random.default_rng(0)
    probs = rng.dirichlet(np.ones(4), size=100)
    embs = rng.standard_normal((100, 16)).astype(np.float32)
    embs /= np.linalg.norm(embs, axis=1, keepdims=True)
    idx = hybrid_select(probs, embs, batch_size=10, alpha=0.7, rng=rng)
    assert idx.shape == (10,)
    assert len(set(idx.tolist())) == 10


def test_hybrid_alpha_bounds_enforced() -> None:
    probs = np.zeros((10, 3))
    embs = np.zeros((10, 5))
    with pytest.raises(ActiveLearningError, match="alpha"):
        hybrid_select(probs, embs, batch_size=3, alpha=1.5)


def test_hybrid_alpha_one_equals_uncertainty() -> None:
    rng = np.random.default_rng(0)
    probs = rng.dirichlet(np.ones(3), size=20)
    embs = rng.standard_normal((20, 4))
    embs /= np.linalg.norm(embs, axis=1, keepdims=True)
    hybrid = hybrid_select(probs, embs, batch_size=5, alpha=1.0, rng=rng)
    pure = uncertainty_sampler(probs, top_k=5)
    # Same samples, possibly different order — compare as sets.
    assert set(hybrid.tolist()) == set(pure.tolist())


def test_hybrid_alpha_zero_equals_diversity() -> None:
    rng = np.random.default_rng(0)
    probs = np.zeros((10, 3))
    embs = rng.standard_normal((10, 4))
    embs /= np.linalg.norm(embs, axis=1, keepdims=True)
    hybrid = hybrid_select(probs, embs, batch_size=3, alpha=0.0,
                            rng=np.random.default_rng(42))
    pure = diversity_sampler(embs, top_k=3, rng=np.random.default_rng(42))
    assert list(hybrid) == list(pure)
