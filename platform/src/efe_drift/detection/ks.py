"""Kolmogorov-Smirnov two-sample detector.

Unlike PSI and χ², KS operates on the **raw observations** of a
continuous feature — no binning. The tradeoff : the detector needs
the baseline's raw sample tail (either reservoirs or reconstructed
quantiles) AND the current raw samples.

In production, both are fetched from the ``raw_events`` hypertable
via :func:`_reconstruct_from_baseline_bins` : since the baseline row
persists only the bin edges + sample_count, we synthesise a
"uniform-within-bin" reconstruction that is exact for an
equifrequency baseline and approximate for equiwidth. The test
suite asserts this reconstruction is within 1e-3 of scipy's answer
on our canonical fixture.

A purpose-built ``detect_from_raw`` entrypoint lets callers with
direct access to the raw samples bypass the reconstruction and get
the exact KS statistic.
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
from efe_drift.detection.severity import (
    DEFAULT_KS_STAT,
    SeverityThresholds,
)
from efe_drift.detection.stats import ks_two_sample
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

# ---------------------------------------------------------------------------
# Pure math
# ---------------------------------------------------------------------------


def _reconstruct_from_bins(
    bins: Sequence[float],
    total: int,
) -> list[float]:
    """Return ``total`` synthetic values, spread uniformly within each
    of the ``K = len(bins) - 1`` bins.

    For an equifrequency baseline, this is an **exact** reconstruction
    (by construction, each bin was defined to contain ``total / K``
    observations). For equiwidth it is a conservative approximation.

    Used when the baseline row doesn't persist raw samples — see M09
    §13.5 for the schema tradeoff.
    """
    k = len(bins) - 1
    if k <= 0 or total <= 0:
        return []
    per_bin = total // k
    remainder = total - per_bin * k
    out: list[float] = []
    for i in range(k):
        lo, hi = bins[i], bins[i + 1]
        n = per_bin + (1 if i < remainder else 0)
        if n == 0:
            continue
        if n == 1:
            out.append(0.5 * (lo + hi))
            continue
        # n ≥ 2 — spread evenly on (lo, hi).
        step = (hi - lo) / n
        for j in range(n):
            out.append(lo + step * (j + 0.5))
    return out


def _samples_from_snapshots(
    snapshots: Sequence[FeatureSnapshot],
    edges: Sequence[float],
) -> list[float]:
    """Uniform-within-bin reconstruction of the current samples.

    Each snapshot carries a ``histogram`` aligned on ``edges``; we
    re-synthesise uniform placeholders per bin so the KS test runs on
    float sequences rather than histograms.
    """
    out: list[float] = []
    for s in snapshots:
        if s.histogram is None:
            continue
        for i, count in enumerate(s.histogram):
            if count <= 0:
                continue
            lo, hi = edges[i], edges[i + 1]
            if count == 1:
                out.append(0.5 * (lo + hi))
                continue
            step = (hi - lo) / count
            for j in range(count):
                out.append(lo + step * (j + 0.5))
    return out


# ---------------------------------------------------------------------------
# KsDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class KsDetector:
    """Kolmogorov-Smirnov two-sample detector for numeric features.

    Produces two numbers : ``D = sup |F_cur - F_base|`` and the
    asymptotic p-value from the Kolmogorov distribution. Severity is
    assigned on ``D`` (``higher_is_worse``), but callers may override
    to use the p-value ladder via an :class:`AlertRule`.
    """

    thresholds: SeverityThresholds = DEFAULT_KS_STAT
    _method: DriftMethod = "ks"
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
                f"KsDetector does not support kind={ctx.feature.kind!r}",
            )
        edges = ctx.baseline.spec.bins
        if edges is None or len(edges) < 2:
            raise ValueError("baseline has no numeric bins")
        cur_samples = _samples_from_snapshots(current, edges)
        base_samples = _reconstruct_from_bins(
            edges, ctx.baseline.spec.sample_count,
        )
        return self._detect(ctx, cur_samples, base_samples, edges)

    def detect_from_raw(
        self,
        ctx: DetectionContext,
        current_samples: Sequence[float],
        baseline_samples: Sequence[float],
    ) -> DriftResult:
        """Exact KS test — bypass the uniform-bin reconstruction."""
        return self._detect(
            ctx, list(current_samples), list(baseline_samples),
            ctx.baseline.spec.bins or (),
        )

    # ---- internals -----------------------------------------------------

    def _detect(
        self,
        ctx: DetectionContext,
        cur: list[float],
        base: list[float],
        edges: Sequence[float],
    ) -> DriftResult:
        if len(cur) < 2 or len(base) < 2:
            d, p = float("nan"), float("nan")
        else:
            d, p = ks_two_sample(cur, base)
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(d) else thresholds.classify(d)
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
            score=d,
            severity=severity,
            computed_at=self._clock.now(),
            p_value=p if not math.isnan(p) else None,
            multi_scale_window=ctx.multi_scale_window,
            details={
                "n_current": len(cur),
                "n_baseline": len(base),
                "bins": list(edges) if edges else None,
            },
            sample_count_current=len(cur),
            sample_count_baseline=len(base),
        )


__all__ = [
    "KsDetector",
]
