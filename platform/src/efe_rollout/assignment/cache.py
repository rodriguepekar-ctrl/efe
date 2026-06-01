"""Assignment cache abstraction.

The hot path **never** computes a hash after the first request — it
reads a ``(experiment_id, user_id) → variant_id`` mapping from the
cache. On a miss we compute, persist, and return. On a hit the round
trip is sub-millisecond.

Two implementations :

* :class:`InMemoryAssignmentCache` — dict of dicts, used by tests and
  by single-process deployments.
* ``redis://`` — out of scope for the MVP tests but the Protocol is
  stable so a :class:`RedisAssignmentCache` can plug in without code
  changes to callers.
"""

from __future__ import annotations

import asyncio
from typing import Protocol


class AssignmentCache(Protocol):
    async def get(
        self, *, experiment_id: str, user_id: str,
    ) -> str | None:  # pragma: no cover
        ...

    async def put(
        self, *, experiment_id: str, user_id: str, variant_id: str | None,
    ) -> None:  # pragma: no cover
        ...

    async def delete(
        self, *, experiment_id: str, user_id: str,
    ) -> None:  # pragma: no cover
        ...

    async def invalidate_experiment(
        self, experiment_id: str,
    ) -> None:  # pragma: no cover
        ...


class InMemoryAssignmentCache:
    """In-process cache — safe for unit tests and single-pod use.

    Uses an :class:`asyncio.Lock` around mutation so concurrent tasks
    in the same event loop don't race on the same ``(exp, user)`` key.
    We explicitly store the sentinel ``""`` (empty string) for holdout
    users whose variant is ``None`` to distinguish "known holdout" from
    "unknown miss" on read.
    """

    _HOLDOUT_SENTINEL: str = "\x00holdout"

    __slots__ = ("_lock", "_store")

    def __init__(self) -> None:
        self._store: dict[str, dict[str, str]] = {}
        self._lock = asyncio.Lock()

    async def get(
        self, *, experiment_id: str, user_id: str,
    ) -> str | None:
        exp = self._store.get(experiment_id)
        if not exp:
            return None
        raw = exp.get(user_id)
        if raw is None:
            return None
        if raw == self._HOLDOUT_SENTINEL:
            return None
        return raw

    async def has(
        self, *, experiment_id: str, user_id: str,
    ) -> bool:
        """Distinguishes ``None-is-holdout`` from ``None-is-miss``."""
        exp = self._store.get(experiment_id)
        return bool(exp and user_id in exp)

    async def put(
        self, *, experiment_id: str, user_id: str, variant_id: str | None,
    ) -> None:
        async with self._lock:
            exp = self._store.setdefault(experiment_id, {})
            exp[user_id] = variant_id if variant_id else self._HOLDOUT_SENTINEL

    async def delete(
        self, *, experiment_id: str, user_id: str,
    ) -> None:
        async with self._lock:
            exp = self._store.get(experiment_id)
            if exp is not None:
                exp.pop(user_id, None)

    async def invalidate_experiment(self, experiment_id: str) -> None:
        async with self._lock:
            self._store.pop(experiment_id, None)

    async def size(self, experiment_id: str) -> int:
        return len(self._store.get(experiment_id, {}))


__all__ = ["AssignmentCache", "InMemoryAssignmentCache"]
