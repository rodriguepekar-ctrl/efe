"""CUPED — variance reduction property."""

from __future__ import annotations

import random

from efe_rollout.analysis.cuped import apply_cuped


def test_cuped_reduces_variance_when_covariate_correlated():
    rng = random.Random(42)
    n = 2_000
    # X drives 80 % of Y variance.
    x = [rng.gauss(0, 1.0) for _ in range(n)]
    y = [xi + rng.gauss(0, 0.5) for xi in x]
    res = apply_cuped(y, x)
    assert res.variance_reduction > 0.25
    # theta ≈ 1.0 since Y = X + noise
    assert 0.7 < res.theta < 1.3


def test_cuped_noop_when_covariate_is_constant():
    y = [1.0, 2.0, 3.0, 4.0]
    x = [5.0] * 4
    res = apply_cuped(y, x)
    assert res.theta == 0.0
    assert res.variance_reduction == 0.0


def test_cuped_preserves_mean_approximately():
    rng = random.Random(7)
    n = 500
    x = [rng.gauss(0, 1) for _ in range(n)]
    y = [xi * 0.5 + rng.gauss(0, 0.5) for xi in x]
    res = apply_cuped(y, x)
    # mean(Y_adj) ≈ mean(Y) because we subtract θ·(X - mean(X))
    assert abs(sum(res.y_adj) / n - sum(y) / n) < 0.05
