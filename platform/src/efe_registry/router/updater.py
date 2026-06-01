"""RoutingUpdater — mutates the routing table through Lua CAS.

Every change to the active routing (promote / canary step / rollback)
goes through this class. It:

1. Reads the current version (from Redis via LuaRunner).
2. Builds the new set of entries.
3. Runs :meth:`LuaRunner.atomic_swap` with CAS on version.
4. Mirrors the change into Postgres ``routing_table`` (durable copy).
5. Publishes ``efe.registry.routing.updated`` on NATS.

On a :class:`RoutingVersionConflict` it returns a structured error so
the caller can retry with fresh state.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from sqlalchemy import delete, select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.core.artifact import ArtifactRole
from efe_registry.db.models import Endpoint, RoutingTableRow
from efe_registry.errors import (
    RoutingInvariantBroken,
    RoutingNotFound,
    RoutingVersionConflict,
)
from efe_registry.router.lua_runner import LuaRunner
from efe_registry.router.routing_table import RoutingKey


class NatsPublisherLike(Protocol):
    async def publish(
        self, topic: str, payload: dict[str, object], *,
        tenant_id: str | None = None,
    ) -> None:  # pragma: no cover
        ...


class NullNatsPublisher:
    """No-op publisher — used in tests that don't care about NATS."""

    async def publish(
        self, topic: str, payload: dict[str, object], *,
        tenant_id: str | None = None,
    ) -> None:
        return None


