"""Batch updater that replays a window of outcomes into bandit state.

Batch vs online :

* Online updates (per request) would need a DB lock per call — fine
  under ~100 rps, but latency-sensitive.
* Batch updates every 5 min replay every outcome since the last tick
  and write once. Scales to 100 k rps.

Atomicity : ``apply_batch`` does ``load → mutate → save``. Two
concurrent calls targeting the same experiment must not lose updates
(a classic read-modify-write race). :class:`BanditUpdater` serialises
concurrent invocations per ``experiment_id`` via a
:class:`asyncio.Lock` registry, so a racing second call waits for the
first to finish its write before starting its own load.
"""

from __future__ import annotations

import asyncio
from collections.abc import Iterable
from dataclasses import dataclass

from efe_rollout.bandits.state import (
    BanditArmState,
    BanditStateStore,
    InMemoryBanditStateStore,
)
from efe_rollout.clock import Clock, SystemClock
from efe_rollout.records import Outcome


@dataclass(frozen=True, slots=True)
class BanditUpdateSummary:
    experiment_id: str
    processed_outcomes: int
    pulls_added: dict[str, int]
    successes_added: dict[str, int]


class BanditUpdater:
    """Applies a window of outcomes to a store atomically per experiment.

    The assumption is that ``record`` outcomes carry a ``variant_id``
    (provided by the assigner via exposures) and a binary ``metric_value``
    (``0``/``1`` = failure/success). Non-binary metrics are clamped to
    [0, 1] before the Beta update.
    """

    __slots__ = ("_clock", "_locks", "_locks_lock", "_store")

    def __init__(
        self,
        *,
        store: BanditStateStore | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._store = store or InMemoryBanditStateStore()
        self._clock = clock or SystemClock()
        # Per-experiment locks. The outer ``_locks_lock`` serialises
        # the dict mutation while installing a new entry.
        self._locks: dict[str, asyncio.Lock] = {}
        self._locks_lock = asyncio.Lock()

    async def apply_batch(
        self,
        *,
        experiment_id: str,
        outcomes: Iterable[Outcome],
        primary_metric: str,
    ) -> BanditUpdateSummary:
        # Materialise outcomes once — the iterable may be a generator
        # and we want a stable re-runnable sequence after taking the lock.
        buffered = tuple(outcomes)
        lock = await self._get_lock(experiment_id)
        async with lock:
            existing = await self._store.load(experiment_id)
            pulls_added: dict[str, int] = {}
            successes_added: dict[str, int] = {}
            processed = 0
            for o in buffered:
                if o.metric_name != primary_metric:
                    continue
                if o.variant_id is None:
                    continue
                reward = min(1.0, max(0.0, float(o.metric_value)))
                arm = existing.get(
                    o.variant_id,
                    BanditArmState(variant_id=o.variant_id),
                )
                before_successes = arm.n_successes
                arm.record(reward)
                existing[o.variant_id] = arm
                pulls_added[o.variant_id] = pulls_added.get(o.variant_id, 0) + 1
                delta_s = arm.n_successes - before_successes
                if delta_s:
                    successes_added[o.variant_id] = (
                        successes_added.get(o.variant_id, 0) + delta_s
                    )
                processed += 1
            await self._store.save(experiment_id, existing)
            return BanditUpdateSummary(
                experiment_id=experiment_id,
                processed_outcomes=processed,
                pulls_added=pulls_added,
                successes_added=successes_added,
            )

    async def _get_lock(self, experiment_id: str) -> asyncio.Lock:
        # Fast path : lock already exists.
        lock = self._locks.get(experiment_id)
        if lock is not None:
            return lock
        async with self._locks_lock:
            # Double-check in case another task created it in between.
            lock = self._locks.get(experiment_id)
            if lock is None:
                lock = asyncio.Lock()
                self._locks[experiment_id] = lock
            return lock


__all__ = ["BanditUpdateSummary", "BanditUpdater"]
