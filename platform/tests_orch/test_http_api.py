"""HTTP API integration tests — exercise the FastAPI app end-to-end."""

from __future__ import annotations

import pytest
from httpx import ASGITransport, AsyncClient

from efe_orch.api import OrchContext, build_app


@pytest.fixture
async def client(orch_context: OrchContext):  # type: ignore[no-untyped-def]
    app = build_app(orch_context)
    transport = ASGITransport(app=app)
    async with AsyncClient(
        transport=transport, base_url="http://testserver",
    ) as c:
        yield c


async def test_healthz(client: AsyncClient) -> None:
    r = await client.get("/healthz")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"


async def test_list_pools(client: AsyncClient) -> None:
    r = await client.get("/v1/orch/pools")
    assert r.status_code == 200
    names = [p["name"] for p in r.json()["pools"]]
    assert "cpu-small" in names
    assert "gpu-a10" in names


async def test_submit_run_flow(client: AsyncClient) -> None:
    body = {
        "tenant_id": "acme",
        "task_id": "routing",
        "submitted_by": "alice@acme",
        "snapshot_id": "sn-1",
        "dataset_rows": 200,
    }
    r = await client.post("/v1/orch/runs", json=body)
    assert r.status_code == 200
    data = r.json()
    assert data["state"] == "queued"
    assert data["pool"] == "cpu-small"

    # GET it back
    run_id = data["id"]
    r = await client.get(f"/v1/orch/runs/{run_id}")
    assert r.status_code == 200
    assert r.json()["id"] == run_id


async def test_submit_bad_request(client: AsyncClient) -> None:
    r = await client.post("/v1/orch/runs", json={"tenant_id": "acme"})
    assert r.status_code == 422  # pydantic validation on required fields


async def test_dispatch_then_heartbeat_and_result(client: AsyncClient) -> None:
    body = {
        "tenant_id": "acme", "task_id": "routing",
        "submitted_by": "alice@acme", "snapshot_id": "sn-1",
        "dataset_rows": 50,
    }
    r = await client.post("/v1/orch/runs", json=body)
    run = r.json()
    run_id = run["id"]

    r = await client.post(f"/v1/orch/runs/{run_id}:dispatch")
    assert r.status_code == 200
    scheduled = r.json()
    worker_id = scheduled["worker_id"]

    r = await client.post("/v1/orch/workers/heartbeat", json={
        "run_id": run_id, "worker_id": worker_id, "beat_seq": 1,
    })
    assert r.status_code == 200
    assert r.json()["state"] == "training"

    r = await client.post("/v1/orch/workers/result", json={
        "run_id": run_id, "worker_id": worker_id,
        "artifact_url": "s3://ok",
        "metrics": {"f1": 0.88},
    })
    assert r.status_code == 200
    assert r.json()["state"] == "succeeded"


async def test_cancel_endpoint(client: AsyncClient) -> None:
    body = {
        "tenant_id": "acme", "task_id": "routing",
        "submitted_by": "alice@acme", "snapshot_id": "sn",
        "dataset_rows": 10,
    }
    r = await client.post("/v1/orch/runs", json=body)
    run_id = r.json()["id"]
    r = await client.post(
        f"/v1/orch/runs/{run_id}:cancel",
        json={"actor": "alice@acme", "reason": "oops"},
    )
    assert r.status_code == 200
    assert r.json()["state"] == "cancelled"


async def test_error_maps_to_orch_error_body(client: AsyncClient) -> None:
    r = await client.get("/v1/orch/runs/00000000-0000-0000-0000-000000000000")
    assert r.status_code == 404
    body = r.json()
    assert body["code"] == "run_not_found"


async def test_list_tenant_runs_paginated(client: AsyncClient) -> None:
    # Submit 3 runs; page_size = 2 → one page + continuation.
    for _ in range(3):
        await client.post("/v1/orch/runs", json={
            "tenant_id": "acme", "task_id": "t",
            "submitted_by": "alice", "snapshot_id": "sn",
            "dataset_rows": 1,
        })
    r = await client.get("/v1/orch/tenants/acme/runs", params={"page_size": 2})
    data = r.json()
    assert len(data["items"]) == 2
    assert data["next_token"] == "2"

    r2 = await client.get(
        "/v1/orch/tenants/acme/runs",
        params={"page_size": 2, "page_token": data["next_token"]},
    )
    data2 = r2.json()
    assert len(data2["items"]) == 1
    assert data2["next_token"] is None


