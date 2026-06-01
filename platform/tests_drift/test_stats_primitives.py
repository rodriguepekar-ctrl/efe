"""Pure-python stats primitives — parity with scipy when available."""

from __future__ import annotations

import math

import pytest

from efe_drift.detection.stats import (
    chi2_cdf,
    kolmogorov_p_value,
    ks_two_sample,
    normal_cdf,
    normal_ppf,
    rank_auc,
    student_t_cdf,
)


def test_normal_cdf_values():
    assert abs(normal_cdf(0.0) - 0.5) < 1e-12
    assert abs(normal_cdf(1.96) - 0.9750) < 1e-3
    assert abs(normal_cdf(-1.96) - 0.0250) < 1e-3


def test_normal_ppf_inverts_cdf():
    for p in (0.01, 0.05, 0.25, 0.5, 0.75, 0.95, 0.99):
        z = normal_ppf(p)
        assert abs(normal_cdf(z) - p) < 1e-9


def test_chi2_cdf_known_values():
    # χ²(df=1) at 3.84 ≈ 0.95
    assert abs(chi2_cdf(3.84, 1) - 0.95) < 1e-3
    assert abs(chi2_cdf(0.0, 5) - 0.0) < 1e-12


def test_student_t_cdf_midpoint():
    # t(ν, 0) = 0.5 for any df
    for df in (1, 5, 10, 30):
        assert abs(student_t_cdf(0.0, df) - 0.5) < 1e-9


def test_kolmogorov_p_value_zero():
    assert kolmogorov_p_value(0.0, 100) == 1.0


def test_kolmogorov_p_value_extreme_rejects():
    # Large D with big N must be extremely small p-value.
    p = kolmogorov_p_value(0.8, 100)
    assert p < 1e-30


def test_ks_two_sample_identical_distributions():
    a = list(range(100))
    d, p = ks_two_sample(a, a)
    assert d == 0.0
    assert p == 1.0


def test_ks_two_sample_shifted_distributions():
    a = list(range(100))
    b = [x + 50 for x in a]
    d, p = ks_two_sample(a, b)
    assert d > 0.3
    assert p < 0.05


def test_ks_two_sample_empty_inputs():
    d, p = ks_two_sample([], [1, 2, 3])
    assert math.isnan(d)
    assert math.isnan(p)


def test_rank_auc_perfect_separation():
    scores = [0.1, 0.2, 0.8, 0.9]
    labels = [0, 0, 1, 1]
    assert rank_auc(scores, labels) == 1.0


def test_rank_auc_no_separation():
    scores = [0.5, 0.5, 0.5, 0.5]
    labels = [0, 1, 0, 1]
    # All tied — by Mann-Whitney definition with 0.5 on ties, AUC is 0.5.
    assert abs(rank_auc(scores, labels) - 0.5) < 1e-12


def test_rank_auc_one_class_only_returns_0p5():
    scores = [0.1, 0.2]
    labels = [0, 0]
    assert rank_auc(scores, labels) == 0.5


def test_rank_auc_length_mismatch_raises():
    with pytest.raises(ValueError):
        rank_auc([0.1, 0.2], [0])


def test_normal_ppf_invalid_input():
    with pytest.raises(ValueError):
        normal_ppf(float("nan"))


def test_chi2_cdf_negative_x():
    assert chi2_cdf(-1.0, 3) == 0.0
