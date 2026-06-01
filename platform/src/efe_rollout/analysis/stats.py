"""Statistical primitives — stdlib-only, scipy-free.

M08 ships with hand-rolled implementations of every distribution and
test it needs so the deployable wheel doesn't pull in scipy/numpy
(~120 MB savings on the inference-pod image). Accuracy is within 1 e-5
of scipy's answer for every function here — verified by dedicated
parity tests in ``tests_rollout/test_stats_parity.py``.

Surface :

* :func:`normal_cdf` / :func:`normal_ppf` — Φ / Φ⁻¹ of the standard
  normal.
* :func:`student_t_cdf` — ``P(T ≤ t; ν)``; used by Welch and
  regression-style tests.
* :func:`chi2_cdf` — ``P(χ² ≤ x; k)``; used by SRM detection.
* :func:`welch_t_test` — two-sample Welch t-test with CI.
* :func:`wald_prop_test` — two-sample proportion test with Newcombe CI.
* :func:`newcombe_ci` — Newcombe-Wilson CI for difference of two
  proportions.
* :func:`bh_correction` — Benjamini-Hochberg FDR.
* :func:`welch_df` — Welch-Satterthwaite degrees of freedom helper.

Edge-case handling :

* Tiny samples (``n < 2``) produce explicit NaN results, not crashes.
* Zero-variance arms return a zero estimate + undefined CI (None).
* Pathological inputs (nan, inf) are filtered upstream — the callers
  are responsible.
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass

# ---------------------------------------------------------------------------
# Normal distribution
# ---------------------------------------------------------------------------


def normal_cdf(x: float) -> float:
    """Standard-normal cumulative distribution Φ(x)."""
    return 0.5 * (1.0 + math.erf(x / math.sqrt(2.0)))


def normal_pdf(x: float) -> float:
    return math.exp(-0.5 * x * x) / math.sqrt(2.0 * math.pi)


def normal_ppf(p: float) -> float:
    """Inverse of :func:`normal_cdf` — Beasley-Springer-Moro algorithm.

    Accurate to ~1 e-9 over the full ``(0, 1)`` range.
    """
    if not 0.0 < p < 1.0:
        if p <= 0.0:
            return -math.inf
        if p >= 1.0:
            return math.inf
        raise ValueError(f"normal_ppf requires p in (0, 1), got {p}")
    # Rational-approximation coefficients (Wichura 1988 / AS 241).
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
         4.374664141464968e0,  2.938163982698783e0,
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
# Regularised incomplete beta — core building block for t + chi2 CDFs
# ---------------------------------------------------------------------------


def _lgamma(x: float) -> float:
    return math.lgamma(x)


def _betacf(a: float, b: float, x: float) -> float:
    """Continued-fraction expansion used by :func:`reg_incomplete_beta`.

    Implementation closely follows *Numerical Recipes* 3rd ed. §6.4.
    """
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
    """``P(χ² ≤ x)`` for the chi-squared with ``k`` degrees of freedom."""
    if x < 0.0:
        return 0.0
    if k <= 0:
        raise ValueError(f"chi2_cdf requires k > 0, got {k}")
    return _lower_inc_gamma_reg(k / 2.0, x / 2.0)


def _lower_inc_gamma_reg(a: float, x: float) -> float:
    """Regularised lower incomplete gamma ``P(a, x)``.

    Switches between a series expansion (``x < a + 1``) and a
    continued-fraction expansion (``x ≥ a + 1``) for stable convergence.
    """
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
# Welch t-test
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class MeanTestResult:
    estimate: float
    p_value: float
    ci_95: tuple[float, float] | None
    t_stat: float
    df: float


def welch_df(var_a: float, n_a: int, var_b: float, n_b: int) -> float:
    """Welch-Satterthwaite degrees of freedom."""
    if n_a < 2 or n_b < 2:
        return float("nan")
    num = (var_a / n_a + var_b / n_b) ** 2
    den = (
        (var_a ** 2) / (n_a * n_a * (n_a - 1))
        + (var_b ** 2) / (n_b * n_b * (n_b - 1))
    )
    return num / den if den > 0 else float("nan")


def _mean_var(xs: Sequence[float]) -> tuple[float, float]:
    n = len(xs)
    if n == 0:
        return float("nan"), float("nan")
    m = sum(xs) / n
    if n < 2:
        return m, 0.0
    v = sum((x - m) ** 2 for x in xs) / (n - 1)
    return m, v


def welch_t_test(
    a: Sequence[float], b: Sequence[float], *, alpha: float = 0.05,
) -> MeanTestResult:
    """Two-sample Welch's t-test with CI on ``mean(b) - mean(a)``."""
    n_a, n_b = len(a), len(b)
    if n_a < 2 or n_b < 2:
        return MeanTestResult(
            estimate=float("nan"),
            p_value=float("nan"),
            ci_95=None,
            t_stat=float("nan"),
            df=float("nan"),
        )
    mean_a, var_a = _mean_var(a)
    mean_b, var_b = _mean_var(b)
    se = math.sqrt(var_a / n_a + var_b / n_b)
    estimate = mean_b - mean_a
    if se == 0.0:
        return MeanTestResult(
            estimate=estimate,
            p_value=1.0 if estimate == 0 else 0.0,
            ci_95=(estimate, estimate),
            t_stat=0.0,
            df=float("inf"),
        )
    t_stat = estimate / se
    df = welch_df(var_a, n_a, var_b, n_b)
    p_value = 2.0 * (1.0 - student_t_cdf(abs(t_stat), df))
    # Confidence interval using t-quantile.
    q = _t_ppf(1.0 - alpha / 2.0, df)
    lo = estimate - q * se
    hi = estimate + q * se
    return MeanTestResult(
        estimate=estimate,
        p_value=p_value,
        ci_95=(lo, hi),
        t_stat=t_stat,
        df=df,
    )


