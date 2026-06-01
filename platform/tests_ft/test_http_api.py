"""Integration tests for the FastAPI HTTP surface."""

from __future__ import annotations

from collections.abc import AsyncIterator

import pytest
import pytest_asyncio

fastapi = pytest.importorskip("fastapi")
httpx = pytest.importorskip("httpx")

from efe_ft.api.dependencies import FtContext
from efe_ft.api.http_app import build_app


@pytest_asyncio.fixture
async def http_client(ft_context: FtContext) -> AsyncIterator:
    app = build_app(ft_context)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(
        transport=transport, base_url="http://test",
    ) as ac:
        yield ac


async def test_healthz(http_client) -> None:  # type: ignore[no-untyped-def]
    r = await http_client.get("/healthz")
    assert r.status_code == 200
    body = r.json()
    assert body["status"] == "ok"
    assert "version" in body


async def test_ingest_single_sample(http_client) -> None:  # type: ignore[no-untyped-def]
    r = await http_client.post(
        "/v1/ft/ingest/acme/routing/sample",
        json={
            "tenant_id": "acme",
            "task_id": "routing",
            "text": "Je ne peux pas me connecter ce matin",
            "source": "batch",
            "actor_id": "alice@acme",
        },
    )
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["n_accepted"] == 1
    assert body["n_rejected"] == 0


async def test_ingest_batch_plus_list(http_client) -> None:  # type: ignore[no-untyped-def]
    batch = {
        "items": [
            {
                "tenant_id": "acme", "task_id": "routing",
                "text": f"Requete tres distincte {i}: " + " ".join(
                    chr(97 + (i * 7 + j) % 26) * (2 + j) for j in range(6)
                ),
                "source": "batch", "actor_id": "alice@acme",
                "label": "authentication" if i % 2 == 0 else "billing",
            }
            for i in range(5)
        ]
    }
    r = await http_client.post(
        "/v1/ft/ingest/acme/routing/batch", json=batch,
    )
    assert r.status_code == 200
    body = r.json()
    assert body["n_accepted"] == 5

    r = await http_client.get("/v1/ft/samples/acme/routing?page_size=10")
    assert r.status_code == 200
    body = r.json()
    assert len(body["items"]) == 5


async def test_ingest_pii_masked_in_persisted_text(http_client) -> None:  # type: ignore[no-untyped-def]
    r = await http_client.post(
        "/v1/ft/ingest/acme/routing/sample",
        json={
            "tenant_id": "acme", "task_id": "routing",
            "text": "Contactez alice@acme.fr d'urgence",
            "source": "batch", "actor_id": "alice@acme",
        },
    )
    assert r.status_code == 200
    r = await http_client.get("/v1/ft/samples/acme/routing")
    body = r.json()
    assert len(body["items"]) == 1
    assert "<EMAIL>" in body["items"][0]["text"]
    assert "alice@acme.fr" not in body["items"][0]["text"]


async def test_ingest_tenant_mismatch_rejected(http_client) -> None:  # type: ignore[no-untyped-def]
    r = await http_client.post(
        "/v1/ft/ingest/acme/routing/sample",
        json={
            "tenant_id": "OTHER", "task_id": "routing",
            "text": "hello", "source": "batch", "actor_id": "x",
        },
    )
    assert r.status_code == 400


async def test_snapshot_roundtrip(http_client) -> None:  # type: ignore[no-untyped-def]
    # Seed 12 labelled samples
    items = [
        {
            "tenant_id": "acme", "task_id": "routing",
            "text": f"Snap seed {i}: " + " ".join(
                chr(97 + (i * 11 + j) % 26) * (3 + j) for j in range(8)
            ),
            "source": "batch", "actor_id": "alice@acme",
            "label": "billing" if i % 2 == 0 else "refund",
        }
        for i in range(12)
    ]
    await http_client.post(
        "/v1/ft/ingest/acme/routing/batch", json={"items": items},
    )
    r = await http_client.post(
        "/v1/ft/snapshots/acme/routing",
        json={
            "actor_id": "alice",
            "status_filter": "labelled",
            "min_samples": 5,
            "train": 8, "val": 2, "holdout": 2, "seed": 42,
        },
    )
    assert r.status_code == 200, r.text
    snap = r.json()
    assert snap["n_samples"] == 12
    snap_id = snap["id"]

    r = await http_client.get(f"/v1/ft/snapshots/{snap_id}")
    assert r.status_code == 200


async def test_labeling_flow_end_to_end(http_client) -> None:  # type: ignore[no-untyped-def]
    items = [
        {
            "tenant_id": "acme", "task_id": "routing",
            "text": f"sample unique al {i}",
            "source": "batch", "actor_id": "alice",
        }
        for i in range(3)
    ]
    await http_client.post(
        "/v1/ft/ingest/acme/routing/batch", json={"items": items},
    )
    # Start a session
    r = await http_client.post(
        "/v1/ft/labeling/sessions",
        json={
            "annotator_id": "alice", "tenant_id": "acme",
            "task_id": "routing",
        },
    )
    assert r.status_code == 200
    session_id = r.json()["session_id"]

    # Seed the session batch by hand via the store
    from efe_ft.api.dependencies import get_context
    ctx = get_context()
    state = await ctx.session_store.get(session_id)
    # get existing sample ids
    r = await http_client.get("/v1/ft/samples/acme/routing")
    ids = [s["id"] for s in r.json()["items"]]
    await ctx.session_store.set(state.with_batch(ids), ttl_s=300)

    # Submit a label
    r = await http_client.post(
        "/v1/ft/labeling/labels",
        json={
            "tenant_id": "acme", "session_id": session_id,
            "sample_id": ids[0], "label": "billing", "time_ms": 3000,
        },
    )
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["accepted"]
    assert body["next_sample_id"] == ids[1]


async def test_unknown_label_returns_422(http_client) -> None:  # type: ignore[no-untyped-def]
    r = await http_client.post(
        "/v1/ft/ingest/acme/routing/sample",
        json={
            "tenant_id": "acme", "task_id": "routing",
            "text": "une ligne", "source": "batch",
            "actor_id": "alice",
        },
    )
    ids = [r.json()]  # reuse
    r = await http_client.post(
        "/v1/ft/labeling/sessions",
        json={
            "annotator_id": "alice", "tenant_id": "acme",
            "task_id": "routing",
        },
    )
    session_id = r.json()["session_id"]

    from efe_ft.api.dependencies import get_context
    ctx = get_context()
    # fetch sample id
    r = await http_client.get("/v1/ft/samples/acme/routing")
    sid = r.json()["items"][0]["id"]
    state = await ctx.session_store.get(session_id)
    await ctx.session_store.set(state.with_batch([sid]), ttl_s=300)

    r = await http_client.post(
        "/v1/ft/labeling/labels",
        json={
            "tenant_id": "acme", "session_id": session_id,
            "sample_id": sid, "label": "not_a_real_class",
        },
    )
    assert r.status_code == 422
