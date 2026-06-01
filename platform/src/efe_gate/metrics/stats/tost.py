"""TOST — Two One-Sided Tests for equivalence.

The question TOST answers is:

    "Can we assert that the candidate is *at most* δ worse than the
    champion, with statistical confidence?"

The flipped null hypothesis is what makes TOST right for a
non-regression gate: a classical two-sided t-test's null "they are
equal" is easy to retain with a small sample — failing to reject is
not evidence of equivalence. TOST demands a positive assertion.

We implement the CI-based form (equivalent to the dual-one-sided form
for symmetric tests): if the ``conf``-level CI of ``candidate −
champion`` lies entirely inside ``[-δ, +δ]``, the candidate is
statistically equivalent. We extend with a "candidate strictly
better" branch so a metric where the candidate has a large positive
Δ is not flagged as a regression.

Outcomes:

    PASS      — no regression; promote-ok for this metric
    FAIL      — regression proven (CI upper bound < -δ)
    AMBIGUOUS — CI too wide to decide (spans the δ boundary)
    BETTER    — CI lower bound > +δ; candidate is strictly better
                (treated as PASS by the decision engine, but surfaced
                separately for reporting)
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

import numpy as np

from efe_gate.errors import InsufficientSample
from efe_gate.metrics.stats.bootstrap import (
    BootstrapResult,
    bootstrap_delta_ci,
)


class TostVerdict(str, Enum):
    PASS = "pass"        # equivalence band
    FAIL = "fail"        # regression proven
    AMBIGUOUS = "ambiguous"
    BETTER = "better"    # strictly better


@dataclass(frozen=True, slots=True)
class TostResult:
    verdict: TostVerdict
    observed_delta: float
    ci_lower: float
    ci_upper: float
    delta_margin: float
    conf: float
    n_samples: int
    # Whether higher is better for this metric (direction of FAIL).
    higher_is_better: bool


def tost_equivalence(
    champion: np.ndarray,
    candidate: np.ndarray,
    *,
    delta_margin: float,
    conf: float = 0.95,
    n_resamples: int = 10_000,
    seed: int = 1337,
    higher_is_better: bool = True,
) -> TostResult:
    """Run TOST equivalence via a paired bootstrap.

    Parameters
    ----------
    champion, candidate
        Per-sample aligned score arrays.
    delta_margin
        Equivalence half-width δ in *absolute* metric units.
    conf
        Confidence level (0.95 ≡ α = 5% per one-sided test).
    higher_is_better
        When False, the sign of δ flips for the FAIL branch —
        a candidate with a *larger* latency is a regression.

    Returns :class:`TostResult`.
    """
    a = np.asarray(champion, dtype=np.float64).ravel()
    b = np.asarray(candidate, dtype=np.float64).ravel()
    if a.shape != b.shape:
        raise ValueError(
            f"TOST shape mismatch {a.shape} vs {b.shape}",
        )
    if a.size < 1:
        raise InsufficientSample("TOST needs ≥ 1 paired sample")
    if delta_margin < 0.0:
        raise ValueError("delta_margin must be >= 0")

    ci = bootstrap_delta_ci(
        champion=a, candidate=b,
        n_resamples=n_resamples, conf=conf, seed=seed,
    )

    # For "higher is better": positive Δ is good, negative is bad.
    # For "lower is better" (latency, size): invert the sign of Δ
    # when interpreting FAIL / BETTER. We do this by flipping.
    d = ci.mean
    lo = ci.ci_lower
    hi = ci.ci_upper
    if not higher_is_better:
        d = -d
        lo, hi = -hi, -lo

    dm = float(delta_margin)
    if hi < -dm:
        v = TostVerdict.FAIL
    elif lo > +dm:
        v = TostVerdict.BETTER
    elif lo >= -dm and hi <= +dm:
        v = TostVerdict.PASS
    else:
        v = TostVerdict.AMBIGUOUS

    return TostResult(
        verdict=v,
        observed_delta=float(d),
        ci_lower=float(lo),
        ci_upper=float(hi),
        delta_margin=dm,
        conf=float(conf),
        n_samples=int(ci.n_samples),
        higher_is_better=bool(higher_is_better),
    )


__all__ = ["TostResult", "TostVerdict", "tost_equivalence"]


_ = BootstrapResult  # keep the import visible for type-checkers
