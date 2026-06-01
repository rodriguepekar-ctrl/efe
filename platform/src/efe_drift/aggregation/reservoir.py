"""Reservoir sampler for embedding vectors.

Vitter's Algorithm R (1985) — maintains a uniform random sample of
``size`` items from a stream of unknown length. Memory-bounded so we
can run against 100 k req/s.

The sampler also maintains a **running centroid + running variance
per dimension** so callers that only need moments don't pay the full
reservoir cost. Welford updates keep numerical stability.
"""

from __future__ import annotations

import math
import random
from collections.abc import Sequence
from dataclasses import dataclass, field


@dataclass(slots=True)
class EmbeddingReservoir:
    """Uniform random sample of fixed ``size`` + running moments.

    * :meth:`add` accepts one vector (``Sequence[float]``). All added
      vectors must have the same ``dim`` (checked on first call and
      enforced afterwards).
    * :meth:`sample` returns the current reservoir as a tuple.
    * :meth:`centroid` / :meth:`std` return the per-dimension
      running mean / running stdev on **every** seen vector (not just
      the reservoir).
    """

    size: int = 10_000
    seed: int = 20260420
    _items: list[tuple[float, ...]] = field(default_factory=list)
    _seen: int = 0
    _dim: int | None = None
    _rng: random.Random | None = None
    _mean: list[float] = field(default_factory=list)
    _m2: list[float] = field(default_factory=list)

    def __post_init__(self) -> None:
        if self.size < 1:
            raise ValueError(f"size must be ≥ 1, got {self.size}")
        self._rng = random.Random(self.seed)

    # ---- mutation ------------------------------------------------------

    def add(self, vec: Sequence[float]) -> None:
        if self._dim is None:
            self._dim = len(vec)
            self._mean = [0.0] * self._dim
            self._m2 = [0.0] * self._dim
        elif len(vec) != self._dim:
            raise ValueError(
                f"dim mismatch : expected {self._dim}, got {len(vec)}",
            )
        if self._dim == 0:
            # Degenerate case — still count the seen.
            self._seen += 1
            return
        self._seen += 1
        # Welford per dimension.
        for i in range(self._dim):
            v = float(vec[i])
            delta = v - self._mean[i]
            self._mean[i] += delta / self._seen
            delta2 = v - self._mean[i]
            self._m2[i] += delta * delta2
        # Vitter sampling.
        frozen = tuple(float(x) for x in vec)
        if len(self._items) < self.size:
            self._items.append(frozen)
        else:
            assert self._rng is not None
            j = self._rng.randrange(self._seen)
            if j < self.size:
                self._items[j] = frozen

    def add_many(self, vecs: Sequence[Sequence[float]]) -> None:
        for v in vecs:
            self.add(v)

    # ---- read ----------------------------------------------------------

    @property
    def seen(self) -> int:
        return self._seen

    @property
    def dim(self) -> int | None:
        return self._dim

    def sample(self) -> tuple[tuple[float, ...], ...]:
        return tuple(self._items)

    def centroid(self) -> tuple[float, ...]:
        return tuple(self._mean)

    def std(self) -> tuple[float, ...]:
        if self._seen < 2 or self._dim is None:
            return tuple(0.0 for _ in range(self._dim or 0))
        return tuple(
            math.sqrt(m2 / (self._seen - 1)) for m2 in self._m2
        )


__all__ = ["EmbeddingReservoir"]
