"""Alignment cone domain model + Admin-service client.

Cones are the **mandatory policy surface** of every swarm run: the
arbiter refuses to start a swarm whose problem embedding lies outside
the cone's anchor set by more than ``max_projection_distance``. This
enforces tenant-scoped semantic restrictions.

The module provides:

* :class:`AlignmentCone` — pure-data class mirroring the proto shape.
* :class:`ConeReference` — thin wrapper over ``tenant/task@vN``.
* :class:`ConeRegistryClient` — typed async client for the six RPCs
  exposed by ``efe.v1.admin.AdminService`` (``RegisterCone``,
  ``EvolveCone``, ``GetCone``, ``ListVersions``, ``DeleteCone``,
  ``BreachHistory``).

The registry client is intentionally separate from
:class:`SwarmClient`: data plane (``Execute``) and control plane
(``Admin*``) have different traffic patterns, deadlines, and
authorization scopes.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_platform.errors import SwarmUnavailable
from efe_platform.swarm.protocol import AnchorRef, ConeRef

# ---------------------------------------------------------------------------
#  Value objects
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class AlignmentCone:
    """Full in-memory representation of a cone."""

    anchors: tuple[AnchorRef, ...]
    max_projection_distance: float = 1.0

    def __post_init__(self) -> None:
        if not self.anchors:
            raise ValueError("cone must contain at least one anchor")
        dims = {a.dim() for a in self.anchors}
        if len(dims) != 1:
            raise ValueError(f"anchor dimensions must all match, got {dims}")
        if self.max_projection_distance <= 0:
            raise ValueError("max_projection_distance must be > 0")

    @property
    def dim(self) -> int:
        return self.anchors[0].dim()

    @property
    def anchor_count(self) -> int:
        return len(self.anchors)


@dataclass(frozen=True, slots=True)
class ConeMetadata:
    """Metadata mirrors ``efe.v1.admin.ConeMetadata``."""

    created_at: datetime
    created_by: str
    description: str = ""
    parent_key: str = ""
    evolution_op: str = ""
    labels: dict[str, str] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ConeReference:
    """Stable handle for an already-registered cone."""

    key: str
    cone: AlignmentCone
    metadata: ConeMetadata

    def as_ref(self) -> ConeRef:
        """Produce the wire-side canonical reference (tenant/task@vN)."""
        tenant, rest = self.key.split("/", 1)
        task, version_part = rest.split("@v", 1)
        return ConeRef(tenant=tenant, task=task, version=int(version_part))


# ---------------------------------------------------------------------------
#  In-memory transport abstraction
# ---------------------------------------------------------------------------


class AdminTransport:
    """Narrow Protocol-like interface for the admin RPCs we consume.

    Keeping this as a plain class (not a ``typing.Protocol``) makes it
    trivial to substitute with an in-memory fake in tests, including the
    one at :mod:`tests.conftest`. Real implementations wrap the
    generated gRPC stub.
    """

    async def register_cone(
        self,
        *,
        tenant: str,
        task: str,
        version: int,
        cone: AlignmentCone,
        created_by: str,
        description: str,
        labels: dict[str, str],
    ) -> ConeReference:  # pragma: no cover — abstract
        raise NotImplementedError

    async def get_cone(self, key: str) -> ConeReference:  # pragma: no cover
        raise NotImplementedError

    async def list_versions(
        self, *, tenant: str, task: str
    ) -> tuple[str, ...]:  # pragma: no cover
        raise NotImplementedError

    async def delete_cone(
        self, key: str, *, force: bool
    ) -> None:  # pragma: no cover
        raise NotImplementedError

    def breach_history(
        self,
        *,
        limit: int,
        tenant: str | None,
        task: str | None,
        min_magnitude: float,
    ) -> AsyncIterator[dict[str, Any]]:  # pragma: no cover
        raise NotImplementedError


# ---------------------------------------------------------------------------
#  In-memory registry — reference implementation for tests + local dev
# ---------------------------------------------------------------------------


class InMemoryAdminTransport(AdminTransport):
    """Thread-unsafe but coroutine-safe reference registry.

    Used for:

    * local development (run the platform without the Rust server),
    * unit tests (cone CRUD, evolution).
    """

    def __init__(self) -> None:
        self._cones: dict[str, ConeReference] = {}
        self._breaches: list[dict[str, Any]] = []

    async def register_cone(
        self,
        *,
        tenant: str,
        task: str,
        version: int,
        cone: AlignmentCone,
        created_by: str,
        description: str,
        labels: dict[str, str],
    ) -> ConeReference:
        key = f"{tenant}/{task}@v{version}"
        if key in self._cones:
            raise SwarmUnavailable(f"cone {key} already registered")
        meta = ConeMetadata(
            created_at=datetime.now(timezone.utc),
            created_by=created_by,
            description=description,
            labels=dict(labels),
        )
        ref = ConeReference(key=key, cone=cone, metadata=meta)
        self._cones[key] = ref
        return ref

    async def get_cone(self, key: str) -> ConeReference:
        if key not in self._cones:
            raise SwarmUnavailable(f"cone {key} not found")
        return self._cones[key]

    async def list_versions(self, *, tenant: str, task: str) -> tuple[str, ...]:
        prefix = f"{tenant}/{task}@v"
        keys = [k for k in self._cones if k.startswith(prefix)]
        return tuple(sorted(keys))

    async def delete_cone(self, key: str, *, force: bool) -> None:
        if key not in self._cones:
            if force:
                return
            raise SwarmUnavailable(f"cone {key} not found")
        del self._cones[key]

    async def breach_history(
        self,
        *,
        limit: int,
        tenant: str | None,
        task: str | None,
        min_magnitude: float,
    ) -> AsyncIterator[dict[str, Any]]:
        def _ok(b: dict[str, Any]) -> bool:
            if tenant and not b["cone_key"].startswith(f"{tenant}/"):
                return False
            if task and f"/{task}@" not in b["cone_key"]:
                return False
            return bool(b["observed_delta"] >= min_magnitude)

        filtered = [b for b in self._breaches if _ok(b)]
        for item in filtered[: max(1, limit)]:
            yield item

    # ------------------------------------------------------------ test hook

    def record_breach(self, entry: dict[str, Any]) -> None:
        """Test-only hook to seed the breach history."""
        self._breaches.append(entry)


# ---------------------------------------------------------------------------
#  High-level client
# ---------------------------------------------------------------------------


class ConeRegistryClient:
    """High-level façade over an :class:`AdminTransport`.

    Adds:

    * input sanitation (canonical key formatting),
    * caching: recently fetched cones are cached with a short TTL to
      avoid hammering the control plane on repeated ``GetCone`` during
      bot planning.
    """

    def __init__(self, transport: AdminTransport, *, cache_ttl_s: float = 30.0) -> None:
        self._transport = transport
        self._cache: dict[str, tuple[float, ConeReference]] = {}
        self._cache_ttl_s = cache_ttl_s

    # ------------------------------------------------------ Public methods

    async def register(
        self,
        *,
        tenant: str,
        task: str,
        version: int,
        cone: AlignmentCone,
        created_by: str = "platform",
        description: str = "",
        labels: dict[str, str] | None = None,
    ) -> ConeReference:
        ref = await self._transport.register_cone(
            tenant=tenant,
            task=task,
            version=version,
            cone=cone,
            created_by=created_by,
            description=description,
            labels=labels or {},
        )
        self._cache[ref.key] = (self._now(), ref)
        return ref

    async def get(self, key: str, *, use_cache: bool = True) -> ConeReference:
        if use_cache and key in self._cache:
            when, cached = self._cache[key]
            if self._now() - when < self._cache_ttl_s:
                return cached
        ref = await self._transport.get_cone(key)
        self._cache[key] = (self._now(), ref)
        return ref

    async def list_versions(self, tenant: str, task: str) -> tuple[str, ...]:
        return await self._transport.list_versions(tenant=tenant, task=task)

    async def delete(self, key: str, *, force: bool = False) -> None:
        await self._transport.delete_cone(key, force=force)
        self._cache.pop(key, None)

    async def breach_history(
        self,
        *,
        limit: int = 100,
        tenant: str | None = None,
        task: str | None = None,
        min_magnitude: float = 0.0,
    ) -> list[dict[str, Any]]:
        out = [
            b
            async for b in self._transport.breach_history(
                limit=limit, tenant=tenant, task=task, min_magnitude=min_magnitude
            )
        ]
        return out

    # ------------------------------------------------------- Helpers

    def invalidate(self, key: str) -> None:
        self._cache.pop(key, None)

    @staticmethod
    def _now() -> float:
        from time import monotonic

        return monotonic()


__all__ = [
    "AdminTransport",
    "AlignmentCone",
    "ConeMetadata",
    "ConeReference",
    "ConeRegistryClient",
    "InMemoryAdminTransport",
]
