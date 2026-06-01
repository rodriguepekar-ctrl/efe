"""Bandit arm state — mutable Beta(α, β) counters per arm.

Thompson Sampling needs ``(α, β)`` pairs for each arm; UCB1 needs
``(n_pulls, mean_reward)``. We store both and present whichever the
sampler asks for.

The state lives under an ``asyncio.Lock`` so concurrent batch updates
don't race: a missed update quietly corrupts regret measurements.

For persistence, :class:`InMemoryBanditStateStore` is a dict of dicts
used by unit tests. Production wires a DB-backed store that persists
into the ``bandit_state`` table.
"""

from __future__ import annotations

import asyncio
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol


@dataclass(slots=True)
class BanditArmState:
    variant_id: str
    alpha: float = 1.0
    beta: float = 1.0
    n_pulls: int = 0
    n_successes: int = 0

    @property
    def mean_reward(self) -> float:
        return self.n_successes / self.n_pulls if self.n_pulls else 0.0

    def snapshot(self) -> BanditArmState:
        return BanditArmState(
            variant_id=self.variant_id,
            alpha=self.alpha,
            beta=self.beta,
            n_pulls=self.n_pulls,
            n_successes=self.n_successes,
        )

    def record(self, reward: float) -> None:
        """Update counters by one observation.

        ``reward`` is interpreted as:
          * 0 → failure (no success); alpha is unchanged, beta +=1.
          * 1 → success; alpha +=1, beta unchanged.
          * anything in between: fractional — alpha += reward,
            beta += 1-reward.
        """
        if reward < 0 or reward > 1:
            raise ValueError(f"reward must be in [0, 1], got {reward}")
        self.n_pulls += 1
        self.alpha += reward
        self.beta += 1.0 - reward
        if reward >= 0.5:
            self.n_successes += 1


class BanditStateStore(Protocol):
    async def load(
        self, experiment_id: str,
    ) -> dict[str, BanditArmState]:  # pragma: no cover
        ...

    async def save(
        self, experiment_id: str, state: dict[str, BanditArmState],
    ) -> None:  # pragma: no cover
        ...


class InMemoryBanditStateStore:
    """Asyncio-safe in-memory bandit state store."""

    __slots__ = ("_lock", "_store")

    def __init__(self) -> None:
        self._store: dict[str, dict[str, BanditArmState]] = {}
        self._lock = asyncio.Lock()

    async def load(
        self, experiment_id: str,
    ) -> dict[str, BanditArmState]:
        async with self._lock:
            state = self._store.get(experiment_id, {})
            return {vid: a.snapshot() for vid, a in state.items()}

    async def save(
        self, experiment_id: str, state: dict[str, BanditArmState],
    ) -> None:
        async with self._lock:
            self._store[experiment_id] = {
                vid: a.snapshot() for vid, a in state.items()
            }

    async def init(
        self, experiment_id: str, arms: Iterable[str],
    ) -> None:
        async with self._lock:
            self._store.setdefault(experiment_id, {})
            for vid in arms:
                self._store[experiment_id].setdefault(
                    vid, BanditArmState(variant_id=vid),
                )


__all__ = [
    "BanditArmState",
    "BanditStateStore",
    "InMemoryBanditStateStore",
]
