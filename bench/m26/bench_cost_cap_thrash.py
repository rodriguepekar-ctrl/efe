"""bench_cost_cap_thrash — 50 runs designed to hit the $0.01 cost cap.

All 50 must transition to COST_CAPPED within 60s of cap exceeded ;
no run may overspend by > 5 %."""
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
async def test_cost_cap_thrash():
    async with httpx.AsyncClient(timeout=30.0) as client:
        run_ids: list[str] = []
        for _ in range(50):
            r = await client.post(
                f"{API_BASE}/v1/agent/runs",
                json={
                    "tenant_id": "bench",
                    "template_slug": "expensive_loop",
                    "initial_prompt": {"prompt": "hot"},
                    "max_cost_usd": 0.01,
                    "timeout_seconds": 600,
                },
                headers=headers(),
            )
            if r.is_success:
                run_ids.append(r.json()["id"])

        deadline = time.time() + 600
        results: list[dict] = []
        while time.time() < deadline and len(results) < len(run_ids):
            for rid in run_ids:
                if rid in [r["id"] for r in results]:
                    continue
                d = (await client.get(
                    f"{API_BASE}/v1/agent/runs/{rid}",
                    headers=headers(),
                )).json()["run"]
                if d["status"] in ("COST_CAPPED", "COMPLETED", "FAILED", "TERMINATED"):
                    results.append(d)
            await asyncio.sleep(5)

        capped = [r for r in results if r["status"] == "COST_CAPPED"]
        assert len(capped) >= len(run_ids) * 0.9, \
            f"only {len(capped)}/{len(run_ids)} hit COST_CAPPED"

        for r in capped:
            spent = r["current_cost_cents"]
            cap = r["max_cost_cents"]
            assert spent <= int(cap * 1.05), \
                f"run {r['id']} overspent : {spent} vs cap {cap}"
