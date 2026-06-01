"""Quota enforcer — thin wrapper over the Lua token-bucket."""

from __future__ import annotations

from dataclasses import dataclass

from efe_registry.clock import Clock, SystemClock
from efe_registry.errors import QuotaExceeded
from efe_registry.router.lua_runner import LuaRunner


@dataclass(frozen=True, slots=True)
class QuotaSpec:
    capacity: float = 100.0
    refill_per_s: float = 10.0
    cost: float = 1.0


class QuotaEnforcer:
    def __init__(
        self,
        *,
        lua: LuaRunner,
        default_spec: QuotaSpec | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._lua = lua
        self._spec = default_spec or QuotaSpec()
        self._clock: Clock = clock or SystemClock()
        # Per-(tenant, api_key) overrides loaded from DB at startup; kept
        # in-process for p99 < 2ms.
        self._overrides: dict[tuple[str, str], QuotaSpec] = {}

    def set_override(
        self, *, tenant: str, api_key: str, spec: QuotaSpec,
    ) -> None:
        self._overrides[(tenant, api_key)] = spec

    async def check(
        self, *, tenant: str, api_key: str, cost: float | None = None,
    ) -> bool:
        spec = self._overrides.get((tenant, api_key), self._spec)
        # Millisecond epoch — start at a safe non-zero value so Redis
        # EXPIRE doesn't see a 0 TTL on first access.
        now_ms = max(1, int(self._clock.now_monotonic_s() * 1000 + 1))
        ok = await self._lua.quota_check(
            quota_key=f"quota:{tenant}:{api_key}",
            now_ms=now_ms,
            capacity=spec.capacity,
            refill_per_s=spec.refill_per_s,
            cost=cost if cost is not None else spec.cost,
        )
        return ok

    async def enforce(
        self, *, tenant: str, api_key: str, cost: float | None = None,
    ) -> None:
        """Raise :class:`QuotaExceeded` if the bucket refuses."""
        if not await self.check(tenant=tenant, api_key=api_key, cost=cost):
            raise QuotaExceeded(
                f"quota exhausted for ({tenant!r}, {api_key!r})",
            )


__all__ = ["QuotaEnforcer", "QuotaSpec"]
