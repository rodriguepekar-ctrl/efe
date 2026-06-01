"""Maximum Mean Discrepancy (MMD²) detector — embedding drift primary.

For two sets of vectors ``X = {x_1,…,x_n}`` and ``Y = {y_1,…,y_m}``
with kernel ``k`` ::

                          1                1
    MMD²_u(X,Y)  =   ─────────── Σ_{i≠j} k(x_i,x_j)  +  ─────────── Σ_{i≠j} k(y_i,y_j)
                     n(n-1)                              m(m-1)
                               2
                         − ─────── Σ_{i,j} k(x_i,y_j)
                           n·m

RBF kernel : ``k(x, y) = exp(−‖x-y‖² / (2σ²))`` with σ selected via
the **median heuristic** : ``σ² = median{‖z_i-z_j‖² : i<j}`` on the
concatenated sample ``Z = X ∪ Y``.

Permutation-based p-value : shuffle ``X ∪ Y``, split back into two
halves, compute MMD²; p-value = fraction of permutations whose MMD²
exceeds the observed value. We default to ``n_permutations = 200``
(M09 §9.4).

Memory optimisation : for large n, callers should down-sample each
side via the reservoir (:class:`EmbeddingReservoir`) before calling
:meth:`detect_from_snapshots`. In this module, if the **centroid +
std** is available on both the baseline and the aggregated snapshot,
we fall back to a centroid-distance shortcut that is cheap but
unsigned by the permutation test.
"""

from __future__ import annotations

import math
import random
from collections.abc import Sequence
from dataclasses import dataclass, field

from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
)
from efe_drift.detection.severity import (
    DEFAULT_MMD,
    Severity,
    SeverityThresholds,
)
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

# ---------------------------------------------------------------------------
# Pure math
# ---------------------------------------------------------------------------


Vector = Sequence[float]


def _squared_euclidean(a: Vector, b: Vector) -> float:
    total = 0.0
    for x, y in zip(a, b, strict=True):
        diff = x - y
        total += diff * diff
    return total


def _median_heuristic(pool: Sequence[Vector]) -> float:
    """Return ``σ²`` for the RBF kernel using the median-of-pairwise-
    squared-distances heuristic."""
    pairs: list[float] = []
    n = len(pool)
    for i in range(n):
        for j in range(i + 1, n):
            pairs.append(_squared_euclidean(pool[i], pool[j]))
    if not pairs:
        return 1.0
    pairs.sort()
    mid = len(pairs) // 2
    if len(pairs) % 2 == 0:
        return 0.5 * (pairs[mid - 1] + pairs[mid]) / 2.0 + 1e-12
    return pairs[mid] / 2.0 + 1e-12


def _rbf(d_sq: float, sigma_sq: float) -> float:
    return math.exp(-d_sq / (2.0 * sigma_sq))


def mmd_squared_rbf(
    x: Sequence[Vector],
    y: Sequence[Vector],
    *,
    sigma_sq: float | None = None,
) -> tuple[float, float]:
    """Return ``(MMD²_unbiased, σ²)``.

    Memory : O(n² + m²) pairwise evaluations. For n = 1 000 this is
    the reasonable upper bound — down-sample via reservoir for larger
    inputs.
    """
    n, m = len(x), len(y)
    if n < 2 or m < 2:
        return float("nan"), sigma_sq or float("nan")
    if sigma_sq is None:
        sigma_sq = _median_heuristic(list(x) + list(y))
    kxx = 0.0
    for i in range(n):
        for j in range(n):
            if i == j:
                continue
            kxx += _rbf(_squared_euclidean(x[i], x[j]), sigma_sq)
    kyy = 0.0
    for i in range(m):
        for j in range(m):
            if i == j:
                continue
            kyy += _rbf(_squared_euclidean(y[i], y[j]), sigma_sq)
    kxy = 0.0
    for i in range(n):
        for j in range(m):
            kxy += _rbf(_squared_euclidean(x[i], y[j]), sigma_sq)
    mmd_sq = (
        kxx / (n * (n - 1))
        + kyy / (m * (m - 1))
        - 2.0 * kxy / (n * m)
    )
    return mmd_sq, sigma_sq


def mmd_permutation_p_value(
    x: Sequence[Vector],
    y: Sequence[Vector],
    *,
    n_permutations: int = 200,
    seed: int = 20260420,
) -> tuple[float, float]:
    """Permutation p-value for MMD² under H0.

    Returns ``(observed_mmd², p_value)``.
    """
    n = len(x)
    m = len(y)
    if n < 2 or m < 2:
        return float("nan"), float("nan")
    observed, sigma_sq = mmd_squared_rbf(x, y)
    rng = random.Random(seed)
    pool: list[Vector] = [*x, *y]
    count_exceed = 0
    for _ in range(n_permutations):
        shuffled = list(pool)
        rng.shuffle(shuffled)
        xp = shuffled[:n]
        yp = shuffled[n:]
        stat, _ = mmd_squared_rbf(xp, yp, sigma_sq=sigma_sq)
        if stat >= observed:
            count_exceed += 1
    p_value = (count_exceed + 1) / (n_permutations + 1)
    return observed, p_value


# ---------------------------------------------------------------------------
# Centroid shortcut
# ---------------------------------------------------------------------------


