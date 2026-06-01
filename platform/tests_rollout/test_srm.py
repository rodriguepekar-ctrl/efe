"""Sample Ratio Mismatch detection."""

from __future__ import annotations

import pytest

from efe_rollout.analysis.srm import detect_srm


def test_srm_not_flagged_when_observed_matches_expected():
    report = detect_srm(
        observed={"a": 5000, "b": 5000},
        expected_pct={"a": 50, "b": 50},
    )
    assert not report.has_mismatch
    assert report.p_value > 0.5


def test_srm_flagged_when_big_skew():
    report = detect_srm(
        observed={"a": 6000, "b": 4000},
        expected_pct={"a": 50, "b": 50},
    )
    assert report.has_mismatch
    assert report.p_value < 1e-3


def test_srm_empty_observed():
    report = detect_srm(
        observed={},
        expected_pct={"a": 50, "b": 50},
    )
    assert not report.has_mismatch
    assert report.chi_squared == 0.0


def test_srm_rejects_empty_expected():
    with pytest.raises(ValueError):
        detect_srm(observed={"a": 10}, expected_pct={})
