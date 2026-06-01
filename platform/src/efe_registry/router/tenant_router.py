"""TenantRouter — resolves a request to a concrete :class:`Endpoint`.

Hot-path chain:

    1. QuotaEnforcer.enforce(tenant, api_key)
    2. RoutingCache.get(RoutingKey)
    3. Dispatcher.pick(entries)
    4. return RouteDecision(endpoint_id, …)

All four steps are O(1) in expectation; the only possible stall is a
cache miss which fans out to Redis (Lua GET) for the authoritative
version + entries. The cache is populated by a loader function
provided at construction time.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from efe_registry.core.artifact import ArtifactRole
from efe_registry.errors import RoutingNotFound
from efe_registry.router.cache import Loader, RoutingCache
from efe_registry.router.dispatcher import Dispatcher, StratifiedDispatcher
from efe_registry.router.quota import QuotaEnforcer
from efe_registry.router.routing_table import RoutingEntry, RoutingKey


class EndpointResolverLike(Protocol):
    async def resolve(
        self, endpoint_id: str,
    ) -> EndpointView:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class EndpointView:
    endpoint_id: str
    url: str
    artifact_id: str | None
    role: ArtifactRole
    state: str


@dataclass(frozen=True, slots=True)
class RouteDecision:
    key: RoutingKey
    endpoint_id: str
    endpoint_url: str
    role: ArtifactRole
    artifact_id: str | None
    canary_percent: int


class TenantRouter:
    def __init__(
        self,
        *,
        cache: RoutingCache,
        dispatcher: Dispatcher | None = None,
        quota: QuotaEnforcer | None = None,
        endpoint_resolver: EndpointResolverLike | None = None,
    ) -> None:
        self._cache = cache
        self._dispatcher = dispatcher or StratifiedDispatcher()
        self._quota = quota
        self._resolver = endpoint_resolver

    @classmethod
    def from_loader(
        cls,
        *,
        loader: Loader,
        ttl_s: float = 1.0,
        dispatcher: Dispatcher | None = None,
        quota: QuotaEnforcer | None = None,
        endpoint_resolver: EndpointResolverLike | None = None,
    ) -> TenantRouter:
        return cls(
            cache=RoutingCache(loader=loader, ttl_s=ttl_s),
            dispatcher=dispatcher,
            quota=quota,
            endpoint_resolver=endpoint_resolver,
        )

    async def route(
        self,
        *,
        tenant_id: str,
        task_id: str,
        api_key: str,
        variant: str = "default",
        cost: float | None = None,
    ) -> RouteDecision:
        if self._quota is not None:
            await self._quota.enforce(
                tenant=tenant_id, api_key=api_key, cost=cost,
            )
        key = RoutingKey(tenant_id, task_id, variant)
        entries = await self._cache.get(key)
        if not entries:
            raise RoutingNotFound(
                f"no routing entries for ({tenant_id!r}, "
                f"{task_id!r}, {variant!r})",
            )
        chosen = self._dispatcher.pick(entries)
        canary = _current_canary_percent(entries)

        # Resolve URL + artifact_id if resolver provided (tests may skip).
        if self._resolver is not None:
            view = await self._resolver.resolve(chosen.endpoint_id)
            url = view.url
            artifact_id = view.artifact_id
        else:
            url = f"http://{chosen.endpoint_id}:8000"
            artifact_id = None
        return RouteDecision(
            key=key,
            endpoint_id=chosen.endpoint_id,
            endpoint_url=url,
            role=chosen.role,
            artifact_id=artifact_id,
            canary_percent=canary,
        )

    def invalidate(self, tenant_key: str) -> int:
        return self._cache.invalidate(tenant_key)


def _current_canary_percent(
    entries: tuple[RoutingEntry, ...],
) -> int:
    for e in entries:
        if e.role == ArtifactRole.CANDIDATE:
            return e.weight_percent
    return 0


__all__ = [
    "EndpointResolverLike",
    "EndpointView",
    "RouteDecision",
    "TenantRouter",
]