async def test_admin_reap_endpoint(client: AsyncClient) -> None:
    r = await client.post("/v1/orch/admin/reap")
    assert r.status_code == 200
    data = r.json()
    assert "scanned" in data
    assert "reaped" in data


async def test_transitions_endpoint(client: AsyncClient) -> None:
    """GET /v1/orch/runs/{id}/transitions returns the audit trail."""
    r = await client.post("/v1/orch/runs", json={
        "tenant_id": "acme", "task_id": "t",
        "submitted_by": "alice", "snapshot_id": "sn",
        "dataset_rows": 10,
    })
    run_id = r.json()["id"]
    r = await client.get(f"/v1/orch/runs/{run_id}/transitions")
    assert r.status_code == 200
    items = r.json()["items"]
    # DRAFT→QUEUED was written by submit.
    assert len(items) >= 1
    assert items[0]["to_state"] == "queued"


async def test_heartbeats_endpoint(client: AsyncClient) -> None:
    r = await client.post("/v1/orch/runs", json={
        "tenant_id": "acme", "task_id": "t",
        "submitted_by": "a", "snapshot_id": "sn",
        "dataset_rows": 10,
    })
    run_id = r.json()["id"]
    d = (await client.post(f"/v1/orch/runs/{run_id}:dispatch")).json()
    worker_id = d["worker_id"]

    for seq in (1, 2, 3):
        await client.post("/v1/orch/workers/heartbeat", json={
            "run_id": run_id, "worker_id": worker_id, "beat_seq": seq,
        })
    r = await client.get(f"/v1/orch/runs/{run_id}/heartbeats")
    assert r.status_code == 200
    items = r.json()["items"]
    # 3 heartbeats persisted.
    assert len(items) == 3
    # Newest-first by observed_at.
    seqs = [i["beat_seq"] for i in items]
    assert seqs == [3, 2, 1]


async def test_checkpoints_endpoint(client: AsyncClient) -> None:
    r = await client.post("/v1/orch/runs", json={
        "tenant_id": "acme", "task_id": "t",
        "submitted_by": "a", "snapshot_id": "sn",
        "dataset_rows": 10,
    })
    run_id = r.json()["id"]
    d = (await client.post(f"/v1/orch/runs/{run_id}:dispatch")).json()
    worker_id = d["worker_id"]
    await client.post("/v1/orch/workers/heartbeat", json={
        "run_id": run_id, "worker_id": worker_id, "beat_seq": 1,
    })
    for step in (10, 50):
        await client.post("/v1/orch/workers/checkpoint", json={
            "run_id": run_id, "worker_id": worker_id,
            "step": step, "blob_uri": f"s3://b/ckpt-{step}",
            "is_best": step == 50,
        })
    r = await client.get(f"/v1/orch/runs/{run_id}/checkpoints")
    assert r.status_code == 200
    items = r.json()["items"]
    assert len(items) == 2
    steps = [i["step"] for i in items]
    assert steps == [10, 50]


async def test_admin_get_quota_endpoint(client: AsyncClient) -> None:
    r = await client.get("/v1/orch/admin/tenants/acme/quota")
    assert r.status_code == 200
    body = r.json()
    assert body["tenant_id"] == "acme"
    assert body["current_concurrent_runs"] == 0


async def test_admin_reset_quota_endpoint(client: AsyncClient) -> None:
    # Submit a run to increment the counter.
    await client.post("/v1/orch/runs", json={
        "tenant_id": "acme", "task_id": "t",
        "submitted_by": "a", "snapshot_id": "sn",
        "dataset_rows": 10,
    })
    before = (await client.get("/v1/orch/admin/tenants/acme/quota")).json()
    assert before["current_monthly_runs"] == 1

    r = await client.post("/v1/orch/admin/tenants/acme/quota:reset")
    assert r.status_code == 200
    after = (await client.get("/v1/orch/admin/tenants/acme/quota")).json()
    assert after["current_monthly_runs"] == 0
    # Concurrency counter is NOT reset (reflects in-flight runs).
    assert after["current_concurrent_runs"] == before["current_concurrent_runs"]
