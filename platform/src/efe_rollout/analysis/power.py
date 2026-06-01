"""Power analysis — sample-size computations.

Two canonical use-cases:

* **Binary metric (proportions)** — CTR, conversion, thumbs-up. Uses
  the normal approximation formula common in Kohavi/Bakshy.
* **Continuous metric (means)** — latency, tokens, cost. Same formula
  with variance inputs rather than ``p(1-p)``.

:func:`sample_size_proportions` is the primary API. It takes a baseline
proportion, a *relative* lift target, α, and target power and returns
the per-variant sample size. Callers feed the returned ``n`` into
:attr:`ExperimentSpec.planned_sample_size`.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

from efe_rollout.analysis.stats import normal_ppf


@dataclass(frozen=True, slots=True)
class PowerCalcResult:
    per_variant_sample_size: int
    total_sample_size: int
    detectable_effect_abs: float
    baseline: float
    alpha: float
    power: float


def sample_size_proportions(
    *,
    baseline: float,
    relative_lift: float,
    alpha: float = 0.05,
    power: float = 0.8,
    two_sided: bool = True,
    variants: int = 2,
) -> PowerCalcResult:
    """Per-variant sample size for a two-proportion test.

    Formula (Fleiss 1981, §2.4.3 simplified) :

    ``n = (z_{α/2} + z_β)² · [p₁(1-p₁) + p₂(1-p₂)] / (p₂ - p₁)²``

    ``variants > 2`` uses a Bonferroni tightening of ``α``.
    """
    if not 0.0 < baseline < 1.0:
        raise ValueError(
            f"baseline must be in (0, 1), got {baseline}",
        )
    if relative_lift == 0.0:
        raise ValueError("relative_lift must be non-zero")
    if not 0.0 < alpha < 0.5:
        raise ValueError(f"alpha must be in (0, 0.5), got {alpha}")
    if not 0.5 < power < 1.0:
        raise ValueError(f"power must be in (0.5, 1), got {power}")
    if variants < 2:
        raise ValueError(f"variants must be ≥ 2, got {variants}")
    adj_alpha = alpha / max(1, variants - 1)
    z_alpha = normal_ppf(1.0 - adj_alpha / 2.0) if two_sided \
        else normal_ppf(1.0 - adj_alpha)
    z_beta = normal_ppf(power)
    p1 = baseline
    p2 = baseline * (1.0 + relative_lift)
    if not 0.0 < p2 < 1.0:
        raise ValueError(
            f"computed p2={p2} out of (0, 1) — shrink relative_lift",
        )
    delta = p2 - p1
    num = (z_alpha + z_beta) ** 2 * (p1 * (1.0 - p1) + p2 * (1.0 - p2))
    n = math.ceil(num / (delta ** 2))
    return PowerCalcResult(
        per_variant_sample_size=n,
        total_sample_size=n * variants,
        detectable_effect_abs=abs(delta),
        baseline=baseline,
        alpha=alpha,
        power=power,
    )


def sample_size_means(
    *,
    sigma: float,
    minimum_detectable_effect: float,
    alpha: float = 0.05,
    power: float = 0.8,
    two_sided: bool = True,
    variants: int = 2,
) -> PowerCalcResult:
    """Per-variant sample size for a two-mean test (equal variance).

    ``n = 2·σ² · (z_{α/2} + z_β)² / δ²``
    """
    if sigma <= 0.0:
        raise ValueError(f"sigma must be positive, got {sigma}")
    if minimum_detectable_effect == 0.0:
        raise ValueError("minimum_detectable_effect must be non-zero")
    if variants < 2:
        raise ValueError(f"variants must be ≥ 2, got {variants}")
    adj_alpha = alpha / max(1, variants - 1)
    z_alpha = normal_ppf(1.0 - adj_alpha / 2.0) if two_sided \
        else normal_ppf(1.0 - adj_alpha)
    z_beta = normal_ppf(power)
    delta = abs(minimum_detectable_effect)
    n = math.ceil(
        2.0 * sigma * sigma * (z_alpha + z_beta) ** 2 / (delta ** 2),
    )
    return PowerCalcResult(
        per_variant_sample_size=n,
        total_sample_size=n * variants,
        detectable_effect_abs=delta,
        baseline=float("nan"),
        alpha=alpha,
        power=power,
    )


def detectable_effect_proportions(
    *, baseline: float, n_per_variant: int,
    alpha: float = 0.05, power: float = 0.8, two_sided: bool = True,
) -> float:
    """Invert :func:`sample_size_proportions` — given ``n``, return the
    minimum detectable effect (relative).
    """
    if n_per_variant <= 0:
        raise ValueError("n_per_variant must be positive")
    if not 0.0 < baseline < 1.0:
        raise ValueError(f"baseline out of range: {baseline}")
    z_alpha = normal_ppf(1.0 - alpha / 2.0) if two_sided \
        else normal_ppf(1.0 - alpha)
    z_beta = normal_ppf(power)
    # Solve for δ s.t. n = (z_α+z_β)² · (2·p(1-p)) / δ²  (pooled var)
    num = (z_alpha + z_beta) ** 2 * 2.0 * baseline * (1.0 - baseline)
    delta = math.sqrt(num / n_per_variant)
    return delta / baseline


__all__ = [
    "PowerCalcResult",
    "detectable_effect_proportions",
    "sample_size_means",
    "sample_size_proportions",
]
