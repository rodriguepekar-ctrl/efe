"""Multi-armed bandit sampling policies.

Three pure-function samplers that take a ``list[BanditArmState]`` and
return the chosen arm :

* :class:`ThompsonSampler` — draws one sample from each ``Beta(α, β)``
  and selects the argmax. Near-optimal regret ``O(√(K T log T))`` and
  the only sampler that makes sense for bandits on binary outcomes
  (the MVP's focus).

* :class:`Ucb1Sampler` — deterministic upper-confidence bound:
  ``mean + √(2 ln T / n)``. Popular for continuous / bounded rewards.
  Has to sweep all arms at least once before it becomes useful.

* :class:`EpsilonGreedySampler` — with probability ``ε`` pick uniform;
  otherwise the arm with the best empirical mean. Simpler but worse
  regret than Thompson on most workloads. ``ε`` decays geometrically
  each step.

All samplers are deterministic under a seeded
:class:`~random.Random` so tests can replay decisions.
"""

from __future__ import annotations

import math
import random
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Protocol

from efe_rollout.bandits.state import BanditArmState
from efe_rollout.errors import BanditError


class BanditSampler(Protocol):
    def choose(
        self, arms: Sequence[BanditArmState],
    ) -> BanditArmState:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class SamplerResult:
    chosen: BanditArmState
    scores: dict[str, float]


class ThompsonSampler:
    """Thompson-sampling over Beta posteriors."""

    __slots__ = ("_minimum_allocation_bps", "_rng")

    def __init__(
        self,
        *,
        seed: int = 20260420,
        minimum_allocation_pct: int = 0,
    ) -> None:
        if not 0 <= minimum_allocation_pct <= 50:
            raise ValueError(
                "minimum_allocation_pct must be in [0, 50]",
            )
        self._rng = random.Random(seed)
        self._minimum_allocation_bps = minimum_allocation_pct * 100

    def choose(
        self, arms: Sequence[BanditArmState],
    ) -> BanditArmState:
        if not arms:
            raise BanditError("thompson sampler requires at least one arm")
        # Minimum-allocation guard (anti-starvation): with probability
        # equal to minimum_allocation_bps / 10000 per arm, pick uniform.
        if self._minimum_allocation_bps > 0:
            n_arms = len(arms)
            roll = self._rng.randint(0, 9_999)
            if roll < self._minimum_allocation_bps * n_arms:
                return arms[roll % n_arms]
        # Otherwise standard Thompson sampling.
        best = arms[0]
        best_score = -1.0
        for a in arms:
            score = self._rng.betavariate(a.alpha, a.beta)
            if score > best_score:
                best_score = score
                best = a
        return best


class Ucb1Sampler:
    """UCB1 (Auer, Cesa-Bianchi, Fischer 2002)."""

    __slots__ = ("_c",)

    def __init__(self, *, exploration_constant: float = 2.0) -> None:
        if exploration_constant <= 0:
            raise ValueError("exploration_constant must be positive")
        self._c = float(exploration_constant)

    def choose(
        self, arms: Sequence[BanditArmState],
    ) -> BanditArmState:
        if not arms:
            raise BanditError("UCB1 sampler requires at least one arm")
        # Play any untried arm first.
        for a in arms:
            if a.n_pulls == 0:
                return a
        total = sum(a.n_pulls for a in arms)
        # Avoid log(0): when all n_pulls==0 we already returned above.
        log_t = math.log(total)
        best = arms[0]
        best_score = -math.inf
        for a in arms:
            score = a.mean_reward + math.sqrt(self._c * log_t / a.n_pulls)
            if score > best_score:
                best_score = score
                best = a
        return best


class EpsilonGreedySampler:
    """ε-greedy with geometric decay."""

    __slots__ = ("_decay", "_epsilon", "_rng")

    def __init__(
        self,
        *,
        epsilon: float = 0.05,
        decay: float = 1.0,
        seed: int = 20260420,
    ) -> None:
        if not 0.0 < epsilon < 1.0:
            raise ValueError(f"epsilon must be in (0, 1), got {epsilon}")
        if not 0.0 < decay <= 1.0:
            raise ValueError(f"decay must be in (0, 1], got {decay}")
        self._epsilon = float(epsilon)
        self._decay = float(decay)
        self._rng = random.Random(seed)

    def choose(
        self, arms: Sequence[BanditArmState],
    ) -> BanditArmState:
        if not arms:
            raise BanditError("ε-greedy requires at least one arm")
        if self._rng.random() < self._epsilon:
            chosen = arms[self._rng.randrange(len(arms))]
        else:
            chosen = max(arms, key=lambda a: a.mean_reward)
        self._epsilon *= self._decay
        return chosen

    @property
    def epsilon(self) -> float:
        return self._epsilon


__all__ = [
    "BanditSampler",
    "EpsilonGreedySampler",
    "SamplerResult",
    "ThompsonSampler",
    "Ucb1Sampler",
]
