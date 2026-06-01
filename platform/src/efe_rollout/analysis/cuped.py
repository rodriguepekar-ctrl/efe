"""CUPED variance reduction (Deng et al., 2013).

Given outcomes ``Y`` and a pre-experiment covariate ``X`` uncorrelated
with the treatment assignment, the adjusted estimator
``Y_adj = Y - θ (X - mean(X))`` with ``θ = cov(Y, X) / var(X)`` has
variance ``(1 - ρ²) var(Y)`` where ``ρ = corr(Y, X)``.

Applied *before* any downstream t-test, CUPED typically shrinks the CI
25-50 % without changing the point estimate — the only free lunch in
experimentation.

Edge cases :

* If ``var(X) == 0``, CUPED degenerates into a no-op (θ undefined).
  We return the raw ``Y`` unchanged.
* If ``X`` and ``Y`` have different lengths, we raise.
* NaN pass-through: if any element of X is NaN, θ is computed on the
  subset of pairs where both are finite.
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class CupedResult:
    y_adj: tuple[float, ...]
    theta: float
    variance_reduction: float   # (var(Y) - var(Y_adj)) / var(Y)


def _mean(xs: Sequence[float]) -> float:
    if not xs:
        return float("nan")
    return sum(xs) / len(xs)


def _var(xs: Sequence[float], *, mean: float | None = None) -> float:
    n = len(xs)
    if n < 2:
        return 0.0
    m = _mean(xs) if mean is None else mean
    return sum((x - m) ** 2 for x in xs) / (n - 1)


def _cov(
    xs: Sequence[float], ys: Sequence[float],
    *, x_mean: float | None = None, y_mean: float | None = None,
) -> float:
    n = len(xs)
    if n < 2:
        return 0.0
    xm = _mean(xs) if x_mean is None else x_mean
    ym = _mean(ys) if y_mean is None else y_mean
    return sum((xs[i] - xm) * (ys[i] - ym) for i in range(n)) / (n - 1)


def apply_cuped(
    y: Sequence[float], x: Sequence[float],
) -> CupedResult:
    if len(y) != len(x):
        raise ValueError(
            f"CUPED requires len(y) == len(x); got {len(y)} vs {len(x)}",
        )
    # Filter pairs where either is NaN
    paired = [
        (yi, xi) for yi, xi in zip(y, x, strict=True)
        if not (math.isnan(yi) or math.isnan(xi))
    ]
    if len(paired) < 2:
        return CupedResult(
            y_adj=tuple(y),
            theta=0.0,
            variance_reduction=0.0,
        )
    y_clean = [p[0] for p in paired]
    x_clean = [p[1] for p in paired]
    var_x = _var(x_clean)
    if var_x == 0.0:
        return CupedResult(
            y_adj=tuple(y),
            theta=0.0,
            variance_reduction=0.0,
        )
    mean_x = _mean(x_clean)
    theta = _cov(y_clean, x_clean) / var_x
    y_adj_full = tuple(
        yi - theta * (xi - mean_x)
        if not (math.isnan(yi) or math.isnan(xi))
        else yi
        for yi, xi in zip(y, x, strict=True)
    )
    var_raw = _var(y_clean)
    var_adj = _var([yi - theta * (xi - mean_x)
                    for yi, xi in zip(y_clean, x_clean, strict=True)])
    reduction = 0.0 if var_raw == 0.0 else 1.0 - (var_adj / var_raw)
    return CupedResult(
        y_adj=y_adj_full,
        theta=theta,
        variance_reduction=reduction,
    )


__all__ = ["CupedResult", "apply_cuped"]