def centroid_distance(
    current_centroid: Vector,
    baseline_centroid: Vector,
    *,
    baseline_std: Vector | None = None,
    eps: float = 1e-6,
) -> float:
    """Normalised L2 distance between two centroids.

    Per-dimension divisor is ``std_i + eps`` for numerical stability.
    Used as a fast drift proxy when full MMD² is cost-prohibitive.
    """
    if len(current_centroid) != len(baseline_centroid):
        raise ValueError("centroid dim mismatch")
    std = baseline_std if baseline_std is not None else [1.0] * len(current_centroid)
    if len(std) != len(current_centroid):
        raise ValueError("std dim mismatch")
    total = 0.0
    for c, b, s in zip(current_centroid, baseline_centroid, std, strict=True):
        diff = (c - b) / (s + eps)
        total += diff * diff
    return math.sqrt(total)


# ---------------------------------------------------------------------------
# MmdDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class MmdDetector:
    """MMD² drift detector for embedding features."""

    thresholds: SeverityThresholds = DEFAULT_MMD
    n_permutations: int = 200
    seed: int = 20260420
    max_samples_per_side: int = 500
    _method: DriftMethod = "mmd"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("embedding",)

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        if ctx.feature.kind != "embedding":
            raise ValueError(
                f"MmdDetector does not support kind={ctx.feature.kind!r}",
            )
        # This MVP path uses the centroid shortcut : real production
        # deployments should pass raw embeddings via detect_from_raw.
        cur_centroid = _aggregate_embedding_centroid(current)
        base_centroid = ctx.baseline.spec.centroid
        base_std = ctx.baseline.spec.std
        severity: Severity
        if cur_centroid is None or base_centroid is None:
            score = float("nan")
            details: dict[str, object] = {"mode": "centroid", "note": "missing data"}
            severity = "none"
            n_current = sum(s.sample_count for s in current)
        else:
            score = centroid_distance(
                cur_centroid, base_centroid, baseline_std=base_std,
            )
            thresholds = ctx.thresholds or self.thresholds
            severity = (
                "none" if math.isnan(score)
                else thresholds.classify(score)
            )
            details = {
                "mode": "centroid",
                "current_centroid": list(cur_centroid),
                "baseline_centroid": list(base_centroid),
                "baseline_std": list(base_std) if base_std else None,
            }
            n_current = sum(s.sample_count for s in current)
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
            details=details,
            sample_count_current=n_current,
            sample_count_baseline=ctx.baseline.spec.sample_count,
        )

    def detect_from_raw(
        self,
        ctx: DetectionContext,
        current_vectors: Sequence[Vector],
        baseline_vectors: Sequence[Vector],
    ) -> DriftResult:
        """Exact MMD²-RBF path with permutation p-value.

        Down-samples both sides to ``max_samples_per_side`` before
        computing — O(n²) kernel cost would explode otherwise.
        """
        rng = random.Random(self.seed)
        cur = _downsample(current_vectors, self.max_samples_per_side, rng)
        base = _downsample(baseline_vectors, self.max_samples_per_side, rng)
        if len(cur) < 2 or len(base) < 2:
            return self._nan_result(ctx, cur, base)
        observed, p_value = mmd_permutation_p_value(
            cur, base,
            n_permutations=self.n_permutations, seed=self.seed,
        )
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(observed)
            else thresholds.classify(observed)
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
            score=observed,
            severity=severity,
            computed_at=self._clock.now(),
            p_value=p_value if not math.isnan(p_value) else None,
            multi_scale_window=ctx.multi_scale_window,
            details={
                "mode": "mmd_rbf_permutation",
                "n_permutations": self.n_permutations,
                "n_current": len(cur),
                "n_baseline": len(base),
            },
            sample_count_current=len(cur),
            sample_count_baseline=len(base),
        )

    def _nan_result(
        self,
        ctx: DetectionContext,
        cur: Sequence[Vector],
        base: Sequence[Vector],
    ) -> DriftResult:
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
            score=float("nan"),
            severity="none",
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={"mode": "insufficient_samples"},
            sample_count_current=len(cur),
            sample_count_baseline=len(base),
        )


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _aggregate_embedding_centroid(
    snapshots: Sequence[FeatureSnapshot],
) -> tuple[float, ...] | None:
    """Weighted average of per-snapshot centroids."""
    totals: list[float] | None = None
    total_weight = 0
    for s in snapshots:
        if s.embedding_centroid is None or s.sample_count <= 0:
            continue
        if totals is None:
            totals = [0.0] * len(s.embedding_centroid)
        if len(totals) != len(s.embedding_centroid):
            continue
        for i, c in enumerate(s.embedding_centroid):
            totals[i] += c * s.sample_count
        total_weight += s.sample_count
    if totals is None or total_weight == 0:
        return None
    return tuple(v / total_weight for v in totals)


def _downsample(
    xs: Sequence[Vector],
    cap: int,
    rng: random.Random,
) -> list[Vector]:
    if len(xs) <= cap:
        return list(xs)
    return rng.sample(list(xs), cap)


__all__ = [
    "MmdDetector",
    "centroid_distance",
    "mmd_permutation_p_value",
    "mmd_squared_rbf",
]
