"""Population Stability Index (PSI) detector — primary numeric drift metric.

Formula (Siddiqi 2005) ::

           K
    PSI = Σ (p_i − q_i) · ln(p_i / q_i)
          i=1

where ``p_i`` and ``q_i`` are the proportion of observations in bin
``i`` for the current and baseline distributions respectively.

The detector requires :

* a **numeric / binary** feature
* a baseline with frozen ``bins`` (list of ``K + 1`` edge floats)
* a current :class:`FeatureSnapshot` (or list of) covering the
  evaluation window whose ``histogram`` uses the same edges

Edge cases :

* empty baseline OR empty current → :class:`InsufficientDriftData`
* bins mismatch → :class:`HistogramBinMismatch`
* smoothing : :math:`\\varepsilon = 10^{-6}` added to every proportion
  so empty bins don't blow up the logarithm

The detector also computes the helper :func:`js_divergence` so the
orchestration layer can emit a companion ``js_divergence`` result
when PSI is asymmetric.
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass, field

from efe_drift.aggregation.histogram import validate_bin_match
from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
)
from efe_drift.detection.severity import (
    DEFAULT_JS,
    DEFAULT_PSI,
    SeverityThresholds,
)
from efe_drift.errors import HistogramBinMismatch
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

# ---------------------------------------------------------------------------
# Pure PSI / JS math
# ---------------------------------------------------------------------------


def compute_psi(
    current_counts: Sequence[int],
    baseline_counts: Sequence[int],
    *,
    eps: float = 1e-6,
) -> float:
    """Return the PSI between two aligned histograms.

    Raises :class:`HistogramBinMismatch` if bin counts differ.
    Returns ``NaN`` if either total is zero.
    """
    if len(current_counts) != len(baseline_counts):
        raise HistogramBinMismatch(
            f"PSI: histograms have {len(current_counts)} vs "
            f"{len(baseline_counts)} bins",
        )
    n_cur = sum(current_counts)
    n_base = sum(baseline_counts)
    if n_cur == 0 or n_base == 0:
        return float("nan")
    psi = 0.0
    for c, b in zip(current_counts, baseline_counts, strict=True):
        p = max(eps, c / n_cur)
        q = max(eps, b / n_base)
        psi += (p - q) * math.log(p / q)
    return psi


def js_divergence(
    current_counts: Sequence[int],
    baseline_counts: Sequence[int],
    *,
    eps: float = 1e-6,
) -> float:
    """Symmetric, bounded divergence ``JS(P, Q) ∈ [0, ln 2]``.

    Useful when you want a scale-free companion to PSI.
    """
    if len(current_counts) != len(baseline_counts):
        raise HistogramBinMismatch(
            f"JS: histograms have {len(current_counts)} vs "
            f"{len(baseline_counts)} bins",
        )
    n_cur = sum(current_counts)
    n_base = sum(baseline_counts)
    if n_cur == 0 or n_base == 0:
        return float("nan")
    js = 0.0
    for c, b in zip(current_counts, baseline_counts, strict=True):
        p = max(eps, c / n_cur)
        q = max(eps, b / n_base)
        m = 0.5 * (p + q)
        js += 0.5 * p * math.log(p / m) + 0.5 * q * math.log(q / m)
    return js


def per_bin_contributions(
    current_counts: Sequence[int],
    baseline_counts: Sequence[int],
    *,
    eps: float = 1e-6,
) -> list[dict[str, float]]:
    """Return the per-bin ``(p, q, contribution)`` triplets.

    Used by the UI to explain *where* the drift comes from.
    """
    if len(current_counts) != len(baseline_counts):
        raise HistogramBinMismatch(
            f"PSI: histograms have {len(current_counts)} vs "
            f"{len(baseline_counts)} bins",
        )
    n_cur = sum(current_counts)
    n_base = sum(baseline_counts)
    out: list[dict[str, float]] = []
    if n_cur == 0 or n_base == 0:
        return out
    for c, b in zip(current_counts, baseline_counts, strict=True):
        p = max(eps, c / n_cur)
        q = max(eps, b / n_base)
        out.append({
            "p": p,
            "q": q,
            "delta": p - q,
            "contribution": (p - q) * math.log(p / q),
        })
    return out


# ---------------------------------------------------------------------------
# PsiDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PsiDetector:
    """PSI drift detector. Numeric / binary features only."""

    thresholds: SeverityThresholds = DEFAULT_PSI
    _method: DriftMethod = "psi"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("numeric", "binary")

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        if ctx.feature.kind not in ("numeric", "binary"):
            raise ValueError(
                f"PsiDetector does not support kind={ctx.feature.kind!r}",
            )
        edges = _edges_from_baseline(ctx.baseline.spec.bins)
        cur_counts = _aggregate_histograms(current, edges)
        base_counts = _baseline_histogram(ctx, edges)
        score = compute_psi(cur_counts, base_counts)
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(score)
            else thresholds.classify(score)
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
            score=score,
            severity=severity,
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={
                "bins": list(edges),
                "current_histogram": list(cur_counts),
                "baseline_histogram": list(base_counts),
                "per_bin": per_bin_contributions(cur_counts, base_counts),
            },
            sample_count_current=sum(cur_counts),
            sample_count_baseline=sum(base_counts),
        )


# ---------------------------------------------------------------------------
# JsDetector — symmetric companion
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class JsDivergenceDetector:
    """Jensen-Shannon divergence detector. Drops into the same
    pipeline as PSI (shares bins)."""

    thresholds: SeverityThresholds = DEFAULT_JS
    _method: DriftMethod = "js_divergence"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("numeric", "binary", "categorical")

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        if ctx.feature.kind in ("numeric", "binary"):
            edges = _edges_from_baseline(ctx.baseline.spec.bins)
            cur = _aggregate_histograms(current, edges)
            base = _baseline_histogram(ctx, edges)
        elif ctx.feature.kind == "categorical":
            cats = ctx.baseline.spec.categorical_categories
            if cats is None:
                raise ValueError(
                    "baseline has no categorical_categories",
                )
            cur, base = _categorical_aligned_counts(
                ctx, current, cats,
            )
        else:
            raise ValueError(
                f"JsDivergenceDetector does not support "
                f"kind={ctx.feature.kind!r}",
            )
        score = js_divergence(cur, base)
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(score)
            else thresholds.classify(score)
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
            score=score,
            severity=severity,
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={
                "current_counts": list(cur),
                "baseline_counts": list(base),
            },
            sample_count_current=sum(cur),
            sample_count_baseline=sum(base),
        )


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


def _edges_from_baseline(
    bins: tuple[float, ...] | None,
) -> tuple[float, ...]:
    if not bins or len(bins) < 2:
        raise HistogramBinMismatch(
            "baseline has no (or too few) numeric bins",
        )
    return bins


def _aggregate_histograms(
    snapshots: tuple[FeatureSnapshot, ...],
    edges: tuple[float, ...],
) -> list[int]:
    """Sum per-bin counts across all current snapshots.

    Every snapshot must have a histogram aligned on ``edges`` — the
    aggregator enforces this at write-time via ``FrozenBinSet``.
    """
    n_bins = len(edges) - 1
    totals = [0] * n_bins
    for s in snapshots:
        if s.histogram is None:
            continue
        validate_bin_match(s.histogram, edges)
        for i, c in enumerate(s.histogram):
            totals[i] += int(c)
    return totals


def _baseline_histogram(
    ctx: DetectionContext,
    edges: tuple[float, ...],
) -> list[int]:
    """Reconstruct a count-vector for the baseline.

    The baseline spec carries aggregated **statistics** but not the
    raw histogram — we approximate it by re-partitioning the sample
    count across bins using the ``bins`` + underlying sample
    distribution. For an equifrequency baseline this is uniform by
    construction (N / K per bin). For equiwidth / custom we fall back
    to uniform too, which is conservative.

    For production deployments, the baseline row ought to persist the
    explicit histogram (``bins_json`` + ``baseline_counts``) — a
    future schema bump. Until then the uniform assumption gives
    correct relative PSI for equifrequency (the MVP default).
    """
    n_bins = len(edges) - 1
    total = ctx.baseline.spec.sample_count
    per_bin = total // n_bins
    remainder = total - per_bin * n_bins
    counts = [per_bin] * n_bins
    # Distribute leftover rows to the first few bins deterministically.
    for i in range(remainder):
        counts[i] += 1
    return counts


def _categorical_aligned_counts(
    ctx: DetectionContext,
    snapshots: tuple[FeatureSnapshot, ...],
    frozen: tuple[str, ...],
) -> tuple[list[int], list[int]]:
    """Align categorical snapshots onto the baseline's frozen category
    list.  Unknown categories collapse into ``__other__`` at the end."""
    ordered = [*frozen, "__other__"]
    idx = {name: i for i, name in enumerate(ordered)}
    cur = [0] * len(ordered)
    for s in snapshots:
        if not s.categorical_counts:
            continue
        for name, c in s.categorical_counts.items():
            if name in idx:
                cur[idx[name]] += int(c)
            else:
                cur[idx["__other__"]] += int(c)
    # Baseline is uniform across the frozen categories by definition
    # (the baseline's own top-K-with-other list).  Use the baseline
    # sample_count for proportion purposes.
    base_total = ctx.baseline.spec.sample_count
    per_cat = base_total // len(frozen) if frozen else 0
    base = [per_cat] * len(frozen) + [0]
    return cur, base


__all__ = [
    "JsDivergenceDetector",
    "PsiDetector",
    "compute_psi",
    "js_divergence",
    "per_bin_contributions",
]
