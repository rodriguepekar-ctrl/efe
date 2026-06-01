"""Tests for the FastAPI HTTP surface.

Skipped automatically when FastAPI is not installed (it's an optional
extra). ``httpx.AsyncClient`` drives the app in-process without starting
uvicorn.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import pytest

fastapi = pytest.importorskip("fastapi")
pytest.importorskip("httpx")

from efe_platform.api.dependencies import PlatformContext, clear_context, set_context
from efe_platform.api.http_app import build_app
from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.budget import Budget
from efe_platform.config import PlatformConfig
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import (
    AlignmentCone,
    ConeRegistryClient,
    InMemoryAdminTransport,
)
from efe_platform.swarm.protocol import AnchorRef
from efe_platform.telemetry.audit import AuditLog

# ---------------------------------------------------------------------------
#  Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def context(tmp_path: Path) -> AsyncIterator[PlatformContext]:
    transport = InMemorySwarmTransport()
    swarm = SwarmClient(SwarmEndpoint("in-mem"), transport=transport, max_retries=0)
    cones = ConeRegistryClient(InMemoryAdminTransport())
    await cones.register(
        tenant="acme",
        task="routing",
        version=1,
        cone=AlignmentCone(
            anchors=(AnchorRef(tuple(0.1 * i for i in range(16))),),
            max_projection_distance=2.0,
        ),
    )
    bot = Bot(
        BotConfig(budget=Budget(max_wall_clock_ms=2_000)),
        swarm=swarm,
        cones=cones,
    )
    audit = AuditLog(tmp_path / "audit.jsonl", fsync_each=False)
    ctx = PlatformContext(
        config=PlatformConfig(),
        bot=bot,
        swarm=swarm,
        cones=cones,
        audit=audit,
    )
    set_context(ctx)
    try:
        yield ctx
    finally:
        audit.close()
        await swarm.close()
        clear_context()


@pytest.fixture
async def client(context: PlatformContext) -> AsyncIterator:
    import httpx

    app = build_app(context)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as ac:
        yield ac


# ---------------------------------------------------------------------------
#  Routes
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_healthz(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/healthz")
    assert r.status_code == 200
    body = r.json()
    assert body["status"] == "ok"
    assert "version" in body


@pytest.mark.asyncio
async def test_metrics(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/metrics")
    assert r.status_code == 200
    assert "efe_platform_requests_total" in r.text
    assert r.headers["content-type"].startswith("text/plain")


@pytest.mark.asyncio
async def test_answer_conversational(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.post("/v1/answer", json={"prompt": "Bonjour"})
    assert r.status_code == 200
    body = r.json()
    assert body["ok"] is True
    assert body["recap"]["verdict"] == "conversational"
    assert body["final_text"]


@pytest.mark.asyncio
async def test_answer_with_plan(client) -> None:  # type: ignore[no-untyped-def]
    prompt = (
        "plan: swarm.classify "
        "tenant=acme task=routing cone_version=1 "
        "problem_embedding=[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,"
        "0.9,1.0,0.1,0.2,0.3,0.4,0.5,0.6] "
        "dim_latent=16"
    )
    r = await client.post("/v1/answer", json={"prompt": prompt, "tenant": "acme"})
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["ok"] is True
    assert body["ledger"]["tool_calls_made"] == 1


@pytest.mark.asyncio
async def test_answer_empty_prompt_rejected(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.post("/v1/answer", json={"prompt": ""})
    # pydantic min_length=1 → 422
    assert r.status_code == 422


@pytest.mark.asyncio
async def test_catalog_route(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/v1/catalog")
    assert r.status_code == 200
    body = r.json()
    assert "tasks" in body
    assert body["version"] == 0  # no catalog loaded in fixture


@pytest.mark.asyncio
async def test_catalog_cones_route(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/v1/catalog/cones?tenant=acme&task=routing")
    assert r.status_code == 200
    body = r.json()
    assert body["tenant"] == "acme"
    assert body["task"] == "routing"
    assert body["versions"] == ["acme/routing@v1"]


@pytest.mark.asyncio
async def test_catalog_cones_empty_tenant(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/v1/catalog/cones?tenant=ghost&task=ghost")
    assert r.status_code == 200
    assert r.json()["versions"] == []


@pytest.mark.asyncio
async def test_answer_budget_ledger_exposed(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.post("/v1/answer", json={"prompt": "Bonjour"})
    body = r.json()
    assert "elapsed_ms" in body["ledger"]
    assert "tokens_consumed" in body["ledger"]


@pytest.mark.asyncio
async def test_answer_custom_budget(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.post(
        "/v1/answer",
        json={
            "prompt": "Bonjour",
            "budget_ms": 1_000,
            "budget_tool_calls": 2,
            "budget_tokens": 500,
            "budget_usd": 0.01,
        },
    )
    assert r.status_code == 200


@pytest.mark.asyncio
async def test_stream_route(client) -> None:  # type: ignore[no-untyped-def]
    # httpx consumes the SSE stream as body text.
    r = await client.post(
        "/v1/answer/stream", json={"prompt": "Bonjour"}, timeout=5.0
    )
    assert r.status_code == 200
    assert "event: plan" in r.text
    assert "event: final" in r.text


@pytest.mark.asyncio
async def test_openapi_available(client) -> None:  # type: ignore[no-untyped-def]
    r = await client.get("/v1/openapi.json")
    assert r.status_code == 200
    spec = r.json()
    assert spec["info"]["title"] == "EFE Platform"
