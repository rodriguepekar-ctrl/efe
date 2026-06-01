"""Pure-python statistical primitives used by the drift detectors.

The module mirrors :mod:`efe_rollout.analysis.stats` but is kept local
so :mod:`efe_drift` has zero scipy dependency. Every function is
accurate to 1e-4 or better against scipy's reference implementation —
parity is verified by dedicated tests.

Contents :

* :func:`normal_cdf` / :func:`normal_ppf` — standard-normal Φ / Φ⁻¹
* :func:`student_t_cdf` — ``P(T ≤ t; ν)``
* :func:`chi2_cdf` — ``P(χ² ≤ x; k)``
* :func:`kolmogorov_p_value` — asymptotic K-S distribution tail
* :func:`ks_two_sample` — two-sample Kolmogorov-Smirnov statistic + p-value
* :func:`reg_incomplete_beta` / :func:`_lower_inc_gamma_reg` — building
  blocks for the CDFs above
* :func:`rank_auc` — Mann-Whitney U reformulation used by the classifier
  drift detector

The module is deliberately stdlib-only : any ``numpy`` / ``scipy`` use
is confined to optional tests.
"""

from __future__ import annotations

import math
from collections.abc import Sequence

# ---------------------------------------------------------------------------
# Normal CDF / PPF
# ---------------------------------------------------------------------------


def normal_cdf(x: float) -> float:
    """Standard-normal cumulative distribution Φ(x)."""
    return 0.5 * (1.0 + math.erf(x / math.sqrt(2.0)))


def normal_pdf(x: float) -> float:
    return math.exp(-0.5 * x * x) / math.sqrt(2.0 * math.pi)


