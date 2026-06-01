"""Stats primitives — correctness on canonical examples."""

from __future__ import annotations

import math


from efe_rollout.analysis.stats import (
    bh_adjusted_p,
    bh_correction,
    chi2_cdf,
    newcombe_ci,
    normal_cdf,
    normal_ppf,
    reg_incomplete_beta,
    student_t_cdf,
    wald_prop_test,
    welch_t_test,
)


def test_normal_cdf_symmetry():
    for x in (-2.0, -1.0, 0.0, 1.0, 2.0):
        assert abs(normal_cdf(x) + normal_cdf(-x) - 1.0) < 1e-12


def test_normal_ppf_inverts_cdf():
    for p in (0.025, 0.05, 0.5, 0.95, 0.975):
        assert abs(normal_cdf(normal_ppf(p)) - p) < 1e-9


def test_normal_ppf_critical_z_0_05():
    # two-sided 95% → z ≈ 1.959964
    assert abs(normal_ppf(0.975) - 1.959964) < 1e-4


def test_student_t_cdf_matches_known_values():
    # t.cdf(1.0, df=10) ≈ 0.82955
    assert abs(student_t_cdf(1.0, 10.0) - 0.82955) < 1e-3
    # t.cdf(0.0, df=5) == 0.5
    assert abs(student_t_cdf(0.0, 5.0) - 0.5) < 1e-6


def test_chi2_cdf_matches_known_values():
    # chi2.cdf(3.84, 1) ≈ 0.95
    assert abs(chi2_cdf(3.84, 1.0) - 0.95) < 1e-3
    # chi2.cdf(5.99, 2) ≈ 0.95
    assert abs(chi2_cdf(5.99, 2.0) - 0.95) < 1e-3


def test_reg_incomplete_beta_symmetry():
    # I_0.5(2, 2) = 0.5
    assert abs(reg_incomplete_beta(2.0, 2.0, 0.5) - 0.5) < 1e-9


def test_welch_t_test_detects_clear_difference():
    ctrl = [10.0 + 0.1 * i for i in range(100)]  # mean 14.95
    trt = [14.0 + 0.1 * i for i in range(100)]   # mean 18.95
    res = welch_t_test(ctrl, trt, alpha=0.05)
    assert res.p_value < 1e-9
    assert res.estimate > 3.9
    assert res.ci_95 is not None
    assert res.ci_95[0] > 3.0


def test_welch_t_test_null_case_has_high_p_value():
    ctrl = [1.0, 2.0, 3.0, 4.0, 5.0] * 10
    trt = list(ctrl)
    res = welch_t_test(ctrl, trt, alpha=0.05)
    assert res.p_value >= 0.5


def test_welch_t_test_handles_tiny_samples():
    res = welch_t_test([1.0], [2.0])
    assert math.isnan(res.p_value)


def test_wald_prop_test_significant_case():
    # 50/100 vs 80/100 — very significant
    res = wald_prop_test(50, 100, 80, 100, alpha=0.05)
    assert res.p_value < 1e-4
    assert res.ci_95 is not None
    assert res.ci_95[0] > 0.0


def test_wald_prop_test_null_case():
    res = wald_prop_test(50, 100, 52, 100)
    assert res.p_value > 0.5


def test_newcombe_ci_contains_point_estimate():
    lo, hi = newcombe_ci(40, 100, 60, 100)
    # estimate = 0.2, CI should contain it
    assert lo <= 0.2 <= hi


def test_bh_correction_orders_by_p_value():
    # With m=4 and α=0.05 the BH thresholds are
    # [0.0125, 0.025, 0.0375, 0.05]. Only p=0.01 crosses.
    p = [0.01, 0.05, 0.5, 0.04]
    rejected = bh_correction(p, alpha=0.05)
    assert rejected[0]        # p=0.01 → rejected
    assert not rejected[2]    # p=0.5 → never rejected
    # Stronger case: drop p=0.5 and add a near-zero hit so BH picks up
    # multiple.
    p2 = [0.001, 0.01, 0.02, 0.04]
    rejected2 = bh_correction(p2, alpha=0.05)
    assert all(rejected2), f"all four should be BH-rejected, got {rejected2}"


def test_bh_adjusted_p_is_monotone_non_decreasing():
    p = [0.005, 0.01, 0.03, 0.05, 0.20]
    q = bh_adjusted_p(p)
    # sorted pattern — monotone non-decreasing after restoring order
    srt = sorted(zip(p, q, strict=True), key=lambda t: t[0])
    qs = [t[1] for t in srt]
    for i in range(len(qs) - 1):
        assert qs[i] <= qs[i + 1] + 1e-12


def test_bh_empty_input():
    assert bh_correction([], alpha=0.05) == []
    assert bh_adjusted_p([]) == []
