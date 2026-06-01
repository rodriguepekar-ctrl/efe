"""Request router — hot path of the serving tier."""

from __future__ import annotations

from efe_registry.router.cache import RoutingCache
from efe_registry.router.dispatcher import (
    StratifiedDispatcher,
    WeightedDispatcher,
)
from efe_registry.router.lua_runner import (
    InMemoryLuaRunner,
    LuaRunner,
    LuaSwapResult,
)
from efe_registry.router.quota import QuotaEnforcer
from efe_registry.router.routing_table import (
    RoutingEntry,
    RoutingKey,
    RoutingTable,
)
from efe_registry.router.scripts import ATOMIC_SWAP_LUA, QUOTA_BUCKET_LUA
from efe_registry.router.tenant_router import RouteDecision, TenantRouter
from efe_registry.router.updater import RoutingUpdater

__all__ = [
    "ATOMIC_SWAP_LUA",
    "QUOTA_BUCKET_LUA",
    "InMemoryLuaRunner",
    "LuaRunner",
    "LuaSwapResult",
    "QuotaEnforcer",
    "RouteDecision",
    "RoutingCache",
    "RoutingEntry",
    "RoutingKey",
    "RoutingTable",
    "RoutingUpdater",
    "StratifiedDispatcher",
    "TenantRouter",
    "WeightedDispatcher",
]
