"""Severity classification + monotonicity invariant (I3)."""

from __future__ import annotations

import pytest

from efe_drift.detection.severity import (
    DEFAULT_CHI_SQUARED_P,
    DEFAULT_PSI,
    DEFAULTS,
    SeverityThresholds,
)


def test_default_psi_thresholds_monotone():
    assert DEFAULT_PSI.warning == 0.10
    assert DEFAULT_PSI.major == 0.25
    assert DEFAULT_PSI.critical == 0.40
    assert DEFAULT_PSI.warning < DEFAULT_PSI.major < DEFAULT_PSI.critical


def test_classify_higher_is_worse():
    th = SeverityThresholds(warning=0.1, major=0.25, critical=0.4)
    assert th.classify(0.05) == "none"
    assert th.classify(0.15) == "warning"
    assert th.classify(0.30) == "major"
    assert th.classify(0.50) == "critical"


def test_classify_lower_is_worse():
    th = DEFAULT_CHI_SQUARED_P
    # p-value ladder : lower = worse
    assert th.classify(0.10) == "none"
    assert th.classify(0.03) == "warning"
    assert th.classify(0.005) == "major"
    assert th.classify(0.0001) == "critical"


def test_classify_missing_levels_skipped():
    th = SeverityThresholds(warning=None, major=0.25, critical=None)
    assert th.classify(0.1) == "none"
    assert th.classify(0.3) == "major"
    assert th.classify(1.0) == "major"   # no critical → clamp at major


def test_thresholds_reject_non_monotone_higher():
    # warning > major in higher_is_worse orientation — invalid.
    with pytest.raises(ValueError):
        SeverityThresholds(warning=0.5, major=0.1)


def test_thresholds_reject_non_monotone_lower():
    # warning < critical in lower_is_worse orientation — invalid.
    with pytest.raises(ValueError):
        SeverityThresholds(
            warning=0.01, major=0.05,
            orientation="lower_is_worse",
        )


def test_all_method_defaults_present():
    required = {
        "psi", "js_divergence", "ks", "chi_squared",
        "wasserstein", "mmd", "classifier_based",
    }
    assert required <= set(DEFAULTS.keys())


def test_classify_monotone_invariant():
    """I3 — for any SeverityThresholds, score ↑ ⇒ severity ↑."""
    th = SeverityThresholds(warning=0.1, major=0.2, critical=0.3)
    ordered = ["none", "warning", "major", "critical"]
    prev_idx = -1
    for score in [0.0, 0.05, 0.15, 0.25, 0.40, 0.5]:
        sev = th.classify(score)
        idx = ordered.index(sev)
        assert idx >= prev_idx
        prev_idx = idx
