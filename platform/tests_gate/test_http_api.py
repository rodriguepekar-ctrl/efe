"""HTTP API integration tests via httpx.ASGITransport."""

from __future__ import annotations

import hashlib
from typing import Any

import numpy as np
import pytest_asyncio
from httpx import ASGITransport, AsyncClient
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_gate.api import GateContext, build_app, set_context, clear_context
from efe_gate.clock import FakeClock
from efe_gate.config import GateSettings
from efe_gate.decision.rule import (
    MetricRuleSpec,
    PromotionRule,
    ReviewPolicy,
)
from efe_gate.runner import FakeScorer


@pytest_asyncio.fixture
async def gate_context(
    settings: GateSettings,
    sessionmaker: async_sessionmaker[AsyncSession],
    fake_scorer: FakeScorer,
    fake_clock: FakeClock,
) -> Any:
    ctx = GateContext(
        settings=settings, sessionmaker=sessionmaker,
        scorer=fake_scorer, clock=fake_clock,
    )
    set_context(ctx)
    try:
        yield ctx
    finally:
        clear_context()


@pytest_asyncio.fixture
async def client(gate_context: GateContext):  # type: ignore[no-untyped-def]
    app = build_app(gate_context)
    transport = ASGITransport(app=app)
    async with AsyncClient(
        transport=transport, base_url="http://testserver",
    ) as c:
        yield c


async def test_healthz(client: AsyncClient) -> None:
    r = await client.get("/healthz")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"


async def test_metrics_catalog(client: AsyncClient) -> None:
    r = await client.get("/v1/metrics/catalog")
    assert r.status_code == 200
    names = [m["name"] for m in r.json()["metrics"]]
    assert "accuracy" in names
    assert "p99_latency_ms" in names


async def test_create_and_get_golden(client: AsyncClient) -> None:
    r = await client.post("/v1/golden", json={
        "tenant_id": "acme", "task_id": "routing",
        "snapshot_uri": "file:///g",
        "content_hash": hashlib.sha256(b"x").hexdigest(),
        "n_samples": 100,
        "created_by": "curator",
    })
    assert r.status_code == 200
    gs_id = r.json()["id"]
    r2 = await client.get(f"/v1/golden/{gs_id}")
    assert r2.status_code == 200
    assert r2.json()["n_samples"] == 100


async def test_upsert_gate(client: AsyncClient) -> None:
    rule = PromotionRule(
        primary=(MetricRuleSpec("accuracy", min_equivalence_margin=0.01),),
        budget=(MetricRuleSpec("p99_latency_ms", hard_cap=100.0),),
        review_policy=ReviewPolicy(timeout_hours=1),
    )
    r = await client.put("/v1/gates/acme/routing", json={
        "rule": rule.as_dict(), "updated_by": "ops",
    })
    assert r.status_code == 200
    r2 = await client.get("/v1/gates/acme/routing")
    assert r2.status_code == 200
    assert r2.json()["rule"]["primary"][0]["metric"] == "accuracy"


async def test_submit_and_get_eval(
    client: AsyncClient,
    sessionmaker: async_sessionmaker[AsyncSession],
    fake_scorer: FakeScorer,
    fake_clock: FakeClock,
) -> None:
    # Pre-create a golden + gate directly via API.
    ch = hashlib.sha256(b"eval-golden").hexdigest()
    r = await client.post("/v1/golden", json={
        "tenant_id": "acme", "task_id": "routing",
        "snapshot_uri": "file:///g",
        "content_hash": ch, "n_samples": 200,
        "created_by": "curator",
    })
    gs_id = r.json()["id"]
    rule = PromotionRule(
        primary=(MetricRuleSpec(
            "accuracy", min_equivalence_margin=0.05,
        ),),
        budget=(),
        review_policy=ReviewPolicy(timeout_hours=1),
    )
    await client.put("/v1/gates/acme/routing", json={
        "rule": rule.as_dict(), "updated_by": "ops",
    })
    # Submit eval.
    fake_scorer.scores[("s3://cand", "accuracy")] = (
        np.random.default_rng(1).random(200) < 0.9
    ).astype(np.int64)
    r = await client.post("/v1/eval/submit", json={
        "tenant_id": "acme", "task_id": "routing",
        "task_type": "classif",
        "candidate_run_id": "rrrr-1111-1111-1111-111111111111",
        "candidate_artifact_uri": "s3://cand",
        "submitted_by": "m03",
        "golden_set_id": gs_id,
        "metric_names": ["accuracy"],
    })
    assert r.status_code == 200
    eid = r.json()["id"]
    # Process.
    r2 = await client.post(f"/v1/eval/{eid}:process")
    assert r2.status_code == 200
    # No champion yet → first promote.
    r3 = await client.get(f"/v1/eval/{eid}?tenant_id=acme")
    assert r3.json()["state"] == "promoted"


async def test_champion_and_error_mapping(client: AsyncClient) -> None:
    # No champion → 404 shape.
    r = await client.get("/v1/champions/nope/none")
    assert r.status_code == 404
    assert r.json()["code"] == "no_champion"
