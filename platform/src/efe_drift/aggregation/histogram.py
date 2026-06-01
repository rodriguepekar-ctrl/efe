"""Histogram builder + bin-edge computation.

Three strategies (see M09 §7.2) :

* :func:`equifrequency_bins` — quantile-based. Produces bins such
  that each contains roughly ``N / K`` observations of the baseline.
  Robust to outliers, recommended default for numeric features.

* :func:`equiwidth_bins` — linearly spaced between baseline min and
  max. Cheap to compute but sensitive to heavy tails.

* :func:`custom_bins` — caller supplies the edges. The helper just
  validates them (monotone, ≥ 2, finite).

Once bins are chosen they are **frozen** — observations collected
after promotion are classified against the same edges (invariant
I2). :class:`HistogramBuilder` encapsulates that contract : its
:meth:`count` method always uses its fixed ``edges`` attribute.

Edge semantics :

* Bins are ``[edge_i, edge_{i+1})`` for ``i < K-1`` (left-closed,
  right-open), and the last bin is ``[edge_{K-1}, edge_K]``
  (right-closed) so the max sample lands in-range.
* Values below ``edges[0]`` fall into bin 0 (saturated low).
* Values above ``edges[K]`` fall into bin ``K-1`` (saturated high).

This clamp-on-tails behaviour is what keeps PSI computation valid
across spec drifts on the tails of the distribution.
"""

from __future__ import annotations

import bisect
import math
from collections.abc import Sequence
from dataclasses import dataclass

from efe_drift.errors import (
    EmptySampleSet,
    HistogramBinMismatch,
    InvalidBinningStrategy,
)

# ---------------------------------------------------------------------------
# Bin-edge construction
# ---------------------------------------------------------------------------


def equifrequency_bins(
    samples: Sequence[float], *, n_bins: int = 10,
) -> tuple[float, ...]:
    """Return ``n_bins + 1`` edges placed at equal-frequency quantiles.

    Duplicate edges (constant region of the distribution) are
    deduplicated *monotonically* — ``jitter = 1e-12`` is added to
    subsequent equal values so the edges remain strictly increasing.
    """
    if n_bins < 2:
        raise InvalidBinningStrategy(
            f"n_bins must be ≥ 2, got {n_bins}",
        )
    if not samples:
        raise EmptySampleSet("cannot compute equifrequency bins from empty input")
    sorted_vals = sorted(float(v) for v in samples)
    n = len(sorted_vals)
    edges: list[float] = []
    for i in range(n_bins + 1):
        q = i / n_bins
        # Interpolated quantile (type 7 — numpy default).
        if i == 0:
            idx = 0
        elif i == n_bins:
            idx = n - 1
        else:
            idx_f = q * (n - 1)
            lo = math.floor(idx_f)
            frac = idx_f - lo
            if lo + 1 < n:
                edges.append(
                    sorted_vals[lo] * (1 - frac)
                    + sorted_vals[lo + 1] * frac,
                )
                continue
            idx = lo
        edges.append(sorted_vals[idx])
    # Strictly-increasing enforcement.
    monotonic: list[float] = []
    for e in edges:
        if monotonic and e <= monotonic[-1]:
            e = monotonic[-1] + 1e-12
        monotonic.append(e)
    return tuple(monotonic)


def equiwidth_bins(
    samples: Sequence[float], *, n_bins: int = 10,
) -> tuple[float, ...]:
    if n_bins < 2:
        raise InvalidBinningStrategy(
            f"n_bins must be ≥ 2, got {n_bins}",
        )
    if not samples:
        raise EmptySampleSet("cannot compute equiwidth bins from empty input")
    lo = min(float(v) for v in samples)
    hi = max(float(v) for v in samples)
    if lo == hi:
        # Degenerate — spread a tiny jitter so detection doesn't divide
        # by zero downstream.
        hi = lo + 1e-9
    step = (hi - lo) / n_bins
    return tuple(lo + i * step for i in range(n_bins + 1))


def custom_bins(edges: Sequence[float]) -> tuple[float, ...]:
    if len(edges) < 3:
        raise InvalidBinningStrategy(
            f"custom bins must have ≥ 3 edges (≥ 2 bins), got {len(edges)}",
        )
    floats = tuple(float(e) for e in edges)
    for i in range(len(floats) - 1):
        if floats[i + 1] <= floats[i]:
            raise InvalidBinningStrategy(
                f"custom bins must be strictly increasing at index {i}: "
                f"{floats[i]} -> {floats[i + 1]}",
            )
        if not math.isfinite(floats[i]) or not math.isfinite(floats[i + 1]):
            raise InvalidBinningStrategy(
                f"custom bins must be finite, got {floats}",
            )
    return floats


