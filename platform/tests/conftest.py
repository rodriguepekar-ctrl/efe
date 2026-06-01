"""Shared pytest fixtures."""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterator
from pathlib import Path
from typing import Any

import pytest

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.budget import Budget
from efe_platform.bot.providers.mock import MockProvider
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import (
    AlignmentCone,
    ConeRegistryClient,
    InMemoryAdminTransport,
)
from efe_platform.swarm.protocol import AnchorRef
from efe_platform.telemetry.audit import AuditLog


@pytest.fixture
def tmp_audit_path(tmp_path: Path) -> Path:
    return tmp_path / "audit.jsonl"


@pytest.fixture
def audit_log(tmp_audit_path: Path) -> Iterator[AuditLog]:
    log = AuditLog(tmp_audit_path, fsync_each=False)
    yield log
    log.close()


@pytest.fixture
def swarm_transport() -> InMemorySwarmTransport:
    return InMemorySwarmTransport()


@pytest.fixture
async def swarm(
    swarm_transport: InMemorySwarmTransport,
) -> AsyncIterator[SwarmClient]:
    client = SwarmClient(
        SwarmEndpoint("in-memory"),
        transport=swarm_transport,
        max_retries=0,
    )
    async with client:
        yield client


@pytest.fixture
def admin_transport() -> InMemoryAdminTransport:
    return InMemoryAdminTransport()


@pytest.fixture
def cones(admin_transport: InMemoryAdminTransport) -> ConeRegistryClient:
    return ConeRegistryClient(admin_transport, cache_ttl_s=0.01)


@pytest.fixture
def example_cone() -> AlignmentCone:
    # 8-d anchors, 3 anchors, comfortable radius.
    return AlignmentCone(
        anchors=tuple(
            AnchorRef(tuple([float(j + i * 0.1) for j in range(8)])) for i in range(3)
        ),
        max_projection_distance=2.0,
    )


@pytest.fixture
def bot(swarm: SwarmClient, cones: ConeRegistryClient) -> Bot:
    return Bot(
        BotConfig(budget=Budget(max_wall_clock_ms=2_000)),
        swarm=swarm,
        cones=cones,
        provider=MockProvider(),
    )


@pytest.fixture
def deterministic_embedding() -> tuple[float, ...]:
    """16-d problem embedding used throughout tests."""
    return tuple(i * 0.1 for i in range(16))


@pytest.fixture
def catalog_yaml(tmp_path: Path) -> Path:
    p = tmp_path / "catalog.yaml"
    p.write_text(
        """\
version: 1
metadata:
  owner: test
  created_at: 2026-04-19
  tags: [mvp]
anchors:
  - id: a1
    values: [0.1, 0.2, 0.3, 0.4]
tasks:
  - id: routing
    description: Route tickets
    tenant: acme
    cone_key: "acme/routing@v1"
    dim_latent: 4
    anchors: [a1]
    tags: [prod]
""",
        encoding="utf-8",
    )
    return p


def assert_dict_subset(actual: dict[str, Any], expected: dict[str, Any]) -> None:
    """Like ``assert actual >= expected`` for dicts, with a clearer message."""
    for k, v in expected.items():
        assert k in actual, f"missing key {k!r} in {actual}"
        assert actual[k] == v, f"key {k!r}: {actual[k]!r} != {v!r}"
