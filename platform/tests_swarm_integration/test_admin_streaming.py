"""Admin client + streaming consumer."""

from __future__ import annotations

import asyncio

import pytest

from efe_swarm_integration.admin.client import AdminClient
from efe_swarm_integration.admin.cone_ops import (
    append_anchor_op,
    remove_anchor_op,
    update_radius_op,
)
from efe_swarm_integration.errors import StreamAborted
from efe_swarm_integration.streaming.server_stream import (
    StreamEvent,
    consume_stream,
)
from efe_swarm_integration.transport.rpc_driver import RpcResponse
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeSpec,
)

from tests_swarm_integration._fakes import FakeRpcDriver


def test_admin_register_cone() -> None:
    async def scenario() -> None:
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.admin.AdminService/RegisterCone": RpcResponse(
                    payload={
                        "key": "acme/triage@v1",
                        "cone": {
                            "cone": {
                                "anchors": [{"values": [0.0, 1.0]}],
                                "max_projection_distance": 0.3,
                            },
                            "metadata": {
                                "created_at": "2026-04-23T00:00:00+00:00",
                                "created_by": "alice",
                                "description": "",
                                "labels": {},
                            },
                        },
                    }
                )
            }
        )
        client = AdminClient(driver=driver)
        spec = ConeSpec(
            tenant="acme",
            task="triage",
            version=1,
            cone=AlignmentCone(
                anchors=(Anchor(values=(0.0, 1.0)),),
                max_projection_distance=0.3,
            ),
            created_by="alice",
        )
        handle = await client.register_cone(
            spec, tenant_id="acme", actor_id="alice"
        )
        assert handle.key == "acme/triage@v1"

    asyncio.run(scenario())


def test_admin_evolve_cone() -> None:
    async def scenario() -> None:
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.admin.AdminService/EvolveCone": RpcResponse(
                    payload={
                        "key": "acme/triage@v2",
                        "cone": {
                            "cone": {
                                "anchors": [{"values": [0.0, 1.0]}],
                                "max_projection_distance": 0.42,
                            },
                            "metadata": {
                                "created_at": "2026-04-23T00:00:00+00:00",
                                "created_by": "bob",
                                "description": "",
                                "labels": {},
                            },
                        },
                    }
                )
            }
        )
        client = AdminClient(driver=driver)
        h1 = await client.evolve_cone(
            from_key="acme/triage@v1",
            op=update_radius_op(0.42),
            tenant_id="acme",
            actor_id="bob",
        )
        assert h1.key == "acme/triage@v2"

    asyncio.run(scenario())


def test_admin_list_versions() -> None:
    async def scenario() -> None:
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.admin.AdminService/ListVersions": RpcResponse(
                    payload={"keys": ["acme/t@v1", "acme/t@v2"]}
                )
            }
        )
        client = AdminClient(driver=driver)
        keys = await client.list_versions(tenant="acme", task="t", actor_tenant="acme")
        assert keys == ["acme/t@v1", "acme/t@v2"]

    asyncio.run(scenario())


def test_admin_ops_builders() -> None:
    a = append_anchor_op(Anchor(values=(1.0,)))
    assert a.kind == "append_anchor"
    r = remove_anchor_op(0)
    assert r.kind == "remove_anchor_idx"


# ---------------------------------------------------------------------------
# Streaming
# ---------------------------------------------------------------------------


async def _async_iter_from_list(items: list[dict]):
    for it in items:
        yield it


def test_consume_stream_yields_final() -> None:
    async def scenario() -> None:
        chunks = [
            {"text_delta": "hel"},
            {"text_delta": "lo"},
            {
                "final": {
                    "swarm_id": "s",
                    "request_id": "r",
                    "output": {"text": "hello"},
                    "confidence": 1.0,
                    "verdict": 2,
                    "rounds_executed": 1,
                    "total_ms": 1.0,
                    "alignment_delta": 0.0,
                }
            },
        ]
        events: list[StreamEvent] = []
        async for ev in consume_stream(_async_iter_from_list(chunks)):
            events.append(ev)
        assert events[-1].kind == "final"
        assert sum(1 for e in events if e.kind == "text_delta") == 2

    asyncio.run(scenario())


def test_consume_stream_aborts_without_final() -> None:
    async def scenario() -> None:
        chunks = [{"text_delta": "partial"}]
        events: list[StreamEvent] = []
        with pytest.raises(StreamAborted):
            async for ev in consume_stream(_async_iter_from_list(chunks)):
                events.append(ev)

    asyncio.run(scenario())
