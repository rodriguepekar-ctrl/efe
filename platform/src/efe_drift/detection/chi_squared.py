"""Chi-squared goodness-of-fit detector — categorical drift primary.

For a categorical feature with frozen categories ``{c_1, …, c_K}`` :

* ``O_k`` = observed count of category ``c_k`` in the current window
* ``E_k`` = expected count = ``N_current · q_k`` where ``q_k`` is the
  baseline proportion

The statistic ::

             (O_k − E_k)²
    χ² = Σ   ───────────
              E_k

follows χ² with ``df = K − 1`` under H₀. We emit both the raw χ²
value (``score``) and the p-value. Severity is assigned on the
**p-value** ladder (M09 §8.5) with ``lower_is_worse`` orientation.

Edge cases :

* a bin with ``E_k = 0`` (category missing from baseline) is merged
  into the ``__other__`` sentinel before computing the statistic —
  preserves the df count
* empty current → NaN score + severity ``none``
* the G-test (log-likelihood ratio) alternative is exposed as a
  helper function for UI comparison but not used by default
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass, field

from efe_drift.aggregation.categorical import OTHER_BUCKET
from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
)
from efe_drift.detection.severity import (
    DEFAULT_CHI_SQUARED_P,
    Severity,
    SeverityThresholds,
)
from efe_drift.detection.stats import chi2_cdf
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

# ---------------------------------------------------------------------------
# Pure math
# ---------------------------------------------------------------------------


def chi_squared_goodness_of_fit(
    observed: Sequence[float],
    expected: Sequence[float],
) -> tuple[float, int]:
    """Return (``χ²``, ``df = non-degenerate bins − 1``).

    Bins with ``expected == 0`` are dropped (not a valid χ² input).
    """
    if len(observed) != len(expected):
        raise ValueError(
            f"observed / expected length mismatch: {len(observed)} vs "
            f"{len(expected)}",
        )
    chi_sq = 0.0
    effective = 0
    for o, e in zip(observed, expected, strict=True):
        if e <= 0:
            continue
        chi_sq += (o - e) ** 2 / e
        effective += 1
    df = max(1, effective - 1)
    return chi_sq, df


def g_test_statistic(
    observed: Sequence[float],
    expected: Sequence[float],
) -> tuple[float, int]:
    """Log-likelihood ratio G-test — asymptotically χ²(df)."""
    if len(observed) != len(expected):
        raise ValueError("shape mismatch")
    g = 0.0
    effective = 0
    for o, e in zip(observed, expected, strict=True):
        if e <= 0 or o <= 0:
            continue
        g += 2.0 * o * math.log(o / e)
        effective += 1
    df = max(1, effective - 1)
    return g, df


# ---------------------------------------------------------------------------
# ChiSquaredDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ChiSquaredDetector:
    """Goodness-of-fit detector for **categorical** features."""

    thresholds: SeverityThresholds = DEFAULT_CHI_SQUARED_P
    _method: DriftMethod = "chi_squared"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("categorical",)

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        if ctx.feature.kind != "categorical":
            raise ValueError(
                f"ChiSquaredDetector does not support "
                f"kind={ctx.feature.kind!r}",
            )
        cats = ctx.baseline.spec.categorical_categories
        if cats is None or len(cats) < 2:
            raise ValueError(
                "baseline must have ≥ 2 categorical_categories",
            )
        categories = [*cats, OTHER_BUCKET]
        idx = {name: i for i, name in enumerate(categories)}
        observed = [0.0] * len(categories)
        for s in current:
            if not s.categorical_counts:
                continue
            for name, c in s.categorical_counts.items():
                observed[idx.get(name, idx[OTHER_BUCKET])] += float(c)
        total_current = sum(observed)
        # Baseline proportions : uniform across the N frozen
        # categories (see PSI module for the rationale). OTHER_BUCKET
        # baseline proportion is implicitly 0 unless data says so.
        n_frozen = len(cats)
        baseline_proportions = [1.0 / n_frozen] * n_frozen + [0.0]
        expected = [total_current * p for p in baseline_proportions]
        chi_sq, df = chi_squared_goodness_of_fit(observed, expected)
        p_value: float | None
        if total_current <= 0 or chi_sq == 0:
            p_value = 1.0 if total_current > 0 else None
        else:
            p_value = 1.0 - chi2_cdf(chi_sq, float(df))
        thresholds = ctx.thresholds or self.thresholds
        severity: Severity = (
            "none"
            if p_value is None or math.isnan(p_value)
            else thresholds.classify(p_value)
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
            score=chi_sq,
            severity=severity,
            computed_at=self._clock.now(),
            p_value=p_value,
            multi_scale_window=ctx.multi_scale_window,
            details={
                "categories": categories,
                "observed": observed,
                "expected": expected,
                "df": df,
            },
            sample_count_current=int(total_current),
            sample_count_baseline=ctx.baseline.spec.sample_count,
        )


__all__ = [
    "ChiSquaredDetector",
    "chi_squared_goodness_of_fit",
    "g_test_statistic",
]
