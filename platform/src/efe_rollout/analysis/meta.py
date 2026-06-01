"""DerSimonian-Laird random-effects meta-analysis.

When several past experiments on the same ``(tenant, task, metric)``
are pooled, we want a principled weighted average of their ATEs that
accounts for *between-study* variance ``τ²`` alongside within-study
variance.

:func:`meta_analyze` implements the DL estimator :

1. Compute fixed-effects weights ``w_i = 1 / var_i``.
2. Compute DL estimator of τ² from Q statistic:

       Q = Σ w_i (θ_i - θ̄_FE)²
       τ² = max(0, (Q - (k-1)) / (Σ w_i - Σ w_i² / Σ w_i))

3. Recompute random-effects weights ``w_i* = 1 / (var_i + τ²)``.
4. Return pooled estimate ``Σ w_i* θ_i / Σ w_i*`` + its SE.

This is the exact algorithm implemented by R's ``metafor::rma(..., method="DL")``.
"""

from __future__ import annotations

import math
from collections.abc import Iterable
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class StudySummary:
    study_id: str
    estimate: float
    variance: float          # σ²_i of the estimator


@dataclass(frozen=True, slots=True)
class MetaAnalysisResult:
    pooled_estimate: float
    pooled_se: float
    q_statistic: float
    tau_squared: float
    i_squared: float         # heterogeneity share in [0, 1]
    k: int                   # number of studies


def meta_analyze(studies: Iterable[StudySummary]) -> MetaAnalysisResult:
    studies = tuple(studies)
    k = len(studies)
    if k == 0:
        raise ValueError("meta_analyze requires at least one study")
    # Filter out zero-variance studies (happens on degenerate CIs).
    finite = [
        s for s in studies
        if s.variance > 0 and not math.isnan(s.estimate)
    ]
    if not finite:
        raise ValueError("no studies with positive variance")
    # Fixed-effects weights + pooled mean.
    w = [1.0 / s.variance for s in finite]
    sum_w = sum(w)
    pooled_fe = sum(w[i] * finite[i].estimate for i in range(len(finite))) / sum_w
    # Q statistic
    q = sum(w[i] * (finite[i].estimate - pooled_fe) ** 2 for i in range(len(finite)))
    k_eff = len(finite)
    if k_eff == 1:
        return MetaAnalysisResult(
            pooled_estimate=pooled_fe,
            pooled_se=math.sqrt(1.0 / sum_w),
            q_statistic=0.0,
            tau_squared=0.0,
            i_squared=0.0,
            k=1,
        )
    sum_w2 = sum(wi * wi for wi in w)
    denom = sum_w - sum_w2 / sum_w
    tau_sq = 0.0 if denom <= 0 else max(0.0, (q - (k_eff - 1)) / denom)
    # Random-effects weights.
    w_star = [1.0 / (s.variance + tau_sq) for s in finite]
    sum_w_star = sum(w_star)
    pooled_re = sum(
        w_star[i] * finite[i].estimate for i in range(len(finite))
    ) / sum_w_star
    se_re = math.sqrt(1.0 / sum_w_star)
    # I² = max(0, (Q - (k-1)) / Q) ∈ [0, 1]
    i2 = 0.0 if q <= 0 else max(0.0, (q - (k_eff - 1)) / q)
    return MetaAnalysisResult(
        pooled_estimate=pooled_re,
        pooled_se=se_re,
        q_statistic=q,
        tau_squared=tau_sq,
        i_squared=i2,
        k=k_eff,
    )


__all__ = ["MetaAnalysisResult", "StudySummary", "meta_analyze"]
