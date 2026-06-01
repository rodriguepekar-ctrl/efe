"""Per-tenant quotas — token bucket impl.

Each :class:`QuotaKind` has its own bucket per tenant. Buckets are kept
in-memory for the MVP ; production uses Redis with an atomic Lua script
for cross-pod consistency.
"""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass, field
from typing import Protocol

from efe_security.clock_adapter import _now_mono  # local helper (created below)
from efe_security.errors import QuotaExceeded

__all__ = ["QuotaBucket", "QuotaKind", "QuotaManager", "QuotaSpec"]


class QuotaKind(str, enum.Enum):
    api_rps = "api_rps"
    ingest_eps = "ingest_eps"
    storage_bytes = "storage_bytes"
    cpu_hours = "cpu_hours"
    jobs_per_day = "jobs_per_day"
    schedules_total = "schedules_total"


@dataclass(frozen=True, slots=True)
class QuotaSpec:
    kind: QuotaKind
    capacity: float        # max tokens
    refill_per_s: float    # tokens added per second (0 for hard caps)
    hard_cap: float | None = None  # for cumulative counters


@dataclass(slots=True)
class QuotaBucket:
    spec: QuotaSpec
    tokens: float
    last_refill_mono: float
    lifetime_used: float = 0.0


class _ClockProto(Protocol):
    def now_monotonic_s(self) -> float: ...  # pragma: no cover


class QuotaManager:
    """Token-bucket manager keyed by ``(tenant_id, QuotaKind)``."""

    __slots__ = ("_buckets", "_clock", "_defaults", "_lock")

    def __init__(
        self,
        *,
        defaults: dict[QuotaKind, QuotaSpec] | None = None,
        clock: _ClockProto | None = None,
    ) -> None:
        self._defaults = defaults or _default_quotas()
        self._clock: _ClockProto = clock if clock is not None else _SystemMonotonicClock()
        self._buckets: dict[tuple[str, QuotaKind], QuotaBucket] = {}
        self._lock = asyncio.Lock()

    async def consume(
        self,
        *,
        tenant_id: str,
        kind: QuotaKind,
        cost: float = 1.0,
    ) -> None:
        """Consume ``cost`` from the bucket or raise :class:`QuotaExceeded`.

        Two bucket shapes are supported :

        * **Rate-limit** (``capacity > 0``) — classic token bucket.
        * **Cumulative only** (``capacity == 0`` and ``hard_cap > 0``) —
          pure counter against a total ceiling ; rate check is skipped
          because there is no refill.
        """
        if cost < 0:
            raise ValueError("cost must be >= 0")
        async with self._lock:
            bucket = self._buckets.get((tenant_id, kind))
            if bucket is None:
                spec = self._defaults.get(kind)
                if spec is None:
                    return  # unknown quotas pass-through
                bucket = QuotaBucket(
                    spec=spec,
                    tokens=spec.capacity,
                    last_refill_mono=self._clock.now_monotonic_s(),
                )
                self._buckets[(tenant_id, kind)] = bucket
            now = self._clock.now_monotonic_s()
            elapsed = max(0.0, now - bucket.last_refill_mono)
            if bucket.spec.capacity > 0:
                bucket.tokens = min(
                    bucket.spec.capacity,
                    bucket.tokens + elapsed * bucket.spec.refill_per_s,
                )
            bucket.last_refill_mono = now

            # Cumulative cap check — always applies if set.
            if bucket.spec.hard_cap is not None:
                if bucket.lifetime_used + cost > bucket.spec.hard_cap:
                    raise QuotaExceeded(
                        f"hard cap exceeded for {kind.value}",
                        detail={
                            "tenant_id": tenant_id,
                            "lifetime_used": bucket.lifetime_used,
                            "cap": bucket.spec.hard_cap,
                        },
                    )

            # Rate-limit check — only applies to buckets with a capacity.
            if bucket.spec.capacity > 0 and bucket.tokens < cost:
                raise QuotaExceeded(
                    f"{kind.value} rate-limit exhausted for tenant {tenant_id}",
                    detail={
                        "remaining_tokens": bucket.tokens,
                        "cost": cost,
                    },
                )
            if bucket.spec.capacity > 0:
                bucket.tokens -= cost
            bucket.lifetime_used += cost

    async def snapshot(self) -> dict[tuple[str, QuotaKind], QuotaBucket]:
        async with self._lock:
            return {k: QuotaBucket(
                spec=v.spec,
                tokens=v.tokens,
                last_refill_mono=v.last_refill_mono,
                lifetime_used=v.lifetime_used,
            ) for k, v in self._buckets.items()}


class _SystemMonotonicClock:
    def now_monotonic_s(self) -> float:
        import time as _t

        return _t.monotonic()


def _default_quotas() -> dict[QuotaKind, QuotaSpec]:
    return {
        QuotaKind.api_rps: QuotaSpec(kind=QuotaKind.api_rps, capacity=120.0, refill_per_s=60.0),
        QuotaKind.ingest_eps: QuotaSpec(kind=QuotaKind.ingest_eps, capacity=10_000.0, refill_per_s=5_000.0),
        QuotaKind.storage_bytes: QuotaSpec(
            kind=QuotaKind.storage_bytes,
            capacity=0.0,
            refill_per_s=0.0,
            hard_cap=100.0 * 1024**3,
        ),
        QuotaKind.cpu_hours: QuotaSpec(
            kind=QuotaKind.cpu_hours,
            capacity=0.0,
            refill_per_s=0.0,
            hard_cap=1000.0,
        ),
        QuotaKind.jobs_per_day: QuotaSpec(
            kind=QuotaKind.jobs_per_day,
            capacity=100_000.0,
            refill_per_s=100_000.0 / 86_400.0,
        ),
        QuotaKind.schedules_total: QuotaSpec(
            kind=QuotaKind.schedules_total,
            capacity=0.0,
            refill_per_s=0.0,
            hard_cap=10_000.0,
        ),
    }
