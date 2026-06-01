"""Process-local routing cache with PubSub-triggered invalidation.

Read-hot-path flow:

1. Caller asks :meth:`get_entries`.
2. If cached and not expired, return directly.
3. Otherwise fetch from the supplied loader coroutine, populate cache,
   return.
4. When a ``routing.updated`` message arrives on NATS/Redis PubSub,
   the subscriber calls :meth:`invalidate` with the tenant_key to drop
   the stale entry.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

from efe_registry.clock import Clock, SystemClock
from efe_registry.router.routing_table import RoutingEntry, RoutingKey

T = TypeVar("T")


@dataclass(frozen=True, slots=True)
class _CacheEntry(Generic[T]):
    value: T
    fetched_at_mono: float
    version: int


Loader = Callable[[RoutingKey], Awaitable[tuple[tuple[RoutingEntry, ...], int]]]


class RoutingCache:
    """Tiny TTL cache with explicit invalidation."""

    def __init__(
        self,
        *,
        loader: Loader,
        ttl_s: float = 1.0,
        clock: Clock | None = None,
    ) -> None:
        self._loader = loader
        self._ttl = float(ttl_s)
        self._clock: Clock = clock or SystemClock()
        self._data: dict[tuple[str, str, str], _CacheEntry[tuple[RoutingEntry, ...]]] = {}
        self._locks: dict[tuple[str, str, str], asyncio.Lock] = {}

    async def get(self, key: RoutingKey) -> tuple[RoutingEntry, ...]:
        k = (key.tenant_id, key.task_id, key.variant)
        now = self._clock.now_monotonic_s()
        hit = self._data.get(k)
        if hit is not None and now - hit.fetched_at_mono < self._ttl:
            return hit.value

        # Lock per key to avoid stampede.
        lock = self._locks.setdefault(k, asyncio.Lock())
        async with lock:
            hit = self._data.get(k)
            if hit is not None and self._clock.now_monotonic_s() - hit.fetched_at_mono < self._ttl:
                return hit.value
            entries, version = await self._loader(key)
            self._data[k] = _CacheEntry(
                value=entries,
                fetched_at_mono=self._clock.now_monotonic_s(),
                version=version,
            )
            return entries

    def invalidate(self, tenant_key: str) -> int:
        """Invalidate entries matching ``routing:{tenant}:{task}``.

        Returns the number of cache entries dropped. Also prunes the
        per-key asyncio.Lock so long-running routers don't accumulate
        stale lock objects (M4 fix).
        """
        # tenant_key format: "routing:{tenant}:{task}"
        parts = tenant_key.split(":")
        if len(parts) < 3 or parts[0] != "routing":
            return 0
        tenant_id = parts[1]
        task_id = parts[2]
        dropped = 0
        for k in list(self._data.keys()):
            if k[0] == tenant_id and k[1] == task_id:
                self._data.pop(k, None)
                self._locks.pop(k, None)
                dropped += 1
        return dropped

    def invalidate_all(self) -> int:
        n = len(self._data)
        self._data.clear()
        self._locks.clear()
        return n

    def size(self) -> int:
        return len(self._data)


__all__ = ["Loader", "RoutingCache"]
