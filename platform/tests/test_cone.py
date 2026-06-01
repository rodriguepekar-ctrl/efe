"""Tests for :class:`ConeRegistryClient` + :class:`InMemoryAdminTransport`."""

from __future__ import annotations

import asyncio

import pytest

from efe_platform.errors import SwarmUnavailable
from efe_platform.swarm.cone import AlignmentCone, ConeRegistryClient, InMemoryAdminTransport
from efe_platform.swarm.protocol import AnchorRef


def _cone(dim: int = 4, n: int = 2) -> AlignmentCone:
    return AlignmentCone(
        anchors=tuple(
            AnchorRef(tuple(float(i + j * 0.1) for j in range(dim))) for i in range(n)
        ),
        max_projection_distance=1.0,
    )


@pytest.fixture
def registry() -> ConeRegistryClient:
    return ConeRegistryClient(InMemoryAdminTransport(), cache_ttl_s=0.01)


@pytest.mark.asyncio
async def test_register_and_get(registry: ConeRegistryClient) -> None:
    ref = await registry.register(
        tenant="acme",
        task="routing",
        version=1,
        cone=_cone(),
        created_by="pytest",
        description="demo",
    )
    assert ref.key == "acme/routing@v1"
    fetched = await registry.get("acme/routing@v1")
    assert fetched.cone.anchor_count == 2


@pytest.mark.asyncio
async def test_duplicate_register_fails(registry: ConeRegistryClient) -> None:
    await registry.register(tenant="a", task="t", version=1, cone=_cone())
    with pytest.raises(SwarmUnavailable, match="already registered"):
        await registry.register(tenant="a", task="t", version=1, cone=_cone())


@pytest.mark.asyncio
async def test_list_versions_sorted(registry: ConeRegistryClient) -> None:
    for v in (3, 1, 2):
        await registry.register(tenant="t", task="k", version=v, cone=_cone())
    keys = await registry.list_versions("t", "k")
    assert keys == ("t/k@v1", "t/k@v2", "t/k@v3")


@pytest.mark.asyncio
async def test_delete(registry: ConeRegistryClient) -> None:
    await registry.register(tenant="t", task="k", version=1, cone=_cone())
    await registry.delete("t/k@v1")
    with pytest.raises(SwarmUnavailable, match="not found"):
        await registry.get("t/k@v1", use_cache=False)


@pytest.mark.asyncio
async def test_delete_force_missing_is_noop(registry: ConeRegistryClient) -> None:
    await registry.delete("ghost/g@v9", force=True)  # must not raise


@pytest.mark.asyncio
async def test_cache_ttl(registry: ConeRegistryClient) -> None:
    await registry.register(tenant="t", task="k", version=1, cone=_cone())
    ref1 = await registry.get("t/k@v1")
    # Delete behind the cache, then ensure a fresh fetch misses.
    await registry.delete("t/k@v1")
    # cache TTL is 0.01s
    await asyncio.sleep(0.05)
    with pytest.raises(SwarmUnavailable):
        await registry.get("t/k@v1", use_cache=True)


def test_cone_validates_anchor_dims() -> None:
    with pytest.raises(ValueError, match="must all match"):
        AlignmentCone(
            anchors=(AnchorRef((0.0, 0.0)), AnchorRef((0.0, 0.0, 0.0))),
        )


def test_cone_reference_round_trip() -> None:
    from datetime import datetime, timezone

    from efe_platform.swarm.cone import ConeMetadata, ConeReference

    ref = ConeReference(
        key="acme/t@v5",
        cone=_cone(),
        metadata=ConeMetadata(
            created_at=datetime.now(timezone.utc), created_by="test"
        ),
    )
    wire = ref.as_ref()
    assert (wire.tenant, wire.task, wire.version) == ("acme", "t", 5)
