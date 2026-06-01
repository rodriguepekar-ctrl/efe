"""Cluster-robust standard errors (Huber-White sandwich).

When the randomisation unit is a *user* but outcomes are recorded at
*request* level (one user ⟶ many requests), the naive SE treats each
request as an independent draw and is therefore too small. Cluster-
robust SE fixes this by aggregating within-cluster residuals before
sandwiching the OLS variance.

The estimator implemented here assumes a simple treatment-vs-control
regression ``y ~ β·d`` where ``d`` is 0/1. For more than two groups
callers are expected to one-hot encode upstream (the rollout pipeline
only supports two-arm tests in MVP anyway).

Reference :
  * Huber (1967), White (1980), Liang-Zeger (1986).
  * `statsmodels.OLS(...).fit(cov_type='cluster')` — our output matches
    within 1e-6.
"""

from __future__ import annotations

import math
from collections import defaultdict
from collections.abc import Sequence
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ClusterRobustResult:
    beta: float                       # estimated treatment effect
    se: float                         # cluster-robust SE
    t_stat: float
    n_clusters: int


def cluster_robust_se(
    *,
    y: Sequence[float],
    d: Sequence[int],
    clusters: Sequence[str],
) -> ClusterRobustResult:
    """Two-arm cluster-robust OLS for ``y ~ intercept + β·d``."""
    n = len(y)
    if n == 0 or n != len(d) or n != len(clusters):
        raise ValueError("y, d, clusters must have same positive length")
    # Intercept + β·d via the normal equations; no matrix dependency.
    sum_d = sum(d)
    sum_d2 = sum_d  # d ∈ {0,1} so d² = d
    sum_y = sum(y)
    sum_dy = sum(y[i] * d[i] for i in range(n))
    denom = n * sum_d2 - sum_d * sum_d
    if denom == 0:
        return ClusterRobustResult(
            beta=float("nan"), se=float("nan"),
            t_stat=float("nan"), n_clusters=0,
        )
    beta = (n * sum_dy - sum_d * sum_y) / denom
    alpha = (sum_y - beta * sum_d) / n
    # Residuals + cluster-level score aggregation.
    resid = [y[i] - alpha - beta * d[i] for i in range(n)]
    # X = [1, d_i]; we need (X'X)^{-1} and Σ X_g' u_g u_g' X_g.
    # Build cluster aggregates for u_g · (sum within cluster of x_i u_i).
    agg_intercept: dict[str, float] = defaultdict(float)
    agg_d: dict[str, float] = defaultdict(float)
    for i in range(n):
        agg_intercept[clusters[i]] += resid[i]
        agg_d[clusters[i]] += resid[i] * d[i]
    # "Meat" of the sandwich: Σ_g (X_g' u_g)(X_g' u_g)'
    m11 = sum(v * v for v in agg_intercept.values())
    m12 = sum(
        agg_intercept[g] * agg_d[g]
        for g in agg_intercept
    )
    m22 = sum(v * v for v in agg_d.values())
    # (X'X)^{-1}
    det_xtx = denom
    xtx_inv_12 = -sum_d / det_xtx
    xtx_inv_22 = n / det_xtx
    # sandwich = (X'X)^{-1} · M · (X'X)^{-1}
    # Only the (1,1)_β element of the inverse matters for SE(β).
    # Compute the full [β row]_resulting element:
    r_beta_1 = xtx_inv_12 * m11 + xtx_inv_22 * m12
    r_beta_2 = xtx_inv_12 * m12 + xtx_inv_22 * m22
    var_beta = r_beta_1 * xtx_inv_12 + r_beta_2 * xtx_inv_22
    # Small-sample DOF correction for cluster-robust (Stata default).
    n_groups = len(agg_intercept)
    if n_groups > 1 and n > 2:
        var_beta *= (n_groups / (n_groups - 1)) * ((n - 1) / (n - 2))
    se = math.sqrt(max(0.0, var_beta))
    t_stat = beta / se if se > 0 else float("nan")
    return ClusterRobustResult(
        beta=beta, se=se, t_stat=t_stat, n_clusters=n_groups,
    )


__all__ = ["ClusterRobustResult", "cluster_robust_se"]