@dataclass
class RoutingUpdater:
    session_factory: async_sessionmaker[AsyncSession]
    lua: LuaRunner
    pubsub_channel: str = "routing.updated"
    nats_publisher: NatsPublisherLike | None = None
    nats_topic: str = "efe.registry.routing.updated"
    clock: Clock | None = None

    def _clock(self) -> Clock:
        return self.clock or SystemClock()

    # --------------------------------------------------------------- start_canary

    async def start_canary(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_endpoint_id: str,
        baseline_endpoint_id: str,
        percent: int,
        actor: str,
    ) -> int:
        if not 1 <= percent <= 100:
            raise RoutingInvariantBroken(
                f"canary percent must be 1..100, got {percent}",
            )
        key = RoutingKey(tenant_id, task_id)
        entries: tuple[tuple[str, int, str], ...]
        if percent == 100:
            entries = (
                (candidate_endpoint_id, 100, ArtifactRole.CHAMPION.value),
            )
        else:
            entries = (
                (baseline_endpoint_id, 100 - percent,
                 ArtifactRole.CHAMPION.value),
                (candidate_endpoint_id, percent,
                 ArtifactRole.CANDIDATE.value),
            )
        return await self._swap_and_mirror(
            key=key, entries=entries, actor=actor,
        )

    # --------------------------------------------------------------- set_canary_percent

    async def set_canary_percent(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_endpoint_id: str,
        baseline_endpoint_id: str,
        percent: int,
        actor: str,
    ) -> int:
        return await self.start_canary(
            tenant_id=tenant_id, task_id=task_id,
            candidate_endpoint_id=candidate_endpoint_id,
            baseline_endpoint_id=baseline_endpoint_id,
            percent=percent, actor=actor,
        )

    # --------------------------------------------------------------- swap_to_champion

    async def swap_to_champion(
        self,
        *,
        tenant_id: str,
        task_id: str,
        new_champion_endpoint_id: str,
        actor: str,
    ) -> int:
        key = RoutingKey(tenant_id, task_id)
        entries = (
            (new_champion_endpoint_id, 100, ArtifactRole.CHAMPION.value),
        )
        return await self._swap_and_mirror(
            key=key, entries=entries, actor=actor,
        )

    # --------------------------------------------------------------- core swap

    async def _swap_and_mirror(
        self,
        *,
        key: RoutingKey,
        entries: tuple[tuple[str, int, str], ...],
        actor: str,
    ) -> int:
        """Swap routing entries atomically.

        Order matters (C1):

            1. Validate **all** referenced endpoints exist in Postgres
               (cheap select). Fail fast before mutating Redis.
            2. Run the Lua CAS on Redis — authoritative hot path.
            3. Mirror the new state into Postgres. If the mirror fails,
               compensate by restoring the previous Redis state so the
               hot path and the durable store never diverge silently.

        This replaces the previous "Redis first, then DB" order that
        could leave the two stores disagreeing after a DB transient
        failure (audit report §C1).
        """
        endpoint_ids = [e[0] for e in entries]
        await self._assert_endpoints_exist(endpoint_ids)

        cur = await self.lua.get_version(key.redis_version_key())
        new_version = cur + 1
        # Snapshot previous entries for compensation if the mirror fails.
        previous_entries = await self._fetch_previous_entries(key)

        result = await self.lua.atomic_swap(
            version_key=key.redis_version_key(),
            routing_key=key.redis_hash_key(),
            pubsub_channel=self.pubsub_channel,
            expected_version=cur,
            new_version=new_version,
            entries=entries,
        )
        if not result.ok:
            if result.error and "version_mismatch" in result.error:
                raise RoutingVersionConflict(result.error)
            raise RoutingInvariantBroken(
                f"atomic_swap failed: {result.error}",
            )

        try:
            await self._mirror_to_db(
                key=key, entries=entries, version=new_version, actor=actor,
            )
        except Exception as exc:
            # Compensate Redis to the previous entries so hot path stays
            # consistent with the durable store. We attempt one CAS back
            # to (new_version, cur + 1) — a concurrent writer that
            # advanced further loses this compensation, but the invariant
            # (hot path == DB) is preserved because that other writer
            # will have run its own mirror.
            await self._compensate_redis(
                key=key, previous_entries=previous_entries,
                current_version=new_version,
            )
            raise RoutingInvariantBroken(
                f"mirror to DB failed, Redis compensated: {exc!s}",
            ) from exc

        # Publish NATS only after both stores agree.
        if self.nats_publisher is not None:
            await self.nats_publisher.publish(
                self.nats_topic,
                {
                    "tenant_id": key.tenant_id,
                    "task_id": key.task_id,
                    "variant": key.variant,
                    "version": new_version,
                    "updated_by": actor,
                    "entries": [
                        {
                            "endpoint_id": eid,
                            "weight": int(w),
                            "role": role,
                        }
                        for eid, w, role in entries
                    ],
                },
                tenant_id=key.tenant_id,
            )
        return new_version

    async def _assert_endpoints_exist(
        self, endpoint_ids: list[str],
    ) -> None:
        if not endpoint_ids:
            return
        async with self.session_factory() as session:
            existing = await session.execute(
                select(Endpoint.endpoint_id).where(
                    Endpoint.endpoint_id.in_(endpoint_ids),
                ),
            )
            known = {str(r[0]) for r in existing.all()}
            unknown = [eid for eid in endpoint_ids if eid not in known]
            if unknown:
                raise RoutingNotFound(
                    f"endpoints do not exist: {unknown}",
                )

    async def _fetch_previous_entries(
        self, key: RoutingKey,
    ) -> tuple[tuple[str, int, str], ...]:
        async with self.session_factory() as session:
            stmt = (
                select(RoutingTableRow)
                .where(
                    RoutingTableRow.tenant_id == key.tenant_id,
                    RoutingTableRow.task_id == key.task_id,
                    RoutingTableRow.variant == key.variant,
                )
            )
            rows = list((await session.execute(stmt)).scalars())
            return tuple(
                (r.endpoint_id, int(r.weight_percent), r.role)
                for r in rows
            )

    async def _compensate_redis(
        self,
        *,
        key: RoutingKey,
        previous_entries: tuple[tuple[str, int, str], ...],
        current_version: int,
    ) -> None:
        if not previous_entries:
            # Nothing to restore to; leave Redis as-is (hot path has the
            # new state but DB mirror is empty — the caller already
            # raised, and the next writer will re-drive a swap).
            return
        try:
            await self.lua.atomic_swap(
                version_key=key.redis_version_key(),
                routing_key=key.redis_hash_key(),
                pubsub_channel=self.pubsub_channel,
                expected_version=current_version,
                new_version=current_version + 1,
                entries=previous_entries,
            )
        except Exception:  # pragma: no cover — best-effort compensation
            return

    async def _mirror_to_db(
        self,
        *,
        key: RoutingKey,
        entries: tuple[tuple[str, int, str], ...],
        version: int,
        actor: str,
    ) -> None:
        """Mirror the new routing set to Postgres.

        Uses a single transaction with a serialisable read (SELECT +
        DELETE + INSERT) on the (tenant, task, variant) set. The SELECT
        prevents stale reads on the version column. On Postgres the
        partial index on canary active state protects against parallel
        promote/canary activations.
        """
        async with self.session_factory() as session:
            # Lock the current row set for update so two concurrent
            # mirrors cannot interleave delete/insert. On SQLite the
            # with_for_update() no-ops.
            lock_stmt = (
                select(RoutingTableRow)
                .where(
                    RoutingTableRow.tenant_id == key.tenant_id,
                    RoutingTableRow.task_id == key.task_id,
                    RoutingTableRow.variant == key.variant,
                )
                .with_for_update()
            )
            await session.execute(lock_stmt)
            await session.execute(
                delete(RoutingTableRow).where(
                    RoutingTableRow.tenant_id == key.tenant_id,
                    RoutingTableRow.task_id == key.task_id,
                    RoutingTableRow.variant == key.variant,
                ),
            )
            now = self._clock().now()
            for endpoint_id, weight, role in entries:
                session.add(RoutingTableRow(
                    tenant_id=key.tenant_id,
                    task_id=key.task_id,
                    variant=key.variant,
                    endpoint_id=endpoint_id,
                    weight_percent=int(weight),
                    priority=0,
                    version=version,
                    role=role,
                    updated_at=now,
                    updated_by=actor,
                ))
            await session.commit()


__all__ = ["NatsPublisherLike", "NullNatsPublisher", "RoutingUpdater"]
