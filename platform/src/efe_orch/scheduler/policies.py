"""Core scheduling logic — pool selection.

Given a :class:`RunShape` (dataset size, model family, priority, GPU
hint) and live :class:`PoolCapacity`, the :class:`PoolSelector` picks
the cheapest pool that satisfies every constraint:

1. **Capacity** — ``current_runs < max_concurrent_runs``
2. **Dataset fit** — ``shape.dataset_rows <= pool.max_dataset_rows``
3. **GPU requirement** — if the shape needs GPU, the pool must have one
4. **Priority class** — ``priority ∈ pool.priority_classes``
5. **Cost** — among all candidates, return the minimum cost_score

On no match it raises :class:`NoSuitablePool` (500-class) or
:class:`PoolSaturated` (503, retryable) depending on which constraint
failed.

The selector is **pure** — no DB, no K8s, no I/O. The
:class:`SchedulerService` gathers the live capacity from the DB and
hands it in.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Literal

from efe_orch.config import PoolSettings
from efe_orch.errors import NoSuitablePool, PoolSaturated
from efe_orch.scheduler.pools import PoolRegistry


@dataclass(frozen=True, slots=True)
class RunShape:
    """The caller's request, distilled to what matters for placement."""

    dataset_rows: int
    priority: Literal["standard", "premium"] = "standard"
    needs_gpu: bool = False
    requested_pool: str | None = None  # explicit override (optional)
    allowed_pools: tuple[str, ...] = ()  # per-tenant whitelist (optional)

    def __post_init__(self) -> None:
        if self.dataset_rows < 0:
            raise ValueError("dataset_rows must be >= 0")


@dataclass(frozen=True, slots=True)
class PoolCapacity:
    """Snapshot of how many active runs each pool currently holds."""

    current: Mapping[str, int]

    def load(self, pool_name: str) -> int:
        return int(self.current.get(pool_name, 0))


@dataclass(frozen=True, slots=True)
class SchedulingDecision:
    """What the selector returned."""

    pool: str
    reason: str
    cost_score: int
    considered: tuple[str, ...]
    rejected: tuple[tuple[str, str], ...]  # (pool_name, reason)


class PoolSelector:
    """Pure scheduling decision maker."""

    def __init__(self, registry: PoolRegistry) -> None:
        self._registry = registry

    def pick(
        self,
        shape: RunShape,
        capacity: PoolCapacity,
    ) -> SchedulingDecision:
        considered: list[str] = []
        rejected: list[tuple[str, str]] = []

        # 0) Tenant-allowed whitelist (empty tuple = no restriction).
        whitelist = set(shape.allowed_pools) if shape.allowed_pools else None

        # 1) Explicit override: honour it *if* it survives the checks.
        candidates: list[PoolSettings]
        if shape.requested_pool is not None:
            try:
                forced = self._registry.by_name(shape.requested_pool)
            except KeyError as exc:
                raise NoSuitablePool(
                    f"requested pool {shape.requested_pool!r} is unknown",
                ) from exc
            candidates = [forced]
        else:
            # Cheapest-first; gpu/cpu segmentation happens per-check.
            candidates = list(self._registry.sort_by_cost())

        for pool in candidates:
            considered.append(pool.name)
            if whitelist is not None and pool.name not in whitelist:
                rejected.append((pool.name, "not in tenant whitelist"))
                continue
            if shape.priority not in pool.priority_classes:
                rejected.append((pool.name, "priority not admitted"))
                continue
            if shape.needs_gpu and pool.gpu is None:
                rejected.append((pool.name, "needs gpu, cpu-only pool"))
                continue
            if shape.dataset_rows > pool.max_dataset_rows:
                rejected.append((pool.name, "dataset too large"))
                continue
            current = capacity.load(pool.name)
            if current >= pool.max_concurrent_runs:
                rejected.append((pool.name, "saturated"))
                continue

            return SchedulingDecision(
                pool=pool.name,
                reason=_explain(pool, shape, current),
                cost_score=pool.cost_score,
                considered=tuple(considered),
                rejected=tuple(rejected),
            )

        # No pool matched → decide between 503 (transient saturation)
        # and 500 (no pool structurally fits).
        saturated = [r for r in rejected if r[1] == "saturated"]
        any_other_reject = len(rejected) - len(saturated) > 0

        if saturated and not any_other_reject:
            raise PoolSaturated(
                "every admissible pool is at capacity",
                detail={"rejected": list(rejected)},
            )
        raise NoSuitablePool(
            "no pool matches the run shape",
            detail={"rejected": list(rejected),
                    "dataset_rows": shape.dataset_rows,
                    "needs_gpu": shape.needs_gpu,
                    "priority": shape.priority},
        )


def _explain(pool: PoolSettings, shape: RunShape, current: int) -> str:
    gpu_s = " gpu" if pool.gpu else " cpu"
    return (
        f"selected{gpu_s} pool {pool.name!r} "
        f"(cost={pool.cost_score}, load={current}/{pool.max_concurrent_runs}, "
        f"dataset_rows={shape.dataset_rows}, priority={shape.priority})"
    )


__all__ = [
    "PoolCapacity",
    "PoolSelector",
    "RunShape",
    "SchedulingDecision",
]
