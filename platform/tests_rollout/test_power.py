"""Power analysis — canonical example matches Kohavi's tables."""

from __future__ import annotations

import pytest

from efe_rollout.analysis.power import (
    detectable_effect_proportions,
    sample_size_means,
    sample_size_proportions,
)


def test_baseline_5_pct_lift_10_pct_needs_about_31k_per_variant():
    result = sample_size_proportions(
        baseline=0.05,
        relative_lift=0.10,
        alpha=0.05,
        power=0.80,
    )
    # Fleiss formula gives ~31k per variant; allow ±10%.
    assert 27_000 <= result.per_variant_sample_size <= 34_000


def test_sample_size_scales_with_baseline():
    low = sample_size_proportions(
        baseline=0.02, relative_lift=0.10,
    ).per_variant_sample_size
    high = sample_size_proportions(
        baseline=0.20, relative_lift=0.10,
    ).per_variant_sample_size
    # Smaller baseline → larger sample needed.
    assert low > high


def test_sample_size_means_formula():
    # With σ=1 and MDE=0.1, z's sum ≈ 2.80 → n ≈ 2·1²·2.80²/0.01 ≈ 1568
    result = sample_size_means(
        sigma=1.0,
        minimum_detectable_effect=0.1,
        alpha=0.05,
        power=0.8,
    )
    assert 1_400 <= result.per_variant_sample_size <= 1_700


def test_rejects_zero_lift():
    with pytest.raises(ValueError):
        sample_size_proportions(
            baseline=0.1, relative_lift=0.0,
        )


def test_detectable_effect_is_consistent_with_sample_size():
    n = sample_size_proportions(
        baseline=0.10, relative_lift=0.20,
    ).per_variant_sample_size
    mde = detectable_effect_proportions(
        baseline=0.10, n_per_variant=n,
    )
    # Round-trip agreement within a factor 2 (formulas use different
    # variance assumptions — pooled vs per-arm).
    assert 0.10 <= mde <= 0.30
