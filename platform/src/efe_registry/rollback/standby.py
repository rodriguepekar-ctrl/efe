"""Pre-warmed standby management.

The standby pod is a serving instance whose ``role`` is
``standby_warm``. It serves 0 % of traffic but responds to
health-checks and to a synthetic ping every ``standby_ping_interval_s``
so its CUDA graph + KV cache stay hot.

This module provides:

* :class:`StandbyProbe` — protocol for checking liveness.
* :class:`InMemoryStandbyProbe` — test double.
* :class:`PreWarmedStandbyManager` — pick a standby for a rollback,
  spawn a replacement after use.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Protocol

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_registry.clock import Clock, SystemClock
from efe_registry.core.artifact import ArtifactRole
from efe_registry.db.models import Endpoint
from efe_registry.errors import NoStandbyAvailable


class StandbyProbe(Protocol):
    async def is_ready(
        self, endpoint: Endpoint,
    ) -> bool:  # pragma: no cover
        ...


@dataclass
class InMemoryStandbyProbe:
    ready_endpoints: set[str] = field(default_factory=set)

    async def is_ready(self, endpoint: Endpoint) -> bool:
        return endpoint.endpoint_id in self.ready_endpoints


@dataclass(frozen=True, slots=True)
class StandbyCheck:
    endpoint: Endpoint
    ready: bool
    heartbeat_age_s: float


class PreWarmedStandbyManager:
    """Picks a usable standby for a given artifact, checks liveness.

    The manager does **not** create pods — that is the K8s operator's
    job. We only inspect what's already in the ``endpoints`` table.
    """

    def __init__(
        self,
        *,
        probe: StandbyProbe,
        heartbeat_stale_s: float = 15.0,
        clock: Clock | None = None,
    ) -> None:
        self._probe = probe
        self._stale_s = heartbeat_stale_s
        self._clock: Clock = clock or SystemClock()

    async def pick_for_artifact(
        self, session: AsyncSession, *, artifact_id: str,
    ) -> StandbyCheck:
        stmt = (
            select(Endpoint)
            .where(
                Endpoint.artifact_id == artifact_id,
                Endpoint.role == ArtifactRole.STANDBY_WARM.value,
                Endpoint.state == "ready",
            )
            .limit(1)
        )
        endpoint = (
            await session.execute(stmt)
        ).scalar_one_or_none()
        if endpoint is None:
            raise NoStandbyAvailable(
                f"no standby_warm endpoint ready for artifact "
                f"{artifact_id!r}",
            )
        heartbeat_age = _heartbeat_age_s(
            endpoint.last_heartbeat_at, self._clock.now(),
        )
        if heartbeat_age > self._stale_s:
            raise NoStandbyAvailable(
                f"standby heartbeat stale ({heartbeat_age:.1f}s) "
                f"for artifact {artifact_id!r}",
            )
        ready = await self._probe.is_ready(endpoint)
        if not ready:
            raise NoStandbyAvailable(
                f"standby probe returned not-ready for "
                f"{endpoint.pod_name!r}",
            )
        return StandbyCheck(
            endpoint=endpoint,
            ready=True,
            heartbeat_age_s=heartbeat_age,
        )


def _heartbeat_age_s(hb: datetime | None, now: datetime) -> float:
    if hb is None:
        return float("inf")
    if hb.tzinfo is None:
        hb = hb.replace(tzinfo=timezone.utc)
    return (now - hb).total_seconds()


__all__ = [
    "InMemoryStandbyProbe",
    "PreWarmedStandbyManager",
    "StandbyCheck",
    "StandbyProbe",
]
