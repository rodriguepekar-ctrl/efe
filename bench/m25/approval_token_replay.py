"""Approval-token replay test : the same approval_token must work
exactly ONCE. A second use must return 403 with `approval_already_consumed`."""
from __future__ import annotations

import os
import time

import httpx
import pytest

API = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
TENANT = "bench"


def _h() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
def test_approval_token_is_single_use() -> None:
    # 1. Create a run that requires write approval.
    body = {
        "tenant_id": TENANT,
        "template_slug": "catalog_curator_demo",
        "initial_prompt": {"action": "upsert"},
        "max_cost_usd": 0.05,
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=10)
    r.raise_for_status()
    run_id = r.json()["id"]

    # 2. Wait until AWAITING_APPROVAL.
    approval_id = None
    deadline = time.time() + 60
    while time.time() < deadline:
        d = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=5).json()
        if d["pending_approvals"]:
            approval_id = d["pending_approvals"][0]["id"]
            break
        time.sleep(2)
    assert approval_id, "no approval ever requested"

    # 3. Approve.
    a = httpx.post(
        f"{API}/v1/agent/approvals/approve",
        json={"approval_id": approval_id, "reason": "bench test approval (replay attack expected)"},
        headers=_h(),
        timeout=5,
    )
    a.raise_for_status()
    token = a.json()["approval_token"]

    # 4. Replay the SAME token via the mcp-server tool call simulator.
    r1 = httpx.post(
        f"{API}/v1/agent/_test/replay_token",
        json={"approval_token": token, "run_id": run_id},
        headers=_h(),
        timeout=5,
    )
    assert r1.status_code in (200, 204), f"first use should succeed: {r1.status_code}"

    r2 = httpx.post(
        f"{API}/v1/agent/_test/replay_token",
        json={"approval_token": token, "run_id": run_id},
        headers=_h(),
        timeout=5,
    )
    assert r2.status_code == 403
    body = r2.json()
    assert body.get("code") in ("approval_already_consumed", "invalid_approval_token")
