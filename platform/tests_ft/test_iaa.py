"""Tests for Cohen + Fleiss kappa."""

from __future__ import annotations

import numpy as np
import pytest

from efe_ft.labeling.iaa import (
    cohen_kappa,
    fleiss_kappa,
    pairwise_kappas,
)
from efe_ft.errors import LabelingError


def test_cohen_perfect_agreement() -> None:
    r = cohen_kappa(["a", "b", "c", "a"], ["a", "b", "c", "a"])
    assert r.kappa == 1.0
    assert r.n_agreements == 4
    assert r.n_samples == 4


def test_cohen_complete_disagreement() -> None:
    # Each time they disagree when they could agree
    r = cohen_kappa(["a", "a", "a", "a"], ["b", "b", "b", "b"])
    assert r.kappa == 0.0
    assert r.n_agreements == 0


def test_cohen_realistic_partial() -> None:
    r = cohen_kappa(
        ["a", "b", "a", "b", "a", "b", "a", "b"],
        ["a", "b", "a", "a", "a", "b", "b", "b"],
    )
    assert 0.3 < r.kappa < 0.9


def test_cohen_length_mismatch_raises() -> None:
    with pytest.raises(LabelingError, match="length"):
        cohen_kappa(["a", "b"], ["a"])


def test_cohen_empty_returns_zero() -> None:
    r = cohen_kappa([], [])
    assert r.kappa == 0.0
    assert r.n_samples == 0


def test_fleiss_perfect_agreement() -> None:
    ratings = np.array([
        [3, 0, 0], [3, 0, 0], [0, 3, 0], [0, 0, 3],
    ])
    r = fleiss_kappa(ratings)
    assert r.kappa == 1.0
    assert r.n_agreements == 4


def test_fleiss_all_raters_disagree() -> None:
    # Each item: each rater picks a different class
    ratings = np.array([
        [1, 1, 1], [1, 1, 1], [1, 1, 1], [1, 1, 1],
    ])
    r = fleiss_kappa(ratings)
    assert r.kappa < 0.1


def test_fleiss_mixed_ratings() -> None:
    ratings = np.array([
        [3, 0, 0], [2, 1, 0], [0, 3, 0], [1, 0, 2], [0, 2, 1],
    ])
    r = fleiss_kappa(ratings)
    assert 0.2 < r.kappa < 0.9


def test_fleiss_inconsistent_rater_count_raises() -> None:
    ratings = np.array([[3, 0], [2, 0]])  # second row sums to 2
    with pytest.raises(LabelingError, match="same number of raters"):
        fleiss_kappa(ratings)


def test_pairwise_kappas_compute_overlap_only() -> None:
    by_ann = {
        "alice":   {"s1": "a", "s2": "b", "s3": "a"},
        "bob":     {"s1": "a", "s2": "b", "s4": "c"},  # s4 only bob
        "carol":   {"s1": "b", "s3": "a"},
    }
    pairs = pairwise_kappas(by_ann)
    assert ("alice", "bob") in pairs
    assert pairs[("alice", "bob")].n_samples == 2  # s1+s2 overlap
    # alice-carol overlap is s1, s3 → 2 samples
    assert pairs[("alice", "carol")].n_samples == 2
