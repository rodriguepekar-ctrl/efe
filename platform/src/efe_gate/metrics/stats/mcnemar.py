"""McNemar test for paired binary outcomes.

Given per-sample correctness vectors for champion and candidate on
the SAME sample set, build the 2×2 disagreement table:

                 Candidate CORRECT   Candidate WRONG
    Champion CORRECT      a                  b
    Champion WRONG        c                  d

Under H0 (both models have equal error rate), ``b`` and ``c`` are
expected to be equal. The statistic is

    χ² = (|b - c| - 1)² / (b + c)   (continuity correction)

with one degree of freedom. When ``b + c`` is small (≤ 25 by
convention), use the exact binomial test on ``min(b, c)``.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from scipy.stats import binom, chi2

from efe_gate.errors import InsufficientSample, StatisticalTestError


@dataclass(frozen=True, slots=True)
class McNemarResult:
    statistic: float
    p_value: float
    b: int
    c: int
    test: str  # "chi2" | "binom_exact"
    candidate_better: bool  # c > b → candidate gets more right
    n: int


def mcnemar_test(
    champion_correct: np.ndarray,
    candidate_correct: np.ndarray,
    *,
    exact_threshold: int = 25,
) -> McNemarResult:
    """Run a paired McNemar test on two binary correctness vectors.

    Parameters
    ----------
    champion_correct, candidate_correct
        1-D arrays of 0/1 (or bool), same length, aligned sample-wise.
    exact_threshold
        Use the exact binomial when ``b + c <= exact_threshold``.

    Returns :class:`McNemarResult`.
    """
    x = np.asarray(champion_correct).astype(bool).ravel()
    y = np.asarray(candidate_correct).astype(bool).ravel()
    if x.shape != y.shape:
        raise StatisticalTestError(
            f"shape mismatch: champion {x.shape} vs candidate {y.shape}",
        )
    n = int(x.size)
    if n < 1:
        raise InsufficientSample("McNemar needs at least 1 sample")

    # Disagreement counts
    b = int(np.sum(x & ~y))   # champion right, candidate wrong
    c = int(np.sum(~x & y))   # champion wrong, candidate right

    discord = b + c
    if discord == 0:
        return McNemarResult(
            statistic=0.0, p_value=1.0, b=b, c=c,
            test="chi2", candidate_better=False, n=n,
        )

    if discord <= exact_threshold:
        k = min(b, c)
        # Two-sided: P(X ≤ k) + P(X ≥ n−k) under Binom(n=discord, p=0.5)
        # sf(k-1) gives P(X ≥ k); we use sf on both tails symmetrically.
        p_half = float(binom.cdf(k, discord, 0.5))
        p_value = min(1.0, 2.0 * p_half)
        statistic = float(k)
        test = "binom_exact"
    else:
        stat = (abs(b - c) - 1) ** 2 / discord
        p_value = float(chi2.sf(stat, df=1))
        statistic = float(stat)
        test = "chi2"

    return McNemarResult(
        statistic=statistic,
        p_value=float(p_value),
        b=b, c=c,
        test=test,
        candidate_better=(c > b),
        n=n,
    )


__all__ = ["McNemarResult", "mcnemar_test"]