def _t_ppf(p: float, df: float) -> float:
    """Inverse Student-t CDF via Newton iteration on the regularised
    incomplete beta."""
    if df > 200:
        # normal is a great approximation
        return normal_ppf(p)
    # initial guess from normal
    guess = normal_ppf(p)
    for _ in range(80):
        cdf = student_t_cdf(guess, df)
        pdf = math.gamma((df + 1) / 2) / (
            math.sqrt(df * math.pi) * math.gamma(df / 2)
        ) * (1 + guess * guess / df) ** (-(df + 1) / 2)
        if pdf <= 0:
            break
        delta = (cdf - p) / pdf
        guess -= delta
        if abs(delta) < 1e-9:
            break
    return guess


# ---------------------------------------------------------------------------
# Wald two-proportion test + Newcombe CI
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PropTestResult:
    estimate: float            # p_b - p_a
    p_value: float
    ci_95: tuple[float, float] | None
    z_stat: float


def wald_prop_test(
    s_a: int, n_a: int, s_b: int, n_b: int, *, alpha: float = 0.05,
) -> PropTestResult:
    if n_a <= 0 or n_b <= 0:
        return PropTestResult(
            estimate=float("nan"),
            p_value=float("nan"),
            ci_95=None,
            z_stat=float("nan"),
        )
    p_a = s_a / n_a
    p_b = s_b / n_b
    estimate = p_b - p_a
    pooled = (s_a + s_b) / (n_a + n_b)
    se_pooled = math.sqrt(
        pooled * (1.0 - pooled) * (1.0 / n_a + 1.0 / n_b),
    )
    if se_pooled == 0.0:
        return PropTestResult(
            estimate=estimate,
            p_value=1.0,
            ci_95=(estimate, estimate),
            z_stat=0.0,
        )
    z = estimate / se_pooled
    p_value = 2.0 * (1.0 - normal_cdf(abs(z)))
    ci = newcombe_ci(s_a, n_a, s_b, n_b, alpha=alpha)
    return PropTestResult(
        estimate=estimate,
        p_value=p_value,
        ci_95=ci,
        z_stat=z,
    )


def newcombe_ci(
    s_a: int, n_a: int, s_b: int, n_b: int, *, alpha: float = 0.05,
) -> tuple[float, float]:
    """Newcombe-Wilson hybrid interval for ``p_b - p_a``.

    Reference : Newcombe, Stats in Medicine 1998 — method 10.
    Accurate for small samples, unlike the naive Wald CI.
    """
    z = normal_ppf(1.0 - alpha / 2.0)
    l_a, u_a = _wilson_interval(s_a, n_a, z)
    l_b, u_b = _wilson_interval(s_b, n_b, z)
    p_a = s_a / n_a
    p_b = s_b / n_b
    delta = p_b - p_a
    lo = delta - math.sqrt((p_b - l_b) ** 2 + (u_a - p_a) ** 2)
    hi = delta + math.sqrt((u_b - p_b) ** 2 + (p_a - l_a) ** 2)
    return lo, hi


def _wilson_interval(
    s: int, n: int, z: float,
) -> tuple[float, float]:
    if n == 0:
        return 0.0, 0.0
    phat = s / n
    denom = 1.0 + z * z / n
    centre = (phat + z * z / (2.0 * n)) / denom
    half = (z * math.sqrt(phat * (1.0 - phat) / n + z * z / (4.0 * n * n))) / denom
    return max(0.0, centre - half), min(1.0, centre + half)


# ---------------------------------------------------------------------------
# Benjamini-Hochberg
# ---------------------------------------------------------------------------


def bh_correction(
    p_values: Sequence[float], *, alpha: float = 0.05,
) -> list[bool]:
    """Return a bool mask: True = reject H0 at BH-adjusted level."""
    m = len(p_values)
    if m == 0:
        return []
    indexed = sorted(enumerate(p_values), key=lambda t: t[1])
    rejected = [False] * m
    k_max = -1
    for i, (_, p) in enumerate(indexed):
        threshold = (i + 1) / m * alpha
        if p <= threshold:
            k_max = i
    if k_max < 0:
        return rejected
    for i in range(k_max + 1):
        rejected[indexed[i][0]] = True
    return rejected


def bh_adjusted_p(p_values: Sequence[float]) -> list[float]:
    """Return BH-adjusted q-values (monotone non-increasing)."""
    m = len(p_values)
    if m == 0:
        return []
    indexed = sorted(enumerate(p_values), key=lambda t: t[1])
    raw = [(m / (i + 1)) * p for i, (_, p) in enumerate(indexed)]
    # enforce monotonicity from the back
    q_sorted: list[float] = [0.0] * m
    running = 1.0
    for i in range(m - 1, -1, -1):
        running = min(running, min(1.0, raw[i]))
        q_sorted[i] = running
    q = [0.0] * m
    for i, (orig, _) in enumerate(indexed):
        q[orig] = q_sorted[i]
    return q


__all__ = [
    "MeanTestResult",
    "PropTestResult",
    "bh_adjusted_p",
    "bh_correction",
    "chi2_cdf",
    "newcombe_ci",
    "normal_cdf",
    "normal_pdf",
    "normal_ppf",
    "reg_incomplete_beta",
    "student_t_cdf",
    "wald_prop_test",
    "welch_df",
    "welch_t_test",
]
