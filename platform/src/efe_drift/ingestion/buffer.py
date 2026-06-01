"""In-memory event buffer with bounded size + backpressure.

Mirrors the ``exposure_buffer`` pattern in :mod:`efe_rollout.metrics`
but tuned for the drift ingestion path :

* :meth:`put` is :class:`~asyncio.Lock`-guarded ; a second concurrent
  producer never loses an event due to races on :attr:`_items`.
* :attr:`put_nowait` raises :class:`BufferFull` when at capacity
  instead of blocking — callers can log and drop, or back off.
* :meth:`drain_batch` returns up to ``max_items`` and removes them
  from the buffer atomically.
* :meth:`size` and :attr:`capacity` are cheap reads, safe from any
  task.

The buffer is **deliberately simple** : no priority, no per-tenant
partitioning. Fairness is the concern of the caller (who can shard
into one buffer per tenant).
"""

from __future__ import annotations

import asyncio
from collections import deque
from typing import Generic, TypeVar

from efe_drift.errors import BufferFull

T = TypeVar("T")


class EventBuffer(Generic[T]):
    """Bounded FIFO buffer for ingestion events.

    The type parameter ``T`` is typically
    :class:`~efe_drift.records.InferenceEvent` but the buffer itself
    is value-agnostic so we can reuse it for outcomes / samples.
    """

    __slots__ = ("_capacity", "_items", "_lock", "_not_empty")

    def __init__(self, *, capacity: int) -> None:
        if capacity < 1:
            raise ValueError(f"capacity must be ≥ 1, got {capacity}")
        self._capacity = int(capacity)
        self._items: deque[T] = deque()
        self._lock = asyncio.Lock()
        self._not_empty = asyncio.Event()

    @property
    def capacity(self) -> int:
        return self._capacity

    def size(self) -> int:
        return len(self._items)

    def is_empty(self) -> bool:
        return not self._items

    def is_full(self) -> bool:
        return len(self._items) >= self._capacity

    # ---- producer ------------------------------------------------------

    async def put(self, item: T) -> None:
        """Append ``item`` unless the buffer is at capacity.

        Raises :class:`BufferFull` rather than blocking — in drift
        ingestion we prefer to shed load explicitly.
        """
        async with self._lock:
            if len(self._items) >= self._capacity:
                raise BufferFull(
                    f"buffer full ({self._capacity} items) — refusing write",
                    detail={"capacity": self._capacity},
                )
            self._items.append(item)
            self._not_empty.set()

    def put_nowait(self, item: T) -> None:
        """Synchronous variant — same semantics as :meth:`put` but
        without the lock (caller accepts the race in exchange for
        avoiding the ``await``)."""
        if len(self._items) >= self._capacity:
            raise BufferFull(
                f"buffer full ({self._capacity} items) — refusing write",
                detail={"capacity": self._capacity},
            )
        self._items.append(item)
        self._not_empty.set()

    # ---- consumer ------------------------------------------------------

    async def drain_batch(self, max_items: int) -> tuple[T, ...]:
        """Pop up to ``max_items`` items in FIFO order."""
        if max_items < 1:
            raise ValueError(f"max_items must be ≥ 1, got {max_items}")
        async with self._lock:
            n = min(len(self._items), max_items)
            batch = tuple(self._items.popleft() for _ in range(n))
            if not self._items:
                self._not_empty.clear()
            return batch

    async def wait_non_empty(self, *, timeout: float | None = None) -> bool:
        """Block until the buffer has at least one item.

        Returns ``True`` if an item is available; ``False`` on
        timeout. Useful for pull-based consumers.
        """
        if self._items:
            return True
        try:
            await asyncio.wait_for(self._not_empty.wait(), timeout=timeout)
            return True
        except TimeoutError:
            return False


__all__ = ["EventBuffer"]
