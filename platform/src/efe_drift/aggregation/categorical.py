"""Categorical counters — top-K + ``__other__`` fallback.

High-cardinality categorical features (``country`` with 200+ codes)
must be bucketed into a fixed **top-K** before histograms so the
downstream PSI / χ² calculations have a manageable number of
categories (invariant I2).

The top-K list is frozen at baseline time. This module provides :

* :class:`CategoricalCounter` — incrementally tallies occurrences
  over an arbitrary period, then exposes :meth:`top_k_with_other`
  which applies the bucketing rule.

* :func:`project_into_categories` — given an arbitrary counter and a
  **fixed** category list (plus sentinel ``__other__``), project the
  counter into that schema. Unknown categories go to ``__other__``.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass, field

from efe_drift.errors import ValidationError

OTHER_BUCKET = "__other__"


@dataclass(slots=True)
class CategoricalCounter:
    """Monotonic counter over string values."""

    counts: dict[str, int] = field(default_factory=dict)

    def add(self, value: str, *, n: int = 1) -> None:
        if n < 1:
            raise ValidationError(f"n must be ≥ 1, got {n}")
        if not value:
            raise ValidationError("empty category value")
        self.counts[value] = self.counts.get(value, 0) + n

    def add_many(self, values: Iterable[str]) -> None:
        for v in values:
            self.add(v)

    @property
    def total(self) -> int:
        return sum(self.counts.values())

    @property
    def cardinality(self) -> int:
        return len(self.counts)

    # ---- bucketing -----------------------------------------------------

    def top_k_with_other(
        self, *, k: int, tiebreak: str = "alphabetical",
    ) -> dict[str, int]:
        """Return a dict of at most ``k + 1`` entries (top-K + other).

        Ties are broken alphabetically by default so the bucketing is
        deterministic even when two categories have identical counts.
        """
        if k < 1:
            raise ValidationError(f"k must be ≥ 1, got {k}")
        if tiebreak not in ("alphabetical",):
            raise ValidationError(f"unknown tiebreak {tiebreak!r}")
        if not self.counts:
            return {}
        sorted_items = sorted(
            self.counts.items(),
            # Primary : higher count first. Secondary : alphabetical
            # by category name for determinism.
            key=lambda t: (-t[1], t[0]),
        )
        top = dict(sorted_items[:k])
        rest_total = sum(c for _, c in sorted_items[k:])
        if rest_total > 0:
            top[OTHER_BUCKET] = rest_total
        return top

    def categories_sorted(self) -> tuple[str, ...]:
        """Deterministic category order — alphabetical."""
        return tuple(sorted(self.counts))


def project_into_categories(
    counter: CategoricalCounter,
    *,
    frozen_categories: tuple[str, ...],
) -> dict[str, int]:
    """Project ``counter`` onto ``frozen_categories + __other__``.

    Exactly the operation the detection layer runs when computing
    drift against a baseline : categories not in the baseline list
    must be collapsed into the ``__other__`` bucket so the two
    distributions have the same support.
    """
    if OTHER_BUCKET in frozen_categories:
        raise ValidationError(
            f"{OTHER_BUCKET!r} must not be in the frozen_categories list",
        )
    expected = set(frozen_categories)
    out: dict[str, int] = {name: 0 for name in frozen_categories}
    other_total = 0
    for name, count in counter.counts.items():
        if name in expected:
            out[name] += count
        else:
            other_total += count
    if other_total > 0:
        out[OTHER_BUCKET] = other_total
    return out


__all__ = [
    "OTHER_BUCKET",
    "CategoricalCounter",
    "project_into_categories",
]
