"""Leader election Protocol + in-memory implementation.

In production, a Postgres ``pg_try_advisory_lock``-based implementation
lives in :mod:`efe_scheduling.storage.sql`. The in-memory implementation
here is used by tests and by single-node deployments.
"""

from __future__ import annotations

import asyncio
from typing import Protocol

__all__ = ["InMemoryLeaderElection", "LeaderElection"]


class LeaderElection(Protocol):
    """Cooperative leader election.

    Semantics :

    * :meth:`acquire` — try to become leader ; returns True on success.
    * :meth:`release` — give up leadership (idempotent).
    * :meth:`is_leader` — non-blocking read of the current state.
    * :meth:`renew` — refresh the lease TTL.

    All calls are safe to interleave with the scheduler tick loop.
    """

    async def acquire(self, *, ttl_s: int = 30) -> bool: ...  # pragma: no cover
    async def release(self) -> None: ...  # pragma: no cover
    async def renew(self, *, ttl_s: int = 30) -> bool: ...  # pragma: no cover
    def is_leader(self) -> bool: ...  # pragma: no cover


class InMemoryLeaderElection:
    """Single-process leader election using an :class:`asyncio.Lock`.

    Multiple instances of this class inside the same process share the
    same class-level registry, keyed by ``key`` — simulating the
    coordination that, in production, is done via SQL advisory locks.
    """

    _registry: dict[str, str] = {}
    _mutex: asyncio.Lock = asyncio.Lock()

    def __init__(self, key: str, owner_id: str) -> None:
        self._key = key
        self._owner_id = owner_id
        self._lease_expiry: float = 0.0

    async def acquire(self, *, ttl_s: int = 30) -> bool:
        async with self._mutex:
            current = self._registry.get(self._key)
            if current is None or current == self._owner_id:
                self._registry[self._key] = self._owner_id
                self._lease_expiry = ttl_s
                return True
            return False

    async def release(self) -> None:
        async with self._mutex:
            if self._registry.get(self._key) == self._owner_id:
                del self._registry[self._key]

    async def renew(self, *, ttl_s: int = 30) -> bool:
        async with self._mutex:
            if self._registry.get(self._key) == self._owner_id:
                self._lease_expiry = ttl_s
                return True
            return False

    def is_leader(self) -> bool:
        return self._registry.get(self._key) == self._owner_id