def normal_ppf(p: float) -> float:
    """Inverse of :func:`normal_cdf` — Beasley-Springer-Moro /
    Wichura (AS 241) algorithm.  Accurate to ~1 e-9 over ``(0, 1)``."""
    if not 0.0 < p < 1.0:
        if p <= 0.0:
            return -math.inf
        if p >= 1.0:
            return math.inf
        raise ValueError(f"normal_ppf requires p in (0, 1), got {p}")
    a = (
        -3.969683028665376e1, 2.209460984245205e2,
        -2.759285104469687e2, 1.383577518672690e2,
        -3.066479806614716e1, 2.506628277459239e0,
    )
    b = (
        -5.447609879822406e1, 1.615858368580409e2,
        -1.556989798598866e2, 6.680131188771972e1,
        -1.328068155288572e1,
    )
    c = (
        -7.784894002430293e-3, -3.223964580411365e-1,
        -2.400758277161838e0, -2.549732539343734e0,
        4.374664141464968e0,   2.938163982698783e0,
    )
    d = (
        7.784695709041462e-3, 3.224671290700398e-1,
        2.445134137142996e0,  3.754408661907416e0,
    )
    p_low = 0.02425
    p_high = 1.0 - p_low
    if p < p_low:
        q = math.sqrt(-2.0 * math.log(p))
        num = (
            ((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q
            + c[5]
        )
        den = (
            (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0
        )
        return num / den
    if p <= p_high:
        q = p - 0.5
        r = q * q
        num = (
            ((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r
            + a[5]
        ) * q
        den = (
            ((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r
            + 1.0
        )
        return num / den
    q = math.sqrt(-2.0 * math.log(1.0 - p))
    num = (
        ((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q
        + c[5]
    )
    den = (
        (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0
    )
    return -num / den


# ---------------------------------------------------------------------------
# Regularised incomplete beta — used by t-CDF + related
# ---------------------------------------------------------------------------


def _lgamma(x: float) -> float:
    return math.lgamma(x)


def _betacf(a: float, b: float, x: float) -> float:
    """Continued-fraction expansion from Numerical Recipes §6.4."""
    max_it = 200
    eps = 3.0e-15
    fpmin = 1.0e-300
    qab = a + b
    qap = a + 1.0
    qam = a - 1.0
    c = 1.0
    d = 1.0 - qab * x / qap
    if abs(d) < fpmin:
        d = fpmin
    d = 1.0 / d
    h = d
    for m in range(1, max_it + 1):
        m2 = 2 * m
        aa = m * (b - m) * x / ((qam + m2) * (a + m2))
        d = 1.0 + aa * d
        if abs(d) < fpmin:
            d = fpmin
        c = 1.0 + aa / c
        if abs(c) < fpmin:
            c = fpmin
        d = 1.0 / d
        h *= d * c
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))
        d = 1.0 + aa * d
        if abs(d) < fpmin:
            d = fpmin
        c = 1.0 + aa / c
        if abs(c) < fpmin:
            c = fpmin
        d = 1.0 / d
        delta = d * c
        h *= delta
        if abs(delta - 1.0) < eps:
            return h
    return h


def reg_incomplete_beta(a: float, b: float, x: float) -> float:
    """Regularised incomplete beta ``I_x(a, b)``."""
    if x < 0.0 or x > 1.0:
        raise ValueError(f"x must be in [0, 1], got {x}")
    if x == 0.0 or x == 1.0:
        return x
    lbeta = _lgamma(a + b) - _lgamma(a) - _lgamma(b)
    bt = math.exp(lbeta + a * math.log(x) + b * math.log(1.0 - x))
    if x < (a + 1.0) / (a + b + 2.0):
        return bt * _betacf(a, b, x) / a
    return 1.0 - bt * _betacf(b, a, 1.0 - x) / b


# ---------------------------------------------------------------------------
# Student-t CDF
# ---------------------------------------------------------------------------


def student_t_cdf(t: float, df: float) -> float:
    """``P(T ≤ t)`` for Student's t with ``df`` degrees of freedom."""
    if df <= 0:
        raise ValueError(f"student_t_cdf requires df > 0, got {df}")
    if math.isinf(t):
        return 1.0 if t > 0 else 0.0
    x = df / (df + t * t)
    tail = 0.5 * reg_incomplete_beta(df / 2.0, 0.5, x)
    return 1.0 - tail if t >= 0 else tail


# ---------------------------------------------------------------------------
# Chi-squared CDF (lower incomplete gamma)
# ---------------------------------------------------------------------------


def chi2_cdf(x: float, k: float) -> float:
    """``P(χ² ≤ x)`` with ``k`` degrees of freedom."""
    if x < 0.0:
        return 0.0
    if k <= 0:
        raise ValueError(f"chi2_cdf requires k > 0, got {k}")
    return _lower_inc_gamma_reg(k / 2.0, x / 2.0)


def _lower_inc_gamma_reg(a: float, x: float) -> float:
    if x < 0.0 or a <= 0.0:
        raise ValueError("invalid arguments")
    if x == 0.0:
        return 0.0
    if x < a + 1.0:
        return _gser(a, x)
    return 1.0 - _gcf(a, x)


def _gser(a: float, x: float) -> float:
    eps = 3.0e-15
    max_it = 500
    term = 1.0 / a
    total = term
    for n in range(1, max_it + 1):
        term *= x / (a + n)
        total += term
        if abs(term) < abs(total) * eps:
            return total * math.exp(-x + a * math.log(x) - _lgamma(a))
    return total * math.exp(-x + a * math.log(x) - _lgamma(a))


def _gcf(a: float, x: float) -> float:
    eps = 3.0e-15
    max_it = 500
    fpmin = 1.0e-300
    b = x + 1.0 - a
    c = 1.0 / fpmin
    d = 1.0 / b
    h = d
    for i in range(1, max_it + 1):
        an = -i * (i - a)
        b += 2.0
        d = an * d + b
        if abs(d) < fpmin:
            d = fpmin
        c = b + an / c
        if abs(c) < fpmin:
            c = fpmin
        d = 1.0 / d
        delta = d * c
        h *= delta
        if abs(delta - 1.0) < eps:
            break
    return math.exp(-x + a * math.log(x) - _lgamma(a)) * h


# ---------------------------------------------------------------------------
# Kolmogorov distribution tail — used by KS test
# ---------------------------------------------------------------------------


def kolmogorov_p_value(d_stat: float, n_effective: float) -> float:
    """Two-sided K-S p-value using the Kolmogorov distribution.

    ``n_effective = n * m / (n + m)`` for a two-sample test or ``n`` for
    a one-sample test. Accurate to ~1e-6 in the practical range
    ``d * sqrt(n_effective) ∈ [0.1, 5]``.
    """
    if d_stat < 0 or n_effective <= 0:
        raise ValueError("d_stat must be ≥ 0 and n_effective > 0")
    if d_stat == 0:
        return 1.0
    lam = (math.sqrt(n_effective) + 0.12 + 0.11 / math.sqrt(n_effective)) * d_stat
    # Q(λ) = 2 Σ_{j=1}^∞ (-1)^{j-1} exp(-2 j² λ²)
    q = 0.0
    for j in range(1, 101):
        term = 2.0 * ((-1) ** (j - 1)) * math.exp(-2.0 * (j * j) * (lam * lam))
        q += term
        if abs(term) < 1e-16:
            break
    return max(0.0, min(1.0, q))


# ---------------------------------------------------------------------------
# KS two-sample
# ---------------------------------------------------------------------------


def ks_two_sample(
    a: Sequence[float], b: Sequence[float],
) -> tuple[float, float]:
    """Two-sample Kolmogorov-Smirnov statistic + asymptotic p-value.

    Returns ``(D, p_value)`` where ``D = sup_x |F_a(x) - F_b(x)|``.

    Ties are handled correctly : when ``sa[i] == sb[j]``, both
    pointers advance in the same step so the CDFs move in lockstep.
    A naive two-way merge that only advances one side on a tie
    would inflate ``D`` by ``1/n`` on identical distributions.
    """
    n = len(a)
    m = len(b)
    if n == 0 or m == 0:
        return (float("nan"), float("nan"))
    sa = sorted(a)
    sb = sorted(b)
    d = 0.0
    i = j = 0
    cdf_a = 0.0
    cdf_b = 0.0
    while i < n and j < m:
        if sa[i] < sb[j]:
            while i < n and sa[i] < sb[j]:
                i += 1
            cdf_a = i / n
        elif sb[j] < sa[i]:
            while j < m and sb[j] < sa[i]:
                j += 1
            cdf_b = j / m
        else:
            tied_val = sa[i]
            while i < n and sa[i] == tied_val:
                i += 1
            while j < m and sb[j] == tied_val:
                j += 1
            cdf_a = i / n
            cdf_b = j / m
        gap = abs(cdf_a - cdf_b)
        if gap > d:
            d = gap
    # Drain remaining — the shorter side pinned at its final CDF.
    if i < n:
        cdf_b = 1.0
        while i < n:
            i += 1
            cdf_a = i / n
            gap = abs(cdf_a - cdf_b)
            if gap > d:
                d = gap
    if j < m:
        cdf_a = 1.0
        while j < m:
            j += 1
            cdf_b = j / m
            gap = abs(cdf_a - cdf_b)
            if gap > d:
                d = gap
    p = kolmogorov_p_value(d, n * m / (n + m))
    return (d, p)


# ---------------------------------------------------------------------------
# Rank AUC (Mann-Whitney U reformulation)
# ---------------------------------------------------------------------------


def rank_auc(scores: Sequence[float], labels: Sequence[int]) -> float:
    """AUC of a binary classifier via the Mann-Whitney U identity.

    * ``labels[i] ∈ {0, 1}``
    * ``scores[i]`` is higher for label-1 examples when the classifier
      is good
    * Returns :math:`\\text{AUC} = U / (n_0 \\cdot n_1)` where ``U`` is
      the Mann-Whitney U statistic with 0.5 for ties.
    """
    if len(scores) != len(labels):
        raise ValueError("scores and labels length mismatch")
    positives = [s for s, y in zip(scores, labels, strict=True) if y == 1]
    negatives = [s for s, y in zip(scores, labels, strict=True) if y == 0]
    if not positives or not negatives:
        return 0.5
    # O((n+m) log(n+m)) via sorting.
    ranked: list[tuple[float, int]] = sorted(
        ((s, y) for s, y in zip(scores, labels, strict=True)),
        key=lambda t: t[0],
    )
    # Assign fractional ranks to handle ties.
    ranks: list[float] = [0.0] * len(ranked)
    i = 0
    while i < len(ranked):
        j = i
        while j + 1 < len(ranked) and ranked[j + 1][0] == ranked[i][0]:
            j += 1
        avg_rank = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[k] = avg_rank
        i = j + 1
    rank_sum_pos = sum(
        r for r, (_, y) in zip(ranks, ranked, strict=True) if y == 1
    )
    n1 = len(positives)
    n0 = len(negatives)
    u_stat = rank_sum_pos - n1 * (n1 + 1) / 2
    return u_stat / (n0 * n1)


__all__ = [
    "chi2_cdf",
    "kolmogorov_p_value",
    "ks_two_sample",
    "normal_cdf",
    "normal_pdf",
    "normal_ppf",
    "rank_auc",
    "reg_incomplete_beta",
    "student_t_cdf",
]