# ---------------------------------------------------------------------------
# Classification
# ---------------------------------------------------------------------------


def classify(value: float, edges: Sequence[float]) -> int:
    """Return the bin index for ``value`` against ``edges``.

    Edges are expected to be strictly increasing. Values below
    ``edges[0]`` saturate to 0; values above ``edges[-1]`` saturate
    to ``len(edges) - 2``.
    """
    if len(edges) < 2:
        raise HistogramBinMismatch(
            f"need ≥ 2 edges, got {len(edges)}",
        )
    if value <= edges[0]:
        return 0
    if value >= edges[-1]:
        return len(edges) - 2
    idx = bisect.bisect_right(edges, value) - 1
    # Guard against pathological inputs that slip past the bracket
    # checks (NaN etc.).
    if idx < 0:
        return 0
    if idx >= len(edges) - 1:
        return len(edges) - 2
    return idx


# ---------------------------------------------------------------------------
# HistogramBuilder — stateful accumulator
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class HistogramBuilder:
    """Accumulates integer counts into a histogram with fixed edges.

    The builder tracks both the per-bin counts *and* sufficient
    statistics (mean, m2, min, max) via Welford's online algorithm so
    baselines can reuse the same pass to populate ``stats_json``.
    """

    edges: tuple[float, ...]
    counts: list[int]
    _n: int = 0
    _mean: float = 0.0
    _m2: float = 0.0
    _min: float = float("inf")
    _max: float = float("-inf")

    @classmethod
    def from_edges(cls, edges: Sequence[float]) -> HistogramBuilder:
        if len(edges) < 2:
            raise HistogramBinMismatch(
                f"need ≥ 2 edges, got {len(edges)}",
            )
        return cls(
            edges=tuple(edges),
            counts=[0] * (len(edges) - 1),
        )

    # ---- mutation ------------------------------------------------------

    def add(self, value: float) -> None:
        if math.isnan(value):
            return
        idx = classify(value, self.edges)
        self.counts[idx] += 1
        self._n += 1
        # Welford
        delta = value - self._mean
        self._mean += delta / self._n
        delta2 = value - self._mean
        self._m2 += delta * delta2
        if value < self._min:
            self._min = value
        if value > self._max:
            self._max = value

    def add_many(self, values: Sequence[float]) -> None:
        for v in values:
            self.add(v)

    # ---- read ----------------------------------------------------------

    @property
    def n(self) -> int:
        return self._n

    @property
    def mean(self) -> float:
        return self._mean if self._n else float("nan")

    @property
    def variance(self) -> float:
        if self._n < 2:
            return 0.0
        return self._m2 / (self._n - 1)

    @property
    def std(self) -> float:
        return math.sqrt(self.variance)

    def stats_summary(self) -> dict[str, float]:
        """Materialise a dict safe to drop into a snapshot's
        ``stats_json``. Empty builders return NaN values so the
        downstream caller knows the bucket had no data."""
        return {
            "n": float(self._n),
            "mean": self.mean,
            "std": self.std,
            "min": self._min if self._n else float("nan"),
            "max": self._max if self._n else float("nan"),
        }

    def histogram(self) -> tuple[int, ...]:
        return tuple(self.counts)

    def reset(self) -> None:
        self.counts = [0] * (len(self.edges) - 1)
        self._n = 0
        self._mean = 0.0
        self._m2 = 0.0
        self._min = float("inf")
        self._max = float("-inf")


# ---------------------------------------------------------------------------
# Validation helper
# ---------------------------------------------------------------------------


def validate_bin_match(
    histogram: Sequence[int], edges: Sequence[float],
) -> None:
    """Raise :class:`HistogramBinMismatch` if lengths don't line up."""
    expected = len(edges) - 1
    if len(histogram) != expected:
        raise HistogramBinMismatch(
            f"histogram has {len(histogram)} bins but edges imply {expected}",
        )


__all__ = [
    "HistogramBuilder",
    "classify",
    "custom_bins",
    "equifrequency_bins",
    "equiwidth_bins",
    "validate_bin_match",
]
