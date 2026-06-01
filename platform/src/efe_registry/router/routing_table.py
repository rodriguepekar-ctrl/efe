"""RoutingTable — bridge between Postgres durable copy and Redis hot path.

The router primarily reads from the in-process cache (populated from
Redis on demand, invalidated by NATS PubSub). The DB is used to seed
Redis on boot and to provide the durable ground truth.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_registry.core.artifact import ArtifactRole
from efe_registry.db.models import RoutingTableRow


@dataclass(frozen=True, slots=True)
class RoutingKey:
    tenant_id: str
    task_id: str
    variant: str = "default"

    def redis_hash_key(self) -> str:
        return f"routing:{self.tenant_id}:{self.task_id}"

    def redis_version_key(self) -> str:
        return f"routing:{self.tenant_id}:{self.task_id}:v"


@dataclass(frozen=True, slots=True)
class RoutingEntry:
    endpoint_id: str
    weight_percent: int
    role: ArtifactRole


class RoutingTableBackend(Protocol):
    async def get_entries(
        self, key: RoutingKey,
    ) -> tuple[RoutingEntry, ...]:  # pragma: no cover
        ...

    async def get_version(
        self, key: RoutingKey,
    ) -> int:  # pragma: no cover
        ...


class PostgresRoutingTable:
    """Durable copy — used on boot + as fallback when Redis flaps."""

    def __init__(self, session: AsyncSession) -> None:
        self._s = session

    async def get_entries(self, key: RoutingKey) -> tuple[RoutingEntry, ...]:
        stmt = (
            select(RoutingTableRow)
            .where(
                RoutingTableRow.tenant_id == key.tenant_id,
                RoutingTableRow.task_id == key.task_id,
                RoutingTableRow.variant == key.variant,
            )
        )
        rows = list((await self._s.execute(stmt)).scalars())
        return tuple(
            RoutingEntry(
                endpoint_id=r.endpoint_id,
                weight_percent=r.weight_percent,
                role=ArtifactRole(r.role),
            )
            for r in rows
        )

    async def get_version(self, key: RoutingKey) -> int:
        stmt = (
            select(RoutingTableRow.version)
            .where(
                RoutingTableRow.tenant_id == key.tenant_id,
                RoutingTableRow.task_id == key.task_id,
                RoutingTableRow.variant == key.variant,
            )
            .limit(1)
        )
        v = (await self._s.execute(stmt)).scalar_one_or_none()
        return int(v or 0)


@dataclass
class RoutingTable:
    """Composition — prefer Redis, fallback on durable DB."""

    redis_backend: RoutingTableBackend
    db_backend: RoutingTableBackend | None = None

    async def get_entries(self, key: RoutingKey) -> tuple[RoutingEntry, ...]:
        entries = await self.redis_backend.get_entries(key)
        if entries:
            return entries
        if self.db_backend is not None:
            return await self.db_backend.get_entries(key)
        return ()

    async def get_version(self, key: RoutingKey) -> int:
        return await self.redis_backend.get_version(key)


__all__ = [
    "PostgresRoutingTable",
    "RoutingEntry",
    "RoutingKey",
    "RoutingTable",
    "RoutingTableBackend",
]
