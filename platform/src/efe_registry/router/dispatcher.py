"""Weighted / stratified dispatcher — picks one RoutingEntry per request.

The stratified variant ensures exact distribution on small percentages
(e.g. 1 % at 1000 req actually hits candidate 10 times, not "around
10" with poisson noise). This matters for canary validation at low
traffic.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Protocol

from efe_registry.core.artifact import ArtifactRole
from efe_registry.errors import RoutingInvariantBroken
from efe_registry.router.routing_table import RoutingEntry


class Dispatcher(Protocol):
    def pick(
        self, entries: tuple[RoutingEntry, ...],
    ) -> RoutingEntry:  # pragma: no cover
        ...


@dataclass
class WeightedDispatcher:
    """Poisson-style random-weighted dispatcher."""

    rng_seed: int = 0
    _counter: int = 0

    def pick(self, entries: tuple[RoutingEntry, ...]) -> RoutingEntry:
        dispatchable = _dispatchable(entries)
        total = sum(e.weight_percent for e in dispatchable)
        if total != 100:
            raise RoutingInvariantBroken(
                f"weights sum to {total}, expected 100",
            )
        # PRNG — deterministic in tests via rng_seed if used repeatedly.
        self._counter = (self._counter * 1103515245 + 12345 + self.rng_seed)
        bucket = (self._counter >> 16) % 100
        acc = 0
        for e in dispatchable:
            acc += e.weight_percent
            if bucket < acc:
                return e
        return dispatchable[-1]


class StratifiedDispatcher:
    """Exact stratified dispatcher — used in production.

    Keeps a thread-safe atomic counter ; index = counter % 100 selects
    the bucket deterministically and cycles, giving exact percentages.
    Two routers running in parallel are only "locally stratified" —
    but at aggregate levels the distribution remains exact over N·100
    requests.
    """

    __slots__ = ("_counter", "_lock")

    def __init__(self) -> None:
        self._counter = 0
        self._lock = threading.Lock()

    def pick(self, entries: tuple[RoutingEntry, ...]) -> RoutingEntry:
        dispatchable = _dispatchable(entries)
        total = sum(e.weight_percent for e in dispatchable)
        if total != 100:
            raise RoutingInvariantBroken(
                f"weights sum to {total}, expected 100",
            )
        with self._lock:
            bucket = self._counter % 100
            self._counter += 1
        acc = 0
        for e in dispatchable:
            acc += e.weight_percent
            if bucket < acc:
                return e
        return dispatchable[-1]


def _dispatchable(
    entries: tuple[RoutingEntry, ...],
) -> tuple[RoutingEntry, ...]:
    """Filter out shadow and standby_warm — they never serve user traffic."""
    out = tuple(
        e for e in entries
        if e.role in (ArtifactRole.CHAMPION, ArtifactRole.CANDIDATE)
    )
    if not out:
        raise RoutingInvariantBroken(
            "no dispatchable routing entries (champion/candidate) found",
        )
    return out


__all__ = [
    "Dispatcher",
    "StratifiedDispatcher",
    "WeightedDispatcher",
]
