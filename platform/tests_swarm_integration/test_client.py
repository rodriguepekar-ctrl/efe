"""SwarmClient end-to-end tests using FakeRpcDriver + FakeEmbedder."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone

import pytest

from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.catalog.store import InMemoryConeBindingStore
from efe_swarm_integration.client import ClassifyRequest, SwarmClient
from efe_swarm_integration.config import DEFAULTS
from efe_swarm_integration.errors import (
    CircuitOpen,
    ConeRefNotBound,
    UnauthorisedM12,
)
from efe_swarm_integration.flow_control.bulkhead import BulkheadRegistry
from efe_swarm_integration.flow_control.circuit_breaker import (
    CircuitBreakerRegistry,
)
from efe_swarm_integration.transport.rpc_driver import RpcResponse
from efe_swarm_integration.types import (
    ConeRef,
    PopulationEntry,
    PopulationMix,
    SwarmSpec,
    TemplateCode,
    VerdictEnum,
)

from tests_swarm_integration._fakes import FakeEmbedder, FakePdp, FakeRpcDriver


def _wire_response() -> dict:
    return {
        "swarm_id": "swarm_42",
        "request_id": "req_1",
        "output": {"text": "fraud"},
        "confidence": 0.88,
        "verdict": 2,  # CONVERGED
        "rounds_executed": 4,
        "total_ms": 12.3,
        "alignment_delta": 0.12,
    }


async def _make_client(
    *,
    unary_responses: dict | None = None,
    pdp_allow: bool = True,
) -> tuple[SwarmClient, FakeRpcDriver]:
    driver = FakeRpcDriver(
        unary_responses={
            "/efe.v1.SwarmService/Execute": RpcResponse(
                payload=unary_responses or _wire_response()
            ),
        }
    )
    store = InMemoryConeBindingStore()
    bridge = CatalogBridge(store=store)
    await bridge.register_binding(
        tenant_id="acme",
        feature_id="support_triage",
        task_id=None,
        cone_ref=ConeRef("acme", "triage", 1),
        embedder_family="st",
        embedder_version="2",
        anchor_count=4,
        labels=("fraud", "billing", "tech", "other"),
        now=datetime(2026, 4, 23, tzinfo=timezone.utc),
    )
    client = SwarmClient(
        driver=driver,
        config=DEFAULTS,
        embedder=FakeEmbedder(dim=4),
        catalog=bridge,
        bulkheads=BulkheadRegistry(
            capacity=DEFAULTS.max_inflight_per_tenant,
            queue_wait_ms=DEFAULTS.max_queue_wait_ms,
        ),
        breakers=CircuitBreakerRegistry(
            error_rate_threshold=DEFAULTS.circuit_breaker.error_rate_threshold,
            window_s=DEFAULTS.circuit_breaker.window_s,
            min_samples=DEFAULTS.circuit_breaker.min_samples,
            open_cooldown_s=DEFAULTS.circuit_breaker.open_cooldown_s,
        ),
        pdp=FakePdp(allow=pdp_allow),
    )
    return client, driver


def test_classify_happy_path() -> None:
    async def scenario() -> None:
        client, driver = await _make_client()
        req = ClassifyRequest(
            text="I was debited without consent.",
            catalog_key=("support_triage", None),
            tenant_id="acme",
            actor_id="sa-bot",
            budget_ms=500,
        )
        resp = await client.classify(req)
        assert resp.label == "fraud"
        assert resp.verdict == VerdictEnum.CONVERGED
        # Metadata carries tenant and actor.
        _, md = driver.call_log[0]
        kv = dict(md)
        assert kv["x-efe-tenant-id"] == "acme"
        assert kv["x-efe-actor-id"] == "sa-bot"

    asyncio.run(scenario())


def test_classify_denied_by_pdp() -> None:
    async def scenario() -> None:
        client, _ = await _make_client(pdp_allow=False)
        req = ClassifyRequest(
            text="x",
            catalog_key=("support_triage", None),
            tenant_id="acme",
            actor_id="intruder",
        )
        with pytest.raises(UnauthorisedM12):
            await client.classify(req)

    asyncio.run(scenario())


def test_classify_unknown_catalog() -> None:
    async def scenario() -> None:
        client, _ = await _make_client()
        req = ClassifyRequest(
            text="x",
            catalog_key=("missing", None),
            tenant_id="acme",
            actor_id="sa",
        )
        with pytest.raises(ConeRefNotBound):
            await client.classify(req)

    asyncio.run(scenario())


def test_classify_divergence_returns_uncertain() -> None:
    wire = {
        "swarm_id": "s",
        "request_id": "r",
        "output": {"text": ""},
        "confidence": 0.0,
        "verdict": 3,  # DIVERGENCE
        "rounds_executed": 10,
        "total_ms": 50.0,
        "alignment_delta": 0.9,
    }

    async def scenario() -> None:
        client, _ = await _make_client(unary_responses=wire)
        req = ClassifyRequest(
            text="text",
            catalog_key=("support_triage", None),
            tenant_id="acme",
            actor_id="sa",
        )
        resp = await client.classify(req)
        assert resp.is_uncertain
        assert "verdict:divergence" in resp.reasons

    asyncio.run(scenario())
