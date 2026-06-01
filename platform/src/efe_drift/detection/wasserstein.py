"""Wasserstein-1 (Earth-Mover's) distance detector — 1D numeric.

For two 1D distributions :math:`P, Q` ::

    W_1(P, Q) = \\int_{-\\infty}^{\\infty} |F_P(x) - F_Q(x)| \\, dx
              = \\int_0^1 |F_P^{-1}(u) - F_Q^{-1}(u)| \\, du

We compute the left form directly from the sorted quantiles of two
samples — for equal-size samples the right form reduces to the
mean-of-sorted-pair-differences, which is the trick we use in
:func:`wasserstein_1d_sorted`.

Units : same as the input. For ``latency_ms`` the output is in
milliseconds — often more interpretable than PSI. The default
severity ladder is **relative to the baseline standard deviation**
so thresholds generalise across features (M09 §3.4).
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass, field

from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
)
from efe_drift.detection.ks import _reconstruct_from_bins, _samples_from_snapshots
from efe_drift.detection.severity import (
    DEFAULT_WASSERSTEIN,
    SeverityThresholds,
)
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

# ---------------------------------------------------------------------------
# Pure math
# ---------------------------------------------------------------------------


def wasserstein_1d_sorted(
    a_sorted: Sequence[float], b_sorted: Sequence[float],
) -> float:
    """W_1 between two **sorted** 1D samples.

    Implementation matches :func:`scipy.stats.wasserstein_distance` :

    1. Take all breakpoints ``u`` where the CDFs jump = sorted union.
    2. Compute CDF values at each breakpoint.
    3. Trapezoidal integration of :math:`|F_a - F_b|` along that axis.
    """
    n = len(a_sorted)
    m = len(b_sorted)
    if n == 0 or m == 0:
        return float("nan")
    # Merge sorted union of breakpoints.
    merged: list[float] = []
    i = j = 0
    while i < n and j < m:
        if a_sorted[i] < b_sorted[j]:
            merged.append(a_sorted[i])
            i += 1
        else:
            merged.append(b_sorted[j])
            j += 1
    merged.extend(a_sorted[i:])
    merged.extend(b_sorted[j:])
    # CDFs on each breakpoint (excluding the last — we need deltas).
    total = 0.0
    idx_a = idx_b = 0
    # Walk pairs (x_k, x_{k+1}).
    for k in range(len(merged) - 1):
        x_k = merged[k]
        x_next = merged[k + 1]
        while idx_a < n and a_sorted[idx_a] <= x_k:
            idx_a += 1
        while idx_b < m and b_sorted[idx_b] <= x_k:
            idx_b += 1
        f_a = idx_a / n
        f_b = idx_b / m
        total += abs(f_a - f_b) * (x_next - x_k)
    return total


def wasserstein_1d(
    a: Sequence[float], b: Sequence[float],
) -> float:
    return wasserstein_1d_sorted(sorted(a), sorted(b))


# ---------------------------------------------------------------------------
# WassersteinDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class WassersteinDetector:
    """Wasserstein-1 drift detector — 1D numeric features.

    The ``thresholds`` on the score are expressed **in multiples of the
    baseline std** by default : a shift of ``0.25 × σ_baseline``
    triggers ``major``. This makes the severity ladder portable across
    features of different scales (latency ms vs cost USD).
    """

    thresholds: SeverityThresholds = DEFAULT_WASSERSTEIN
    normalise_by_std: bool = True
    _method: DriftMethod = "wasserstein"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("numeric",)

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        if ctx.feature.kind != "numeric":
            raise ValueError(
                f"WassersteinDetector does not support "
                f"kind={ctx.feature.kind!r}",
            )
        edges = ctx.baseline.spec.bins
        if edges is None or len(edges) < 2:
            raise ValueError("baseline has no numeric bins")
        cur = _samples_from_snapshots(current, edges)
        base = _reconstruct_from_bins(
            edges, ctx.baseline.spec.sample_count,
        )
        return self._detect(ctx, cur, base)

    def detect_from_raw(
        self,
        ctx: DetectionContext,
        current_samples: Sequence[float],
        baseline_samples: Sequence[float],
    ) -> DriftResult:
        return self._detect(ctx, list(current_samples), list(baseline_samples))

    # ---- internals -----------------------------------------------------

    def _detect(
        self,
        ctx: DetectionContext,
        cur: list[float],
        base: list[float],
    ) -> DriftResult:
        if len(cur) < 2 or len(base) < 2:
            raw = float("nan")
            normalised = float("nan")
        else:
            raw = wasserstein_1d(cur, base)
            if self.normalise_by_std:
                std = float(ctx.baseline.spec.stats.get("std", 0.0))
                normalised = raw / std if std > 0 else raw
            else:
                normalised = raw
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(normalised)
            else thresholds.classify(normalised)
        )
        return DriftResult(
            evaluation_id=new_uuid(),
            tenant_id=ctx.tenant_id,
            task_id=ctx.task_id,
            feature_id=ctx.feature.feature_id,
            feature_name=ctx.feature.name,
            baseline_id=ctx.baseline.baseline_id,
            window_start=ctx.window_start,
            window_end=ctx.window_end,
            method=self._method,
            score=normalised,
            severity=severity,
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={
                "raw_distance": raw,
                "normalised_by_std": self.normalise_by_std,
                "baseline_std": ctx.baseline.spec.stats.get("std"),
                "n_current": len(cur),
                "n_baseline": len(base),
            },
            sample_count_current=len(cur),
            sample_count_baseline=len(base),
        )


__all__ = [
    "WassersteinDetector",
    "wasserstein_1d",
    "wasserstein_1d_sorted",
]
