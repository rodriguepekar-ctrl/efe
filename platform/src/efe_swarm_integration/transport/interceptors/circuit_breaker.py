"""CircuitBreakerInterceptor — short-circuits calls when the CB is open."""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.flow_control.circuit_breaker import (
    CircuitBreakerRegistry,
)

__all__ = ["CircuitBreakerInterceptor"]


@dataclass(slots=True)
class CircuitBreakerInterceptor:
    registry: CircuitBreakerRegistry

    async def pre(self, tenant_id: str) -> None:
        cb = await self.registry.for_tenant(tenant_id)
        await cb.pre_call()

    async def post(self, tenant_id: str, *, ok: bool) -> None:
        cb = await self.registry.for_tenant(tenant_id)
        await cb.record(success=ok)
