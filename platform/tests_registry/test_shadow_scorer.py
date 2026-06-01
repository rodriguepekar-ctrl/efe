"""Shadow scoring primitives."""

from __future__ import annotations

import pytest

from efe_registry.shadow.scorer import (
    bleu4_on_tokens,
    cosine_similarity,
    latency_delta_ms,
)


def test_cosine_identical_vectors():
    assert cosine_similarity([1.0, 0.0], [1.0, 0.0]) == pytest.approx(1.0)


def test_cosine_orthogonal_vectors():
    assert cosine_similarity([1.0, 0.0], [0.0, 1.0]) == pytest.approx(0.0)


def test_cosine_rejects_mismatched_lengths():
    with pytest.raises(ValueError):
        cosine_similarity([1.0, 2.0], [1.0])


def test_cosine_zero_vector_returns_zero():
    assert cosine_similarity([0.0, 0.0], [1.0, 0.0]) == 0.0


def test_bleu4_identical_sequences():
    toks = ["the", "quick", "brown", "fox", "jumps"]
    assert bleu4_on_tokens(toks, toks) == pytest.approx(1.0, abs=1e-6)


def test_bleu4_totally_different():
    ref = ["alpha", "beta", "gamma", "delta"]
    cand = ["omega", "zeta", "phi", "chi"]
    # Smoothing gives a small non-zero floor; identical-tokens test
    # ensures the top range is 1.0, this one pins the low bound well
    # below the high-quality threshold.
    assert bleu4_on_tokens(ref, cand) < 0.3


def test_bleu4_empty_is_zero():
    assert bleu4_on_tokens([], ["x"]) == 0.0
    assert bleu4_on_tokens(["x"], []) == 0.0


def test_latency_delta():
    b = [10.0, 12.0, 14.0]
    c = [12.0, 14.0, 16.0]
    assert latency_delta_ms(b, c) == pytest.approx(2.0)
