"""Sample Ratio Mismatch (SRM) detection.

Every running A/B should receive traffic in the proportions declared
in its :class:`ExperimentSpec`. If the *observed* allocation drifts —
typically because of a bug in the upstream routing layer — the test
becomes biased in a silent way.

:func:`detect_srm` implements the canonical χ² goodness-of-fit test
against the designed allocation. A p-value below ``threshold``
(default 1e-3) flags the experiment. Callers read the flag and can
auto-pause or surface in the dashboards.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

from efe_rollout.analysis.stats import chi2_cdf


@dataclass(frozen=True, slots=True)
class SrmReport:
    chi_squared: float
    p_value: float
    has_mismatch: bool
    expected: dict[str, float]
    observed: dict[str, int]


def detect_srm(
    *,
    observed: Mapping[str, int],
    expected_pct: Mapping[str, float],
    threshold: float = 1e-3,
) -> SrmReport:
    """χ² SRM test.

    ``expected_pct`` values sum to 100 (or 1.0 — we auto-detect). Any
    variant present in ``expected_pct`` but missing from ``observed``
    is treated as zero observations.
    """
    if not expected_pct:
        raise ValueError("expected_pct is empty")
    total_obs = sum(observed.values())
    if total_obs == 0:
        return SrmReport(
            chi_squared=0.0, p_value=1.0,
            has_mismatch=False,
            expected={k: 0.0 for k in expected_pct},
            observed=dict(observed),
        )
    # Normalise expected_pct onto a probability distribution.
    tot = sum(expected_pct.values())
    if tot <= 0:
        raise ValueError("expected_pct must sum to > 0")
    probs = {k: v / tot for k, v in expected_pct.items()}
    chi_sq = 0.0
    expected_abs: dict[str, float] = {}
    for name, p in probs.items():
        e = total_obs * p
        expected_abs[name] = e
        o = observed.get(name, 0)
        if e > 0:
            chi_sq += (o - e) ** 2 / e
    df = max(1, len(probs) - 1)
    p_value = 1.0 - chi2_cdf(chi_sq, float(df))
    return SrmReport(
        chi_squared=chi_sq,
        p_value=p_value,
        has_mismatch=p_value < threshold,
        expected=expected_abs,
        observed=dict(observed),
    )


__all__ = ["SrmReport", "detect_srm"]
