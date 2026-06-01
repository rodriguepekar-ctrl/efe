"""Tests for the statistical tests — McNemar, bootstrap, TOST, Welch."""

from __future__ import annotations

from typing import Any

import numpy as np
import pytest

from efe_gate.errors import InsufficientSample, StatisticalTestError
from efe_gate.metrics.stats import (
    bootstrap_ci,
    bootstrap_delta_ci,
    mcnemar_test,
    tost_equivalence,
    welch_test,
)
from efe_gate.metrics.stats.tost import TostVerdict


# ---------------- McNemar ----------------


def test_mcnemar_identical_perfect_agreement() -> None:
    x = np.array([1, 1, 0, 0, 1])
    res = mcnemar_test(x, x)
    assert res.b == 0 and res.c == 0
    assert res.p_value == 1.0


def test_mcnemar_candidate_better() -> None:
    # Champion wrong on half, candidate right everywhere.
    champ = np.array([0] * 10 + [1] * 10)
    cand = np.ones(20)
    res = mcnemar_test(champ, cand)
    assert res.candidate_better is True
    assert res.p_value < 0.1


def test_mcnemar_uses_binom_when_small() -> None:
    champ = np.array([1, 0, 1, 1])
    cand = np.array([0, 1, 1, 1])
    res = mcnemar_test(champ, cand)
    assert res.test == "binom_exact"


def test_mcnemar_uses_chi2_when_large(bern: Any) -> None:
    champ = bern(0.7, 500, 1)
    cand = bern(0.75, 500, 2)
    res = mcnemar_test(champ, cand)
    assert res.test == "chi2"


def test_mcnemar_shape_mismatch() -> None:
    with pytest.raises(StatisticalTestError):
        mcnemar_test(np.array([1, 0]), np.array([1, 0, 1]))


def test_mcnemar_empty() -> None:
    with pytest.raises(InsufficientSample):
        mcnemar_test(np.array([]), np.array([]))


# ---------------- Bootstrap ----------------


def test_bootstrap_ci_covers_true_mean(bern: Any) -> None:
    x = bern(0.8, 1000, 42)
    res = bootstrap_ci(x, n_resamples=2000, seed=42)
    assert res.ci_lower <= 0.8 <= res.ci_upper


def test_bootstrap_ci_empty_raises() -> None:
    with pytest.raises(InsufficientSample):
        bootstrap_ci(np.array([]))


def test_bootstrap_delta_ci_preserves_sign(bern: Any) -> None:
    champ = bern(0.7, 500, 1)
    cand = bern(0.85, 500, 1)  # same seed — paired-ish
    res = bootstrap_delta_ci(champ, cand, n_resamples=1000, seed=42)
    # mean(candidate - champion) should be positive
    assert res.mean > 0
    assert res.ci_lower > -0.1


def test_bootstrap_delta_shape_mismatch() -> None:
    with pytest.raises(ValueError):
        bootstrap_delta_ci(np.array([1, 0]), np.array([1, 0, 1]))


def test_bootstrap_bad_conf() -> None:
    with pytest.raises(ValueError):
        bootstrap_ci(np.array([1.0]), conf=1.5)


# ---------------- TOST ----------------


def test_tost_pass_when_equivalent(bern: Any) -> None:
    x = bern(0.8, 500, 42)
    y = bern(0.80, 500, 43)  # same rough rate
    res = tost_equivalence(
        x, y, delta_margin=0.03, n_resamples=500, seed=1,
    )
    assert res.verdict in (TostVerdict.PASS, TostVerdict.AMBIGUOUS)


def test_tost_fail_on_regression(bern: Any) -> None:
    champ = bern(0.9, 2000, 1)
    cand = bern(0.70, 2000, 1)
    res = tost_equivalence(
        champ, cand, delta_margin=0.02, n_resamples=500, seed=1,
    )
    assert res.verdict is TostVerdict.FAIL


def test_tost_better(bern: Any) -> None:
    champ = bern(0.7, 2000, 1)
    cand = bern(0.9, 2000, 1)
    res = tost_equivalence(
        champ, cand, delta_margin=0.02, n_resamples=500, seed=1,
    )
    assert res.verdict is TostVerdict.BETTER


def test_tost_lower_is_better(bern: Any) -> None:
    # Latency: candidate is 50 ms, champion 30 ms → FAIL.
    champ_ms = np.full(500, 30.0)
    cand_ms = np.full(500, 50.0)
    res = tost_equivalence(
        champ_ms, cand_ms,
        delta_margin=5.0, higher_is_better=False,
        n_resamples=500, seed=1,
    )
    assert res.verdict is TostVerdict.FAIL


def test_tost_bad_margin() -> None:
    with pytest.raises(ValueError):
        tost_equivalence(
            np.array([1.0]), np.array([1.0]),
            delta_margin=-0.5, n_resamples=200,
        )


# ---------------- Welch ----------------


def test_welch_candidate_slower() -> None:
    # candidate is significantly slower
    rng = np.random.default_rng(1)
    champ = rng.normal(30.0, 2.0, 500)
    cand = rng.normal(40.0, 2.0, 500)
    res = welch_test(champ, cand, alternative="greater")
    assert res.p_value < 1e-10
    assert res.candidate_mean > res.champion_mean


def test_welch_no_signif_when_equal() -> None:
    rng = np.random.default_rng(42)
    a = rng.normal(10, 1, 500)
    b = rng.normal(10, 1, 500)
    res = welch_test(a, b, alternative="greater")
    assert res.p_value > 0.05


def test_welch_insufficient() -> None:
    with pytest.raises(InsufficientSample):
        welch_test(np.array([1.0]), np.array([1.0]))
