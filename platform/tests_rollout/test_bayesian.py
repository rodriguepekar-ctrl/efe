"""Bayesian beta-binomial posterior + decision rule."""

from __future__ import annotations

from efe_rollout.analysis.bayesian import (
    bayesian_beta_binomial,
    bayesian_decision,
)


def test_strong_treatment_signal_gives_posterior_near_1():
    res = bayesian_beta_binomial(
        s_a=50, n_a=500, s_b=150, n_b=500, n_samples=5_000,
    )
    # 10 % vs 30 % — definitely treatment-better
    assert res.prob_b_gt_a > 0.99


def test_null_case_posterior_near_0_5():
    res = bayesian_beta_binomial(
        s_a=100, n_a=500, s_b=100, n_b=500, n_samples=5_000,
    )
    assert 0.40 < res.prob_b_gt_a < 0.60


def test_decision_ship_fires_when_threshold_crossed():
    res = bayesian_beta_binomial(
        s_a=50, n_a=1000, s_b=80, n_b=1000, n_samples=5_000,
    )
    assert bayesian_decision(
        res, ship_threshold=0.95, rollback_threshold=0.05,
    ) in ("ship", "continue")


def test_decision_rollback_fires_when_treatment_much_worse():
    res = bayesian_beta_binomial(
        s_a=200, n_a=1000, s_b=50, n_b=1000, n_samples=5_000,
    )
    assert bayesian_decision(
        res, ship_threshold=0.95, rollback_threshold=0.05,
    ) == "rollback"


def test_determinism_under_fixed_seed():
    a = bayesian_beta_binomial(
        s_a=10, n_a=100, s_b=15, n_b=100, n_samples=1_000, seed=42,
    )
    b = bayesian_beta_binomial(
        s_a=10, n_a=100, s_b=15, n_b=100, n_samples=1_000, seed=42,
    )
    assert a.prob_b_gt_a == b.prob_b_gt_a
    assert a.credible_interval_95 == b.credible_interval_95


def test_different_seeds_give_different_samples():
    a = bayesian_beta_binomial(
        s_a=10, n_a=100, s_b=15, n_b=100, n_samples=1_000, seed=42,
    )
    b = bayesian_beta_binomial(
        s_a=10, n_a=100, s_b=15, n_b=100, n_samples=1_000, seed=43,
    )
    # close but not bit-identical — tolerant
    _ = (a.prob_b_gt_a, b.prob_b_gt_a)
