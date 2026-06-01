"""Welch's t-test for unequal variances.

Used for latency distributions which are non-normal and have
very different variances between champion and candidate. The
alternative hypothesis defaults to ``"greater"`` — the candidate is
*significantly worse* (i.e. larger latency) — so that the test
aligns with "budget breaches"; flip via ``alternative=``.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

import numpy as np
from scipy.stats import ttest_ind

from efe_gate.errors import InsufficientSample

Alternative = Literal["two-sided", "greater", "less"]


@dataclass(frozen=True, slots=True)
class WelchResult:
    statistic: float
    p_value: float
    candidate_mean: float
    champion_mean: float
    candidate_std: float
    champion_std: float
    n_candidate: int
    n_champion: int
    alternative: str


def welch_test(
    champion: np.ndarray,
    candidate: np.ndarray,
    *,
    alternative: Alternative = "greater",
) -> WelchResult:
    """Welch t-test on two independent samples."""
    a = np.asarray(champion, dtype=np.float64).ravel()
    b = np.asarray(candidate, dtype=np.float64).ravel()
    if a.size < 2 or b.size < 2:
        raise InsufficientSample(
            "Welch t-test needs ≥ 2 samples per group",
        )
    # scipy ttest_ind(a=cand, b=champ): positive statistic means
    # cand > champ. With ``alternative="greater"`` we answer "is the
    # candidate significantly larger?". That's our budget-regression
    # framing.
    res = ttest_ind(b, a, equal_var=False, alternative=alternative)
    return WelchResult(
        statistic=float(res.statistic),
        p_value=float(res.pvalue),
        candidate_mean=float(b.mean()),
        champion_mean=float(a.mean()),
        candidate_std=float(b.std(ddof=1)),
        champion_std=float(a.std(ddof=1)),
        n_candidate=int(b.size),
        n_champion=int(a.size),
        alternative=alternative,
    )


__all__ = ["Alternative", "WelchResult", "welch_test"]
