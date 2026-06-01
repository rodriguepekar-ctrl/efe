"""Bootstrap confidence intervals.

Two flavours:

* :func:`bootstrap_ci` — CI for a single sample's mean. Used when we
  only have the candidate's scores (no paired champion).
* :func:`bootstrap_delta_ci` — CI for the *paired* difference
  ``candidate - champion``. Preserves pairing (resample indices, not
  independently). Essential for TOST.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from efe_gate.errors import InsufficientSample


@dataclass(frozen=True, slots=True)
class BootstrapResult:
    mean: float
    ci_lower: float
    ci_upper: float
    n_resamples: int
    n_samples: int


def bootstrap_ci(
    scores: np.ndarray,
    *,
    n_resamples: int = 10_000,
    conf: float = 0.95,
    seed: int = 1337,
) -> BootstrapResult:
    """Mean + percentile CI for a 1-D sample."""
    x = np.asarray(scores, dtype=np.float64).ravel()
    n = int(x.size)
    if n < 1:
        raise InsufficientSample("bootstrap_ci needs at least 1 sample")
    if not (0.0 < conf < 1.0):
        raise ValueError("conf must be in (0, 1)")
    if n_resamples < 1:
        raise ValueError("n_resamples must be >= 1")
    rng = np.random.default_rng(seed)
    # Sample in one go for speed (memory O(B × n) is ok for n up to ~100k).
    # For safety switch to a loop when n_resamples * n > 10M.
    if n_resamples * n <= 10_000_000:
        idx = rng.integers(0, n, size=(n_resamples, n))
        resampled = x[idx].mean(axis=1)
    else:
        resampled = np.empty(n_resamples, dtype=np.float64)
        for i in range(n_resamples):
            resampled[i] = x[rng.integers(0, n, size=n)].mean()
    alpha = 1.0 - conf
    lo = float(np.percentile(resampled, 100.0 * alpha / 2.0))
    hi = float(np.percentile(resampled, 100.0 * (1.0 - alpha / 2.0)))
    return BootstrapResult(
        mean=float(x.mean()),
        ci_lower=lo,
        ci_upper=hi,
        n_resamples=int(n_resamples),
        n_samples=n,
    )


def bootstrap_delta_ci(
    champion: np.ndarray,
    candidate: np.ndarray,
    *,
    n_resamples: int = 10_000,
    conf: float = 0.95,
    seed: int = 1337,
) -> BootstrapResult:
    """Paired bootstrap CI for ``candidate.mean() - champion.mean()``.

    Both arrays must be the same length and aligned sample-wise. We
    resample *indices* and compute the mean of the paired diff at each
    iteration — this preserves the dependence structure between the
    two models evaluated on the same samples, which is usually the
    thing we actually care about.
    """
    a = np.asarray(champion, dtype=np.float64).ravel()
    b = np.asarray(candidate, dtype=np.float64).ravel()
    if a.shape != b.shape:
        raise ValueError(
            f"champion / candidate shape mismatch: {a.shape} vs {b.shape}",
        )
    n = int(a.size)
    if n < 1:
        raise InsufficientSample(
            "bootstrap_delta_ci needs at least 1 paired sample",
        )
    if not (0.0 < conf < 1.0):
        raise ValueError("conf must be in (0, 1)")
    rng = np.random.default_rng(seed)
    diff = b - a
    if n_resamples * n <= 10_000_000:
        idx = rng.integers(0, n, size=(n_resamples, n))
        resampled = diff[idx].mean(axis=1)
    else:
        resampled = np.empty(n_resamples, dtype=np.float64)
        for i in range(n_resamples):
            resampled[i] = diff[rng.integers(0, n, size=n)].mean()
    alpha = 1.0 - conf
    lo = float(np.percentile(resampled, 100.0 * alpha / 2.0))
    hi = float(np.percentile(resampled, 100.0 * (1.0 - alpha / 2.0)))
    return BootstrapResult(
        mean=float(diff.mean()),
        ci_lower=lo,
        ci_upper=hi,
        n_resamples=int(n_resamples),
        n_samples=n,
    )


__all__ = [
    "BootstrapResult",
    "bootstrap_ci",
    "bootstrap_delta_ci",
]
