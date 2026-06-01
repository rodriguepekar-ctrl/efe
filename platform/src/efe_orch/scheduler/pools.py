"""Pool registry — wraps the immutable :class:`PoolSettings` list.

The registry is built from :class:`SchedulerSettings.pools` once at
service construction. It exposes:

* :meth:`all`               — every configured pool
* :meth:`by_name`           — O(1) lookup
* :meth:`sort_by_cost`      — ascending-cost iterator, used by the
  selector's decision tree
* :meth:`admits_priority`   — predicate used to filter candidates
"""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass

from efe_orch.config import PoolSettings


@dataclass(frozen=True, slots=True)
class PoolRegistry:
    """Typed wrapper around a tuple of :class:`PoolSettings`."""

    pools: tuple[PoolSettings, ...]

    def __post_init__(self) -> None:
        names = [p.name for p in self.pools]
        if len(names) != len(set(names)):
            raise ValueError("duplicate pool names in registry")

    # ---- basic lookups ----

    def all(self) -> tuple[PoolSettings, ...]:
        return self.pools

    def by_name(self, name: str) -> PoolSettings:
        for p in self.pools:
            if p.name == name:
                return p
        raise KeyError(f"pool {name!r} is not registered")

    def has(self, name: str) -> bool:
        return any(p.name == name for p in self.pools)

    # ---- filtered views ----

    def sort_by_cost(self) -> Iterator[PoolSettings]:
        """Iterate pools in *ascending* ``cost_score`` order.

        Ties are broken by ``max_concurrent_runs`` (higher first) so
        that two pools of the same cost are used preferentially in the
        more-elastic one.
        """
        return iter(sorted(
            self.pools,
            key=lambda p: (p.cost_score, -p.max_concurrent_runs),
        ))

    def admits_priority(self, pool_name: str, priority: str) -> bool:
        """Return True if the pool's ``priority_classes`` admits ``priority``."""
        try:
            pool = self.by_name(pool_name)
        except KeyError:
            return False
        return priority in pool.priority_classes

    def cpu_only(self) -> tuple[PoolSettings, ...]:
        return tuple(p for p in self.pools if p.gpu is None)

    def gpu_pools(self) -> tuple[PoolSettings, ...]:
        return tuple(p for p in self.pools if p.gpu is not None)


__all__ = ["PoolRegistry"]
