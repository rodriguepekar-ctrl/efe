"""Bayesian beta-binomial analyser.

Conjugate updating : ``Beta(α, β)`` prior + ``k`` successes out of
``n`` trials → ``Beta(α + k, β + n - k)`` posterior.

For two arms with binomial outcomes we compute three quantities by
Monte-Carlo sampling :

1. ``P(θ_B > θ_A)`` — the direct probability that the treatment is
   better than the control.
2. Expected lift ``E[(θ_B - θ_A) / θ_A]`` (when ``θ_A > 0``).
3. 95 % credible interval on ``θ_B - θ_A``.

We use a deterministic :class:`random.Random` seeded per call so two
identical inputs yield identical outputs — crucial for deterministic
tests and for reproducing an analyst's verdict months later.
"""

from __future__ import annotations

import math
import random
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class BayesianPosterior:
    prob_b_gt_a: float
    expected_lift: float
    credible_interval_95: tuple[float, float]
    n_samples: int


def _sample_beta(alpha: float, beta: float, rng: random.Random) -> float:
    """Draw one sample from Beta(α, β).

    Uses :meth:`random.Random.betavariate`, which is the Cheng (1978)
    BB/BC algorithm — correct over the full (0, ∞) parameter range.
    """
    return rng.betavariate(alpha, beta)


def bayesian_beta_binomial(
    *,
    s_a: int, n_a: int, s_b: int, n_b: int,
    prior_alpha: float = 1.0,
    prior_beta: float = 1.0,
    n_samples: int = 50_000,
    seed: int = 20260420,
) -> BayesianPosterior:
    if n_a < 0 or n_b < 0 or s_a < 0 or s_b < 0:
        raise ValueError("negative counts are invalid")
    if s_a > n_a or s_b > n_b:
        raise ValueError("successes cannot exceed trials")
    if prior_alpha <= 0 or prior_beta <= 0:
        raise ValueError("prior params must be positive")
    if n_samples < 100:
        raise ValueError("too few samples for stable estimates")
    rng = random.Random(seed)
    a_alpha = prior_alpha + s_a
    a_beta = prior_beta + (n_a - s_a)
    b_alpha = prior_alpha + s_b
    b_beta = prior_beta + (n_b - s_b)
    diffs: list[float] = []
    lifts: list[float] = []
    b_better = 0
    for _ in range(n_samples):
        ta = _sample_beta(a_alpha, a_beta, rng)
        tb = _sample_beta(b_alpha, b_beta, rng)
        if tb > ta:
            b_better += 1
        diff = tb - ta
        diffs.append(diff)
        if ta > 1e-12:
            lifts.append(diff / ta)
    prob_b_gt_a = b_better / n_samples
    expected_lift = (
        sum(lifts) / len(lifts) if lifts else 0.0
    )
    diffs.sort()
    lo = diffs[math.floor(0.025 * n_samples)]
    hi = diffs[math.ceil(0.975 * n_samples) - 1]
    return BayesianPosterior(
        prob_b_gt_a=prob_b_gt_a,
        expected_lift=expected_lift,
        credible_interval_95=(lo, hi),
        n_samples=n_samples,
    )


def bayesian_decision(
    posterior: BayesianPosterior,
    *,
    ship_threshold: float = 0.95,
    rollback_threshold: float = 0.05,
) -> str:
    """Map a posterior to a suggested decision.

    * ``P(B > A) ≥ ship_threshold`` → ``'ship'``.
    * ``P(B > A) ≤ rollback_threshold`` → ``'rollback'``.
    * otherwise → ``'continue'`` (not yet inconclusive — needs more data).
    """
    if posterior.prob_b_gt_a >= ship_threshold:
        return "ship"
    if posterior.prob_b_gt_a <= rollback_threshold:
        return "rollback"
    return "continue"


__all__ = ["BayesianPosterior", "bayesian_beta_binomial", "bayesian_decision"]
