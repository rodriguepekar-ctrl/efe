"""Per-tenant bulkhead — bounds ``inflight_requests(tenant)``."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field

from efe_swarm_integration.errors import BackpressureRejected

__all__ = ["Bulkhead", "BulkheadRegistry"]


@dataclass(slots=True)
class Bulkhead:
    """One semaphore per tenant."""

    tenant_id: str
    capacity: int
    queue_wait_ms: int
    _semaphore: asyncio.Semaphore = field(init=False, repr=False)
    _inflight: int = field(default=0, init=False)

    def __post_init__(self) -> None:
        if self.capacity < 1:
            raise ValueError("Bulkhead capacity must be >= 1")
        self._semaphore = asyncio.Semaphore(self.capacity)

    async def __aenter__(self) -> "Bulkhead":
        try:
            await asyncio.wait_for(
                self._semaphore.acquire(), timeout=self.queue_wait_ms / 1000.0
            )
        except asyncio.TimeoutError as exc:
            raise BackpressureRejected(
                f"bulkhead exhausted for tenant {self.tenant_id}",
                detail={"capacity": self.capacity, "queue_wait_ms": self.queue_wait_ms},
            ) from exc
        self._inflight += 1
        return self

    async def __aexit__(self, *_: object) -> None:
        self._inflight -= 1
        self._semaphore.release()

    @property
    def inflight(self) -> int:
        return self._inflight


class BulkheadRegistry:
    """One :class:`Bulkhead` per tenant. Created lazily."""

    __slots__ = ("_bulkheads", "_capacity", "_lock", "_queue_wait_ms")

    def __init__(self, *, capacity: int, queue_wait_ms: int) -> None:
        self._capacity = capacity
        self._queue_wait_ms = queue_wait_ms
        self._bulkheads: dict[str, Bulkhead] = {}
        self._lock = asyncio.Lock()

    async def for_tenant(self, tenant_id: str) -> Bulkhead:
        async with self._lock:
            bh = self._bulkheads.get(tenant_id)
            if bh is None:
                bh = Bulkhead(
                    tenant_id=tenant_id,
                    capacity=self._capacity,
                    queue_wait_ms=self._queue_wait_ms,
                )
                self._bulkheads[tenant_id] = bh
            return bh

    def inflight_snapshot(self) -> dict[str, int]:
        return {t: bh.inflight for t, bh in self._bulkheads.items()}
