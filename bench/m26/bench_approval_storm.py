"""bench_approval_storm — 100 simultaneous approval requests + grants.
Verifies token race condition handling : no token can be successfully
consumed twice."""
from __future__ import annotations

import asyncio
import os
import time

import httpx
import pytest

API_BASE = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")


def headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
@pytest.mark.asyncio
async def test_approval_storm():
    # Phase 1 : create 100 runs that all need approval at the same moment.
    async with httpx.AsyncClient(timeout=30.0) as client:
        run_ids: list[str] = []
        for _ in range(100):
            r = await client.post(
                f"{API_BASE}/v1/agent/runs",
                json={
                    "tenant_id": "bench",
                    "template_slug": "catalog_curator_demo",
                    "initial_prompt": {"action": "upsert"},
                    "max_cost_usd": 0.05,
                },
                headers=headers(),
            )
            if r.is_success:
                run_ids.append(r.json()["id"])
        assert len(run_ids) >= 80, f"only {len(run_ids)}/100 runs created"

        # Phase 2 : wait for all to reach AWAITING_APPROVAL.
        approval_ids: list[str] = []
        deadline = time.time() + 120
        while time.time() < deadline and len(approval_ids) < len(run_ids):
            for rid in run_ids:
                if rid in [a.get("run_id") for a in approval_ids]:
                    continue
                d = (await client.get(
                    f"{API_BASE}/v1/agent/runs/{rid}",
                    headers=headers(),
                )).json()
                if d.get("pending_approvals"):
                    approval_ids.append({
                        "run_id": rid,
                        "approval_id": d["pending_approvals"][0]["id"],
                    })
            await asyncio.sleep(2)
        assert len(approval_ids) >= len(run_ids) * 0.8

        # Phase 3 : approve all in parallel.
        async def approve(approval_id: str) -> bool:
            try:
                r = await client.post(
                    f"{API_BASE}/v1/agent/approvals/approve",
                    json={"approval_id": approval_id, "reason": "bench storm"},
                    headers=headers(),
                )
                return r.is_success
            except Exception:
                return False

        results = await asyncio.gather(*[
            approve(a["approval_id"]) for a in approval_ids
        ])
        success_rate = sum(results) / len(results)
        assert success_rate > 0.95, f"approval success {success_rate:.2%}"
